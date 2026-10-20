use super::*;
use crate::layout::Node;
use crate::render::{Filter, Frame, pack_chooser, unpack_chooser, visible_entries};
use crate::server::{Client, Prompt, Session, Window};
use std::sync::{Arc, Mutex};

fn fixture() -> Shared {
    let mut st = crate::server::test_state();
    let now = Instant::now();
    st.sessions = (1..=3)
        .map(|id| Session {
            id,
            name: format!("project.{id}:中文"),
            windows: (1..=3)
                .map(|n| {
                    let tid = id * 10 + n;
                    Window {
                        id: tid,
                        name: format!("review.{n}:中文"),
                        manual_name: true,
                        layout: Node::Leaf(tid * 10),
                        active: tid * 10,
                        zoomed: n == 2,
                    }
                })
                .collect(),
            active: 1,
            cols: 120,
            rows: 30,
            sidebar: true,
            cwd: None,
            env: BTreeMap::new(),
            last_used: now,
        })
        .collect();
    st.next_workspace_id = 3;
    st.next_tab_id = 33;
    Arc::new(Mutex::new(st))
}

fn request(shared: &Shared, req: Request) -> Value {
    let Reply::Json(value) = handle(shared, req).unwrap() else {
        panic!("expected JSON");
    };
    value
}

fn raw_request(shared: &Shared, value: Value) -> Result<Value, ApiError> {
    let req = serde_json::from_value(value).unwrap();
    let Reply::Json(value) = handle(shared, req)? else {
        panic!("expected JSON");
    };
    Ok(value)
}

fn events(shared: &Shared) -> Value {
    let Reply::Json(value) = crate::events::handle(
        shared,
        crate::events::Request::Read {
            after: 0,
            limit: 512,
            epoch: None,
            timeout_ms: 0,
        },
    )
    .unwrap() else {
        panic!("expected JSON");
    };
    value["events"].clone()
}

// All original-list slots for a three-item sequence, including both adjacent no-ops.
const CASES: &[(usize, usize, [usize; 3])] = &[
    (0, 0, [1, 2, 3]),
    (0, 1, [1, 2, 3]),
    (0, 2, [2, 1, 3]),
    (0, 3, [2, 3, 1]),
    (1, 0, [2, 1, 3]),
    (1, 1, [1, 2, 3]),
    (1, 2, [1, 2, 3]),
    (1, 3, [1, 3, 2]),
    (2, 0, [3, 1, 2]),
    (2, 1, [1, 3, 2]),
    (2, 2, [1, 2, 3]),
    (2, 3, [1, 2, 3]),
];

