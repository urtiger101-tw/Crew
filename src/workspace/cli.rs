use super::Request;
use crate::api::Request as ApiRequest;
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
struct Args {
    positional: Vec<String>,
    values: BTreeMap<String, String>,
    env: BTreeMap<String, String>,
    focus: bool,
}

fn scan(args: &[String], allowed: &[&str]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut seen = HashSet::new();
    let mut i = 0;
    while i < args.len() {
        let item = &args[i];
        i += 1;
        if item == "--" {
            out.positional.extend(args[i..].iter().cloned());
            break;
        }
        if !item.starts_with('-') {
            out.positional.push(item.clone());
            continue;
        }
        let (key, inline) = item
            .split_once('=')
            .map_or((item.as_str(), None), |(k, v)| (k, Some(v)));
        if !allowed.contains(&key) {
            return Err(format!("unknown option: {key}"));
        }
        if key != "--env" && !seen.insert(key) {
            return Err(format!("duplicate option: {key}"));
        }
        if matches!(key, "--focus" | "--no-focus" | "--json") {
            if inline.is_some() {
                return Err(format!("{key} does not take a value"));
            }
            if key != "--json" {
                if seen.contains(if key == "--focus" {
                    "--no-focus"
                } else {
                    "--focus"
                }) {
                    return Err("--focus and --no-focus conflict".into());
                }
                out.focus = key == "--focus";
            }
            continue;
        }
        let value = match inline {
            Some(v) => v.to_string(),
            None => {
                let value = args
                    .get(i)
                    .ok_or_else(|| format!("missing value for {key}"))?
                    .clone();
                i += 1;
                value
            }
        };
        if key == "--env" {
            let (key, value) = value.split_once('=').ok_or("--env requires KEY=VALUE")?;
            if out.env.insert(key.to_string(), value.to_string()).is_some() {
                return Err(format!("duplicate environment key: {key}"));
            }
        } else {
            out.values.insert(key.to_string(), value);
        }
    }
    super::validate_env(&out.env).map_err(|e| e.message)?;
    Ok(out)
}

fn count(args: &Args, min: usize, max: usize, usage: &str) -> Result<(), String> {
    if (min..=max).contains(&args.positional.len()) {
        Ok(())
    } else {
        Err(format!("usage: crew {usage}"))
    }
}

fn caller_path(path: Option<String>, default_current: bool) -> Result<Option<String>, String> {
    let path = match path {
        Some(p) => std::path::PathBuf::from(p),
        None if default_current => {
            std::env::current_dir().map_err(|e| format!("current directory: {e}"))?
        }
        None => return Ok(None),
    };
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|e| format!("current directory: {e}"))?
            .join(path)
    };
    Ok(Some(absolute.to_string_lossy().into_owned()))
}

pub fn parse(
    group: &str,
    sub: &str,
    args: &[String],
    current: Option<usize>,
) -> Result<ApiRequest, String> {
    let workspace = group == "workspace";
    if !workspace && group != "tab" {
        return Err(format!("unknown group: {group}"));
    }
    let mut allowed = vec!["--json"];
    if !workspace {
        allowed.push("--workspace");
    }
    if sub == "create" {
        allowed.extend([
            "--name",
            "--cwd",
            "--env",
            "--command",
            "--focus",
            "--no-focus",
        ]);
    }
    if sub == "move" {
        allowed.push("--index");
        if !workspace {
            allowed.push("--to-workspace");
        }
    }
    let mut a = scan(args, &allowed)?;
    let scope = a.values.remove("--workspace");
    let req = match sub {
        "list" => {
            count(&a, 0, 0, &format!("{group} list"))?;
            if workspace {
                Request::WorkspaceList
            } else {
                Request::TabList {
                    workspace: scope,
                    current,
                }
            }
        }
        "get" | "current" => {
            count(
                &a,
                0,
                usize::from(sub == "get"),
                &format!("{group} {sub} [ID|NAME]"),
            )?;
            let target = a.positional.pop();
            if workspace {
                Request::WorkspaceGet {
                    workspace: target,
                    current,
                }
            } else {
                Request::TabGet {
                    tab: target,
                    workspace: scope,
                    current,
                }
            }
        }
        "create" => {
            count(&a, 0, 1, &format!("{group} create [NAME] [OPTIONS]"))?;
            let name = a.values.remove("--name");
            if name.is_some() && !a.positional.is_empty() {
                return Err("use either NAME or --name, not both".into());
            }
            let name = name.or_else(|| a.positional.pop());
            if let Some(name) = &name {
                super::valid_name(name, if workspace { 'w' } else { 't' })
                    .map_err(|e| e.message)?;
            }
            let cwd = caller_path(a.values.remove("--cwd"), workspace)?;
            let command = a.values.remove("--command");
            super::check_command(command.as_deref()).map_err(|e| e.message)?;
            if workspace {
                Request::WorkspaceCreate {
                    name,
                    cwd,
                    env: a.env,
                    command,
                    focus: a.focus,
                }
            } else {
                Request::TabCreate {
                    workspace: scope,
                    name,
                    cwd,
                    env: a.env,
                    command,
                    focus: a.focus,
                    current,
                }
            }
        }
        "rename" => {
            count(&a, 2, 2, &format!("{group} rename ID|NAME NEW_NAME"))?;
            let name = a.positional[1].clone();
            super::valid_name(&name, if workspace { 'w' } else { 't' }).map_err(|e| e.message)?;
            if workspace {
                Request::WorkspaceRename {
                    workspace: a.positional[0].clone(),
                    name,
                }
            } else {
                Request::TabRename {
                    tab: a.positional[0].clone(),
                    workspace: scope,
                    name,
                    current,
                }
            }
        }
        "move" => {
            count(&a, 1, 1, &format!("{group} move ID|NAME --index N"))?;
            let insert_index = a
                .values
                .remove("--index")
                .map(|raw| {
                    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
                        return Err("--index must be a nonnegative integer");
                    }
                    raw.parse::<usize>().map_err(|_| "--index is too large")
                })
                .transpose()?;
            let target = a.positional.remove(0);
            if let Some(to_workspace) = a.values.remove("--to-workspace") {
                if to_workspace.is_empty() {
                    return Err("--to-workspace requires a workspace ID or exact name".into());
                }
                Request::TabTransfer {
                    tab: target,
                    workspace: scope,
                    to_workspace,
                    insert_index,
                    current,
                }
            } else if workspace {
                Request::WorkspaceMove {
                    workspace: target,
                    insert_index: insert_index.ok_or("move requires --index N")?,
                }
            } else {
                Request::TabMove {
                    tab: target,
                    workspace: scope,
                    insert_index: insert_index.ok_or("move requires --index N")?,
                    current,
                }
            }
        }
        "focus" | "close" => {
            count(&a, 1, 1, &format!("{group} {sub} ID|NAME"))?;
            let target = a.positional.remove(0);
            match (workspace, sub) {
                (true, "focus") => Request::WorkspaceFocus { workspace: target },
                (true, _) => Request::WorkspaceClose { workspace: target },
                (false, "focus") => Request::TabFocus {
                    tab: target,
                    workspace: scope,
                    current,
                },
                (false, _) => Request::TabClose {
                    tab: target,
                    workspace: scope,
                    current,
                },
            }
        }
        _ => {
            return Err(format!(
                "unknown {group} subcommand: {sub}; expected list|get|current|create|rename|move|focus|close"
            ));
        }
    };
    Ok(ApiRequest::Workspace(req))
}
