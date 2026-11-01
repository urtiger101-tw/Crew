//! Crew plugin discovery and explicit argv actions. Installed code runs as the user.
mod catalog;
mod install;
mod manifest;
mod path_dirs;
mod process;
mod store;

use anyhow::{Context, Result, bail, ensure};
pub(crate) use path_dirs::PanePluginPaths;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const HELP: &str = r#"Crew plugin marketplace and explicit actions
  crew plugin marketplace list [--json]
  crew plugin marketplace search [QUERY] [--online] [--json]
  crew plugin marketplace show ID [--json]
  crew plugin inspect SOURCE|PATH [--ref REV] [--json]
  crew plugin install ID|owner/repo[/subdir] [--ref REV] [--yes] [--disabled]
  crew plugin link PATH [--yes] [--disabled]
  crew plugin update [ID|owner/repo[/subdir]] [--yes]
  crew plugin list [--json]
  crew plugin enable|disable|unlink|uninstall ID
  crew plugin config-dir|state-dir ID
  crew plugin action list [--plugin ID] [--json]
  crew plugin action invoke ACTION [--plugin ID] [--timeout-ms N]
  crew plugin log list [--plugin ID] [--limit N] [--json]

The bundled marketplace works offline. --online additionally searches public GitHub
repositories tagged crew-plugin, using curl with HTTPS verification. A search result
is discovery metadata, not a reviewed or installed plugin.
Install/link show the exact manifest and require approval (--yes for scripts).
Build/action commands are argv arrays and run with your user permissions, not in a
sandbox. Link does not build; install builds only after approval. No action runs on
install, enable or server startup. Uninstall retains files, config, state and logs.
Manifests are crew-plugin.toml with min_crew_version; Herdr manifests are not accepted.
Actions expand whole argv tokens {crew} and {cwd}; no shell expansion is performed.
Use CREW_PLUGIN_HOME to isolate the registry and all plugin files.
"#;

#[derive(Default)]
struct Args {
    values: BTreeMap<String, String>,
    flags: BTreeSet<String>,
    positional: Vec<String>,
}

fn parse(args: &[String], allowed_values: &[&str], allowed_flags: &[&str]) -> Result<Args> {
    let mut out = Args::default();
    let mut index = 0;
    while index < args.len() {
        let item = &args[index];
        index += 1;
        if item == "--" {
            out.positional.extend_from_slice(&args[index..]);
            break;
        }
        if !item.starts_with('-') {
            out.positional.push(item.clone());
            continue;
        }
        let (key, inline) = item
            .split_once('=')
            .map_or((item.as_str(), None), |(k, v)| (k, Some(v)));
        if allowed_flags.contains(&key) || key == "--json" {
            ensure!(inline.is_none(), "{key} takes no value");
            ensure!(out.flags.insert(key.into()), "duplicate option: {key}");
        } else if allowed_values.contains(&key) {
            let value = match inline {
                Some(v) => v.to_owned(),
                None => {
                    let value = args
                        .get(index)
                        .with_context(|| format!("missing value for {key}"))?
                        .clone();
                    ensure!(!value.starts_with("--"), "missing value for {key}");
                    index += 1;
                    value
                }
            };
            ensure!(
                out.values.insert(key.into(), value).is_none(),
                "duplicate option: {key}"
            );
        } else {
            bail!("unknown option: {key}");
        }
    }
    Ok(out)
}

fn count(args: &Args, min: usize, max: usize) -> Result<()> {
    ensure!(
        (min..=max).contains(&args.positional.len()),
        "expected {min}..{max} positional arguments; use crew plugin --help"
    );
    Ok(())
}

fn number(args: &Args, name: &str, default: u64, max: u64) -> Result<u64> {
    let Some(value) = args.values.get(name) else {
        return Ok(default);
    };
    ensure!(
        !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()),
        "{name} must be a positive integer"
    );
    let value = value
        .parse::<u64>()
        .with_context(|| format!("{name} is too large"))?;
    ensure!((1..=max).contains(&value), "{name} must be in 1..={max}");
    Ok(value)
}

