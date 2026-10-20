//! Agent state model, reports, waits and prompts. OWNER: worker H3.
//!
//! Status precedence: a reported agent state wins (a reported idle with unseen output reads as
//! `Done`); without a report the output heuristic decides. Handlers lock `Shared` only for each
//! check. Waits poll every [`POLL_INTERVAL`] and never sleep while holding the lock.
use super::pane as pane_api;
use super::{
    AgentInfo, ApiError, ApiResult, LABEL_MAX, POLL_INTERVAL, RESTORE_SOURCE, ReadSource, Reply,
    Request, Status, Target, WaitSpec, clean_text, is_agent_name, lock, pane_id_str, pane_json,
};
use crate::config::Config;
use crate::server::{Pane, Shared, State};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Delay between the prompt text and Enter, so TUI agents accept the text as input.
const SUBMIT_ENTER_DELAY_MS: u64 = 150;
/// A waited prompt must start the agent (Working or Blocked) within this window.
const PROMPT_START_LIMIT: Duration = Duration::from_millis(5000);
/// Default wait set when a request names no statuses.
const DEFAULT_UNTIL: [Status; 3] = [Status::Idle, Status::Done, Status::Blocked];

/// Zero is reserved for a restored, not-yet-reported occupant. Tokens are local to one
/// server process: no request or wait survives a server restart.
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug)]
struct Occupant {
    pane: usize,
    instance: u64,
}

impl Occupant {
    fn resolve(st: &State, target: &Target) -> Result<Self, ApiError> {
        let pane = resolve_target(st, target)?;
        let agent = st.panes[&pane]
            .agent
            .as_ref()
            .ok_or_else(|| missing_agent(pane))?;
        Ok(Self {
            pane,
            instance: agent.instance,
        })
    }

    fn check(self, st: &State) -> Result<Status, ApiError> {
        let pane = st
            .panes
            .get(&self.pane)
            .ok_or_else(|| ApiError::pane_not_found(self.pane))?;
        let agent = pane
            .agent
            .as_ref()
            .ok_or_else(|| missing_agent(self.pane))?;
        if agent.instance != self.instance {
            return Err(ApiError::new(
                "agent_replaced",
                format!(
                    "{} now hosts a different agent instance",
                    pane_id_str(self.pane)
                ),
            ));
        }
        Ok(status(pane, &st.cfg))
    }
}

fn missing_agent(pane: usize) -> ApiError {
    ApiError::new(
        "agent_not_found",
        format!(
            "{} has no reported agent; use pane commands for an ordinary terminal",
            pane_id_str(pane)
        ),
    )
}

/// Effective status of a pane.
pub fn status(p: &Pane, cfg: &Config) -> Status {
    derive_status(p.agent.as_ref().map(|a| a.state), p.busy(cfg), p.attention)
}

/// Pure decision table behind [`status`].
pub fn derive_status(reported: Option<Status>, busy: bool, attention: bool) -> Status {
    match reported {
        Some(Status::Idle) if attention => Status::Done,
        Some(state) => state,
        None if busy => Status::Working,
        None if attention => Status::Done,
        None => Status::Idle,
    }
}

/// Fields of a `pane report-agent` request, after the handler has validated them.
struct Report {
    source: String,
    agent: String,
    state: Status,
    message: Option<String>,
    seq: Option<u64>,
    resume: Vec<String>,
}

