//! Layout persistence: save sessions/windows/splits/cwd to a state file and restore them
//! when the server starts. Processes do not survive a restart; only the layout does.
//! A legacy file, once selected, is the active state file: clear, quarantine and partial restore
//! may act on it, because no other binary owns it any more.
//! OWNER: worker H4.
mod resume;
mod snapshot;
#[cfg(test)]
mod tests;

use crate::api::{AgentInfo, Status};
use crate::ipc;
use crate::server::{Session, Shared, State, Window};
use snapshot::{LeafSnap, SessionSnap};
use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Same minimum sizes as `State::new_session`.
const MIN_COLS: u16 = 10;
const MIN_ROWS: u16 = 3;

/// Path of the state file: `$CREW_STATE`, else a per-socket file in the local data dir.
fn state_path() -> Option<PathBuf> {
    if let Some(p) = crate::brand::env_os("STATE").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let file = state_file_name(&ipc::base_name());
    let legacy = legacy_state_file_name();
    resolve_state_path(
        &dirs::data_local_dir()?,
        &file,
        legacy.as_deref(),
        &legacy_socket_live,
    )
}

/// The writable state file for the default socket, or `None` when this server must not read or
/// write any state because a live older server owns the layout. Every write path takes this
/// result, so `None` turns load, autosave, save-on-stop and clear into no-ops.
fn resolve_state_path(
    base: &Path,
    file: &str,
    legacy: Option<&str>,
    legacy_live: &dyn Fn() -> bool,
) -> Option<PathBuf> {
    match default_state_path(base, file, legacy, legacy_live) {
        Ok(selection) => {
            if selection == Selection::Deferred {
                log_deferred_once();
            }
            selection.writable_path()
        }
        Err(e) => {
            // Do not clear/quarantine a selected file if its selection cannot be recorded.
            crate::log!("state path selection failed; state files left unchanged: {e}");
            None
        }
    }
}

/// Logged once per process when this server defers to a live older server.
fn log_deferred_once() {
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        crate::log!(
            "older server on {} is still running; this server neither loads nor saves state until it stops",
            ipc::legacy_base_name()
        );
    }
}

/// State file name for `socket`: `state-<sanitized socket>.json`.
fn state_file_name(socket: &str) -> String {
    format!("state-{}.json", sanitize_file_part(socket))
}

/// The file the pre-rename build used for the default socket. `None` when `CREW_SOCKET` names
/// the endpoint: an explicit socket has its own state and never reads the legacy one.
fn legacy_state_file_name() -> Option<String> {
    (!ipc::socket_overridden()).then(|| state_file_name(&ipc::legacy_base_name()))
}

/// Whether the pre-rename server still listens. Probed at most once per process, so the choice
/// cannot flip while this server runs.
fn legacy_socket_live() -> bool {
    static LIVE: OnceLock<bool> = OnceLock::new();
    *LIVE.get_or_init(|| ipc::endpoint_is_live(&ipc::legacy_base_name()))
}

/// Marker written when a server takes a file as its state.
const SELECTED: &str = ".crew-selected";
/// Marker written when a server leaves a live older server's file alone.
const DEFERRED: &str = ".crew-deferred";

/// One candidate of the state chain, with what is on disk for it.
#[derive(Debug)]
struct Step {
    path: PathBuf,
    exists: bool,
    selected: bool,
    deferred: bool,
    /// `.bad` or `.partial.bad` exists: the file was moved or copied aside.
    quarantined: bool,
    /// The file belongs to the pre-rename default socket.
    from_legacy_socket: bool,
}

impl Step {
    fn read(path: PathBuf, from_legacy_socket: bool) -> Self {
        Self {
            exists: path.exists(),
            selected: with_suffix(&path, SELECTED).exists(),
            deferred: with_suffix(&path, DEFERRED).exists(),
            quarantined: with_suffix(&path, ".bad").exists()
                || with_suffix(&path, ".partial.bad").exists(),
            from_legacy_socket,
            path,
        }
    }

