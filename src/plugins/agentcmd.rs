use super::{manifest, process, store};
use anyhow::{Context, Result, anyhow, ensure};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const REVISION: &str = "617a90c73554ccb5a6c6eebc631e26c78948927a";
const UPSTREAM_URL: &str = "https://github.com/stevenke1981/linux-bash-command.git";
const MARKER: &str = "agentcmd-install.json";

struct InstallStage {
    root: PathBuf,
    installs: PathBuf,
    created_dirs: Vec<PathBuf>,
    registered: bool,
}

impl InstallStage {
    fn new(home: &Path) -> Result<Self> {
        let root = store::stage(home)?;
        let installs = home.join("installs").canonicalize()?;
        ensure!(
            root.parent() == Some(installs.as_path()),
            "invalid installation stage"
        );
        Ok(Self {
            root,
            installs,
            created_dirs: Vec::new(),
            registered: false,
        })
    }

    fn user_dirs(&mut self, home: &Path, id: &str) -> Result<()> {
        for parent in ["config", "state"] {
            let parent = home.join(parent);
            fs::create_dir_all(&parent)?;
            let path = parent.join(format!("p-{id}"));
            match fs::create_dir(&path) {
                Ok(()) => self.created_dirs.push(path),
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

impl Drop for InstallStage {
    fn drop(&mut self) {
        if self.registered {
            return;
        }
        for path in self.created_dirs.iter().rev() {
            if let Err(error) = fs::remove_dir(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!(
                    "could not remove new plugin directory {}: {error}",
                    path.display()
                );
            }
        }
        if self.root.parent() == Some(self.installs.as_path())
            && self.root.canonicalize().ok().as_deref() == Some(self.root.as_path())
            && let Err(error) = fs::remove_dir_all(&self.root)
        {
            eprintln!(
                "could not remove failed agentcmd stage {}: {error}",
                self.root.display()
            );
        }
    }
}

/// Remove only named temporary artifacts contained directly in a verified stage.
fn remove_build_artifacts(stage: &Path) -> Result<()> {
    for name in [
        "source",
        "source.tar",
        "source.git",
        "target",
        "gitconfig",
        "empty-template",
    ] {
        let path = stage.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        ensure!(
            !metadata.file_type().is_symlink(),
            "refusing symlinked build artifact: {}",
            path.display()
        );
        let resolved = path.canonicalize()?;
        ensure!(
            resolved.parent() == Some(stage),
            "build artifact escapes installation stage"
        );
        if metadata.is_dir() {
            fs::remove_dir_all(&resolved)?;
        } else {
            fs::remove_file(&resolved)?;
        }
    }
    Ok(())
}

fn clean_old_build_artifacts(home: &Path, current: &Path) -> Vec<String> {
    let installs = home.join("installs");
    let Ok(root) = installs.canonicalize() else {
        return Vec::new();
    };
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) => return vec![format!("cannot inspect old agentcmd stages: {error}")],
    };
    let mut warnings = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("cannot inspect old agentcmd stage: {error}"));
                continue;
            }
        };
        let path = entry.path();
        let Ok(stage) = path.canonicalize() else {
            continue;
        };
        if stage == current || stage.parent() != Some(root.as_path()) {
            continue;
        }
        let Ok(raw) = store::read_limited(&stage.join(MARKER), 64 * 1024) else {
            continue;
        };
        let Ok(marker) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if marker["kind"] == "crew-managed-agentcmd"
            && let Err(error) = remove_build_artifacts(&stage)
        {
            warnings.push(format!(
                "cannot clean old stage {}: {error:#}",
                stage.display()
            ));
        }
    }
    warnings
}

#[derive(Debug, Serialize)]
struct Utility {
    name: String,
    id: String,
    summary: String,
    conflict: Option<String>,
}

