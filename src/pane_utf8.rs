//! Windows ConPTY bootstrap: set code pages before starting the configured shell.
use anyhow::{Context, Result, anyhow};
use std::ffi::OsString;
use std::process::Command;
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCP, SetConsoleCtrlHandler, SetConsoleOutputCP,
};

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