    /// The file is there, or was selected, deferred or quarantined before.
    fn holds_state(&self) -> bool {
        self.exists || self.once_held()
    }

    /// Selected, deferred or quarantined, whether or not the file is still there.
    fn once_held(&self) -> bool {
        self.selected || self.deferred || self.quarantined
    }
}

/// What the legacy chain decides for one selection.
#[derive(Debug, PartialEq, Eq)]
enum Choice {
    /// Take this existing file in place.
    Use(PathBuf),
    /// A live older server owns this file. Keep the new file and record nothing as selected.
    Defer(PathBuf),
    /// Nothing to take, or an earlier candidate was used and is gone: select the new file.
    Fresh,
}

/// Outcome of selecting the state for this server.
#[derive(Debug, PartialEq, Eq)]
enum Selection {
    /// This file is the state: loaded, autosaved, saved on shutdown and cleared.
    State(PathBuf),
    /// A live older server owns the legacy layout. This server reads and writes no state.
    Deferred,
}

impl Selection {
    /// The one file this server may read and write. A deferred selection has none, so nothing is
    /// loaded, autosaved, saved on shutdown or cleared until the next start.
    fn writable_path(self) -> Option<PathBuf> {
        match self {
            Self::State(path) => Some(path),
            Self::Deferred => None,
        }
    }
}

/// Choose among the candidates in order.
///
/// A file an earlier run already selected wins over an unselected file before it. Otherwise the
/// first existing file is chosen. Before the chosen file, a candidate that was used and is now
/// gone ends the walk (`Fresh`), so a later candidate never comes back. A live older server keeps
/// an unselected legacy-socket file (`Defer`); a selected file is never deferred.
fn choose_legacy(steps: &[Step], legacy_live: bool) -> Choice {
    let chosen = steps
        .iter()
        .position(|step| step.exists && step.selected)
        .or_else(|| steps.iter().position(|step| step.exists));
    let Some(index) = chosen else {
        return Choice::Fresh;
    };
    if steps[..index]
        .iter()
        .any(|step| !step.exists && step.once_held())
    {
        return Choice::Fresh;
    }
    let step = &steps[index];
    if legacy_live && step.from_legacy_socket && !step.selected {
        Choice::Defer(step.path.clone())
    } else {
        Choice::Use(step.path.clone())
    }
}

/// Candidates in priority order. The legacy-socket pair comes first and is present only for the
/// default socket; `tmuxlike/<file>` is the directory fallback.
fn legacy_steps(base: &Path, file: &str, legacy: Option<&str>) -> Vec<Step> {
    let mut steps = Vec::with_capacity(3);
    if let Some(legacy) = legacy {
        steps.push(Step::read(base.join("crew").join(legacy), true));
        steps.push(Step::read(base.join("tmuxlike").join(legacy), true));
    }
    steps.push(Step::read(base.join("tmuxlike").join(file), false));
    steps
}

/// The legacy decision for one selection: `None` once the new file has been used, otherwise the
/// chain's choice. `legacy_live` is asked only when an unselected legacy-socket file exists.
fn legacy_choice(
    base: &Path,
    file: &str,
    legacy: Option<&str>,
    legacy_live: &dyn Fn() -> bool,
) -> Option<Choice> {
    if Step::read(base.join("crew").join(file), false).holds_state() {
        return None;
    }
    let steps = legacy_steps(base, file, legacy);
    let needs_probe = steps
        .iter()
        .any(|step| step.exists && step.from_legacy_socket && !step.selected);
    let live = needs_probe && legacy_live();
    Some(choose_legacy(&steps, live))
}

