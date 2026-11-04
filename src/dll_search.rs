//! Process-wide DLL search restriction for Windows.

/// Restrict later `LoadLibrary` lookups to the application directory, System32,
/// and directories added with `AddDllDirectory`.
///
/// portable-pty opens `conpty.dll` by bare file name. Without this call Windows
/// may satisfy that name from the current directory or `PATH`, which would run a
/// planted DLL inside crew. Call it first thing in `main`, before any PTY exists.
/// A failed call is logged only when `CREW_LOG` is set; the search then keeps the
/// previous behaviour. The PTY spawn path also calls it so in-process users (tests,
/// library callers) are covered; the work runs once per process.
#[cfg(windows)]
pub fn harden_dll_search() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(apply_dll_search_restriction);
}

#[cfg(windows)]
fn apply_dll_search_restriction() {
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, SetDefaultDllDirectories,
    };

    // SAFETY: The call takes only a flags value and touches no Rust-owned memory.
    // It changes the search policy for this process only.
    let ok = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) };
    if ok == 0 {
        crate::log!(
            "SetDefaultDllDirectories failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// No-op on platforms without Windows DLL search rules.
#[cfg(not(windows))]
pub fn harden_dll_search() {}
