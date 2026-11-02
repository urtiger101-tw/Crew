//! Parser tests: shared helpers plus the general tests (help, groups, option-free commands,
//! pane list and ids, option rules). Command groups live in the `tests/` submodules.
mod agent;
mod pane_io;
mod pane_ops;

use super::{HELP, parse};
use crate::api::Request;

fn v(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

fn run(xs: &[&str]) -> Result<Request, String> {
    parse(&v(xs), None)
}

fn run_at(xs: &[&str], current: Option<usize>) -> Result<Request, String> {
    parse(&v(xs), current)
}

fn is_err(xs: &[&str]) -> bool {
    run(xs).is_err()
}

#[test]
fn help_and_missing_subcommand_return_help_text() {
    assert_eq!(run(&["pane"]), Err(HELP.to_string()));
    assert_eq!(run(&["agent", "--help"]), Err(HELP.to_string()));
    assert_eq!(run(&["server", "-h"]), Err(HELP.to_string()));
    assert_eq!(run(&["session", "help"]), Err(HELP.to_string()));
    assert_eq!(run(&["pane", "list", "--help"]), Err(HELP.to_string()));
    assert_eq!(
        run_at(&["pane", "zoom", "--help"], Some(2)),
        Err(HELP.to_string())
    );
}

#[test]
fn help_flags_are_option_values_when_the_option_owns_them() {
    for value in ["-h", "--help"] {
        for group in ["workspace", "tab"] {
            let request = run(&[group, "create", "--command", value]).unwrap();
            let Request::Workspace(request) = request else {
                panic!("wrong request")
            };
            let command = match request {
                crate::workspace::Request::WorkspaceCreate { command, .. }
                | crate::workspace::Request::TabCreate { command, .. } => command,
                _ => panic!("wrong workspace request"),
            };
            assert_eq!(command.as_deref(), Some(value));
        }
        for (group, target) in [("pane", "p1"), ("workspace", "w1")] {
            let request = run(&[
                group,
                "report-metadata",
                target,
                "--source",
                "help-test",
                "--title",
                value,
            ])
            .unwrap();
            let Request::Metadata(request) = request else {
                panic!("wrong request")
            };
            assert_eq!(request.title, crate::metadata::TextPatch::set(value.into()));
        }
    }
    assert!(matches!(
        run(&["pane", "split", "--direction", "right", "--command", "-h"]),
        Ok(Request::PaneSplit { command: Some(command), .. }) if command == "-h"
    ));
    assert!(matches!(
        run(&["pane", "list", "--session", "-h"]),
        Ok(Request::PaneList { session: Some(session) }) if session == "-h"
    ));
    assert!(matches!(
        run(&["events", "list", "--epoch", "-h"]),
        Ok(Request::Events(crate::events::Request::Read { epoch: Some(epoch), .. })) if epoch == "-h"
    ));
    assert!(matches!(
        run(&["worktree", "open", "--branch", "-h"]),
        Ok(Request::Worktree(crate::worktree::Request::Open { branch: Some(branch), .. })) if branch == "-h"
    ));
}

#[test]
fn actual_help_flags_are_recognized_at_option_boundaries() {
    for words in [
        vec!["pane", "list", "--session", "-h", "--help"],
        vec!["workspace", "create", "--command", "-h", "--help"],
        vec!["tab", "create", "--command", "--help", "-h"],
        vec!["pane", "report-metadata", "p1", "--title", "-h", "--help"],
        vec!["worktree", "open", "--branch", "-h", "--help"],
        vec!["events", "list", "--epoch", "-h", "--help"],
        vec!["agent", "wait", "p1", "-h"],
    ] {
        assert_eq!(run(&words), Err(HELP.into()), "{words:?}");
    }
    assert!(matches!(
        run(&["pane", "send-text", "p1", "--", "--help"]),
        Ok(Request::PaneSendText { text, .. }) if text == "--help"
    ));
    assert!(matches!(
        run(&["agent", "prompt", "p1", "--", "-h"]),
        Ok(Request::AgentPrompt { text, .. }) if text == "-h"
    ));
}

#[test]
fn destructive_api_help_cannot_be_hidden_by_values_or_separator() {
    for command in [
        ["pane", "close"],
        ["workspace", "close"],
        ["tab", "close"],
        ["worktree", "remove"],
        ["server", "stop"],
    ] {
        for flag in ["--help", "-h"] {
            for prefix in ["--", "--path", "--workspace", "-n", "-s", "-x", "-y"] {
                let words = [command[0], command[1], prefix, flag];
                assert_eq!(run(&words), Err(HELP.into()), "{words:?}");
            }
        }
    }
}

#[test]
fn unknown_group_and_subcommand_are_usage_errors() {
    assert!(run(&["nope", "list"]).unwrap_err().contains("group"));
    let e = run(&["pane", "frob"]).unwrap_err();
    assert!(e.contains("frob") && e.contains("report-agent"), "{e}");
    assert!(run(&["agent", "frob"]).unwrap_err().contains("prompt"));
    assert!(is_err(&["api", "stop"]));
    assert!(is_err(&["server", "start"]));
}

#[test]
fn option_free_commands_parse() {
    assert_eq!(run(&["api", "snapshot"]), Ok(Request::ApiSnapshot));
    assert_eq!(run(&["session", "list"]), Ok(Request::SessionList));
    assert_eq!(
        run(&["session", "list", "--json"]),
        Ok(Request::SessionList)
    );
    assert_eq!(run(&["server", "stop"]), Ok(Request::ServerStop));
    assert_eq!(run(&["agent", "list"]), Ok(Request::AgentList));
    assert!(is_err(&["server", "stop", "now"]));
    assert!(is_err(&["api", "snapshot", "--json"]));
    assert!(is_err(&["agent", "list", "x"]));
}

#[test]
fn pane_list_session_option() {
    assert_eq!(
        run(&["pane", "list"]),
        Ok(Request::PaneList { session: None })
    );
    assert_eq!(
        run(&["pane", "list", "--session", "work"]),
        Ok(Request::PaneList {
            session: Some("work".into())
        })
    );
    assert_eq!(
        run(&["pane", "list", "--session=a=b=c"]),
        Ok(Request::PaneList {
            session: Some("a=b=c".into())
        })
    );
    assert!(is_err(&["pane", "list", "--session="]));
    assert!(is_err(&["pane", "list", "extra"]));
}

#[test]
fn single_id_commands_accept_p_and_percent_forms() {
    assert_eq!(
        run(&["pane", "get", "p12"]),
        Ok(Request::PaneGet { pane: 12 })
    );
    assert_eq!(
        run(&["pane", "get", "%7"]),
        Ok(Request::PaneGet { pane: 7 })
    );
    assert_eq!(
        run(&["pane", "close", "p3"]),
        Ok(Request::PaneClose { pane: 3 })
    );
    assert_eq!(
        run(&["pane", "focus", "p4"]),
        Ok(Request::PaneFocus { pane: 4 })
    );
}

#[test]
fn bad_pane_ids_and_extra_words_are_rejected() {
    for bad in ["p0", "w1:p1", "42", "p01", "p"] {
        assert!(is_err(&["pane", "get", bad]), "{bad}");
    }
    assert!(is_err(&["pane", "get"]));
    assert!(is_err(&["pane", "get", "p1", "p2"]));
    assert!(is_err(&["pane", "get", "p1", "--", "x"]));
    assert!(is_err(&["pane", "close", "p1", "--force"]));
}

#[test]
fn unknown_and_duplicate_options_are_rejected() {
    assert!(is_err(&["pane", "list", "--bogus"]));
    assert!(is_err(&["pane", "list", "-s"]));
    assert!(is_err(&["pane", "list", "--session"]));
    assert!(is_err(&["pane", "list", "--session", "--other"]));
    assert!(is_err(&[
        "pane",
        "list",
        "--session",
        "a",
        "--session",
        "b"
    ]));
    assert!(is_err(&["pane", "get", "p1", "--json"]));
    assert!(is_err(&["session", "list", "--json=1"]));
    assert!(run(&["agent", "wait", "p1", "--until", "idle", "--until", "done"]).is_ok());
}
