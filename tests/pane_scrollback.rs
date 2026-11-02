//! Real isolated daemon/PTY regression: Crew history and child mouse input stay separate.
//! Windows: place modern conpty.dll + OpenConsole.exe beside the test daemon and set
//! CREW_TEST_MODERN_CONPTY=1 to include alternate-screen and exact mouse-byte checks.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use interprocess::TryClone;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Stream, prelude::*};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Server {
    root: PathBuf,
    socket: String,
}

impl Server {
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
        command
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_CONFIG", self.root.join("config.toml"))
            .env("CREW_STATE", self.root.join("state.json"))
            .env("CREW_LOG", self.root.join("server.log"))
            .env("CREW_LANG", "en")
            .env_remove("CREW")
            .env_remove("CREW_PANE_ID")
            .env_remove("CREW_SERVER_PID");
        command
    }

    fn api(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
    }

    fn mode(&self, sequence: usize, mode: &str) {
        std::fs::write(
            self.root.join("p2").join("control"),
            format!("{sequence} {mode}"),
        )
        .unwrap();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Client {
    stream: Stream,
    output: Receiver<Value>,
    screen: vt100::Parser,
}

impl Client {
    fn send(&mut self, message: Value) {
        let bytes = serde_json::to_vec(&message).unwrap();
        self.stream
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .unwrap();
        self.stream.write_all(&bytes).unwrap();
        self.stream.flush().unwrap();
    }

    fn key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        self.send(json!({"Event": Event::Key(KeyEvent::new(code, modifiers))}));
    }

    fn wheel(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        self.send(json!({"Event": Event::Mouse(MouseEvent {
            kind, column, row, modifiers: KeyModifiers::NONE,
        })}));
    }

    fn wait_for(&mut self, marker: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = self.screen.screen().contents();
            if text.contains(marker) {
                return text;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = self
                .output
                .recv_timeout(remaining)
                .unwrap_or_else(|error| panic!("waiting for {marker}: {error}; frame:\n{text}"));
            if let Some(bytes) = message["Output"].as_str() {
                self.screen.process(bytes.as_bytes());
            } else {
                panic!("unexpected attached-client message: {message}");
            }
        }
    }

    /// A notice on this same attached stream is processed after every preceding input event.
    fn barrier(&mut self, marker: &str) -> String {
        self.send(json!({"Hello": {"Run": {"line": format!("display-message {marker}")}}}));
        self.wait_for(marker)
    }

    /// A raw character after leaving Crew history acknowledges all earlier child input.
    fn assert_input(&mut self, server: &Server, expected: &mut Vec<u8>, bytes: &[u8], fence: char) {
        let previous_len = expected.len();
        expected.extend_from_slice(bytes);
        expected.push(fence as u8);
        self.key(KeyCode::Char(fence), KeyModifiers::NONE);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let actual = std::fs::read(server.root.join("p2").join("input.bin")).unwrap();
            if actual.len() > previous_len && actual.last() == Some(&(fence as u8)) {
                assert_eq!(actual, *expected, "unexpected bytes delivered to the child");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "child did not acknowledge {fence}; expected {expected:?}, got {actual:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn wheel_and_explicit_history_do_not_become_child_command_history() {
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let server = Server {
        root: std::env::temp_dir().join(format!("crew-scrollback-{suffix}")),
        socket: format!("crew-scrollback-{suffix}.sock"),
    };
    std::fs::create_dir_all(&server.root).unwrap();
    let executable = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let shell = format!("\"{executable}\" --ignored --nocapture --exact scrollback_fixture");
    std::fs::write(
        server.root.join("config.toml"),
        format!(
            "persist = false\nautomatic_rename = false\nsidebar = false\ntoolbar = false\nhints = false\npane_titles = \"always\"\nshell = {}\n",
            serde_json::to_string(&shell).unwrap()
        ),
    )
    .unwrap();
    server.api(&[
        "workspace",
        "create",
        "scrollback",
        "--cwd",
        server.root.to_str().unwrap(),
    ]);

    let name = if GenericNamespaced::is_supported() {
        server
            .socket
            .clone()
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
    } else {
        std::env::temp_dir()
            .join(&server.socket)
            .to_fs_name::<GenericFilePath>()
            .unwrap()
    };
    let stream = Stream::connect(name).unwrap();
    let mut reader = stream.try_clone().unwrap();
    let (sender, output) = channel();
    std::thread::spawn(move || {
        loop {
            let mut len = [0; 4];
            if reader.read_exact(&mut len).is_err() {
                break;
            }
            let mut bytes = vec![0; u32::from_le_bytes(len) as usize];
            if reader.read_exact(&mut bytes).is_err() {
                break;
            }
            if sender
                .send(serde_json::from_slice::<Value>(&bytes).unwrap())
                .is_err()
            {
                break;
            }
        }
    });
    let mut client = Client {
        stream,
        output,
        screen: vt100::Parser::new(20, 300, 0),
    };
    client.send(json!({"Hello": {"Attach": {
        "session": "scrollback", "create": false, "cols": 300, "rows": 20, "cwd": null,
    }}}));
    client.wait_for("NORMAL_LIVE_p1");
    server.api(&["pane", "split", "p1", "--direction", "right", "--focus"]);
    client.wait_for("NORMAL_LIVE_p2");
    let pane_cols = server.api(&["pane", "get", "p2"])["pane"]["cols"]
        .as_u64()
        .unwrap() as u16;
    let pane_x = 300 - pane_cols;
    // The right pane's content starts below its one-row title: both offsets are nonzero.
    assert!(pane_x > 0 && pane_cols >= 110);
    let mut expected = Vec::new();
    client.key(KeyCode::Up, KeyModifiers::NONE);
    client.key(KeyCode::Down, KeyModifiers::NONE);
    client.assert_input(&server, &mut expected, b"\x1b[A\x1b[B", 'a');

    client.wheel(MouseEventKind::ScrollUp, pane_x + 9, 4);
    let normal_history = client.barrier("BARRIER_NORMAL_WHEEL");
    client.key(KeyCode::Char('q'), KeyModifiers::NONE);
    client.assert_input(&server, &mut expected, b"", 'b');
    assert!(normal_history.contains("HISTORY_p2_"), "{normal_history}");
    assert!(
        !normal_history.contains("NORMAL_LIVE_p2"),
        "{normal_history}"
    );

    client.key(KeyCode::PageUp, KeyModifiers::SHIFT);
    let shifted_history = client.barrier("BARRIER_NORMAL_SHIFT");
    client.key(KeyCode::Esc, KeyModifiers::NONE);
    client.assert_input(&server, &mut expected, b"", 'c');
    assert!(shifted_history.contains("HISTORY_p2_"), "{shifted_history}");
    assert!(
        !shifted_history.contains("NORMAL_LIVE_p2"),
        "{shifted_history}"
    );

    client.key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    client.key(KeyCode::Char('['), KeyModifiers::NONE);
    client.key(KeyCode::PageUp, KeyModifiers::NONE);
    let prefix_history = client.barrier("BARRIER_NORMAL_PREFIX");
    client.key(KeyCode::Char('q'), KeyModifiers::NONE);
    client.assert_input(&server, &mut expected, b"", 'x');
    assert!(prefix_history.contains("HISTORY_p2_"), "{prefix_history}");
    let live = client.barrier("BARRIER_NORMAL_LIVE");
    assert!(live.contains("NORMAL_LIVE_p2"), "{live}");

    // The legacy OS ConPTY normalizes these DECSET sequences into redraws. Unix PTYs
    // and modern app-local ConPTY preserve them; Windows opts in only when equipped.
    if !cfg!(windows) || std::env::var("CREW_TEST_MODERN_CONPTY").as_deref() == Ok("1") {
        server.mode(1, "alt");
        client.wait_for("MODE_alt_p2");
        client.wheel(MouseEventKind::ScrollUp, pane_x + 9, 4);
        let alternate_history = client.barrier("BARRIER_ALT_WHEEL");
        client.key(KeyCode::Char('q'), KeyModifiers::NONE);
        client.assert_input(&server, &mut expected, b"", 'd');
        assert!(
            alternate_history.contains("HISTORY_p2_"),
            "{alternate_history}"
        );
        let live = client.barrier("BARRIER_ALT_LIVE");
        assert!(live.contains("MODE_alt_p2"), "{live}");

        client.key(KeyCode::PageUp, KeyModifiers::SHIFT);
        let alternate_shift = client.barrier("BARRIER_ALT_SHIFT");
        client.key(KeyCode::Esc, KeyModifiers::NONE);
        client.assert_input(&server, &mut expected, b"", 'e');
        assert!(alternate_shift.contains("HISTORY_p2_"), "{alternate_shift}");
        client.key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        client.key(KeyCode::Char('['), KeyModifiers::NONE);
        client.key(KeyCode::PageUp, KeyModifiers::NONE);
        let prefix_history = client.barrier("BARRIER_ALT_PREFIX");
        client.key(KeyCode::Char('q'), KeyModifiers::NONE);
        client.assert_input(&server, &mut expected, b"", 'f');
        assert!(prefix_history.contains("HISTORY_p2_"), "{prefix_history}");
        client.key(KeyCode::Up, KeyModifiers::NONE);
        client.key(KeyCode::Down, KeyModifiers::NONE);
        client.assert_input(&server, &mut expected, b"\x1b[A\x1b[B", 'g');

        for (sequence, mode, column, wheel_bytes, fence) in [
            (
                2,
                "sgr",
                110,
                b"\x1b[<64;110;4M\x1b[<65;110;4M".as_slice(),
                'h',
            ),
            (3, "default", 10, b"\x1b[M`*$\x1b[Ma*$".as_slice(), 'i'),
            (
                4,
                "utf8",
                110,
                b"\x1b[M`\xc2\x8e$\x1b[Ma\xc2\x8e$".as_slice(),
                'j',
            ),
        ] {
            server.mode(sequence, mode);
            client.wait_for(&format!("MODE_{mode}_p2"));
            client.wheel(MouseEventKind::ScrollUp, pane_x + column - 1, 4);
            client.wheel(MouseEventKind::ScrollDown, pane_x + column - 1, 4);
            client.assert_input(&server, &mut expected, wheel_bytes, fence);
        }
        // Explicit Crew history remains accessible even while the app asks for mouse input.
        client.key(KeyCode::PageUp, KeyModifiers::SHIFT);
        let mouse_history = client.barrier("BARRIER_MOUSE_SHIFT");
        client.key(KeyCode::Esc, KeyModifiers::NONE);
        client.assert_input(&server, &mut expected, b"", 'k');
        assert!(mouse_history.contains("HISTORY_p2_"), "{mouse_history}");
    } else {
        eprintln!(
            "alternate-screen/mouse-byte checks skipped: set CREW_TEST_MODERN_CONPTY=1 with modern ConPTY sidecars"
        );
    }
}

#[test]
#[ignore = "self-executed deterministic PTY child for the scrollback integration test"]
fn scrollback_fixture() {
    crossterm::terminal::enable_raw_mode().unwrap();
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{
            ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode,
            GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
        };
        // SAFETY: This child owns a PTY console; enable its VT byte input after raw mode.
        let handle = GetStdHandle(STD_INPUT_HANDLE);
        let mut mode = 0;
        assert_ne!(GetConsoleMode(handle, &mut mode), 0);
        assert_ne!(
            SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_INPUT),
            0
        );
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        assert_ne!(GetConsoleMode(output, &mut mode), 0);
        assert_ne!(
            SetConsoleMode(output, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING),
            0
        );
    }
    let pane = std::env::var("CREW_PANE_ID").unwrap();
    let root = std::env::current_dir().unwrap().join(&pane);
    std::fs::create_dir_all(&root).unwrap();
    let mut input_log = std::fs::File::create(root.join("input.bin")).unwrap();
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        let mut bytes = [0; 1024];
        while let Ok(count) = input.read(&mut bytes) {
            if count == 0 {
                break;
            }
            input_log.write_all(&bytes[..count]).unwrap();
            input_log.flush().unwrap();
        }
    });
    let mut output = std::io::stdout().lock();
    let mut initial = String::from("\x1b[?1l\x1b[2J\x1b[H");
    for line in 0..120 {
        initial.push_str(&format!("HISTORY_{pane}_{line:03}\r\n"));
    }
    initial.push_str(&format!("NORMAL_LIVE_{pane}\r\n"));
    output.write_all(initial.as_bytes()).unwrap();
    output.flush().unwrap();
    let mut last = String::new();
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if let Ok(control) = std::fs::read_to_string(root.join("control"))
            && control != last
            && let Some((_, mode)) = control.split_once(' ')
        {
            write!(
                output,
                "\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l"
            )
            .unwrap();
            match mode {
                "alt" => write!(output, "\x1b[?1049h").unwrap(),
                "sgr" => write!(output, "\x1b[?1000h\x1b[?1006h").unwrap(),
                "default" => write!(output, "\x1b[?1000h").unwrap(),
                "utf8" => write!(output, "\x1b[?1000h\x1b[?1005h").unwrap(),
                _ => panic!("unknown fixture mode: {mode}"),
            }
            write!(output, "\x1b[2J\x1b[HMODE_{mode}_{pane}").unwrap();
            output.flush().unwrap();
            last = control;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
