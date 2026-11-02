//! Windows ConPTY bootstrap: set code pages before starting the configured shell.
use anyhow::{Context, Result, anyhow};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCP, SetConsoleCtrlHandler, SetConsoleOutputCP,
};

/// Resolve the shell using the bootstrap's cwd and inherited PATH before it starts.
pub(crate) fn resolve_shell(program: &str, cmd: &portable_pty::CommandBuilder) -> Result<PathBuf> {
    let cwd = std::path::absolute(cmd.get_cwd().map(Path::new).unwrap_or(Path::new(".")))?;
    let path = Path::new(program);
    let explicit = path.components().count() > 1 || path.is_absolute();
    let candidates = if explicit {
        let path = cwd.join(path);
        if program.to_ascii_lowercase().ends_with(".exe") {
            vec![path]
        } else {
            let mut with_exe = path.as_os_str().to_os_string();
            with_exe.push(".exe");
            vec![PathBuf::from(with_exe), path]
        }
    } else {
        let mut directories = vec![std::env::current_exe()?.parent().unwrap().to_path_buf()];
        for get_directory in [
            windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW,
            windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW,
        ] {
            let mut buffer = vec![0u16; 32768];
            // SAFETY: Both APIs write at most the supplied buffer capacity.
            let length =
                unsafe { get_directory(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
            anyhow::ensure!(
                length > 0 && length < buffer.len(),
                "cannot resolve system shell directory"
            );
            directories.push(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }
        if let Some(paths) = cmd.get_env("PATH") {
            directories.extend(std::env::split_paths(paths).filter(|p| !p.as_os_str().is_empty()));
        }
        let filename = if program.contains('.') {
            program.to_owned()
        } else {
            format!("{program}.exe")
        };
        directories
            .into_iter()
            .map(|dir| cwd.join(dir).join(&filename))
            .collect()
    };
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow!("pane shell not found: {program}"))
}

// Ctrl-C and Ctrl-Break are delivered to every process attached to the console. Keep the
// bootstrap waiting for its child; this per-process handler is not inherited by the shell.
unsafe extern "system" fn handle_control(event: u32) -> i32 {
    i32::from(matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT))
}

pub(crate) fn run(mut args: impl Iterator<Item = OsString>) -> Result<i32> {
    let shell = args.next().ok_or_else(|| anyhow!("missing pane shell"))?;
    // SAFETY: These APIs operate on this process's attached console; the handler is static.
    unsafe {
        if SetConsoleCP(65001) == 0 {
            return Err(std::io::Error::last_os_error()).context("set console input to UTF-8");
        }
        if SetConsoleOutputCP(65001) == 0 {
            return Err(std::io::Error::last_os_error()).context("set console output to UTF-8");
        }
        // The detached server's process group disables Ctrl-C; clear that inherited flag
        // so the shell and its children can handle Ctrl-C normally.
        if SetConsoleCtrlHandler(None, 0) == 0 {
            return Err(std::io::Error::last_os_error()).context("enable pane Ctrl-C handling");
        }
        if SetConsoleCtrlHandler(Some(handle_control), 1) == 0 {
            return Err(std::io::Error::last_os_error()).context("install pane control handler");
        }
    }
    let status = Command::new(shell)
        .args(args)
        .status()
        .context("start pane shell")?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::CommandBuilder;

    #[test]
    fn missing_shell_is_rejected_before_bootstrap() {
        let mut cmd = CommandBuilder::new("crew");
        cmd.env_remove("PATH");
        let error = resolve_shell("crew-nonexistent-shell-smoke", &cmd).unwrap_err();
        assert!(error.to_string().contains("pane shell not found"));
        assert!(resolve_shell("cmd", &cmd).unwrap().is_file());
    }

    #[test]
    fn shell_resolution_uses_pane_cwd_and_path() {
        let relative = PathBuf::from(format!(
            "target/crew-shell-resolution-{}",
            std::process::id()
        ));
        let root = std::env::current_dir().unwrap().join(&relative);
        std::fs::create_dir_all(&root).unwrap();
        let shell = root.join("custom.shell.exe");
        std::fs::write(&shell, []).unwrap();
        let mut cmd = CommandBuilder::new("crew");
        cmd.cwd(&relative);
        cmd.env("PATH", &root);
        assert_eq!(resolve_shell("custom.shell.exe", &cmd).unwrap(), shell);
        assert_eq!(
            resolve_shell(".\\custom.shell", &cmd).unwrap(),
            root.join(".\\custom.shell.exe")
        );
        assert_eq!(resolve_shell(shell.to_str().unwrap(), &cmd).unwrap(), shell);
        std::fs::remove_file(shell).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
