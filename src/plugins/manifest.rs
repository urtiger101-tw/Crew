use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_MANIFEST: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub min_crew_version: String,
    #[serde(default)]
    pub description: String,
    pub platforms: Vec<String>,
    #[serde(default)]
    pub build: Vec<Build>,
    #[serde(default)]
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub command: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub title: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
}

pub fn identifier(value: &str, dotted: bool) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || (dotted && c == b'.'))
        && !value.ends_with('.')
}

fn version(value: &str) -> Result<[u64; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    ensure!(
        parts.len() == 3,
        "version must be MAJOR.MINOR.PATCH: {value}"
    );
    let mut result = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        ensure!(
            !part.is_empty()
                && part.bytes().all(|c| c.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0')),
            "version must contain canonical nonnegative integers: {value}"
        );
        result[index] = part.parse().context("version component is too large")?;
    }
    Ok(result)
}

fn text(value: &str, max: usize, field: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control),
        "{field} must be nonempty, at most {max} characters and contain no controls"
    );
    Ok(())
}

fn platforms(values: &[String], required: bool) -> Result<()> {
    ensure!(
        !required || !values.is_empty(),
        "platforms must be explicit"
    );
    ensure!(values.len() <= 3, "at most three platforms are supported");
    let mut seen = BTreeSet::new();
    for value in values {
        ensure!(
            matches!(value.as_str(), "linux" | "macos" | "windows"),
            "unknown platform: {value}"
        );
        ensure!(seen.insert(value), "duplicate platform: {value}");
    }
    Ok(())
}

fn command(args: &[String]) -> Result<()> {
    ensure!(
        !args.is_empty() && args.len() <= 128,
        "command needs 1..128 argv elements"
    );
    ensure!(!args[0].is_empty(), "command executable must not be empty");
    ensure!(
        args.iter().all(|s| s.len() <= 8192 && !s.contains('\0')),
        "command contains NUL or an oversized argument"
    );
    Ok(())
}

pub fn compatible(platforms: &[String], inherited: &[String]) -> bool {
    let list = if platforms.is_empty() {
        inherited
    } else {
        platforms
    };
    list.iter().any(|p| p == std::env::consts::OS)
}

impl Manifest {
    pub fn parse(raw: &str) -> Result<Self> {
        ensure!(raw.len() <= MAX_MANIFEST, "manifest exceeds 128 KiB");
        let manifest: Self = toml::from_str(raw).context("invalid crew-plugin.toml")?;
        ensure!(identifier(&manifest.id, true), "invalid portable plugin id");
        text(&manifest.name, 80, "name")?;
        if !manifest.description.is_empty() {
            text(&manifest.description, 512, "description")?;
        }
        version(&manifest.version)?;
        ensure!(
            version(&manifest.min_crew_version)? <= version(env!("CARGO_PKG_VERSION"))?,
            "plugin requires Crew {} or newer",
            manifest.min_crew_version
        );
        platforms(&manifest.platforms, true)?;
        ensure!(
            manifest.build.len() <= 16 && manifest.actions.len() <= 64,
            "manifest exceeds 16 build steps or 64 actions"
        );
        ensure!(
            !manifest.actions.is_empty(),
            "plugin must declare at least one action"
        );
        for build in &manifest.build {
            command(&build.command)?;
            platforms(&build.platforms, false)?;
        }
        let mut ids = BTreeSet::new();
        for action in &manifest.actions {
            ensure!(
                identifier(&action.id, false),
                "action ids cannot contain dots: {}",
                action.id
            );
            ensure!(ids.insert(&action.id), "duplicate action: {}", action.id);
            text(&action.title, 80, "action title")?;
            command(&action.command)?;
            platforms(&action.platforms, false)?;
        }
        Ok(manifest)
    }

    pub fn require_platform(&self) -> Result<()> {
        if !compatible(&[], &self.platforms) {
            bail!(
                "plugin {} does not support {}",
                self.id,
                std::env::consts::OS
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw() -> String {
        format!(
            "id='example.tools'\nname='Tools'\nversion='1.0.0'\nmin_crew_version='{}'\nplatforms=['linux','windows','macos']\n[[actions]]\nid='list'\ntitle='List'\ncommand=['{{crew}}','workspace','list']\n",
            env!("CARGO_PKG_VERSION")
        )
    }
    #[test]
    fn validates_manifest_and_rejects_future_version_and_unknown_capabilities() {
        assert!(Manifest::parse(&raw()).is_ok());
        assert!(Manifest::parse(&raw().replace(env!("CARGO_PKG_VERSION"), "999.0.0")).is_err());
        assert!(Manifest::parse(&(raw() + "unknown_capability=true\n")).is_err());
        assert!(Manifest::parse(&raw().replace("id='list'", "id='a.b'")).is_err());
        assert!(Manifest::parse(&raw().replace("example.tools", "../escape")).is_err());
        assert!(
            Manifest::parse(&raw().replace("['linux','windows','macos']", "['windows','windows']"))
                .is_err()
        );
    }
    #[test]
    fn argv_is_data_and_version_comparison_is_numeric() {
        assert!(command(&["python".into(), "$(touch no); quoted ' \"".into()]).is_ok());
        assert!(command(&["".into()]).is_err());
        assert!(command(&["echo".into(), "bad\0arg".into()]).is_err());
        assert!(version("1.10.0").unwrap() > version("1.9.0").unwrap());
        assert!(version("01.0.0").is_err());
        assert!(!identifier("a/b", true));
        assert!(!identifier("a:", true));
    }
}
