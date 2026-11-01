use super::snapshot::*;
use super::*;
use crate::layout::{Dir, Node};
use crate::server::{Session, Window};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

fn plain_leaf(id: usize) -> LeafSnap {
    LeafSnap {
        cwd: Some(format!("/work/{id}")),
        ..LeafSnap::default()
    }
}

/// Pane 2 is an agent with a label and a resume command.
fn lookup(id: usize) -> LeafSnap {
    if id == 2 {
        LeafSnap {
            cwd: Some("/work/api".into()),
            env: Default::default(),
            label: Some("api".into()),
            agent: Some("claude".into()),
            agent_name: Some("reviewer".into()),
            resume: vec!["claude".into(), "--resume".into(), "abc123".into()],
        }
    } else {
        plain_leaf(id)
    }
}

fn three_pane_session() -> Session {
    let layout = Node::Split {
        dir: Dir::H,
        ratio: 0.4,
        a: Box::new(Node::Leaf(1)),
        b: Box::new(Node::Split {
            dir: Dir::V,
            ratio: 0.5,
            a: Box::new(Node::Leaf(2)),
            b: Box::new(Node::Leaf(3)),
        }),
    };
    Session {
        id: 1,
        env: Default::default(),
        name: "main".into(),
        windows: vec![Window {
            id: 1,
            name: "sh".into(),
            manual_name: false,
            layout,
            active: 3,
            zoomed: true,
        }],
        active: 0,
        cols: 120,
        rows: 40,
        sidebar: true,
        cwd: Some("/work".into()),
        last_used: Instant::now(),
    }
}

fn sample() -> Snapshot {
    snapshot_of(&[three_pane_session()], &lookup)
}

fn set_first_window(snap: &mut Snapshot, f: impl FnOnce(&mut WindowSnap)) {
    f(&mut snap.sessions[0].windows[0]);
}

fn chain(splits: usize) -> LayoutSnap {
    let mut node = LayoutSnap::Leaf(LeafSnap::default());
    for _ in 0..splits {
        node = LayoutSnap::Split {
            dir: Dir::H,
            ratio: 0.5,
            a: Box::new(LayoutSnap::Leaf(LeafSnap::default())),
            b: Box::new(node),
        };
    }
    node
}

/// Fresh directory for one test, so parallel tests never share files.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("crew-persist-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn json_roundtrip_of_three_pane_tree() {
    let snap = sample();
    let text = serialize(&snap).unwrap();
    let back = parse_snapshot(&text).unwrap();
    assert_eq!(back, snap);
    let w = &back.sessions[0].windows[0];
    assert_eq!(w.active, 2, "pane 3 is leaf index 2");
    assert!(w.zoomed);
    assert_eq!(
        back.sessions[0].windows[0].layout,
        snap.sessions[0].windows[0].layout
    );
}

#[test]
fn validate_rejects_bad_version() {
    let mut snap = sample();
    snap.version = 2;
    assert!(validate(&snap).is_err());
}

#[test]
fn validate_rejects_empty_windows() {
    let mut snap = sample();
    snap.sessions[0].windows.clear();
    assert!(validate(&snap).is_err());
}

#[test]
fn validate_rejects_out_of_range_indices() {
    let mut snap = sample();
    set_first_window(&mut snap, |w| w.active = 3);
    assert!(
        validate(&snap).is_err(),
        "3 leaves, index 3 is out of range"
    );

    let mut snap = sample();
    snap.sessions[0].active_window = 1;
    assert!(validate(&snap).is_err());
}

#[test]
fn validate_rejects_duplicate_and_empty_names() {
    let mut snap = sample();
    let dup = snap.sessions[0].clone();
    snap.sessions.push(dup);
    assert!(validate(&snap).is_err());

    let mut snap = sample();
    snap.sessions[0].name.clear();
    assert!(validate(&snap).is_err());
}

#[test]
fn validate_rejects_non_finite_ratio() {
    let mut snap = sample();
    set_first_window(&mut snap, |w| {
        w.layout = LayoutSnap::Split {
            dir: Dir::V,
            ratio: f32::NAN,
            a: Box::new(LayoutSnap::Leaf(LeafSnap::default())),
            b: Box::new(LayoutSnap::Leaf(LeafSnap::default())),
        };
    });
    assert!(validate(&snap).is_err());
}

