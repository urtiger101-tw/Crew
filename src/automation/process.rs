//! Direct argv execution for configured local adapters. Output files avoid pipe deadlocks.
use anyhow::{Context, Result, bail};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Files(PathBuf);

impl Drop for Files {
    fn drop(&mut self) {
        for name in ["input", "stdout", "stderr"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

pub fn run(argv: &[String], cwd: &Path, input: Option<&[u8]>, timeout_ms: u64) -> Result<Output> {
    let executable = argv.first().filter(|s| !s.trim().is_empty());
    let executable = executable.context("adapter command must contain an executable")?;
    if timeout_ms == 0 {
        bail!("adapter timeout_ms must be positive");
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(timeout_ms))
        .context("adapter timeout_ms exceeds the platform clock range")?;
    let files = Files(std::env::temp_dir().join(format!(
        "crew-automation-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&files.0)?;
    let output_file = |name| -> Result<File> {
        Ok(OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(files.0.join(name))?)
    };
    let stdin = if let Some(bytes) = input {
        std::fs::write(files.0.join("input"), bytes)?;
        Stdio::from(File::open(files.0.join("input"))?)
    } else {
        Stdio::null()
    };
    let mut command = Command::new(executable);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(stdin)
        .stdout(output_file("stdout")?)
        .stderr(output_file("stderr")?);
    // Keep timed-out adapter children in one process group on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("start adapter {executable}"))?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            #[cfg(windows)]
            {
                // Agent CLIs commonly spawn a runtime child; kill that tree as well.
                let _ = Command::new("taskkill.exe")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "adapter {executable} timed out after {timeout_ms} ms; task outcome requires inspection"
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(Output {
        status,
        stdout: std::fs::read(files.0.join("stdout"))?,
        stderr: std::fs::read(files.0.join("stderr"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_and_stdin_are_forwarded_without_shell_interpolation() {
        #[cfg(windows)]
        let args = [
            "powershell.exe",
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "[Console]::Out.Write([Console]::In.ReadToEnd()); [Console]::Error.Write('diagnostic')",
        ];
        #[cfg(not(windows))]
        let args = ["sh", "-c", "cat; printf diagnostic >&2"];
        let output = run(
            &args.map(str::to_owned),
            Path::new("."),
            Some(b"literal $(payload)"),
            10000,
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"literal $(payload)");
        assert_eq!(output.stderr, b"diagnostic");
    }

    #[test]
    fn slow_adapter_is_stopped() {
        #[cfg(windows)]
        let args = ["powershell.exe", "-NoProfile", "-Command", "Start-Sleep 30"];
        #[cfg(not(windows))]
        let args = ["sh", "-c", "sleep 30"];
        let started = Instant::now();
        assert!(
            run(&args.map(str::to_owned), Path::new("."), None, 100)
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
