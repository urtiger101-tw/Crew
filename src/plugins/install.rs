use super::{catalog, manifest, process, store};
use anyhow::{Context, Result, bail, ensure};
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

struct Prepared {
    root: PathBuf,
    source: store::Source,
    raw: String,
    manifest: manifest::Manifest,
}

fn git(root: &Path, config: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", config)
        .arg("-C")
        .arg(root)
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
        ])
        .args(args);
    let output = process::capture(command, Duration::from_secs(120), 128 * 1024)?;
    ensure!(
        output.success() && !output.truncated,
        "git failed: {}",
        serde_json::to_string(&output)?
    );
    Ok(output.stdout.trim().to_owned())
}

fn prepare(home: &Path, source: &str, revision: Option<&str>) -> Result<Prepared> {
    if let Some(raw) = catalog::bundled(source) {
        ensure!(revision.is_none(), "bundled plugins do not accept --ref");
        let parsed = manifest::Manifest::parse(raw)?;
        let root = store::stage(home)?;
        fs::write(root.join("crew-plugin.toml"), raw)?;
        return Ok(Prepared {
            root,
            source: store::Source::Bundled {
                id: parsed.id.clone(),
            },
            raw: raw.into(),
            manifest: parsed,
        });
    }
    let (repo, subdir) = catalog::github_source(source)?;
    if let Some(value) = revision {
        catalog::git_ref(value)?;
    }
    let stage = store::stage(home)?;
    let empty_config = stage.join("gitconfig");
    fs::write(&empty_config, b"")?;
    let template = stage.join("empty-template");
    fs::create_dir(&template)?;
    let checkout = stage.join("checkout");
    fs::create_dir(&checkout)?;
    git(
        &checkout,
        &empty_config,
        &[
            "init",
            "--quiet",
            &format!("--template={}", template.to_string_lossy()),
        ],
    )?;
    let url = format!("https://github.com/{repo}.git");
    git(
        &checkout,
        &empty_config,
        &[
            "fetch",
            "--quiet",
            "--depth=1",
            "--no-tags",
            &url,
            revision.unwrap_or("HEAD"),
        ],
    )?;
    git(
        &checkout,
        &empty_config,
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "--detach",
            "FETCH_HEAD",
        ],
    )?;
    let commit = git(&checkout, &empty_config, &["rev-parse", "HEAD"])?;
    ensure!(
        [40, 64].contains(&commit.len()) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid fetched commit"
    );
    let base = checkout.canonicalize()?;
    let root = base
        .join(&subdir)
        .canonicalize()
        .context("plugin subdirectory does not exist")?;
    ensure!(
        root.starts_with(&base),
        "plugin subdirectory escapes its checkout"
    );
    let manifest_path = root.join("crew-plugin.toml").canonicalize()?;
    ensure!(
        manifest_path.starts_with(&base),
        "plugin manifest escapes its checkout"
    );
    let (raw, parsed) = store::manifest(&root)?;
    Ok(Prepared {
        root,
        source: store::Source::Github {
            shorthand: source.into(),
            revision: revision.map(str::to_owned),
            commit,
        },
        raw,
        manifest: parsed,
    })
}

fn approve(prepared: &Prepared, yes: bool, builds: bool) -> Result<()> {
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "plugin":prepared.manifest,"source":prepared.source,
            "root":prepared.root,"builds_will_run":builds,
            "trust":"Plugin commands are ordinary code with your user permissions; Crew does not sandbox them."
        }))?
    );
    if yes {
        return Ok(());
    }
    ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "approval required: inspect this manifest, then repeat with --yes"
    );
    eprint!(
        "Trust this plugin and register it{}? [y/N] ",
        if builds {
            " after running its build commands"
        } else {
            ""
        }
    );
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    ensure!(
        matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"),
        "plugin installation cancelled"
    );
    Ok(())
}

