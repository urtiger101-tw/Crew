//! Raw attached test client and fake PTY agents on an owned private endpoint.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use interprocess::TryClone;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Stream, prelude::*};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct PrivateServer {
    root: PathBuf,
    socket: String,
    child: Option<Child>,
}
impl PrivateServer {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suffix = format!("{}-{nonce}", std::process::id());
        let root = std::env::temp_dir().join(format!("crew-af-test-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        let python = Command::new("python")
            .args(["-c", "import sys; print(sys.executable)"])
            .output()
            .unwrap();
        assert!(python.status.success());
        let python = String::from_utf8(python.stdout)
            .unwrap()
            .trim()
            .replace('\\', "/");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/autofocus_agent.py")
            .to_string_lossy()
            .replace('\\', "/");
        let config = json!({"persist":true,"toolbar":false,"sidebar":false,"pane_titles":"off","mouse":false,
            "pane_utf8":false,"agent_detection":false,"lang":"en","auto_focus":"on",
            "shell":format!("\"{python}\" -u \"{fixture}\"")});
        std::fs::write(root.join("config.toml"), toml::to_string(&config).unwrap()).unwrap();
        let leaf = || json!({"Leaf":{"cwd":root,"env":{"CREW_AF_TEST_ROOT":root},"label":"fake","agent":null,"agent_name":null,"resume":[]}});
        std::fs::write(root.join("state.json"),json!({"version":1,"sessions":[{
            "id":1,"name":"private-af","cwd":root,"env":{},"cols":120,"rows":30,"sidebar":false,"active_window":0,
            "windows":[{"id":1,"name":"fake-agents","manual_name":true,"zoomed":false,"active":0,
                "layout":{"Split":{"dir":"H","ratio":0.5,"a":leaf(),"b":leaf()}}}]}]}).to_string()).unwrap();
        let mut server = Self {
            root,
            socket: format!("crew-af-test-{suffix}.sock"),
            child: None,
        };
        server.child = Some(
            server
                .command()
                .arg("__server")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = server.command().args(["api", "snapshot"]).output().unwrap();
            if output.status.success() {
                break;
            }
            assert!(Instant::now() < deadline, "private server did not start");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            server.json(&["doctor", "--json"])["pid"],
            server.child.as_ref().unwrap().id()
        );
        for id in ["p1", "p2"] {
            assert_eq!(
                server.json(&[
                    "pane",
                    "wait-output",
                    id,
                    "--match",
                    "FAKE_READY",
                    "--timeout",
                    "15000"
                ])["matched"],
                true
            );
        }
        eprintln!(
            "isolated AF endpoint={} state={} owned_pid={}",
            server.socket,
            server.root.join("state.json").display(),
            server.child.as_ref().unwrap().id()
        );
        server
    }
    fn command(&self) -> Command {
        assert!(self.socket.starts_with("crew-af-test-"));
        let mut c = Command::new(env!("CARGO_BIN_EXE_crew"));
        c.env_clear()
            .current_dir(&self.root)
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_STATE", self.root.join("state.json"))
            .env("CREW_CONFIG", self.root.join("config.toml"))
            .env("CREW_LOG", self.root.join("server.log"))
            .env("CREW_LANG", "en")
            .stdin(Stdio::null());
        for key in ["PATH", "SystemRoot", "WINDIR", "TMP", "TEMP", "TMPDIR"] {
            if let Some(value) = std::env::var_os(key) {
                c.env(key, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        c
    }
    fn output(&self, args: &[&str]) -> std::process::Output {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_slice::<Value>(&self.output(args).stdout).unwrap()["result"].clone()
    }
    fn report(&self, state: &str) {
        self.json(&[
            "pane",
            "report-agent",
            "p2",
            "--source",
            "af-fixture",
            "--agent",
            "fake",
            "--state",
            state,
        ]);
    }
    fn focused(&self) -> String {
        self.json(&["api", "snapshot"])["sessions"][0]["windows"][0]["active_pane"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn wait_focus(&self, id: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while self.focused() != id {
            assert!(Instant::now() < deadline, "focus did not move to {id}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
impl Drop for PrivateServer {
    fn drop(&mut self) {
        let _ = self.command().args(["server", "stop"]).output();
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
        if std::thread::panicking() {
            eprintln!("preserved AF fixture {}", self.root.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

fn send(stream: &mut Stream, value: Value) {
    let bytes = serde_json::to_vec(&value).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    stream.flush().unwrap();
}
struct Attached {
    stream: Stream,
    output: Receiver<Vec<u8>>,
    screen: vt100::Parser,
}
impl Attached {
    fn new(server: &PrivateServer) -> Self {
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
        let mut stream = Stream::connect(name).unwrap();
        let mut reader = stream.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            loop {
                let mut length = [0; 4];
                if reader.read_exact(&mut length).is_err() {
                    break;
                }
                let mut bytes = vec![0; u32::from_le_bytes(length) as usize];
                if reader.read_exact(&mut bytes).is_err() {
                    break;
                }
                let reply: Value = serde_json::from_slice(&bytes).unwrap();
                if let Some(output) = reply["Output"].as_str() {
                    let _ = tx.send(output.as_bytes().to_vec());
                }
            }
        });
        send(
            &mut stream,
            json!({"Hello":{"Attach":{"session":"private-af","create":false,"cols":120,"rows":30,"cwd":null,"detached":false}}}),
        );
        let mut attached = Self {
            stream,
            output: rx,
            screen: vt100::Parser::new(30, 120, 0),
        };
        attached.wait_text("AF", Duration::from_secs(5));
        attached
    }
    fn event(&mut self, event: Event) {
        send(&mut self.stream, json!({"Event":event}));
    }
    fn wait_text(&mut self, text: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !self.screen.screen().contents().contains(text) {
            let bytes = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|_| panic!("missing {text}: {}", self.screen.screen().contents()));
            self.screen.process(&bytes);
        }
    }
    fn last_pane(&mut self) {
        self.event(Event::Key(KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        )));
        self.event(Event::Key(KeyEvent::new(
            KeyCode::Char(';'),
            KeyModifiers::NONE,
        )));
    }
}

#[test]
fn fake_agent_edges_queue_toast_release_and_never_send_input() {
    let server = PrivateServer::new();
    server.report("working");
    server.report("blocked");
    assert_eq!(server.focused(), "p1", "headless has no auto jump");
    let mut client = Attached::new(&server);
    server.report("blocked");
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        server.focused(),
        "p1",
        "attaching and repeated levels do not replay an old edge"
    );
    server.report("working");
    server.report("blocked");
    server.wait_focus("p2");
    client.last_pane();
    server.wait_focus("p1");
    client.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 1,
        row: 1,
        modifiers: KeyModifiers::NONE,
    }));
    server.report("working");
    server.report("blocked");
    assert_eq!(
        server.focused(),
        "p1",
        "recent client input and dwell must queue"
    );
    client.wait_text("to jump", Duration::from_secs(2));
    server.wait_focus("p2");
    client.last_pane();
    server.wait_focus("p1");
    client.event(Event::Key(KeyEvent::new(
        KeyCode::Char('b'),
        KeyModifiers::CONTROL,
    )));
    client.event(Event::Key(KeyEvent::new(
        KeyCode::Char('['),
        KeyModifiers::NONE,
    )));
    server.report("working");
    server.report("blocked");
    std::thread::sleep(Duration::from_millis(4200));
    assert_eq!(
        server.focused(),
        "p1",
        "copy mode protects focus after both time guards expire"
    );
    client.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    server.wait_focus("p2");
    client.last_pane();
    server.wait_focus("p1");
    server.report("blocked");
    std::thread::sleep(Duration::from_millis(4200));
    assert_eq!(
        server.focused(),
        "p1",
        "remaining blocked does not retrigger after guards expire"
    );
    assert_eq!(
        server.json(&["pane", "auto-focus", "p2", "off"])["pane"]["auto_focus"],
        false
    );
    server.report("working");
    server.report("blocked");
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(server.focused(), "p1", "opted-out pane cannot steal focus");
    server.json(&["pane", "auto-focus", "p2", "on"]);
    server.report("blocked");
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(server.focused(), "p1", "opt-in is not a new edge");
    server.output(&["set", "auto-focus", "blocked"]);
    std::fs::write(server.root.join("p2.output"), b"signal").unwrap();
    server.json(&[
        "pane",
        "wait-output",
        "p2",
        "--match",
        "FAKE_OUTPUT",
        "--timeout",
        "5000",
    ]);
    server.report("idle");
    assert_eq!(
        server.json(&["pane", "get", "p2"])["pane"]["status"],
        "done"
    );
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(server.focused(), "p1", "blocked mode ignores Done");
    server.output(&["set", "auto-focus", "on"]);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        server.focused(),
        "p1",
        "mode enable does not replay a Done level"
    );
    server.report("working");
    server.report("idle");
    server.wait_focus("p2");
    client.last_pane();
    server.wait_focus("p1");
    let fresh = server.json(&["pane", "split", "p2", "--direction", "down"]);
    assert_eq!(fresh["pane"]["pane_id"], "p3");
    server.json(&[
        "pane",
        "wait-output",
        "p3",
        "--match",
        "FAKE_READY",
        "--timeout",
        "5000",
    ]);
    // The saved panes remain marked as restored shells after releasing a report.
    // Use a fresh pane to exercise the ordinary, unreported PTY status path.
    std::thread::sleep(Duration::from_millis(2100));
    server.output(&["set", "busy-secs", "1"]);
    std::fs::write(server.root.join("p3.output"), b"signal").unwrap();
    server.json(&[
        "pane",
        "wait-output",
        "p3",
        "--match",
        "FAKE_OUTPUT 1",
        "--timeout",
        "5000",
    ]);
    assert_eq!(
        server.json(&["pane", "get", "p3"])["pane"]["status"],
        "working"
    );
    server.wait_focus("p3"); // Only the existing busy→idle tick creates this Done edge.
    for id in ["p1", "p2", "p3"] {
        assert!(
            std::fs::read(server.root.join(format!("{id}.input")))
                .unwrap()
                .is_empty(),
            "auto-focus or its navigation sent input to {id}"
        );
    }
    server.output(&["set", "lang", "auto"]);
    for theme in ["studio", "aurora"] {
        server.output(&["set", "theme", theme]);
        let text = server.output(&["debug-frame", "--text", "--lang", "zh-TW"]);
        assert!(
            String::from_utf8_lossy(&text.stdout).contains("自焦"),
            "{theme}: {}",
            String::from_utf8_lossy(&text.stdout)
        );
    }
}
