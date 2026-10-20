//! Git's NUL-delimited porcelain is used as data; user values are never shell code.
use crate::api::ApiError;
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(super) struct Entry {
    pub path: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
    pub primary: bool,
}

pub(super) fn path_text(path: &Path) -> Result<&str, ApiError> {
    path.to_str()
        .ok_or_else(|| ApiError::invalid("worktree paths must be valid UTF-8"))
}

// Keep canonical paths for filesystem identity checks. Only Git arguments need the
// ordinary Windows spelling: `worktree add` rejects canonicalize()'s \\?\ prefix.
fn argument_path(path: &Path) -> Result<PathBuf, ApiError> {
    #[cfg(windows)]
    {
        windows_argument_path(path_text(path)?).map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        Ok(path.to_owned())
    }
}

#[cfg(any(windows, test))]
fn windows_argument_path(path: &str) -> Result<String, ApiError> {
    let unsupported =
        || ApiError::invalid("unsupported Windows verbatim or device namespace for Git");
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
                "Git requires a complete UNC server/share path with ordinary Windows names",
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
                "path requires Windows verbatim semantics; cannot pass it to Git without changing its meaning",
            ));
        }
    }
    Ok(converted)
}

fn absolute(path: &str) -> Result<&Path, ApiError> {
    let value = Path::new(path);
    if path.is_empty() || path.contains('\0') || !value.is_absolute() {
        return Err(ApiError::invalid(
            "worktree paths must be absolute and must not contain NUL",
        ));
    }
    Ok(value)
}

pub(super) fn existing_dir(path: &str) -> Result<PathBuf, ApiError> {
    let path = absolute(path)?;
    let path = path.canonicalize().map_err(|e| {
        ApiError::new(
            "path_not_found",
            format!("cannot resolve {}: {e}", path.display()),
        )
    })?;
    if !path.is_dir() {
        return Err(ApiError::invalid("worktree path must be a directory"));
    }
    path_text(&path)?;
    Ok(path)
}

pub(super) fn new_target(path: &str) -> Result<PathBuf, ApiError> {
    let path = absolute(path)?;
    match path.symlink_metadata() {
        Ok(_) => {
            return Err(ApiError::new(
                "worktree_exists",
                "destination already exists; choose a new path or open an existing worktree",
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(ApiError::invalid(format!(
                "cannot inspect destination: {e}"
            )));
        }
    }
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let part = ancestor
            .file_name()
            .ok_or_else(|| ApiError::invalid("cannot resolve destination parent"))?;
        suffix.push(part.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| ApiError::invalid("destination has no parent"))?;
    }
    let mut target = ancestor
        .canonicalize()
        .map_err(|e| ApiError::invalid(format!("cannot resolve destination parent: {e}")))?;
    if !target.is_dir() {
        return Err(ApiError::invalid("destination parent must be a directory"));
    }
    for part in suffix.into_iter().rev() {
        target.push(part);
    }
    path_text(&target)?;
    Ok(target)
}

fn run(repo: &Path, args: &[&str]) -> Result<Output, ApiError> {
    let mut command = Command::new("git");
    // An existing executable is a file, so no post-checkout hook can exist below it.
    // This avoids relying on platform-specific /dev/null or creating temporary hook dirs.
    let mut no_hooks = OsString::from("core.hooksPath=");
    let executable = std::env::current_exe()
        .map_err(|e| ApiError::new("git_failed", format!("cannot disable Git hooks: {e}")))?;
    no_hooks.push(argument_path(&executable)?);
    command
        .arg("-C")
        .arg(argument_path(repo)?)
        .arg("-c")
        .arg(no_hooks)
        .args(["-c", "core.fsmonitor=false", "-c", "core.quotePath=false"])
        .args(args)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    // A server started from another worktree must not accidentally operate on that repo.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_NAMESPACE",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
    ] {
        command.env_remove(key);
    }
    command.output().map_err(|e| {
        ApiError::new(
            if e.kind() == std::io::ErrorKind::NotFound {
                "git_not_found"
            } else {
                "git_failed"
            },
            format!("cannot run Git: {e}"),
        )
    })
}

fn failure(output: &Output) -> ApiError {
    let stderr: String = String::from_utf8_lossy(&output.stderr)
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(2000)
        .collect();
    ApiError::new(
        "git_failed",
        format!("Git exited with {}: {}", output.status, stderr.trim()),
    )
}

