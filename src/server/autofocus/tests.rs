use super::*;

fn policy() -> Policy {
    Policy {
        mode: Mode::On,
        scope: Scope::Session,
        idle: Duration::from_millis(2500),
        dwell: Duration::from_millis(4000),
    }
}
fn viewer() -> Viewer {
    Viewer {
        id: 1,
        session: 1,
        window: 1,
        pane: 1,
        last_input: None,
        last_jump: None,
        guarded: false,
    }
}
fn pane(id: usize, status: Status, generation: u64) -> Candidate {
    Candidate {
        edge: Edge {
            pane: id,
            status,
            generation,
        },
        session: 1,
        window: 1,
        enabled: true,
        changes_shared_view: true,
    }
}

#[test]
fn exact_typing_and_dwell_thresholds_and_all_viewer_guards() {
    let now = Instant::now();
    let p = pane(2, Status::Blocked, 1);
    let q = [p.edge];
    for (input, dwell, want) in [
        (Some(2499), None, false),
        (Some(2500), None, true),
        (None, Some(3999), false),
        (None, Some(4000), true),
        (None, None, true),
    ] {
        let mut v = viewer();
        v.last_input = input.map(|ms| now - Duration::from_millis(ms));
        v.last_jump = dwell.map(|ms| now - Duration::from_millis(ms));
        assert_eq!(
            decide(&q, &[p], &v, &[v], policy(), now).0,
            want.then_some(2)
        );
    }
    let mut v = viewer();
    v.guarded = true;
    assert_eq!(
        decide(&q, &[p], &v, &[v], policy(), now),
        (None, Some(p.edge))
    );
    v.guarded = false;
    for (guarded, typing, dwell) in [
        (true, None, None),
        (false, Some(now), None),
        (false, None, Some(now)),
    ] {
        let other = Viewer {
            id: 2,
            guarded,
            last_input: typing,
            last_jump: dwell,
            ..v
        };
        assert_eq!(decide(&q, &[p], &v, &[v, other], policy(), now).0, None);
        let unaffected = Viewer {
            session: 2,
            ..other
        };
        assert_eq!(
            decide(&q, &[p], &v, &[v, unaffected], policy(), now).0,
            Some(2)
        );
        let unchanged = Candidate {
            changes_shared_view: false,
            ..p
        };
        assert_eq!(
            decide(&q, &[unchanged], &v, &[v, other], policy(), now).0,
            Some(2)
        );
    }
}

#[test]
fn priority_modes_scope_stale_occupants_optout_and_focused_targets() {
    let now = Instant::now();
    let v = viewer();
    let done = pane(2, Status::Done, 1);
    let distant = Candidate {
        window: 2,
        ..pane(3, Status::Blocked, 2)
    };
    let local = pane(4, Status::Blocked, 3);
    let newer = pane(5, Status::Blocked, 4);
    let panes = [done, distant, newer, local];
    let q = panes.map(|p| p.edge);
    assert_eq!(decide(&q, &panes, &v, &[v], policy(), now).0, Some(4));
    assert_eq!(
        decide(
            &q,
            &panes,
            &v,
            &[v],
            Policy {
                mode: Mode::Off,
                ..policy()
            },
            now
        )
        .0,
        None
    );
    assert_eq!(
        decide(
            &[done.edge],
            &[done],
            &v,
            &[v],
            Policy {
                mode: Mode::Blocked,
                ..policy()
            },
            now
        )
        .0,
        None
    );
    for scope in [Scope::Window, Scope::Session, Scope::All] {
        for (session, window) in [(1, 1), (1, 2), (2, 2)] {
            let p = Candidate {
                session,
                window,
                ..distant
            };
            let allowed = match scope {
                Scope::Window => session == 1 && window == 1,
                Scope::Session => session == 1,
                Scope::All => true,
            };
            assert_eq!(
                decide(&[p.edge], &[p], &v, &[v], Policy { scope, ..policy() }, now).0,
                allowed.then_some(3)
            );
        }
    }
    for changed in [
        Candidate {
            enabled: false,
            ..local
        },
        Candidate {
            edge: Edge {
                generation: 99,
                ..local.edge
            },
            ..local
        },
        Candidate {
            edge: Edge {
                status: Status::Working,
                ..local.edge
            },
            ..local
        },
    ] {
        assert_eq!(
            decide(&[local.edge], &[changed], &v, &[v], policy(), now).0,
            None
        );
    }
    assert_eq!(
        decide(
            &[local.edge],
            &[local],
            &Viewer { pane: 4, ..v },
            &[v],
            policy(),
            now
        )
        .0,
        None
    );
}

