//! Read images on the attached client's desktop, then paste a durable local PNG path.
use anyhow::{Result, anyhow};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use std::path::PathBuf;

/// Some(true) is the dedicated image shortcut; Some(false) is ordinary Ctrl+V.
/// Text-only Ctrl+V must keep reaching the pane unchanged.
pub(crate) fn image_shortcut(event: &Event) -> Option<bool> {
    let Event::Key(key) = event else { return None };
    if key.kind == KeyEventKind::Release || key.code != KeyCode::Char('v') {
        return None;
    }
    match key.modifiers {
        KeyModifiers::CONTROL if cfg!(windows) => Some(false),
        modifiers if modifiers == KeyModifiers::CONTROL | KeyModifiers::ALT => Some(true),
        _ => None,
    }
}

#[cfg(windows)]
pub(crate) fn read_image() -> Result<Option<PathBuf>> {
    use anyhow::Context;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::{
        DataExchange::{
            CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            RegisterClipboardFormatW,
        },
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        Ole::CF_BITMAP,
    };

    // The native availability check keeps ordinary text paste fast and does not open
    // or change the clipboard. Windows synthesizes CF_BITMAP from DIB formats.
    // SAFETY: the PNG name is a valid, NUL-terminated UTF-16 constant for the call;
    // availability queries take scalar format IDs and transfer no ownership.
    let (png, has_png, has_bitmap) = unsafe {
        let png = RegisterClipboardFormatW([80, 78, 71, 0].as_ptr());
        (
            png,
            png != 0 && IsClipboardFormatAvailable(png) != 0,
            IsClipboardFormatAvailable(u32::from(CF_BITMAP)) != 0,
        )
    };
    if !has_png && !has_bitmap {
        return Ok(None);
    }
    let directory = dirs::data_local_dir()
        .ok_or_else(|| anyhow!("local data directory is unavailable"))?
        .join("crew")
        .join("clipboard");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let path = directory.join(format!("clipboard-{stamp}-{}.png", std::process::id()));
    if has_png {
        // SAFETY: OpenClipboard keeps the clipboard-owned HGLOBAL stable. GlobalSize
        // bounds the slice, GlobalLock must succeed, and the bytes are copied before
        // unlocking/closing on every result path. The clipboard handle is never freed.
        let bytes = unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return Err(std::io::Error::last_os_error()).context("open clipboard PNG");
            }
            let result = (|| -> Result<Vec<u8>> {
                let handle = GetClipboardData(png);
                if handle.is_null() {
                    return Err(std::io::Error::last_os_error()).context("read clipboard PNG");
                }
                let size = GlobalSize(handle);
                if size < 8 || size > isize::MAX as usize {
                    return Err(anyhow!("invalid clipboard PNG size"));
                }
                let pointer = GlobalLock(handle);
                if pointer.is_null() {
                    return Err(std::io::Error::last_os_error()).context("lock clipboard PNG");
                }
                let data = std::slice::from_raw_parts(pointer.cast::<u8>(), size);
                // Do not deserialize custom clipboard formats via legacy WinForms.
                let result = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
                    Ok(data.to_vec())
                } else {
                    Err(anyhow!("clipboard PNG format does not contain a PNG image"))
                };
                GlobalUnlock(handle);
                result
            })();
            CloseClipboard();
            result
        }?;
        std::fs::create_dir_all(&directory).context("create clipboard image directory")?;
        std::fs::write(&path, bytes).context("save clipboard PNG")?;
        return Ok(Some(path));
    }
    let powershell = PathBuf::from(
        std::env::var_os("SystemRoot")
            .ok_or_else(|| anyhow!("Windows system directory is unavailable"))?,
    )
    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    // Use Windows' installed STA clipboard/PNG support without another runtime package.
    // The script is constant; paths travel in an environment value, never as script text.
    let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$image = [System.Windows.Forms.Clipboard]::GetImage()
if ($null -eq $image) { exit 3 }
$file = $env:CREW_CLIPBOARD_FILE
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($file)) | Out-Null
$image.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
$image.Dispose()
"#;
    let output = std::process::Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-Command",
            script,
        ])
        .env("CREW_CLIPBOARD_FILE", &path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .context("read clipboard image with Windows PowerShell")?;
    match output.status.code() {
        Some(0) => Ok(Some(path)),
        Some(3) => Ok(None), // clipboard changed between the availability check and read
        _ => Err(anyhow!(
            "Windows clipboard image save failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

#[cfg(not(windows))]
pub(crate) fn read_image() -> Result<Option<PathBuf>> {
    Err(anyhow!(
        "clipboard image paste is currently supported on Windows only"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn shortcut_preserves_normal_paste_events_and_other_keys() {
        assert_eq!(image_shortcut(&Event::Paste("hello".into())), None);
        assert_eq!(
            image_shortcut(&Event::Key(KeyEvent::new(
                KeyCode::Char('v'),
                KeyModifiers::NONE
            ))),
            None
        );
        assert_eq!(
            image_shortcut(&Event::Key(KeyEvent::new(
                KeyCode::Char('v'),
                KeyModifiers::CONTROL
            ))),
            cfg!(windows).then_some(false)
        );
        assert_eq!(
            image_shortcut(&Event::Key(KeyEvent::new(
                KeyCode::Char('v'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            ))),
            Some(true)
        );
        assert_eq!(
            image_shortcut(&Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('v'),
                KeyModifiers::CONTROL,
                KeyEventKind::Release
            ))),
            None
        );
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "reads the desktop clipboard; run manually with an image copied"]
    fn desktop_clipboard_image_smoke() {
        let path = read_image()
            .unwrap()
            .expect("copy an image before this smoke test");
        println!("Saved clipboard image: {}", path.display());
        assert!(std::fs::metadata(path).unwrap().len() > 0);
    }
}
