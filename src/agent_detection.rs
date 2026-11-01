//! Conservative, read-only process-kind hints. Explicit reports remain the lifecycle authority.
use crate::api::lock;
use crate::server::{Pane, Shared};
use serde::Serialize;
#[cfg(any(windows, test))]
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Detection {
    pub agent: String,
    pub process_id: u32,
    /// Only the executable basename is exposed, never command arguments or environment.
    pub executable: String,
    pub evidence: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seed {
    pane: usize,
    child: u32,
    foreground: Option<u32>,
}

fn seed(id: usize, p: &Pane) -> Option<Seed> {
    #[cfg(unix)]
    let foreground = p
        .master
        .process_group_leader()
        .and_then(|pid| u32::try_from(pid).ok());
    #[cfg(not(unix))]
    let foreground = None;
    Some(Seed {
        pane: id,
        child: p.child.process_id()?,
        foreground,
    })
}

/// I/O happens outside State's mutex. A second identity check prevents attaching a stale
/// result to a replaced pane child. No shell, agent command or external probe is launched.
pub fn refresh(shared: &Shared) {
    let seeds: Vec<_> = {
        let st = lock(shared);
        if st.cfg.agent_detection {
            st.panes.iter().map(|(&id, p)| (id, seed(id, p))).collect()
        } else {
            Vec::new()
        }
    };
    #[cfg(windows)]
    let processes = if seeds.iter().any(|(_, s)| s.is_some()) {
        windows_processes()
    } else {
        None
    };
    let results: Vec<_> = seeds
        .into_iter()
        .map(|(id, s)| {
            let result = s.and_then(|s| {
                #[cfg(target_os = "linux")]
                let result = linux_detect(s.foreground);
                #[cfg(target_os = "macos")]
                let result = macos_detect(s.foreground);
                #[cfg(windows)]
                let result = processes
                    .as_ref()
                    .and_then(|p| select_descendant(s.child, p));
                #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
                let result: Option<Detection> = None;
                result
            });
            (id, s, result)
        })
        .collect();
    let mut st = lock(shared);
    let enabled = st.cfg.agent_detection;
    let mut changed = Vec::new();
    if !enabled {
        for (&id, pane) in &mut st.panes {
            if pane.detected_agent.take().is_some() {
                changed.push(id);
            }
        }
    } else {
        for (id, observed, result) in results {
            if let Some(pane) = st.panes.get_mut(&id) {
                let result = if seed(id, pane) == observed {
                    result
                } else {
                    None
                };
                if pane.detected_agent != result {
                    pane.detected_agent = result;
                    changed.push(id);
                }
            }
        }
    }
    for pid in changed {
        let detected = st.panes.get(&pid).and_then(|p| p.detected_agent.clone());
        st.events.record(
            "agent.detected",
            Some(pid),
            serde_json::json!({"detected_agent": detected}),
        );
        st.relayout_pane(pid);
    }
}

pub fn backend() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux-foreground-process"
    } else if cfg!(target_os = "macos") {
        "macos-foreground-process"
    } else if cfg!(windows) {
        "windows-descendant-process"
    } else {
        "unsupported"
    }
}

#[cfg(any(target_os = "linux", test))]
fn base(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn kind(name: &str) -> Option<&'static str> {
    let name = name.strip_suffix(".exe").unwrap_or(name);
    match name {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "opencode" => Some("opencode"),
        "gemini" => Some("gemini"),
        "grok" => Some("grok"),
        "copilot" => Some("copilot"),
        "cursor-agent" => Some("cursor"),
        "pi" => Some("pi"),
        "omp" => Some("omp"),
        "mastracode" => Some("mastracode"),
        "kiro-cli" => Some("kiro"),
        "droid" => Some("droid"),
        "hermes" => Some("hermes"),
        "kilo" => Some("kilo"),
        "kimi" => Some("kimi"),
        "amp" => Some("amp"),
        "qwen" => Some("qwen"),
        "letta" => Some("letta"),
        "maki" => Some("maki"),
        "muse" => Some("muse"),
        "aider" => Some("aider"),
        _ => None,
    }
}

