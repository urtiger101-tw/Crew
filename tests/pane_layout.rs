//! Real PTY/server test using an explicit private socket, config and state only.
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
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
        let root = std::env::temp_dir().join(format!("crew-layout-test-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        let config = json!({"persist":true,"sidebar":false,"toolbar":false,"pane_titles":"off",
            "pane_utf8":false,"agent_detection":false,"lang":"en", "scrollback":1000,
            "shell": if cfg!(windows) {
                "powershell.exe -NoLogo -NoProfile -NoExit -Command \"function global:prompt { 'L> ' }\""
            } else { "/bin/sh" }});
        std::fs::write(root.join("config.toml"), toml::to_string(&config).unwrap()).unwrap();
        let leaf = |label| json!({"Leaf":{"cwd":root,"label":label,"agent":null,"agent_name":null,"resume":[]}});
        std::fs::write(root.join("state.json"), json!({"version":1,"sessions":[{
            "id":1,"name":"layout-private","cwd":root,"env":{},"cols":40,"rows":11,"sidebar":false,"active_window":0,
            "windows":[{"id":1,"name":"three","manual_name":true,"zoomed":true,"active":1,
                "layout":{"Split":{"dir":"V","ratio":0.5,"a":leaf("one"),
                    "b":{"Split":{"dir":"V","ratio":0.5,"a":leaf("two"),"b":leaf("three")}}}}},
                {"id":2,"name":"single","manual_name":true,"zoomed":false,"active":0,"layout":leaf("four")}]
        }]}).to_string()).unwrap();
        let mut server = Self {
            root,
            socket: format!("crew-layout-test-{suffix}.sock"),
            child: None,
        };
        server.child = Some(
            server
                .command()
                .arg("__server")
                .stdin(Stdio::null())
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
            assert!(
                Instant::now() < deadline,
                "private server startup failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let doctor = server.json(&["doctor", "--json"]);
        assert_eq!(
            doctor["pid"],
            server.child.as_ref().unwrap().id(),
            "endpoint must be the child owned by this fixture"
        );
        eprintln!(
            "isolated layout endpoint={} state={} pid={}",
            server.socket,
            server.root.join("state.json").display(),
            server.child.as_ref().unwrap().id()
        );
        server
    }

    fn command(&self) -> Command {
        assert!(self.socket.starts_with("crew-layout-test-"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
        command
            .env_clear()
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
                command.env(key, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        command
    }

    fn output(&self, args: &[&str]) -> Output {
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
}

impl Drop for PrivateServer {
    fn drop(&mut self) {
        // This command always carries the fixture's private endpoint and state.
        let _ = self.command().args(["server", "stop"]).output();
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        if std::thread::panicking() {
            eprintln!("preserved layout test artifacts: {}", self.root.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn private_api_layout_preserves_processes_ids_focus_history_and_saved_tree() {
    let server = PrivateServer::new();
    let initial = server.json(&["api", "snapshot"]);
    let before = initial["sessions"][0]["windows"][0]["panes"]
        .as_array()
        .unwrap();
    let identities: Vec<_> = before
        .iter()
        .map(|p| (p["pane_id"].clone(), p["root_process_id"].clone()))
        .collect();
    assert_eq!(identities.len(), 3);
    for (id, pid) in &identities {
        assert!(pid.as_u64().unwrap() > 0);
        let id = id.as_str().unwrap();
        let marker = format!("KEEP_{id}");
        let line = if cfg!(windows) {
            format!("Write-Output ('KEEP_' + '{id}')")
        } else {
            format!("printf 'KEEP_%s\\n' '{id}'")
        };
        server.json(&["pane", "run", id, &line]);
        assert_eq!(
            server.json(&[
                "pane",
                "wait-output",
                id,
                "--match",
                &marker,
                "--timeout",
                "15000"
            ])["matched"],
            true
        );
    }
    for preset in [
        "even-horizontal",
        "even-vertical",
        "main-horizontal",
        "main-vertical",
        "tiled",
    ] {
        let reply = server.json(&[
            "pane",
            "layout",
            preset,
            "--window",
            "0",
            "--session",
            "layout-private",
        ]);
        assert_eq!(reply["layout"], preset);
        assert_eq!(reply["active_pane"], "p2");
        assert_eq!(reply["zoomed"], false);
        let panes = reply["panes"].as_array().unwrap();
        assert_eq!(
            panes
                .iter()
                .map(|p| (p["pane_id"].clone(), p["root_process_id"].clone()))
                .collect::<Vec<_>>(),
            identities
        );
        for pane in panes {
            assert!(pane["cols"].as_u64().unwrap() > 0 && pane["rows"].as_u64().unwrap() > 0);
            let cwd = pane["cwd"].as_str().unwrap();
            assert!(!cwd.starts_with(r"\\?\"), "display-only cwd prefix: {cwd}");
            let id = pane["pane_id"].as_str().unwrap();
            let recent =
                server.output(&["pane", "read", id, "--source", "recent", "--lines", "1000"]);
            let text = String::from_utf8_lossy(&recent.stdout);
            assert!(
                text.replace(['\r', '\n'], "")
                    .contains(&format!("KEEP_{id}")),
                "{id}: {text}"
            );
        }
        let snapshot = server.json(&["api", "snapshot"]);
        assert_eq!(snapshot["sessions"][0]["windows"][0]["layout_name"], preset);
        assert_eq!(snapshot["sessions"][0]["windows"][0]["active_pane"], "p2");
    }
    for _ in 0..5 {
        server.output(&["next-layout"]);
    }
    assert_eq!(
        server.json(&["api", "snapshot"])["sessions"][0]["windows"][0]["layout_name"],
        "tiled"
    );
    let single = server.json(&[
        "pane",
        "layout",
        "tiled",
        "--window",
        "1",
        "--session",
        "layout-private",
    ]);
    assert_eq!(single["changed"], false);
    assert_eq!(single["active_pane"], "p4");
    assert_eq!(
        server.json(&["api", "snapshot"])["sessions"][0]["active_window"],
        0
    );
    let bad = server
        .command()
        .args(["pane", "layout", "unknown"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    let help = server.output(&["pane", "--help"]);
    assert!(String::from_utf8_lossy(&help.stdout).contains("pane layout"));
    let schema: Value = serde_json::from_slice(&server.output(&["api", "schema"]).stdout).unwrap();
    assert!(
        schema["methods"]
            .as_array()
            .unwrap()
            .contains(&json!("pane.layout"))
    );
    // Save via this fixture's private daemon only. The snapshot preserves cycle state.
    server.json(&["server", "stop"]);
    let saved: Value =
        serde_json::from_slice(&std::fs::read(server.root.join("state.json")).unwrap()).unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["sessions"][0]["windows"][0]["layout_preset"], "tiled");
    assert_eq!(saved["sessions"][0]["windows"][0]["active"], 1);
}
