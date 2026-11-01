#![cfg(windows)]

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct ProbeFile(PathBuf);

impl Drop for ProbeFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn write_probe() -> ProbeFile {
    let path = std::env::temp_dir().join(format!(
        "crew-p3-probe-{}-{}.ps1",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let probe = ProbeFile(path);
    let text = r#"
Write-Output ('P3INPUT=' + [Console]::InputEncoding.CodePage)
Write-Output ('P3OUTPUT=' + [Console]::OutputEncoding.CodePage)
[Console]::Write("`r")
[Console]::Write([char]0x53d1)
$width = [Console]::CursorLeft
[Console]::WriteLine()
Write-Output ('P3WIDTH=' + $width)
Write-Output 'P3TEXT=发•中'
Write-Output ('P3ENV=' + $env:P3_TEST_VALUE)
Write-Output 'P3TEST_DONE'
"#;
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend_from_slice(text.as_bytes());
    std::fs::write(&probe.0, bytes).unwrap();
    probe
}

#[test]
fn bootstrap_preserves_shell_commands_and_child_console_encoding() {
    let probe = write_probe();
    let path = probe.0.to_string_lossy();

    for shell in ["powershell.exe", "pwsh.exe", "cmd.exe"] {
        if shell == "pwsh.exe"
            && std::process::Command::new(shell)
                .arg("-Version")
                .output()
                .is_err()
        {
            eprintln!("pwsh.exe unavailable; skipping that executable");
            continue;
        }
        let args = if shell == "cmd.exe" {
            vec![
                "/d".into(),
                "/c".into(),
                "powershell.exe".into(),
                "-NoProfile".into(),
                "-File".into(),
                path.to_string(),
            ]
        } else {
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                format!("& '{}'", path.replace('\'', "''")),
            ]
        };
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = [0; 2048];
            let mut output = Vec::new();
            let mut checked = 0;
            while let Ok(n) = reader.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                output.extend_from_slice(&bytes[..n]);
                while checked + 4 <= output.len() {
                    if &output[checked..checked + 4] == b"\x1b[6n" {
                        let _ = writer.write_all(b"\x1b[1;1R");
                        let _ = writer.flush();
                        checked += 4;
                    } else {
                        checked += 1;
                    }
                }
                let _ = tx.send(output.clone());
                if output.windows(11).any(|part| part == b"P3TEST_DONE") {
                    break;
                }
            }
        });
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_crew"));
        command.arg("__pane_utf8");
        command.arg(shell);
        for arg in args {
            command.arg(arg);
        }
        command.env("CREW_PANE_ID", "p1");
        command.env("P3_TEST_VALUE", "retained");
        let mut child = pair.slave.spawn_command(command).unwrap();
        eprintln!("testing {shell}, bootstrap pid {:?}", child.process_id());
        drop(pair.slave);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut output = String::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining) {
                Ok(bytes) => {
                    output = String::from_utf8_lossy(&bytes).into_owned();
                    if output.contains("P3TEST_DONE") {
                        break;
                    }
                }
                Err(error) => {
                    let _ = child.kill();
                    panic!("{shell}: output timeout: {error}; partial={output:?}");
                }
            }
        }
        assert!(child.wait().unwrap().success(), "{shell}: nonzero exit");
        for expected in [
            "P3INPUT=65001",
            "P3OUTPUT=65001",
            "P3WIDTH=2",
            "P3TEXT=发•中",
            "P3ENV=retained",
            "P3TEST_DONE",
        ] {
            assert!(
                output.contains(expected),
                "{shell}: expected {expected:?}, got {output:?}"
            );
        }
    }
}

struct IsolatedServer {
    socket: String,
    state: PathBuf,
    config: PathBuf,
    running: bool,
}

impl IsolatedServer {
    fn new() -> Self {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        Self {
            socket: format!("codex-p3-test-{suffix}.sock"),
            state: std::env::temp_dir().join(format!("codex-p3-{suffix}.json")),
            config: std::env::temp_dir().join(format!("codex-p3-{suffix}.toml")),
            running: true,
        }
    }

    fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_crew"));
        command
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_STATE", &self.state)
            .env("CREW_CONFIG", &self.config)
            .env_remove("CREW_PANE_ID")
            .env_remove("TMUXLIKE_PANE_ID");
        command
    }

    fn run(&self, args: &[&str]) -> String {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn stop(&mut self) {
        self.run(&["kill-server"]);
        self.running = false;
    }
}

impl Drop for IsolatedServer {
    fn drop(&mut self) {
        if self.running {
            match self.command().arg("kill-server").output() {
                Ok(output) if output.status.success() => {}
                Ok(output) => eprintln!(
                    "isolated server cleanup failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
                Err(error) => eprintln!("isolated server cleanup failed: {error}"),
            }
        }
        let _ = std::fs::remove_file(&self.state);
        let _ = std::fs::remove_file(&self.config);
    }
}

#[test]
fn isolated_server_default_and_startup_commands_use_utf8() {
    let mut server = IsolatedServer::new();
    let probe = write_probe();
    let script = format!("& '{}'", probe.0.to_string_lossy().replace('\'', "''"));
    server.run(&[
        "workspace",
        "create",
        "p3-test",
        "--env",
        "P3_TEST_VALUE=retained",
    ]);
    let list: serde_json::Value = serde_json::from_str(&server.run(&["pane", "list"])).unwrap();
    let initial = list["result"]["panes"][0]["pane_id"].as_str().unwrap();
    server.run(&["pane", "run", initial, &script]);
    let created: serde_json::Value =
        serde_json::from_str(&server.run(&["tab", "create", "p3-startup", "--command", &script]))
            .unwrap();
    let startup = created["result"]["tab"]["active_pane"].as_str().unwrap();
    for pane in [initial, startup] {
        server.run(&[
            "pane",
            "wait-output",
            pane,
            "--match",
            "P3TEST_DONE",
            "--timeout",
            "15000",
        ]);
        let output = server.run(&["pane", "read", pane, "--source", "recent", "--lines", "30"]);
        for expected in [
            "P3INPUT=65001",
            "P3OUTPUT=65001",
            "P3WIDTH=2",
            "P3TEXT=发•中",
            "P3ENV=retained",
        ] {
            assert!(
                output.contains(expected),
                "{pane}: expected {expected:?}, got {output:?}"
            );
        }
    }
    server.run(&["pane", "run", startup, "ping -t 127.0.0.1"]);
    server.run(&[
        "pane",
        "wait-output",
        startup,
        "--match",
        "TTL=",
        "--timeout",
        "10000",
    ]);
    server.run(&["pane", "send-keys", startup, "C-c"]);
    server.run(&[
        "pane",
        "wait-output",
        startup,
        "--match",
        "Control-C",
        "--timeout",
        "5000",
    ]);
    server.run(&[
        "pane",
        "run",
        startup,
        "Write-Output ('P3_' + 'AFTER_CTRL')",
    ]);
    server.run(&[
        "pane",
        "wait-output",
        startup,
        "--match",
        "P3_AFTER_CTRL",
        "--timeout",
        "5000",
    ]);
    server.run(&["pane", "close", startup]);
    server.stop();
}

#[test]
fn bootstrap_rejects_use_outside_a_pane() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_crew"))
        .env_remove("CREW_PANE_ID")
        .arg("__pane_utf8")
        .arg("cmd.exe")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a Crew pane"));
}
