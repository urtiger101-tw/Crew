//! Stable workspace/tab automation over the existing session/window model.
//! CLI and raw API requests share validation; directory checks precede creation.
mod cli;
#[cfg(test)]
mod tests;
use crate::api::{ApiError, ApiResult, Reply, lock, pane_id_str, pane_json};
use crate::server::{Shared, State};
pub use cli::parse;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Instant;

pub const HELP: &str = r#"WORKSPACES AND TABS (stable wN / tN identifiers):
  crew workspace list|get [ID|NAME]
  crew workspace create [NAME] [--cwd PATH] [--env KEY=VALUE] [--command TEXT] [--focus]
  crew workspace rename ID|NAME NEW_NAME
  crew workspace focus|close ID|NAME
  crew tab list|get [ID|NAME] [--workspace ID|NAME]
  crew tab create [NAME] [--workspace ID|NAME] [--cwd PATH] [--env KEY=VALUE]
                  [--command TEXT] [--focus|--no-focus]
  crew tab rename ID|NAME NEW_NAME [--workspace ID|NAME]
  crew tab focus|close ID|NAME [--workspace ID|NAME]
Creation defaults to --no-focus. A workspace defaults to the caller's current directory;
a tab inherits its workspace's directory and environment. --env can be repeated.
IDs survive rename, unrelated deletion and restart. Processes do not survive a restart.
Names are exact, including periods and colons. Use name:NAME for legacy ID-shaped names.
"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    WorkspaceList,
    WorkspaceGet {
        workspace: Option<String>,
        current: Option<usize>,
    },
    WorkspaceCreate {
        name: Option<String>,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
        command: Option<String>,
        focus: bool,
    },
    WorkspaceRename {
        workspace: String,
        name: String,
    },
    WorkspaceFocus {
        workspace: String,
    },
    WorkspaceClose {
        workspace: String,
    },
    TabList {
        workspace: Option<String>,
        current: Option<usize>,
    },
    TabGet {
        tab: Option<String>,
        workspace: Option<String>,
        current: Option<usize>,
    },
    TabCreate {
        workspace: Option<String>,
        name: Option<String>,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
        command: Option<String>,
        focus: bool,
        current: Option<usize>,
    },
    TabRename {
        tab: String,
        workspace: Option<String>,
        name: String,
        current: Option<usize>,
    },
    TabFocus {
        tab: String,
        workspace: Option<String>,
        current: Option<usize>,
    },
    TabClose {
        tab: String,
        workspace: Option<String>,
        current: Option<usize>,
    },
}

pub fn starts_server(req: &Request) -> bool {
    matches!(req, Request::WorkspaceCreate { .. })
}
pub fn workspace_id_str(id: usize) -> String {
    format!("w{id}")
}
pub fn tab_id_str(id: usize) -> String {
    format!("t{id}")
}

