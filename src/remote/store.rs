//! User-owned connection metadata. Credentials remain in OpenSSH.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CATALOG: u64 = 1024 * 1024;
pub const MAX_MACHINES: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub id: String,
    pub label: String,
    pub destination: String,
    pub binary: String,
    pub socket: Option<String>,
    pub ssh_config: Option<PathBuf>,
    pub enabled: bool,
}

impl Machine {
    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.id) {
            bail!("invalid machine id: {}", self.id);
        }
        validate_label(&self.label)?;
        // OpenSSH receives this as exactly one destination argument after `--`.
        // Keep shell syntax, whitespace, URI options and leading flags out.
        if self.destination.is_empty()
            || self.destination.len() > 255
            || self.destination.starts_with('-')
            || !self.destination.bytes().all(|b| {
                b.is_ascii_alphanumeric()
                    || matches!(b, b'@' | b'.' | b'_' | b'-' | b':' | b'[' | b']')
            })
        {
            bail!(
                "SSH destination must be a host alias or user@host; configure ports in SSH config"
            );
        }
        validate_text(&self.binary, 2048, "remote binary")?;
        if self.binary.starts_with('-')
            || self.binary.starts_with('~')
            || (self.binary.contains('/') && !self.binary.starts_with('/'))
            || self.binary.contains('\\')
        {
            bail!("remote binary must be a command name or absolute POSIX path");
        }
        if let Some(socket) = &self.socket {
            validate_text(socket, 160, "remote socket")?;
            if socket.contains(['/', '\\']) {
                bail!("remote socket must be a socket name, not a path");
            }
        }
        if self.ssh_config.as_ref().is_some_and(|p| !p.is_absolute()) {
            bail!("SSH config path must be absolute");
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix('m')
        .is_some_and(|n| !n.starts_with('0') && n.parse::<u64>().is_ok_and(|n| n > 0))
}

pub fn validate_text(value: &str, max: usize, name: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        bail!("{name} must be non-empty, at most {max} bytes and contain no control characters");
    }
    Ok(())
}

pub fn validate_label(label: &str) -> Result<()> {
    validate_text(label, 160, "label")?;
    if label != label.trim() || label == "local" || valid_id(label) {
        bail!("label cannot have surrounding spaces or use the reserved local/mN names");
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    version: u32,
    next_id: u64,
    pub machines: Vec<Machine>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            machines: Vec::new(),
        }
    }
}

impl Catalog {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.machines.len() > MAX_MACHINES || self.next_id == 0 {
            bail!("unsupported or invalid machines catalog");
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut labels = std::collections::BTreeSet::new();
        for machine in &self.machines {
            machine.validate()?;
            if !ids.insert(&machine.id) || !labels.insert(&machine.label) {
                bail!("duplicate machine id or label in catalog");
            }
            let number = machine.id[1..].parse::<u64>()?;
            if number >= self.next_id {
                bail!("catalog next_id must exceed all saved machine ids");
            }
        }
        Ok(())
    }

    pub fn select(&self, selector: &str, enabled: bool) -> Result<&Machine> {
        let found = self
            .machines
            .iter()
            .find(|m| m.id == selector || m.label == selector)
            .with_context(|| format!("saved machine not found: {selector}"))?;
        if enabled && !found.enabled {
            bail!("machine {} is disabled", found.id);
        }
        Ok(found)
    }

    pub fn add(&mut self, mut machine: Machine) -> Result<Machine> {
        if self.machines.len() >= MAX_MACHINES {
            bail!("at most {MAX_MACHINES} saved machines are supported");
        }
        if self.machines.iter().any(|m| m.label == machine.label) {
            bail!("machine label already exists: {}", machine.label);
        }
        let next = self
            .next_id
            .checked_add(1)
            .context("machine id space exhausted")?;
        machine.id = format!("m{}", self.next_id);
        machine.validate()?;
        self.next_id = next;
        self.machines.push(machine.clone());
        Ok(machine)
    }
}

pub fn path() -> Result<PathBuf> {
    if let Some(path) = crate::brand::env_os("MACHINES") {
        if path.is_empty() {
            bail!("CREW_MACHINES cannot be empty");
        }
        return Ok(PathBuf::from(path));
    }
    // A custom config gets an adjacent catalog, keeping test and project configs isolated.
    let config =
        crate::config::config_path().context("cannot determine Crew configuration directory")?;
    Ok(config
        .parent()
        .unwrap_or(Path::new("."))
        .join("machines.json"))
}

pub fn load(path: &Path) -> Result<Catalog> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Catalog::default()),
        Err(e) => return Err(e).context("open machine catalog"),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CATALOG + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CATALOG {
        bail!("machine catalog exceeds 1 MiB");
    }
    let catalog: Catalog =
        serde_json::from_slice(&bytes).context("invalid machines.json; original file preserved")?;
    catalog.validate()?;
    Ok(catalog)
}

/// Serialize read/modify/write and replace the entire validated catalog atomically.
pub fn edit<T>(path: &Path, change: impl FnOnce(&mut Catalog) -> Result<T>) -> Result<T> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let lock_path = path.with_extension("json.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path)?;
    lock.lock()?;
    let mut catalog = load(path)?;
    let value = change(&mut catalog)?;
    catalog.validate()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = parent.join(format!(".crew-machines-{}-{stamp}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, &catalog)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    drop(lock);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        Machine {
            id: "m1".into(),
            label: "Build machine".into(),
            destination: "user@workbox".into(),
            binary: "/opt/Crew space/crew".into(),
            socket: None,
            ssh_config: None,
            enabled: true,
        }
    }

    #[test]
    fn selectors_and_ids_are_unambiguous_and_never_reused() {
        let mut catalog = Catalog::default();
        let first = catalog.add(machine()).unwrap();
        assert_eq!(
            catalog.select(&first.id, true).unwrap().label,
            "Build machine"
        );
        assert!(catalog.add(machine()).is_err());
        catalog.machines.clear();
        assert_eq!(catalog.add(machine()).unwrap().id, "m2");
        catalog.machines[0].enabled = false;
        assert!(catalog.select("m2", true).is_err());
        assert!(catalog.select("m2", false).is_ok());
    }

    #[test]
    fn reject_ssh_option_injection_and_invalid_catalog() {
        for destination in [
            "-oProxyCommand=touch",
            "host;id",
            "host\nother",
            "a b",
            "$(id)",
            "ssh://a",
        ] {
            let mut m = machine();
            m.destination = destination.into();
            assert!(m.validate().is_err());
        }
        for label in ["local", "m1", " ", "label\u{1b}", " x"] {
            assert!(validate_label(label).is_err());
        }
        let mut catalog = Catalog::default();
        catalog.add(machine()).unwrap();
        catalog.machines.push(catalog.machines[0].clone());
        assert!(catalog.validate().is_err());
    }
}