fn parse_registry(source: &str) -> Result<Vec<Utility>> {
    let section = source
        .split_once("pub static UTILITIES: &[Utility] = &[")
        .context("agentcmd registry does not contain the expected utilities array")?
        .1
        .split_once("];")
        .context("agentcmd registry utilities array is incomplete")?
        .0;
    let pattern = Regex::new(
        r#"(?s)Utility\s*\{\s*name:\s*("[^"\\]*"),\s*run:\s*[A-Za-z0-9_:]+,\s*summary:\s*("[^"\\]*"),\s*origin:\s*Origin::[A-Za-z0-9_]+,\s*windows_conflict:\s*(?:None|Some\(("[^"\\]*")\)),\s*\}"#,
    )?;
    let mut utilities = Vec::new();
    let mut ids = BTreeSet::new();
    let mut position = 0;
    for captures in pattern.captures_iter(section) {
        let matched = captures.get(0).unwrap();
        ensure!(
            section[position..matched.start()]
                .chars()
                .all(|c| c.is_whitespace() || c == ','),
            "agentcmd registry contains an unrecognized utility entry"
        );
        position = matched.end();
        let name: String = serde_json::from_str(&captures[1])?;
        let summary: String = serde_json::from_str(&captures[2])?;
        let conflict: Option<String> = captures
            .get(3)
            .map(|value| serde_json::from_str(value.as_str()))
            .transpose()?;
        ensure!(
            name == "[" || manifest::identifier(&name, false),
            "unsafe agentcmd executable name: {name}"
        );
        let id = format!("agentcmd.{}", if name == "[" { "bracket" } else { &name });
        ensure!(
            ids.insert(id.to_ascii_lowercase()),
            "duplicate agentcmd module ID: {id}"
        );
        utilities.push(Utility {
            name,
            id,
            summary,
            conflict,
        });
    }
    ensure!(
        !utilities.is_empty()
            && section[position..]
                .chars()
                .all(|c| c.is_whitespace() || c == ','),
        "agentcmd registry contains an unrecognized utility entry"
    );
    Ok(utilities)
}

fn validate_list(utilities: &[Utility], output: &str) -> Result<()> {
    let expected: BTreeSet<_> = utilities
        .iter()
        .map(|utility| utility.name.as_str())
        .collect();
    let listed: Vec<_> = output.lines().collect();
    let actual: BTreeSet<_> = listed.iter().copied().collect();
    ensure!(
        listed.len() == actual.len() && expected == actual,
        "agentcmd --list does not match its source registry (missing: {:?}; unexpected: {:?}; duplicate entries: {})",
        expected.difference(&actual).collect::<Vec<_>>(),
        actual.difference(&expected).collect::<Vec<_>>(),
        listed.len().saturating_sub(actual.len())
    );
    Ok(())
}

fn executable_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn child_path(path: &Path) -> Result<PathBuf> {
    crate::process_path::for_child(path).map_err(|error| anyhow!(error.message))
}

fn managed(home: &Path, id: &str, entry: &store::Entry) -> bool {
    let store::Source::Local { path } = &entry.source else {
        return false;
    };
    if path != &entry.root || entry.root.file_name().is_none_or(|name| name != id) {
        return false;
    }
    let Some(modules) = entry.root.parent() else {
        return false;
    };
    let Some(stage) = modules.parent() else {
        return false;
    };
    if modules.file_name().is_none_or(|name| name != "modules")
        || home.join("installs").canonicalize().ok().as_deref() != stage.parent()
    {
        return false;
    }
    let Ok(raw) = store::read_limited(&stage.join(MARKER), 64 * 1024) else {
        return false;
    };
    let Ok(marker) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    marker["kind"] == "crew-managed-agentcmd"
        && marker["plugins"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item == id))
}