/// Dispatch for agent requests. Other requests are routed by `api::handle`.
pub fn handle(shared: &Shared, req: Request) -> ApiResult {
    let start = Instant::now();
    let value = match req {
        Request::PaneReportAgent {
            pane,
            source,
            agent,
            state,
            message,
            seq,
            resume,
        } => report_agent(
            shared,
            pane,
            Report {
                source,
                agent,
                state,
                message,
                seq,
                resume,
            },
        ),
        Request::PaneReleaseAgent {
            pane,
            source,
            agent,
            seq,
        } => release_agent(shared, pane, &source, &agent, seq),
        Request::AgentList => Ok(agent_list(&lock(shared))),
        Request::AgentGet { target } => {
            let st = lock(shared);
            let pid = resolve_target(&st, &target)?;
            Ok(json!({"agent": pane_obj(&st, pid)?}))
        }
        Request::AgentRead {
            target,
            source,
            lines,
        } => return read_agent(shared, &target, source, lines),
        Request::AgentSendKeys { target, keys } => send_keys_to(shared, &target, &keys),
        Request::AgentPrompt { target, text, wait } => {
            agent_prompt(shared, &target, &text, wait.as_ref(), start)
        }
        Request::AgentWait { target, wait } => agent_wait(shared, &target, &wait, start),
        Request::AgentRename { target, name } => rename_agent(shared, &target, name),
        Request::AgentFocus { target } => focus_agent(shared, &target),
        other => {
            return Err(ApiError::new(
                "internal",
                format!("agent::handle got a non-agent request: {other:?}"),
            ));
        }
    };
    value.map(Reply::Json)
}

fn pane_obj(st: &State, pid: usize) -> Result<Value, ApiError> {
    pane_json(st, pid).ok_or_else(|| ApiError::pane_not_found(pid))
}

/// Pane ids whose reported agent carries `name`.
fn owners_of(st: &State, name: &str) -> Vec<usize> {
    st.panes
        .iter()
        .filter(|(_, p)| p.agent.as_ref().and_then(|a| a.name.as_deref()) == Some(name))
        .map(|(pid, _)| *pid)
        .collect()
}

/// The single owner of a name, if exactly one pane holds it.
fn unique_owner(owners: &[usize]) -> Option<usize> {
    match owners {
        [one] => Some(*one),
        _ => None,
    }
}

/// True when a live agent other than `me` already holds the name.
fn name_conflict(owners: &[usize], me: usize) -> bool {
    owners.iter().any(|&owner| owner != me)
}

/// Resolve an agent target to a pane id. A name must match exactly one live agent.
pub fn resolve_target(st: &State, t: &Target) -> Result<usize, ApiError> {
    match t {
        Target::Pane(pid) if st.panes.contains_key(pid) => Ok(*pid),
        Target::Pane(pid) => Err(ApiError::pane_not_found(*pid)),
        Target::Name(name) => unique_owner(&owners_of(st, name)).ok_or_else(|| {
            ApiError::new("agent_not_found", format!("no unique agent named {name:?}"))
        }),
    }
}

/// Trim, drop control characters, cap at [`LABEL_MAX`]. Empty text becomes `None`.
pub fn normalize_message(raw: Option<&str>) -> Option<String> {
    clean_text(raw?, LABEL_MAX)
}

/// Agent label as stored and compared: cleaned display text, never empty.
fn normalize_label(raw: &str) -> Result<String, ApiError> {
    clean_text(raw, LABEL_MAX).ok_or_else(|| ApiError::invalid("agent label must not be empty"))
}

/// Agent name for `agent rename`. `None` clears the name. Otherwise the name must pass
/// [`is_agent_name`], which also rejects pane-id shaped names such as `p1`.
fn normalize_name(raw: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let name = raw.trim();
    if !is_agent_name(name) {
        return Err(ApiError::invalid(
            "agent name must start with a letter, use only a-z 0-9 _ -, be at most 32 \
             characters, and not look like a pane id",
        ));
    }
    Ok(Some(name.to_string()))
}

/// True when a report or release must be ignored: the same reporter already sent an equal or
/// newer seq. Reports without a seq are never stale.
pub fn is_stale(existing: Option<&AgentInfo>, source: &str, seq: Option<u64>) -> bool {
    match (existing, seq) {
        (Some(old), Some(new)) => old.source == source && old.seq.is_some_and(|prev| prev >= new),
        _ => false,
    }
}

/// True when a release may act on the stored agent: the same reporter and label, or an agent
/// restored after a server restart (source [`RESTORE_SOURCE`]) with the same label, so the
/// hook that owns the agent can take it over.
fn may_release(existing: Option<&AgentInfo>, source: &str, agent: &str) -> bool {
    existing.is_some_and(|a| a.agent == agent && (a.source == source || a.source == RESTORE_SOURCE))
}