fn success(repo: &Path, args: &[&str]) -> Result<Vec<u8>, ApiError> {
    let output = run(repo, args)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(failure(&output))
    }
}

pub(super) fn repository(path: &str) -> Result<PathBuf, ApiError> {
    let path = existing_dir(path)?;
    let output = run(&path, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Err(ApiError::new(
            "not_git_repository",
            format!("{} is not a Git checkout", path.display()),
        ));
    }
    let stdout = std::str::from_utf8(&output.stdout)
        .map_err(|_| ApiError::invalid("repository path must be valid UTF-8"))?;
    // Git appends one LF. Do not trim real whitespace in the repository's name.
    let root = stdout.strip_suffix('\n').unwrap_or(stdout);
    existing_dir(root)
}

pub(super) fn list(repo: &Path) -> Result<Vec<Entry>, ApiError> {
    parse(&success(repo, &["worktree", "list", "--porcelain", "-z"])?)
}

fn git_path(repo: &Path, flag: &str) -> Result<PathBuf, ApiError> {
    let output = success(repo, &["rev-parse", "--path-format=absolute", flag])?;
    let text = std::str::from_utf8(&output)
        .map_err(|_| ApiError::new("invalid_git_output", "Git returned a non-UTF-8 directory"))?;
    existing_dir(text.strip_suffix('\n').unwrap_or(text))
}

/// A stale registration or changed .git pointer must never authorize a removal.
pub(super) fn validate_checkout(repo: &Path, target: &Path, primary: bool) -> Result<(), ApiError> {
    let mismatch = || {
        ApiError::new(
            "worktree_mismatch",
            "checkout .git directory does not match this repository's worktree registration",
        )
    };
    if repository(path_text(target)?)? != target
        || git_path(repo, "--git-common-dir")? != git_path(target, "--git-common-dir")?
    {
        return Err(mismatch());
    }
    if !primary {
        let directory = git_path(target, "--absolute-git-dir")?;
        let backlink = std::fs::read_to_string(directory.join("gitdir")).map_err(|_| mismatch())?;
        let backlink = Path::new(backlink.strip_suffix('\n').unwrap_or(&backlink))
            .canonicalize()
            .map_err(|_| mismatch())?;
        if backlink != target.join(".git").canonicalize().map_err(|_| mismatch())? {
            return Err(mismatch());
        }
    }
    Ok(())
}

fn parse(bytes: &[u8]) -> Result<Vec<Entry>, ApiError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ApiError::new(
            "invalid_git_output",
            "Git returned a non-UTF-8 worktree path",
        )
    })?;
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    for field in text.split('\0') {
        if field.is_empty() {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            continue;
        }
        let (key, value) = field.split_once(' ').unwrap_or((field, ""));
        if key == "worktree" {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(Entry {
                path: value.into(),
                primary: entries.is_empty(),
                ..Entry::default()
            });
            continue;
        }
        let entry = current.as_mut().ok_or_else(|| {
            ApiError::new("invalid_git_output", "Git worktree record has no path")
        })?;
        match key {
            "HEAD" => entry.head = Some(value.into()),
            "branch" => {
                entry.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).into())
            }
            "detached" => entry.detached = true,
            "bare" => entry.bare = true,
            "locked" => entry.locked = Some(value.into()),
            "prunable" => entry.prunable = Some(value.into()),
            _ => {} // Future optional fields must not break older Crew versions.
        }
    }
    if let Some(entry) = current {
        entries.push(entry);
    }
    if entries.is_empty() || entries.iter().any(|e| e.path.is_empty()) {
        return Err(ApiError::new(
            "invalid_git_output",
            "Git returned an empty worktree listing",
        ));
    }
    Ok(entries)
}

pub(super) fn validate_branch(repo: &Path, branch: &str) -> Result<(), ApiError> {
    if branch.is_empty()
        || branch.starts_with('-')
        || branch == "HEAD"
        || branch.chars().any(char::is_control)
    {
        return Err(ApiError::invalid(
            "branch must be a valid literal Git branch name",
        ));
    }
    let reference = format!("refs/heads/{branch}");
    if !run(repo, &["check-ref-format", &reference])?
        .status
        .success()
    {
        return Err(ApiError::invalid(
            "branch must be a valid literal Git branch name",
        ));
    }
    Ok(())
}