#[test]
fn config_defaults_aliases_and_key_overrides_are_safe() {
    let mut cfg = crate::config::Config::default();
    assert_eq!(cfg.auto_focus, Mode::Off);
    assert_eq!(cfg.auto_focus_scope, Scope::Session);
    assert_eq!(
        (cfg.auto_focus_idle_ms, cfg.auto_focus_dwell_ms),
        (2500, 4000)
    );
    let parsed:crate::config::Config=toml::from_str("auto-focus='blocked'\nauto-focus-scope='all'\nauto-focus-idle-ms=1\nauto-focus-dwell-ms=2\n").unwrap();
    assert_eq!(parsed.auto_focus, Mode::Blocked);
    assert_eq!(parsed.auto_focus_scope, Scope::All);
    for (key, value) in [
        ("auto-focus", "bad"),
        ("auto-focus-scope", "bad"),
        ("auto-focus-idle-ms", "-1"),
    ] {
        assert!(cfg.set(key, value).is_err());
    }
    for theme in ["studio", "aurora"] {
        cfg.theme.preset = theme.into();
        let (bindings, errors) = cfg.bindings();
        assert!(errors.is_empty());
        assert_eq!(bindings["F"], "toggle-auto-focus");
        assert_eq!(bindings["a"], "next-attention");
        assert_eq!(bindings["b"], "toggle-sidebar");
        assert_eq!(bindings[";"], "last-pane");
    }
    cfg.bind.insert("F".into(), "display custom".into());
    assert_eq!(cfg.bindings().0["F"], "display custom");
    assert_eq!(indicator(Mode::On, Lang::En), "AF");
    assert_eq!(indicator(Mode::Blocked, Lang::ZhTw), "自焦:待回應");
}

#[test]
fn edges_do_not_repeat_and_old_occupants_cannot_match_new_generations() {
    let mut tracker = Tracker::default();
    assert_eq!(tracker.observe(2, Status::Working, Some(1)), None);
    let blocked = tracker.observe(2, Status::Blocked, Some(1)).unwrap();
    assert_eq!(tracker.observe(2, Status::Blocked, Some(1)), None);
    let replaced = tracker.observe(2, Status::Blocked, Some(2)).unwrap();
    assert_ne!(blocked.generation, replaced.generation);
    assert_eq!(tracker.observe(2, Status::Working, Some(2)), None);
    let done = tracker.observe(2, Status::Done, Some(2)).unwrap();
    assert_eq!(tracker.observe(2, Status::Done, Some(2)), None);
    assert!(done.generation > replaced.generation);
}

#[test]
fn input_activity_excludes_resize_and_key_release_and_view_modes_are_guarded() {
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
    };
    let mut st = super::super::test_state();
    st.cfg.mouse = false;
    st.sessions
        .push(super::super::test_session("private", 120, 30, 1));
    st.clients.insert(1, super::super::test_client("private"));
    let shared = std::sync::Arc::new(std::sync::Mutex::new(st));
    let events = [
        (Event::Resize(120, 30), false),
        (
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('x'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
            false,
        ),
        (
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            true,
        ),
        (Event::Paste("test".into()), true),
        (
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
            true,
        ),
    ];
    for (event, want) in events {
        let mut st = shared.lock().unwrap();
        st.clients.get_mut(&1).unwrap().focus.last_input = None;
        st.handle_event(&shared, 1, event);
        assert_eq!(st.clients[&1].focus.last_input.is_some(), want);
    }
    for which in ["prefix", "prompt", "chooser", "popup", "drag"] {
        let mut st = shared.lock().unwrap();
        let client = st.clients.get_mut(&1).unwrap();
        match which {
            "prefix" => client.prefix = true,
            "prompt" => {
                client.prompt = Some(super::super::Prompt::new(
                    super::super::PromptKind::Confirm,
                    "question".into(),
                ))
            }
            "chooser" => client.chooser = Some(0),
            "popup" => client.popup = Some((vec![], 0)),
            _ => {
                client.drag = Some(super::super::Drag::Copy {
                    pane: 1,
                    row: 0,
                    col: 0,
                    moved: false,
                })
            }
        }
        assert!(st.autofocus_viewers()[0].guarded, "{which}");
        let client = st.clients.get_mut(&1).unwrap();
        client.prefix = false;
        client.prompt = None;
        client.chooser = None;
        client.popup = None;
        client.drag = None;
    }
}

#[test]
fn cross_session_return_preserves_prior_identity_and_unzooms_target() {
    let mut st = super::super::test_state();
    st.sessions
        .push(super::super::test_session("first", 120, 30, 1));
    let mut second = super::super::test_session("second", 120, 30, 2);
    second.id = 2;
    second.windows[0].id = 2;
    second.windows[0].zoomed = true;
    st.sessions.push(second);
    st.clients.insert(1, super::super::test_client("first"));
    st.clients.insert(2, super::super::test_client("second"));
    st.autofocus_jump(1, 2, true, Instant::now());
    assert_eq!(st.clients[&1].session, "second");
    assert_eq!(st.clients[&1].focus.prior, Some(1));
    assert!(!st.sessions[1].windows[0].zoomed);
    assert!(st.clients[&2].focus.last_jump.is_some());
    st.last_pane(1);
    assert_eq!(st.clients[&1].session, "first");
    assert_eq!(st.active_pane(0), 1);
    assert_eq!(st.clients[&1].focus.prior, Some(2));
}
