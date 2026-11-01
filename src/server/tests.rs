use super::{TitleMode, contain_vt, wants_title};

#[test]
fn epoch_attachment_rejects_reused_ids_and_preserves_same_daemon_ids() {
    let mut st = super::test_state();
    let epoch = st.events.epoch().to_owned();
    st.sessions.push(super::Session {
        id: 1,
        name: "original".into(),
        windows: vec![],
        active: 0,
        cols: 80,
        rows: 24,
        sidebar: false,
        cwd: None,
        env: Default::default(),
        last_used: std::time::Instant::now(),
    });
    assert_eq!(
        super::workspace_attach_name(&st, 1, Some(&epoch)).unwrap(),
        "original"
    );
    st.sessions[0].name = "renamed".into();
    assert_eq!(
        super::workspace_attach_name(&st, 1, Some(&epoch)).unwrap(),
        "renamed"
    );
    assert!(super::workspace_attach_name(&st, 2, Some(&epoch)).is_err());

    // A new daemon can have the same ID and name; neither makes an old
    // cached selection valid in the new lifetime.
    st.events = crate::events::Journal::default();
    st.sessions[0].name = "original".into();
    assert_ne!(st.events.epoch(), epoch);
    let error = super::workspace_attach_name(&st, 1, Some(&epoch)).unwrap_err();
    assert!(error.to_string().contains("server changed since selection"));
    assert!(st.clients.is_empty());
    assert_eq!(
        super::workspace_attach_name(&st, 1, Some(st.events.epoch())).unwrap(),
        "original"
    );
    // Explicit legacy ID attachment retains its existing contract.
    assert_eq!(
        super::workspace_attach_name(&st, 1, None).unwrap(),
        "original"
    );
}

#[test]
fn pane_area_leaves_row_zero_to_the_toolbar() {
    use crate::layout::Rect;
    let mut st = super::test_state();
    let s = super::test_session("t", 80, 24, 7);
    // test_state keeps the legacy geometry
    assert!(!st.toolbar_shown(&s));
    assert_eq!(
        st.pane_area(&s),
        Rect {
            x: 0,
            y: 0,
            w: 80,
            h: 23
        }
    );
    st.cfg.toolbar = true;
    assert!(st.toolbar_shown(&s));
    assert_eq!(
        st.pane_area(&s),
        Rect {
            x: 0,
            y: 1,
            w: 80,
            h: 22
        }
    );
    let rects = st.visible_rects(&s, &s.windows[0]);
    assert_eq!(
        rects,
        vec![(
            7,
            Rect {
                x: 0,
                y: 1,
                w: 80,
                h: 22
            }
        )]
    );
    // the sidebar keeps its columns; the pane area (and toolbar) narrows
    let mut side = super::test_session("t", 80, 24, 7);
    side.sidebar = true;
    assert_eq!(
        st.pane_area(&side),
        Rect {
            x: 0,
            y: 1,
            w: 52,
            h: 22
        }
    );
}

#[test]
fn toolbar_needs_four_rows() {
    use crate::layout::Rect;
    let mut st = super::test_state();
    st.cfg.toolbar = true;
    let s = super::test_session("t", 40, 4, 1);
    assert_eq!(
        st.pane_area(&s),
        Rect {
            x: 0,
            y: 1,
            w: 40,
            h: 2
        }
    );
    let s = super::test_session("t", 40, 3, 1);
    assert!(!st.toolbar_shown(&s));
    assert_eq!(
        st.pane_area(&s),
        Rect {
            x: 0,
            y: 0,
            w: 40,
            h: 2
        }
    );
}

