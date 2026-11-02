//! Real isolated daemon/PTY coverage; mouse events enter through the client protocol.
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use interprocess::TryClone;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Stream, prelude::*};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Server {
    root: PathBuf,
    socket: String,
}

impl Server {
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_crew"));
        cmd.env("CREW_SOCKET", &self.socket)
            .env("CREW_CONFIG", self.root.join("config.toml"))
            .env("CREW_STATE", self.root.join("state.json"))
            .env("CREW_LOG", self.root.join("server.log"))
            .env("CREW_LANG", "en")
            .env_remove("CREW")
            .env_remove("CREW_PANE_ID")
            .env_remove("CREW_SERVER_PID");
        cmd
    }

    fn api(&self, args: &[&str]) -> Value {
        let out = self.command().args(args).output().unwrap();
        if !out.status.success() && args.get(1) == Some(&"wait-output") {
            let capture = self
                .command()
                .args(["pane", "read", args[2], "--source", "recent"])
                .output()
                .unwrap();
            eprintln!(
                "pane after timeout: {}",
                String::from_utf8_lossy(&capture.stdout)
            );
        }
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    }

    fn tabs(&self) -> Value {
        self.api(&["workspace", "get", "drag"])["workspace"]["tabs"].clone()
    }

    fn await_order(&self, expected: &[&str]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let tabs = self.tabs();
            let panes: Vec<_> = tabs[0]["panes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["pane_id"].as_str().unwrap())
                .collect();
            if panes == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "expected {expected:?}, got {panes:?}"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn send(stream: &mut Stream, message: Value) {
    let bytes = serde_json::to_vec(&message).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    stream.flush().unwrap();
}

fn mouse(stream: &mut Stream, kind: MouseEventKind, column: u16, row: u16) {
    send(
        stream,
        json!({"Event": Event::Mouse(MouseEvent {
            kind, column, row, modifiers: KeyModifiers::NONE,
        })}),
    );
}

#[test]
fn swapping_and_client_drags_preserve_live_occupants_and_saved_order() {
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let server = Server {
        root: std::env::temp_dir().join(format!("crew-reorder-{suffix}")),
        socket: format!("crew-reorder-{suffix}.sock"),
    };
    std::fs::create_dir_all(&server.root).unwrap();
    let shell = if cfg!(windows) {
        "cmd.exe /Q"
    } else {
        "/bin/sh"
    };
    std::fs::write(server.root.join("config.toml"), format!(
        "persist = true\nautomatic_rename = false\nsidebar = false\ntoolbar = false\npane_titles = \"always\"\nshell = {shell:?}\n"
    )).unwrap();
    server.api(&["workspace", "create", "drag", "--focus"]);
    server.api(&["tab", "rename", "t1", "alpha"]);
    server.api(&[
        "pane",
        "split",
        "p1",
        "--direction",
        "right",
        "--ratio",
        "0.3",
    ]);
    server.api(&["pane", "split", "p2", "--direction", "down"]);
    let mut identities = Vec::new();
    for id in ["p1", "p2", "p3"] {
        server.api(&["pane", "rename", id, id]);
        let command = if cfg!(windows) {
            format!("set \"REORDER_VALUE={id}\"&echo READY_{id}")
        } else {
            format!("REORDER_VALUE='{id}'; echo READY_{id}")
        };
        server.api(&["pane", "run", id, &command]);
        assert_eq!(
            server.api(&[
                "pane",
                "wait-output",
                id,
                "--regex",
                &format!("^READY_{id}\\s*$"),
                "--timeout",
                "15000"
            ])["matched"],
            true
        );
        identities.push(server.api(&["pane", "get", id])["pane"]["root_process_id"].clone());
    }
    assert_eq!(server.api(&["pane", "swap", "p1", "p3"])["swapped"], true);
    server.await_order(&["p3", "p2", "p1"]);
    assert_eq!(server.api(&["pane", "swap", "p1", "p1"])["swapped"], false);
    let other = server.api(&["tab", "create", "beta", "--workspace", "drag"])["tab"]["active_pane"]
        .as_str()
        .unwrap()
        .to_owned();
    server.api(&["pane", "rename", &other, "other-shell"]);
    let bad = server
        .command()
        .args(["pane", "swap", "p1", &other])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    server.await_order(&["p3", "p2", "p1"]);

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
    let mut client = Stream::connect(name).unwrap();
    let mut reader = client.try_clone().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
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
            let _ = tx.send(serde_json::from_slice::<Value>(&bytes).unwrap());
        }
    });
    send(
        &mut client,
        json!({"Hello": {"Attach": {
            "session": "drag", "create": false, "cols": 120, "rows": 30, "cwd": null
        }}}),
    );
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap()["Output"].is_string());
    server.api(&[
        "pane",
        "report-agent",
        "p3",
        "--source",
        "reorder-test",
        "--agent",
        "fixture",
        "--state",
        "working",
        "--seq",
        "1",
    ]);
    // The large left title is p3; the upper right body is p2.
    mouse(&mut client, MouseEventKind::Down(MouseButton::Left), 2, 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.tabs()[0]["active_pane"] != "p3" {
        assert!(Instant::now() < deadline, "title Down did not focus p3");
        std::thread::sleep(Duration::from_millis(30));
    }
    server.api(&[
        "pane",
        "report-agent",
        "p3",
        "--source",
        "reorder-test",
        "--agent",
        "fixture",
        "--state",
        "working",
        "--seq",
        "2",
    ]);
    mouse(&mut client, MouseEventKind::Drag(MouseButton::Left), 110, 2);
    mouse(&mut client, MouseEventKind::Up(MouseButton::Left), 110, 2);
    server.await_order(&["p2", "p3", "p1"]);
    assert_eq!(server.tabs()[0]["active_pane"], "p3");
    // Escape cancels an in-progress drag before release.
    mouse(&mut client, MouseEventKind::Down(MouseButton::Left), 2, 0);
    mouse(&mut client, MouseEventKind::Drag(MouseButton::Left), 110, 2);
    send(
        &mut client,
        json!({"Event": Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))}),
    );
    mouse(&mut client, MouseEventKind::Up(MouseButton::Left), 110, 2);
    // A command on this same stream acts as an ordering barrier for the assertion.
    send(
        &mut client,
        json!({"Event": Event::Key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL))}),
    );
    send(
        &mut client,
        json!({"Event": Event::Key(KeyEvent::new(KeyCode::Char('}'), KeyModifiers::NONE))}),
    );
    server.await_order(&["p3", "p2", "p1"]);

    // Find actual rendered status labels; their columns vary with theme/status content.
    let frame = server
        .command()
        .args(["debug-frame", "-t", "drag", "--text"])
        .output()
        .unwrap();
    assert!(
        frame.status.success(),
        "{}",
        String::from_utf8_lossy(&frame.stderr)
    );
    let frame = String::from_utf8(frame.stdout).unwrap();
    let status = frame.lines().last().unwrap();
    let alpha = status[..status.find("0:alpha").unwrap()].chars().count() as u16;
    let beta = status[..status.find("1:beta").unwrap()].chars().count() as u16 + 5;
    mouse(
        &mut client,
        MouseEventKind::Down(MouseButton::Left),
        alpha,
        29,
    );
    mouse(
        &mut client,
        MouseEventKind::Drag(MouseButton::Left),
        beta,
        29,
    );
    mouse(&mut client, MouseEventKind::Up(MouseButton::Left), beta, 29);
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.tabs()[0]["name"] != "beta" {
        assert!(Instant::now() < deadline, "tab drag did not reorder");
        std::thread::sleep(Duration::from_millis(30));
    }
    for (index, id) in ["p1", "p2", "p3"].into_iter().enumerate() {
        assert_eq!(
            server.api(&["pane", "get", id])["pane"]["root_process_id"],
            identities[index]
        );
        let command = if cfg!(windows) {
            "echo KEPT_%REORDER_VALUE%"
        } else {
            "echo KEPT_$REORDER_VALUE"
        };
        server.api(&["pane", "run", id, command]);
        assert_eq!(
            server.api(&[
                "pane",
                "wait-output",
                id,
                "--regex",
                &format!("^KEPT_{id}\\s*$"),
                "--timeout",
                "15000"
            ])["matched"],
            true
        );
    }
    let before = server.tabs();
    server.api(&["server", "stop"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while server
        .command()
        .arg("ls")
        .output()
        .unwrap()
        .status
        .success()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(30));
    }
    server.api(&["workspace", "create", "wake"]);
    let after = server.tabs();
    for index in 0..2 {
        assert_eq!(after[index]["name"], before[index]["name"]);
        // Persistence restores occupants into fresh pane IDs, in saved leaf order.
        let labels = |tab: &Value| {
            tab["panes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["label"].clone())
                .collect::<Vec<_>>()
        };
        let active_index = |tab: &Value| {
            tab["panes"]
                .as_array()
                .unwrap()
                .iter()
                .position(|p| p["pane_id"] == tab["active_pane"])
                .unwrap()
        };
        assert_eq!(labels(&after[index]), labels(&before[index]));
        assert_eq!(active_index(&after[index]), active_index(&before[index]));
    }
}