/// Seq to keep when a report omits `--seq`: the stored seq, but only while the same source
/// owns the agent. Another source's numbering must not gate this reporter's reports.
fn kept_seq(existing: Option<&AgentInfo>, source: &str) -> Option<u64> {
    existing
        .filter(|old| old.source == source)
        .and_then(|old| old.seq)
}

/// An accepted report retains its instance only while source and kind both match. An
/// explicit release removes the old instance, so even identical re-reports allocate anew.
fn instance_for(existing: Option<&AgentInfo>, r: &Report) -> Result<u64, ApiError> {
    if let Some(old) =
        existing.filter(|old| old.source == r.source && old.agent == r.agent && old.instance != 0)
    {
        return Ok(old.instance);
    }
    NEXT_INSTANCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| ApiError::new("internal", "agent instance counter exhausted"))
}

/// Names and resume commands belong to one occupant. Restored metadata may be claimed by
/// the first live reporter of the same kind, but an unrelated source cannot inherit it.
fn merge_info(existing: Option<&AgentInfo>, r: Report) -> Result<AgentInfo, ApiError> {
    let instance = instance_for(existing, &r)?;
    let keep_metadata = existing.is_some_and(|old| {
        old.agent == r.agent && (old.source == r.source || old.source == RESTORE_SOURCE)
    });
    let name = if keep_metadata {
        existing.and_then(|old| old.name.clone())
    } else {
        None
    };
    let resume = match existing {
        Some(old) if keep_metadata && r.resume.is_empty() => old.resume.clone(),
        _ => r.resume,
    };
    let seq = r.seq.or_else(|| kept_seq(existing, &r.source));
    Ok(AgentInfo {
        instance,
        source: r.source,
        agent: r.agent,
        state: r.state,
        message: normalize_message(r.message.as_deref()),
        seq,
        name,
        resume,
    })
}

fn validate_report(r: &Report) -> Result<(), ApiError> {
    if !matches!(
        r.state,
        Status::Idle | Status::Working | Status::Blocked | Status::Unknown
    ) {
        return Err(ApiError::invalid(format!(
            "state must be idle|working|blocked|unknown, got {}",
            r.state.as_str()
        )));
    }
    if r.source.trim().is_empty() {
        return Err(ApiError::invalid("source must not be empty"));
    }
    Ok(())
}

fn report_agent(shared: &Shared, pid: usize, mut r: Report) -> Result<Value, ApiError> {
    validate_report(&r)?;
    r.agent = normalize_label(&r.agent)?;
    let mut st = lock(shared);
    let p = st
        .panes
        .get_mut(&pid)
        .ok_or_else(|| ApiError::pane_not_found(pid))?;
    let applied = !is_stale(p.agent.as_ref(), &r.source, r.seq);
    if applied {
        let turned_blocked = r.state == Status::Blocked
            && !p.agent.as_ref().is_some_and(|a| a.state == Status::Blocked);
        let agent = merge_info(p.agent.as_ref(), r)?;
        let event = json!({"instance": agent.instance, "source": agent.source, "agent": agent.agent, "state": agent.state.as_str()});
        p.agent = Some(agent);
        st.events.record("agent.status", Some(pid), event);
        st.relayout_pane(pid);
        if turned_blocked {
            crate::commands::notify_blocked(&mut st, pid);
        }
    }
    Ok(json!({"applied": applied, "pane": pane_obj(&st, pid)?}))
}

fn release_agent(
    shared: &Shared,
    pid: usize,
    source: &str,
    agent: &str,
    seq: Option<u64>,
) -> Result<Value, ApiError> {
    let agent = normalize_label(agent)?;
    let mut st = lock(shared);
    let p = st
        .panes
        .get_mut(&pid)
        .ok_or_else(|| ApiError::pane_not_found(pid))?;
    if !may_release(p.agent.as_ref(), source, &agent) {
        return Err(ApiError::new(
            "agent_not_found",
            format!("no agent {agent:?} from {source:?} on {}", pane_id_str(pid)),
        ));
    }
    if is_stale(p.agent.as_ref(), source, seq) {
        return Ok(json!({"released": false}));
    }
    let event = p
        .agent
        .as_ref()
        .map(|a| json!({"instance": a.instance, "source": a.source, "agent": a.agent}))
        .unwrap_or_default();
    p.agent = None;
    st.events.record("agent.released", Some(pid), event);
    st.relayout_pane(pid);
    Ok(json!({"released": true}))
}

