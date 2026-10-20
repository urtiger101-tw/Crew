//! Ordering changes vector positions only. PTYs, stable IDs and agent occupants stay put.
use super::{tab_id_str, tab_json, workspace_id_str, workspace_json};
use crate::api::ApiError;
use crate::commands::{self, Cmd};
use crate::render::{Entry, Filter, pack_chooser, unpack_chooser, visible_entries};
use crate::server::{PromptKind, State};
use serde_json::{Value, json};

/// Slots refer to the original sequence, including the slot after its last item.
fn destination(len: usize, source: usize, insert_index: usize) -> Result<usize, ApiError> {
    if insert_index > len {
        return Err(ApiError::invalid(format!(
            "insert_index must be in 0..={len}; got {insert_index}"
        )));
    }
    Ok(insert_index - usize::from(source < insert_index))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowTarget {
    Workspace(usize),
    Tab(usize),
    Pane(usize),
}

fn row_target(st: &State, (si, wi, pi): Entry) -> Option<RowTarget> {
    let s = st.sessions.get(si)?;
    let Some(wi) = wi else {
        return Some(RowTarget::Workspace(s.id));
    };
    let w = s.windows.get(wi)?;
    match pi {
        Some(pi) => w.panes().get(pi).copied().map(RowTarget::Pane),
        None => Some(RowTarget::Tab(w.id)),
    }
}

struct ClientTargets {
    client: usize,
    chooser: Option<(Filter, Option<RowTarget>)>,
    confirm_tab: Option<usize>,
}

/// The chooser and close confirmation use display positions, so retain their meaning too.
fn remember_clients(st: &State) -> Vec<ClientTargets> {
    st.clients
        .iter()
        .map(|(&client, c)| {
            let chooser = c.chooser.map(|raw| {
                let (sel, filter) = unpack_chooser(raw);
                let entries = visible_entries(st, filter);
                let target = entries
                    .get(sel.min(entries.len().saturating_sub(1)))
                    .and_then(|&entry| row_target(st, entry));
                (filter, target)
            });
            let confirm_tab = c
                .prompt
                .as_ref()
                .filter(|p| p.kind == PromptKind::Confirm)
                .and_then(|p| p.pending.as_deref())
                .and_then(|command| match commands::parse(command).ok()? {
                    Cmd::KillWindow { target } => {
                        let (si, wi) =
                            st.window_for(st.sidx(&c.session), target.as_deref()).ok()?;
                        Some(st.sessions[si].windows[wi].id)
                    }
                    _ => None,
                });
            ClientTargets {
                client,
                chooser,
                confirm_tab,
            }
        })
        .collect()
}

fn restore_clients(st: &mut State, targets: Vec<ClientTargets>) {
    for saved in targets {
        let chooser = saved.chooser.map(|(filter, target)| {
            target.and_then(|target| {
                visible_entries(st, filter)
                    .iter()
                    .position(|&entry| row_target(st, entry) == Some(target))
                    .map(|sel| pack_chooser(sel, filter))
            })
        });
        let pending = saved.confirm_tab.and_then(|id| {
            st.sessions.iter().find_map(|s| {
                s.windows
                    .iter()
                    .position(|w| w.id == id)
                    .map(|wi| commands::kill_window_command(&s.name, wi))
            })
        });
        if let Some(c) = st.clients.get_mut(&saved.client) {
            if let Some(chooser) = chooser {
                c.chooser = chooser;
            }
            if let (Some(prompt), Some(pending)) = (c.prompt.as_mut(), pending) {
                prompt.pending = Some(pending);
            }
            c.frame.invalidate();
        }
    }
}

pub(super) fn workspace(
    st: &mut State,
    from: usize,
    insert_index: usize,
) -> Result<Value, ApiError> {
    let to = destination(st.sessions.len(), from, insert_index)?;
    let id = workspace_id_str(st.sessions[from].id);
    let moved = from != to;
    if moved {
        let clients = remember_clients(st);
        let workspace = st.sessions.remove(from);
        st.sessions.insert(to, workspace);
        restore_clients(st, clients);
        st.events.record("workspace.moved", None, json!({
            "workspace_id": id, "insert_index": insert_index,
            "from_index": from, "to_index": to,
            "workspace_ids": st.sessions.iter().map(|s| workspace_id_str(s.id)).collect::<Vec<_>>(),
        }));
        st.mark_dirty();
    }
    Ok(json!({
        "moved": moved, "workspace_id": id, "insert_index": insert_index,
        "from_index": from, "to_index": to,
        "workspaces": (0..st.sessions.len()).map(|si| workspace_json(st, si)).collect::<Vec<_>>(),
    }))
}

pub(super) fn tab(
    st: &mut State,
    si: usize,
    from: usize,
    insert_index: usize,
) -> Result<Value, ApiError> {
    let to = destination(st.sessions[si].windows.len(), from, insert_index)?;
    let wid = workspace_id_str(st.sessions[si].id);
    let tid = tab_id_str(st.sessions[si].windows[from].id);
    let moved = from != to;
    if moved {
        let clients = remember_clients(st);
        let s = &mut st.sessions[si];
        let active_id = s.windows[s.active].id;
        let tab = s.windows.remove(from);
        s.windows.insert(to, tab);
        s.active = s
            .windows
            .iter()
            .position(|w| w.id == active_id)
            .expect("reordering keeps the active tab");
        restore_clients(st, clients);
        st.events.record("tab.moved", None, json!({
            "workspace_id": wid, "tab_id": tid, "insert_index": insert_index,
            "from_index": from, "to_index": to,
            "tab_ids": st.sessions[si].windows.iter().map(|w| tab_id_str(w.id)).collect::<Vec<_>>(),
        }));
        st.mark_dirty();
    }
    Ok(json!({
        "moved": moved, "workspace_id": wid, "tab_id": tid,
        "insert_index": insert_index, "from_index": from, "to_index": to,
        "tabs": (0..st.sessions[si].windows.len()).map(|wi| tab_json(st, si, wi)).collect::<Vec<_>>(),
    }))
}
