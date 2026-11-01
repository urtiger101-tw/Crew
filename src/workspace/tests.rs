use super::*;
use crate::api::Request as ApiRequest;

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn powershell_startup_preserves_argv_and_literal_command() {
    let command = "$value = '引號 \" $literal';\nWrite-Output $value";
    let shell = format!(
        "{} -NoProfile -ExecutionPolicy RemoteSigned",
        crate::commands::quote(r"C:\Program Files\PowerShell\7\PWSH.EXE")
    );
    let argv = powershell_startup_shell(&shell, Some(command))
        .unwrap()
        .unwrap();
    assert_eq!(
        &argv[..4],
        &[
            r"C:\Program Files\PowerShell\7\PWSH.EXE",
            "-NoProfile",
            "-ExecutionPolicy",
            "RemoteSigned"
        ]
    );
    assert_eq!(&argv[4..], &["-NoExit", "-Command", command]);
    let argv = powershell_startup_shell("powershell.exe -NoExit -NoProfile", Some(command))
        .unwrap()
        .unwrap();
    assert_eq!(
        argv.iter().filter(|arg| arg.as_str() == "-NoExit").count(),
        1
    );
}

#[test]
fn powershell_startup_rejects_existing_execution_modes_and_scripts() {
    for mode in [
        "-Command echo old",
        "-c echo old",
        "-EncodedCommand AAAA",
        "-ec AAAA",
        "-EncodedArguments AAAA",
        "-File old.ps1",
        "-f old.ps1",
        "-CommandWithArgs echo old",
        "-ServerMode",
        "-SSHServerMode",
        "-SocketServerMode",
        "-so",
        "-NamedPipeServerMode",
        "-nam",
        "-V2SocketServerMode",
        "-v2so",
        "-Version",
        "-v",
        "-Help",
        "old.ps1",
        "-WorkingDirectory",
    ] {
        assert!(
            powershell_startup_shell(&format!("pwsh {mode}"), Some("echo new")).is_err(),
            "{mode}"
        );
    }
}

#[test]
fn powershell_startup_accepts_interactive_and_format_aliases() {
    for options in [
        "-i",
        "-in",
        "-if Text",
        "-of Text",
        "-i -if Text -of Text",
        "-ConfigurationFile 'config with spaces.pssc'",
    ] {
        let shell = format!("pwsh -NoProfile {options}");
        let original = crate::commands::tokenize(&shell);
        let argv = powershell_startup_shell(&shell, Some("Write-Output 'once'"))
            .unwrap()
            .unwrap();
        assert_eq!(&argv[..original.len()], &original);
        assert_eq!(
            &argv[original.len()..],
            &["-NoExit", "-Command", "Write-Output 'once'"]
        );
    }
}

