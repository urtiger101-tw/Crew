//! Explicit, provider-independent hooks. These commands never install hooks into another
//! product's configuration, start an agent, or infer a running agent from a PATH entry.
//! `report` translates our documented events to the existing pane report/release API.
use crate::api::{ApiError, Reply, Request, Status, Target};
use crate::proto::{ClientMsg, Command, ServerMsg, read_msg, write_msg};
use anyhow::Result;
use interprocess::local_socket::prelude::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const HELP: &str = "Crew explicit integration hooks
  crew integration list [--json]
  crew integration doctor [AGENT] [--json]
  crew integration explain TARGET|--current [--json]
  crew integration report AGENT EVENT --source ID [--pane pN|--current]
                          [--seq N] [--message TEXT] [-- RESUME_ARG...]

Events: start/working -> working, blocked -> blocked, idle/done -> idle,
        unknown -> unknown, exit -> release-agent.
done becomes the server's done badge only when the completion is unseen.
Use one unique source ID per agent process and report exit before reusing it.
Omitted --pane uses CREW_PANE_ID, then TMUXLIKE_PANE_ID; it never guesses a pane.
Arguments after -- are saved as resume argv, not executed by this command.
explain distinguishes explicit reports from output-activity heuristics.
doctor checks executable files on PATH only; it does not prove hooks are installed.
This is Crew's generic event contract, not a Claude/Codex official hook format.";

struct Descriptor {
    id: &'static str,
    commands: &'static [&'static str],
}

/// Known executable names are diagnostic hints, not claims of installed integrations.
const REGISTRY: &[Descriptor] = &[
    Descriptor {
        id: "claude",
        commands: &["claude"],
    },
    Descriptor {
        id: "codex",
        commands: &["codex"],
    },
    Descriptor {
        id: "opencode",
        commands: &["opencode"],
    },
    Descriptor {
        id: "gemini",
        commands: &["gemini"],
    },
    Descriptor {
        id: "grok",
        commands: &["grok"],
    },
    Descriptor {
        id: "copilot",
        commands: &["copilot"],
    },
    Descriptor {
        id: "cursor",
        commands: &["cursor-agent", "agent"],
    },
    Descriptor {
        id: "pi",
        commands: &["pi"],
    },
    Descriptor {
        id: "omp",
        commands: &["omp"],
    },
    Descriptor {
        id: "mastracode",
        commands: &["mastracode"],
    },
    Descriptor {
        id: "kiro",
        commands: &["kiro-cli"],
    },
    Descriptor {
        id: "droid",
        commands: &["droid"],
    },
    Descriptor {
        id: "hermes",
        commands: &["hermes"],
    },
    Descriptor {
        id: "kilo",
        commands: &["kilo"],
    },
    Descriptor {
        id: "kimi",
        commands: &["kimi"],
    },
    Descriptor {
        id: "amp",
        commands: &["amp"],
    },
    Descriptor {
        id: "qwen",
        commands: &["qwen"],
    },
    Descriptor {
        id: "letta",
        commands: &["letta"],
    },
    Descriptor {
        id: "maki",
        commands: &["maki"],
    },
    Descriptor {
        id: "muse",
        commands: &["muse"],
    },
    Descriptor {
        id: "qodercli",
        commands: &["qodercli"],
    },
    Descriptor {
        id: "devin",
        commands: &["devin"],
    },
    Descriptor {
        id: "agy",
        commands: &["agy"],
    },
    Descriptor {
        id: "cline",
        commands: &["cline"],
    },
];

#[derive(Debug, PartialEq)]
enum Action {
    Help,
    List,
    Doctor(Option<String>),
    Explain(Target),
    Report(Request),
}