fn agent_list(st: &State) -> Value {
    let mut agents = Vec::new();
    for pid in st
        .sessions
        .iter()
        .flat_map(|s| s.windows.iter())
        .flat_map(|w| w.panes())
    {
        if st
            .panes
            .get(&pid)
            .is_some_and(|p| p.agent.is_some() || p.detected_agent.is_some())
        {
            agents.extend(pane_json(st, pid));
        }
    }
    json!({"agents": agents})
}

fn read_agent(
    shared: &Shared,
    target: &Target,
    source: ReadSource,
    lines: Option<usize>,
) -> ApiResult {
    let mut st = lock(shared);
    let pid = resolve_target(&st, target)?;
    let text = pane_api::read_text(&mut st, pid, source, lines)?;
    Ok(Reply::Text(text))
}

fn send_keys_to(shared: &Shared, target: &Target, keys: &[String]) -> Result<Value, ApiError> {
    let mut st = lock(shared);
    let pid = resolve_target(&st, target)?;
    pane_api::send_keys(&mut st, pid, keys)?;
    Ok(json!({"sent": true}))
}

fn focus_agent(shared: &Shared, target: &Target) -> Result<Value, ApiError> {
    let mut st = lock(shared);
    let pid = resolve_target(&st, target)?;
    if !st.focus_pane(pid) {
        return Err(ApiError::pane_not_found(pid));
    }
    st.mark_dirty();
    Ok(json!({"agent": pane_obj(&st, pid)?}))
}

fn rename_agent(shared: &Shared, target: &Target, name: Option<String>) -> Result<Value, ApiError> {
    let name = normalize_name(name)?;
    let mut st = lock(shared);
    let pid = resolve_target(&st, target)?;
    if st.panes.get(&pid).is_none_or(|p| p.agent.is_none()) {
        return Err(ApiError::new(
            "agent_not_found",
            format!("{} has no reported agent", pane_id_str(pid)),
        ));
    }
    let taken = name
        .as_deref()
        .is_some_and(|n| name_conflict(&owners_of(&st, n), pid));
    if taken {
        let n = name.as_deref().unwrap_or_default();
        return Err(ApiError::new(
            "name_taken",
            format!("agent name {n:?} is in use"),
        ));
    }
    if let Some(a) = st.panes.get_mut(&pid).and_then(|p| p.agent.as_mut()) {
        a.name = name;
    }
    st.mark_dirty();
    Ok(json!({"agent": pane_obj(&st, pid)?}))
}

/// Statuses that end a wait. An empty request means the default set.
fn until_set(requested: &[Status]) -> Vec<Status> {
    if requested.is_empty() {
        DEFAULT_UNTIL.to_vec()
    } else {
        requested.to_vec()
    }
}

/// `Idle` in the set also matches `Done`: a finished agent is idle, just not yet seen.
pub fn matches_until(until: &[Status], s: Status) -> bool {
    until.contains(&s) || (s == Status::Done && until.contains(&Status::Idle))
}

/// Absolute deadline for a caller timeout measured from `start`. `None` waits forever.
fn deadline(start: Instant, timeout_ms: Option<u64>) -> Option<Instant> {
    timeout_ms.and_then(|ms| start.checked_add(Duration::from_millis(ms)))
}

/// Sleep until the next poll, but never past the deadline.
fn next_sleep(now: Instant, deadline: Option<Instant>) -> Duration {
    deadline.map_or(POLL_INTERVAL, |d| {
        POLL_INTERVAL.min(d.saturating_duration_since(now))
    })
}

enum Outcome {
    Matched(Status, Value),
    Expired,
}

