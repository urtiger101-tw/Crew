use super::move_tests::{add_client, events, raw_request, request};
use super::*;
use crate::render::{Filter, unpack_chooser, visible_entries};
use crate::server::Prompt;

fn fixture() -> Shared {
    let shared = super::move_tests::fixture();
    for s in &mut lock(&shared).sessions {
        for w in &mut s.windows {
            w.name = format!("tab.{}:中文", w.id);
        }
    }
    shared
}

fn transfer(shared: &Shared, tab: &str, target: &str, index: Option<usize>) -> Value {
    raw_request(
        shared,
        json!({"TabTransfer": {
            "tab": tab, "to_workspace": target, "insert_index": index,
            "workspace": null, "current": null,
        }}),
    )
    .unwrap()
}

#[test]
fn transfer_all_destination_slots_preserves_tab_contents_and_existing_selections() {
    for index in 0..=3 {
        let shared = fixture();
        let reply = transfer(&shared, "t11", "w2", Some(index));
        assert_eq!(reply["from_workspace_id"], "w1");
        assert_eq!(reply["to_workspace_id"], "w2");
        assert_eq!(reply["source_workspace_closed"], false);
        assert_eq!(reply["to_index"], index);
        let st = lock(&shared);
        assert_eq!(
            st.sessions[0]
                .windows
                .iter()
                .map(|w| w.id)
                .collect::<Vec<_>>(),
            vec![12, 13]
        );
        assert_eq!(st.sessions[0].windows[st.sessions[0].active].id, 12);
        assert_eq!(st.sessions[1].windows[st.sessions[1].active].id, 22);
        let moved = &st.sessions[1].windows[index];
        assert_eq!((moved.id, moved.active, moved.zoomed), (11, 110, false));
        assert_eq!(moved.panes(), vec![110]);
        assert_eq!(st.locate(110), Some((1, index)));
        assert_eq!(st.sessions[st.target_session(None).unwrap()].id, 3);
        assert_eq!((st.next_workspace_id, st.next_tab_id), (3, 33));
        drop(st);
        let emitted = events(&shared);
        assert_eq!(emitted.as_array().unwrap().len(), 1);
        assert_eq!(emitted[0]["kind"], "tab.moved");
        assert_eq!(emitted[0]["data"]["source_tab_ids"], json!(["t12", "t13"]));
        assert_eq!(emitted[0]["data"]["tab_ids"], reply["tab_ids"]);
    }
}

#[test]
fn transfer_selected_tab_uses_next_or_previous_neighbour_without_losing_its_zoom() {
    for (selected, expected) in [(1, 13), (2, 12)] {
        let shared = fixture();
        lock(&shared).sessions[0].active = selected;
        transfer(&shared, &format!("t{}", 11 + selected), "w2", None);
        let st = lock(&shared);
        assert_eq!(st.sessions[0].windows[st.sessions[0].active].id, expected);
        assert_eq!(st.sessions[1].windows[st.sessions[1].active].id, 22);
        assert_eq!(st.sessions[1].windows[3].active, (11 + selected) * 10);
        assert_eq!(st.sessions[1].windows[3].zoomed, selected == 1);
    }
}

#[test]
fn transfer_to_same_workspace_keeps_original_ordering_and_noop_semantics() {
    let shared = fixture();
    let reply = transfer(&shared, "t12", "w1", Some(2));
    assert_eq!(reply["moved"], false);
    assert_eq!(events(&shared), json!([]));
    let reply = transfer(&shared, "t12", "w1", None);
    assert_eq!(reply["to_index"], 2);
    assert_eq!(reply["tab_id"], "t12");
    assert_eq!(reply["tabs"][2]["active"], true);
}

