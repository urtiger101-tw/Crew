//! Bounded, noninteractive argv execution. No implicit shell is used.
use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, Serialize)]
pub struct Output {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub output_incomplete: bool,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out && !self.output_incomplete
    }
}

/// Anchor an explicit relative executable path before setting the child's cwd.
/// Bare names still use normal PATH lookup. The root is the absolute directory
/// saved by install/link. Keeping missing paths absolute also retains ordinary
/// spawn-error logging instead of failing before a command record is created.
pub fn plugin_command(root: &Path, program: &str) -> Command {
    let path = Path::new(program);
    let has_directory = path.parent().is_some_and(|p| !p.as_os_str().is_empty());
    let executable = if !path.is_absolute() && has_directory {
        root.join(path)
    } else {
        path.to_owned()
    };
    let mut command = Command::new(executable);
    command.current_dir(root);
    command
}

fn reader(mut input: impl Read + Send + 'static, limit: usize) -> mpsc::Receiver<(Vec<u8>, bool)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        let mut truncated = false;
        loop {
            match input.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let take = n.min(limit.saturating_sub(kept.len()));
                    kept.extend_from_slice(&buf[..take]);
                    truncated |= take < n;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    truncated = true;
                    break;
                }
            }
        }
        let _ = tx.send((kept, truncated));
    });
    rx
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        // Every captured child is placed in its own process group below.
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn capture(mut command: Command, timeout: Duration, limit: usize) -> Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0200); // CREATE_NEW_PROCESS_GROUP
    }
    let mut child = command.spawn().context("could not start plugin command")?;
    let out = reader(child.stdout.take().expect("piped stdout"), limit);
    let err = reader(child.stderr.take().expect("piped stderr"), limit);
    let start = Instant::now();
    let (exit_code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), false),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                terminate(&mut child);
                break (None, true);
            }
            Err(e) => {
                terminate(&mut child);
                return Err(e.into());
            }
        }
    };
    let mut output_incomplete = false;
    let mut collect =
        |rx: mpsc::Receiver<(Vec<u8>, bool)>| match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(data) => data,
            Err(_) => {
                output_incomplete = true;
                (Vec::new(), true)
            }
        };
    let (stdout, out_cut) = collect(out);
    let (stderr, err_cut) = collect(err);
    if output_incomplete {
        terminate(&mut child);
    }
    Ok(Output {
        exit_code,
        timed_out,
        output_incomplete,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        truncated: out_cut || err_cut,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn explicit_relative_executable_is_anchored_before_child_cwd_changes() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "crew-relative-command-{}-{}",
            std::process::id(),
            super::super::store::stamp()
        ));
        let plugin = root.join("plugin");
        let caller = root.join("caller");
        for (dir, label) in [(&plugin, "plugin"), (&caller, "caller")] {
            std::fs::create_dir_all(dir.join("bin")).unwrap();
            let executable = dir.join("bin/action");
            std::fs::write(&executable, format!("#!/bin/sh\nprintf '{label}'\n")).unwrap();
            std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut command = plugin_command(&plugin, "./bin/action");
        // A distinct cwd must not change which executable was selected.
        command.current_dir(&caller);
        let output = capture(command, Duration::from_secs(2), 1024).unwrap();
        assert!(output.success());
        assert_eq!(output.stdout, "plugin");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn argv_is_not_shell_expanded_and_failures_keep_output() {
        let mut c = Command::new("printf");
        c.args(["%s", "$(echo injected); *"]);
        let output = capture(c, Duration::from_secs(2), 1024).unwrap();
        assert!(output.success());
        assert_eq!(output.stdout, "$(echo injected); *");
        let mut c = Command::new("sh");
        c.args(["-c", "printf error >&2; exit 7"]);
        let output = capture(c, Duration::from_secs(2), 1024).unwrap();
        assert_eq!(output.exit_code, Some(7));
        assert_eq!(output.stderr, "error");
        assert!(!output.success());
    }
    #[test]
    fn output_is_capped_and_timeouts_terminate_the_process_group() {
        let mut c = Command::new("sh");
        c.args(["-c", "printf 123456789"]);
        let output = capture(c, Duration::from_secs(2), 4).unwrap();
        assert_eq!(output.stdout, "1234");
        assert!(output.truncated);
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 10 & wait"]);
        let output = capture(c, Duration::from_millis(60), 1024).unwrap();
        assert!(output.timed_out);
        assert!(!output.output_incomplete);
    }
}