/// Poll one occupant, not merely a pane. A release or replacement cannot satisfy a wait.
/// The matched status and agent snapshot are captured under one lock, so a replacement
/// between the match and reply cannot accidentally put the new agent in the old reply.
fn poll_until(
    shared: &Shared,
    occupant: Occupant,
    deadline: Option<Instant>,
    hit: impl Fn(Status) -> bool,
) -> Result<Outcome, ApiError> {
    loop {
        {
            let st = lock(shared);
            let status = occupant.check(&st)?;
            if hit(status) {
                return Ok(Outcome::Matched(status, pane_obj(&st, occupant.pane)?));
            }
        }
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d) {
            return Ok(Outcome::Expired);
        }
        std::thread::sleep(next_sleep(now, deadline));
    }
}

fn timeout_error(pid: usize, what: &str) -> ApiError {
    ApiError::new("timeout", format!("{}: {what}", pane_id_str(pid)))
}

/// Wait until the pane reaches one of `until`, or fail with `timeout`.
fn wait_for(
    shared: &Shared,
    occupant: Occupant,
    deadline: Option<Instant>,
    until: &[Status],
) -> Result<(Status, Value), ApiError> {
    match poll_until(shared, occupant, deadline, |s| matches_until(until, s))? {
        Outcome::Matched(s, agent) => Ok((s, agent)),
        Outcome::Expired => Err(timeout_error(
            occupant.pane,
            "no waited status before timeout",
        )),
    }
}

fn agent_wait(
    shared: &Shared,
    target: &Target,
    wait: &WaitSpec,
    start: Instant,
) -> Result<Value, ApiError> {
    let occupant = Occupant::resolve(&lock(shared), target)?;
    let until = until_set(&wait.until);
    let (status, agent) = wait_for(shared, occupant, deadline(start, wait.timeout_ms), &until)?;
    Ok(json!({"agent": agent, "status": status.as_str()}))
}

/// Phase 1 of a waited prompt: the agent must start working (or block) in time. The caller's
/// timeout, when shorter than the start limit, takes the `timeout` code instead.
fn await_start(
    shared: &Shared,
    occupant: Occupant,
    caller: Option<Instant>,
) -> Result<(), ApiError> {
    let stall = Instant::now() + PROMPT_START_LIMIT;
    let caller_first = caller.is_some_and(|c| c <= stall);
    let deadline = if caller_first { caller } else { Some(stall) };
    let started = |s: Status| matches!(s, Status::Working | Status::Blocked);
    match poll_until(shared, occupant, deadline, started)? {
        Outcome::Matched(..) => Ok(()),
        Outcome::Expired if caller_first => {
            Err(timeout_error(occupant.pane, "prompt not started in time"))
        }
        Outcome::Expired => Err(ApiError::new(
            "agent_prompt_stalled",
            format!(
                "{} did not start after the prompt",
                pane_id_str(occupant.pane)
            ),
        )),
    }
}