fn emit(value: Value) -> Result<()> {
    println!("{}", json!({"result":value}));
    Ok(())
}

fn marketplace(args: &[String]) -> Result<()> {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let tail = if args.is_empty() { args } else { &args[1..] };
    let a = parse(tail, &[], if sub == "search" { &["--online"] } else { &[] })?;
    let mut entries = match sub {
        "list" => {
            count(&a, 0, 0)?;
            catalog::search("")?
        }
        "search" => {
            count(&a, 0, 1)?;
            catalog::search(a.positional.first().map(String::as_str).unwrap_or(""))?
        }
        "show" => {
            count(&a, 1, 1)?;
            let raw = catalog::bundled(&a.positional[0]).context(
                "bundled plugin not found; inspect a GitHub source to preview a third-party plugin",
            )?;
            let m = manifest::Manifest::parse(raw)?;
            return emit(
                json!({"source":format!("builtin:{}",m.id),"manifest":m,"code_executed":false}),
            );
        }
        _ => bail!("unknown marketplace command: {sub}"),
    };
    if a.flags.contains("--online") {
        entries.extend(discover(
            a.positional.first().map(String::as_str).unwrap_or(""),
        )?);
    }
    if a.flags.contains("--json") {
        return emit(json!({"plugins":entries,"online_requested":a.flags.contains("--online")}));
    }
    println!("Crew plugin marketplace");
    for e in &entries {
        println!(
            "{}  {}\n  {}\n  source: {}",
            e["id"].as_str().unwrap_or(""),
            e["name"].as_str().unwrap_or(""),
            clean(e["description"].as_str().unwrap_or(""), 240),
            e["source"].as_str().unwrap_or("")
        );
    }
    println!(
        "{} result(s). Inspect before installing: crew plugin inspect SOURCE",
        entries.len()
    );
    Ok(())
}

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

fn discover(query: &str) -> Result<Vec<Value>> {
    ensure!(
        query.len() <= 200 && !query.chars().any(char::is_control),
        "search query is too long or contains controls"
    );
    let text = format!("topic:crew-plugin {query}");
    let encoded: String = text
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let url = format!("https://api.github.com/search/repositories?q={encoded}&per_page=20");
    let mut command = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    command.args([
        "--disable",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "5",
        "--max-time",
        "15",
        "--max-filesize",
        "1048576",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "User-Agent: Crew-plugin-marketplace",
        "--url",
        &url,
    ]);
    let output = process::capture(command, Duration::from_secs(20), 1024 * 1024)?;
    ensure!(
        output.success() && !output.truncated,
        "GitHub discovery failed (curl/HTTPS required): {}",
        serde_json::to_string(&output)?
    );
    let response: Value =
        serde_json::from_str(&output.stdout).context("GitHub discovery did not return JSON")?;
    let items = response["items"]
        .as_array()
        .context("GitHub discovery response has no repositories")?;
    let mut entries = Vec::new();
    for repo in items.iter().take(20) {
        let Some(source) = repo["full_name"].as_str() else {
            continue;
        };
        if catalog::github_source(source).is_err() {
            continue;
        }
        entries.push(json!({"id":source,"name":clean(repo["name"].as_str().unwrap_or(source),80),"source":source,
            "description":clean(repo["description"].as_str().unwrap_or(""),512),"kind":"github_discovery",
            "manifest_verified":false,"reviewed":false,"url":format!("https://github.com/{source}")}));
    }
    Ok(entries)
}

// The pane-spawn integration consumes this contract in the next implementation step.
#[allow(dead_code)]
pub(crate) fn pane_path_dirs() -> Result<PanePluginPaths> {
    path_dirs::collect(&store::home()?)
}

