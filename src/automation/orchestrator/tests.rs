use super::*;
use router::Model;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "crew-scheduler-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn plan(&self) -> Plan {
        #[cfg(windows)]
        let command = [
            "powershell.exe",
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "[Console]::Out.Write('{\"usage\":{\"input_tokens\":2,\"output_tokens\":1},\"summary\":\"done\"}')",
        ];
        #[cfg(not(windows))]
        let command = [
            "sh",
            "-c",
            "printf '%s' '{\"usage\":{\"input_tokens\":2,\"output_tokens\":1},\"summary\":\"done\"}'",
        ];
        Plan {
            state_file: self.0.join("state.json"),
            max_parallel: 2,
            budget_microusd: 100,
            router: RouterConfig {
                command: vec![],
                timeout_ms: 10000,
                models: vec![Model {
                    id: "local-fixture".into(),
                    input_microusd_per_million: 1_000_000,
                    output_microusd_per_million: 1_000_000,
                }],
            },
            agents: ["a", "b"]
                .map(|id| Agent {
                    id: id.into(),
                    capabilities: vec!["code".into()],
                    command: command.iter().map(|s| (*s).to_owned()).collect(),
                })
                .to_vec(),
            tasks: ["first", "second"]
                .map(|id| Task {
                    id: id.into(),
                    capability: "code".into(),
                    prompt: "fixture".into(),
                    cwd: self.0.clone(),
                    write_scopes: vec![id.into()],
                    depends_on: vec![],
                    priority: 0,
                    input_tokens: 5,
                    output_tokens: 5,
                    timeout_ms: 10000,
                })
                .to_vec(),
        }
    }
    fn state(&self, plan: &Plan) -> State {
        State {
            plan_sha256: "fixture".into(),
            tasks: plan
                .tasks
                .iter()
                .map(|t| (t.id.clone(), TaskState::default()))
                .collect(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only files created by this test; no recursive cleanup.
        for name in [
            "state.json",
            "state.pending",
            "plan.json",
            "trace",
            "plan.json.lock",
            "state.json.lock",
            "alias.json",
            "state.pending",
        ] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[test]
fn two_plan_names_share_one_state_lock() {
    let f = Fixture::new();
    let plan = f.plan();
    let bytes = serde_json::to_vec(&plan).unwrap();
    let state_lock = PathBuf::from(format!("{}.lock", plan.state_file.display()));
    std::fs::write(&state_lock, b"other scheduler").unwrap();
    for name in ["plan.json", "alias.json"] {
        let path = f.0.join(name);
        std::fs::write(&path, &bytes).unwrap();
        let error = run(&[
            "run".into(),
            path.to_string_lossy().into_owned(),
            "--execute".into(),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("locked"));
    }
    assert!(!plan.state_file.exists());
}

#[test]
fn actual_usage_overrun_stops_later_dispatch() {
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.max_parallel = 1;
    plan.budget_microusd = 1;
    for task in &mut plan.tasks {
        task.input_tokens = 0;
        task.output_tokens = 0;
    }
    let mut state = f.state(&plan);
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert_eq!(state.tasks["first"].charged_microusd, 3);
    assert_eq!(state.tasks["second"].status, Status::Blocked);
    assert_eq!(total(&state).unwrap(), 3);
}

#[test]
fn state_save_preserves_existing_pending_sibling() {
    let f = Fixture::new();
    let plan = f.plan();
    let sentinel = plan.state_file.with_extension("pending");
    std::fs::write(&sentinel, b"user file").unwrap();
    save(&plan.state_file, &f.state(&plan)).unwrap();
    assert_eq!(std::fs::read(sentinel).unwrap(), b"user file");
}

#[test]
fn same_plan_bytes_in_different_directories_cannot_reuse_workspace_results() {
    let a = Fixture::new();
    let b = Fixture::new();
    let mut plan = a.plan();
    for task in &mut plan.tasks {
        task.cwd = ".".into();
    }
    let bytes = serde_json::to_vec(&plan).unwrap();
    let a_path = a.0.join("plan.json");
    let b_path = b.0.join("plan.json");
    std::fs::write(&a_path, &bytes).unwrap();
    std::fs::write(&b_path, &bytes).unwrap();
    let (_, state, path) = load(&a_path).unwrap();
    save(&path, &state).unwrap();
    assert!(
        load(&b_path)
            .unwrap_err()
            .to_string()
            .contains("plan changed")
    );
}

#[test]
fn dag_is_executed_and_finished_work_is_not_replayed() {
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.tasks[1].depends_on = vec!["first".into()];
    let mut state = f.state(&plan);
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert!(state.tasks.values().all(|t| t.status == Status::Succeeded));
    assert_eq!(total(&state).unwrap(), 6);
    assert!(
        state
            .tasks
            .values()
            .all(|t| t.usage.is_some() && t.reserved_microusd == 0)
    );
    // A broken command must not run for already finished work.
    plan.agents[0].command = vec!["nonexistent-fixture-command".into()];
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert_eq!(total(&state).unwrap(), 6);
}

#[test]
fn failure_blocks_dependencies_and_retains_unknown_usage_estimate() {
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.agents.truncate(1);
    plan.agents[0].command = vec!["nonexistent-fixture-command".into()];
    plan.tasks[1].depends_on = vec!["first".into()];
    let mut state = f.state(&plan);
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert_eq!(state.tasks["first"].status, Status::Failed);
    assert!(state.tasks["first"].usage.is_none());
    assert_eq!(state.tasks["first"].charged_microusd, 10);
    assert_eq!(state.tasks["second"].status, Status::Blocked);
}

#[test]
fn reservation_blocks_dispatch_above_budget() {
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.budget_microusd = 9;
    let mut state = f.state(&plan);
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert!(
        state
            .tasks
            .values()
            .all(|t| t.status == Status::Blocked && t.agent.is_none())
    );
    assert_eq!(total(&state).unwrap(), 0);
}

#[test]
fn cycles_unknown_dependencies_and_scope_escapes_are_rejected() {
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.tasks[0].depends_on = vec!["second".into()];
    plan.tasks[1].depends_on = vec!["first".into()];
    assert!(validate(&plan).is_err());
    plan.tasks[1].depends_on = vec!["unknown".into()];
    assert!(validate(&plan).is_err());
    plan.tasks[0].depends_on.clear();
    plan.tasks[1].depends_on.clear();
    plan.tasks[0].write_scopes = vec!["../escape".into()];
    assert!(validate(&plan).is_err());
}

#[test]
fn scopes_use_component_boundaries_and_prior_running_state_is_not_replayed() {
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.tasks[0].write_scopes = vec!["src".into()];
    plan.tasks[1].write_scopes = vec!["src/file.rs".into()];
    assert!(conflicts(&plan.tasks[0], &plan.tasks[1]));
    plan.tasks[1].write_scopes = vec!["src-other".into()];
    assert!(!conflicts(&plan.tasks[0], &plan.tasks[1]));
    let mut state = f.state(&plan);
    state.tasks.get_mut("first").unwrap().status = Status::Running;
    assert!(
        execute(&plan, &mut state, &plan.state_file)
            .unwrap_err()
            .to_string()
            .contains("interrupted")
    );
}

#[test]
fn plan_hash_prevents_reusing_old_results_and_run_requires_execution_flag() {
    let f = Fixture::new();
    let plan = f.plan();
    let path = f.0.join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let (_, state, state_path) = load(&path).unwrap();
    save(&state_path, &state).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.push(b'\n');
    std::fs::write(&path, bytes).unwrap();
    assert!(
        load(&path)
            .unwrap_err()
            .to_string()
            .contains("plan changed")
    );
    assert!(
        run(&["run".into(), path.to_string_lossy().into_owned()])
            .unwrap_err()
            .to_string()
            .contains("--execute")
    );
}

#[test]
fn priority_decides_which_task_gets_a_limited_budget() {
    let f = Fixture::new();
    let mut plan = f.plan();
    plan.max_parallel = 1;
    plan.budget_microusd = 10;
    plan.tasks[1].priority = 10;
    let mut state = f.state(&plan);
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert_eq!(state.tasks["second"].status, Status::Succeeded);
    assert_eq!(state.tasks["first"].status, Status::Blocked);
}

#[test]
#[cfg(windows)]
fn parallel_limit_and_conflicting_scopes_are_enforced_during_real_execution() {
    let f = Fixture::new();
    let mut plan = f.plan();
    let script = "$s=Get-Content state.json -Raw|ConvertFrom-Json; $n=@($s.tasks.psobject.Properties.Value | Where-Object status -eq running).Count; Start-Sleep -Milliseconds 200; [Console]::Out.Write('{\"usage\":{\"input_tokens\":2,\"output_tokens\":1},\"summary\":\"' + $n + '\"}')";
    for agent in &mut plan.agents {
        agent.command[4] = script.into();
    }
    let mut state = f.state(&plan);
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert!(state.tasks.values().all(|t| t.status == Status::Succeeded));
    assert!(state.tasks.values().any(|t| t.note == "2"));
    plan.max_parallel = 1;
    let mut state = f.state(&plan);
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert!(state.tasks.values().all(|t| t.note == "1"));
    plan.max_parallel = 2;
    plan.tasks[1].write_scopes = plan.tasks[0].write_scopes.clone();
    let mut state = f.state(&plan);
    execute(&plan, &mut state, &plan.state_file).unwrap();
    assert!(state.tasks.values().all(|t| t.note == "1"));
}
