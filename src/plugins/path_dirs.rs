use super::{manifest, store};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct PanePluginPaths {
    pub dirs: Vec<PathBuf>,
    pub diagnostics: Vec<String>,
}

/// Resolve only directories that were part of the approved manifest and are still inside
/// the plugin root after following symlinks. Return paths suitable for a child process.
pub(super) fn resolve(entry: &store::Entry, manifest: &manifest::Manifest) -> Result<Vec<PathBuf>> {
    if manifest.path_dirs.is_empty() {
        return Ok(Vec::new());
    }
    let root = entry
        .root
        .canonicalize()
        .context("plugin root is unavailable")?;
    let mut result = Vec::new();
    for dir in &manifest.path_dirs {
        if !manifest::compatible(&dir.platforms, &manifest.platforms) {
            continue;
        }
        let path = root
            .join(&dir.path)
            .canonicalize()
            .with_context(|| format!("plugin PATH directory is unavailable: {}", dir.path))?;
        ensure!(
            path.starts_with(&root),
            "plugin PATH directory escapes its root: {}",
            dir.path
        );
        ensure!(
            path.is_dir(),
            "plugin PATH entry is not a directory: {}",
            dir.path
        );
        let path = crate::process_path::for_child(&path)
            .map_err(|error| anyhow::anyhow!("{}", error.message))?;
        std::env::join_paths([&path]).context("plugin PATH directory cannot be used in PATH")?;
        result.push(path);
    }
    Ok(result)
}