#[test]
fn validate_limits_depth_to_64() {
    let ok = |n| {
        let mut snap = sample();
        set_first_window(&mut snap, |w| {
            w.layout = chain(n);
            w.active = 0;
        });
        validate(&snap)
    };
    assert!(ok(63).is_ok(), "64 levels");
    assert!(ok(64).is_err(), "65 levels");
}

#[test]
fn rebuild_assigns_leaf_ids_in_order() {
    let mut next = 9;
    let node = rebuild(
        &sample().sessions[0].windows[0].layout,
        &mut |_: &LeafSnap| {
            next += 1;
            Ok(next)
        },
    )
    .unwrap();
    let mut leaves = vec![];
    node.leaves(&mut leaves);
    assert_eq!(leaves, vec![10, 11, 12]);
}

#[test]
fn rebuild_aborts_on_first_closure_error() {
    let mut calls = 0;
    let layout = &sample().sessions[0].windows[0].layout;
    let result = rebuild(layout, &mut |_: &LeafSnap| {
        calls += 1;
        if calls == 2 {
            Err("boom".to_string())
        } else {
            Ok(calls)
        }
    });
    assert_eq!(result.unwrap_err(), "boom");
    assert_eq!(calls, 2, "no leaves after the failing one");
}

#[test]
fn rebuild_clamps_finite_ratios() {
    let split = |ratio: f32| LayoutSnap::Split {
        dir: Dir::H,
        ratio,
        a: Box::new(LayoutSnap::Leaf(LeafSnap::default())),
        b: Box::new(LayoutSnap::Leaf(LeafSnap::default())),
    };
    let mut id = 0;
    let mut next = |_: &LeafSnap| {
        id += 1;
        Ok(id)
    };
    let high = rebuild(&split(2.0), &mut next).unwrap();
    let low = rebuild(&split(-1.0), &mut next).unwrap();
    assert!(matches!(high, Node::Split { ratio, .. } if ratio == 0.95));
    assert!(matches!(low, Node::Split { ratio, .. } if ratio == 0.05));
    assert!(rebuild(&split(f32::INFINITY), &mut next).is_err());
}

#[test]
fn sanitize_keeps_only_safe_chars() {
    assert_eq!(sanitize_file_part("crew-user.sock"), "crew-user.sock");
    assert_eq!(sanitize_file_part("a b/c\\d:e"), "a_b_c_d_e");
    assert_eq!(sanitize_file_part("..\\x"), ".._x");
    assert_eq!(sanitize_file_part("ü"), "_");
}

#[test]
fn snapshot_maps_focus_and_skips_empty_sessions() {
    let mut empty = three_pane_session();
    empty.name = "empty".into();
    empty.windows.clear();
    let snap = snapshot_of(&[empty, three_pane_session()], &lookup);
    assert_eq!(snap.sessions.len(), 1);
    assert_eq!(snap.sessions[0].windows[0].active, 2);
    assert!(validate(&snap).is_ok());
}

#[test]
fn parse_rejects_invalid_json() {
    assert!(parse_snapshot("{not json").is_err());
}

#[test]
fn write_atomic_replaces_existing_file() {
    let dir = scratch_dir("atomic");
    let path = dir.join("state.json");
    write_atomic(&path, "one").unwrap();
    write_atomic(&path, "two").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "two");
    assert!(!with_suffix(&path, ".tmp").exists());
    let _ = fs::remove_dir_all(&dir);
}

// M3: the state file is read as bytes; anything that is not a valid snapshot is an error.

#[test]
fn decode_rejects_non_utf8_bytes() {
    assert!(decode_snapshot(&[0xff, 0xfe, b'{']).is_err());
}

#[test]
fn decode_accepts_valid_snapshot_bytes() {
    let bytes = serialize(&sample()).unwrap().into_bytes();
    assert_eq!(decode_snapshot(&bytes).unwrap(), sample());
}

#[test]
fn decode_rejects_truncated_json_bytes() {
    assert!(decode_snapshot(b"{\"version\": 1, \"sessions\": [").is_err());
}