#[test]
fn workspace_move_all_slots_keep_focus_even_when_recency_ties() {
    for &(source, slot, expected) in CASES {
        let shared = fixture();
        let before_used = lock(&shared).sessions[0].last_used;
        let reply = raw_request(
            &shared,
            json!({"WorkspaceMove": {
                "workspace": format!("w{}", source + 1), "insert_index": slot,
            }}),
        )
        .unwrap();
        let moved = expected != [1, 2, 3];
        assert_eq!(reply["moved"], moved);
        let st = lock(&shared);
        assert_eq!(
            st.sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(st.sessions[st.target_session(None).unwrap()].id, 3);
        for s in &st.sessions {
            assert_eq!(s.last_used, before_used);
            assert_eq!(s.windows[s.active].id, s.id * 10 + 2);
            assert!(s.windows[s.active].zoomed);
        }
        assert_eq!((st.next_workspace_id, st.next_tab_id), (3, 33));
        assert_eq!(st.dirty, moved);
        let ordered_ids = expected.map(workspace_id_str);
        assert_eq!(
            reply["workspaces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| w["workspace_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ordered_ids
        );
        drop(st);
        let emitted = events(&shared);
        assert_eq!(emitted.as_array().unwrap().len(), usize::from(moved));
        if moved {
            assert_eq!(emitted[0]["kind"], "workspace.moved");
            assert_eq!(emitted[0]["data"]["workspace_ids"], json!(ordered_ids));
            assert_eq!(emitted[0]["data"]["from_index"], source);
            assert_eq!(emitted[0]["data"]["to_index"], reply["to_index"]);
            assert_eq!(emitted[0]["data"]["insert_index"], slot);
        }
    }
}

#[test]
fn tab_move_all_slots_keep_active_pane_zoom_and_background_focus() {
    for &(source, slot, expected) in CASES {
        let shared = fixture();
        let reply = raw_request(
            &shared,
            json!({"TabMove": {
                "tab": format!("t{}", 11 + source), "workspace": "w1",
                "insert_index": slot, "current": null,
            }}),
        )
        .unwrap();
        let moved = expected != [1, 2, 3];
        assert_eq!(reply["moved"], moved);
        let st = lock(&shared);
        let s = &st.sessions[0];
        let expected_tabs = expected.map(|n| n + 10);
        assert_eq!(
            s.windows.iter().map(|w| w.id).collect::<Vec<_>>(),
            expected_tabs
        );
        assert_eq!(s.windows[s.active].id, 12);
        assert_eq!(s.windows[s.active].active, 120);
        assert!(s.windows[s.active].zoomed);
        for w in &s.windows {
            assert_eq!(w.panes(), vec![w.id * 10]);
        }
        assert_eq!(st.sessions[st.target_session(None).unwrap()].id, 3);
        assert_eq!(st.sessions[1].windows[1].id, 22);
        assert_eq!((st.next_workspace_id, st.next_tab_id), (3, 33));
        assert_eq!(st.dirty, moved);
        drop(st);
        let emitted = events(&shared);
        assert_eq!(emitted.as_array().unwrap().len(), usize::from(moved));
        if moved {
            assert_eq!(emitted[0]["kind"], "tab.moved");
            assert_eq!(emitted[0]["data"]["workspace_id"], "w1");
            assert_eq!(
                emitted[0]["data"]["tab_ids"],
                json!(expected_tabs.map(tab_id_str))
            );
            assert_eq!(emitted[0]["data"]["to_index"], reply["to_index"]);
        }
    }
}

#[test]
fn invalid_raw_moves_do_not_change_state_focus_events_or_dirty_flag() {
    let shared = fixture();
    let before = request(&shared, Request::WorkspaceList);
    for (value, code) in [
        (
            json!({"WorkspaceMove": {"workspace": "w1", "insert_index": 4}}),
            "invalid_argument",
        ),
        (
            json!({"WorkspaceMove": {"workspace": "w1", "insert_index": usize::MAX}}),
            "invalid_argument",
        ),
        (
            json!({"WorkspaceMove": {"workspace": "w999", "insert_index": 0}}),
            "workspace_not_found",
        ),
        (
            json!({"WorkspaceMove": {"workspace": "project.1", "insert_index": 0}}),
            "workspace_not_found",
        ),
        (
            json!({"TabMove": {"tab": "t11", "workspace": "w1", "current": null, "insert_index": 4}}),
            "invalid_argument",
        ),
        (
            json!({"TabMove": {"tab": "t11", "workspace": "w2", "current": null, "insert_index": 0}}),
            "tab_not_found",
        ),
        (
            json!({"TabMove": {"tab": "review.1:中文", "workspace": null, "current": 9999, "insert_index": 0}}),
            "pane_not_found",
        ),
    ] {
        assert_eq!(raw_request(&shared, value).unwrap_err().code, code);
        assert_eq!(request(&shared, Request::WorkspaceList), before);
        assert_eq!(events(&shared), json!([]));
        assert!(!lock(&shared).dirty);
    }
    for invalid in [json!(-1), json!(1.5), json!("1"), json!(null)] {
        assert!(
            serde_json::from_value::<Request>(json!({"WorkspaceMove": {
                "workspace": "w1", "insert_index": invalid,
            }}))
            .is_err()
        );
    }
}

#[test]
fn move_names_keep_exact_matching_and_background_caller_scope() {
    let shared = fixture();
    raw_request(
        &shared,
        json!({"WorkspaceMove": {
            "workspace": "project.1:中文", "insert_index": 3,
        }}),
    )
    .unwrap();
    // Current pane is in the now-last workspace, even though w3 remains globally focused.
    let reply = raw_request(
        &shared,
        json!({"TabMove": {
            "tab": "review.1:中文", "workspace": null, "current": 110, "insert_index": 3,
        }}),
    )
    .unwrap();
    assert_eq!(reply["workspace_id"], "w1");
    assert_eq!(reply["tab_id"], "t11");
    assert_eq!(reply["tabs"][2]["tab_id"], "t11");
    let current = request(
        &shared,
        Request::TabGet {
            tab: None,
            workspace: None,
            current: Some(110),
        },
    );
    assert_eq!(current["tab"]["tab_id"], "t11");
    assert_eq!(current["tab"]["index"], 2);
    // An explicit scope still wins over caller context.
    let reply = raw_request(
        &shared,
        json!({"TabMove": {
            "tab": "review.1:中文", "workspace": "w2", "current": 110, "insert_index": 3,
        }}),
    )
    .unwrap();
    assert_eq!(reply["tab_id"], "t21");
}

fn add_client(st: &mut State, id: usize, entry: crate::render::Entry, filter: Filter) {
    let selection = visible_entries(st, filter)
        .iter()
        .position(|&e| e == entry)
        .unwrap();
    st.clients.insert(
        id,
        Client {
            session: st.sessions[1].name.clone(),
            sender: Box::new(std::io::sink()),
            prefix: false,
            frame: Frame::default(),
            prompt: None,
            chooser: Some(pack_chooser(selection, filter)),
            popup: None,
            message: None,
            drag: Some((120, 5, 2, true)),
            env: Default::default(),
        },
    );
}

#[test]
fn workspace_move_preserves_attached_clients_and_chooser_targets() {
    let shared = fixture();
    {
        let mut st = lock(&shared);
        add_client(&mut st, 1, (0, None, None), Filter::All);
        add_client(&mut st, 2, (0, Some(1), None), Filter::All);
        add_client(
            &mut st,
            3,
            (0, Some(1), Some(0)),
            Filter::Only(crate::api::Status::Unknown),
        );
    }
    raw_request(
        &shared,
        json!({"WorkspaceMove": {"workspace": "w1", "insert_index": 3}}),
    )
    .unwrap();
    let st = lock(&shared);
    for (cid, expected) in [
        (1, (2, None, None)),
        (2, (2, Some(1), None)),
        (3, (2, Some(1), Some(0))),
    ] {
        let c = &st.clients[&cid];
        assert_eq!(c.session, "project.2:中文");
        assert_eq!(c.drag, Some((120, 5, 2, true)));
        let (sel, filter) = unpack_chooser(c.chooser.unwrap());
        assert_eq!(visible_entries(&st, filter)[sel], expected);
        if cid == 3 {
            assert_eq!(filter, Filter::Only(crate::api::Status::Unknown));
        }
    }
}

#[test]
fn tab_move_keeps_chooser_and_pending_close_on_the_original_tab() {
    let shared = fixture();
    {
        let mut st = lock(&shared);
        add_client(&mut st, 1, (0, Some(1), None), Filter::All);
        add_client(&mut st, 2, (0, Some(2), Some(0)), Filter::All);
        st.clients.get_mut(&1).unwrap().prompt = Some(Prompt::confirm(
            "close review.2:中文?".into(),
            crate::commands::kill_window_command("project.1:中文", 1),
        ));
    }
    raw_request(
        &shared,
        json!({"TabMove": {
            "tab": "t11", "workspace": null, "current": null, "insert_index": 3,
        }}),
    )
    .unwrap();
    let st = lock(&shared);
    for (cid, expected) in [(1, (0, Some(0), None)), (2, (0, Some(1), Some(0)))] {
        let (sel, filter) = unpack_chooser(st.clients[&cid].chooser.unwrap());
        assert_eq!(visible_entries(&st, filter)[sel], expected);
    }
    let prompt = st.clients[&1].prompt.as_ref().unwrap();
    assert_eq!(prompt.buf, "close review.2:中文?");
    assert_eq!(
        prompt.pending.as_deref(),
        Some(crate::commands::kill_window_command("project.1:中文", 0).as_str())
    );
    let crate::commands::Cmd::KillWindow { target } =
        crate::commands::parse(prompt.pending.as_ref().unwrap()).unwrap()
    else {
        panic!("expected close command");
    };
    let (si, wi) = st
        .window_for(st.sidx(&st.clients[&1].session), target.as_deref())
        .unwrap();
    assert_eq!(st.sessions[si].windows[wi].id, 12);
}

#[test]
fn singleton_lists_accept_both_slots_without_events() {
    let shared = fixture();
    {
        let mut st = lock(&shared);
        st.sessions.truncate(1);
        st.sessions[0].windows.truncate(1);
        st.sessions[0].active = 0;
    }
    for slot in [0, 1] {
        let workspace = raw_request(
            &shared,
            json!({"WorkspaceMove": {"workspace": "w1", "insert_index": slot}}),
        )
        .unwrap();
        assert_eq!(workspace["moved"], false);
        let tab = raw_request(&shared, json!({"TabMove": {"tab": "t11", "workspace": null, "current": null, "insert_index": slot}})).unwrap();
        assert_eq!(tab["moved"], false);
    }
    assert_eq!(events(&shared), json!([]));
    assert!(!lock(&shared).dirty);
}
