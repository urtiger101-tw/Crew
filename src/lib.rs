/// Append a line to $CREW_LOG (if set). Used to diagnose the detached server.
#[macro_export]
macro_rules! log {
    ($($t:tt)*) => {{
        if let Some(p) = $crate::brand::env_os("LOG") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "[{:?}] {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0), format!($($t)*));
            }
        }
    }};
}

mod agent_detection;
mod api;
mod brand;
mod bridge;
mod client;
mod commands;
mod config;
mod copy;
mod events;
mod i18n;
mod input;
mod integrations;
mod ipc;
mod keys;
mod layout;
mod metadata;
#[cfg(windows)]
mod pane_utf8;
mod persist;
mod plugins;
mod process_path;
mod proto;
mod remote;
mod render;
mod server;
mod theme;
mod workspace;
mod worktree;

use i18n::tr;
use proto::Command;

fn flag(args: &[String], f: &str) -> Option<String> {
    args.iter()
        .position(|a| a == f)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Resolve CLI help before any command can attach, start a server or mutate state.
fn help_args(args: &[String]) -> Option<Vec<String>> {
    let sub = args.first()?.as_str();
    if sub.starts_with("__") {
        return None;
    }
    let split = matches!(sub, "split-window" | "splitw");
    let binding = matches!(sub, "bind-key" | "bind");
    let confirm = matches!(sub, "confirm-before" | "confirm");
    let mut tail = args.iter().skip(1);
    while let Some(arg) = tail.next() {
        if arg == "--" {
            break;
        }
        if matches!(
            arg.as_str(),
            "--command"
                | "--message"
                | "--source"
                | "--pane"
                | "--seq"
                | "--label"
                | "--binary"
                | "--socket"
                | "--ssh-config"
                | "--workspace"
                | "--timeout-ms"
                | "--ref"
                | "--plugin"
                | "--limit"
                | "--prompt"
                | "--lang"
                | "-t"
                | "-s"
                | "-x"
                | "-y"
        ) || (!binding && arg == "-n")
            || (confirm && arg == "-p")
        {
            tail.next(); // option values may be literal help flags.
            continue;
        }
        if arg == "--help" || (arg == "-h" && !split) {
            let group = if sub == "--machine" {
                Some("machine")
            } else if api::cli::API_COMMANDS.contains(&sub)
                || matches!(sub, "plugin" | "integration" | "machine" | "federation")
            {
                Some(sub)
            } else {
                None
            };
            return Some(
                group
                    .into_iter()
                    .chain(std::iter::once("--help"))
                    .map(str::to_owned)
                    .collect(),
            );
        }
        if (binding || confirm) && !arg.starts_with('-') {
            break; // the binding key or wrapped command begins the payload.
        }
    }
    None
}

pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = help_args(&args).unwrap_or(args);
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    let lang = client::lang();
    let res = match sub {
        #[cfg(windows)]
        "__pane_utf8" => {
            if std::env::var_os("CREW_PANE_ID").is_none() {
                Err(anyhow::anyhow!(
                    "internal pane bootstrap requires a Crew pane"
                ))
            } else {
                match pane_utf8::run(std::env::args_os().skip(2)) {
                    Ok(code) => std::process::exit(code),
                    Err(e) => Err(e),
                }
            }
        }
        "__server" => server::run(),
        "__remote-api" => remote::transport_serve(),
        "machine" | "federation" | "--machine" => match remote::run(&args) {
            Ok(code) => std::process::exit(code),
            Err(e) => Err(e),
        },
        "server" if args.len() == 1 => server::run(),
        "integration" => match integrations::run(&args[1..]) {
            Ok(code) => std::process::exit(code),
            Err(e) => Err(e),
        },
        "plugin" => match plugins::run(&args[1..]) {
            Ok(code) => std::process::exit(code),
            Err(e) => Err(e),
        },
        "api" if args.get(1).is_some_and(|s| s == "serve") => {
            if args.len() != 2 {
                Err(anyhow::anyhow!("usage: crew api serve"))
            } else {
                bridge::serve()
            }
        }
        "api" if args.get(1).is_some_and(|s| s == "schema") => {
            if args.len() != 2 {
                Err(anyhow::anyhow!("usage: crew api schema"))
            } else {
                println!("{}", bridge::schema());
                Ok(())
            }
        }
        g if api::cli::API_COMMANDS.contains(&g) => {
            let current = crate::brand::env_var("PANE_ID")
                .ok()
                .and_then(|v| api::parse_pane_id(&v));
            match api::cli::parse(&args, current) {
                Ok(req) => match client::api(req) {
                    Ok(code) => std::process::exit(code),
                    Err(e) => Err(e),
                },
                Err(help) if help == api::cli::HELP => {
                    println!("{}", tr(lang, "cli.api_help"));
                    println!(
                        "\n{}\n{}\n{}\n{}",
                        workspace::HELP,
                        worktree::HELP,
                        bridge::HELP,
                        metadata::HELP
                    );
                    Ok(())
                }
                Err(usage) => {
                    eprintln!("crew: {usage}");
                    std::process::exit(2);
                }
            }
        }
        "" => client::attach(None, false).or_else(|_| client::attach(None, true)),
        "new" | "new-session" if args.iter().any(|a| a == "-d") => {
            client::new_detached(flag(&args, "-s"))
        }
        "new" | "new-session" => client::attach(flag(&args, "-s"), true),
        "attach" | "a" | "attach-session" => client::attach(flag(&args, "-t"), false),
        "attach-workspace" if args.len() == 2 => client::attach_workspace(&args[1]),
        "attach-workspace" => Err(anyhow::anyhow!("usage: crew attach-workspace wN")),
        "__attach-workspace-epoch" if args.len() == 3 => {
            client::attach_workspace_epoch(&args[1], &args[2])
        }
        "__attach-workspace-epoch" => Err(anyhow::anyhow!(
            "usage: crew __attach-workspace-epoch wN EXPECTED_EPOCH"
        )),
        "ls" | "list-sessions" => client::simple(Command::List),
        "kill-session" => match flag(&args, "-t") {
            Some(s) => client::simple(Command::Kill { session: s }),
            None => Err(anyhow::anyhow!("usage: crew kill-session -t NAME")),
        },
        "kill-server" => client::simple(Command::KillServer),
        "keys" | "list-keys" => {
            println!("{}", tr(lang, "cli.keys"));
            Ok(())
        }
        "-V" | "--version" | "version" => {
            println!("crew {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "-h" | "--help" | "help" => {
            println!("{}", tr(lang, "cli.help"));
            println!(
                "\ncrew workspace|tab|worktree|events|integration|plugin --help\n{}",
                bridge::HELP
            );
            println!("\n{}", remote::HELP);
            Ok(())
        }
        _ => {
            // any other tmux-style command is sent to the running server (e.g. source-file, set, bind-key)
            let line = args
                .iter()
                .map(|a| commands::quote(a))
                .collect::<Vec<_>>()
                .join(" ");
            match commands::parse(&line) {
                Ok(_) => client::simple(Command::Run { line }),
                Err(e) => Err(anyhow::anyhow!("{e}\n\n{}", tr(lang, "cli.help"))),
            }
        }
    };
    if let Err(e) = res {
        eprintln!("crew: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use crate::i18n::{Lang, tr};

    const HELP_KEYS: [&str; 3] = ["cli.help", "cli.keys", "cli.api_help"];

    #[test]
    fn help_respects_split_direction_and_command_payloads() {
        for args in [
            vec!["split-window", "-h"],
            vec!["splitw", "--command", "--help"],
            vec!["new-window", "--command", "-h"],
            vec!["pane", "send-text", "p1", "--", "--help"],
            vec![
                "integration",
                "report",
                "codex",
                "start",
                "--",
                "codex",
                "--help",
            ],
            vec!["bind-key", "v", "split-window", "-h"],
            vec!["confirm-before", "-p", "--help", "split-window", "-h"],
            vec!["__pane_utf8", "powershell.exe", "--help"],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
            assert_eq!(super::help_args(&args), None, "{args:?}");
        }
        let args = ["split-window", "-h", "--help"].map(str::to_owned);
        assert_eq!(super::help_args(&args), Some(vec!["--help".into()]));
        for flag in ["--help", "-h"] {
            for args in [
                vec![
                    "integration",
                    "report",
                    "codex",
                    "start",
                    "--source",
                    "s",
                    "--pane",
                    "p1",
                    "--message",
                    flag,
                ],
                vec!["machine", "rename", "m1", "--label", flag],
                vec!["new", "-d", "-s", flag],
                vec!["new-window", "-n", flag],
                vec!["debug-frame", "--message", flag],
            ] {
                let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
                assert_eq!(super::help_args(&args), None, "{args:?}");
            }
        }
    }

    /// Command, option and key names that the help texts must keep in zh-TW.
    const NAMES: [&str; 14] = [
        "kill-server",
        "kill-session",
        "send-keys",
        "capture-pane",
        "source-file",
        "bind-key",
        "send-text",
        "wait-output",
        "report-agent",
        "release-agent",
        "pane-titles",
        "confirm-close",
        "Ctrl-b",
        "--until",
    ];

    /// Flags such as `-d`, `-t`, `--direction`: tokens that start with `-` and a letter.
    fn flags(text: &str) -> Vec<&str> {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|w| {
                w.starts_with('-')
                    && w.trim_start_matches('-')
                        .starts_with(|c: char| c.is_ascii_alphabetic())
            })
            .collect()
    }

    /// Usage lines (`crew new [-d] [-s NAME]`, `pane split [TARGET] ...`): the command
    /// words (`crew new`, `pane split`) are never translated. The first line is a title;
    /// prose lines (ending in a period) are not usage lines.
    fn usage_heads(text: &str) -> Vec<String> {
        const GROUPS: [&str; 6] = ["crew ", "pane ", "agent ", "api ", "session ", "server "];
        text.lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| !l.ends_with('.') && GROUPS.iter().any(|g| l.starts_with(g)))
            .map(|l| l.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
            .collect()
    }

    #[test]
    fn help_and_keys_are_non_empty_and_translated() {
        for key in HELP_KEYS {
            let (en, zh) = (tr(Lang::En, key), tr(Lang::ZhTw, key));
            assert!(!en.is_empty() && !zh.is_empty(), "{key}: empty text");
            assert_ne!(en, zh, "{key}: zh-TW equals English");
        }
    }

    #[test]
    fn english_help_keeps_its_original_first_lines() {
        assert!(
            tr(Lang::En, "cli.help").starts_with("crew - a cross-platform terminal multiplexer")
        );
        assert!(tr(Lang::En, "cli.keys").starts_with("Prefix: Ctrl-b (configurable), then:"));
        assert!(
            tr(Lang::En, "cli.api_help")
                .starts_with("crew pane|agent|api|session|server  (herdr-style")
        );
    }

    #[test]
    fn zh_tw_help_uses_glossary_terms() {
        let main_help = tr(Lang::ZhTw, "cli.help");
        assert!(main_help.contains("工作階段") && main_help.contains("窗格"));
        let api_help = tr(Lang::ZhTw, "cli.api_help");
        assert!(api_help.contains("工作階段") && api_help.contains("窗格"));
        assert!(tr(Lang::ZhTw, "cli.keys").contains("窗格"));
    }

    #[test]
    fn command_names_and_flags_survive_translation() {
        for key in HELP_KEYS {
            let (en, zh) = (tr(Lang::En, key), tr(Lang::ZhTw, key));
            for head in usage_heads(en) {
                assert!(
                    zh.contains(head.as_str()),
                    "{key}: zh-TW lost usage '{head}'"
                );
            }
            for flag in flags(en) {
                assert!(zh.contains(flag), "{key}: zh-TW lost flag '{flag}'");
            }
        }
    }

    #[test]
    fn named_commands_and_keys_survive_translation() {
        let all_en: String = HELP_KEYS.iter().map(|k| tr(Lang::En, k)).collect();
        let all_zh: String = HELP_KEYS.iter().map(|k| tr(Lang::ZhTw, k)).collect();
        for name in NAMES {
            assert!(all_en.contains(name), "{name} is not in the English help");
            assert!(all_zh.contains(name), "zh-TW lost {name}");
        }
        assert!(
            tr(Lang::ZhTw, "cli.api_help").contains("pane split [TARGET] --direction right|down")
        );
    }
}
