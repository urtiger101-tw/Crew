//! Saved SSH machines and a client-side federated workspace/agent view.
mod dashboard;
mod store;
mod transport;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::IsTerminal;
use std::path::PathBuf;
use store::Machine;

pub use transport::serve as transport_serve;

pub const HELP: &str = r#"SSH MACHINES (native OpenSSH, no remote installation or server restart):
  crew machine add HOST [--label TEXT] [--binary ABSOLUTE_PATH|crew]
                        [--socket NAME] [--ssh-config PATH] [--no-probe]
  crew machine list [--json]
  crew machine rename ID|LABEL --label TEXT
  crew machine enable|disable|remove ID|LABEL
  crew machine status [ID|LABEL] [--timeout-ms N] [--json]
  crew machine reconnect ID|LABEL [--timeout-ms N]
  crew machine attach ID|LABEL [--workspace NAME]
  crew --machine ID|LABEL [--timeout-ms N] workspace|tab|pane|agent|worktree|events ...
  crew --machine ID|LABEL api snapshot
  crew federation [list|workspaces|agents] [--timeout-ms N] [--json]
  crew federation watch [--timeout-ms N]
HOST is a saved OpenSSH alias or user@host; set ports/keys in SSH config.
The remote POSIX host must already have this Crew version and a running daemon.
`--machine` never falls back to Local and never retries a possibly delivered request.
Federation output includes machine_id on every resource; IDs are only unique per machine.
The interactive chooser uses Up/Down, Enter to attach, r to refresh, q to exit.
"#;

#[cfg(test)]
fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|s| s.to_string()).collect()
}

pub fn run(args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("machine") => machine_cli(&args[1..]),
        Some("federation") => federation_cli(&args[1..]),
        Some("--machine") => match route(&args[1..]) {
            Ok(value) => {
                print_result(value);
                Ok(0)
            }
            Err(error) => {
                eprintln!(
                    "{}",
                    json!({"error":{"code":"remote_error","message":error.to_string()}})
                );
                Ok(1)
            }
        },
        _ => bail!("unknown remote command"),
    }
}

fn print_result(value: Value) {
    if let Some(text) = value.as_str() {
        print!("{text}");
        if !text.is_empty() && !text.ends_with('\n') {
            println!();
        }
    } else {
        println!("{}", json!({"result":value}));
    }
}

fn timeout(value: &str) -> Result<u64> {
    let value: u64 = value.parse().context("timeout-ms must be an integer")?;
    if !(250..=300_000).contains(&value) {
        bail!("timeout-ms must be 250..300000");
    }
    Ok(value)
}

fn route(args: &[String]) -> Result<Value> {
    let (selector, rest) = args
        .split_first()
        .context("usage: crew --machine ID|LABEL COMMAND ...")?;
    let (duration, rest) = if rest.first().is_some_and(|s| s == "--timeout-ms") {
        (
            timeout(rest.get(1).context("missing timeout-ms")?)?,
            &rest[2..],
        )
    } else {
        (transport::DEFAULT_TIMEOUT, rest)
    };
    // Validate before opening SSH; no implicit local pane id in a different server.
    crate::api::cli::parse(rest, None)
        .map_err(|e| anyhow::anyhow!("invalid remote command: {e}"))?;
    let catalog = store::load(&store::path()?)?;
    let machine = catalog.select(selector, true)?;
    transport::remote(machine, rest, false, duration)
}