#[test]
fn quarantine_moves_file_to_bad_suffix() {
    let dir = scratch_dir("quarantine");
    let path = dir.join("state.json");
    fs::write(&path, b"broken").unwrap();
    let msg = quarantine(&path, "parse: bad");
    assert!(!path.exists(), "original moved away");
    assert_eq!(fs::read(with_suffix(&path, ".bad")).unwrap(), b"broken");
    assert!(msg.contains("parse: bad"));
    let _ = fs::remove_dir_all(&dir);
}

// M2: a partial restore keeps the original file as `<file>.partial.bad`.

#[test]
fn partial_restore_means_some_but_not_all_came_back() {
    assert!(is_partial(3, 1));
    assert!(is_partial(2, 1));
    assert!(!is_partial(3, 3), "full restore");
    assert!(!is_partial(3, 0), "zero restored is handled as a failure");
    assert!(!is_partial(0, 0), "nothing attempted");
}

#[test]
fn keep_partial_copy_preserves_original_bytes() {
    let dir = scratch_dir("partial");
    let path = dir.join("state.json");
    fs::write(&path, b"original snapshot").unwrap();
    keep_partial_copy(&path);
    let kept = with_suffix(&path, ".partial.bad");
    assert_eq!(fs::read(&kept).unwrap(), b"original snapshot");
    assert!(path.exists(), "the state file itself stays in place");
    let _ = fs::remove_dir_all(&dir);
}

// M1: restored agent names must be valid and unique.

#[test]
fn keep_agent_name_accepts_valid_unique_name() {
    let mut taken = HashSet::new();
    assert_eq!(
        keep_agent_name(Some("reviewer"), &mut taken).as_deref(),
        Some("reviewer")
    );
    assert!(taken.contains("reviewer"));
}

#[test]
fn keep_agent_name_drops_invalid_names() {
    let mut taken = HashSet::new();
    assert_eq!(keep_agent_name(None, &mut taken), None);
    assert_eq!(keep_agent_name(Some("p1"), &mut taken), None, "pane-shaped");
    assert_eq!(keep_agent_name(Some("%7"), &mut taken), None, "pane-shaped");
    assert_eq!(keep_agent_name(Some("Bad Name"), &mut taken), None);
    assert_eq!(keep_agent_name(Some(""), &mut taken), None);
    let too_long = "a".repeat(33);
    assert_eq!(keep_agent_name(Some(too_long.as_str()), &mut taken), None);
    assert!(taken.is_empty(), "rejected names claim nothing");
}

#[test]
fn keep_agent_name_drops_duplicates() {
    let mut taken = HashSet::new();
    assert!(keep_agent_name(Some("reviewer"), &mut taken).is_some());
    assert_eq!(keep_agent_name(Some("reviewer"), &mut taken), None);
    assert_eq!(taken.len(), 1);
}

#[test]
fn legacy_state_without_ids_migrates_without_colliding_with_saved_ids() {
    let mut snap = sample();
    let mut legacy = serde_json::to_value(&snap).unwrap();
    legacy.as_object_mut().unwrap().remove("next_workspace_id");
    legacy.as_object_mut().unwrap().remove("next_tab_id");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("focused_workspace_id");
    legacy["sessions"][0].as_object_mut().unwrap().remove("id");
    legacy["sessions"][0].as_object_mut().unwrap().remove("env");
    legacy["sessions"][0]["windows"][0]
        .as_object_mut()
        .unwrap()
        .remove("id");
    let mut decoded = parse_snapshot(&legacy.to_string()).unwrap();
    assert_eq!(decoded.sessions[0].id, 0);
    let mut second = snap.sessions.remove(0);
    second.id = 10;
    second.name = "saved".into();
    second.windows[0].id = 20;
    decoded.sessions.push(second);
    assign_legacy_ids(&mut decoded).unwrap();
    assert_eq!(decoded.sessions[0].id, 11);
    assert_eq!(decoded.sessions[0].windows[0].id, 21);
    assert_eq!(decoded.sessions[1].id, 10);
    assert_eq!(decoded.sessions[1].windows[0].id, 20);
}

#[test]
fn duplicate_saved_ids_are_rejected_before_spawning() {
    let mut snap = sample();
    let mut duplicate = snap.sessions[0].clone();
    duplicate.name = "other".into();
    snap.sessions.push(duplicate);
    assert!(
        validate(&snap)
            .unwrap_err()
            .contains("duplicate workspace id")
    );
    snap.sessions[1].id = 2;
    assert!(validate(&snap).unwrap_err().contains("duplicate tab id"));
    snap.sessions[1].windows[0].id = 2;
    assert!(validate(&snap).is_ok());
}

