#![cfg(windows)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

struct Process {
    pid: u32,
    parent: u32,
    name: String,
}

fn processes() -> Vec<Process> {
    // SAFETY: A process-only snapshot is read using a correctly sized entry and closed below.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        assert_ne!(snapshot, INVALID_HANDLE_VALUE, "process snapshot failed");
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = Vec::new();
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap();
            found.push(Process {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        assert!(!found.is_empty(), "process snapshot returned no processes");
        found
    }
}

fn terminate(pid: u32) {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "could not terminate test process {pid}");
}

struct IsolatedServer {
    socket: String,
    config: PathBuf,
    state: PathBuf,
    ping: Option<(u32, u32)>,
    shell: Option<(u32, u32)>,
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
        let config = std::env::temp_dir().join(format!("crew-p5-{suffix}.toml"));
        std::fs::write(
            &config,
            "persist = false\nshell = \"powershell.exe -NoProfile\"\n",
        )
        .unwrap();
        Self {
            socket: format!("crew-p5-test-{suffix}.sock"),
            config,
            state: std::env::temp_dir().join(format!("crew-p5-{suffix}.json")),
            ping: None,
            shell: None,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
        command
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_CONFIG", &self.config)
            .env("CREW_STATE", &self.state)
            .env_remove("CREW_PANE_ID");
        command
    }