#[test]
fn ordinary_shell_launches_do_not_gain_startup_argv() {
    for shell in [
        "powershell",
        "pwsh -NoProfile",
        "cmd.exe",
        "/bin/sh",
        "fake-powershell.exe",
    ] {
        assert!(powershell_startup_shell(shell, None).unwrap().is_none());
    }
    for shell in ["cmd.exe", "/bin/sh", "fake-powershell.exe"] {
        assert!(
            powershell_startup_shell(shell, Some("echo new"))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn create_keeps_commands_environment_and_no_focus_default() {
    let r = parse(
        "workspace",
        "create",
        &args(&[
            "研究.prod:api",
            "--cwd",
            ".",
            "--env",
            "TOKEN=one=two 中文",
            "--command",
            "echo 'hello world'",
        ]),
        None,
    )
    .unwrap();
    let ApiRequest::Workspace(Request::WorkspaceCreate {
        name,
        cwd,
        env,
        command,
        focus,
    }) = r
    else {
        panic!("wrong request")
    };
    assert_eq!(name.as_deref(), Some("研究.prod:api"));
    assert!(std::path::Path::new(&cwd.unwrap()).is_absolute());
    assert_eq!(env.get("TOKEN").map(String::as_str), Some("one=two 中文"));
    assert_eq!(command.as_deref(), Some("echo 'hello world'"));
    assert!(!focus);
}

#[test]
fn tab_create_keeps_caller_context_and_optional_scope() {
    let r = parse(
        "tab",
        "create",
        &args(&["review", "--workspace=w7", "--focus"]),
        Some(12),
    )
    .unwrap();
    let ApiRequest::Workspace(Request::TabCreate {
        workspace,
        current,
        focus,
        cwd,
        ..
    }) = r
    else {
        panic!("wrong request")
    };
    assert_eq!(workspace.as_deref(), Some("w7"));
    assert_eq!(current, Some(12));
    assert!(focus);
    assert_eq!(cwd, None);
}

#[test]
fn parser_rejects_conflicting_and_unknown_arguments() {
    for bad in [
        vec!["--focus", "--no-focus"],
        vec!["--cwd", ".", "--cwd", "."],
        vec!["a", "--name", "b"],
        vec!["--env", "A=1", "--env", "A=2"],
        vec!["--env", "broken"],
        vec!["--focus=yes"],
        vec!["--unknown"],
        vec!["--command", ""],
        vec!["--env", "crew_socket=wrong"],
    ] {
        assert!(
            parse("workspace", "create", &args(&bad), None).is_err(),
            "{bad:?}"
        );
    }
    assert!(parse("tab", "close", &args(&[]), None).is_err());
    assert!(parse("workspace", "list", &args(&["unexpected"]), None).is_err());
}

#[test]
fn names_cannot_shadow_stable_ids_but_keep_exact_punctuation() {
    for bad in ["", " ", "x\n", "w1", "w01", "name:prod", "a "] {
        assert!(valid_name(bad, 'w').is_err(), "{bad:?}");
    }
    for good in ["prod.api:two", "中文 專案", "w-1", "t1"] {
        assert!(valid_name(good, 'w').is_ok(), "{good:?}");
    }
    for bad in ["t0", "t003", "t8"] {
        assert!(valid_name(bad, 't').is_err());
    }
    assert_eq!(parse_id("w1", 'w'), Some(1));
    for bad in ["w0", "w01", "w-1", "w+1", "1", "t1", "w１"] {
        assert_eq!(parse_id(bad, 'w'), None);
    }
}

#[test]
fn environment_validation_is_portable_and_keeps_identity() {
    for key in [
        "CREW",
        "CREW_PANE_ID",
        "crew_socket",
        "TMUXLIKE",
        "TMUXLIKE_PANE_ID",
        "tmuxlike_socket",
        "A=B",
        "1X",
        "X Y",
        "",
    ] {
        assert!(
            validate_env(&BTreeMap::from([(key.to_string(), "value".to_string())])).is_err(),
            "{key}"
        );
    }
    assert!(validate_env(&BTreeMap::from([("KEY".into(), "nul\0byte".into())])).is_err());
    assert!(
        validate_env(&BTreeMap::from([
            ("KEY".into(), "line1\nline2".into()),
            ("_VALUE".into(), "中文".into())
        ]))
        .is_ok()
    );
}

#[test]
fn cwd_must_be_an_existing_directory() {
    assert!(checked_cwd(Some(file!().into())).is_err());
    assert!(
        checked_cwd(Some(
            "/this-directory-does-not-exist-crew-workspace-test".into()
        ))
        .is_err()
    );
    assert!(checked_cwd(Some(".".into())).unwrap().is_some());
}

#[test]
fn windows_env_override_replaces_inherited_key_regardless_of_case() {
    let base = BTreeMap::from([("Path".into(), "old".into())]);
    let overrides = BTreeMap::from([("PATH".into(), "new".into())]);
    let windows = merge_env_for(&base, overrides.clone(), true);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows["PATH"], "new");
    assert_eq!(merge_env_for(&base, overrides, false).len(), 2);
}

#[test]
fn move_parser_uses_insertion_slots_and_keeps_caller_scope() {
    let workspace = parse(
        "workspace",
        "move",
        &args(&["project.中文:api", "--index=3"]),
        Some(12),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(workspace).unwrap(),
        json!({"Workspace": {"WorkspaceMove": {
            "workspace": "project.中文:api", "insert_index": 3,
        }}})
    );
    let tab = parse(
        "tab",
        "move",
        &args(&["t7", "--index", "0", "--workspace", "w2"]),
        Some(12),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(tab).unwrap(),
        json!({"Workspace": {"TabMove": {
            "tab": "t7", "insert_index": 0, "workspace": "w2", "current": 12,
        }}})
    );
}

#[test]
fn move_parser_rejects_missing_negative_overflow_and_extra_arguments() {
    for group in ["workspace", "tab"] {
        for bad in [
            vec![],
            vec!["target"],
            vec!["target", "--index"],
            vec!["--index", "0"],
            vec!["target", "extra", "--index", "0"],
            vec!["target", "--index", "-1"],
            vec!["target", "--index", "+1"],
            vec!["target", "--index", "1.0"],
            vec!["target", "--index", "１"],
            vec!["target", "--index", "184467440737095516160"],
            vec!["target", "--index=0", "--index=1"],
            vec!["target", "--index=0", "--focus"],
            vec!["target", "--index=0", "--cwd", "."],
        ] {
            assert!(
                parse(group, "move", &args(&bad), None).is_err(),
                "{group} {bad:?}"
            );
        }
    }
    assert!(
        parse(
            "workspace",
            "move",
            &args(&["w1", "--index=0", "--workspace=w2"]),
            None
        )
        .is_err()
    );
    assert!(parse("workspace", "list", &args(&["--index=0"]), None).is_err());
}

#[test]
fn transfer_parser_keeps_source_scope_and_optional_destination_slot() {
    for (extra, index) in [(vec![], json!(null)), (vec!["--index=2"], json!(2))] {
        let mut argv = vec![
            "review.first:中文",
            "--workspace=w1",
            "--to-workspace",
            "project.destination:中文",
        ];
        argv.extend(extra);
        let req = parse("tab", "move", &args(&argv), Some(12)).unwrap();
        assert_eq!(
            serde_json::to_value(req).unwrap(),
            json!({"Workspace":{"TabTransfer":{
                "tab":"review.first:中文","workspace":"w1","to_workspace":"project.destination:中文",
                "insert_index":index,"current":12,
            }}})
        );
    }
    for bad in [
        vec!["t1", "--to-workspace"],
        vec!["t1", "--to-workspace="],
        vec!["t1", "--to-workspace=w1", "--to-workspace=w2"],
        vec!["t1", "--to-workspace=w1", "--index=-1"],
        vec!["t1", "--to-workspace=w1", "--index=184467440737095516160"],
        vec!["t1", "--to-workspace=w1", "--index=0", "--index=1"],
        vec!["t1", "extra", "--to-workspace=w1"],
        vec!["t1", "--to-workspace=w1", "--focus"],
    ] {
        assert!(parse("tab", "move", &args(&bad), None).is_err(), "{bad:?}");
    }
    assert!(
        parse(
            "workspace",
            "move",
            &args(&["w1", "--to-workspace=w2", "--index=0"]),
            None
        )
        .is_err()
    );
    assert!(parse("tab", "get", &args(&["t1", "--to-workspace=w2"]), None).is_err());
}