/// CLI entry point. The caller passes arguments after the `integration` group word.
pub fn run(args: &[String]) -> Result<i32> {
    let current = crate::brand::env_var("PANE_ID")
        .ok()
        .and_then(|v| crate::api::parse_pane_id(&v));
    let action = match parse(args, current) {
        Ok(action) => action,
        Err(message) => {
            eprintln!("crew integration: {message}");
            return Ok(2);
        }
    };
    match action {
        Action::Help => {
            println!("{HELP}");
            Ok(0)
        }
        Action::List => print_json(json!({
            "contract_version": 1,
            "delivery": "explicit-generic-hook",
            "automatic_installation": false,
            "automatic_detection": false,
            "agents": REGISTRY.iter().map(|d| json!({"agent": d.id, "executable_names": d.commands})).collect::<Vec<_>>(),
            "custom_agents": true,
            "events": {"start": "working", "working": "working", "blocked": "blocked", "idle": "idle", "done": "idle", "unknown": "unknown", "exit": "release"}
        })),
        Action::Doctor(agent) => {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
            let results: Vec<Value> = REGISTRY.iter().filter(|d| agent.as_ref().is_none_or(|id| id == d.id))
                .map(|d| {
                    let matches: Vec<Value> = d.commands.iter().filter_map(|command| find_executable(command, &dirs)
                        .map(|path| json!({"command": command, "path": path.to_string_lossy(), "path_lossy": path.to_str().is_none()}))).collect();
                    json!({"agent": d.id, "executable_found": !matches.is_empty(), "matches": matches,
                        "hook_status": "unverified", "evidence": "path-file-check-only", "agent_started": false})
                }).collect();
            print_json(json!({"agents": results, "server_probed": false,
                "current_pane": current.map(crate::api::pane_id_str),
                "socket": crate::ipc::base_name(),
                "note": "Executable presence is not running-agent detection or proof that a hook is installed. No commands were executed."}))
        }
        Action::Explain(target) => match query(Request::AgentGet { target }) {
            Ok(value) => {
                let pane = &value["agent"];
                let reported = pane["agent"].is_object();
                print_json(json!({
                    "pane": pane,
                    "classification_source": if reported { "explicit-report" } else { "output-activity-heuristic" },
                    "reported": reported,
                    "process_detection": false,
                    "evidence": if reported { "The server received report-agent from the displayed source; Crew has not independently verified the process identity." }
                        else { "No agent report exists. Status only reflects recent PTY output and unseen output; agent kind and task completion are unknown." }
                }))
            }
            Err(error) => {
                eprintln!("{}", json!({"error": error}));
                Ok(1)
            }
        },
        Action::Report(request) => crate::client::api(request),
    }
}

fn print_json(value: Value) -> Result<i32> {
    println!("{}", json!({"result": value}));
    Ok(0)
}

fn query(request: Request) -> std::result::Result<Value, ApiError> {
    let stream =
        crate::ipc::connect().map_err(|e| ApiError::new("server_not_running", e.to_string()))?;
    let (mut reader, mut writer) = stream.split();
    write_msg(&mut writer, &ClientMsg::Hello(Command::Api(request)))
        .map_err(|e| ApiError::new("transport_error", e.to_string()))?;
    match read_msg::<ServerMsg>(&mut reader)
        .map_err(|e| ApiError::new("transport_error", e.to_string()))?
    {
        ServerMsg::Api(Ok(Reply::Json(value))) => Ok(value),
        ServerMsg::Api(Err(error)) => Err(error),
        ServerMsg::Error(error) => Err(ApiError::new("server_error", error)),
        _ => Err(ApiError::new(
            "protocol_error",
            "expected a JSON API response",
        )),
    }
}