#[test]
fn transfer_rejects_invalid_destination_slot_scope_and_manual_name_before_mutation() {
    let shared = fixture();
    lock(&shared).sessions[1].windows[0].name = "tab.11:中文".into();
    let before = request(&shared, Request::WorkspaceList);
    for (payload, code) in [
        (
            json!({"tab":"t11","to_workspace":"w99","insert_index":0}),
            "workspace_not_found",
        ),
        (
            json!({"tab":"t99","to_workspace":"w2","insert_index":0}),
            "tab_not_found",
        ),
        (
            json!({"tab":"t11","workspace":"w2","to_workspace":"w3","insert_index":0}),
            "tab_not_found",
        ),
        (
            json!({"tab":"t11","to_workspace":"w2","insert_index":4}),
            "invalid_argument",
        ),
        (
            json!({"tab":"t11","to_workspace":"w2","insert_index":usize::MAX}),
            "invalid_argument",
        ),
        (
            json!({"tab":"t11","to_workspace":"w2","insert_index":0}),
            "name_taken",
        ),
        (
            json!({"tab":"tab.11:中文","to_workspace":"w2","current":999}),
            "pane_not_found",
        ),
    ] {
        assert_eq!(
            raw_request(&shared, json!({"TabTransfer":payload}))
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(request(&shared, Request::WorkspaceList), before);
        assert!(!lock(&shared).dirty);
        assert_eq!(events(&shared), json!([]));
    }
    lock(&shared).sessions[0].windows[0].manual_name = false;
    transfer(&shared, "t11", "w2", Some(0));
    assert_eq!(lock(&shared).sessions[1].windows[0].id, 11);
}

#[test]
fn transfer_current_uses_stable_pane_membership_and_preserves_exact_name_scoping() {
    let shared = fixture();
    let reply = raw_request(
        &shared,
        json!({"TabTransfer":{
            "tab":"tab.12:中文","workspace":null,"to_workspace":"project.2:中文",
            "insert_index":0,"current":120,
        }}),
    )
    .unwrap();
    assert_eq!(reply["tab_id"], "t12");
    let current = request(
        &shared,
        Request::WorkspaceGet {
            workspace: None,
            current: Some(120),
        },
    );
    assert_eq!(current["workspace"]["workspace_id"], "w2");
    let current = request(
        &shared,
        Request::TabGet {
            tab: None,
            workspace: None,
            current: Some(120),
        },
    );
    assert_eq!(current["tab"]["tab_id"], "t12");
    assert_eq!(current["tab"]["zoomed"], true);
}

#[test]
fn last_tab_transfer_removes_only_empty_workspace_in_either_index_direction() {
    for (source, target, tab) in [(0, 1, 11), (2, 0, 31)] {
        let shared = fixture();
        {
            let mut st = lock(&shared);
            st.sessions[source].windows.truncate(1);
            st.sessions[source].active = 0;
            st.sessions[source].last_used = Instant::now();
        }
        let source_id = source + 1;
        let target_id = target + 1;
        let metadata = crate::metadata::Request {
            target: crate::metadata::Target::Workspace(format!("w{source_id}")),
            source: "test".into(),
            tokens: BTreeMap::from([("branch".into(), Some("source".into()))]),
            seq: Some(5),
            ttl_ms: None,
            title: Default::default(),
            display_agent: Default::default(),
            state_labels: BTreeMap::new(),
            agent: None,
            applies_to_source: None,
            clear_presentation: false,
        };
        crate::metadata::handle(&shared, metadata).unwrap();
        let reply = transfer(&shared, &format!("t{tab}"), &format!("w{target_id}"), None);
        assert_eq!(reply["source_workspace_closed"], true);
        assert_eq!(reply["source_tabs"], json!([]));
        let st = lock(&shared);
        assert_eq!(st.sessions.len(), 2);
        assert!(st.sessions.iter().all(|s| s.id != source_id));
        assert_eq!(st.sessions[st.locate(tab * 10).unwrap().0].id, target_id);
        assert_eq!(st.sessions[st.target_session(None).unwrap()].id, target_id);
        assert!(
            st.metadata
                .values(crate::metadata::Key::Workspace(source_id), Instant::now())
                .is_empty()
        );
        assert_eq!((st.next_workspace_id, st.next_tab_id), (3, 33));
        drop(st);
        let emitted = events(&shared);
        let closed: Vec<_> = emitted
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "workspace.closed")
            .collect();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0]["data"]["reason"], "last_tab_transferred");
        assert!(
            !emitted
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "pane.closed" || e["kind"] == "tab.closed")
        );
    }
}

#[test]
fn transfer_repairs_clients_chooser_close_confirmation_and_stale_drag() {
    let shared = fixture();
    {
        let mut st = lock(&shared);
        st.sessions[0].windows.truncate(1);
        st.sessions[0].active = 0;
        add_client(&mut st, 1, (0, Some(0), None), Filter::All);
        add_client(&mut st, 2, (0, Some(0), Some(0)), Filter::All);
        add_client(&mut st, 3, (0, None, None), Filter::All);
        st.clients.get_mut(&1).unwrap().session = "project.1:中文".into();
        st.clients.get_mut(&1).unwrap().prompt = Some(Prompt::confirm(
            "close tab.11:中文?".into(),
            crate::commands::kill_window_command("project.1:中文", 0),
        ));
    }
    transfer(&shared, "t11", "w2", Some(0));
    let st = lock(&shared);
    assert_eq!(st.clients[&1].session, "project.2:中文");
    for (cid, expected) in [(1, (0, Some(0), None)), (2, (0, Some(0), Some(0)))] {
        let c = &st.clients[&cid];
        let (sel, filter) = unpack_chooser(c.chooser.unwrap());
        assert_eq!(visible_entries(&st, filter)[sel], expected);
        assert_eq!(c.drag, None);
    }
    assert!(st.clients[&3].chooser.is_none());
    let pending = st.clients[&1]
        .prompt
        .as_ref()
        .unwrap()
        .pending
        .as_ref()
        .unwrap();
    let crate::commands::Cmd::KillWindow { target } = crate::commands::parse(pending).unwrap()
    else {
        panic!("expected close")
    };
    let (si, wi) = st.window_for(st.client_si(1), target.as_deref()).unwrap();
    assert_eq!(st.sessions[si].windows[wi].id, 11);
    assert_eq!(st.sessions[si].windows[st.sessions[si].active].id, 22);
}
