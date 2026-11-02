use super::*;
use crate::server::{test_client, test_session, test_state};

#[test]
fn aurora_default_content_budget_is_at_least_v060() {
    use crate::layout::{Dir, Node};
    for (cols, rows) in [(120, 30), (80, 24)] {
        let mut old = test_state();
        old.cfg = crate::config::Config::default();
        old.cfg.theme.preset = "studio".into();
        let mut new = test_state();
        new.cfg = crate::config::Config::default();
        new.cfg.set("theme", "aurora").unwrap();
        assert!(
            !new.cfg.toolbar
                && !new.cfg.sidebar
                && !new.cfg.workspace_tree
                && !new.cfg.changed_files
        );
        let mut session = test_session("budget", cols, rows, 1);
        let before = old.pane_area(&session);
        let after = new.pane_area(&session);
        assert_eq!(
            before,
            Rect {
                x: 0,
                y: 2,
                w: cols,
                h: rows - 3
            }
        );
        assert_eq!(
            after,
            Rect {
                x: 0,
                y: 0,
                w: cols,
                h: rows - 1
            }
        );
        assert!(
            u32::from(after.w) * u32::from(after.h) >= u32::from(before.w) * u32::from(before.h)
        );
        assert_eq!(new.tab_row(&session), rows - 1);
        new.cfg.workspace_tree = true;
        session.rows = 3;
        assert_eq!(new.tree_width(&session), 0);
        session.rows = rows;
        new.cfg.workspace_tree = false;
        // Explicit toolbar choices and split headers still cost exactly the old rows.
        session.windows[0].layout = Node::Split {
            dir: Dir::H,
            ratio: 0.5,
            a: Box::new(Node::Leaf(1)),
            b: Box::new(Node::Leaf(2)),
        };
        old.cfg.pane_titles = "always".into();
        new.cfg.pane_titles = "always".into();
        for toolbar in [false, true] {
            old.cfg.toolbar = toolbar;
            new.cfg.toolbar = toolbar;
            assert_eq!(new.pane_area(&session), old.pane_area(&session));
            assert_eq!(
                new.visible_rects(&session, &session.windows[0]),
                old.visible_rects(&session, &session.windows[0])
            );
        }
    }
}

#[test]
fn narrow_unicode_tabs_keep_active_visible_and_hit_cells_match() {
    for cols in [20, 40, 80, 120, 200] {
        for sidebar in [false, true] {
            let mut st = test_state();
            st.cfg.theme.preset = "aurora".into();
            st.cfg.changed_files = true;
            st.cfg.workspace_tree = sidebar;
            let mut session = test_session("工作室 👩‍💻 Aurora", cols, 30, 1);
            session.sidebar = sidebar;
            for index in 1..15 {
                let mut window = test_session("unused", cols, 30, index + 1)
                    .windows
                    .remove(0);
                window.name = format!("中文工作 {index}");
                session.windows.push(window);
            }
            session.active = 13;
            st.sessions.push(session);
            let s = &st.sessions[0];
            let c = test_client(&s.name);
            let t = Theme::resolve(&st.cfg.theme, &c.env);
            let runs = tab_runs(&st, &c, s, &t);
            assert!(
                runs.iter().any(|r| r.hit == Some(Hit::Tab(0, 13))),
                "cols={cols}, sidebar={sidebar}"
            );
            for run in runs {
                assert!(usize::from(run.x) + run.text.width() <= usize::from(s.cols));
                if let Some(Hit::Tab(_, wi)) = run.hit {
                    for cell in run.x..run.x + run.text.width() as u16 {
                        assert_eq!(tab_at(&st, &c, s, cell), Some(wi));
                        assert_eq!(hit_at(&st, &c, s, cell, run.y), run.hit);
                    }
                }
            }
            for run in tree::tree_runs(&st, &c, s, &t) {
                assert!(usize::from(run.x) + run.text.width() <= usize::from(st.tree_width(s)));
                assert!(run.y < s.rows - 1);
            }
        }
    }
}