fn preflight(
    home: &Path,
    registry: &store::Registry,
    utilities: &[Utility],
) -> Result<BTreeMap<String, bool>> {
    let mut enabled = BTreeMap::new();
    for utility in utilities {
        if let Some((key, entry)) = registry
            .plugins
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&utility.id))
        {
            ensure!(
                key == &utility.id && managed(home, key, entry),
                "agentcmd module {} conflicts with an unrelated installed plugin; uninstall or rename that plugin first",
                utility.id
            );
            store::loaded(entry).with_context(|| {
                format!(
                    "previous {} manifest changed; inspect and uninstall it before reinstalling",
                    utility.id
                )
            })?;
            enabled.insert(utility.id.clone(), entry.enabled);
        }
    }
    ensure!(
        registry.plugins.len() + utilities.len() - enabled.len() <= 128,
        "plugin registry is full; agentcmd needs {} additional slots",
        utilities.len() - enabled.len()
    );
    Ok(enabled)
}

fn register_modules(
    home: &Path,
    stage: &mut InstallStage,
    binary: &Path,
    version: &str,
    utilities: &[Utility],
    registry: &mut store::Registry,
) -> Result<(Vec<Value>, Vec<String>)> {
    let enabled = preflight(home, registry, utilities)?;
    let root_stage = stage.root.clone();
    let shared = root_stage.join("shared");
    fs::create_dir_all(&shared)?;
    let shared_binary = shared.join(executable_name("agentcmd"));
    fs::copy(binary, &shared_binary).context("cannot store the shared agentcmd binary")?;
    store::atomic_json(
        &root_stage.join(MARKER),
        &json!({
            "kind":"crew-managed-agentcmd", "revision":REVISION,
            "plugins":utilities.iter().map(|utility| &utility.id).collect::<Vec<_>>()
        }),
    )?;
    let mut entries = Vec::new();
    let mut result = Vec::new();
    for utility in utilities {
        let root = root_stage.join("modules").join(&utility.id);
        let bin = root.join("bin");
        fs::create_dir_all(&bin)?;
        let target = bin.join(executable_name(&utility.name));
        if fs::hard_link(&shared_binary, &target).is_err() {
            fs::copy(&shared_binary, &target)
                .with_context(|| format!("cannot create executable for {}", utility.id))?;
        }
        let mut description = format!("{} (agentcmd commit {REVISION})", utility.summary);
        if let Some(conflict) = &utility.conflict {
            description.push_str(&format!("; Windows conflict: {conflict}"));
        }
        let plugin = manifest::Manifest {
            id: utility.id.clone(),
            name: format!("agentcmd: {}", utility.name),
            version: version.into(),
            min_crew_version: env!("CARGO_PKG_VERSION").into(),
            description,
            platforms: vec!["linux".into(), "macos".into(), "windows".into()],
            build: Vec::new(),
            actions: Vec::new(),
            path_dirs: vec![manifest::PathDir {
                path: "bin".into(),
                platforms: Vec::new(),
            }],
        };
        let raw = toml::to_string(&plugin)?;
        manifest::Manifest::parse(&raw)?;
        fs::write(root.join("crew-plugin.toml"), &raw)?;
        stage.user_dirs(home, &utility.id)?;
        let is_enabled = enabled.get(&utility.id).copied().unwrap_or(false);
        entries.push((
            utility.id.clone(),
            store::Entry {
                source: store::Source::Local { path: root.clone() },
                root: root.clone(),
                enabled: is_enabled,
                manifest_toml: raw,
                installed_at: store::stamp(),
            },
        ));
        result.push(json!({"id":utility.id,"command":utility.name,"enabled":is_enabled,"windows_conflict":utility.conflict,"root":root}));
    }
    let mut next = store::Registry {
        version: registry.version,
        plugins: registry.plugins.clone(),
    };
    next.plugins.extend(entries);
    let license = root_stage.join("source").join("LICENSE");
    if license.is_file() {
        fs::copy(license, root_stage.join("LICENSE"))?;
    }
    store::write(home, &next)?;
    *registry = next;
    stage.registered = true;
    let cleanup_warnings = remove_build_artifacts(&root_stage)
        .err()
        .map(|error| format!("cannot clean new stage {}: {error:#}", root_stage.display()))
        .into_iter()
        .collect();
    Ok((result, cleanup_warnings))
}