/// Match only a real executable or the interpreter's script position, never arbitrary args.
#[cfg(any(target_os = "linux", test))]
fn classify(executable: &str, args: &[String]) -> Option<&'static str> {
    if let Some(kind) = kind(base(executable)) {
        return Some(kind);
    }
    if !matches!(
        base(executable),
        "node" | "nodejs" | "bun" | "python" | "python3"
    ) {
        return None;
    }
    let script = args.get(1)?;
    if script.starts_with('-') {
        return None;
    }
    if let Some(kind) = kind(base(script)) {
        return Some(kind);
    }
    let normalized = script.replace('\\', "/");
    let parts: Vec<_> = normalized.split('/').collect();
    for (package, kind) in [
        ("@anthropic-ai/claude-code", "claude"),
        ("@openai/codex", "codex"),
        ("@google/gemini-cli", "gemini"),
        ("@github/copilot", "copilot"),
        ("@mariozechner/pi-coding-agent", "pi"),
    ] {
        let components: Vec<_> = package.split('/').collect();
        if parts
            .windows(components.len() + 1)
            .any(|w| w[0] == "node_modules" && w[1..] == components)
        {
            return Some(kind);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn linux_detect(foreground: Option<u32>) -> Option<Detection> {
    use std::io::Read;
    let pid = foreground?;
    let root = std::path::PathBuf::from(format!("/proc/{pid}"));
    let before = std::fs::read_to_string(root.join("stat")).ok()?;
    let executable = std::fs::read_link(root.join("exe")).ok()?;
    let mut bytes = Vec::new();
    std::fs::File::open(root.join("cmdline"))
        .ok()?
        .take(16_385)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 16_384 {
        return None;
    }
    let args: Vec<_> = bytes
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .map(|v| String::from_utf8(v.to_vec()).ok())
        .collect::<Option<_>>()?;
    let after = std::fs::read_to_string(root.join("stat")).ok()?;
    if std::fs::read_link(root.join("exe")).ok()? != executable {
        return None;
    }
    // pid/starttime must remain the same even if a process exits during observation.
    let identity = |s: &str| {
        s.rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().nth(19))
            .map(str::to_string)
    };
    if identity(&before).is_none() || identity(&before) != identity(&after) {
        return None;
    }
    let executable = executable.to_str()?;
    Some(Detection {
        agent: classify(executable, &args)?.into(),
        process_id: pid,
        executable: base(executable).to_string(),
        evidence: "linux-foreground-process",
    })
}

#[cfg(target_os = "macos")]
fn macos_detect(foreground: Option<u32>) -> Option<Detection> {
    let pid = foreground?;
    let mut buf = [0u8; 256];
    // SAFETY: buf is writable for its full advertised length and proc_name does not retain it.
    let n = unsafe {
        libc::proc_name(
            i32::try_from(pid).ok()?,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
        )
    };
    if n <= 0 || n as usize >= buf.len() {
        return None;
    }
    let name = std::str::from_utf8(&buf[..n as usize])
        .ok()?
        .trim_end_matches('\0');
    Some(Detection {
        agent: kind(name)?.into(),
        process_id: pid,
        executable: name.into(),
        evidence: "macos-foreground-process",
    })
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone)]
struct Process {
    pid: u32,
    parent: u32,
    executable: String,
}

/// `root` (if still running) and every running descendant of it, as (pid, executable),
/// root first then by pid. A descendant whose parent already exited still counts: Windows
/// keeps the parent id. Empty off Windows (a Unix PTY reaches EOF only when the session
/// leader is gone) or when the process table cannot be read.
pub(crate) fn live_tree(root: u32) -> Vec<(u32, String)> {
    #[cfg(windows)]
    {
        windows_processes()
            .map(|all| tree_of(root, &all))
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Vec::new()
    }
}

#[cfg(any(windows, test))]
fn tree_of(root: u32, processes: &[Process]) -> Vec<(u32, String)> {
    let mut members: Vec<u32> = vec![root];
    // breadth-first over parent links; bounded by the table size, so a pid cycle cannot loop
    let mut i = 0;
    while i < members.len() && members.len() <= processes.len() {
        let parent = members[i];
        for p in processes {
            if p.parent == parent && p.pid != parent && !members.contains(&p.pid) {
                members.push(p.pid);
            }
        }
        i += 1;
    }
    let mut out: Vec<(u32, String)> = processes
        .iter()
        .filter(|p| members.contains(&p.pid))
        .map(|p| (p.pid, p.executable.clone()))
        .collect();
    out.sort_by_key(|(pid, _)| (*pid != root, *pid));
    out
}