fn entries(home: &Path) -> Result<Vec<Value>> {
    store::read(home)?
        .plugins
        .into_iter()
        .map(|(id, entry)| {
            let parsed = manifest::Manifest::parse(&entry.manifest_toml)?;
            let compatible = manifest::compatible(&[], &parsed.platforms);
            let current = store::loaded(&entry);
            let unchanged = current.is_ok();
            let resolved = current.and_then(|manifest| path_dirs::resolve(&entry, &manifest));
            let diagnostic = resolved.as_ref().err().map(|error| format!("{error:#}"));
            let dirs = resolved.unwrap_or_default();
            let available = entry.enabled && compatible && diagnostic.is_none();
            let effective_dirs = if entry.enabled && compatible {
                dirs.clone()
            } else {
                Vec::new()
            };
            Ok(json!({
                "id": id, "name": parsed.name, "version": parsed.version, "enabled": entry.enabled,
                "source": entry.source, "root": entry.root, "platform_compatible": compatible,
                "manifest_unchanged": unchanged, "available": available, "diagnostic": diagnostic,
                "resolved_path_dirs": dirs, "effective_path_dirs": effective_dirs
            }))
        })
        .collect()
}

fn mutate(home: &Path, verb: &str, id: &str) -> Result<Value> {
    let _lock = store::lock(home)?;
    let mut registry = store::read(home)?;
    let key = registry
        .plugins
        .iter()
        .find(|(key, entry)| {
            key.as_str() == id
                || matches!(&entry.source,store::Source::Github{shorthand,..} if shorthand==id)
        })
        .map(|(key, _)| key.clone())
        .context("plugin is not installed")?;
    if matches!(verb, "enable" | "disable") {
        let entry = registry.plugins.get_mut(&key).unwrap();
        if verb == "enable" {
            store::loaded(entry)?.require_platform()?;
        }
        entry.enabled = verb == "enable";
    } else {
        registry.plugins.remove(&key);
    }
    store::write(home, &registry)?;
    Ok(
        json!({"plugin_id":key,"operation":verb,"files_retained":true,"config_and_state_retained":true}),
    )
}

fn actions(home: &Path, filter: Option<&str>) -> Result<Vec<Value>> {
    let registry = store::read(home)?;
    ensure!(
        filter.is_none_or(|id| registry.plugins.contains_key(id)),
        "plugin is not installed"
    );
    let mut result = Vec::new();
    for (id, entry) in &registry.plugins {
        if filter.is_some_and(|filter| filter != id) {
            continue;
        }
        let manifest = manifest::Manifest::parse(&entry.manifest_toml)?;
        let diagnostic = store::loaded(entry).err().map(|error| error.to_string());
        for action in &manifest.actions {
            let compatible = manifest::compatible(&action.platforms, &manifest.platforms);
            result.push(json!({"id":format!("{id}.{}",action.id),"local_id":action.id,"plugin_id":id,"title":action.title,
                "command":action.command,"enabled":entry.enabled,"platform_compatible":compatible,
                "available":entry.enabled && compatible && diagnostic.is_none(),
                "manifest_unchanged":diagnostic.is_none(),"diagnostic":diagnostic}));
        }
    }
    Ok(result)
}

