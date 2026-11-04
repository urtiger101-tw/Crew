#![cfg(windows)]

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Server {
    root: PathBuf,
    socket: String,
}

impl Server {
    fn new() -> Self {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("crew-nesting-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("config.toml"),
            "persist = false\nshell = \"powershell.exe -NoLogo -NoProfile\"\n",
        )
        .unwrap();
        Self {
            root,
            socket: format!("crew-nesting-{suffix}.sock"),
        }
    }

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

    fn run(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn pane_attach_error(&self, clear: Option<&str>) -> String {
        let tag = clear.unwrap_or("nested");
        let output = self.root.join(format!("{tag}.txt"));
        let done = self.root.join(format!("{tag}.done"));
        let quote = |path: &std::path::Path| path.to_string_lossy().replace('\'', "''");
        let line = format!(
            "{}& '{}' attach -t missing 2>&1 | Out-File -Encoding utf8 '{}'; [IO.File]::WriteAllText('{}', 'done')",
            clear
                .map(|key| format!("Remove-Item Env:{key}; "))
                .unwrap_or_default(),
            env!("CARGO_BIN_EXE_crew").replace('\'', "''"),
            quote(&output),
            quote(&done),
        );
        self.run(&["pane", "run", "p1", &line]);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done.exists() {
            assert!(
                Instant::now() < deadline,
                "pane did not finish: {}",
                self.run(&["pane", "read", "p1"])
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        std::fs::read_to_string(output).unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn stale_markers_do_not_block_attach_but_live_pane_children_do() {
    let server = Server::new();
    server.run(&["workspace", "create", "nesting"]);

    // Older panes have only these two markers. Even when a restarted daemon reuses p1,
    // an outside shell with its inherited values is not a child of the replacement pane.
    for pane_marker in [Some("p1"), None, Some("p99999")] {
        let mut command = server.command();
        command.env("CREW", "1");
        if let Some(pane_marker) = pane_marker {
            command.env("CREW_PANE_ID", pane_marker);
        }
        let output = command.args(["attach", "-t", "missing"]).output().unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(error.contains("can't find session: missing"), "{error}");
        assert!(!error.contains("nested with care"), "{error}");
    }

    let pane = server.run(&["pane", "get", "p1"]);
    assert!(pane["result"]["pane"]["root_process_id"].as_u64().unwrap() > 0);
    let nested = server.pane_attach_error(None);
    assert!(nested.contains("nested with care"), "{nested}");
    assert!(!nested.contains("can't find session: missing"), "{nested}");
    let legacy_nested = server.pane_attach_error(Some("CREW_SERVER_PID"));
    assert!(
        legacy_nested.contains("nested with care"),
        "{legacy_nested}"
    );
    let forced = server.pane_attach_error(Some("CREW"));
    assert!(forced.contains("can't find session: missing"), "{forced}");
}

#[test]
fn stale_server_marker_attaches_after_isolated_restart() {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::io::{Read, Write};
    use std::sync::mpsc;

    let server = Server::new();
    server.run(&["workspace", "create", "before-restart"]);
    let pid_file = server.root.join("server-pid.txt");
    server.run(&[
        "pane",
        "run",
        "p1",
        &format!(
            "[IO.File]::WriteAllText('{}', $env:CREW_SERVER_PID)",
            pid_file.to_string_lossy().replace('\'', "''"),
        ),
    ]);
    let deadline = Instant::now() + Duration::from_secs(15);
    while !pid_file.exists() {
        assert!(
            Instant::now() < deadline,
            "server PID was not exported to pane"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let old_pid = std::fs::read_to_string(pid_file).unwrap();
    assert!(old_pid.parse::<u32>().unwrap() > 0);
    server.run(&["server", "stop"]);
    while server
        .command()
        .arg("ls")
        .output()
        .unwrap()
        .status
        .success()
    {
        assert!(Instant::now() < deadline, "isolated server did not stop");
        std::thread::sleep(Duration::from_millis(50));
    }
    server.run(&["workspace", "create", "after-restart"]);
    // The pane ID is deliberately reused by the replacement daemon.
    server.run(&["pane", "get", "p1"]);
    server.run(&["pane", "run", "p1", "Write-Output 'RESTART_ATTACH_OK'"]);

    // This harness opens a PTY in-process, so restrict conpty.dll lookup like crew's main does.
    crew::harden_dll_search();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = [0; 4096];
        while let Ok(size) = reader.read(&mut bytes) {
            if size == 0 {
                break;
            }
            // Keep draining through ConPTY shutdown even if the assertion receiver left.
            let _ = tx.send(bytes[..size].to_vec());
        }
    });
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_crew"));
    command.args(["attach", "-t", "after-restart"]);
    command.env("CREW_SOCKET", &server.socket);
    command.env("CREW_CONFIG", server.root.join("config.toml"));
    command.env("CREW_STATE", server.root.join("state.json"));
    command.env("CREW_LANG", "en");
    command.env("CREW", "1");
    command.env("CREW_PANE_ID", "p1");
    command.env("CREW_SERVER_PID", old_pid.trim());
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut output = Vec::new();
    let mut checked = 0;
    while !String::from_utf8_lossy(&output).contains("RESTART_ATTACH_OK") {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(bytes) => {
                output.extend(bytes);
                while checked + 4 <= output.len() {
                    if &output[checked..checked + 4] == b"\x1b[6n" {
                        writer.write_all(b"\x1b[1;1R").unwrap();
                        writer.flush().unwrap();
                        checked += 4;
                    } else {
                        checked += 1;
                    }
                }
            }
            Err(error) => {
                let _ = child.kill();
                panic!(
                    "stale client did not attach: {error}; {}",
                    String::from_utf8_lossy(&output)
                );
            }
        }
    }
    writer.write_all(b"\x02d").unwrap();
    writer.flush().unwrap();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "client did not detach cleanly");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("attached client did not detach");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