fn parse_id(raw: &str, prefix: char) -> Option<usize> {
    let digits = raw.strip_prefix(prefix)?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn id_shaped(raw: &str, prefix: char) -> bool {
    raw.strip_prefix(prefix)
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

fn valid_name(raw: &str, prefix: char) -> Result<(), ApiError> {
    if raw.trim().is_empty()
        || raw != raw.trim()
        || raw.chars().count() > 80
        || raw.chars().any(char::is_control)
        || id_shaped(raw, prefix)
        || raw.starts_with("name:")
    {
        return Err(ApiError::invalid(
            "name must be 1..80 characters without surrounding whitespace or controls; ID-shaped names and the name: prefix are reserved",
        ));
    }
    Ok(())
}

pub(crate) fn validate_env(env: &BTreeMap<String, String>) -> Result<(), ApiError> {
    let mut keys = std::collections::HashSet::new();
    for (key, value) in env {
        if cfg!(windows) && !keys.insert(key.to_ascii_uppercase()) {
            return Err(ApiError::invalid(
                "duplicate case-insensitive environment key",
            ));
        }
        let mut bytes = key.bytes();
        if !bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.contains('\0')
        {
            return Err(ApiError::invalid(
                "environment keys must use [A-Za-z_][A-Za-z0-9_]* and values must not contain NUL",
            ));
        }
        if matches!(
            key.to_ascii_uppercase().as_str(),
            "CREW"
                | "CREW_PANE_ID"
                | "CREW_SOCKET"
                | "TMUXLIKE"
                | "TMUXLIKE_PANE_ID"
                | "TMUXLIKE_SOCKET"
        ) {
            return Err(ApiError::invalid(format!(
                "reserved environment key: {key}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn merged_env(
    base: &BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    merge_env_for(base, overrides, cfg!(windows))
}

fn merge_env_for(
    base: &BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
    case_insensitive: bool,
) -> BTreeMap<String, String> {
    let mut merged = base.clone();
    for (key, value) in overrides {
        if case_insensitive {
            merged.retain(|old, _| !old.eq_ignore_ascii_case(&key));
        }
        merged.insert(key, value);
    }
    merged
}

fn checked_cwd(raw: Option<String>) -> Result<Option<String>, ApiError> {
    raw.map(|raw| {
        let path = std::fs::canonicalize(&raw)
            .map_err(|e| ApiError::invalid(format!("cannot use cwd {raw:?}: {e}")))?;
        if !path.is_dir() {
            return Err(ApiError::invalid(format!(
                "cwd is not a directory: {raw:?}"
            )));
        }
        Ok(path.to_string_lossy().into_owned())
    })
    .transpose()
}

fn check_command(command: Option<&str>) -> Result<(), ApiError> {
    if command.is_some_and(|c| c.trim().is_empty() || c.contains('\0')) {
        return Err(ApiError::invalid(
            "command must not be empty or contain NUL",
        ));
    }
    Ok(())
}

/// Exact names, or stable ids. name:NAME disambiguates legacy names such as w1.
pub fn workspace_index(st: &State, target: &str) -> Result<usize, ApiError> {
    let found = if let Some(name) = target.strip_prefix("name:") {
        st.sidx(name)
    } else if let Some(id) = parse_id(target, 'w') {
        st.sessions.iter().position(|s| s.id == id)
    } else {
        st.sidx(target)
    };
    found.ok_or_else(|| {
        ApiError::new(
            "workspace_not_found",
            format!("workspace not found: {target}"),
        )
    })
}

fn selected_workspace(
    st: &State,
    target: Option<&str>,
    current: Option<usize>,
) -> Result<usize, ApiError> {
    if let Some(target) = target {
        return workspace_index(st, target);
    }
    if let Some(pid) = current {
        return st
            .locate(pid)
            .map(|(si, _)| si)
            .ok_or_else(|| ApiError::pane_not_found(pid));
    }
    st.target_session(None)
        .map_err(|_| ApiError::new("workspace_not_found", "no workspaces"))
}

fn tab_index(
    st: &State,
    target: Option<&str>,
    workspace: Option<&str>,
    current: Option<usize>,
) -> Result<(usize, usize), ApiError> {
    let scope = workspace.map(|w| workspace_index(st, w)).transpose()?;
    if target.is_none()
        && scope.is_none()
        && let Some(pid) = current
    {
        return st.locate(pid).ok_or_else(|| ApiError::pane_not_found(pid));
    }
    if let Some(target) = target
        && let Some(id) = parse_id(target, 't')
    {
        return st
            .sessions
            .iter()
            .enumerate()
            .filter(|(si, _)| scope.is_none_or(|scope| scope == *si))
            .find_map(|(si, s)| s.windows.iter().position(|w| w.id == id).map(|wi| (si, wi)))
            .ok_or_else(|| ApiError::new("tab_not_found", format!("tab not found: {target}")));
    }
    let si = scope.map_or_else(|| selected_workspace(st, None, current), Ok)?;
    let Some(target) = target else {
        return Ok((si, st.sessions[si].active));
    };
    let name = target.strip_prefix("name:").unwrap_or(target);
    let matches: Vec<usize> = st.sessions[si]
        .windows
        .iter()
        .enumerate()
        .filter_map(|(wi, w)| (w.name == name).then_some(wi))
        .collect();
    match matches.as_slice() {
        [wi] => Ok((si, *wi)),
        [] => Err(ApiError::new(
            "tab_not_found",
            format!("tab not found: {target}"),
        )),
        _ => Err(ApiError::new(
            "ambiguous_target",
            "several tabs have this name; use a tN id",
        )),
    }
}

pub fn tab_json(st: &State, si: usize, wi: usize) -> Value {
    let s = &st.sessions[si];
    let w = &s.windows[wi];
    json!({
        "tab_id": tab_id_str(w.id), "workspace_id": workspace_id_str(s.id),
        "name": w.name, "index": wi, "active": s.active == wi,
        "active_pane": pane_id_str(w.active), "zoomed": w.zoomed,
        "panes": w.panes().iter().filter_map(|pid| pane_json(st, *pid)).collect::<Vec<_>>(),
    })
}

pub fn workspace_json(st: &State, si: usize) -> Value {
    let s = &st.sessions[si];
    json!({
        "workspace_id": workspace_id_str(s.id), "name": s.name, "cwd": s.cwd,
        "env_keys": s.env.keys().collect::<Vec<_>>(),
        "active_tab": s.windows.get(s.active).map(|w| tab_id_str(w.id)),
        "focused": st.target_session(None).ok() == Some(si),
        "attached": st.clients.values().filter(|c| c.session == s.name).count(),
        "cols": s.cols, "rows": s.rows,
        "tabs": (0..s.windows.len()).map(|wi| tab_json(st, si, wi)).collect::<Vec<_>>(),
    })
}

fn recent_name(st: &State) -> Option<String> {
    st.target_session(None)
        .ok()
        .map(|si| st.sessions[si].name.clone())
}

/// Only clients viewing the previous current workspace move. Independent sessions stay put.
fn focus_workspace(st: &mut State, si: usize, previous: Option<String>) {
    let clients: Vec<usize> = st
        .clients
        .iter()
        .filter_map(|(id, c)| (Some(c.session.as_str()) == previous.as_deref()).then_some(*id))
        .collect();
    for cid in clients {
        st.switch_session(cid, si);
    }
    st.sessions[si].last_used = Instant::now();
    st.mark_dirty();
    st.events.record(
        "workspace.focused",
        None,
        json!({"workspace_id": workspace_id_str(st.sessions[si].id)}),
    );
}

pub fn handle(shared: &Shared, req: Request) -> ApiResult {
    match req {
        Request::WorkspaceCreate {
            name,
            cwd,
            env,
            command,
            focus,
        } => {
            if let Some(name) = &name {
                valid_name(name, 'w')?;
            }
            validate_env(&env)?;
            check_command(command.as_deref())?;
            let cwd = checked_cwd(cwd)?;
            let (reply, pid) = {
                let mut st = lock(shared);
                if name.as_ref().is_some_and(|name| st.sidx(name).is_some()) {
                    return Err(ApiError::new(
                        "name_taken",
                        "workspace name is already in use",
                    ));
                }
                let previous = recent_name(&st);
                let size = st
                    .target_session(None)
                    .ok()
                    .map(|si| (st.sessions[si].cols, st.sessions[si].rows))
                    .unwrap_or((120, 30));
                let name = st
                    .new_workspace(shared, name, size, cwd, env, focus)
                    .map_err(|e| ApiError::new("spawn_failed", e.to_string()))?;
                let si = st
                    .sidx(&name)
                    .ok_or_else(|| ApiError::new("internal", "created workspace missing"))?;
                if focus {
                    focus_workspace(&mut st, si, previous);
                }
                st.mark_dirty();
                (
                    json!({"workspace": workspace_json(&st, si)}),
                    st.active_pane(si),
                )
            };
            if let Some(command) = command {
                crate::api::pane::submit(shared, pid, &command, 30)?;
            }
            Ok(Reply::Json(reply))
        }
        Request::TabCreate {
            workspace,
            name,
            cwd,
            env,
            command,
            focus,
            current,
        } => {
            if let Some(name) = &name {
                valid_name(name, 't')?;
            }
            validate_env(&env)?;
            check_command(command.as_deref())?;
            let cwd = checked_cwd(cwd)?;
            let (reply, pid) = {
                let mut st = lock(shared);
                let si = selected_workspace(&st, workspace.as_deref(), current)?;
                if name
                    .as_ref()
                    .is_some_and(|name| st.sessions[si].windows.iter().any(|w| &w.name == name))
                {
                    return Err(ApiError::new(
                        "name_taken",
                        "tab name is already in use in this workspace",
                    ));
                }
                if cwd.is_none()
                    && st.sessions[si]
                        .cwd
                        .as_ref()
                        .is_some_and(|p| !std::path::Path::new(p).is_dir())
                {
                    return Err(ApiError::invalid(
                        "workspace cwd no longer exists; provide --cwd",
                    ));
                }
                let previous = recent_name(&st);
                let tid = st
                    .new_tab(shared, si, name, cwd, env, focus)
                    .map_err(|e| ApiError::new("spawn_failed", e.to_string()))?;
                let wi = st.sessions[si]
                    .windows
                    .iter()
                    .position(|w| w.id == tid)
                    .ok_or_else(|| ApiError::new("internal", "created tab missing"))?;
                if focus {
                    focus_workspace(&mut st, si, previous);
                }
                st.mark_dirty();
                (
                    json!({"tab": tab_json(&st, si, wi)}),
                    st.sessions[si].windows[wi].active,
                )
            };
            if let Some(command) = command {
                crate::api::pane::submit(shared, pid, &command, 30)?;
            }
            Ok(Reply::Json(reply))
        }
        other => handle_existing(shared, other),
    }
}

fn handle_existing(shared: &Shared, req: Request) -> ApiResult {
    let mut st = lock(shared);
    let result = match req {
        Request::WorkspaceList => {
            json!({"workspaces": (0..st.sessions.len()).map(|si| workspace_json(&st, si)).collect::<Vec<_>>()})
        }
        Request::WorkspaceGet { workspace, current } => {
            let si = selected_workspace(&st, workspace.as_deref(), current)?;
            json!({"workspace": workspace_json(&st, si)})
        }
        Request::WorkspaceRename { workspace, name } => {
            valid_name(&name, 'w')?;
            let si = workspace_index(&st, &workspace)?;
            if st.sidx(&name).is_some_and(|other| other != si) {
                return Err(ApiError::new(
                    "name_taken",
                    "workspace name is already in use",
                ));
            }
            let old = std::mem::replace(&mut st.sessions[si].name, name.clone());
            for c in st.clients.values_mut().filter(|c| c.session == old) {
                c.session = name.clone();
                c.frame.invalidate();
            }
            let id = st.sessions[si].id;
            st.events.record(
                "workspace.renamed",
                None,
                json!({"workspace_id": workspace_id_str(id), "name": name}),
            );
            st.mark_dirty();
            json!({"workspace": workspace_json(&st, si)})
        }
        Request::WorkspaceFocus { workspace } => {
            let si = workspace_index(&st, &workspace)?;
            let previous = recent_name(&st);
            focus_workspace(&mut st, si, previous);
            json!({"workspace": workspace_json(&st, si)})
        }
        Request::WorkspaceClose { workspace } => {
            let si = workspace_index(&st, &workspace)?;
            let id = st.sessions[si].id;
            st.kill_session(si);
            json!({"closed": true, "workspace_id": workspace_id_str(id)})
        }
        Request::TabList { workspace, current } => {
            let si = selected_workspace(&st, workspace.as_deref(), current)?;
            json!({"tabs": (0..st.sessions[si].windows.len()).map(|wi| tab_json(&st, si, wi)).collect::<Vec<_>>()})
        }
        Request::TabGet {
            tab,
            workspace,
            current,
        } => {
            let (si, wi) = tab_index(&st, tab.as_deref(), workspace.as_deref(), current)?;
            json!({"tab": tab_json(&st, si, wi)})
        }
        Request::TabRename {
            tab,
            workspace,
            name,
            current,
        } => {
            valid_name(&name, 't')?;
            let (si, wi) = tab_index(&st, Some(&tab), workspace.as_deref(), current)?;
            if st.sessions[si]
                .windows
                .iter()
                .enumerate()
                .any(|(i, w)| i != wi && w.name == name)
            {
                return Err(ApiError::new(
                    "name_taken",
                    "tab name is already in use in this workspace",
                ));
            }
            let w = &mut st.sessions[si].windows[wi];
            w.name = name.clone();
            w.manual_name = true;
            let id = w.id;
            st.events.record(
                "tab.renamed",
                None,
                json!({"tab_id": tab_id_str(id), "name": name}),
            );
            st.mark_dirty();
            json!({"tab": tab_json(&st, si, wi)})
        }
        Request::TabFocus {
            tab,
            workspace,
            current,
        } => {
            let (si, wi) = tab_index(&st, Some(&tab), workspace.as_deref(), current)?;
            let previous = recent_name(&st);
            let pid = st.sessions[si].windows[wi].active;
            st.focus_pane(pid);
            focus_workspace(&mut st, si, previous);
            let id = st.sessions[si].windows[wi].id;
            st.events
                .record("tab.focused", Some(pid), json!({"tab_id": tab_id_str(id)}));
            json!({"tab": tab_json(&st, si, wi)})
        }
        Request::TabClose {
            tab,
            workspace,
            current,
        } => {
            let (si, wi) = tab_index(&st, Some(&tab), workspace.as_deref(), current)?;
            let w = &st.sessions[si].windows[wi];
            let id = w.id;
            let panes = w.panes();
            for pid in panes {
                st.kill_pane(pid);
            }
            json!({"closed": true, "tab_id": tab_id_str(id)})
        }
        Request::WorkspaceCreate { .. } | Request::TabCreate { .. } => {
            return Err(ApiError::new(
                "internal",
                "create request routed incorrectly",
            ));
        }
    };
    Ok(Reply::Json(result))
}
