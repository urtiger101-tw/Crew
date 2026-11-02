#![cfg(windows)]

use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct RestoredServer {
    root: PathBuf,
    socket: String,
    child: Option<Child>,
}

impl RestoredServer {
    fn new(resume: Vec<String>) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suffix = format!("{}-{stamp}", std::process::id());
        let root = std::env::temp_dir().join(format!("crew-resume-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        // PSReadLine 2.0 can throw at the right margin when the temporary cwd fills
        // the prompt row. Keep its real input loop, with a short deterministic prompt.
        std::fs::write(
            root.join("config.toml"),
            r#"persist = true
agent_detection = false
shell = "powershell.exe -NoLogo -NoProfile -NoExit -Command \"function global:prompt { 'PS > ' }\""
"#,
        )
        .unwrap();
        let snapshot = json!({
            "version": 1,
            "sessions": [{
                "id": 1, "name": "main", "cwd": root,
                "env": {"CREW_TEST_RESUME_LOG": root.join("resume.log")},
                "cols": 100, "rows": 24, "sidebar": true, "active_window": 0,
                "windows": [{"id": 1, "name": "powershell", "manual_name": false,
                    "zoomed": false, "active": 0,
                    "layout": {"Leaf": {"cwd": root, "label": "saved-agent", "agent": "claude", "agent_name": "saved-peer", "resume": resume}}
                }]
            }]
        });
        std::fs::write(
            root.join("state.json"),
            serde_json::to_vec_pretty(&snapshot).unwrap(),
        )
        .unwrap();
        let mut server = Self {
            root,
            socket: format!("crew-resume-{suffix}.sock"),
            child: None,
        };
        let mut command = server.command();
        command
            .arg("__server")
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
        server.child = Some(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(15);
        while !server
            .command()
            .args(["pane", "get", "p1"])
            .output()
            .unwrap()
            .status
            .success()
        {
            assert!(
                Instant::now() < deadline,
                "restore did not start: {}",
                std::fs::read_to_string(server.root.join("server.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        server.run(&[
            "pane",
            "wait-output",
            "p1",
            "--match",
            "PS ",
            "--timeout",
            "15000",
        ]);
        server
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
        command
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_STATE", self.root.join("state.json"))
            .env("CREW_CONFIG", self.root.join("config.toml"))
            .env("CREW_LOG", self.root.join("server.log"))
            .env("CREW_LANG", "en")
            .env_remove("CREW")
            .env_remove("CREW_PANE_ID")
            .env_remove("CREW_SERVER_PID");
        command
    }

    fn run(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for RestoredServer {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if std::thread::panicking() {
            eprintln!("preserved failed test artifacts: {}", self.root.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn restored_agent_waits_for_explicit_resume_and_repeated_requests_execute_once() {
    let argv = vec![
        "powershell.exe".into(),
        "-NoProfile".into(),
        "-Command".into(),
        "Add-Content -LiteralPath $env:CREW_TEST_RESUME_LOG -Value invoked".into(),
    ];
    let server = RestoredServer::new(argv.clone());
    let log = server.root.join("resume.log");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !log.exists(),
        "restore executed saved argv before explicit resume"
    );
    let pane = server.run(&["pane", "get", "p1"]);
    assert_eq!(pane["result"]["pane"]["restore_state"], "not-running");
    assert_eq!(pane["result"]["pane"]["agent"]["resume"], json!(argv));
    assert_eq!(
        server.run(&["pane", "resume", "p1"])["result"]["resume_status"],
        "requested"
    );
    assert_eq!(
        server.run(&["pane", "resume", "p1"])["result"]["resume_status"],
        "already-requested"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !log.exists() {
        assert!(
            Instant::now() < deadline,
            "resume command did not run: {}",
            String::from_utf8_lossy(
                &server
                    .command()
                    .args(["pane", "read", "p1"])
                    .output()
                    .unwrap()
                    .stdout
            )
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(std::fs::read_to_string(log).unwrap().lines().count(), 1);
    let duplicate = server
        .command()
        .args(["new", "-d", "-s", "main"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&duplicate.stderr);
    assert!(!duplicate.status.success());
    assert!(error.contains("crew attach -t main"), "{error}");
}

#[test]
fn restored_agent_without_saved_command_remains_visible_and_resume_is_retryable() {
    let server = RestoredServer::new(vec![]);
    for _ in 0..2 {
        let output = server
            .command()
            .args(["pane", "resume", "p1"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], "resume_unavailable");
        let pane = server.run(&["pane", "get", "p1"]);
        assert_eq!(pane["result"]["pane"]["restore_state"], "not-running");
    }
}

#[test]
fn restored_pane_displays_its_state_resume_hint_and_saved_argv() {
    let server = RestoredServer::new(vec!["claude".into(), "--resume".into(), "abc".into()]);
    let output = server
        .command()
        .args(["debug-frame", "-t", "main", "--text"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let frame = String::from_utf8(output.stdout).unwrap();
    assert!(frame.contains("restored, not running"), "{frame}");
    assert!(frame.contains("C-b R"), "{frame}");
    assert!(frame.contains("claude"), "{frame}");
    assert!(frame.contains("--resume"), "{frame}");
}
