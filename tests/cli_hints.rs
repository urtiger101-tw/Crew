//! Human-first CLI errors: short typo hints, a discoverable doctor, and a next step when no
//! server is running. Every run uses an isolated, unused socket and the English catalog.
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Case {
    root: PathBuf,
    socket: String,
}

impl Case {
    fn new(label: &str) -> Self {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("crew-hints-{label}-{suffix}"));
        std::fs::create_dir(&root).unwrap();
        Self {
            root,
            socket: format!("crew-hints-{label}-{suffix}.sock"),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crew"));
        for (key, _) in std::env::vars_os() {
            let upper = key.to_string_lossy().to_ascii_uppercase();
            if upper.starts_with("CREW_")
                || upper == "CREW"
                || upper.starts_with("TMUXLIKE")
                || matches!(
                    upper.as_str(),
                    "LC_ALL" | "LC_MESSAGES" | "LANG" | "LANGUAGE"
                )
            {
                command.env_remove(key);
            }
        }
        command
            .args(args)
            .current_dir(&self.root)
            .env("CREW_SOCKET", &self.socket)
            .env("CREW_CONFIG", self.root.join("unused-config.toml"))
            .env("CREW_STATE", self.root.join("unused-state.json"))
            .env("CREW_PLUGIN_HOME", self.root.join("unused-plugins"))
            .env("CREW_LANG", "en")
            .env("PATH", "")
            .output()
            .unwrap()
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn typo_gets_suggestion_not_full_help() {
    let case = Case::new("typo");
    let output = case.run(&["spilt-window"]);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{err}");
    assert!(err.contains("unknown command: spilt-window"), "{err}");
    assert!(err.contains("Did you mean 'split-window'?"), "{err}");
    assert!(
        err.contains("Run `crew --help` for the command list."),
        "{err}"
    );
    assert!(err.lines().count() <= 4, "{err}");
    assert!(!err.contains("USAGE:"), "{err}");
    assert!(output.stdout.is_empty());
}

#[test]
fn unrelated_word_has_no_suggestion() {
    let case = Case::new("unrelated");
    let output = case.run(&["frobnicate"]);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{err}");
    assert!(err.contains("unknown command: frobnicate"), "{err}");
    assert!(!err.contains("Did you mean"), "{err}");
    assert!(err.contains("crew --help"), "{err}");
    assert!(err.lines().count() <= 3, "{err}");
    assert!(!err.contains("USAGE:"), "{err}");
}

#[test]
fn known_word_with_bad_arguments_does_not_suggest_itself() {
    let case = Case::new("self");
    let output = case.run(&["server", "extra"]);
    let err = stderr(&output);
    assert_ne!(output.status.code(), Some(0), "{err}");
    assert!(!err.contains("Did you mean 'server'"), "{err}");
}

#[test]
fn usage_error_prints_only_the_error_and_the_help_pointer() {
    let case = Case::new("usage");
    let output = case.run(&["last-pane", "extra"]);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{err}");
    assert!(err.contains("usage: last-pane"), "{err}");
    assert!(!err.contains("Did you mean"), "{err}");
    assert!(err.contains("crew --help"), "{err}");
    assert_eq!(err.lines().count(), 2, "{err}");
}

#[test]
fn help_lists_doctor() {
    let case = Case::new("help");
    let output = case.run(&["--help"]);
    assert!(output.status.success());
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(out.contains("crew doctor [--json]"), "{out}");
}

#[test]
fn no_server_error_names_next_step() {
    let case = Case::new("noserver");
    let output = case.run(&["ls"]);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{err}");
    assert!(err.contains("no server running ("), "{err}");
    assert!(err.contains("crew new -s NAME"), "{err}");
}