#[test]
fn footer_focus_badge_and_restored_shell_use_live_pane_evidence() {
    use crate::api::{AgentInfo, Status};
    use std::sync::{Arc, Mutex};
    struct Fixture(crate::server::Shared);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let mut st = crate::api::lock(&self.0);
            while !st.sessions.is_empty() {
                st.kill_session(0);
            }
        }
    }
    let mut st = test_state();
    st.cfg.theme.preset = "aurora".into();
    st.cfg.shell = Some(
        if cfg!(windows) {
            "cmd.exe /Q"
        } else {
            "/bin/sh"
        }
        .into(),
    );
    let shared = Arc::new(Mutex::new(st));
    let _fixture = Fixture(shared.clone());
    let mut st = crate::api::lock(&shared);
    st.new_session(&shared, Some("aurora".into()), 120, 30, None)
        .unwrap();
    let pid = st.active_pane(0);
    let p = st.panes.get_mut(&pid).unwrap();
    p.restored_shell = true;
    assert_eq!(crate::persist::restore_state(p), Some("restored-shell"));
    p.attention = true;
    p.last_output = std::time::Instant::now() - std::time::Duration::from_secs(10);
    assert!(crate::commands::attention_items(&st).is_empty());
    assert_eq!(
        crate::semantic::pane(&st.panes[&pid], &st.cfg).word(Lang::En),
        "restored shell"
    );
    st.write_pane(pid, b"");
    assert_eq!(
        crate::persist::restore_state(&st.panes[&pid]),
        Some("restored-shell")
    );
    st.write_pane(pid, b" ");
    assert_eq!(crate::persist::restore_state(&st.panes[&pid]), None);
    let p = st.panes.get_mut(&pid).unwrap();
    p.restored_shell = true;
    p.agent = Some(AgentInfo {
        instance: 1,
        source: "test".into(),
        agent: "codex".into(),
        state: Status::Blocked,
        message: None,
        seq: None,
        name: Some("codex".into()),
        resume: Vec::new(),
    });
    assert_eq!(
        crate::persist::restore_state(p),
        None,
        "live reporter claims restored shell"
    );
    st.chrome.snapshot.inbox.insert("codex".into(), 3);
    let c = test_client("aurora");
    let t = Theme::resolve(&st.cfg.theme, &c.env);
    let runs = footer_runs(&st, &c, &st.sessions[0], &t);
    let chip = runs.iter().find(|r| r.hit == Some(Hit::Pane(pid))).unwrap();
    assert!(chip.text.contains("needs input"));
    assert!(chip.text.contains("[3]"));
    assert_eq!(
        hit_at(&st, &c, &st.sessions[0], chip.x, chip.y),
        Some(Hit::Pane(pid))
    );
    st.panes
        .get_mut(&pid)
        .unwrap()
        .agent
        .as_mut()
        .unwrap()
        .state = Status::Working;
    drop(st);
    // A real report can complete long after its last terminal output.
    let report = || crate::api::Request::PaneReportAgent {
        pane: pid,
        source: "test".into(),
        agent: "codex".into(),
        state: Status::Idle,
        message: None,
        seq: None,
        resume: Vec::new(),
    };
    crate::api::agent::handle(&shared, report()).unwrap();
    let first_done = {
        let st = crate::api::lock(&shared);
        let visual = crate::semantic::pane(&st.panes[&pid], &st.cfg);
        assert_eq!(visual.state, crate::semantic::DisplayState::Done);
        st.panes[&pid].done_since.unwrap()
    };
    crate::api::agent::handle(&shared, report()).unwrap();
    let mut st = crate::api::lock(&shared);
    assert_eq!(
        st.panes[&pid].done_since,
        Some(first_done),
        "heartbeat does not restart visual decay"
    );
    st.panes.get_mut(&pid).unwrap().done_since =
        Some(first_done - std::time::Duration::from_secs(6));
    let display = crate::metadata::display::pane(&st, pid, Lang::En).unwrap();
    assert_eq!(display.semantic.state, crate::semantic::DisplayState::Idle);
    assert_eq!(display.state_label, "idle");
    assert_eq!(display.semantic.status, Status::Done);
    st.panes
        .get_mut(&pid)
        .unwrap()
        .agent
        .as_mut()
        .unwrap()
        .source = crate::api::RESTORE_SOURCE.into();
    assert!(
        !footer_runs(&st, &c, &st.sessions[0], &t)
            .iter()
            .any(|r| r.hit == Some(Hit::Pane(pid)))
    );
}
