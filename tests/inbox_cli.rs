//! B v1 acceptance through the real CLI and JSON-lines bridge on isolated daemons.
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Server {
    root: PathBuf,
    socket: String,
    inbox_store: Option<PathBuf>,
    artifact_links: Vec<PathBuf>,
}

impl Server {
    fn new() -> Self {
        Self::with_persistence(false)
    }

    fn with_persistence(persist: bool) -> Self {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("crew-inbox-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        let shell = if cfg!(windows) {
            format!(
                "'{}' -NoLogo -NoProfile",
                PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                    .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
                    .display()
            )
        } else {
            "'/bin/sh'".into()
        };
        fs::write(
            root.join("config.toml"),
            format!("persist = {persist}\nshell = {}\n", json!(shell)),
        )
        .unwrap();
        let inbox_store = Some(root.join("inbox.json"));
        Self {
            root,
            socket: format!("crew-inbox-{suffix}.sock"),
            inbox_store,
            artifact_links: Vec::new(),
        }
    }

    fn command(&self, caller: Option<&str>) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
        for (key, _) in std::env::vars_os() {
            let upper = key.to_string_lossy().to_ascii_uppercase();
            if upper.starts_with("CREW_")
                || upper.starts_with("TMUXLIKE_")
                || matches!(upper.as_str(), "CREW" | "TMUXLIKE")
            {
                command.env_remove(key);
            }
        }
        command
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_STATE", self.root.join("state.json"))
            .env("CREW_CONFIG", self.root.join("config.toml"))
            .env("CREW_PLUGIN_HOME", self.root.join("plugins"))
            .env("CREW_LOG", self.root.join("server.log"))
            .env("CREW_LANG", "en");
        if let Some(path) = &self.inbox_store {
            command.env("CREW_INBOX_STORE", path);
        }
        if let Some(caller) = caller {
            command.env("CREW_PANE_ID", caller);
        }
        command
    }

    fn run(&self, caller: Option<&str>, args: &[&str]) -> Value {
        let output = self.command(caller).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
    }

    fn error(&self, caller: Option<&str>, args: &[&str], code: &str) {
        let output = self.command(caller).args(args).output().unwrap();
        assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
        let value: Value = serde_json::from_slice(&output.stderr)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stderr)));
        assert_eq!(value["error"]["code"], code, "{value}");
    }

    fn bridge(&self, caller: Option<&str>, method: &str, args: &[&str]) -> Value {
        let mut child = self
            .command(caller)
            .args(["api", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(
            child.stdin.take().unwrap(),
            "{}",
            json!({"id":"inbox-test","method":method,"params":{"args":args}})
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["id"], "inbox-test");
        value
    }

    fn peer(&self, name: &str, kind: &str) -> String {
        self.run(None, &["workspace", "create", name]);
        let panes = self.run(None, &["pane", "list"]);
        let pane = panes["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["pane_id"].as_str().unwrap())
            .max_by_key(|id| id[1..].parse::<usize>().unwrap())
            .unwrap()
            .to_owned();
        self.run(
            None,
            &[
                "pane",
                "report-agent",
                &pane,
                "--source",
                "fixture.inbox",
                "--agent",
                kind,
                "--state",
                "idle",
            ],
        );
        self.run(None, &["agent", "rename", &pane, name]);
        pane
    }

    fn stop(&self) {
        self.run(None, &["server", "stop"]);
        let deadline = Instant::now() + Duration::from_secs(10);
        while self
            .command(None)
            .args(["pane", "list"])
            .output()
            .unwrap()
            .status
            .success()
        {
            assert!(
                Instant::now() < deadline,
                "isolated daemon did not stop: {}",
                self.socket
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // This request cannot start a daemon; the command always names our unique socket.
        let _ = self.command(None).args(["server", "stop"]).output();
        // Unlink the junction/symlink itself before any recursive cleanup of our root.
        for link in &self.artifact_links {
            if fs::symlink_metadata(link).is_ok() {
                #[cfg(windows)]
                let removed = fs::remove_dir(link);
                #[cfg(unix)]
                let removed = fs::remove_file(link);
                if let Err(error) = removed {
                    eprintln!(
                        "cannot unlink {}: {error}; retaining {}",
                        link.display(),
                        self.root.display()
                    );
                    assert!(std::thread::panicking(), "artifact link cleanup failed");
                    return;
                }
            }
        }
        if std::thread::panicking() {
            eprintln!("inbox test evidence retained at {}", self.root.display());
        } else {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../docs/collab/claude-codex/contract/b-v1.fixtures.json"
    ))
    .unwrap()
}

#[test]
fn default_store_is_per_socket_and_uses_the_daemon_start_environment() {
    let mut server = Server::new();
    server.inbox_store = None;
    let sender = server.peer("crew-codex", "codex");
    server.peer("crew-claude", "claude");
    let stored = server.run(
        Some(&sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "crew-claude",
            "--key",
            "default-path",
            "--kind",
            "handoff",
            "--summary",
            "Default per-socket store",
        ],
    );
    let expected = server
        .root
        .join("inbox")
        .join(format!("{}.json", server.socket));
    assert!(
        expected.is_file(),
        "missing default store {}",
        expected.display()
    );
    assert!(!server.root.join("inbox.json").exists());
    let before = fs::read(&expected).unwrap();
    let late_override = server.root.join("late-client-override.json");
    server.inbox_store = Some(late_override.clone());
    let second = server.run(
        Some(&sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "crew-claude",
            "--key",
            "still-default-path",
            "--kind",
            "handoff",
            "--summary",
            "CLI environment does not relocate daemon store",
        ],
    );
    assert_ne!(second["message"]["id"], stored["message"]["id"]);
    assert_ne!(fs::read(&expected).unwrap(), before);
    assert!(!late_override.exists());
    let messages = server.run(None, &["msg", "list", "--to", "crew-claude"]);
    assert_eq!(messages["messages"].as_array().unwrap().len(), 2);
    assert_eq!(messages["messages"][0]["id"], stored["message"]["id"]);
    assert_eq!(messages["messages"][1]["id"], second["message"]["id"]);
}

#[test]
fn artifact_link_escape_is_rejected_by_cli_and_bridge_without_storing_or_events() {
    let mut server = Server::new();
    let sender = server.peer("crew-codex", "codex");
    server.peer("crew-claude", "claude");
    let repo = server.root.join("sender-repo");
    let outside = server.root.join("outside-repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let outside_file = outside.join("handoff.txt");
    fs::write(
        &outside_file,
        "This real file is outside the sender repository.",
    )
    .unwrap();
    let link = repo.join("escape");
    server.artifact_links.push(link.clone());
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(&outside, &link).is_err() {
            // Junctions require no symlink privilege, so this acceptance never skips on Windows.
            let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
            let output = Command::new(powershell)
                .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:CREW_TEST_LINK -Target $env:CREW_TEST_OUTSIDE -ErrorAction Stop | Out-Null"])
                .env("CREW_TEST_LINK", &link)
                .env("CREW_TEST_OUTSIDE", &outside)
                .output().unwrap();
            assert!(
                output.status.success(),
                "junction creation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    assert_eq!(
        fs::canonicalize(link.join("handoff.txt")).unwrap(),
        fs::canonicalize(&outside_file).unwrap()
    );
    let args = [
        "msg",
        "send",
        "--from",
        "crew-codex",
        "--to",
        "crew-claude",
        "--key",
        "artifact-escape",
        "--kind",
        "handoff",
        "--summary",
        "Reject linked artifact",
        "--repo-root",
        repo.to_str().unwrap(),
        "--file",
        "escape/handoff.txt",
    ];
    let output = server.command(Some(&sender)).args(args).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("artifact escapes --repo-root"));
    let bridged = server.bridge(Some(&sender), "msg.send", &args[2..]);
    assert_eq!(bridged["error"]["code"], "invalid_argument");
    assert!(
        bridged["error"]["message"]
            .as_str()
            .unwrap()
            .contains("artifact escapes --repo-root")
    );
    let messages = server.run(None, &["msg", "list", "--to", "crew-claude"]);
    assert!(messages["messages"].as_array().unwrap().is_empty());
    let journal = server.run(None, &["events", "list", "--after", "0", "--limit", "100"]);
    assert!(
        journal["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| !event["kind"].as_str().unwrap().starts_with("message."))
    );
    assert_eq!(
        fs::read_to_string(&outside_file).unwrap(),
        "This real file is outside the sender repository."
    );
}

#[test]
fn restored_agent_names_are_not_live_peers_until_reported_again() {
    let server = Server::with_persistence(true);
    let sender = server.peer("crew-codex", "codex");
    server.peer("crew-claude", "claude");
    server.peer("never-messaged", "claude");
    server.run(
        Some(&sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "crew-claude",
            "--key",
            "before-restart",
            "--kind",
            "handoff",
            "--summary",
            "Known from durable history",
        ],
    );
    server.stop();
    server.run(None, &["workspace", "create", "restart-trigger"]);
    let peers = server.run(None, &["peer", "list"]);
    assert!(peers["peers"].as_array().unwrap().is_empty(), "{peers}");
    let panes = server.run(None, &["pane", "list"]);
    let restored = panes["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["agent"]["name"] == "crew-codex")
        .expect("persisted sender metadata must be present for this regression");
    assert_eq!(restored["agent"]["source"], "restore");
    assert_eq!(restored["agent"]["instance"], 0);
    let restored_sender = restored["pane_id"].as_str().unwrap();
    server.error(
        Some(restored_sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "crew-claude",
            "--key",
            "before-restart",
            "--kind",
            "handoff",
            "--summary",
            "Known from durable history",
        ],
        "agent_not_found",
    );
    server.run(
        None,
        &[
            "pane",
            "report-agent",
            restored_sender,
            "--source",
            "fixture.inbox",
            "--agent",
            "codex",
            "--state",
            "idle",
        ],
    );
    let live = server.run(None, &["peer", "list"]);
    assert_eq!(live["peers"].as_array().unwrap().len(), 1);
    assert_eq!(live["peers"][0]["name"], "crew-codex");
    assert!(live["peers"][0]["instance"].as_u64().unwrap() > 0);
    server.error(
        Some(restored_sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "never-messaged",
            "--key",
            "unknown-after-restart",
            "--kind",
            "question",
            "--summary",
            "Placeholder is not live",
        ],
        "unknown_peer",
    );
    let accepted = server.run(
        Some(restored_sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "crew-claude",
            "--key",
            "known-after-restart",
            "--kind",
            "handoff",
            "--summary",
            "History still permits offline delivery",
        ],
    );
    assert_eq!(accepted["message"]["to"], "crew-claude");
    assert_eq!(accepted["deduplicated"], false);
}

#[test]
fn durable_messages_match_b_v1_cli_bridge_and_restart_contract() {
    let fixture = fixtures();
    let server = Server::new();
    let sender = server.peer("crew-codex", "codex");
    let receiver = server.peer("crew-claude", "claude");
    let peers = server.run(None, &["peer", "list"]);
    assert_eq!(peers["peers"].as_array().unwrap().len(), 2);
    for (name, pane, kind) in [
        ("crew-codex", &sender, "codex"),
        ("crew-claude", &receiver, "claude"),
    ] {
        let p = peers["peers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap();
        assert_eq!(p["pane_id"], pane.as_str());
        assert_eq!(p["agent"], kind);
        assert_eq!(p["source"], "fixture.inbox");
        assert!(p["instance"].is_u64());
        assert!(p.get("input_ready").is_none());
    }
    let schema = server.bridge(None, "api.schema", &[])["result"].clone();
    for method in ["peer.list", "msg.send", "msg.list", "msg.read", "msg.ack"] {
        assert!(
            schema["methods"]
                .as_array()
                .unwrap()
                .contains(&json!(method))
        );
    }
    assert_eq!(server.bridge(None, "peer.list", &[])["result"], peers);

    let root_a = server.root.join("sender-a");
    let root_b = server.root.join("sender-b");
    let expected = &fixture["messages"]["stored"];
    let artifact = &expected["artifacts"][0];
    let relative = artifact["path"].as_str().unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    for root in [&root_a, &root_b] {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(&source, path).unwrap();
    }
    let summary = expected["summary"].as_str().unwrap();
    let root_a = root_a.to_str().unwrap();
    let root_b = root_b.to_str().unwrap();
    let send_a = vec![
        "msg",
        "send",
        "--from",
        "crew-codex",
        "--to",
        "crew-claude",
        "--key",
        "review-b-1",
        "--kind",
        "review",
        "--summary",
        summary,
        "--repo-root",
        root_a,
        "--file",
        relative,
    ];
    let mut send_b = send_a.clone();
    send_b[13] = root_b;
    server.error(None, &send_a, "invalid_argument");
    server.error(Some(&receiver), &send_a, "invalid_argument");
    let stored = server.run(Some(&sender), &send_a);
    assert_eq!(stored["deduplicated"], false);
    let original = stored["message"].clone();
    let id = original["id"].as_str().unwrap();
    assert!(!id.is_empty());
    assert!(original["created_at"].as_str().unwrap().ends_with('Z'));
    for key in ["key", "from", "to", "kind", "reply_to", "summary", "acks"] {
        assert_eq!(original[key], expected[key]);
    }
    for key in ["path", "sha256", "bytes"] {
        assert_eq!(original["artifacts"][0][key], artifact[key]);
    }
    let read = server.run(None, &["msg", "read", id]);
    assert_eq!(read["message"], original, "read is retrieval only");
    server.error(
        None,
        &[
            "msg",
            "ack",
            id,
            "--by",
            "crew-claude",
            "--stage",
            "done",
            "--result",
            "ok",
        ],
        "invalid_argument",
    );
    server.error(
        None,
        &["msg", "ack", id, "--by", "crew-codex", "--stage", "read"],
        "invalid_argument",
    );
    let read_ack = server.run(
        None,
        &[
            "msg",
            "ack",
            id,
            "--by",
            "crew-claude",
            "--stage",
            "read",
            "--note",
            "Receiver fetched the message",
        ],
    );
    assert_eq!(read_ack["changed"], true);
    let read_retry = server.bridge(
        None,
        "msg.ack",
        &[
            id,
            "--by",
            "crew-claude",
            "--stage",
            "read",
            "--note",
            "Receiver fetched the message",
        ],
    );
    assert_eq!(read_retry["result"]["changed"], false);
    assert_eq!(read_retry["result"]["message"], read_ack["message"]);
    let retried = server.run(Some(&sender), &send_b);
    assert_eq!(retried["deduplicated"], true);
    assert_eq!(
        retried["message"], read_ack["message"],
        "retry preserves original root and current acknowledgments"
    );
    let mut changed = send_a.clone();
    let summary_at = changed.iter().position(|x| *x == "--summary").unwrap() + 1;
    changed[summary_at] = "different content";
    server.error(Some(&sender), &changed, "message_conflict");

    let reply = server.bridge(
        Some(&receiver),
        "msg.send",
        &[
            "--from",
            "crew-claude",
            "--to",
            "crew-codex",
            "--key",
            "reply-b",
            "--kind",
            "done",
            "--reply-to",
            id,
            "--summary",
            "Explicit reply",
        ],
    );
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(reply["result"]["message"]["reply_to"], id);
    assert_eq!(
        server.run(None, &["msg", "read", id])["message"],
        read_ack["message"],
        "a reply does not acknowledge its parent"
    );
    let open = server.run(None, &["msg", "list", "--to", "crew-claude", "--open"]);
    assert_eq!(
        open["messages"].as_array().unwrap().len(),
        1,
        "read-but-not-done stays open"
    );
    assert_eq!(open["messages"][0]["read"], true);
    assert_eq!(open["messages"][0]["done"], Value::Null);
    let unread = server.run(None, &["msg", "list", "--to", "crew-claude", "--unread"]);
    assert!(unread["messages"].as_array().unwrap().is_empty());
    let done = server.run(
        None,
        &[
            "msg",
            "ack",
            id,
            "--by",
            "crew-claude",
            "--stage",
            "done",
            "--result",
            "ok",
            "--note",
            "Review recorded in a reply",
        ],
    );
    assert_eq!(done["changed"], true);
    let done_retry = server.run(
        None,
        &[
            "msg",
            "ack",
            id,
            "--by",
            "crew-claude",
            "--stage",
            "done",
            "--result",
            "ok",
            "--note",
            "Review recorded in a reply",
        ],
    );
    assert_eq!(done_retry["changed"], false);
    assert_eq!(done_retry["message"], done["message"]);
    let old_read_retry = server.run(
        None,
        &[
            "msg",
            "ack",
            id,
            "--by",
            "crew-claude",
            "--stage",
            "read",
            "--note",
            "Receiver fetched the message",
        ],
    );
    assert_eq!(old_read_retry["changed"], false);
    assert_eq!(old_read_retry["message"], done["message"]);
    server.error(
        None,
        &[
            "msg",
            "ack",
            id,
            "--by",
            "crew-claude",
            "--stage",
            "done",
            "--result",
            "failed",
            "--note",
            "Review recorded in a reply",
        ],
        "ack_conflict",
    );
    let later = server.run(
        Some(&sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "crew-claude",
            "--key",
            "later",
            "--kind",
            "question",
            "--summary",
            "Later",
        ],
    );
    let first_page = server.run(
        None,
        &["msg", "list", "--to", "crew-claude", "--limit", "1"],
    );
    assert_eq!(first_page["messages"][0]["id"], id);
    assert_eq!(first_page["messages"][0]["done"], "ok");
    assert!(first_page["messages"][0].get("artifacts").is_none());
    assert_eq!(first_page["has_more"], true);
    let cursor = first_page["next_cursor"].as_str().unwrap();
    let second_page = server.bridge(
        None,
        "msg.list",
        &["--to", "crew-claude", "--after", cursor, "--limit", "1"],
    );
    assert_eq!(
        second_page["result"]["messages"][0]["id"],
        later["message"]["id"]
    );
    assert_eq!(second_page["result"]["has_more"], false);
    let last_cursor = second_page["result"]["next_cursor"].as_str().unwrap();
    let empty = server.run(
        None,
        &["msg", "list", "--to", "crew-claude", "--after", last_cursor],
    );
    assert!(empty["messages"].as_array().unwrap().is_empty());
    assert_eq!(empty["next_cursor"], last_cursor);
    assert_eq!(
        server.bridge(
            None,
            "msg.list",
            &["--to", "crew-claude", "--open", "--unread"]
        )["error"]["code"],
        "invalid_argument"
    );
    server.error(
        None,
        &[
            "msg",
            "list",
            "--to",
            "crew-claude",
            "--after",
            "not-an-inbox-cursor",
        ],
        "invalid_argument",
    );

    server.run(
        None,
        &[
            "pane",
            "release-agent",
            &receiver,
            "--source",
            "fixture.inbox",
            "--agent",
            "claude",
        ],
    );
    assert_eq!(
        server.run(None, &["peer", "list"])["peers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    server.error(Some(&receiver), &send_b, "agent_not_found");
    server.run(
        Some(&sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "crew-claude",
            "--key",
            "offline",
            "--kind",
            "handoff",
            "--summary",
            "Known offline peer",
        ],
    );
    let dedup_offline = server.run(Some(&sender), &send_b);
    assert_eq!(dedup_offline["message"], done["message"]);
    server.error(
        Some(&sender),
        &[
            "msg",
            "send",
            "--from",
            "crew-codex",
            "--to",
            "unknown-peer",
            "--key",
            "unknown",
            "--kind",
            "question",
            "--summary",
            "Unknown",
        ],
        "unknown_peer",
    );
    let journal = server.run(None, &["events", "list", "--after", "0", "--limit", "100"]);
    let messages: Vec<_> = journal["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"].as_str().unwrap().starts_with("message."))
        .collect();
    assert_eq!(
        messages
            .iter()
            .filter(|e| e["kind"] == "message.stored")
            .count(),
        4
    );
    assert_eq!(
        messages
            .iter()
            .filter(|e| e["kind"] == "message.acked")
            .count(),
        2
    );
    for event in messages {
        assert_eq!(event["pane_id"], Value::Null);
        let data = event["data"].as_object().unwrap();
        assert!(!data.contains_key("summary") && !data.contains_key("artifacts"));
        assert!(data.contains_key("message_id"));
        assert!(event["timestamp_ms"].is_u64() && event["cursor"].is_u64());
        assert_eq!(
            data.len(),
            if event["kind"] == "message.acked" && data["stage"] == "done" {
                4
            } else {
                3
            }
        );
    }

    server.stop();
    let new_sender = server.peer("crew-codex", "codex");
    assert_ne!(server.run(None, &["peer", "list"])["epoch"], peers["epoch"]);
    assert_eq!(
        server.bridge(None, "msg.read", &[id])["result"]["message"],
        done["message"]
    );
    let restored = server.run(
        None,
        &["msg", "list", "--to", "crew-claude", "--after", cursor],
    );
    assert_eq!(restored["messages"].as_array().unwrap().len(), 2);
    assert_eq!(restored["messages"][0]["id"], later["message"]["id"]);
    assert_eq!(
        server.run(Some(&new_sender), &send_b)["message"],
        done["message"]
    );
}

#[test]
fn help_and_schema_have_no_daemon_or_inbox_side_effects() {
    let server = Server::new();
    for args in [
        vec!["peer", "list", "--help"],
        vec![
            "msg",
            "send",
            "--repo-root",
            "/missing",
            "--file",
            "missing",
            "--help",
        ],
        vec!["msg", "list", "-h"],
        vec!["msg", "read", "--help"],
        vec!["msg", "ack", "--help"],
    ] {
        let output = server.command(None).args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        server
            .command(None)
            .args(["api", "schema"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!server.root.join("inbox.json").exists());
    assert!(!server.root.join("server.log").exists());
}
