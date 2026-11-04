#![cfg(windows)]

//! Startup DLL search hardening. A `conpty.dll` planted on PATH or in the current
//! directory must not be loaded, while a sideloaded `conpty.dll` beside the
//! executable must still be used. Every probe runs in a fresh process so the
//! process-wide search policy cannot leak between tests.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, LoadLibraryW};

const CONPTY_DLL: &str = "conpty.dll";
/// Set on the re-executed test binary to `hardened` or `unhardened`.
const PROBE_ENV: &str = "CREW_CONPTY_SEARCH_PROBE";
const PROBE_RESULT: &str = "CREW_CONPTY_PROBE_RESULT=";

/// Runs only when re-executed by a parent test; a no-op in a normal run.
/// It mirrors portable-pty's first lookup, `ConPtyFuncs::open("conpty.dll")`,
/// which is a bare `LoadLibraryW` call.
#[test]
fn probe_entry() {
    let Some(mode) = std::env::var_os(PROBE_ENV) else {
        return;
    };
    if mode.to_str() == Some("hardened") {
        crew::harden_dll_search();
    }
    println!("{PROBE_RESULT}{}", load_conpty_dll());
}

fn load_conpty_dll() -> String {
    let name: Vec<u16> = CONPTY_DLL.encode_utf16().chain([0]).collect();
    // SAFETY: `name` is NUL-terminated and outlives the call.
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    if module.is_null() {
        let code = std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or_default();
        return format!("none:{code}");
    }
    let mut buffer = vec![0u16; 32768];
    // SAFETY: `module` is the live handle returned above; the buffer length is passed exactly.
    let length =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    assert!(
        length > 0 && length < buffer.len(),
        "GetModuleFileNameW failed"
    );
    OsString::from_wide(&buffer[..length])
        .to_string_lossy()
        .into_owned()
}

fn system32() -> PathBuf {
    let root = std::env::var_os("SystemRoot").expect("SystemRoot is set on Windows");
    PathBuf::from(root).join("System32")
}

fn test_exe() -> PathBuf {
    std::env::current_exe().unwrap()
}

fn original_path() -> OsString {
    std::env::var_os("PATH").unwrap_or_default()
}

/// The current PATH with `dir` placed first, as a hostile PATH entry would be.
fn path_with_first(dir: &Path) -> OsString {
    let original = original_path();
    let entries = std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&original));
    std::env::join_paths(entries).unwrap()
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("crew-conpty-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `planted` holds a real, loadable DLL under the name `conpty.dll`. Loading it
/// proves which search location won. `work` is an empty current directory.
struct Fixture {
    root: TempDir,
    planted: PathBuf,
    work: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root = TempDir::new(tag);
        let planted = root.0.join("planted");
        let work = root.0.join("work");
        std::fs::create_dir_all(&planted).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::copy(system32().join("version.dll"), planted.join(CONPTY_DLL)).unwrap();
        Self {
            root,
            planted,
            work,
        }
    }
}

fn run_probe(exe: &Path, mode: &str, path: &OsStr, cwd: &Path) -> String {
    let output = Command::new(exe)
        .args(["--exact", "probe_entry", "--nocapture", "--test-threads=1"])
        .env(PROBE_ENV, mode)
        .env("PATH", path)
        .current_dir(cwd)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "probe ({mode}) failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // libtest may print its "test probe_entry ... " prefix on the same line.
    stdout
        .lines()
        .find_map(|line| {
            line.find(PROBE_RESULT)
                .map(|at| line[at + PROBE_RESULT.len()..].trim_end().to_owned())
        })
        .unwrap_or_else(|| panic!("probe ({mode}) printed no result:\n{stdout}"))
}

/// Case-insensitive, component-aware containment for Windows paths.
fn is_inside(loaded: &str, dir: &Path) -> bool {
    let mut prefix = dir.to_string_lossy().to_ascii_lowercase();
    if !prefix.ends_with('\\') {
        prefix.push('\\');
    }
    loaded.to_ascii_lowercase().starts_with(&prefix)
}

#[test]
fn control_unhardened_search_loads_planted_conpty_from_path() {
    // Control: proves the planted copy is reachable without the fix. If System32
    // supplies its own conpty.dll, that wins the search and the control is moot.
    if system32().join(CONPTY_DLL).exists() {
        eprintln!("skipping control: System32 provides its own {CONPTY_DLL}");
        return;
    }
    let fixture = Fixture::new("control-path");
    let loaded = run_probe(
        &test_exe(),
        "unhardened",
        &path_with_first(&fixture.planted),
        &fixture.work,
    );
    assert!(
        is_inside(&loaded, &fixture.planted),
        "control did not load the planted DLL: {loaded}"
    );
}

#[test]
fn control_unhardened_search_loads_planted_conpty_from_cwd() {
    if system32().join(CONPTY_DLL).exists() {
        eprintln!("skipping control: System32 provides its own {CONPTY_DLL}");
        return;
    }
    let fixture = Fixture::new("control-cwd");
    let loaded = run_probe(
        &test_exe(),
        "unhardened",
        &original_path(),
        &fixture.planted,
    );
    assert!(
        is_inside(&loaded, &fixture.planted),
        "control did not load the planted DLL from the current directory: {loaded}"
    );
}