fn parse(args: &[String], current: Option<usize>) -> std::result::Result<Action, String> {
    let Some((sub, rest)) = args.split_first() else {
        return Ok(Action::Help);
    };
    if matches!(sub.as_str(), "help" | "--help" | "-h")
        || rest
            .first()
            .is_some_and(|s| matches!(s.as_str(), "--help" | "-h"))
    {
        return Ok(Action::Help);
    }
    match sub.as_str() {
        "list" if rest.is_empty() || rest == ["--json"] => Ok(Action::List),
        "doctor" => {
            let words: Vec<&str> = rest
                .iter()
                .map(String::as_str)
                .filter(|v| *v != "--json")
                .collect();
            if rest.iter().filter(|v| *v == "--json").count() > 1 || words.len() > 1 {
                return Err("usage: crew integration doctor [AGENT] [--json]".into());
            }
            let agent = words.first().copied();
            if agent.is_some_and(|id| !REGISTRY.iter().any(|d| d.id == id)) {
                return Err("unknown diagnostic agent; use integration list".into());
            }
            Ok(Action::Doctor(agent.map(String::from)))
        }
        "explain" => {
            let words: Vec<&str> = rest
                .iter()
                .map(String::as_str)
                .filter(|v| *v != "--json")
                .collect();
            if rest.iter().filter(|v| *v == "--json").count() > 1 || words.len() != 1 {
                return Err("usage: crew integration explain TARGET|--current [--json]".into());
            }
            let target = if words[0] == "--current" {
                Target::Pane(current.ok_or("--current requires CREW_PANE_ID or TMUXLIKE_PANE_ID")?)
            } else if let Some(pane) = crate::api::parse_pane_id(words[0]) {
                Target::Pane(pane)
            } else if crate::api::is_agent_name(words[0]) {
                Target::Name(words[0].into())
            } else {
                return Err("target must be a pane ID or live agent name".into());
            };
            Ok(Action::Explain(target))
        }
        "report" => parse_report(rest, current).map(Action::Report),
        _ => Err(format!(
            "unknown integration command or arguments: {sub}; use integration --help"
        )),
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
}

fn parse_report(args: &[String], current: Option<usize>) -> std::result::Result<Request, String> {
    let [agent, event, rest @ ..] = args else {
        return Err(
            "usage: crew integration report AGENT EVENT --source ID [--pane pN|--current]".into(),
        );
    };
    if !identifier(agent) {
        return Err(
            "AGENT must use 1..80 ASCII letters, digits, colon, dot, underscore or hyphen".into(),
        );
    }
    let state = match event.as_str() {
        "start" | "working" => Some(Status::Working),
        "blocked" => Some(Status::Blocked),
        "idle" | "done" => Some(Status::Idle),
        "unknown" => Some(Status::Unknown),
        "exit" => None,
        _ => {
            return Err(
                "EVENT must be start, working, blocked, idle, done, unknown or exit".into(),
            );
        }
    };
    let mut options = HashMap::new();
    let mut resume = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--" {
            resume = rest[i + 1..].to_vec();
            if resume.is_empty() {
                return Err("resume argv after -- must not be empty".into());
            }
            break;
        }
        let (key, inline) = rest[i]
            .split_once('=')
            .map_or((rest[i].as_str(), None), |(k, v)| (k, Some(v)));
        i += 1;
        if !matches!(
            key,
            "--source" | "--pane" | "--current" | "--seq" | "--message"
        ) {
            return Err(format!("unknown option: {key}"));
        }
        let value = if key == "--current" {
            if inline.is_some() {
                return Err("--current is a flag".into());
            }
            ""
        } else if let Some(value) = inline {
            value
        } else {
            let value = rest
                .get(i)
                .ok_or_else(|| format!("missing value for {key}"))?;
            i += 1;
            value
        };
        if options.insert(key, value).is_some() {
            return Err(format!("duplicate option: {key}"));
        }
    }
    if options.contains_key("--pane") && options.contains_key("--current") {
        return Err("use only one of --pane and --current".into());
    }
    let pane = if let Some(value) = options.get("--pane") {
        crate::api::parse_pane_id(value).ok_or("--pane must be a pane ID such as p1")?
    } else {
        current.ok_or("specify --pane or set CREW_PANE_ID; no pane is guessed")?
    };
    let source = *options
        .get("--source")
        .ok_or("--source is required; use one unique source ID per agent process")?;
    if !identifier(source) {
        return Err(
            "--source must use 1..80 ASCII letters, digits, colon, dot, underscore or hyphen"
                .into(),
        );
    }
    let seq = options
        .get("--seq")
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| "--seq must be a nonnegative integer".to_string())
        })
        .transpose()?;
    let message = options.get("--message").map(|value| (*value).to_owned());
    match state {
        Some(state) => Ok(Request::PaneReportAgent {
            pane,
            source: source.into(),
            agent: agent.clone(),
            state,
            message,
            seq,
            resume,
        }),
        None => {
            if message.is_some() || !resume.is_empty() {
                return Err("exit does not accept --message or resume argv".into());
            }
            Ok(Request::PaneReleaseAgent {
                pane,
                source: source.into(),
                agent: agent.clone(),
                seq,
            })
        }
    }
}