fn machine_cli(args: &[String]) -> Result<i32> {
    let Some((command, rest)) = args.split_first() else {
        println!("{HELP}");
        return Ok(0);
    };
    if matches!(command.as_str(), "--help" | "-h" | "help") {
        println!("{HELP}");
        return Ok(0);
    }
    let path = store::path()?;
    match command.as_str() {
        "list" => {
            if !rest.is_empty() && rest != ["--json"] {
                bail!("usage: crew machine list [--json]");
            }
            print_result(json!({"machines": store::load(&path)?.machines}));
        }
        "add" => {
            let (destination, options) = rest
                .split_first()
                .context("machine add needs an SSH target")?;
            let mut machine = Machine {
                id: "m1".into(),
                label: destination.clone(),
                destination: destination.clone(),
                binary: "crew".into(),
                socket: None,
                ssh_config: None,
                enabled: true,
            };
            let mut probe = true;
            let mut seen = std::collections::BTreeSet::new();
            let mut i = 0;
            while i < options.len() {
                let option = &options[i];
                if !seen.insert(option) {
                    bail!("repeated option: {option}");
                }
                if option == "--no-probe" {
                    probe = false;
                    i += 1;
                    continue;
                }
                let value = options
                    .get(i + 1)
                    .with_context(|| format!("missing value for {option}"))?;
                match option.as_str() {
                    "--label" => machine.label = value.clone(),
                    "--binary" => machine.binary = value.clone(),
                    "--socket" => machine.socket = Some(value.clone()),
                    "--ssh-config" => {
                        machine.ssh_config = Some(
                            std::fs::canonicalize(PathBuf::from(value))
                                .context("SSH config must exist")?,
                        )
                    }
                    _ => bail!("unknown machine add option: {option}"),
                }
                i += 2;
            }
            machine.validate()?;
            // Network work occurs before taking the local registry write lock.
            let catalog = store::load(&path)?;
            if catalog.machines.iter().any(|m| m.label == machine.label) {
                bail!("machine label already exists");
            }
            if probe {
                transport::remote(&machine, &[], true, transport::DEFAULT_TIMEOUT)?;
            }
            let saved = store::edit(&path, |catalog| catalog.add(machine))?;
            print_result(json!({"machine":saved,"probed":probe}));
        }
        "rename" | "enable" | "disable" | "remove" => {
            let selector = rest
                .first()
                .context("machine operation needs an id or label")?;
            let label = if command == "rename" {
                if rest.len() != 3 || rest[1] != "--label" {
                    bail!("usage: crew machine rename ID --label TEXT");
                }
                store::validate_label(&rest[2])?;
                Some(rest[2].clone())
            } else {
                if rest.len() != 1 {
                    bail!("machine {command} takes one selector");
                }
                None
            };
            let result = store::edit(&path, |catalog| {
                let id = catalog.select(selector, false)?.id.clone();
                let index = catalog.machines.iter().position(|m| m.id == id).unwrap();
                if let Some(label) = label {
                    if catalog
                        .machines
                        .iter()
                        .any(|m| m.id != id && m.label == label)
                    {
                        bail!("machine label already exists");
                    }
                    catalog.machines[index].label = label;
                } else if command == "remove" {
                    catalog.machines.remove(index);
                } else {
                    catalog.machines[index].enabled = command == "enable";
                }
                Ok(json!({"machine_id":id,"operation":command}))
            })?;
            print_result(result);
        }
        "status" | "probe" | "reconnect" => {
            let (selector, duration) = status_options(rest)?;
            if command == "reconnect" && selector.is_none() {
                bail!("reconnect requires a saved machine selector");
            }
            let catalog = store::load(&path)?;
            let machines = match selector {
                Some(selector) => vec![catalog.select(&selector, true)?.clone()],
                None => catalog.machines.clone(),
            };
            let views = gather(machines, false, duration);
            let complete = views
                .iter()
                .all(|v| v["state"] == "reachable" || v["state"] == "disabled");
            print_result(json!({"machines":views,"complete":complete}));
            return Ok(i32::from(!complete));
        }
        "attach" => {
            let selector = rest
                .first()
                .context("attach needs a saved machine selector")?;
            let workspace = match &rest[1..] {
                [] => None,
                [flag, value] if flag == "--workspace" => Some(value.as_str()),
                _ => bail!("usage: crew machine attach ID|LABEL [--workspace NAME]"),
            };
            let catalog = store::load(&path)?;
            return attach(
                Some(catalog.select(selector, true)?),
                AttachTarget::Name(workspace),
            );
        }
        _ => bail!("unknown machine operation: {command}\n{HELP}"),
    }
    Ok(0)
}

fn status_options(args: &[String]) -> Result<(Option<String>, u64)> {
    let mut selector = None;
    let mut duration = transport::DEFAULT_TIMEOUT;
    let mut seen_timeout = false;
    let mut seen_json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" if !seen_json => {
                seen_json = true;
                i += 1;
            }
            "--timeout-ms" if !seen_timeout => {
                duration = timeout(args.get(i + 1).context("missing timeout-ms value")?)?;
                seen_timeout = true;
                i += 2;
            }
            item if !item.starts_with('-') && selector.is_none() => {
                selector = Some(item.into());
                i += 1;
            }
            item => bail!("unexpected or repeated option: {item}"),
        }
    }
    Ok((selector, duration))
}