#[test]
fn saved_high_water_marks_prevent_id_reuse_after_deletion() {
    let mut snap = sample();
    snap.next_workspace_id = 100;
    snap.next_tab_id = 200;
    snap.sessions[0].id = 0;
    snap.sessions[0].windows[0].id = 0;
    assign_legacy_ids(&mut snap).unwrap();
    assert_eq!(snap.sessions[0].id, 101);
    assert_eq!(snap.sessions[0].windows[0].id, 201);
    assert_eq!(snap.next_workspace_id, 101);
    assert_eq!(snap.next_tab_id, 201);
    let original = snap.clone();
    assign_legacy_ids(&mut snap).unwrap();
    assert_eq!(snap, original);
}

#[test]
fn clear_of_new_state_never_resurrects_unselected_legacy_state() {
    let base = scratch_dir("new-state-selection");
    let file = "state-socket.json";
    let new = base.join("crew").join(file);
    let old = base.join("tmuxlike").join(file);
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&new, "new state").unwrap();
    fs::write(&old, "legacy must stay untouched").unwrap();
    assert_eq!(
        state_of(default_state_path(&base, file, None, &never_live)),
        new
    );
    remove_state_file(&new);
    assert!(!new.exists());
    assert_eq!(
        state_of(default_state_path(&base, file, None, &never_live)),
        new
    );
    assert_eq!(
        fs::read_to_string(&old).unwrap(),
        "legacy must stay untouched"
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn quarantined_new_state_never_falls_back_to_legacy_even_without_marker() {
    let base = scratch_dir("bad-state-selection");
    let file = "state-socket.json";
    let new = base.join("crew").join(file);
    let old = base.join("tmuxlike").join(file);
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&new, "invalid new state").unwrap();
    fs::write(&old, "old sessions").unwrap();
    assert_eq!(
        state_of(default_state_path(&base, file, None, &never_live)),
        new
    );
    quarantine(&new, "invalid JSON");
    assert_eq!(
        state_of(default_state_path(&base, file, None, &never_live)),
        new
    );
    fs::remove_file(with_suffix(&new, ".crew-selected")).unwrap();
    assert_eq!(
        state_of(default_state_path(&base, file, None, &never_live)),
        new
    );
    assert_eq!(fs::read_to_string(&old).unwrap(), "old sessions");
    assert_eq!(
        fs::read_to_string(with_suffix(&new, ".bad")).unwrap(),
        "invalid new state"
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn existing_legacy_state_is_used_until_a_new_state_is_selected() {
    let base = scratch_dir("legacy-state-selection");
    let file = "state-socket.json";
    let old = base.join("tmuxlike").join(file);
    let new = base.join("crew").join(file);
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, "old sessions").unwrap();
    assert_eq!(
        state_of(default_state_path(&base, file, None, &never_live)),
        old
    );
    assert!(!with_suffix(&new, ".crew-selected").exists());
    remove_state_file(&old);
    assert_eq!(
        state_of(default_state_path(&base, file, None, &never_live)),
        new
    );
    // A later stale legacy file cannot override the recorded empty new location.
    fs::write(&old, "stale sessions").unwrap();
    assert_eq!(
        state_of(default_state_path(&base, file, None, &never_live)),
        new
    );
    fs::remove_dir_all(base).unwrap();
}

const NEW_SOCKET_FILE: &str = "state-crew-alice.sock.json";
const LEGACY_SOCKET_FILE: &str = "state-tmuxlike-alice.sock.json";

#[test]
fn legacy_socket_file_name_is_the_pre_rename_state_file() {
    assert_eq!(state_file_name("tmuxlike-alice.sock"), LEGACY_SOCKET_FILE);
    assert_eq!(state_file_name("crew-alice.sock"), NEW_SOCKET_FILE);
}

fn never_live() -> bool {
    false
}

fn live() -> bool {
    true
}

/// Write `contents` to `path`, creating its directory first.
fn put(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// The raw selection for the default socket, with `probe` standing in for the liveness check.
fn select_legacy(base: &Path, probe: &dyn Fn() -> bool) -> io::Result<Selection> {
    default_state_path(base, NEW_SOCKET_FILE, Some(LEGACY_SOCKET_FILE), probe)
}

/// The state file chosen for the default socket; a deferred selection fails the test here.
fn pick_legacy(base: &Path, probe: &dyn Fn() -> bool) -> PathBuf {
    state_of(select_legacy(base, probe))
}

/// The state file of a selection; a deferred selection fails the test here.
fn state_of(selection: io::Result<Selection>) -> PathBuf {
    match selection.unwrap() {
        Selection::State(path) => path,
        Selection::Deferred => panic!("expected a state file, got a deferred selection"),
    }
}

#[test]
fn legacy_socket_candidates_are_tried_in_brief_order() {
    // Each stage removes the candidate that came first, so the chain must fall through in order.
    let all = scratch_dir("legacy-socket-order-all");
    let crew_legacy = all.join("crew").join(LEGACY_SOCKET_FILE);
    put(&crew_legacy, "old");
    put(&all.join("tmuxlike").join(LEGACY_SOCKET_FILE), "old");
    put(&all.join("tmuxlike").join(NEW_SOCKET_FILE), "old");
    assert_eq!(pick_legacy(&all, &never_live), crew_legacy);

    let middle = scratch_dir("legacy-socket-order-middle");
    let tmux_legacy = middle.join("tmuxlike").join(LEGACY_SOCKET_FILE);
    put(&tmux_legacy, "old");
    put(&middle.join("tmuxlike").join(NEW_SOCKET_FILE), "old");
    assert_eq!(pick_legacy(&middle, &never_live), tmux_legacy);

    let last = scratch_dir("legacy-socket-order-last");
    let tmux_new = last.join("tmuxlike").join(NEW_SOCKET_FILE);
    put(&tmux_new, "old");
    assert_eq!(pick_legacy(&last, &never_live), tmux_new);

    let none = scratch_dir("legacy-socket-order-none");
    let fresh = none.join("crew").join(NEW_SOCKET_FILE);
    assert_eq!(pick_legacy(&none, &never_live), fresh);
    for dir in [all, middle, last, none] {
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn removed_legacy_candidate_never_resurrects_the_next_one() {
    let base = scratch_dir("legacy-socket-removed");
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    let tmux_legacy = base.join("tmuxlike").join(LEGACY_SOCKET_FILE);
    put(&crew_legacy, "crew legacy");
    put(&tmux_legacy, "tmux legacy");
    assert_eq!(pick_legacy(&base, &never_live), crew_legacy);
    fs::remove_file(&crew_legacy).unwrap();
    let new = base.join("crew").join(NEW_SOCKET_FILE);
    assert_eq!(pick_legacy(&base, &never_live), new);
    assert_eq!(fs::read_to_string(&tmux_legacy).unwrap(), "tmux legacy");
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn quarantined_legacy_candidate_never_resurrects_the_next_one() {
    let base = scratch_dir("legacy-socket-quarantined");
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    let tmux_legacy = base.join("tmuxlike").join(LEGACY_SOCKET_FILE);
    put(&crew_legacy, "crew legacy");
    put(&tmux_legacy, "tmux legacy");
    assert_eq!(pick_legacy(&base, &never_live), crew_legacy);
    quarantine(&crew_legacy, "invalid JSON");
    let new = base.join("crew").join(NEW_SOCKET_FILE);
    assert_eq!(pick_legacy(&base, &never_live), new);
    assert_eq!(fs::read_to_string(&tmux_legacy).unwrap(), "tmux legacy");
    assert_eq!(
        fs::read_to_string(with_suffix(&crew_legacy, ".bad")).unwrap(),
        "crew legacy"
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn quarantined_legacy_file_blocks_the_chain_even_without_a_selection_marker() {
    let base = scratch_dir("legacy-socket-bad-only");
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    put(&with_suffix(&crew_legacy, ".bad"), "crew legacy");
    put(
        &base.join("tmuxlike").join(LEGACY_SOCKET_FILE),
        "tmux legacy",
    );
    let new = base.join("crew").join(NEW_SOCKET_FILE);
    assert_eq!(pick_legacy(&base, &never_live), new);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn live_legacy_server_keeps_its_state_until_it_stops() {
    let base = scratch_dir("legacy-socket-live");
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    put(&crew_legacy, "live legacy sessions");
    assert_eq!(
        select_legacy(&base, &live).unwrap(),
        Selection::Deferred,
        "a live older server is not taken over"
    );
    assert_eq!(
        select_legacy(&base, &live).unwrap(),
        Selection::Deferred,
        "still deferred on the next tick"
    );
    assert_eq!(
        fs::read_to_string(&crew_legacy).unwrap(),
        "live legacy sessions"
    );
    assert_eq!(
        pick_legacy(&base, &never_live),
        crew_legacy,
        "older server stopped: the next start selects the legacy layout"
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn deferred_selection_writes_no_state_file() {
    let base = scratch_dir("legacy-socket-defer-writes");
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    put(&crew_legacy, "live legacy sessions");
    let selection = select_legacy(&base, &live).unwrap();
    assert_eq!(selection, Selection::Deferred);
    assert_eq!(
        selection.writable_path(),
        None,
        "nothing is loaded, autosaved, saved on shutdown or cleared"
    );
    // Only the legacy file and its deferred marker exist: no new state file, no selection marker.
    let mut names: Vec<String> = fs::read_dir(base.join("crew"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            LEGACY_SOCKET_FILE.to_string(),
            format!("{LEGACY_SOCKET_FILE}.crew-deferred"),
        ]
    );
    assert!(!base.join("tmuxlike").exists());
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn deferred_legacy_file_removed_by_its_live_server_is_not_resurrected() {
    let base = scratch_dir("legacy-socket-deferred-removed");
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    put(&crew_legacy, "live legacy sessions");
    put(
        &base.join("tmuxlike").join(LEGACY_SOCKET_FILE),
        "tmux legacy",
    );
    let new = base.join("crew").join(NEW_SOCKET_FILE);
    assert_eq!(select_legacy(&base, &live).unwrap(), Selection::Deferred);
    // The older server's own kill-server removes its file while it is still running.
    fs::remove_file(&crew_legacy).unwrap();
    assert_eq!(pick_legacy(&base, &never_live), new);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn only_a_state_selection_has_a_writable_path() {
    assert_eq!(Selection::Deferred.writable_path(), None);
    assert_eq!(
        Selection::State("state.json".into()).writable_path(),
        Some("state.json".into())
    );
}

fn step_at(path: &str) -> Step {
    Step {
        path: PathBuf::from(path),
        exists: false,
        selected: false,
        deferred: false,
        quarantined: false,
        from_legacy_socket: false,
    }
}

#[test]
fn choose_legacy_takes_the_first_existing_candidate() {
    let steps = [
        Step {
            exists: true,
            from_legacy_socket: true,
            ..step_at("crew-legacy")
        },
        Step {
            exists: true,
            from_legacy_socket: true,
            ..step_at("tmux-legacy")
        },
    ];
    assert_eq!(
        choose_legacy(&steps, false),
        Choice::Use("crew-legacy".into())
    );
}

#[test]
fn choose_legacy_defers_an_unselected_file_while_its_server_is_live() {
    let steps = [Step {
        exists: true,
        from_legacy_socket: true,
        ..step_at("crew-legacy")
    }];
    assert_eq!(
        choose_legacy(&steps, true),
        Choice::Defer("crew-legacy".into())
    );
}

#[test]
fn choose_legacy_never_switches_away_from_a_selected_file() {
    let steps = [Step {
        exists: true,
        selected: true,
        from_legacy_socket: true,
        ..step_at("crew-legacy")
    }];
    assert_eq!(
        choose_legacy(&steps, true),
        Choice::Use("crew-legacy".into())
    );
}

#[test]
fn choose_legacy_ignores_liveness_for_the_directory_fallback() {
    let steps = [Step {
        exists: true,
        ..step_at("tmux-new")
    }];
    assert_eq!(choose_legacy(&steps, true), Choice::Use("tmux-new".into()));
}

#[test]
fn choose_legacy_stops_at_a_used_candidate_that_is_gone() {
    for used in [
        Step {
            selected: true,
            from_legacy_socket: true,
            ..step_at("crew-legacy")
        },
        Step {
            deferred: true,
            from_legacy_socket: true,
            ..step_at("crew-legacy")
        },
        Step {
            quarantined: true,
            from_legacy_socket: true,
            ..step_at("crew-legacy")
        },
    ] {
        let later = Step {
            exists: true,
            from_legacy_socket: true,
            ..step_at("tmux-legacy")
        };
        assert_eq!(choose_legacy(&[used, later], false), Choice::Fresh);
    }
}

#[test]
fn choose_legacy_falls_through_a_missing_candidate_that_was_never_used() {
    let steps = [
        step_at("crew-legacy"),
        Step {
            exists: true,
            ..step_at("tmux-new")
        },
    ];
    assert_eq!(choose_legacy(&steps, false), Choice::Use("tmux-new".into()));
    assert_eq!(
        choose_legacy(&[step_at("crew-legacy")], false),
        Choice::Fresh
    );
}

#[test]
fn legacy_socket_state_is_never_resurrected_after_new_state_is_cleared() {
    let base = scratch_dir("legacy-socket-clear");
    let old = base.join("crew").join(LEGACY_SOCKET_FILE);
    let new = base.join("crew").join(NEW_SOCKET_FILE);
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, "legacy sessions").unwrap();
    let pick = || pick_legacy(&base, &never_live);
    assert_eq!(pick(), old, "legacy is used while no new state exists");
    remove_state_file(&old);
    assert_eq!(pick(), new, "legacy consumed, new selected");
    fs::write(&new, "new sessions").unwrap();
    remove_state_file(&new);
    fs::write(&old, "stale legacy").unwrap();
    assert_eq!(pick(), new, "cleared new state must not bring legacy back");
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn quarantined_new_socket_state_never_falls_back_to_legacy() {
    let base = scratch_dir("legacy-socket-quarantine");
    let old = base.join("crew").join(LEGACY_SOCKET_FILE);
    let new = base.join("crew").join(NEW_SOCKET_FILE);
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, "legacy sessions").unwrap();
    fs::write(&new, "invalid new state").unwrap();
    let pick = || pick_legacy(&base, &never_live);
    assert_eq!(pick(), new);
    quarantine(&new, "invalid JSON");
    let _ = fs::remove_file(with_suffix(&new, ".crew-selected"));
    assert_eq!(pick(), new, "quarantined new state still blocks legacy");
    assert_eq!(fs::read_to_string(&old).unwrap(), "legacy sessions");
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn explicit_socket_never_reads_the_legacy_socket_state() {
    let base = scratch_dir("legacy-socket-explicit");
    let custom = "state-custom.sock.json";
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    let tmux_legacy = base.join("tmuxlike").join(LEGACY_SOCKET_FILE);
    fs::create_dir_all(crew_legacy.parent().unwrap()).unwrap();
    fs::create_dir_all(tmux_legacy.parent().unwrap()).unwrap();
    fs::write(&crew_legacy, "default-socket sessions").unwrap();
    fs::write(&tmux_legacy, "default-socket sessions").unwrap();
    // `None` is what `state_path` passes when `CREW_SOCKET` picks the endpoint.
    assert_eq!(
        state_of(default_state_path(&base, custom, None, &never_live)),
        base.join("crew").join(custom)
    );
    assert!(crew_legacy.exists() && tmux_legacy.exists());
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn choose_legacy_prefers_an_existing_selected_file_over_an_earlier_unselected_one() {
    let steps = [
        Step {
            exists: true,
            from_legacy_socket: true,
            ..step_at("crew-legacy")
        },
        Step {
            exists: true,
            selected: true,
            from_legacy_socket: true,
            ..step_at("tmux-legacy")
        },
    ];
    assert_eq!(
        choose_legacy(&steps, false),
        Choice::Use("tmux-legacy".into())
    );
    assert_eq!(
        choose_legacy(&steps, true),
        Choice::Use("tmux-legacy".into()),
        "a selected file is never deferred"
    );
}

#[test]
fn choose_legacy_keeps_the_chain_rule_ahead_of_a_selected_file() {
    let steps = [
        Step {
            deferred: true,
            from_legacy_socket: true,
            ..step_at("crew-legacy")
        },
        Step {
            exists: true,
            selected: true,
            from_legacy_socket: true,
            ..step_at("tmux-legacy")
        },
    ];
    assert_eq!(choose_legacy(&steps, false), Choice::Fresh);
}

#[test]
fn choose_legacy_ignores_used_candidates_after_the_chosen_one() {
    let steps = [
        Step {
            exists: true,
            from_legacy_socket: true,
            ..step_at("crew-legacy")
        },
        Step {
            deferred: true,
            from_legacy_socket: true,
            ..step_at("tmux-legacy")
        },
    ];
    assert_eq!(
        choose_legacy(&steps, false),
        Choice::Use("crew-legacy".into())
    );
}

#[test]
fn choose_legacy_does_not_count_an_existing_deferred_file_as_gone() {
    let steps = [
        Step {
            exists: true,
            deferred: true,
            from_legacy_socket: true,
            ..step_at("crew-legacy")
        },
        Step {
            exists: true,
            from_legacy_socket: true,
            ..step_at("tmux-legacy")
        },
    ];
    assert_eq!(
        choose_legacy(&steps, false),
        Choice::Use("crew-legacy".into())
    );
}

#[test]
fn deferral_is_expected_only_for_an_unselected_legacy_file_while_its_server_is_live() {
    let base = scratch_dir("legacy-deferral-expected");
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    let decide = |probe: &dyn Fn() -> bool| {
        legacy_choice(&base, NEW_SOCKET_FILE, Some(LEGACY_SOCKET_FILE), probe)
    };
    assert_eq!(
        decide(&live),
        Some(Choice::Fresh),
        "no legacy file to defer"
    );
    put(&crew_legacy, "legacy sessions");
    assert_eq!(decide(&never_live), Some(Choice::Use(crew_legacy.clone())));
    assert_eq!(decide(&live), Some(Choice::Defer(crew_legacy.clone())));
    // An earlier run selected the file, so it stays in use even while its server is live.
    mark(&crew_legacy, SELECTED).unwrap();
    assert_eq!(decide(&live), Some(Choice::Use(crew_legacy.clone())));
    fs::remove_dir_all(base).unwrap();
}

/// A state with one session, enough for every write path to try to save it.
fn one_session_state() -> State {
    let mut st = crate::server::test_state();
    st.sessions
        .push(crate::server::test_session("main", 80, 24, 1));
    st
}

/// The names directly inside `dir`, sorted.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn deferred_resolution_makes_save_autosave_clear_and_restore_write_nothing() {
    let base = scratch_dir("legacy-socket-api-deferred");
    let crew_legacy = base.join("crew").join(LEGACY_SOCKET_FILE);
    put(&crew_legacy, "live legacy sessions");
    let path = resolve_state_path(&base, NEW_SOCKET_FILE, Some(LEGACY_SOCKET_FILE), &live);
    assert_eq!(path, None, "a deferred selection resolves to no state path");
    let st = one_session_state();
    let mut last = None;
    autosave_at(path.as_deref(), &st, &mut last);
    save_now_at(path.as_deref(), &st);
    clear_at(path.as_deref());
    let shared: Shared = Arc::new(Mutex::new(crate::server::test_state()));
    assert_eq!(restore_from(path.as_deref(), &shared), Ok(0));
    assert_eq!(last, None, "autosave wrote nothing and kept no snapshot");
    assert_eq!(
        fs::read_to_string(&crew_legacy).unwrap(),
        "live legacy sessions"
    );
    assert_eq!(
        names_in(&base.join("crew")),
        vec![
            LEGACY_SOCKET_FILE.to_string(),
            format!("{LEGACY_SOCKET_FILE}.crew-deferred"),
        ]
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn the_same_write_paths_do_write_when_a_state_file_is_selected() {
    let base = scratch_dir("legacy-socket-api-control");
    let path = base.join("state.json");
    let st = one_session_state();
    let mut last = None;
    autosave_at(Some(path.as_path()), &st, &mut last);
    assert!(path.exists(), "autosave writes a selected file");
    fs::remove_file(&path).unwrap();
    save_now_at(Some(path.as_path()), &st);
    assert!(path.exists(), "save_now writes a selected file");
    clear_at(Some(path.as_path()));
    assert!(!path.exists(), "clear removes a selected file");
    fs::remove_dir_all(base).unwrap();
}