fn find_executable(command: &str, directories: &[PathBuf]) -> Option<PathBuf> {
    for dir in directories {
        let path = dir.join(command);
        if executable_file(&path) {
            return Some(absolute_path(path));
        }
        #[cfg(windows)]
        for extension in ["exe", "cmd", "bat", "com"] {
            let path = dir.join(format!("{command}.{extension}"));
            if executable_file(&path) {
                return Some(absolute_path(path));
            }
        }
    }
    None
}

fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map_or_else(|_| path.clone(), |cwd| cwd.join(&path))
    }
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|v| (*v).into()).collect()
    }

    #[test]
    fn event_contract_maps_without_running_an_agent() {
        for (event, expected) in [
            ("start", Status::Working),
            ("working", Status::Working),
            ("blocked", Status::Blocked),
            ("done", Status::Idle),
            ("idle", Status::Idle),
            ("unknown", Status::Unknown),
        ] {
            let request = parse_report(
                &args(&["custom-agent", event, "--source", "hook:run-1"]),
                Some(7),
            )
            .unwrap();
            assert!(
                matches!(request, Request::PaneReportAgent { pane: 7, state, .. } if state == expected)
            );
        }
        let request = parse_report(
            &args(&["custom-agent", "exit", "--source", "hook:run-1", "--seq=12"]),
            Some(7),
        )
        .unwrap();
        assert!(matches!(
            request,
            Request::PaneReleaseAgent {
                pane: 7,
                seq: Some(12),
                ..
            }
        ));
    }

    #[test]
    fn report_keeps_argv_and_message_literal() {
        let request = parse_report(
            &args(&[
                "codex",
                "start",
                "--pane",
                "p2",
                "--source",
                "runner-1",
                "--message",
                "中文 $x",
                "--",
                "codex",
                "resume",
                "a b;$x",
            ]),
            None,
        )
        .unwrap();
        assert!(
            matches!(request, Request::PaneReportAgent { pane: 2, message: Some(ref text), ref resume, .. } if text == "中文 $x" && resume == &vec!["codex".to_string(), "resume".to_string(), "a b;$x".to_string()])
        );
    }

    #[test]
    fn report_requires_explicit_context_and_validates_before_transport() {
        let valid = args(&["codex", "working", "--source", "run-1"]);
        assert!(parse_report(&valid, None).is_err());
        for words in [
            vec!["codex", "working"],
            vec!["codex", "success", "--source", "x"],
            vec!["codex", "working", "--source", "x y"],
            vec!["codex", "working", "--source", "x", "--pane", "p01"],
            vec![
                "codex",
                "working",
                "--source",
                "x",
                "--current",
                "--pane",
                "p1",
            ],
            vec!["codex", "working", "--source", "x", "--source", "y"],
            vec!["codex", "working", "--source", "x", "--seq", "-1"],
            vec!["codex", "working", "--source", "x", "--bad", "a"],
            vec!["codex", "working", "--source", "x", "--"],
            vec!["codex", "exit", "--source", "x", "--message", "no"],
        ] {
            assert!(parse_report(&args(&words), Some(1)).is_err(), "{words:?}");
        }
    }

    #[test]
    fn diagnostics_are_explicit_and_registry_is_unique() {
        let mut names = std::collections::HashSet::new();
        for d in REGISTRY {
            assert!(names.insert(d.id));
            assert!(!d.commands.is_empty());
        }
        assert_eq!(
            parse(&args(&["doctor", "codex", "--json"]), None).unwrap(),
            Action::Doctor(Some("codex".into()))
        );
        assert!(parse(&args(&["doctor", "made-up"]), None).is_err());
        assert!(parse(&args(&["explain", "--current"]), None).is_err());
        assert_eq!(
            parse(&args(&["explain", "--current"]), Some(3)).unwrap(),
            Action::Explain(Target::Pane(3))
        );
        assert_eq!(
            parse(&args(&["explain", "reviewer"]), None).unwrap(),
            Action::Explain(Target::Name("reviewer".into()))
        );
    }
}