fn invoke(home: &Path, name: &str, filter: Option<&str>, timeout: u64) -> Result<(Value, i32)> {
    let registry = store::read(home)?;
    let mut selected = Vec::new();
    for (id, entry) in &registry.plugins {
        if filter.is_some_and(|filter| filter != id) {
            continue;
        }
        let m = manifest::Manifest::parse(&entry.manifest_toml)?;
        for action in &m.actions {
            if name == action.id || name == format!("{id}.{}", action.id) {
                selected.push((id.clone(), entry.clone(), action.clone()));
            }
        }
    }
    ensure!(
        selected.len() == 1,
        "action must identify exactly one installed plugin; use a qualified plugin.id.action or --plugin"
    );
    let (id, entry, action) = selected.pop().unwrap();
    ensure!(entry.enabled, "plugin is disabled: {id}");
    let manifest = store::loaded(&entry)?;
    manifest.require_platform()?;
    ensure!(
        manifest::compatible(&action.platforms, &manifest.platforms),
        "action is not compatible with this platform"
    );
    let executable = std::env::current_exe()?;
    let cwd = std::env::current_dir()?;
    let (config, state) = store::user_dirs(home, &id)?;
    let command: Vec<String> = action
        .command
        .iter()
        .map(|arg| match arg.as_str() {
            "{crew}" => executable.to_string_lossy().into_owned(),
            "{cwd}" => cwd.to_string_lossy().into_owned(),
            _ => arg.clone(),
        })
        .collect();
    let context = json!({"cwd":cwd,"pane_id":crate::brand::env_var("PANE_ID").ok(),"invocation_source":"cli"});
    let mut child = process::plugin_command(&entry.root, &command[0]);
    child
        .args(&command[1..])
        .env("CREW_BIN_PATH", &executable)
        .env("CREW_PLUGIN_ID", &id)
        .env("CREW_PLUGIN_ROOT", &entry.root)
        .env("CREW_PLUGIN_CONFIG_DIR", &config)
        .env("CREW_PLUGIN_STATE_DIR", &state)
        .env("CREW_PLUGIN_ACTION_ID", &action.id)
        .env("CREW_PLUGIN_CONTEXT_JSON", serde_json::to_string(&context)?);
    let started = store::stamp();
    let captured = process::capture(child, Duration::from_millis(timeout), 64 * 1024);
    let (output, success) = match captured {
        Ok(output) => {
            let success = output.success();
            (serde_json::to_value(output)?, success)
        }
        Err(error) => (
            json!({"spawn_error":error.to_string(),"exit_code":null}),
            false,
        ),
    };
    let log_id = format!("{started}-{}", std::process::id());
    let record = json!({"log_id":log_id,"plugin_id":id,"action_id":action.id,"command":command,"cwd":entry.root,
        "started_at_ms":started,"finished_at_ms":store::stamp(),"success":success,"output":output,"source":entry.source});
    store::atomic_json(
        &home
            .join("logs")
            .join(format!("p-{id}"))
            .join(format!("{log_id}.json")),
        &record,
    )?;
    Ok((record, if success { 0 } else { 1 }))
}

fn logs(home: &Path, filter: Option<&str>, limit: usize) -> Result<Vec<Value>> {
    if let Some(id) = filter {
        ensure!(manifest::identifier(id, true), "invalid plugin id");
    }
    let root = home.join("logs");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for directory in fs::read_dir(&root)?.take(512) {
        let directory = directory?;
        if !directory.file_type()?.is_dir() {
            continue;
        }
        if filter.is_some_and(|id| directory.file_name() != format!("p-{id}").as_str()) {
            continue;
        }
        for file in fs::read_dir(directory.path())?.take(4096) {
            let file = file?;
            if file.file_type()?.is_file() && file.path().extension().is_some_and(|e| e == "json") {
                files.push(file.path());
            }
        }
    }
    files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    let mut bytes = 0usize;
    files
        .into_iter()
        .take(limit)
        .map(|file| {
            let raw = store::read_limited(&file, 2 * 1024 * 1024)?;
            bytes += raw.len();
            ensure!(
                bytes <= 8 * 1024 * 1024,
                "selected logs exceed 8 MiB; use a smaller --limit"
            );
            serde_json::from_str(&raw).context("invalid plugin command log")
        })
        .collect()
}