fn register(
    home: &Path,
    registry: &mut store::Registry,
    prepared: Prepared,
    disabled: bool,
    yes: bool,
    builds: bool,
) -> Result<serde_json::Value> {
    prepared.manifest.require_platform()?;
    let id = &prepared.manifest.id;
    for existing in registry.plugins.keys() {
        ensure!(
            existing == id || !existing.eq_ignore_ascii_case(id),
            "plugin id differs only by case from {existing}"
        );
    }
    if let Some(previous) = registry.plugins.get(id) {
        ensure!(
            !matches!(previous.source, store::Source::Local { .. })
                || matches!(prepared.source, store::Source::Local { .. }),
            "cannot overwrite a locally linked plugin; unlink it first"
        );
    }
    ensure!(
        registry.plugins.contains_key(id) || registry.plugins.len() < 128,
        "plugin registry is full"
    );
    approve(
        &prepared,
        yes,
        builds && !prepared.manifest.build.is_empty(),
    )?;
    if builds {
        for (index, build) in prepared.manifest.build.iter().enumerate() {
            if !manifest::compatible(&build.platforms, &prepared.manifest.platforms) {
                continue;
            }
            let mut command = process::plugin_command(&prepared.root, &build.command[0]);
            command.args(&build.command[1..]);
            for (key, _) in std::env::vars_os() {
                let name = key.to_string_lossy();
                if name == "CREW"
                    || name == "TMUXLIKE"
                    || name.starts_with("CREW_")
                    || name.starts_with("TMUXLIKE_")
                {
                    command.env_remove(key);
                }
            }
            let output = process::capture(command, Duration::from_secs(120), 64 * 1024)?;
            let log = home.join("build-logs").join(format!(
                "{}-{}-{index}.json",
                store::stamp(),
                std::process::id()
            ));
            store::atomic_json(
                &log,
                &serde_json::json!({"plugin_id":id,"command":build.command,"output":output}),
            )?;
            ensure!(
                output.success(),
                "plugin build {index} failed; evidence: {}",
                log.display()
            );
        }
    }
    ensure!(
        store::read_limited(
            &prepared.root.join("crew-plugin.toml"),
            manifest::MAX_MANIFEST
        )? == prepared.raw,
        "manifest changed after approval; installation was not registered"
    );
    let (config, state) = store::user_dirs(home, id)?;
    let entry = store::Entry {
        root: prepared.root,
        source: prepared.source,
        enabled: !disabled,
        manifest_toml: prepared.raw,
        installed_at: store::stamp(),
    };
    registry.plugins.insert(id.clone(), entry.clone());
    store::write(home, registry)?;
    Ok(
        serde_json::json!({"plugin_id":id,"version":prepared.manifest.version,"enabled":entry.enabled,"root":entry.root,
        "source":entry.source,"config_dir":config,"state_dir":state,"old_installations_retained":true}),
    )
}

pub fn inspect(home: &Path, source: &str, revision: Option<&str>) -> Result<serde_json::Value> {
    if Path::new(source).exists() {
        ensure!(revision.is_none(), "local inspection does not accept --ref");
        let root = local_root(Path::new(source))?;
        let (_, manifest) = store::manifest(&root)?;
        return Ok(
            serde_json::json!({"manifest":manifest,"root":root,"source":"local","code_executed":false}),
        );
    }
    let p = prepare(home, source, revision)?;
    Ok(
        serde_json::json!({"manifest":p.manifest,"root":p.root,"source":p.source,"code_executed":false}),
    )
}

fn local_root(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize().context("plugin path does not exist")?;
    if path.is_file() {
        ensure!(
            path.file_name().is_some_and(|n| n == "crew-plugin.toml"),
            "expected crew-plugin.toml"
        );
        return Ok(path.parent().context("manifest has no parent")?.to_owned());
    }
    ensure!(path.is_dir(), "plugin path must be a directory or manifest");
    Ok(path)
}

pub fn link(home: &Path, path: &Path, disabled: bool, yes: bool) -> Result<serde_json::Value> {
    let _lock = store::lock(home)?;
    let mut registry = store::read(home)?;
    let root = local_root(path)?;
    let (raw, manifest) = store::manifest(&root)?;
    let prepared = Prepared {
        source: store::Source::Local { path: root.clone() },
        root,
        raw,
        manifest,
    };
    register(home, &mut registry, prepared, disabled, yes, false)
}

pub fn install(
    home: &Path,
    source: &str,
    revision: Option<&str>,
    disabled: bool,
    yes: bool,
) -> Result<serde_json::Value> {
    let _lock = store::lock(home)?;
    let mut registry = store::read(home)?;
    let prepared = prepare(home, source, revision)?;
    register(home, &mut registry, prepared, disabled, yes, true)
}

