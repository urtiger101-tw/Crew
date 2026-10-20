//! Path spelling at external-process boundaries, separate from canonical identity.
use crate::api::ApiError;
use std::path::{Path, PathBuf};

// Keep canonical paths for filesystem identity checks and metadata. External
// processes receive ordinary Windows paths so Git and shells can resolve them.
pub(crate) fn for_child(path: &Path) -> Result<PathBuf, ApiError> {
    #[cfg(windows)]
    {
        let text = path
            .to_str()
            .ok_or_else(|| ApiError::invalid("process paths must be valid UTF-8"))?;
        windows_argument_path(text).map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        Ok(path.to_owned())
    }
}

#[cfg(any(windows, test))]
fn windows_argument_path(path: &str) -> Result<String, ApiError> {
    let unsupported = || {
        ApiError::invalid("unsupported Windows verbatim or device namespace for child processes")
    };
    if path.starts_with(r"\\.\") {
        return Err(unsupported());
    }
    let Some(verbatim) = path.strip_prefix(r"\\?\") else {
        return Ok(path.to_owned());
    };
    let ordinary_component = |part: &str| {
        !part.ends_with(' ')
            && !part.ends_with('.')
            && !part
                .chars()
                .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '|' | '?' | '*'))
    };
    let (converted, components) = if let Some(unc) = verbatim.strip_prefix(r"UNC\") {
        let mut parts = unc.splitn(3, '\\');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        if server.is_empty()
            || share.is_empty()
            || !ordinary_component(server)
            || !ordinary_component(share)
        {
            return Err(ApiError::invalid(
                "child processes require a complete UNC server/share path with ordinary Windows names",
            ));
        }
        (format!(r"\\{unc}"), parts.next().unwrap_or_default())
    } else if verbatim
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && verbatim.as_bytes().get(1) == Some(&b':')
        && verbatim.as_bytes().get(2) == Some(&b'\\')
    {
        (verbatim.to_owned(), &verbatim[3..])
    } else {
        // Volume GUIDs and other device namespaces have no equivalent drive/UNC spelling.
        return Err(unsupported());
    };
    for part in components.split('\\') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        let numbered_device = stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            });
        if !ordinary_component(part)
            || matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            || numbered_device
        {
            return Err(ApiError::invalid(
                "path requires Windows verbatim semantics; cannot pass it to a child process without changing its meaning",
            ));
        }
    }
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_process_paths_preserve_drive_unc_unicode_and_literal_characters() {
        for (input, expected) in [
            (
                r"\\?\C:\checkout 中文 with spaces",
                r"C:\checkout 中文 with spaces",
            ),
            (r"\\?\c:\", r"c:\"),
            (
                r"\\?\UNC\server\share\中文 $() & repo",
                r"\\server\share\中文 $() & repo",
            ),
            (r"\\?\UNC\server\share", r"\\server\share"),
            (
                r"\\?\D:\.git\COM10\console.txt",
                r"D:\.git\COM10\console.txt",
            ),
        ] {
            assert_eq!(windows_argument_path(input).unwrap(), expected);
        }
    }

    #[test]
    fn windows_process_paths_preserve_ordinary_paths_and_reject_other_namespaces() {
        for path in [r"C:\中文 repo", r"\\server\share\repo"] {
            assert_eq!(windows_argument_path(path).unwrap(), path);
        }
        for path in [
            r"\\?\Volume{1234}\repo",
            r"\\?\GLOBALROOT\Device\repo",
            r"\\.\C:\repo",
            r"\\?\C:relative",
            r"\\?\not-a-drive\repo",
        ] {
            assert_eq!(
                windows_argument_path(path).unwrap_err().code,
                "invalid_argument"
            );
        }
    }

    #[test]
    fn windows_process_paths_reject_paths_whose_win32_meaning_would_change() {
        for part in [
            "repo.",
            "repo ",
            ".",
            "..",
            "NUL",
            "nul.txt",
            "CON .txt",
            "PRN",
            "AUX",
            "COM1",
            "COM¹",
            "LPT9.txt",
            "LPT³",
            "CONIN$",
            "CONOUT$",
            "repo:stream",
            "a/b",
            "a?b",
            "a\nb",
        ] {
            for prefix in [r"\\?\C:\", r"\\?\UNC\server\share\"] {
                let path = format!("{prefix}{part}\\child");
                assert_eq!(
                    windows_argument_path(&path).unwrap_err().code,
                    "invalid_argument",
                    "{path}"
                );
            }
        }
        for path in [
            r"\\?\UNC\server",
            r"\\?\UNC\server\",
            r"\\?\UNC\\share",
            r"\\?\UNC\.\share\repo",
            r"\\?\UNC\?\C:\repo",
            r"\\?\UNC\server\..\repo",
            r"\\?\UNC\server.\share\repo",
            r"\\?\UNC\server\share \repo",
            r"\\?\UNC\server\share:stream\repo",
        ] {
            assert_eq!(
                windows_argument_path(path).unwrap_err().code,
                "invalid_argument"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_process_paths_preserve_backslashes_and_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;
        for bytes in [
            b"/tmp/\\\\?\\C:\\repo".as_slice(),
            b"/tmp/\\\\?\\UNC\\server\\share",
            b"/tmp/non-utf8-\xff",
        ] {
            let path = Path::new(std::ffi::OsStr::from_bytes(bytes));
            assert_eq!(for_child(path).unwrap().as_os_str().as_bytes(), bytes);
        }
    }

    #[cfg(windows)]
    #[test]
    fn native_windows_process_paths_convert_paths_without_lossy_decoding() {
        use std::os::windows::ffi::OsStringExt;
        assert_eq!(
            for_child(Path::new(r"\\?\C:\中文 repo")).unwrap(),
            Path::new(r"C:\中文 repo")
        );
        let invalid = PathBuf::from(std::ffi::OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            0xd800,
        ]));
        assert_eq!(for_child(&invalid).unwrap_err().code, "invalid_argument");
    }
}
