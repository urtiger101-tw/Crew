use crate::commands::{Cmd, DEFAULT_BINDINGS};
use crate::layout::{Node, Preset};
use crate::server::{test_client, test_session, test_state};
use std::sync::{Arc, Mutex};

#[test]
fn next_layout_tracks_all_presets_even_when_their_two_pane_geometry_matches() {
    let mut st = test_state();
    let mut session = test_session("private", 40, 11, 9);
    session.windows[0].layout = Preset::EvenVertical.build(&[9, 3]).unwrap();
    session.windows[0].zoomed = true;
    st.sessions.push(session);
    st.clients.insert(1, test_client("private"));
    let shared = Arc::new(Mutex::new(st));
    for expected in Preset::ALL.into_iter().chain([Preset::EvenHorizontal]) {
        let mut st = shared.lock().unwrap();
        st.exec(&shared, 1, Cmd::NextLayout).unwrap();
        let window = &st.sessions[0].windows[0];
        assert_eq!(window.layout_preset, Some(expected));
        assert_eq!(window.active, 9);
        assert!(!window.zoomed);
        assert_eq!(window.panes(), [9, 3]);
        assert_eq!(window.layout, expected.build(&[9, 3]).unwrap());
    }
    let mut st = shared.lock().unwrap();
    assert!(!st.select_layout(0, 0, Preset::EvenHorizontal));
    st.sessions[0].windows[0].layout = Node::Leaf(9);
    let before = st.sessions[0].windows[0].layout_preset;
    st.sessions[0].windows[0].zoomed = true;
    assert!(!st.select_layout(0, 0, Preset::Tiled));
    assert!(st.sessions[0].windows[0].zoomed, "one pane is a full no-op");
    assert_eq!(st.sessions[0].windows[0].layout_preset, before);
}

#[test]
fn default_layout_keys_are_unique_and_custom_bindings_win() {
    let mut keys = std::collections::BTreeSet::new();
    for (key, _) in DEFAULT_BINDINGS {
        assert!(
            keys.insert(crate::commands::normalize_key(key).unwrap()),
            "duplicate {key}"
        );
    }
    let mut cfg = crate::config::Config::default();
    cfg.bind.insert("M-1".into(), "select-window -t 1".into());
    let (bindings, errors) = cfg.bindings();
    assert!(errors.is_empty());
    assert_eq!(bindings["Space"], "next-layout");
    assert_eq!(bindings["M-1"], "select-window -t 1");
    for (i, preset) in Preset::ALL.into_iter().enumerate().skip(1) {
        assert_eq!(
            bindings[&format!("M-{}", i + 1)],
            format!("select-layout {}", preset.name())
        );
    }
}

#[test]
fn layouts_resize_each_existing_pty_at_most_once_without_input_or_history_reset() {
    use portable_pty::{Child, ChildKiller, ExitStatus, MasterPty, PtySize};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    struct Master(Arc<AtomicUsize>);
    impl MasterPty for Master {
        fn resize(&self, _: PtySize) -> anyhow::Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn get_size(&self) -> anyhow::Result<PtySize> {
            Ok(PtySize {
                rows: 10,
                cols: 40,
                pixel_width: 0,
                pixel_height: 0,
            })
        }
        fn try_clone_reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
            Ok(Box::new(std::io::empty()))
        }
        fn take_writer(&self) -> anyhow::Result<Box<dyn Write + Send>> {
            Ok(Box::new(std::io::sink()))
        }
        #[cfg(unix)]
        fn process_group_leader(&self) -> Option<libc::pid_t> {
            None
        }
        #[cfg(unix)]
        fn as_raw_fd(&self) -> Option<std::os::fd::RawFd> {
            None
        }
        #[cfg(unix)]
        fn tty_name(&self) -> Option<std::path::PathBuf> {
            None
        }
    }
    #[derive(Debug)]
    struct Process(u32);
    impl ChildKiller for Process {
        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(Process(self.0))
        }
    }
    impl Child for Process {
        fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
            Ok(None)
        }
        fn wait(&mut self) -> std::io::Result<ExitStatus> {
            Ok(ExitStatus::with_exit_code(0))
        }
        fn process_id(&self) -> Option<u32> {
            Some(self.0)
        }
        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }
    struct Input(Arc<AtomicUsize>);
    impl Write for Input {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.fetch_add(bytes.len(), Ordering::SeqCst);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut st = test_state();
    st.cfg.pane_titles = "off".into();
    let mut session = test_session("private", 40, 11, 9);
    session.windows[0].layout = Preset::EvenVertical.build(&[9, 3, 7]).unwrap();
    session.windows[0].zoomed = true;
    st.sessions.push(session);
    let input = Arc::new(AtomicUsize::new(0));
    let mut counts = vec![];
    for id in [9, 3, 7] {
        let count = Arc::new(AtomicUsize::new(0));
        counts.push(count.clone());
        let mut parser =
            crate::server::PaneParser::new_with_callbacks(10, 40, 1000, Default::default());
        for _ in 0..40 {
            parser.process(b"KEEP\r\n");
        }
        st.panes.insert(
            id,
            crate::server::Pane {
                parser,
                master: Box::new(Master(count)),
                writer: Box::new(Input(input.clone())),
                child: Box::new(Process(id as u32 + 1000)),
                rect: crate::layout::Rect {
                    x: 0,
                    y: 0,
                    w: 40,
                    h: 10,
                },
                last_output: Instant::now(),
                done_since: None,
                attention: false,
                auto_focus_disabled: false,
                started: Instant::now(),
                copy: Some(crate::copy::CopyState {
                    cx: 0,
                    cy: 0,
                    anchor: None,
                    line: false,
                }),
                agent: None,
                resume_requested: false,
                restored_shell: false,
                detected_agent: None,
                label: None,
                cwd: None,
                env: Default::default(),
                plugin_path_dirs: vec![],
                plugin_path_diagnostics: vec![],
                host_lost: None,
                root_exited: None,
            },
        );
    }
    for preset in Preset::ALL {
        for count in &counts {
            count.store(0, Ordering::SeqCst);
        }
        st.select_layout(0, 0, preset);
        for count in &counts {
            assert!(count.load(Ordering::SeqCst) <= 1);
        }
        for (id, pane) in &mut st.panes {
            assert_eq!(pane.child.process_id(), Some(*id as u32 + 1000));
            assert!(pane.copy.is_some());
            pane.parser.screen_mut().set_scrollback(usize::MAX);
            assert!(pane.parser.screen().contents().contains("KEEP"));
        }
        assert_eq!(st.sessions[0].windows[0].active, 9);
    }
    assert_eq!(input.load(Ordering::SeqCst), 0);
}