fn federation_cli(args: &[String]) -> Result<i32> {
    if args
        .first()
        .is_some_and(|s| matches!(s.as_str(), "--help" | "-h" | "help"))
    {
        println!("{HELP}");
        return Ok(0);
    }
    let (sub, rest) = match args.first() {
        Some(sub) if !sub.starts_with('-') => (sub.as_str(), &args[1..]),
        _ => (
            if args.is_empty() && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
            {
                "watch"
            } else {
                "list"
            },
            args,
        ),
    };
    let (selector, duration) = status_options(rest)?;
    if selector.is_some() {
        bail!("federation queries all saved machines; use --machine for one machine");
    }
    if sub == "watch" {
        return dashboard::run(duration);
    }
    if !matches!(sub, "list" | "workspaces" | "agents") {
        bail!("unknown federation operation: {sub}");
    }
    let catalog = store::load(&store::path()?)?;
    let views = gather(catalog.machines, true, duration);
    let result = merge_views(views, sub);
    let reachable = result["machines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["state"] == "reachable");
    print_result(result);
    Ok(i32::from(!reachable))
}

pub(super) fn gather(machines: Vec<Machine>, local: bool, duration: u64) -> Vec<Value> {
    let mut threads = Vec::new();
    if local {
        threads.push(std::thread::spawn(move || {
            view(
                "local",
                "Local",
                true,
                transport::local(&[], true, duration),
            )
        }));
    }
    for machine in machines {
        threads.push(std::thread::spawn(move || {
            if !machine.enabled {
                return json!({"machine_id":machine.id,"label":machine.label,"state":"disabled"});
            }
            let result = transport::remote(&machine, &[], true, duration);
            view(&machine.id, &machine.label, false, result)
        }));
    }
    threads
        .into_iter()
        .map(|t| {
            t.join()
                .unwrap_or_else(|_| json!({"state":"error","error":"connection worker failed"}))
        })
        .collect()
}

fn view(id: &str, label: &str, local: bool, result: Result<Value>) -> Value {
    match result {
        Ok(data) => {
            json!({"machine_id":id,"label":label,"local":local,"state":"reachable","data":data})
        }
        Err(error) => {
            json!({"machine_id":id,"label":label,"local":local,"state":"unavailable","error":error.to_string()})
        }
    }
}

fn merge_views(views: Vec<Value>, sub: &str) -> Value {
    let mut workspaces = Vec::new();
    let mut agents = Vec::new();
    for view in &views {
        for (key, output) in [("workspaces", &mut workspaces), ("agents", &mut agents)] {
            if let Some(items) = view["data"][key].as_array() {
                for item in items {
                    let mut item = item.clone();
                    item["machine_id"] = view["machine_id"].clone();
                    item["machine_label"] = view["label"].clone();
                    item["machine_epoch"] = view["data"]["epoch"].clone();
                    output.push(item);
                }
            }
        }
    }
    let complete = views
        .iter()
        .all(|v| v["state"] == "reachable" || v["state"] == "disabled");
    let mut result = json!({"machines":views,"complete":complete});
    if sub != "agents" {
        result["workspaces"] = json!(workspaces);
    }
    if sub != "workspaces" {
        result["agents"] = json!(agents);
    }
    result
}

pub(super) enum AttachTarget<'a> {
    Name(Option<&'a str>),
    Snapshot { workspace: &'a str, epoch: &'a str },
}

pub(super) fn attach(machine: Option<&Machine>, target: AttachTarget<'_>) -> Result<i32> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("attach requires an interactive terminal");
    }
    let args = match target {
        AttachTarget::Name(Some(name)) => vec!["attach", "-t", name],
        AttachTarget::Name(None) => vec!["attach"],
        AttachTarget::Snapshot { workspace, epoch } => {
            vec!["__attach-workspace-epoch", workspace, epoch]
        }
    };
    let mut command = if let Some(machine) = machine {
        machine.validate()?;
        let mut command = transport::ssh(machine, transport::DEFAULT_TIMEOUT, true, true);
        let mut words = vec!["env".into()];
        if let Some(socket) = &machine.socket {
            words.push(format!("CREW_SOCKET={socket}"));
        }
        words.push(machine.binary.clone());
        words.extend(args.iter().map(|arg| (*arg).to_owned()));
        command.arg(format!(
            "exec {}",
            words
                .iter()
                .map(|s| transport::quote(s))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        command
    } else {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.args(args);
        command
    };
    // This uses inherited terminal I/O. Native SSH handles host-key and authentication prompts.
    let status = command.status().context("start terminal attachment")?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn federation_keeps_same_ids_scoped_and_unavailable_hosts_visible() {
        let views = vec![
            view(
                "local",
                "Local",
                true,
                Ok(
                    json!({"epoch":"local-epoch","workspaces":[{"workspace_id":"w1"}],"agents":[{"pane_id":"p1"}]}),
                ),
            ),
            view(
                "m1",
                "Build",
                false,
                Ok(
                    json!({"epoch":"remote-epoch","workspaces":[{"workspace_id":"w1"}],"agents":[{"pane_id":"p1"}]}),
                ),
            ),
            view("m2", "Offline", false, Err(anyhow::anyhow!("offline"))),
        ];
        let merged = merge_views(views, "list");
        assert_eq!(merged["workspaces"][0]["machine_id"], "local");
        assert_eq!(merged["workspaces"][1]["machine_id"], "m1");
        assert_eq!(merged["workspaces"][0]["machine_epoch"], "local-epoch");
        assert_eq!(merged["workspaces"][1]["machine_epoch"], "remote-epoch");
        assert_eq!(merged["agents"][1]["machine_epoch"], "remote-epoch");
        assert_eq!(merged["agents"].as_array().unwrap().len(), 2);
        assert_eq!(merged["machines"][2]["state"], "unavailable");
        assert_eq!(merged["complete"], false);
    }
    #[test]
    fn timeout_and_options_are_bounded() {
        for n in ["0", "249", "300001", "-1", "abc"] {
            assert!(timeout(n).is_err());
        }
        assert!(status_options(&strings(&["m1", "m2"])).is_err());
        assert!(status_options(&strings(&["--timeout-ms", "500", "--timeout-ms", "600"])).is_err());
        assert_eq!(
            status_options(&strings(&["m1", "--timeout-ms", "500"])).unwrap(),
            (Some("m1".into()), 500)
        );
    }
}
