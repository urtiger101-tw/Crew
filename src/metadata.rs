//! Ephemeral display tokens. They never participate in agent lifecycle or waits.
mod cli;
#[cfg(test)]
mod tests;

use crate::api::{ApiError, ApiResult, Reply, clean_text, lock, pane_json};
use crate::server::{Shared, State};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

pub use cli::parse;

pub const HELP: &str = r#"DISPLAY TOKENS (independent of agent status):
  crew pane report-metadata pN --source ID [--token KEY=VALUE] [--clear-token KEY]
                            [--seq N] [--ttl-ms N]
  crew workspace report-metadata ID|NAME --source ID [--token KEY=VALUE]
                                 [--clear-token KEY] [--seq N] [--ttl-ms N]
Repeat --token / --clear-token to patch distinct keys; omitted keys remain unchanged.
TTL is 1..86400000 ms and applies only to keys set by that report. Omit it to keep
values until replaced, cleared, the resource closes, or the server restarts.
Tokens appear in get/list/snapshot responses and metadata_updated events.
"#;

const MAX_PATCH: usize = 16;
const MAX_KEYS: usize = 32;
const MAX_SOURCES: usize = 32;
const MAX_TTL_MS: u64 = 86_400_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Target {
    Pane(usize),
    Workspace(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub target: Target,
    pub source: String,
    pub tokens: BTreeMap<String, Option<String>>,
    pub seq: Option<u64>,
    pub ttl_ms: Option<u64>,
}

impl Request {
    /// The same validation is used by the CLI and the native daemon API.
    fn validate(&self) -> Result<(), ApiError> {
        if self.source.is_empty()
            || self.source.len() > 80
            || !self
                .source
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
        {
            return Err(ApiError::invalid(
                "source must be 1..80 of A-Za-z0-9 : . _ -",
            ));
        }
        if !(1..=MAX_PATCH).contains(&self.tokens.len()) {
            return Err(ApiError::invalid(
                "a metadata patch must contain 1..16 distinct keys",
            ));
        }
        for key in self.tokens.keys() {
            if key.is_empty()
                || key.len() > 32
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            {
                return Err(ApiError::invalid(
                    "token keys must be 1..32 of A-Za-z0-9 _ -",
                ));
            }
        }
        if self.ttl_ms.is_some_and(|n| !(1..=MAX_TTL_MS).contains(&n)) {
            return Err(ApiError::invalid("ttl-ms must be 1..86400000"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Pane(usize),
    Workspace(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    value: String,
    deadline: Option<Instant>,
}

impl Token {
    fn alive(&self, now: Instant) -> bool {
        self.deadline.is_none_or(|deadline| now < deadline)
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
struct Resource {
    tokens: BTreeMap<String, Token>,
    // Kept until the resource closes, even after every token has expired or cleared.
    sequences: BTreeMap<String, u64>,
}

#[derive(Default, Debug)]
pub struct Store {
    resources: BTreeMap<Key, Resource>,
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    accepted: bool,
    changed: Vec<String>,
}

impl Store {
    /// Read-time filtering gives exact TTL semantics even between background sweeps.
    /// It does not remove entries, so the sweep can still emit each expiry event once.
    pub(crate) fn values(&self, key: Key, now: Instant) -> BTreeMap<String, String> {
        self.resources
            .get(&key)
            .map_or_else(BTreeMap::new, |resource| {
                resource
                    .tokens
                    .iter()
                    .filter(|(_, token)| token.alive(now))
                    .map(|(name, token)| (name.clone(), token.value.clone()))
                    .collect()
            })
    }

    fn apply(&mut self, key: Key, req: &Request, now: Instant) -> Result<Outcome, ApiError> {
        req.validate()?;
        let previous = self.resources.get(&key);
        if req.seq.is_some_and(|seq| {
            previous
                .and_then(|r| r.sequences.get(&req.source))
                .is_some_and(|last| seq <= *last)
        }) {
            return Ok(Outcome {
                accepted: false,
                changed: Vec::new(),
            });
        }
        if req.seq.is_some()
            && previous.is_some_and(|r| {
                !r.sequences.contains_key(&req.source) && r.sequences.len() >= MAX_SOURCES
            })
        {
            return Err(ApiError::new(
                "metadata_source_limit",
                "resource already has 32 sequenced sources",
            ));
        }
        let deadline = req
            .ttl_ms
            .map(|ms| {
                now.checked_add(Duration::from_millis(ms))
                    .ok_or_else(|| ApiError::invalid("metadata deadline is out of range"))
            })
            .transpose()?;
        let patch: BTreeMap<String, Option<String>> = req
            .tokens
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    value.as_deref().and_then(|v| clean_text(v, 80)),
                )
            })
            .collect();
        let mut names: BTreeSet<String> = self.values(key, now).into_keys().collect();
        for (name, value) in &patch {
            if value.is_some() {
                names.insert(name.clone());
            } else {
                names.remove(name);
            }
        }
        if names.len() > MAX_KEYS {
            return Err(ApiError::new(
                "metadata_key_limit",
                "resource may retain at most 32 token keys",
            ));
        }

        // Commit only after all validation/capacity checks; a rejected report never
        // advances seq or applies only a prefix of a patch.
        let resource = self.resources.entry(key).or_default();
        let mut changed = Vec::new();
        for (name, value) in patch {
            match value {
                Some(value) => {
                    let next = Token { value, deadline };
                    if resource.tokens.get(&name) != Some(&next) {
                        resource.tokens.insert(name.clone(), next);
                        changed.push(name);
                    }
                }
                None => {
                    if resource.tokens.remove(&name).is_some() {
                        changed.push(name);
                    }
                }
            }
        }
        if let Some(seq) = req.seq {
            resource.sequences.insert(req.source.clone(), seq);
        }
        Ok(Outcome {
            accepted: true,
            changed,
        })
    }

    fn expire(&mut self, now: Instant) -> Vec<(Key, Vec<String>)> {
        let mut changed = Vec::new();
        for (key, resource) in &mut self.resources {
            let expired: Vec<String> = resource
                .tokens
                .iter()
                .filter(|(_, token)| !token.alive(now))
                .map(|(name, _)| name.clone())
                .collect();
            if !expired.is_empty() {
                for name in &expired {
                    resource.tokens.remove(name);
                }
                changed.push((*key, expired));
            }
        }
        changed
    }

    fn retain_live(&mut self, live: impl Fn(&Key) -> bool) {
        self.resources.retain(|key, _| live(key));
    }
}

fn record(st: &mut State, key: Key, reason: &str, changed: Vec<String>) {
    let (kind, pane, data) = match key {
        Key::Pane(id) => {
            let Some(pane) = pane_json(st, id) else {
                return;
            };
            (
                "pane.metadata_updated",
                Some(id),
                json!({"reason": reason, "changed_keys": changed, "pane": pane}),
            )
        }
        Key::Workspace(id) => {
            let Some(si) = st.sessions.iter().position(|s| s.id == id) else {
                return;
            };
            (
                "workspace.metadata_updated",
                None,
                json!({"reason": reason, "changed_keys": changed,
                "workspace": crate::workspace::workspace_json(st, si)}),
            )
        }
    };
    st.events.record(kind, pane, data);
    st.mark_dirty();
}

/// Called on the server's existing timer, even when no PTY output arrives. Closing
/// a resource discards its tokens and source high-water marks; there is no restore.
pub(crate) fn sweep(st: &mut State, now: Instant) {
    let panes: BTreeSet<usize> = st.panes.keys().copied().collect();
    let workspaces: BTreeSet<usize> = st.sessions.iter().map(|s| s.id).collect();
    st.metadata.retain_live(|key| match key {
        Key::Pane(id) => panes.contains(id),
        Key::Workspace(id) => workspaces.contains(id),
    });
    for (key, changed) in st.metadata.expire(now) {
        record(st, key, "expired", changed);
    }
}

pub fn handle(shared: &Shared, req: Request) -> ApiResult {
    req.validate()?;
    let mut st = lock(shared);
    let key = match &req.target {
        Target::Pane(id) => {
            if !st.panes.contains_key(id) || st.locate(*id).is_none() {
                return Err(ApiError::pane_not_found(*id));
            }
            Key::Pane(*id)
        }
        Target::Workspace(target) => {
            let si = crate::workspace::workspace_index(&st, target)?;
            Key::Workspace(st.sessions[si].id)
        }
    };
    let now = Instant::now();
    sweep(&mut st, now);
    let outcome = st.metadata.apply(key, &req, now)?;
    let changed = !outcome.changed.is_empty();
    if changed {
        record(&mut st, key, "report", outcome.changed);
    }
    let mut result = json!({"accepted": outcome.accepted, "changed": changed});
    match key {
        Key::Pane(id) => result["pane"] = pane_json(&st, id).unwrap(),
        Key::Workspace(id) => {
            let si = st.sessions.iter().position(|s| s.id == id).unwrap();
            result["workspace"] = crate::workspace::workspace_json(&st, si);
        }
    }
    Ok(Reply::Json(result))
}