pub fn update(home: &Path, target: Option<&str>, yes: bool) -> Result<serde_json::Value> {
    let _lock = store::lock(home)?;
    let mut registry = store::read(home)?;
    let chosen: Vec<_> = registry
        .plugins
        .iter()
        .filter(|(id, entry)| {
            target.is_none_or(|t| {
                t == id.as_str()
                    || matches!(&entry.source,store::Source::Github{shorthand,..} if shorthand==t)
            })
        })
        .map(|(id, entry)| (id.clone(), entry.clone()))
        .collect();
    if target.is_some() && chosen.is_empty() {
        bail!("plugin is not installed");
    }
    let mut results = Vec::new();
    for (id, old) in chosen {
        let (source, revision) = match &old.source {
            store::Source::Local { .. } => {
                results.push(serde_json::json!({"plugin_id":id,"status":"skipped_local_link"}));
                continue;
            }
            store::Source::Bundled { id } => (format!("builtin:{id}"), None),
            store::Source::Github {
                shorthand,
                revision,
                ..
            } => (shorthand.clone(), revision.clone()),
        };
        let next = prepare(home, &source, revision.as_deref())?;
        ensure!(
            next.manifest.id == id,
            "updated source changed plugin id; update refused"
        );
        let same = match (&old.source, &next.source) {
            (store::Source::Github { commit: a, .. }, store::Source::Github { commit: b, .. }) => {
                a == b
            }
            (store::Source::Bundled { .. }, store::Source::Bundled { .. }) => {
                old.manifest_toml == next.raw
            }
            _ => false,
        };
        if same {
            results.push(serde_json::json!({"plugin_id":id,"status":"unchanged"}));
            continue;
        }
        results.push(register(
            home,
            &mut registry,
            next,
            !old.enabled,
            yes,
            true,
        )?);
    }
    Ok(serde_json::json!({"updates":results}))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "crew-plugin-build-{}-{}-{}",
                std::process::id(),
                store::stamp(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn prepared(&self, directory: &str, script: Option<&str>) -> Prepared {
            let root = self.0.join(directory);
            fs::create_dir_all(&root).unwrap();
            let build = script.map_or(String::new(), |script| {
                format!(
                    "\n[[build]]\ncommand = [\"sh\", \"-c\", {}]\n",
                    serde_json::to_string(script).unwrap()
                )
            });
            let raw = format!(
                "id = \"fixture.build\"\nname = \"Build fixture\"\nversion = \"1.0.0\"\n\
                 min_crew_version = \"0.5.0\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n\
                 {build}\n[[actions]]\nid = \"read\"\ntitle = \"Read fixture\"\n\
                 command = [\"sh\", \"-c\", \"printf fixture\"]\n"
            );
            fs::write(root.join("crew-plugin.toml"), &raw).unwrap();
            Prepared {
                root,
                source: store::Source::Bundled {
                    id: "fixture.build".into(),
                },
                manifest: manifest::Manifest::parse(&raw).unwrap(),
                raw,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn approved_build_runs_before_registration_and_keeps_log() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let home = fixture.0.join("home");
        let mut prepared = fixture.prepared(
            "source",
            Some("printf built > built.txt; printf build-output"),
        );
        let build = prepared.root.join("bin/build");
        fs::create_dir_all(build.parent().unwrap()).unwrap();
        fs::write(
            &build,
            "#!/bin/sh\nprintf built > built.txt; printf build-output\n",
        )
        .unwrap();
        fs::set_permissions(&build, fs::Permissions::from_mode(0o700)).unwrap();
        prepared.manifest.build[0].command = vec!["./bin/build".into()];
        prepared.raw = toml::to_string(&prepared.manifest).unwrap();
        fs::write(prepared.root.join("crew-plugin.toml"), &prepared.raw).unwrap();
        let root = prepared.root.clone();
        let mut registry = store::Registry::default();
        register(&home, &mut registry, prepared, true, true, true).unwrap();
        assert_eq!(fs::read_to_string(root.join("built.txt")).unwrap(), "built");
        let saved = store::read(&home).unwrap();
        assert!(!saved.plugins["fixture.build"].enabled);
        let logs: Vec<_> = fs::read_dir(home.join("build-logs")).unwrap().collect();
        assert_eq!(logs.len(), 1);
        let raw = fs::read_to_string(logs[0].as_ref().unwrap().path()).unwrap();
        assert!(raw.contains("build-output"));
    }

    #[test]
    fn failed_build_preserves_previous_installation_and_failure_log() {
        let fixture = Fixture::new();
        let home = fixture.0.join("home");
        let mut registry = store::Registry::default();
        let old = fixture.prepared("old", None);
        let old_root = old.root.clone();
        register(&home, &mut registry, old, false, true, true).unwrap();
        let before = fs::read(home.join("registry.json")).unwrap();
        let replacement = fixture.prepared("replacement", Some("printf rejected >&2; exit 17"));
        let error = register(&home, &mut registry, replacement, false, true, true).unwrap_err();
        assert!(error.to_string().contains("build 0 failed"));
        assert_eq!(registry.plugins["fixture.build"].root, old_root);
        assert_eq!(fs::read(home.join("registry.json")).unwrap(), before);
        let log = fs::read_dir(home.join("build-logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(log.path()).unwrap()).unwrap();
        assert_eq!(record["output"]["exit_code"], 17);
        assert_eq!(record["output"]["stderr"], "rejected");
    }

    #[test]
    fn build_cannot_register_a_manifest_changed_after_approval() {
        let fixture = Fixture::new();
        let home = fixture.0.join("home");
        let prepared =
            fixture.prepared("source", Some("printf '\n# changed\n' >> crew-plugin.toml"));
        let mut registry = store::Registry::default();
        let error = register(&home, &mut registry, prepared, false, true, true).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("manifest changed after approval")
        );
        assert!(registry.plugins.is_empty());
        assert!(!home.join("registry.json").exists());
        assert_eq!(fs::read_dir(home.join("build-logs")).unwrap().count(), 1);
    }
}
