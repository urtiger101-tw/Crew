//! Crew naming and compatibility with tmuxlike installations.
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// New variables win, including an explicitly empty value. Legacy hooks keep working.
pub fn env_os(suffix: &str) -> Option<OsString> {
    let tail = if suffix.is_empty() {
        String::new()
    } else {
        format!("_{suffix}")
    };
    choose_env(
        std::env::var_os(format!("CREW{tail}")),
        std::env::var_os(format!("TMUXLIKE{tail}")),
    )
}

fn choose_env(new: Option<OsString>, old: Option<OsString>) -> Option<OsString> {
    new.or(old)
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
    fn new_environment_has_priority_and_old_hooks_are_accepted() {
        assert_eq!(
            choose_env(Some("new".into()), Some("old".into())),
            Some("new".into())
        );
        assert_eq!(choose_env(None, Some("old".into())), Some("old".into()));
        assert_eq!(
            choose_env(Some("".into()), Some("old".into())),
            Some("".into())
        );
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