/// Accepts only "not found" or a System32 copy; anything else means a planted DLL won.
fn assert_hardened_result(loaded: &str, planted: &Path) {
    assert!(
        !is_inside(loaded, planted),
        "planted conpty.dll was loaded: {loaded}"
    );
    assert!(
        loaded.starts_with("none:") || is_inside(loaded, &system32()),
        "conpty.dll resolved outside System32: {loaded}"
    );
}

#[test]
fn hardened_search_ignores_planted_conpty_on_path() {
    let fixture = Fixture::new("path");
    let loaded = run_probe(
        &test_exe(),
        "hardened",
        &path_with_first(&fixture.planted),
        &fixture.work,
    );
    assert_hardened_result(&loaded, &fixture.planted);
}

#[test]
fn hardened_search_ignores_planted_conpty_in_cwd() {
    let fixture = Fixture::new("cwd");
    let loaded = run_probe(&test_exe(), "hardened", &original_path(), &fixture.planted);
    assert_hardened_result(&loaded, &fixture.planted);
}

#[test]
fn hardened_search_still_loads_sideloaded_conpty_beside_exe() {
    let fixture = Fixture::new("sideload");
    let side = fixture.root.0.join("side");
    std::fs::create_dir_all(&side).unwrap();
    let exe = side.join("crew-conpty-probe.exe");
    std::fs::copy(test_exe(), &exe).unwrap();
    std::fs::copy(system32().join("version.dll"), side.join(CONPTY_DLL)).unwrap();

    let loaded = run_probe(
        &exe,
        "hardened",
        &path_with_first(&fixture.planted),
        &fixture.work,
    );
    let expected = side.join(CONPTY_DLL);
    assert!(
        loaded.eq_ignore_ascii_case(&expected.to_string_lossy()),
        "sideloaded conpty.dll was not used: got {loaded}, expected {}",
        expected.display()
    );
}

// ---- end-to-end: crew still starts a pane with a hostile conpty.dll first on PATH ----

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

fn is_child_named(pid: u32, parent: u32, name: &str) -> bool {
    processes()
        .iter()
        .any(|p| p.pid == pid && p.parent == parent && p.name.eq_ignore_ascii_case(name))
}

struct IsolatedServer {
    socket: String,
    config: PathBuf,
    state: PathBuf,
    path: OsString,
    shell: Option<(u32, u32)>,
}

impl IsolatedServer {
    fn new(path: OsString) -> Self {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let config = std::env::temp_dir().join(format!("crew-conpty-{suffix}.toml"));
        std::fs::write(
            &config,
            "persist = false\nshell = \"powershell.exe -NoProfile\"\n",
        )
        .unwrap();
        Self {
            socket: format!("crew-conpty-test-{suffix}.sock"),
            config,
            state: std::env::temp_dir().join(format!("crew-conpty-{suffix}.json")),
            path,
            shell: None,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
        command
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_CONFIG", &self.config)
            .env("CREW_STATE", &self.state)
            .env("PATH", &self.path)
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
        let _ = self.command().arg("kill-server").output();
        if let Some((pid, parent)) = self.shell {
            let deadline = Instant::now() + Duration::from_secs(5);
            while is_child_named(pid, parent, "powershell.exe") && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if is_child_named(pid, parent, "powershell.exe") {
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        let _ = std::fs::remove_file(&self.state);
        let _ = std::fs::remove_file(&self.config);
    }
}

#[test]
fn pane_starts_with_garbage_conpty_dll_first_on_path() {
    // A non-PE file cannot load, so this checks that crew still reaches the
    // conhost fallback. The loader-level check is in the probe tests above.
    let garbage = TempDir::new("garbage");
    std::fs::write(garbage.0.join(CONPTY_DLL), b"not a DLL").unwrap();
    let mut server = IsolatedServer::new(path_with_first(&garbage.0));

    server.run(&["workspace", "create", "conpty-test"]);
    server.run(&[
        "pane",
        "run",
        "p1",
        "Write-Output ('CONPTY_' + 'SHELL=' + $PID)",
    ]);
    let marker = server.run(&[
        "pane",
        "wait-output",
        "p1",
        "--regex",
        "CONPTY_SHELL=[0-9]+",
        "--timeout",
        "15000",
    ]);
    let line = marker["result"]["line"].as_str().unwrap();
    let shell: u32 = line
        .split("CONPTY_SHELL=")
        .nth(1)
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    let table = processes();
    let shell_process = table.iter().find(|p| p.pid == shell).unwrap();
    assert!(shell_process.name.eq_ignore_ascii_case("powershell.exe"));
    let helper = table
        .iter()
        .find(|p| p.pid == shell_process.parent)
        .unwrap();
    assert!(helper.name.eq_ignore_ascii_case("crew.exe"));
    let daemon = helper.parent;
    let hosts = table
        .iter()
        .filter(|p| p.parent == daemon && p.name.eq_ignore_ascii_case("conhost.exe"))
        .count();
    assert_eq!(hosts, 1, "expected one conhost host in the isolated server");
    server.shell = Some((shell, helper.pid));
}
