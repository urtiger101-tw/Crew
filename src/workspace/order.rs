//! Reorder or transfer existing tab objects without replacing their PTYs or occupants.
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

/// Move one existing tab to another workspace. All fallible validation precedes mutation.
/// The destination keeps its selected tab; a source that loses its selected tab uses its
/// next neighbour (or previous neighbour when moving the last position).
pub(super) fn transfer(
    st: &mut State,
    source: usize,
    from: usize,
    destination: usize,
    insert_index: Option<usize>,
) -> Result<Value, ApiError> {
    let insert_index = insert_index.unwrap_or(st.sessions[destination].windows.len());
    if source == destination {
        return tab(st, source, from, insert_index);
    }
    let target = &st.sessions[destination];
    if insert_index > target.windows.len() {
        return Err(ApiError::invalid(format!(
            "insert_index must be in 0..={}; got {insert_index}",
            target.windows.len()
        )));
    }
    let moving = &st.sessions[source].windows[from];
    // Automatically named shells may already share a name, just as with tab create.
    // User-assigned names retain the existing per-workspace uniqueness rule.
    if moving.manual_name && target.windows.iter().any(|w| w.name == moving.name) {
        return Err(ApiError::new(
            "name_taken",
            "tab name is already in use in the destination workspace; rename it first",
        ));
    }
    let source_id = st.sessions[source].id;
    let destination_id = target.id;
    let source_name = st.sessions[source].name.clone();
    let destination_name = target.name.clone();
    let tab_id = moving.id;
    let panes = moving.panes();
    let source_was_focused = st.target_session(None).ok() == Some(source);
    let clients = remember_clients(st);

    let source_session = &mut st.sessions[source];
    let moving = source_session.windows.remove(from);
    let source_closed = source_session.windows.is_empty();
    if !source_closed {
        source_session.active = if from < source_session.active {
            source_session.active - 1
        } else {
            source_session.active.min(source_session.windows.len() - 1)
        };
    }
    let destination_session = &mut st.sessions[destination];
    // Insertion shifts the index but never selects the incoming tab.
    if insert_index <= destination_session.active {
        destination_session.active += 1;
    }
    destination_session.windows.insert(insert_index, moving);

    let mut destination = destination;
    if source_closed {
        // Do not use kill_session: every pane now belongs to the destination.
        st.sessions.remove(source);
        destination -= usize::from(source < destination);
        if source_was_focused {
            st.sessions[destination].last_used = std::time::Instant::now();
        }
        for client in st.clients.values_mut().filter(|c| c.session == source_name) {
            client.session = destination_name.clone();
        }
    }
    for client in st.clients.values_mut() {
        if client.session == source_name
            || client.session == destination_name
            || client.drag.is_some_and(|(pid, ..)| panes.contains(&pid))
        {
            // A drag began against the old workspace geometry.
            client.drag = None;
        }
    }
    restore_clients(st, clients);
    st.relayout(destination);
    if !source_closed {
        st.relayout(source);
    }

    let destination_tabs: Vec<_> = (0..st.sessions[destination].windows.len())
        .map(|wi| tab_json(st, destination, wi))
        .collect();
    let source_tabs: Vec<_> = if source_closed {
        Vec::new()
    } else {
        (0..st.sessions[source].windows.len())
            .map(|wi| tab_json(st, source, wi))
            .collect()
    };
    let event = json!({
        "workspace_id": workspace_id_str(destination_id), "tab_id": tab_id_str(tab_id),
        "from_workspace_id": workspace_id_str(source_id),
        "to_workspace_id": workspace_id_str(destination_id),
        "insert_index": insert_index, "from_index": from, "to_index": insert_index,
        "source_workspace_closed": source_closed,
        "source_tab_ids": source_tabs.iter().map(|t| &t["tab_id"]).collect::<Vec<_>>(),
        "tab_ids": destination_tabs.iter().map(|t| &t["tab_id"]).collect::<Vec<_>>(),
    });
    st.events.record("tab.moved", None, event.clone());
    if source_closed {
        st.events.record("workspace.closed", None, json!({
            "workspace_id": workspace_id_str(source_id), "name": source_name,
            "reason": "last_tab_transferred", "to_workspace_id": workspace_id_str(destination_id),
        }));
        // Remove the closed workspace's metadata immediately. Live pane tokens remain.
        crate::metadata::sweep(st, std::time::Instant::now());
    }
    st.mark_dirty();
    let mut reply = event;
    reply["moved"] = json!(true);
    reply["tabs"] = json!(destination_tabs);
    reply["source_tabs"] = json!(source_tabs);
    Ok(reply)
}
