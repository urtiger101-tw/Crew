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
        && marker["revision"] == REVISION
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
    stage: &Path,
    binary: &Path,
    version: &str,
    utilities: &[Utility],
    registry: &mut store::Registry,
) -> Result<Vec<Value>> {
    let enabled = preflight(home, registry, utilities)?;
    let shared = stage.join("shared");
    fs::create_dir_all(&shared)?;
    let shared_binary = shared.join(executable_name("agentcmd"));
    fs::copy(binary, &shared_binary).context("cannot store the shared agentcmd binary")?;
    store::atomic_json(
        &stage.join(MARKER),
        &json!({
            "kind":"crew-managed-agentcmd", "revision":REVISION,
            "plugins":utilities.iter().map(|utility| &utility.id).collect::<Vec<_>>()
        }),
    )?;
    let mut entries = Vec::new();
    let mut result = Vec::new();
    for utility in utilities {
        let root = stage.join("modules").join(&utility.id);
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
        store::user_dirs(home, &utility.id)?;
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
    store::write(home, &next)?;
    *registry = next;
    Ok(result)
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
    let stage = store::stage(home)?;
    let config = stage.join("gitconfig");
    fs::write(&config, "")?;
    let template = stage.join("empty-template");
    fs::create_dir(&template)?;
    let repository = if let Some(path) = &local_source {
        path.clone()
    } else {
        let repository = stage.join("source.git");
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
    let archive = stage.join("source.tar");
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
    let checkout = stage.join("source");
    fs::create_dir(&checkout)?;
    let mut extract = Command::new("tar");
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
    let target = stage.join("target");
    let mut build_arguments = vec![
        "cargo".to_owned(),
        "build".to_owned(),
        "--locked".to_owned(),
        "--bin".to_owned(),
        "agentcmd".to_owned(),
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
        .args(["build", "--locked", "--bin", "agentcmd"])
        .arg("--target-dir")
        .arg(child_path(&target)?);
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
    let binary = target.join("debug").join(executable_name("agentcmd"));
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
    let modules = register_modules(home, &stage, &binary, version, &utilities, &mut registry)?;
    Ok(
        json!({"revision":REVISION,"source":UPSTREAM_URL,"local_repository":local_source,"offline":offline,
        "shared_binary":stage.join("shared").join(executable_name("agentcmd")),"build_log":log,
        "modules":modules,"reinstall":"crew plugin agentcmd install","generic_plugin_update":"skipped_local_link"}),
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

        fn stage(&self) -> (PathBuf, PathBuf) {
            let stage = store::stage(&self.0).unwrap();
            let binary = stage.join("fixture-agentcmd");
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
        let (stage, binary) = fixture.stage();
        let mut registry = store::Registry::default();
        register_modules(
            &fixture.0,
            &stage,
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
            assert!(!entry.root.starts_with(stage.join("shared")));
            if let Some(conflict) = &utility.conflict {
                assert!(parsed.description.contains(conflict));
            }
        }
        registry.plugins.get_mut("agentcmd.wc").unwrap().enabled = true;
        store::write(&fixture.0, &registry).unwrap();
        registry = store::read(&fixture.0).unwrap();
        let (next_stage, next_binary) = fixture.stage();
        register_modules(
            &fixture.0,
            &next_stage,
            &next_binary,
            "0.1.0",
            &utilities,
            &mut registry,
        )
        .unwrap();
        assert!(registry.plugins["agentcmd.wc"].enabled);
        assert!(!registry.plugins["agentcmd.sort"].enabled);
    }

    #[test]
    fn unrelated_id_collision_does_not_overwrite_or_register_anything() {
        let fixture = Fixture::new();
        let utilities = parse_registry(UPSTREAM).unwrap();
        let (stage, binary) = fixture.stage();
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
            &stage,
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