#[cfg(any(windows, test))]
fn select_descendant(root: u32, processes: &[Process]) -> Option<Detection> {
    if processes.len() > 16_384 || !processes.iter().any(|p| p.pid == root) {
        return None;
    }
    let by_id: BTreeMap<_, _> = processes.iter().map(|p| (p.pid, p)).collect();
    let mut found = Vec::new();
    for process in processes {
        let Some(agent) = kind(&process.executable.to_ascii_lowercase()) else {
            continue;
        };
        let mut current = process.pid;
        let mut visited = BTreeSet::new();
        for depth in 0..64 {
            if current == root {
                found.push((agent, depth, process));
                break;
            }
            if !visited.insert(current) {
                break;
            }
            current = match by_id.get(&current) {
                Some(p) => p.parent,
                None => break,
            };
        }
    }
    let kinds: BTreeSet<_> = found.iter().map(|(kind, _, _)| *kind).collect();
    if kinds.len() != 1 {
        return None;
    }
    found.sort_by_key(|(_, depth, p)| (*depth, p.pid));
    let (agent, _, process) = found.first()?;
    Some(Detection {
        agent: (*agent).into(),
        process_id: process.pid,
        executable: process.executable.clone(),
        evidence: "windows-descendant-process",
    })
}

#[cfg(windows)]
fn windows_processes() -> Option<Vec<Process>> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    // SAFETY: a process-only read-only snapshot needs no injected code or elevated handle.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    // RAII closes the handle on every early return.
    struct Snapshot(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let _guard = Snapshot(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut records = Vec::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) } == 0 {
        return None;
    }
    loop {
        if records.len() >= 16_384 {
            return None;
        }
        let len = entry.szExeFile.iter().position(|c| *c == 0)?;
        records.push(Process {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            executable: String::from_utf16(&entry.szExeFile[..len]).ok()?,
        });
        if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
            if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
                return None;
            }
            break;
        }
    }
    Some(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn process_matching_does_not_use_printed_text_or_arbitrary_arguments() {
        assert_eq!(classify("/opt/codex", &[]), Some("codex"));
        assert_eq!(classify("/usr/bin/sleep", &args(&["codex", "50"])), None);
        assert_eq!(classify("/bin/echo", &args(&["echo", "codex"])), None);
        assert_eq!(
            classify("/usr/bin/node", &args(&["node", "-e", "codex"])),
            None
        );
        assert_eq!(
            classify(
                "/usr/bin/node",
                &args(&["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js"])
            ),
            Some("codex")
        );
        assert_eq!(
            classify("/usr/bin/node", &args(&["node", "/x/codex-example.js"])),
            None
        );
        assert_eq!(
            classify(
                "/usr/bin/node",
                &args(&["node", "/x/ordinary.js", "@openai/codex"])
            ),
            None
        );
        assert_eq!(
            kind("agent"),
            None,
            "generic executable names are ambiguous"
        );
    }
    #[test]
    fn windows_tree_selection_excludes_unrelated_and_ambiguous_processes() {
        let process = |pid, parent, exe: &str| Process {
            pid,
            parent,
            executable: exe.into(),
        };
        let mut tree = vec![
            process(1, 0, "powershell.exe"),
            process(2, 1, "cmd.exe"),
            process(3, 2, "codex.exe"),
            process(4, 99, "claude.exe"),
        ];
        assert_eq!(select_descendant(1, &tree).unwrap().process_id, 3);
        tree.push(process(5, 1, "claude.exe"));
        assert!(select_descendant(1, &tree).is_none());
        assert!(select_descendant(98, &tree).is_none());
        assert!(
            select_descendant(
                1,
                &[
                    process(1, 0, "cmd.exe"),
                    process(2, 3, "codex.exe"),
                    process(3, 2, "cmd.exe")
                ]
            )
            .is_none()
        );
    }

    fn proc(pid: u32, parent: u32, exe: &str) -> Process {
        Process {
            pid,
            parent,
            executable: exe.into(),
        }
    }

    #[test]
    fn live_tree_keeps_orphaned_descendants_and_skips_strangers() {
        // 10 = shell (already exited, so absent); 20 = codex started by it; 30 = codex child;
        // 40 = unrelated; 50 = a pid-reuse cycle that must not hang the walk
        let table = vec![
            proc(20, 10, "codex.exe"),
            proc(30, 20, "node.exe"),
            proc(40, 1, "explorer.exe"),
            proc(50, 50, "self-parent.exe"),
        ];
        assert_eq!(
            tree_of(10, &table),
            vec![(20, "codex.exe".to_string()), (30, "node.exe".to_string())]
        );
        assert_eq!(tree_of(99, &table), vec![]);
        assert_eq!(
            tree_of(50, &table),
            vec![(50, "self-parent.exe".to_string())]
        );
    }

    #[test]
    fn live_tree_lists_a_running_root_first() {
        let table = vec![proc(7, 5, "child.exe"), proc(5, 1, "shell.exe")];
        assert_eq!(
            tree_of(5, &table),
            vec![(5, "shell.exe".to_string()), (7, "child.exe".to_string())]
        );
    }
}
