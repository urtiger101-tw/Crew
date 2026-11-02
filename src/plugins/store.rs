use super::manifest::{MAX_MANIFEST, Manifest};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    Bundled {
        id: String,
    },
    Github {
        shorthand: String,
        revision: Option<String>,
        commit: String,
    },
    Local {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub root: PathBuf,
    pub source: Source,
    pub enabled: bool,
    pub manifest_toml: String,
    pub installed_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub version: u32,
    pub plugins: BTreeMap<String, Entry>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: 1,
            plugins: BTreeMap::new(),
        }
    }
}

pub fn stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn home() -> Result<PathBuf> {
    if let Some(value) = crate::brand::env_os("PLUGIN_HOME") {
        ensure!(!value.is_empty(), "CREW_PLUGIN_HOME must not be empty");
        let path = PathBuf::from(value);
        return Ok(if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        });
    }
    Ok(dirs::data_local_dir()
        .context("could not locate user data directory")?
        .join("crew/plugins"))
}

pub fn read_limited(path: &Path, limit: usize) -> Result<String> {
    let file = File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "file exceeds {limit} bytes: {}",
        path.display()
    );
    String::from_utf8(bytes).context("file is not UTF-8")
}

pub fn manifest(root: &Path) -> Result<(String, Manifest)> {
    let raw = read_limited(&root.join("crew-plugin.toml"), MAX_MANIFEST)?;
    let parsed = Manifest::parse(&raw)?;
    Ok((raw, parsed))
}

pub fn read(root: &Path) -> Result<Registry> {
    let registry = read_entries(root)?;
    for (id, entry) in &registry.plugins {
        let parsed = Manifest::parse(&entry.manifest_toml)?;
        ensure!(parsed.id == *id, "plugin registry id mismatch");
    }
    Ok(registry)
}

/// Read the bounded registry without parsing individual stored manifests. Pane PATH
/// discovery uses this to report one damaged entry without disabling healthy plugins.
pub(super) fn read_entries(root: &Path) -> Result<Registry> {
    let file = root.join("registry.json");
    if !file.exists() {
        return Ok(Registry::default());
    }
    let registry: Registry = serde_json::from_str(&read_limited(&file, 8 * 1024 * 1024)?)
        .context("invalid plugin registry")?;
    ensure!(
        registry.version == 1 && registry.plugins.len() <= 128,
        "unsupported or oversized plugin registry"
    );
    Ok(registry)
}

pub fn lock(root: &Path) -> Result<File> {
    fs::create_dir_all(root)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("registry.lock"))?;
    file.try_lock()
        .context("another plugin change is in progress; try again after it completes")?;
    Ok(file)
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("file needs a parent directory")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".crew-write-{}-{}", std::process::id(), stamp()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    let result = (|| -> Result<()> {
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn write(root: &Path, registry: &Registry) -> Result<()> {
    ensure!(
        registry.plugins.len() <= 128,
        "at most 128 plugins may be registered"
    );
    let size = serde_json::to_vec_pretty(registry)?.len() + 1;
    ensure!(size <= 8 * 1024 * 1024, "plugin registry is full");
    atomic_json(&root.join("registry.json"), registry)
}

pub fn user_dirs(root: &Path, id: &str) -> Result<(PathBuf, PathBuf)> {
    ensure!(super::manifest::identifier(id, true), "invalid plugin id");
    let config = root.join("config").join(format!("p-{id}"));
    let state = root.join("state").join(format!("p-{id}"));
    fs::create_dir_all(&config)?;
    fs::create_dir_all(&state)?;
    Ok((config, state))
}

pub fn loaded(entry: &Entry) -> Result<Manifest> {
    let (raw, parsed) = manifest(&entry.root)?;
    if raw != entry.manifest_toml {
        bail!("plugin manifest changed since approval; inspect and link/install it again");
    }
    Ok(parsed)
}

pub fn stage(root: &Path) -> Result<PathBuf> {
    let installs = root.join("installs");
    fs::create_dir_all(&installs)?;
    for index in 0..100u32 {
        let path = installs.join(format!("{}-{}-{index}", stamp(), std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => {
                return fs::canonicalize(path).context("cannot resolve installation directory");
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("could not allocate a fresh plugin installation directory")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operating_system_lock_releases_when_handle_is_dropped() {
        let dir = std::env::temp_dir().join(format!(
            "crew-plugin-lock-{}-{}",
            std::process::id(),
            stamp()
        ));
        let held = lock(&dir).unwrap();
        assert!(lock(&dir).is_err());
        drop(held);
        assert!(lock(&dir).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }
}