    fn run(&self, args: &[&str]) -> serde_json::Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for IsolatedServer {
    fn drop(&mut self) {
        for (item, name) in [(self.ping, "ping.exe"), (self.shell, "powershell.exe")] {
            if let Some((pid, parent)) = item
                && processes().iter().any(|p| {
                    p.pid == pid && p.parent == parent && p.name.eq_ignore_ascii_case(name)
                })
            {
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        let _ = self.command().arg("kill-server").output();
        let _ = std::fs::remove_file(&self.state);
        let _ = std::fs::remove_file(&self.config);
    }
}

struct TestTree {
    host: u32,
    helper: u32,
    shell: u32,
    ping: u32,
}

fn start_test_pane(server: &mut IsolatedServer) -> TestTree {
    server.run(&["workspace", "create", "p5-test"]);
    server.run(&[
        "pane",
        "run",
        "p1",
        "Write-Output ('P5_TEST_' + 'SHELL=' + $PID)",
    ]);
    let marker = server.run(&[
        "pane",
        "wait-output",
        "p1",
        "--regex",
        "P5_TEST_SHELL=[0-9]+",
        "--timeout",
        "15000",
    ]);
    let line = marker["result"]["line"].as_str().unwrap();
    let shell: u32 = line
        .split("P5_TEST_SHELL=")
        .nth(1)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    server.run(&["pane", "run", "p1", "ping -t 127.0.0.1"]);
    server.run(&[
        "pane",
        "wait-output",
        "p1",
        "--match",
        "TTL=",
        "--timeout",
        "15000",
    ]);

    let table = processes();
    let shell_process = table.iter().find(|p| p.pid == shell).unwrap();
    assert!(shell_process.name.eq_ignore_ascii_case("powershell.exe"));
    let helper = table
        .iter()
        .find(|p| p.pid == shell_process.parent)
        .unwrap();
    assert!(helper.name.eq_ignore_ascii_case("crew.exe"));
    let daemon = helper.parent;
    assert!(
        table
            .iter()
            .any(|p| p.pid == daemon && p.name.eq_ignore_ascii_case("crew.exe"))
    );
    let hosts: Vec<_> = table
        .iter()
        .filter(|p| p.parent == daemon && p.name.eq_ignore_ascii_case("conhost.exe"))
        .collect();
    assert_eq!(hosts.len(), 1, "expected one host in the isolated server");
    let ping = table
        .iter()
        .find(|p| p.parent == shell && p.name.eq_ignore_ascii_case("ping.exe"))
        .unwrap()
        .pid;
    server.ping = Some((ping, shell));
    server.shell = Some((shell, helper.pid));
    TestTree {
        host: hosts[0].pid,
        helper: helper.pid,
        shell,
        ping,
    }
}

fn wait_for_host_lost(server: &IsolatedServer, ping: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let pane = server.run(&["pane", "get", "p1"]);
        let found = pane["result"]["pane"]["host_lost"]["processes"]
            .as_array()
            .is_some_and(|ps| ps.iter().any(|p| p["pid"] == ping));
        if found {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "host loss was not detected: {pane}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn host_lost_keeps_surviving_grandchild_after_intermediate_shell_exits() {
    let mut server = IsolatedServer::new();
    let tree = start_test_pane(&mut server);
    terminate(tree.host);
    wait_for_host_lost(&server, tree.ping);

    terminate(tree.shell);
    std::thread::sleep(Duration::from_secs(2));
    let table = processes();
    assert!(
        !table
            .iter()
            .any(|p| p.pid == tree.shell || p.pid == tree.helper)
    );
    assert!(
        table.iter().any(|p| p.pid == tree.ping),
        "surviving child was killed"
    );
    let pane = server.run(&["pane", "get", "p1"]);
    assert!(
        pane["result"]["pane"]["host_lost"]["processes"]
            .as_array()
            .is_some_and(|ps| ps.iter().any(|p| p["pid"] == tree.ping)),
        "surviving child was not retained: {pane}"
    );

    server.run(&["pane", "close", "p1"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while processes().iter().any(|p| p.pid == tree.ping) {
        assert!(Instant::now() < deadline, "pane close did not stop ping");
        std::thread::sleep(Duration::from_millis(50));
    }
    server.ping = None;
}

#[test]
fn host_lost_keeps_descendants_when_helper_exits_before_detection() {
    for _ in 0..3 {
        let mut server = IsolatedServer::new();
        let tree = start_test_pane(&mut server);
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Stop-Process -Id {},{} -Force", tree.host, tree.helper),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "could not terminate test host and helper");
        std::thread::sleep(Duration::from_secs(2));
        let table = processes();
        assert!(table.iter().any(|p| p.pid == tree.shell), "shell was lost");
        assert!(table.iter().any(|p| p.pid == tree.ping), "ping was lost");
        wait_for_host_lost(&server, tree.ping);

        server.run(&["pane", "close", "p1"]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while processes()
            .iter()
            .any(|p| p.pid == tree.ping || p.pid == tree.shell)
        {
            assert!(
                Instant::now() < deadline,
                "pane close did not stop the surviving shell and ping"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        server.ping = None;
        server.shell = None;
    }
}

#[test]
fn live_host_is_not_reported_lost_when_helper_exits() {
    let mut server = IsolatedServer::new();
    let tree = start_test_pane(&mut server);
    // Keep the server alive while p1 closes so its reply cannot race server shutdown.
    server.run(&["workspace", "create", "keeper"]);
    terminate(tree.helper);
    std::thread::sleep(Duration::from_secs(2));
    let table = processes();
    assert!(table.iter().any(|p| p.pid == tree.host), "host exited");
    assert!(table.iter().any(|p| p.pid == tree.shell), "shell exited");
    let pane = server.run(&["pane", "get", "p1"]);
    assert!(
        pane["result"]["pane"]["host_lost"].is_null(),
        "live host was incorrectly reported lost: {pane}"
    );

    server.run(&["pane", "send-keys", "p1", "C-c"]);
    server.run(&[
        "pane",
        "wait-output",
        "p1",
        "--source",
        "visible",
        "--regex",
        "^PS .*>\\s*$",
        "--timeout",
        "10000",
    ]);
    server.run(&[
        "pane",
        "run",
        "p1",
        "Write-Output ('P5_TEST_' + 'STILL_ALIVE')",
    ]);
    server.run(&[
        "pane",
        "wait-output",
        "p1",
        "--match",
        "P5_TEST_STILL_ALIVE",
        "--timeout",
        "10000",
    ]);
    server.run(&["pane", "close", "p1"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while processes().iter().any(|p| p.pid == tree.shell) {
        assert!(
            Instant::now() < deadline,
            "pane close did not stop the shell"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    server.ping = None;
    server.shell = None;
}

#[test]
#[ignore = "known boundary: host and shell die before the first survivor scan; see crl 29"]
fn host_and_shell_exiting_together_do_not_orphan_running_child() {
    let mut server = IsolatedServer::new();
    let tree = start_test_pane(&mut server);
    // Give a healthy pane time to observe its process tree before the simultaneous failure.
    std::thread::sleep(Duration::from_millis(1200));
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("Stop-Process -Id {},{} -Force", tree.host, tree.shell),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "could not terminate test host and shell");
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        processes().iter().any(|p| p.pid == tree.ping),
        "surviving child was killed"
    );
    wait_for_host_lost(&server, tree.ping);
    server.run(&["pane", "close", "p1"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while processes().iter().any(|p| p.pid == tree.ping) {
        assert!(Instant::now() < deadline, "pane close did not stop ping");
        std::thread::sleep(Duration::from_millis(50));
    }
    server.ping = None;
    server.shell = None;
}