fn agent_prompt(
    shared: &Shared,
    target: &Target,
    text: &str,
    wait: Option<&WaitSpec>,
    start: Instant,
) -> Result<Value, ApiError> {
    let caller = wait.and_then(|wait| deadline(start, wait.timeout_ms));
    let (occupant, before) = {
        let mut st = lock(shared);
        let occupant = Occupant::resolve(&st, target)?;
        let before = occupant.check(&st)?;
        if before == Status::Blocked {
            return Err(ApiError::new(
                "agent_blocked",
                format!(
                    "{} is blocked; nothing was sent",
                    pane_id_str(occupant.pane)
                ),
            ));
        }
        if caller.is_some_and(|end| Instant::now() >= end) {
            return Err(timeout_error(
                occupant.pane,
                "prompt timeout before submission; nothing was sent",
            ));
        }
        // The identity check and text write share a lock. Do not delegate to pane::submit,
        // which is intentionally identity-agnostic for ordinary shell commands.
        st.paste(occupant.pane, text);
        st.mark_dirty();
        (occupant, before)
    };
    let delay = Duration::from_millis(SUBMIT_ENTER_DELAY_MS);
    std::thread::sleep(caller.map_or(delay, |end| {
        delay.min(end.saturating_duration_since(Instant::now()))
    }));
    {
        let mut st = lock(shared);
        if occupant.check(&st)? == Status::Blocked {
            return Err(ApiError::new(
                "agent_blocked",
                format!(
                    "{} became blocked after text was pasted; Enter was not sent",
                    pane_id_str(occupant.pane)
                ),
            ));
        }
        if caller.is_some_and(|end| Instant::now() >= end) {
            return Err(timeout_error(
                occupant.pane,
                "text was pasted but prompt timed out before Enter",
            ));
        }
        // A replacement after paste must never receive the delayed Enter.
        st.write_pane(occupant.pane, b"\r");
        st.mark_dirty();
        if wait.is_none() {
            return Ok(json!({"sent": true, "agent": pane_obj(&st, occupant.pane)?}));
        }
    }
    let Some(wait) = wait else {
        unreachable!("non-waiting prompt returned after Enter")
    };
    if before != Status::Working {
        await_start(shared, occupant, caller)?;
    }
    let until = until_set(&wait.until);
    let (status, agent) = wait_for(shared, occupant, caller, &until)?;
    Ok(json!({"sent": true, "agent": agent, "status": status.as_str()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(source: &str, agent: &str, seq: Option<u64>) -> AgentInfo {
        AgentInfo {
            instance: 0,
            source: source.into(),
            agent: agent.into(),
            state: Status::Working,
            message: None,
            seq,
            name: None,
            resume: vec![],
        }
    }

    fn report(agent: &str, resume: Vec<String>) -> Report {
        Report {
            source: "hook".into(),
            agent: agent.into(),
            state: Status::Working,
            message: Some("  go  ".into()),
            seq: Some(2),
            resume,
        }
    }

    #[test]
    fn status_without_report_uses_heuristic() {
        assert_eq!(derive_status(None, false, false), Status::Idle);
        assert_eq!(derive_status(None, true, false), Status::Working);
        assert_eq!(derive_status(None, false, true), Status::Done);
        assert_eq!(derive_status(None, true, true), Status::Working);
    }

    #[test]
    fn reported_state_wins_except_idle_with_unseen_output() {
        assert_eq!(derive_status(Some(Status::Idle), false, true), Status::Done);
        assert_eq!(
            derive_status(Some(Status::Idle), false, false),
            Status::Idle
        );
        assert_eq!(
            derive_status(Some(Status::Working), false, true),
            Status::Working
        );
        assert_eq!(
            derive_status(Some(Status::Blocked), true, true),
            Status::Blocked
        );
        assert_eq!(
            derive_status(Some(Status::Unknown), false, false),
            Status::Unknown
        );
    }

    #[test]
    fn message_is_trimmed_stripped_and_capped() {
        assert_eq!(normalize_message(None), None);
        assert_eq!(normalize_message(Some("  \t ")), None);
        assert_eq!(normalize_message(Some("\u{7}\u{1b}")), None);
        assert_eq!(
            normalize_message(Some("  hi\u{7} there \n")),
            Some("hi there".into())
        );
        let long = "字".repeat(200);
        assert_eq!(
            normalize_message(Some(&long)).map(|m| m.chars().count()),
            Some(LABEL_MAX)
        );
        let ascii = "a".repeat(120);
        assert_eq!(
            normalize_message(Some(&ascii)).map(|m| m.len()),
            Some(LABEL_MAX)
        );
    }

    #[test]
    fn stale_seq_only_for_same_source_and_older_or_equal_seq() {
        let old = info("hook", "claude", Some(5));
        assert!(!is_stale(None, "hook", Some(1)));
        assert!(is_stale(Some(&old), "hook", Some(5)));
        assert!(is_stale(Some(&old), "hook", Some(4)));
        assert!(!is_stale(Some(&old), "hook", Some(6)));
        assert!(!is_stale(Some(&old), "other", Some(1)));
        assert!(!is_stale(Some(&old), "hook", None));
        assert!(!is_stale(
            Some(&info("hook", "claude", None)),
            "hook",
            Some(1)
        ));
    }

    #[test]
    fn report_without_seq_keeps_stored_seq_from_same_source() {
        let old = info("hook", "claude", Some(5));
        let quiet = Report {
            seq: None,
            ..report("claude", vec![])
        };
        let merged = merge_info(Some(&old), quiet).unwrap();
        assert_eq!(merged.seq, Some(5));
        assert!(is_stale(Some(&merged), "hook", Some(4)));
        assert!(!is_stale(Some(&merged), "hook", Some(6)));
    }

    #[test]
    fn report_without_seq_drops_seq_of_another_source() {
        let old = info("other", "claude", Some(9));
        let quiet = Report {
            seq: None,
            ..report("claude", vec![])
        };
        assert_eq!(merge_info(Some(&old), quiet).unwrap().seq, None);
        assert_eq!(kept_seq(Some(&old), "hook"), None);
        assert_eq!(kept_seq(Some(&old), "other"), Some(9));
        assert_eq!(kept_seq(None, "hook"), None);
    }

    #[test]
    fn report_with_seq_replaces_stored_seq() {
        let old = info("hook", "claude", Some(5));
        assert_eq!(
            merge_info(Some(&old), report("claude", vec![]))
                .unwrap()
                .seq,
            Some(2)
        );
    }

    #[test]
    fn same_label_report_keeps_name_and_resume_when_resume_empty() {
        let mut old = info("hook", "claude", Some(1));
        old.name = Some("reviewer".into());
        old.resume = vec!["claude".into(), "--resume".into()];
        let merged = merge_info(Some(&old), report("claude", vec![])).unwrap();
        assert_eq!(merged.name.as_deref(), Some("reviewer"));
        assert_eq!(merged.resume, old.resume);
        assert_eq!(merged.message.as_deref(), Some("go"));
        assert_eq!(merged.seq, Some(2));
    }

    #[test]
    fn new_label_drops_name_and_new_resume_replaces_old() {
        let mut old = info("hook", "claude", Some(1));
        old.name = Some("reviewer".into());
        old.resume = vec!["old".into()];
        let other = merge_info(Some(&old), report("codex", vec![])).unwrap();
        assert_eq!(other.name, None);
        assert!(other.resume.is_empty());
        let replaced = merge_info(Some(&old), report("claude", vec!["new".into()])).unwrap();
        assert_eq!(replaced.resume, vec!["new".to_string()]);
        assert_eq!(replaced.name.as_deref(), Some("reviewer"));
    }

    #[test]
    fn report_takes_over_restored_agent_with_same_label() {
        let mut restored = info(RESTORE_SOURCE, "claude", None);
        restored.name = Some("reviewer".into());
        assert!(!is_stale(Some(&restored), "hook", Some(1)));
        let merged = merge_info(Some(&restored), report("claude", vec![])).unwrap();
        assert_eq!(merged.source, "hook");
        assert_eq!(merged.name.as_deref(), Some("reviewer"));
    }

    #[test]
    fn instance_survives_updates_but_not_replacement_or_release() {
        let initial = merge_info(None, report("codex", vec![])).unwrap();
        assert_ne!(initial.instance, 0);
        let updated = merge_info(Some(&initial), report("codex", vec![])).unwrap();
        assert_eq!(updated.instance, initial.instance);
        let replaced = merge_info(Some(&updated), report("claude", vec![])).unwrap();
        assert_ne!(replaced.instance, initial.instance);
        let other_source = merge_info(
            Some(&updated),
            Report {
                source: "new-process".into(),
                ..report("codex", vec![])
            },
        )
        .unwrap();
        assert_ne!(other_source.instance, initial.instance);
        // Release removes AgentInfo entirely. A subsequent report with identical source
        // and kind must allocate a fresh token, even when both arrive within one poll.
        let restarted = merge_info(None, report("codex", vec![])).unwrap();
        assert_ne!(restarted.instance, initial.instance);
    }

    #[test]
    fn unrelated_source_does_not_inherit_name_or_resume() {
        let mut old = info("old-process", "codex", Some(20));
        old.instance = 123;
        old.name = Some("reviewer".into());
        old.resume = vec!["codex".into(), "resume".into(), "old-session".into()];
        let replacement = merge_info(Some(&old), report("codex", vec![])).unwrap();
        assert!(replacement.name.is_none());
        assert!(replacement.resume.is_empty());
        assert_ne!(replacement.instance, old.instance);
    }

    #[test]
    fn release_matches_source_and_label_only() {
        let old = info("hook", "claude", None);
        assert!(may_release(Some(&old), "hook", "claude"));
        assert!(!may_release(Some(&old), "hook", "codex"));
        assert!(!may_release(Some(&old), "other", "claude"));
        assert!(!may_release(None, "hook", "claude"));
    }

    #[test]
    fn release_takes_over_restored_agent_with_same_label_only() {
        let restored = info(RESTORE_SOURCE, "claude", None);
        assert!(may_release(Some(&restored), "hook", "claude"));
        assert!(!may_release(Some(&restored), "hook", "codex"));
        assert!(may_release(Some(&restored), RESTORE_SOURCE, "claude"));
    }

    #[test]
    fn report_state_must_be_reportable() {
        let ok = report("claude", vec![]);
        assert!(validate_report(&ok).is_ok());
        let bad = Report {
            state: Status::Done,
            ..report("claude", vec![])
        };
        assert_eq!(validate_report(&bad).unwrap_err().code, "invalid_argument");
        let empty = Report {
            source: " ".into(),
            ..report("claude", vec![])
        };
        assert_eq!(
            validate_report(&empty).unwrap_err().code,
            "invalid_argument"
        );
    }

    #[test]
    fn agent_label_is_cleaned_and_never_empty() {
        assert_eq!(
            normalize_label("  claude\u{1b}\n"),
            Ok("claude".to_string())
        );
        assert_eq!(
            normalize_label(" \t ").unwrap_err().code,
            "invalid_argument"
        );
        let long = "x".repeat(200);
        assert_eq!(normalize_label(&long).map(|s| s.len()), Ok(LABEL_MAX));
    }

    #[test]
    fn until_set_default_and_idle_matches_done() {
        let defaults = until_set(&[]);
        assert_eq!(defaults, vec![Status::Idle, Status::Done, Status::Blocked]);
        for s in [Status::Idle, Status::Done, Status::Blocked] {
            assert!(matches_until(&defaults, s), "{s:?}");
        }
        assert!(!matches_until(&defaults, Status::Working));
        assert!(!matches_until(&defaults, Status::Unknown));
        assert!(matches_until(&[Status::Idle], Status::Done));
        assert!(!matches_until(&[Status::Done], Status::Idle));
        assert!(matches_until(&[Status::Working], Status::Working));
    }

    #[test]
    fn name_uniqueness_helpers() {
        assert_eq!(unique_owner(&[]), None);
        assert_eq!(unique_owner(&[3]), Some(3));
        assert_eq!(unique_owner(&[1, 2]), None);
        assert!(!name_conflict(&[5], 5));
        assert!(name_conflict(&[5], 3));
        assert!(!name_conflict(&[], 3));
    }

    #[test]
    fn name_normalization_accepts_names_and_rejects_pane_ids() {
        assert_eq!(normalize_name(None), Ok(None));
        assert_eq!(
            normalize_name(Some("  bob ".into())),
            Ok(Some("bob".into()))
        );
        assert_eq!(
            normalize_name(Some("reviewer-2".into())),
            Ok(Some("reviewer-2".into()))
        );
        for bad in ["  ", "a\nb", "p1", " p42 ", "Bob", "1a"] {
            let err = normalize_name(Some(bad.into())).unwrap_err();
            assert_eq!(err.code, "invalid_argument", "{bad:?}");
        }
    }

    #[test]
    fn deadline_and_sleep_bounds() {
        let start = Instant::now();
        assert_eq!(deadline(start, None), None);
        assert_eq!(deadline(start, Some(0)), Some(start));
        assert_eq!(next_sleep(start, None), POLL_INTERVAL);
        let near = start + Duration::from_millis(10);
        assert!(next_sleep(start, Some(near)) <= Duration::from_millis(10));
    }
}
