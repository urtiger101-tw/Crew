//! Crew naming. Only `CREW*` variables are read; the retired `tmuxlike` names are never read.
//! Config and data saved under the old `tmuxlike/` directory are still found (`compatible_file`).
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Variable name for `suffix`: `CREW` for an empty suffix, otherwise `CREW_<suffix>`.
fn env_name(suffix: &str) -> String {
    if suffix.is_empty() {
        "CREW".to_string()
    } else {
        format!("CREW_{suffix}")
    }
}

/// Value of `CREW_<suffix>` (or `CREW`). An explicitly empty value is returned as set.
pub fn env_os(suffix: &str) -> Option<OsString> {
    std::env::var_os(env_name(suffix))
}

/// The retired `TMUXLIKE` and `TMUXLIKE_*` names, matched case-insensitively. They are never
/// read, never exported to panes, and rejected as user `--env` keys.
pub fn is_legacy_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper == "TMUXLIKE" || upper.starts_with("TMUXLIKE_")
}

pub fn env_var(suffix: &str) -> Result<String, std::env::VarError> {
    env_os(suffix)
        .ok_or(std::env::VarError::NotPresent)?
        .into_string()
        .map_err(std::env::VarError::NotUnicode)
}

/// Keep using an existing legacy file until the user explicitly migrates it. This avoids
/// restoring a stale legacy snapshot after a newly named snapshot has been cleared.
pub fn compatible_file(base: &Path, file: &str) -> PathBuf {
    let new = base.join("crew").join(file);
    let old = base.join("tmuxlike").join(file);
    if new.exists() || !old.exists() {
        new
    } else {
        old
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn env_names_are_crew_only() {
        assert_eq!(env_name(""), "CREW");
        assert_eq!(env_name("PANE_ID"), "CREW_PANE_ID");
        assert_eq!(env_name("SOCKET"), "CREW_SOCKET");
        assert_eq!(env_name("STATE"), "CREW_STATE");
        for suffix in ["", "PANE_ID", "SOCKET", "STATE", "LOG", "LANG", "CONFIG"] {
            assert!(!env_name(suffix).starts_with("TMUXLIKE"), "{suffix}");
        }
    }

    #[test]
    fn legacy_names_match_case_insensitively_and_nothing_else() {
        for name in ["TMUXLIKE", "TMUXLIKE_PANE_ID", "tmuxlike_socket"] {
            assert!(is_legacy_env_name(name), "{name}");
        }
        for name in ["CREW", "CREW_PANE_ID", "TMUXLIKEX", "PATH"] {
            assert!(!is_legacy_env_name(name), "{name}");
        }
    }
    #[test]
    fn legacy_file_is_selected_without_copying_or_deleting_it() {
        let root = std::env::temp_dir().join(format!("crew-migration-{}", std::process::id()));
        std::fs::create_dir_all(root.join("tmuxlike")).unwrap();
        std::fs::write(root.join("tmuxlike/config.toml"), "lang = 'zh-TW'").unwrap();
        assert_eq!(
            compatible_file(&root, "config.toml"),
            root.join("tmuxlike/config.toml")
        );
        std::fs::create_dir_all(root.join("crew")).unwrap();
        std::fs::write(root.join("crew/config.toml"), "lang = 'en'").unwrap();
        assert_eq!(
            compatible_file(&root, "config.toml"),
            root.join("crew/config.toml")
        );
        assert!(root.join("tmuxlike/config.toml").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