pub(super) fn collect(home: &Path) -> Result<PanePluginPaths> {
    let mut result = PanePluginPaths::default();
    for (id, entry) in store::read_entries(home)?.plugins {
        if !entry.enabled {
            continue;
        }
        let stored = match manifest::Manifest::parse(&entry.manifest_toml) {
            Ok(stored) if stored.id == id => stored,
            Ok(_) => {
                result
                    .diagnostics
                    .push(format!("{id}: plugin registry id mismatch"));
                continue;
            }
            Err(error) => {
                result.diagnostics.push(format!("{id}: {error:#}"));
                continue;
            }
        };
        if stored.path_dirs.is_empty() || !manifest::compatible(&[], &stored.platforms) {
            continue;
        }
        let paths = store::loaded(&entry).and_then(|manifest| resolve(&entry, &manifest));
        match paths {
            Ok(paths) => {
                for path in paths {
                    if !result.dirs.contains(&path) {
                        result.dirs.push(path);
                    }
                }
            }
            Err(error) => result.diagnostics.push(format!("{id}: {error:#}")),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn enabled_unchanged_plugins_contribute_ordered_confined_directories() {
        let temp = std::env::temp_dir();
        let dir = temp.join(format!(
            "crew-path-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = dir.join("home");
        let mut registry = store::Registry::default();
        for (id, enabled) in [
            ("z.enabled", true),
            ("a.enabled", true),
            ("b.disabled", false),
            ("c.incompatible", true),
        ] {
            let root = dir.join(id);
            fs::create_dir_all(root.join("bin")).unwrap();
            let supported = if id == "c.incompatible" {
                if cfg!(windows) {
                    "'linux'"
                } else {
                    "'windows'"
                }
            } else {
                "'linux','macos','windows'"
            };
            let raw = format!(
                "id='{id}'\nname='Test'\nversion='1.0.0'\nmin_crew_version='{}'\nplatforms=[{supported}]\n[[path_dirs]]\npath='bin'\n",
                env!("CARGO_PKG_VERSION")
            );
            fs::write(root.join("crew-plugin.toml"), &raw).unwrap();
            registry.plugins.insert(
                id.into(),
                store::Entry {
                    root: root.clone(),
                    source: store::Source::Local { path: root },
                    enabled,
                    manifest_toml: raw,
                    installed_at: 0,
                },
            );
        }
        store::write(&home, &registry).unwrap();
        let paths = collect(&home).unwrap();
        assert!(paths.diagnostics.is_empty());
        let expected = ["a.enabled", "z.enabled"].map(|id| {
            crate::process_path::for_child(&dir.join(id).join("bin").canonicalize().unwrap())
                .unwrap()
        });
        assert_eq!(paths.dirs, expected);
        let entries = super::super::entries(&home).unwrap();
        let incompatible = entries
            .iter()
            .find(|entry| entry["id"] == "c.incompatible")
            .unwrap();
        assert_eq!(incompatible["platform_compatible"], false);
        assert_eq!(incompatible["effective_path_dirs"], serde_json::json!([]));
        let disabled = entries
            .iter()
            .find(|entry| entry["id"] == "b.disabled")
            .unwrap();
        assert_eq!(disabled["effective_path_dirs"].as_array().unwrap().len(), 0);
        let enabled = entries
            .iter()
            .find(|entry| entry["id"] == "a.enabled")
            .unwrap();
        assert_eq!(enabled["effective_path_dirs"].as_array().unwrap().len(), 1);

        let mut corrupt = registry.plugins["a.enabled"].clone();
        corrupt.manifest_toml = "not a valid manifest".into();
        registry.plugins.insert("bad.corrupt".into(), corrupt);
        registry
            .plugins
            .insert("bad.mismatch".into(), registry.plugins["z.enabled"].clone());
        store::write(&home, &registry).unwrap();
        let paths = collect(&home).unwrap();
        assert_eq!(paths.dirs, expected);
        assert_eq!(paths.diagnostics.len(), 2);
        assert!(paths.diagnostics[0].contains("bad.corrupt"));
        assert!(paths.diagnostics[1].contains("id mismatch"));
        registry.plugins.remove("bad.corrupt");
        registry.plugins.remove("bad.mismatch");
        store::write(&home, &registry).unwrap();

        fs::write(
            dir.join("a.enabled/crew-plugin.toml"),
            format!(
                "{}\n# changed\n",
                registry.plugins["a.enabled"].manifest_toml
            ),
        )
        .unwrap();
        fs::remove_dir(dir.join("z.enabled/bin")).unwrap();
        let paths = collect(&home).unwrap();
        assert!(paths.dirs.is_empty());
        assert_eq!(paths.diagnostics.len(), 2);
        assert!(paths.diagnostics[0].contains("manifest changed"));
        assert!(paths.diagnostics[1].contains("directory is unavailable"));
        assert!(
            dir.canonicalize()
                .unwrap()
                .starts_with(temp.canonicalize().unwrap())
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_path_directory_symlink_outside_plugin_root_when_supported() {
        let temp = std::env::temp_dir();
        let dir = temp.join(format!(
            "crew-path-link-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = dir.join("plugin");
        let outside = dir.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&outside, root.join("bin"));
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, root.join("bin"));
        #[cfg(not(any(windows, unix)))]
        let linked: std::io::Result<()> = Err(std::io::ErrorKind::Unsupported.into());
        if linked.is_ok() {
            let raw = format!(
                "id='example.bin'\nname='Test'\nversion='1.0.0'\nmin_crew_version='{}'\nplatforms=['linux','macos','windows']\n[[path_dirs]]\npath='bin'\n",
                env!("CARGO_PKG_VERSION")
            );
            let entry = store::Entry {
                root: root.clone(),
                source: store::Source::Local { path: root },
                enabled: true,
                manifest_toml: raw.clone(),
                installed_at: 0,
            };
            let error = resolve(&entry, &manifest::Manifest::parse(&raw).unwrap()).unwrap_err();
            assert!(error.to_string().contains("escapes its root"), "{error}");
        } else {
            eprintln!("directory symlinks unavailable on this system; escape test not exercised");
        }
        assert!(
            dir.canonicalize()
                .unwrap()
                .starts_with(temp.canonicalize().unwrap())
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
