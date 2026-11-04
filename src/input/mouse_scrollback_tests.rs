//! Handler-level mode tests: ConPTY consumes DECSET before it reaches Crew, so
//! these feed the parser directly and capture the exact bytes sent to the child.
use super::*;
use crate::server::{Shared, test_client, test_session, test_state};
use crossterm::event::{Event, KeyCode, KeyEvent};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone)]
struct InertPty;

impl portable_pty::MasterPty for InertPty {
    fn resize(&self, _: portable_pty::PtySize) -> anyhow::Result<()> {
        Ok(())
    }
    fn get_size(&self) -> anyhow::Result<portable_pty::PtySize> {
        Ok(portable_pty::PtySize {
            rows: 5,
            cols: 40,
            pixel_width: 0,
            pixel_height: 0,
        })
    }
    fn try_clone_reader(&self) -> anyhow::Result<Box<dyn io::Read + Send>> {
        Ok(Box::new(io::empty()))
    }
    fn take_writer(&self) -> anyhow::Result<Box<dyn Write + Send>> {
        Ok(Box::new(io::sink()))
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

impl portable_pty::ChildKiller for InertPty {
    fn kill(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(self.clone())
    }
}

impl portable_pty::Child for InertPty {
    fn try_wait(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
        Ok(None)
    }
    fn wait(&mut self) -> io::Result<portable_pty::ExitStatus> {
        Ok(portable_pty::ExitStatus::with_exit_code(0))
    }
    fn process_id(&self) -> Option<u32> {
        None
    }
    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}

#[derive(Clone, Default)]
struct Input(Arc<Mutex<Vec<u8>>>);

impl Write for Input {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn fixture() -> (State, Shared, usize, Input) {
    let mut state = test_state();
    let shared = Arc::new(Mutex::new(test_state()));
    let pid = 7;
    let input = Input::default();
    let mut pane = crate::server::Pane {
        parser: crate::server::PaneParser::new_with_callbacks(5, 40, 100, Default::default()),
        history_view: None,
        master: Box::new(InertPty),
        writer: Box::new(input.clone()),
        child: Box::new(InertPty),
        rect: Rect {
            x: 4,
            y: 2,
            w: 40,
            h: 5,
        },
        last_output: Instant::now(),
        done_since: None,
        attention: false,
        auto_focus_disabled: false,
        started: Instant::now(),
        copy: None,
        agent: None,
        resume_requested: false,
        restored_shell: false,
        detected_agent: None,
        label: None,
        cwd: None,
        env: Default::default(),
        plugin_path_dirs: Vec::new(),
        plugin_path_diagnostics: Vec::new(),
        host_lost: None,
        root_exited: None,
    };
    for line in 0..40 {
        pane.parser
            .process(format!("history-{line:02}\r\n").as_bytes());
    }
    state.panes.insert(pid, pane);
    state.sessions.push(test_session("t", 80, 12, pid));
    state.clients.insert(1, test_client("t"));
    (state, shared, pid, input)
}

fn key(state: &mut State, shared: &Shared, code: KeyCode, modifiers: KeyModifiers) {
    state.handle_event(shared, 1, Event::Key(KeyEvent::new(code, modifiers)));
}

fn wheel(state: &mut State, pid: usize, kind: MouseEventKind) {
    let r = state.panes[&pid].rect;
    state.wheel_pane(
        pid,
        r,
        MouseEvent {
            kind,
            column: r.x + 3,
            row: r.y + 2,
            modifiers: KeyModifiers::NONE,
        },
    );
}

#[test]
fn alternate_wheel_browses_saved_output_without_sending_history_arrows() {
    let (mut state, shared, pid, input) = fixture();
    state
        .panes
        .get_mut(&pid)
        .unwrap()
        .parser
        .process(b"\x1b[?1049hALT-LIVE");
    wheel(&mut state, pid, MouseEventKind::ScrollUp);
    assert!(
        input.0.lock().unwrap().is_empty(),
        "wheel must not become Up keys"
    );
    assert!(state.capture(Some("t")).unwrap().contains("history-33"));
    assert!(state.panes[&pid].parser.screen().alternate_screen());
    state
        .panes
        .get_mut(&pid)
        .unwrap()
        .parser
        .process(b"\r\nNEW-LIVE");
    assert!(!state.capture(Some("t")).unwrap().contains("NEW-LIVE"));
    key(&mut state, &shared, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(state.capture(Some("t")).unwrap().contains("NEW-LIVE"));
    assert!(input.0.lock().unwrap().is_empty(), "q stays in Crew");
    key(&mut state, &shared, KeyCode::Up, KeyModifiers::NONE);
    key(&mut state, &shared, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(*input.0.lock().unwrap(), b"\x1b[A\x1b[B");
    input.0.lock().unwrap().clear();
    key(&mut state, &shared, KeyCode::PageUp, KeyModifiers::SHIFT);
    assert!(state.capture(Some("t")).unwrap().contains("history-31"));
    key(&mut state, &shared, KeyCode::Esc, KeyModifiers::NONE);
    assert!(state.capture(Some("t")).unwrap().contains("NEW-LIVE"));
    assert!(input.0.lock().unwrap().is_empty());
    key(
        &mut state,
        &shared,
        KeyCode::Char('b'),
        KeyModifiers::CONTROL,
    );
    key(&mut state, &shared, KeyCode::Char('['), KeyModifiers::NONE);
    key(&mut state, &shared, KeyCode::PageUp, KeyModifiers::NONE);
    assert!(state.capture(Some("t")).unwrap().contains("history-31"));
    key(&mut state, &shared, KeyCode::Esc, KeyModifiers::NONE);
    assert!(state.panes[&pid].copy.is_none());
    assert!(state.capture(Some("t")).unwrap().contains("NEW-LIVE"));
}

#[test]
fn requested_mouse_wheel_uses_local_coordinates_and_copy_mode_keeps_control() {
    let (mut state, shared, pid, input) = fixture();
    for alternate in [false, true] {
        if alternate {
            state
                .panes
                .get_mut(&pid)
                .unwrap()
                .parser
                .process(b"\x1b[?1049hAPP");
        }
        for (mode, expected) in [
            (
                b"\x1b[?1000h\x1b[?1006h".as_slice(),
                b"\x1b[<64;4;3M\x1b[<65;4;3M".as_slice(),
            ),
            (b"\x1b[?1006l\x1b[?1005l", b"\x1b[M`$#\x1b[Ma$#"),
            (b"\x1b[?1005h", b"\x1b[M`$#\x1b[Ma$#"),
        ] {
            state.panes.get_mut(&pid).unwrap().parser.process(mode);
            wheel(&mut state, pid, MouseEventKind::ScrollUp);
            wheel(&mut state, pid, MouseEventKind::ScrollDown);
            assert_eq!(*input.0.lock().unwrap(), expected, "alternate={alternate}");
            input.0.lock().unwrap().clear();
            key(&mut state, &shared, KeyCode::PageUp, KeyModifiers::SHIFT);
            assert!(state.capture(Some("t")).unwrap().contains("history-31"));
            wheel(&mut state, pid, MouseEventKind::ScrollUp);
            assert!(input.0.lock().unwrap().is_empty());
            key(&mut state, &shared, KeyCode::Esc, KeyModifiers::NONE);
            assert!(state.panes[&pid].copy.is_none());
        }
    }
}

#[test]
fn wheel_encodings_preserve_modifiers_and_bound_legacy_coordinates() {
    use vt100::MouseProtocolEncoding::{Default, Sgr, Utf8};
    let r = Rect {
        x: 4,
        y: 2,
        w: 4096,
        h: 5,
    };
    let mut event = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 113,
        row: 4,
        modifiers: KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL,
    };
    assert_eq!(wheel_bytes(Sgr, r, event), b"\x1b[<92;110;3M");
    assert_eq!(wheel_bytes(Utf8, r, event), "\x1b[M|\u{8e}#".as_bytes());
    assert_eq!(
        wheel_bytes(Default, r, event),
        [0x1b, b'[', b'M', 124, 142, 35]
    );
    event.column = r.x + 223;
    assert!(wheel_bytes(Default, r, event).is_empty());
    event.column = r.x + 2015;
    assert!(wheel_bytes(Utf8, r, event).is_empty());
    assert_eq!(wheel_bytes(Sgr, r, event), b"\x1b[<92;2016;3M");
}

#[test]
fn wheel_over_unfocused_split_gives_its_exit_keys_the_same_target() {
    let (mut state, shared, pid, input) = fixture();
    let (mut other, _, other_pid, other_input) = fixture();
    state
        .panes
        .insert(8, other.panes.remove(&other_pid).unwrap());
    let window = &mut state.sessions[0].windows[0];
    window.layout = crate::layout::Node::Split {
        dir: crate::layout::Dir::H,
        ratio: 0.5,
        a: Box::new(crate::layout::Node::Leaf(pid)),
        b: Box::new(crate::layout::Node::Leaf(8)),
    };
    window.active = 8;
    state.relayout(0);
    let r = state.panes[&pid].rect;
    state.handle_event(
        &shared,
        1,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: r.x + 2,
            row: r.y + 2,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(state.sessions[0].windows[0].active, pid);
    assert!(state.panes[&pid].copy.is_some());
    key(&mut state, &shared, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(state.panes[&pid].copy.is_none());
    assert!(input.0.lock().unwrap().is_empty());
    assert!(
        other_input.0.lock().unwrap().is_empty(),
        "q must not reach the old focus"
    );
}

#[test]
fn saved_view_resize_copy_and_api_reads_keep_live_output_separate() {
    let (mut state, shared, pid, input) = fixture();
    state
        .panes
        .get_mut(&pid)
        .unwrap()
        .parser
        .process(b"\x1b[?1049hALT-LIVE");
    key(&mut state, &shared, KeyCode::PageUp, KeyModifiers::SHIFT);
    let visible =
        crate::api::pane::read_text(&mut state, pid, crate::api::ReadSource::Visible, None)
            .unwrap();
    let recent =
        crate::api::pane::read_text(&mut state, pid, crate::api::ReadSource::Recent, None).unwrap();
    assert!(visible.contains("history-31"));
    assert!(recent.contains("ALT-LIVE") && !recent.contains("history-"));
    state.sessions[0].cols = 70;
    state.sessions[0].rows = 10;
    state.relayout(0);
    let pane = &state.panes[&pid];
    assert_eq!(pane.view().screen().size(), (pane.rect.h, pane.rect.w));
    assert!(pane.parser.screen().alternate_screen());
    key(&mut state, &shared, KeyCode::Char('g'), KeyModifiers::NONE);
    key(&mut state, &shared, KeyCode::Char('V'), KeyModifiers::NONE);
    key(&mut state, &shared, KeyCode::Char('y'), KeyModifiers::NONE);
    assert_eq!(state.buffer.as_deref(), Some("history-00\n"));
    assert!(state.panes[&pid].copy.is_none());
    assert!(state.panes[&pid].history_view.is_none());
    assert!(state.capture(Some("t")).unwrap().contains("ALT-LIVE"));
    assert!(input.0.lock().unwrap().is_empty());
}