pub(super) fn create(
    repo: &Path,
    target: &Path,
    branch: &str,
    base: Option<&str>,
) -> Result<(), ApiError> {
    let argument = argument_path(target)?;
    let path = path_text(&argument)?;
    let reference = format!("refs/heads/{branch}");
    let exists = run(repo, &["show-ref", "--verify", "--quiet", &reference])?;
    if exists.status.success() {
        if base.is_some() {
            return Err(ApiError::invalid(
                "--base can only be used when creating a new branch",
            ));
        }
        success(repo, &["worktree", "add", "--", path, branch])?;
    } else if exists.status.code() == Some(1) {
        let base = base.unwrap_or("HEAD");
        if base.is_empty() || base.chars().any(char::is_control) {
            return Err(ApiError::invalid(
                "base must be a nonempty Git revision without control characters",
            ));
        }
        let expression = format!("{base}^{{commit}}");
        let output = run(
            repo,
            &["rev-parse", "--verify", "--end-of-options", &expression],
        )?;
        if !output.status.success() {
            return Err(ApiError::new(
                "invalid_base",
                "base does not resolve to a commit",
            ));
        }
        let commit = std::str::from_utf8(&output.stdout)
            .map_err(|_| ApiError::new("invalid_git_output", "Git returned an invalid commit ID"))?
            .trim();
        if commit.is_empty() || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::new(
                "invalid_git_output",
                "Git returned an invalid commit ID",
            ));
        }
        success(repo, &["worktree", "add", "-b", branch, "--", path, commit])?;
    } else {
        return Err(failure(&exists));
    }
    Ok(())
}

pub(super) fn dirty(path: &Path) -> Result<bool, ApiError> {
    // Ignored files can contain valuable local data; never discard them implicitly.
    Ok(!success(
        path,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored",
        ],
    )?
    .is_empty())
}

pub(super) fn remove(repo: &Path, target: &Path) -> Result<(), ApiError> {
    let argument = argument_path(target)?;
    success(repo, &["worktree", "remove", "--", path_text(&argument)?])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_git_arguments_preserve_drive_unc_unicode_and_literal_characters() {
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
    fn windows_git_arguments_preserve_ordinary_paths_and_reject_other_namespaces() {
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
    fn windows_git_arguments_reject_paths_whose_win32_meaning_would_change() {
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
    fn unix_git_arguments_preserve_backslashes_and_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;
        for bytes in [
            b"/tmp/\\\\?\\C:\\repo".as_slice(),
            b"/tmp/\\\\?\\UNC\\server\\share",
            b"/tmp/non-utf8-\xff",
        ] {
            let path = Path::new(std::ffi::OsStr::from_bytes(bytes));
            assert_eq!(argument_path(path).unwrap().as_os_str().as_bytes(), bytes);
        }
    }

    #[cfg(windows)]
    #[test]
    fn native_windows_git_arguments_convert_paths_without_lossy_decoding() {
        use std::os::windows::ffi::OsStringExt;
        assert_eq!(
            argument_path(Path::new(r"\\?\C:\中文 repo")).unwrap(),
            Path::new(r"C:\中文 repo")
        );
        let invalid = PathBuf::from(OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            0xd800,
        ]));
        assert_eq!(
            argument_path(&invalid).unwrap_err().code,
            "invalid_argument"
        );
    }

    #[test]
    fn nul_records_preserve_unicode_spaces_newlines_and_flags() {
        let entries = parse(b"worktree /repo one\0HEAD abc\0branch refs/heads/main\0\0worktree /tmp/\xe4\xb8\xad\xe6\x96\x87\nline\0HEAD def\0detached\0locked reason with spaces\0prunable stale record\0\0").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].primary);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[1].path, "/tmp/中文\nline");
        assert!(!entries[1].primary);
        assert!(entries[1].detached);
        assert_eq!(entries[1].locked.as_deref(), Some("reason with spaces"));
        assert_eq!(entries[1].prunable.as_deref(), Some("stale record"));
    }
    #[test]
    fn invalid_porcelain_is_not_a_successful_empty_list() {
        for bytes in [
            b"".as_slice(),
            b"worktree \0\0",
            b"worktree /bad\xff\0",
            b"HEAD abc\0",
        ] {
            assert_eq!(parse(bytes).unwrap_err().code, "invalid_git_output");
        }
    }
}