fn git(root: &Path, config: &Path, args: &[OsString]) -> Result<String> {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", child_path(config)?)
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .arg("-C")
        .arg(child_path(root)?)
        .args([
            "-c",
            "core.hooksPath=",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "core.autocrlf=false",
            "-c",
            "credential.helper=",
        ]);
    command.args(args);
    let output = process::capture(command, Duration::from_secs(120), 128 * 1024)
        .context("Git is required to obtain the pinned agentcmd source")?;
    ensure!(
        output.success() && !output.truncated,
        "could not obtain pinned agentcmd source using Git: {}",
        serde_json::to_string(&output)?
    );
    Ok(output.stdout.trim().to_owned())
}

pub fn install(home: &Path, source: Option<&Path>, offline: bool, yes: bool) -> Result<Value> {
    ensure!(
        !offline || source.is_some(),
        "--offline requires --source PATH to a local Git repository"
    );
    let cargo_version = Command::new("cargo")
        .arg("--version")
        .output()
        .context("cargo is required to build agentcmd; install Rust and Cargo and try again")?;
    ensure!(
        cargo_version.status.success(),
        "cargo --version failed: {}",
        String::from_utf8_lossy(&cargo_version.stderr)
    );
    let local_source = source
        .map(|path| {
            path.canonicalize()
                .context("--source local Git repository does not exist")
        })
        .transpose()?;
    if let Some(path) = &local_source {
        ensure!(
            path.is_dir(),
            "--source must name a local Git repository directory"
        );
    }
    let _lock = store::lock(home)?;
    let mut registry = store::read(home)?;
    let mut stage = InstallStage::new(home)?;
    let stage_path = stage.root.clone();
    let config = stage_path.join("gitconfig");
    fs::write(&config, "")?;
    let template = stage_path.join("empty-template");
    fs::create_dir(&template)?;
    let repository = if let Some(path) = &local_source {
        path.clone()
    } else {
        let repository = stage_path.join("source.git");
        fs::create_dir(&repository)?;
        git(
            &repository,
            &config,
            &[
                "init".into(),
                "--bare".into(),
                "--quiet".into(),
                "--template".into(),
                child_path(&template)?.into_os_string(),
            ],
        )?;
        git(
            &repository,
            &config,
            &[
                "fetch".into(),
                "--quiet".into(),
                "--depth=1".into(),
                "--no-tags".into(),
                UPSTREAM_URL.into(),
                REVISION.into(),
            ],
        )?;
        repository
    };
    let commit = git(
        &repository,
        &config,
        &[
            "rev-parse".into(),
            "--verify".into(),
            format!("{REVISION}^{{commit}}").into(),
        ],
    )?;
    ensure!(
        commit == REVISION,
        "agentcmd source revision mismatch: expected {REVISION}, got {commit}"
    );
    let object_type = git(
        &repository,
        &config,
        &["cat-file".into(), "-t".into(), REVISION.into()],
    )?;
    ensure!(object_type == "commit", "agentcmd revision is not a commit");
    let archive = stage_path.join("source.tar");
    git(
        &repository,
        &config,
        &[
            "archive".into(),
            "--format=tar".into(),
            "--output".into(),
            child_path(&archive)?.into_os_string(),
            REVISION.into(),
        ],
    )?;
    let checkout = stage_path.join("source");
    fs::create_dir(&checkout)?;
    #[cfg(windows)]
    let tar = PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot is not set")?)
        .join("System32")
        .join("tar.exe");
    #[cfg(not(windows))]
    let tar = PathBuf::from("tar");
    let mut extract = Command::new(tar);
    extract
        .arg("-xf")
        .arg(child_path(&archive)?)
        .arg("-C")
        .arg(child_path(&checkout)?);
    let extracted = process::capture(extract, Duration::from_secs(120), 128 * 1024)
        .context("tar is required to extract the pinned agentcmd source")?;
    ensure!(
        extracted.success() && !extracted.truncated,
        "could not extract pinned agentcmd source: {}",
        serde_json::to_string(&extracted)?
    );
    let cargo_toml: toml::Value = toml::from_str(&store::read_limited(
        &checkout.join("Cargo.toml"),
        128 * 1024,
    )?)?;
    let package = cargo_toml
        .get("package")
        .context("agentcmd Cargo.toml has no package")?;
    ensure!(
        package.get("name").and_then(toml::Value::as_str) == Some("agentcmd"),
        "source package is not agentcmd"
    );
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .context("agentcmd package has no version")?;
    let utilities = parse_registry(&store::read_limited(
        &checkout.join("src/registry.rs"),
        1024 * 1024,
    )?)?;
    let enabled = preflight(home, &registry, &utilities)?;
    let target = stage_path.join("target");
    let rustc = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .current_dir(child_path(&checkout)?)
        .arg("-vV")
        .output()
        .context("rustc is required to determine the agentcmd build target")?;
    ensure!(
        rustc.status.success(),
        "rustc -vV failed: {}",
        String::from_utf8_lossy(&rustc.stderr)
    );
    let rustc_output = std::str::from_utf8(&rustc.stdout).context("rustc output is not UTF-8")?;
    let host = rustc_output
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .context("rustc -vV did not report a host target")?;
    ensure!(
        !host.is_empty()
            && host
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.')),
        "rustc reported an invalid host target"
    );
    let mut build_arguments = vec![
        "cargo".to_owned(),
        "build".to_owned(),
        "--locked".to_owned(),
        "--release".to_owned(),
        "--bin".to_owned(),
        "agentcmd".to_owned(),
        "--target".to_owned(),
        host.to_owned(),
        "--target-dir".to_owned(),
        child_path(&target)?.to_string_lossy().into_owned(),
    ];
    if offline {
        build_arguments.push("--offline".into());
    }
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "source":UPSTREAM_URL,"revision":REVISION,"local_repository":local_source,
            "build_command":build_arguments,"cargo_offline":offline,
            "utilities":utilities,"module_path_dirs":["bin"],
            "executables":"One command-specific hard link or copy per module; shared binary directory is not on PATH.",
            "new_modules_enabled":false,"existing_enabled_states_preserved":enabled,
            "trust":"The build executes source code with your user permissions; Crew does not sandbox it."
        }))?
    );
    if !yes {
        ensure!(
            std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
            "approval required: review the agentcmd source and build, then repeat with --yes"
        );
        eprint!("Trust this pinned agentcmd source and run its build? [y/N] ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        ensure!(
            matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"),
            "agentcmd installation cancelled"
        );
    }
    let mut build = Command::new("cargo");
    build
        .current_dir(child_path(&checkout)?)
        .args(["build", "--locked", "--release", "--bin", "agentcmd"])
        .arg("--target")
        .arg(host)
        .arg("--target-dir")
        .arg(child_path(&target)?)
        .env_remove("CARGO_BUILD_TARGET");
    if offline {
        build.arg("--offline");
    }
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if name == "CREW" || name.starts_with("CREW_") {
            build.env_remove(key);
        }
    }
    eprintln!("Building pinned agentcmd once; this may take several minutes.");
    let output = process::capture(build, Duration::from_secs(1800), 1024 * 1024)
        .context("could not run cargo build for agentcmd")?;
    let log = home.join("build-logs").join(format!(
        "agentcmd-{}-{}.json",
        store::stamp(),
        std::process::id()
    ));
    store::atomic_json(
        &log,
        &json!({"source":UPSTREAM_URL,"revision":REVISION,"offline":offline,"output":output}),
    )?;
    ensure!(
        output.success(),
        "agentcmd cargo build failed; evidence: {}",
        log.display()
    );
    let binary = target
        .join(host)
        .join("release")
        .join(executable_name("agentcmd"));
    let mut list_command = Command::new(child_path(&binary)?);
    list_command.arg("--list");
    let listed = process::capture(list_command, Duration::from_secs(30), 64 * 1024)
        .context("could not run built agentcmd --list")?;
    ensure!(
        listed.success() && !listed.truncated,
        "agentcmd --list failed: {}",
        serde_json::to_string(&listed)?
    );
    validate_list(&utilities, &listed.stdout)?;
    let (modules, mut cleanup_warnings) = register_modules(
        home,
        &mut stage,
        &binary,
        version,
        &utilities,
        &mut registry,
    )?;
    cleanup_warnings.extend(clean_old_build_artifacts(home, &stage_path));
    Ok(
        json!({"revision":REVISION,"source":UPSTREAM_URL,"local_repository":local_source,"offline":offline,
        "shared_binary":stage_path.join("shared").join(executable_name("agentcmd")),"build_log":log,
        "modules":modules,"cleanup_warnings":cleanup_warnings,"reinstall":"crew plugin agentcmd install","generic_plugin_update":"skipped_local_link"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    // https://github.com/stevenke1981/linux-bash-command/blob/
    // 617a90c73554ccb5a6c6eebc631e26c78948927a/src/registry.rs
    // The upstream MIT license is in tests/fixtures/agentcmd-LICENSE.txt.
    const UPSTREAM: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/agentcmd_registry_617a90c.rs"
    ));
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "crew-agentcmd-installer-{}-{}-{}",
                std::process::id(),
                store::stamp(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn stage(&self) -> (InstallStage, PathBuf) {
            let stage = InstallStage::new(&self.0).unwrap();
            let binary = stage.root.join("fixture-agentcmd");
            fs::write(&binary, b"agentcmd binary").unwrap();
            (stage, binary)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn pinned_registry_covers_names_conflicts_and_bracket() {
        let utilities = parse_registry(UPSTREAM).unwrap();
        assert_eq!(utilities.len(), 46);
        assert_eq!(
            utilities.iter().filter(|u| u.conflict.is_some()).count(),
            13
        );
        let bracket = utilities.iter().find(|u| u.name == "[").unwrap();
        assert_eq!(bracket.id, "agentcmd.bracket");
        assert_eq!(
            utilities
                .iter()
                .find(|u| u.name == "sort")
                .unwrap()
                .conflict
                .as_deref(),
            Some("PowerShell, Windows System32")
        );
        let listed = utilities
            .iter()
            .map(|u| u.name.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        validate_list(&utilities, &listed).unwrap();
        assert!(validate_list(&utilities, "wc\n").is_err());
        assert!(validate_list(&utilities, &format!("{listed}\nwc\n")).is_err());
        assert!(validate_list(&utilities, &format!("{listed}\nunregistered\n")).is_err());
    }

    #[test]
    fn registry_rejects_unsafe_paths_duplicates_and_unparsed_entries() {
        for source in [
            UPSTREAM.replace("name: \"wc\"", "name: \"../wc\""),
            UPSTREAM.replace("name: \"wc\"", "name: \"cat\""),
            UPSTREAM.replace("name: \"wc\"", "name: \"bracket\""),
            UPSTREAM.replace(
                "summary: \"Count input lines, words and bytes\"",
                "summary: missing",
            ),
        ] {
            assert!(parse_registry(&source).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn child_process_paths_strip_windows_verbatim_prefix() {
        let path = Path::new(r"\\?\D:\agentcmd test\gitconfig");
        assert_eq!(
            child_path(path).unwrap(),
            Path::new(r"D:\agentcmd test\gitconfig")
        );
    }

    #[test]
    fn registers_isolated_disabled_modules_and_preserves_managed_state() {
        let fixture = Fixture::new();
        let utilities = parse_registry(UPSTREAM).unwrap();
        let (mut stage, binary) = fixture.stage();
        let mut registry = store::Registry::default();
        register_modules(
            &fixture.0,
            &mut stage,
            &binary,
            "0.1.0",
            &utilities,
            &mut registry,
        )
        .unwrap();
        assert_eq!(registry.plugins.len(), 46);
        for utility in &utilities {
            let entry = &registry.plugins[&utility.id];
            assert!(!entry.enabled);
            let parsed = store::loaded(entry).unwrap();
            assert_eq!(parsed.path_dirs[0].path, "bin");
            assert_eq!(fs::read_dir(entry.root.join("bin")).unwrap().count(), 1);
            assert_eq!(
                fs::read(entry.root.join("bin").join(executable_name(&utility.name))).unwrap(),
                b"agentcmd binary"
            );
            assert!(!entry.root.starts_with(stage.root.join("shared")));
            if let Some(conflict) = &utility.conflict {
                assert!(parsed.description.contains(conflict));
            }
        }
        registry.plugins.get_mut("agentcmd.wc").unwrap().enabled = true;
        store::write(&fixture.0, &registry).unwrap();
        registry = store::read(&fixture.0).unwrap();
        fs::create_dir_all(stage.root.join("source")).unwrap();
        fs::create_dir_all(stage.root.join("target")).unwrap();
        let old_root = stage.root.clone();
        let (mut next_stage, next_binary) = fixture.stage();
        register_modules(
            &fixture.0,
            &mut next_stage,
            &next_binary,
            "0.1.0",
            &utilities,
            &mut registry,
        )
        .unwrap();
        assert!(registry.plugins["agentcmd.wc"].enabled);
        assert!(!registry.plugins["agentcmd.sort"].enabled);
        assert!(clean_old_build_artifacts(&fixture.0, &next_stage.root).is_empty());
        assert!(!old_root.join("source").exists());
        assert!(!old_root.join("target").exists());
        assert!(old_root.join("shared").is_dir());
        assert!(old_root.join("modules/agentcmd.wc/bin").is_dir());
    }

    #[test]
    fn build_artifact_cleanup_failure_does_not_abort_registration() {
        let fixture = Fixture::new();
        let utilities = parse_registry(UPSTREAM).unwrap();
        let (mut stage, binary) = fixture.stage();
        let blocked = stage.root.join("source.tar");
        #[cfg(windows)]
        let locked = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::write(&blocked, b"temporary archive").unwrap();
            fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&blocked)
                .unwrap()
        };
        #[cfg(unix)]
        std::os::unix::fs::symlink("unrelated", &blocked).unwrap();
        let mut registry = store::Registry::default();
        let result = register_modules(
            &fixture.0,
            &mut stage,
            &binary,
            "0.1.0",
            &utilities,
            &mut registry,
        );
        let (modules, warnings) = result.expect("cleanup failure must not abort registration");
        assert_eq!(modules.len(), 46);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("cannot clean new stage"));
        assert_eq!(registry.plugins.len(), 46);
        assert_eq!(store::read(&fixture.0).unwrap().plugins.len(), 46);
        assert!(stage.registered);
        assert!(stage.root.join("modules/agentcmd.wc/bin").is_dir());
        let root = stage.root.clone();
        drop(stage);
        assert!(root.join("modules/agentcmd.wc/bin").is_dir());
        #[cfg(windows)]
        drop(locked);
    }

    #[test]
    fn failed_stage_cleans_new_directories_but_preserves_existing_user_data() {
        let fixture = Fixture::new();
        let existing = fixture.0.join("config/p-agentcmd.wc");
        fs::create_dir_all(&existing).unwrap();
        fs::write(existing.join("keep"), "user data").unwrap();
        let stage_path;
        {
            let (mut stage, _) = fixture.stage();
            stage_path = stage.root.clone();
            stage.user_dirs(&fixture.0, "agentcmd.wc").unwrap();
            stage.user_dirs(&fixture.0, "agentcmd.grep").unwrap();
            assert!(fixture.0.join("state/p-agentcmd.wc").is_dir());
        }
        assert!(!stage_path.exists());
        assert_eq!(
            fs::read_to_string(existing.join("keep")).unwrap(),
            "user data"
        );
        assert!(!fixture.0.join("state/p-agentcmd.wc").exists());
        assert!(!fixture.0.join("config/p-agentcmd.grep").exists());
        assert!(!fixture.0.join("state/p-agentcmd.grep").exists());
    }

    #[test]
    fn registration_failure_removes_partial_stage_and_new_user_dirs() {
        let fixture = Fixture::new();
        let blocked = fixture.0.join("state/p-agentcmd.basename");
        fs::create_dir_all(blocked.parent().unwrap()).unwrap();
        fs::write(&blocked, "existing file").unwrap();
        let utilities = parse_registry(UPSTREAM).unwrap();
        let (mut stage, binary) = fixture.stage();
        let stage_path = stage.root.clone();
        let mut registry = store::Registry::default();
        assert!(
            register_modules(
                &fixture.0,
                &mut stage,
                &binary,
                "0.1.0",
                &utilities,
                &mut registry,
            )
            .is_err()
        );
        drop(stage);
        assert!(!stage_path.exists());
        assert!(!fixture.0.join("config/p-agentcmd.bracket").exists());
        assert!(!fixture.0.join("state/p-agentcmd.bracket").exists());
        assert!(!fixture.0.join("config/p-agentcmd.basename").exists());
        assert_eq!(fs::read_to_string(blocked).unwrap(), "existing file");
        assert!(registry.plugins.is_empty());
        assert!(!fixture.0.join("registry.json").exists());
    }

    #[test]
    fn older_managed_revision_preserves_enabled_state() {
        let fixture = Fixture::new();
        let utilities = parse_registry(UPSTREAM).unwrap();
        let (mut stage, binary) = fixture.stage();
        let mut registry = store::Registry::default();
        register_modules(
            &fixture.0,
            &mut stage,
            &binary,
            "0.1.0",
            &utilities,
            &mut registry,
        )
        .unwrap();
        let marker = stage.root.join(MARKER);
        let mut value: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
        value["revision"] = json!("an-older-revision");
        store::atomic_json(&marker, &value).unwrap();
        registry.plugins.get_mut("agentcmd.wc").unwrap().enabled = true;
        let enabled = preflight(&fixture.0, &registry, &utilities).unwrap();
        assert_eq!(enabled.get("agentcmd.wc"), Some(&true));
    }

    #[test]
    fn unrelated_id_collision_does_not_overwrite_or_register_anything() {
        let fixture = Fixture::new();
        let utilities = parse_registry(UPSTREAM).unwrap();
        let (mut stage, binary) = fixture.stage();
        let root = fixture.0.join("unrelated");
        fs::create_dir_all(root.join("bin")).unwrap();
        let raw = format!(
            "id = 'agentcmd.wc'\nname = 'Unrelated'\nversion = '1.0.0'\nmin_crew_version = '{}'\nplatforms = ['windows','linux','macos']\n[[path_dirs]]\npath = 'bin'\n",
            env!("CARGO_PKG_VERSION")
        );
        fs::write(root.join("crew-plugin.toml"), &raw).unwrap();
        let mut registry = store::Registry::default();
        registry.plugins.insert(
            "agentcmd.wc".into(),
            store::Entry {
                root: root.clone(),
                source: store::Source::Local { path: root.clone() },
                enabled: true,
                manifest_toml: raw,
                installed_at: 0,
            },
        );
        store::write(&fixture.0, &registry).unwrap();
        let before = fs::read(fixture.0.join("registry.json")).unwrap();
        registry = store::read(&fixture.0).unwrap();
        let error = register_modules(
            &fixture.0,
            &mut stage,
            &binary,
            "0.1.0",
            &utilities,
            &mut registry,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unrelated"), "{error:#}");
        assert_eq!(registry.plugins.len(), 1);
        assert_eq!(registry.plugins["agentcmd.wc"].root, root);
        assert_eq!(fs::read(fixture.0.join("registry.json")).unwrap(), before);
    }
}