/// Config can fall back whenever a file is absent, but state must remember a migration:
/// deleting or quarantining the new state is not permission to resurrect an old session.
///
/// Until the new file has been used, an existing legacy file is taken in place (see
/// `choose_legacy`). `legacy_live` is asked only when an unselected legacy-socket file exists.
/// A deferred result writes only its `.crew-deferred` marker, never state data.
fn default_state_path(
    base: &Path,
    file: &str,
    legacy: Option<&str>,
    legacy_live: &dyn Fn() -> bool,
) -> io::Result<Selection> {
    let new = base.join("crew").join(file);
    match legacy_choice(base, file, legacy, legacy_live) {
        Some(Choice::Use(old)) => {
            mark(&old, SELECTED)?;
            Ok(Selection::State(old))
        }
        Some(Choice::Defer(seen)) => {
            mark(&seen, DEFERRED)?;
            Ok(Selection::Deferred)
        }
        // Create the marker before clear/quarantine can remove the chosen state. The empty
        // marker holds no session data and deliberately remains after kill-server.
        Some(Choice::Fresh) | None => {
            mark(&new, SELECTED)?;
            Ok(Selection::State(new))
        }
    }
}

/// Whether this process's state selection defers to a live older server: the default socket is
/// in use, an unselected legacy-socket file exists, and the older endpoint is live. The CLI asks
/// this before it starts a server, so the warning appears only when deferral will happen.
pub(crate) fn legacy_deferral_expected() -> bool {
    let Some(legacy) = legacy_state_file_name() else {
        return false;
    };
    let Some(dir) = dirs::data_local_dir() else {
        return false;
    };
    let file = state_file_name(&ipc::base_name());
    matches!(
        legacy_choice(&dir, &file, Some(&legacy), &legacy_socket_live),
        Some(Choice::Defer(_))
    )
}