pub fn run(args: &[String]) -> Result<i32> {
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        println!("{HELP}");
        return Ok(0);
    }
    let verb = args[0].as_str();
    if verb == "marketplace" {
        marketplace(&args[1..])?;
        return Ok(0);
    }
    let home = store::home()?;
    let value = match verb {
        "inspect" => {
            let a = parse(&args[1..], &["--ref"], &[])?;
            count(&a, 1, 1)?;
            install::inspect(
                &home,
                &a.positional[0],
                a.values.get("--ref").map(String::as_str),
            )?
        }
        "install" => {
            let a = parse(&args[1..], &["--ref"], &["--yes", "--disabled"])?;
            count(&a, 1, 1)?;
            install::install(
                &home,
                &a.positional[0],
                a.values.get("--ref").map(String::as_str),
                a.flags.contains("--disabled"),
                a.flags.contains("--yes"),
            )?
        }
        "link" => {
            let a = parse(&args[1..], &[], &["--yes", "--disabled"])?;
            count(&a, 1, 1)?;
            install::link(
                &home,
                Path::new(&a.positional[0]),
                a.flags.contains("--disabled"),
                a.flags.contains("--yes"),
            )?
        }
        "update" => {
            let a = parse(&args[1..], &[], &["--yes"])?;
            count(&a, 0, 1)?;
            install::update(
                &home,
                a.positional.first().map(String::as_str),
                a.flags.contains("--yes"),
            )?
        }
        "list" => {
            let a = parse(&args[1..], &[], &[])?;
            count(&a, 0, 0)?;
            json!({"plugins":entries(&home)?})
        }
        "enable" | "disable" | "unlink" | "uninstall" => {
            let a = parse(&args[1..], &[], &[])?;
            count(&a, 1, 1)?;
            mutate(&home, verb, &a.positional[0])?
        }
        "config-dir" | "state-dir" => {
            let a = parse(&args[1..], &[], &[])?;
            count(&a, 1, 1)?;
            ensure!(
                store::read(&home)?.plugins.contains_key(&a.positional[0]),
                "plugin is not installed"
            );
            let (config, state) = store::user_dirs(&home, &a.positional[0])?;
            let path = if verb == "config-dir" { config } else { state };
            if !a.flags.contains("--json") {
                println!("{}", path.display());
                return Ok(0);
            }
            json!({"path":path})
        }
        "action" => {
            let sub = args.get(1).context("action requires list or invoke")?;
            let a = parse(
                &args[2..],
                if sub == "invoke" {
                    &["--plugin", "--timeout-ms"]
                } else {
                    &["--plugin"]
                },
                &[],
            )?;
            let filter = a.values.get("--plugin").map(String::as_str);
            match sub.as_str() {
                "list" => {
                    count(&a, 0, 0)?;
                    json!({"actions":actions(&home,filter)?})
                }
                "invoke" => {
                    count(&a, 1, 1)?;
                    let (result, code) = invoke(
                        &home,
                        &a.positional[0],
                        filter,
                        number(&a, "--timeout-ms", 30000, 300000)?,
                    )?;
                    emit(result)?;
                    return Ok(code);
                }
                _ => bail!("action requires list or invoke"),
            }
        }
        "log" => {
            ensure!(
                args.get(1).is_some_and(|s| s == "list"),
                "log requires list"
            );
            let a = parse(&args[2..], &["--plugin", "--limit"], &[])?;
            count(&a, 0, 0)?;
            json!({"logs":logs(&home,a.values.get("--plugin").map(String::as_str),number(&a,"--limit",20,100)? as usize)?})
        }
        _ => bail!("unknown plugin command: {verb}; use crew plugin --help"),
    };
    emit(value)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_does_not_silently_ignore_unknown_options_or_duplicate_flags() {
        let args = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert!(parse(&args(&["--yes", "--yes"]), &[], &["--yes"]).is_err());
        assert!(parse(&args(&["--ref", "--yes"]), &["--ref"], &["--yes"]).is_err());
        assert!(parse(&args(&["--ref=x"]), &[], &[]).is_err());
        assert_eq!(
            parse(&args(&["--", "--path"]), &[], &[])
                .unwrap()
                .positional,
            vec!["--path"]
        );
    }
    #[test]
    fn bundled_catalog_is_usable_and_not_third_party_approval() {
        let all = catalog::search("").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(catalog::search("worktree").unwrap().len(), 1);
        for (_, raw) in catalog::BUNDLED {
            manifest::Manifest::parse(raw)
                .unwrap()
                .require_platform()
                .unwrap();
        }
    }
}
