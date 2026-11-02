use super::*;
use std::process::{ExitStatus, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(windows)]
fn status(code: u32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(code)
}
#[cfg(unix)]
fn status(code: u32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw((code << 8) as i32)
}
fn response(value: serde_json::Value) -> Output {
    Output {
        status: status(0),
        stdout: serde_json::to_vec(&value).unwrap(),
        stderr: vec![],
    }
}
fn test_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "crew-workflow-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
fn config() -> Config {
    serde_json::from_value(serde_json::json!({"repo":"owner/project","repo_path":".","state_path":"state.json","head":"feature","base":"main","title":"Change","pr_number":7,"required_checks":["test"],"ci":{"workflow":"ci.yml","ref":"feature","inputs":{"allow_hosted_minutes":"true"}}})).unwrap()
}

#[test]
fn requires_approval_and_success_on_current_head() {
    let dir = test_dir();
    let state_path = dir.join("state.json");
    let config = config();
    let mut state = State {
        config_hash: "test".into(),
        pr_number: Some(7),
        ..State::default()
    };
    let mut head = "one";
    let mut approved_head = "one";
    let mut check_success = false;
    for step in 0..3 {
        if step == 1 {
            check_success = true;
        }
        if step == 2 {
            head = "two";
            approved_head = "one";
        }
        let mut runner = |args: &[String], _: &Path, _: Option<&[u8]>, _: u64| -> Result<Output> {
            assert_eq!(args[0], "gh");
            assert_eq!(args[1], "api");
            let endpoint = &args[2];
            Ok(if endpoint.ends_with("/pulls/7") {
                response(
                    serde_json::json!({"number":7,"state":"open","user":{"login":"author"},"head":{"sha":head,"ref":"feature"},"base":{"ref":"main"},"requested_reviewers":[]}),
                )
            } else if endpoint.contains("/reviews?") {
                response(
                    serde_json::json!([[{"user":{"login":"author"},"state":"APPROVED","commit_id":head},{"user":{"login":"reviewer"},"state":"APPROVED","commit_id":approved_head}]]),
                )
            } else if endpoint.contains("/check-runs?") {
                response(
                    serde_json::json!([{"check_runs":[{"id":1,"name":"test","status":"completed","conclusion":if check_success {"success"} else {"failure"}}]}]),
                )
            } else if endpoint.contains("/statuses?") {
                response(serde_json::json!([[]]))
            } else {
                panic!("unexpected endpoint: {endpoint}")
            })
        };
        advance(
            &config,
            &dir,
            &state_path,
            &mut state,
            false,
            false,
            &mut runner,
        )
        .unwrap();
        match step {
            0 => assert!(state.status.contains("waiting for required checks")),
            1 => assert!(state.status.starts_with("ready:")),
            _ => assert!(state.status.contains("waiting for a non-author approval")),
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dispatch_requires_both_flags_and_is_not_repeated() {
    let dir = test_dir();
    let state_path = dir.join("state.json");
    let config = config();
    let mut state = State {
        config_hash: "test".into(),
        pr_number: Some(7),
        ..State::default()
    };
    let mut dispatches = 0;
    let mut runner = |args: &[String], _: &Path, input: Option<&[u8]>, _: u64| -> Result<Output> {
        if args.contains(&"POST".to_string()) {
            assert!(
                args.iter()
                    .any(|a| a.ends_with("actions/workflows/ci.yml/dispatches"))
            );
            let body: serde_json::Value = serde_json::from_slice(input.unwrap())?;
            assert_eq!(body["inputs"]["allow_hosted_minutes"], "true");
            dispatches += 1;
            return Ok(response(serde_json::json!({})));
        }
        let endpoint = &args[2];
        Ok(if endpoint.ends_with("/pulls/7") {
            response(
                serde_json::json!({"number":7,"state":"open","user":{"login":"author"},"head":{"sha":"one","ref":"feature"},"base":{"ref":"main"}}),
            )
        } else if endpoint.contains("/reviews?") {
            response(
                serde_json::json!([[{"user":{"login":"reviewer"},"state":"APPROVED","commit_id":"one"}]]),
            )
        } else if endpoint.contains("/check-runs?") {
            response(serde_json::json!([{"check_runs":[]}]))
        } else if endpoint.contains("/statuses?") {
            response(serde_json::json!([[]]))
        } else {
            panic!("unexpected endpoint: {endpoint}")
        })
    };
    advance(
        &config,
        &dir,
        &state_path,
        &mut state,
        true,
        false,
        &mut runner,
    )
    .unwrap();
    advance(
        &config,
        &dir,
        &state_path,
        &mut state,
        true,
        true,
        &mut runner,
    )
    .unwrap();
    advance(
        &config,
        &dir,
        &state_path,
        &mut state,
        true,
        true,
        &mut runner,
    )
    .unwrap();
    assert_eq!(dispatches, 1);
    assert_eq!(state.dispatched_for_head.as_deref(), Some("one"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_mutation_requires_recovery_before_retry() {
    let dir = test_dir();
    let state_path = dir.join("state.json");
    let mut config = config();
    config.reviewers = vec!["reviewer".into()];
    let mut state = State {
        config_hash: "test".into(),
        pr_number: Some(7),
        ..State::default()
    };
    let mut runner = |args: &[String], _: &Path, _: Option<&[u8]>, _: u64| -> Result<Output> {
        if args.contains(&"POST".to_string()) {
            return Ok(Output {
                status: status(1),
                stdout: vec![],
                stderr: b"timeout".to_vec(),
            });
        }
        Ok(if args[2].ends_with("/pulls/7") {
            response(
                serde_json::json!({"number":7,"state":"open","user":{"login":"author"},"head":{"sha":"one","ref":"feature"},"base":{"ref":"main"}}),
            )
        } else {
            response(serde_json::json!([[]]))
        })
    };
    assert!(
        advance(
            &config,
            &dir,
            &state_path,
            &mut state,
            true,
            false,
            &mut runner
        )
        .is_err()
    );
    let mut saved: State = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(saved.recovery_needed.as_deref(), Some("request reviewers"));
    assert!(
        advance(
            &config,
            &dir,
            &state_path,
            &mut saved,
            true,
            false,
            &mut runner
        )
        .is_err()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cli_uses_configured_adapter_and_refuses_concurrent_advance() {
    let dir = test_dir();
    let log = dir.join("calls.txt");
    #[cfg(windows)]
    let gh_command = {
        let script = dir.join("fake-gh.ps1");
        let path = log.to_string_lossy().replace('\'', "''");
        fs::write(&script, format!("[IO.File]::AppendAllText('{path}', [string]::Join(' ', $args)); [Console]::Out.Write('[]')")).unwrap();
        vec![
            "powershell.exe".to_string(),
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-File".into(),
            script.to_string_lossy().into_owned(),
        ]
    };
    #[cfg(unix)]
    let gh_command = {
        let script = dir.join("fake-gh.sh");
        let path = log.to_string_lossy().replace('\'', "'\\''");
        fs::write(
            &script,
            format!("printf '%s' \"$*\" >> '{path}'\nprintf '[]'\n"),
        )
        .unwrap();
        vec!["sh".to_string(), script.to_string_lossy().into_owned()]
    };
    let path = dir.join("config.json");
    fs::write(&path, serde_json::to_vec(&serde_json::json!({"repo":"owner/project","repo_path":".","state_path":"state.json","head":"feature","base":"main","title":"Change","required_checks":["test"],"gh_command":gh_command})).unwrap()).unwrap();
    let filename = path.to_string_lossy().into_owned();
    run(&["plan".into(), filename.clone()]).unwrap();
    assert!(!log.exists());
    run(&["advance".into(), filename.clone()]).unwrap();
    let args = fs::read_to_string(&log).unwrap();
    assert!(args.contains("pr list"));
    assert!(!args.contains("pr create"));
    fs::write(dir.join("state.json.lock"), b"other process").unwrap();
    assert!(run(&["advance".into(), filename]).is_err());
    assert_eq!(fs::read_to_string(&log).unwrap(), args);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn polling_waits_for_changes_requested_and_paged_statuses() {
    let dir = test_dir();
    let state_path = dir.join("state.json");
    let config = config();
    let mut state = State {
        config_hash: "test".into(),
        pr_number: Some(7),
        ..State::default()
    };
    let mut cycles = 0;
    let mut runner = |args: &[String], _: &Path, _: Option<&[u8]>, _: u64| -> Result<Output> {
        let endpoint = &args[2];
        Ok(if endpoint.ends_with("/pulls/7") {
            response(
                serde_json::json!({"number":7,"state":"open","user":{"login":"author"},"head":{"sha":"one","ref":"feature"},"base":{"ref":"main"}}),
            )
        } else if endpoint.contains("/reviews?") {
            cycles += 1;
            let reviews = match cycles {
                1 => serde_json::json!([]),
                2 => {
                    serde_json::json!([{"user":{"login":"A"},"state":"APPROVED","commit_id":"one"},{"user":{"login":"B"},"state":"CHANGES_REQUESTED","commit_id":"one"}])
                }
                _ => {
                    serde_json::json!([{"user":{"login":"A"},"state":"APPROVED","commit_id":"one"},{"user":{"login":"B"},"state":"APPROVED","commit_id":"one"}])
                }
            };
            response(serde_json::json!([reviews]))
        } else if endpoint.contains("/check-runs?") {
            response(serde_json::json!([{"check_runs":[]}]))
        } else if endpoint.contains("/statuses?") {
            assert!(
                args.contains(&"--paginate".to_string()) && args.contains(&"--slurp".to_string())
            );
            response(
                serde_json::json!([[{"id":1,"context":"test","state":"success"}], [{"id":2,"context":"test","state":if cycles >= 4 {"success"} else {"failure"}}]]),
            )
        } else {
            panic!("unexpected endpoint: {endpoint}")
        })
    };
    poll(
        &config,
        &dir,
        &state_path,
        &mut state,
        (false, false),
        (1, 2000),
        &mut runner,
    )
    .unwrap();
    assert_eq!(cycles, 4);
    assert!(state.status.starts_with("ready:"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn polling_timeout_preserves_pending_status() {
    let dir = test_dir();
    let mut state = State {
        config_hash: "test".into(),
        ..State::default()
    };
    let mut runner = |_: &[String], _: &Path, _: Option<&[u8]>, _: u64| -> Result<Output> {
        Ok(response(serde_json::json!([])))
    };
    let err = poll(
        &config(),
        &dir,
        &dir.join("state.json"),
        &mut state,
        (false, false),
        (1, 10),
        &mut runner,
    )
    .unwrap_err();
    assert!(err.to_string().contains("timed out"));
    assert!(!state.status.starts_with("ready:"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn help_and_config_validation_reject_missing_checks_and_config_as_state() {
    run(&["--help".into()]).unwrap();
    let dir = test_dir();
    let path = dir.join("config.json");
    let mut config = serde_json::json!({"repo":"owner/project","repo_path":".","state_path":"config.json","head":"feature","base":"main","title":"Change","required_checks":[]});
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let args = ["advance".into(), path.to_string_lossy().into_owned()];
    assert!(
        run(&args)
            .unwrap_err()
            .to_string()
            .contains("required check")
    );
    config["required_checks"] = serde_json::json!(["test"]);
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    assert!(run(&args).unwrap_err().to_string().contains("state_path"));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
        config
    );
    fs::remove_dir_all(dir).unwrap();
}