/// Create an empty marker next to `path`, unless it already exists.
fn mark(path: &Path, suffix: &str) -> io::Result<()> {
    let marker = with_suffix(path, suffix);
    if marker.exists() {
        return Ok(());
    }
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(file) => file.sync_all(),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// Keep only `[A-Za-z0-9._-]`; every other char becomes `_`.
fn sanitize_file_part(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Called by the render loop every tick. Writes only when the snapshot changed.
pub fn autosave(st: &State, last: &mut Option<String>) {
    autosave_at(state_path().as_deref(), st, last);
}

/// `autosave` with its path already resolved. `None` means no state may be written.
fn autosave_at(path: Option<&Path>, st: &State, last: &mut Option<String>) {
    let Some(path) = path else { return };
    let snap = snapshot::current_snapshot(st);
    if snap.sessions.is_empty() {
        if last.as_deref() != Some("") {
            remove_state_file(path);
            *last = Some(String::new());
        }
        return;
    }
    let json = match snapshot::serialize(&snap) {
        Ok(j) => j,
        Err(e) => {
            crate::log!("autosave: serialize failed: {e}");
            return;
        }
    };
    if last.as_deref() == Some(json.as_str()) {
        return;
    }
    match write_atomic(path, &json) {
        Ok(()) => *last = Some(json),
        Err(e) => crate::log!("autosave: write {} failed: {e}", path.display()),
    }
}

/// Write the current snapshot immediately (used by `server stop`).
pub fn save_now(st: &State) {
    save_now_at(state_path().as_deref(), st);
}

/// `save_now` with its path already resolved. `None` means no state may be written.
fn save_now_at(path: Option<&Path>, st: &State) {
    let snap = snapshot::current_snapshot(st);
    if snap.sessions.is_empty() {
        return;
    }
    let Some(path) = path else { return };
    match snapshot::serialize(&snap)
        .map_err(|e| e.to_string())
        .and_then(|j| write_atomic(path, &j).map_err(|e| e.to_string()))
    {
        Ok(()) => crate::log!("state saved to {}", path.display()),
        Err(e) => crate::log!("save_now: {} failed: {e}", path.display()),
    }
}

/// Delete the state file (used by `kill-server`).
pub fn clear() {
    clear_at(state_path().as_deref());
}

/// `clear` with its path already resolved. `None` means there is nothing to delete.
fn clear_at(path: Option<&Path>) {
    if let Some(path) = path {
        remove_state_file(path);
    }
}

/// Restore the saved layout into an empty server. Returns the number of sessions restored.
///
/// The whole file is read as bytes, then parsed and validated before anything is spawned. Any
/// read or parse failure moves the file to `<file>.bad`, so later autosaves cannot delete it.
/// When only some sessions come back, the original is kept as `<file>.partial.bad`.
pub fn restore_on_start(shared: &Shared) -> Result<usize, String> {
    restore_from(state_path().as_deref(), shared)
}

/// `restore_on_start` with its path already resolved. `None` restores nothing and touches no file.
fn restore_from(path: Option<&Path>, shared: &Shared) -> Result<usize, String> {
    let Some(path) = path else {
        return Ok(0);
    };
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(quarantine(path, &format!("read failed: {e}"))),
    };
    let mut snap = snapshot::decode_snapshot(&bytes).map_err(|e| quarantine(path, &e))?;
    snapshot::assign_legacy_ids(&mut snap).map_err(|e| quarantine(path, &e))?;
    let mut guard = shared.lock().map_err(|e| format!("state lock: {e}"))?;
    let st: &mut State = &mut guard;
    st.next_workspace_id = st.next_workspace_id.max(snap.next_workspace_id);
    st.next_tab_id = st.next_tab_id.max(snap.next_tab_id);
    let mut names = live_agent_names(st);
    let mut attempted = 0;
    let mut restored = 0;
    for s in &snap.sessions {
        if st.sidx(&s.name).is_some() {
            crate::log!("restore: session {} already exists, skipped", s.name);
            continue;
        }
        attempted += 1;
        match restore_session(st, shared, s, &mut names) {
            Ok(()) => restored += 1,
            Err(e) => crate::log!("restore: session {} failed: {e}", s.name),
        }
    }
    if attempted > 0 && restored == 0 {
        // Keep the snapshot: an empty server would make autosave delete it.
        return Err(quarantine(path, "no saved session could be restored"));
    }
    if is_partial(attempted, restored) {
        keep_partial_copy(path);
    }
    if let Some(id) = snap.focused_workspace_id
        && let Some(s) = st.sessions.iter_mut().find(|s| s.id == id)
    {
        s.last_used = Instant::now();
    }
    st.mark_dirty();
    Ok(restored)
}

/// True when some, but not all, of the attempted sessions came back.
fn is_partial(attempted: usize, restored: usize) -> bool {
    restored > 0 && restored < attempted
}

/// Copy the original state file to `<file>.partial.bad` before the next autosave overwrites it.
/// If the copy fails, the original is moved to `<file>.bad` instead, so it is not lost.
fn keep_partial_copy(path: &Path) {
    let bad = with_suffix(path, ".partial.bad");
    match fs::copy(path, &bad) {
        Ok(_) => crate::log!(
            "restore: partial restore; original kept at {}",
            bad.display()
        ),
        Err(e) => {
            crate::log!("restore: copy to {} failed: {e}", bad.display());
            let _ = quarantine(path, "partial restore; backup copy failed");
        }
    }
}

/// Agent names of panes that are already live. Restored names must not collide with them.
fn live_agent_names(st: &State) -> HashSet<String> {
    st.panes
        .values()
        .filter_map(|p| p.agent.as_ref()?.name.clone())
        .collect()
}

/// Rebuild one session. Panes spawned for it are killed again if any step fails, and the agent
/// names it claimed are released. `names` gains the names of a session that restores.
fn restore_session(
    st: &mut State,
    shared: &Shared,
    s: &SessionSnap,
    names: &mut HashSet<String>,
) -> Result<(), String> {
    let cols = s.cols.max(MIN_COLS);
    let rows = s.rows.max(MIN_ROWS);
    let mut spawned = Vec::new();
    let mut session_names = names.clone();
    let windows = match build_windows(st, shared, s, cols, rows, &mut spawned, &mut session_names) {
        Ok(w) => w,
        Err(e) => {
            for pid in spawned {
                st.kill_pane(pid);
            }
            return Err(e);
        }
    };
    *names = session_names;
    st.sessions.push(Session {
        id: s.id,
        name: s.name.clone(),
        windows,
        active: s.active_window,
        cols,
        rows,
        sidebar: s.sidebar,
        cwd: s.cwd.clone(),
        env: s.env.clone(),
        last_used: Instant::now(),
    });
    let si = st.sessions.len() - 1;
    st.relayout(si);
    st.events.record(
        "workspace.restored",
        None,
        serde_json::json!({
            "workspace_id": crate::workspace::workspace_id_str(s.id), "name": s.name,
        }),
    );
    Ok(())
}

fn build_windows(
    st: &mut State,
    shared: &Shared,
    s: &SessionSnap,
    cols: u16,
    rows: u16,
    spawned: &mut Vec<usize>,
    names: &mut HashSet<String>,
) -> Result<Vec<Window>, String> {
    let mut out = Vec::with_capacity(s.windows.len());
    for w in &s.windows {
        let mut spawn_leaf = |leaf: &LeafSnap| -> Result<usize, String> {
            if leaf
                .cwd
                .as_ref()
                .is_some_and(|cwd| !Path::new(cwd).is_dir())
            {
                return Err(format!("saved pane cwd no longer exists: {:?}", leaf.cwd));
            }
            let env = crate::workspace::merged_env(&s.env, leaf.env.clone());
            let id = st
                .spawn_pane_with_env(shared, rows, cols, leaf.cwd.as_deref(), &env)
                .map_err(|e| format!("spawn pane: {e}"))?;
            spawned.push(id);
            apply_leaf(st, id, leaf, names);
            Ok(id)
        };
        let layout = snapshot::rebuild(&w.layout, &mut spawn_leaf)?;
        let mut leaves = vec![];
        layout.leaves(&mut leaves);
        let active = *leaves
            .get(w.active)
            .ok_or_else(|| format!("window {:?}: active pane out of range", w.name))?;
        out.push(Window {
            id: w.id,
            name: w.name.clone(),
            manual_name: w.manual_name,
            layout,
            active,
            zoomed: w.zoomed,
        });
    }
    if out.is_empty() {
        return Err(format!("session {:?} has no windows", s.name));
    }
    Ok(out)
}

/// Apply saved label and agent data to a freshly spawned pane, and type its resume command.
fn apply_leaf(st: &mut State, id: usize, leaf: &LeafSnap, names: &mut HashSet<String>) {
    let Some(p) = st.panes.get_mut(&id) else {
        return;
    };
    p.label = leaf.label.clone();
    let Some(kind) = leaf.agent.clone() else {
        return;
    };
    p.agent = Some(AgentInfo {
        instance: 0,
        source: crate::api::RESTORE_SOURCE.into(),
        agent: kind,
        state: Status::Unknown,
        message: None,
        seq: None,
        name: snapshot::keep_agent_name(leaf.agent_name.as_deref(), names),
        resume: leaf.resume.clone(),
    });
    if leaf.resume.is_empty() {
        return;
    }
    match resume::quote_resume(&leaf.resume) {
        Some(line) => st.write_pane(id, format!("{line}\r").as_bytes()),
        None => crate::log!("restore: pane {id}: resume command rejected"),
    }
}

/// Write to `<path>.tmp`, fsync it, then rename over `path`.
fn write_atomic(path: &Path, data: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(path, ".tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Move an invalid snapshot to `<path>.bad`. Returns the error message to report.
fn quarantine(path: &Path, reason: &str) -> String {
    let bad = with_suffix(path, ".bad");
    match fs::rename(path, &bad) {
        Ok(()) => {
            crate::log!("restore: {reason}; moved to {}", bad.display());
            format!("{reason}; moved to {}", bad.display())
        }
        Err(e) => {
            crate::log!("restore: {reason}; move to {} failed: {e}", bad.display());
            format!("{reason}; could not move to {}: {e}", bad.display())
        }
    }
}

fn remove_state_file(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => crate::log!("remove {} failed: {e}", path.display()),
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}