#[test]
fn split_layout_starts_below_the_toolbar() {
    use crate::layout::{Dir, Node, Rect};
    let mut st = super::test_state();
    st.cfg.toolbar = true;
    let mut s = super::test_session("t", 80, 24, 1);
    s.windows[0].layout = Node::Split {
        dir: Dir::V,
        ratio: 0.5,
        a: Box::new(Node::Leaf(1)),
        b: Box::new(Node::Leaf(2)),
    };
    let rects = st.visible_rects(&s, &s.windows[0]);
    assert_eq!(
        rects[0],
        (
            1,
            Rect {
                x: 0,
                y: 1,
                w: 80,
                h: 11
            }
        )
    );
    assert_eq!(
        rects[1],
        (
            2,
            Rect {
                x: 0,
                y: 13,
                w: 80,
                h: 10
            }
        )
    );
    assert_eq!(rects[1].1.y + rects[1].1.h, 23, "the status row stays free");
}

#[test]
fn title_policy_auto_needs_split_height_and_info() {
    // auto: split, unzoomed, >= 6 rows, and an agent report or label
    assert!(wants_title(TitleMode::Auto, true, false, 6, true));
    assert!(
        !wants_title(TitleMode::Auto, true, false, 5, true),
        "too short"
    );
    assert!(
        !wants_title(TitleMode::Auto, true, false, 20, false),
        "plain pane"
    );
    assert!(
        !wants_title(TitleMode::Auto, false, false, 20, true),
        "single pane"
    );
    assert!(
        !wants_title(TitleMode::Auto, true, true, 20, true),
        "zoomed"
    );
}

#[test]
fn title_policy_always_and_off() {
    assert!(wants_title(TitleMode::Always, true, false, 2, false));
    assert!(
        !wants_title(TitleMode::Always, true, false, 1, true),
        "no room for content"
    );
    assert!(!wants_title(TitleMode::Always, false, false, 20, true));
    assert!(!wants_title(TitleMode::Off, true, false, 20, true));
}

#[test]
fn vt100_wide_char_resize_panic_is_contained() {
    let mut p = vt100::Parser::new(5, 4, 0);
    p.process("中中中中".as_bytes());
    p.screen_mut().set_size(5, 3);
    let r = contain_vt(&mut p, 5, 3, 0, |vt| vt.process("x中y".as_bytes()));
    assert!(
        r.is_none(),
        "vt100 no longer panics here; the guard can be revisited"
    );
    assert_eq!(p.screen().size(), (5, 3));
    p.process(b"ok");
    assert!(p.screen().contents().contains("ok"));
    assert_eq!(contain_vt(&mut p, 5, 3, 0, |_| 7), Some(7));
}

#[test]
fn agent_session_vars_are_not_inherited_by_panes() {
    for name in [
        "CLAUDECODE",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_MESSAGING_TOKEN",
        "claude_code_entrypoint",
        "CLAUDE_PID",
        "CLAUDE_EFFORT",
    ] {
        assert!(super::is_agent_session_var(name), "{name}");
    }
    for name in [
        "CLAUDE_CONFIG_DIR",
        "ANTHROPIC_API_KEY",
        "PATH",
        "CLAUDE",
        "CODEX_HOME",
    ] {
        assert!(
            !super::is_agent_session_var(name),
            "{name} is user configuration"
        );
    }
}

#[test]
fn pane_user_env_drops_retired_tmuxlike_names_from_restored_or_supplied_env() {
    let env = std::collections::BTreeMap::from([
        ("TMUXLIKE_PANE_ID".to_string(), "9".to_string()),
        ("TMUXLIKE".to_string(), "x".to_string()),
        ("FOO".to_string(), "1".to_string()),
        ("CREW_KEEP".to_string(), "2".to_string()),
    ]);
    let kept: Vec<&str> = super::pane_user_env(&env)
        .map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(kept, vec!["CREW_KEEP", "FOO"]);
}

#[test]
fn legacy_tmuxlike_vars_are_stripped_from_panes_but_crew_vars_are_kept() {
    for name in [
        "TMUXLIKE",
        "TMUXLIKE_PANE_ID",
        "TMUXLIKE_SOCKET",
        "tmuxlike_state",
    ] {
        assert!(crate::brand::is_legacy_env_name(name), "{name}");
    }
    for name in ["CREW", "CREW_PANE_ID", "CREW_SOCKET", "TMUXLIKEX", "PATH"] {
        assert!(!crate::brand::is_legacy_env_name(name), "{name}");
    }
}
