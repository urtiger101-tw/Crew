use super::{process, router};
use anyhow::{Context, Result, bail, ensure};
use router::{RouteDecision, RouteRequest, RouterConfig, Usage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub state_file: PathBuf,
    pub max_parallel: usize,
    pub budget_microusd: u64,
    pub router: RouterConfig,
    pub agents: Vec<Agent>,
    pub tasks: Vec<Task>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    pub capabilities: Vec<String>,
    /// Exact argv placeholders: {model}, {prompt}, {task_id}.
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub capability: String,
    pub prompt: String,
    pub cwd: PathBuf,
    pub write_scopes: Vec<PathBuf>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub priority: i32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Running,
    Succeeded,
    Failed,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TaskState {
    pub status: Status,
    pub agent: Option<String>,
    pub route: Option<RouteDecision>,
    pub reserved_microusd: u64,
    pub charged_microusd: u64,
    pub usage: Option<Usage>,
    pub note: String,
}

impl Default for TaskState {
    fn default() -> Self {
        Self {
            status: Status::Pending,
            agent: None,
            route: None,
            reserved_microusd: 0,
            charged_microusd: 0,
            usage: None,
            note: String::new(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct State {
    pub plan_sha256: String,
    pub tasks: BTreeMap<String, TaskState>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskOutput {
    usage: Usage,
    summary: String,
}

struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn save(path: &Path, state: &State) -> Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(state)?;
    let temp = PathBuf::from(format!(
        "{}.{}-{}.pending",
        path.display(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    let result = file.write_all(&bytes).and_then(|()| {
        drop(file);
        std::fs::rename(&temp, path)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.context("replace automation state")
}

fn load(path: &Path) -> Result<(Plan, State, PathBuf)> {
    let path = std::fs::canonicalize(path).context("open automation plan")?;
    let bytes = std::fs::read(&path)?;
    let mut plan: Plan = serde_json::from_slice(&bytes).context("invalid orchestrator plan")?;
    let base = path.parent().context("plan directory")?;
    if !plan.state_file.is_absolute() {
        plan.state_file = base.join(&plan.state_file);
    }
    plan.state_file = if plan.state_file.exists() {
        std::fs::canonicalize(&plan.state_file)?
    } else {
        let parent = std::fs::canonicalize(plan.state_file.parent().context("state_file parent")?)
            .context("state_file parent must exist")?;
        parent.join(
            plan.state_file
                .file_name()
                .context("state_file needs a filename")?,
        )
    };
    ensure!(
        plan.state_file != path,
        "state_file must differ from the plan"
    );
    for task in &mut plan.tasks {
        task.cwd = std::fs::canonicalize(base.join(&task.cwd))
            .with_context(|| format!("task {} cwd", task.id))?;
        ensure!(task.cwd.is_dir(), "task {} cwd is not a directory", task.id);
    }
    validate(&plan)?;
    // Preserve exact-config identity and the directories those bytes resolve to.
    let hash = format!(
        "{:x}",
        Sha256::new()
            .chain_update(&bytes)
            .chain_update(serde_json::to_vec(&plan)?)
            .finalize()
    );
    let state = load_state(&plan, &hash)?;
    let state_path = plan.state_file.clone();
    Ok((plan, state, state_path))
}

fn load_state(plan: &Plan, hash: &str) -> Result<State> {
    Ok(if plan.state_file.exists() {
        let state: State = serde_json::from_slice(&std::fs::read(&plan.state_file)?)
            .context("invalid orchestrator state; inspect instead of rerunning tasks")?;
        ensure!(
            state.plan_sha256 == hash,
            "plan changed; use a new state_file or restore the original plan"
        );
        ensure!(
            state.tasks.keys().eq(plan
                .tasks
                .iter()
                .map(|t| &t.id)
                .collect::<BTreeSet<_>>()
                .into_iter()),
            "state task IDs differ from plan"
        );
        state
    } else {
        State {
            plan_sha256: hash.into(),
            tasks: plan
                .tasks
                .iter()
                .map(|t| (t.id.clone(), TaskState::default()))
                .collect(),
        }
    })
}

fn validate(plan: &Plan) -> Result<()> {
    ensure!(
        plan.max_parallel > 0 && plan.max_parallel <= 64,
        "max_parallel must be 1..64"
    );
    ensure!(!plan.tasks.is_empty(), "plan requires tasks");
    router::validate(&plan.router)?;
    let mut agents = BTreeSet::new();
    for agent in &plan.agents {
        ensure!(
            !agent.id.trim().is_empty() && agents.insert(&agent.id),
            "agent IDs must be unique and nonempty"
        );
        ensure!(
            agent.command.first().is_some_and(|s| !s.trim().is_empty()),
            "agent {} command is empty",
            agent.id
        );
    }
    let mut ids = BTreeSet::new();
    for task in &plan.tasks {
        ensure!(
            !task.id.trim().is_empty() && ids.insert(&task.id),
            "task IDs must be unique and nonempty"
        );
        ensure!(
            task.timeout_ms > 0,
            "task {} timeout must be positive",
            task.id
        );
        ensure!(
            !task.prompt.trim().is_empty(),
            "task {} prompt is empty",
            task.id
        );
        ensure!(
            plan.agents
                .iter()
                .any(|a| a.capabilities.contains(&task.capability)),
            "task {} has no capable agent",
            task.id
        );
        ensure!(
            !task.write_scopes.is_empty(),
            "task {} requires write_scopes (use . for its whole cwd)",
            task.id
        );
        for scope in &task.write_scopes {
            ensure!(
                !scope.as_os_str().is_empty()
                    && !scope.is_absolute()
                    && scope
                        .components()
                        .all(|p| matches!(p, Component::Normal(_) | Component::CurDir)),
                "task {} scope must be a relative path without ..",
                task.id
            );
        }
    }
    let mut visited = BTreeSet::new();
    while visited.len() < ids.len() {
        let before = visited.len();
        for task in &plan.tasks {
            for dependency in &task.depends_on {
                ensure!(ids.contains(dependency), "unknown dependency {dependency}");
            }
            if task.depends_on.iter().all(|id| visited.contains(id)) {
                visited.insert(&task.id);
            }
        }
        ensure!(visited.len() > before, "task dependencies contain a cycle");
    }
    Ok(())
}

fn scope_paths(task: &Task) -> Vec<PathBuf> {
    task.write_scopes
        .iter()
        .map(|scope| {
            let mut path = task.cwd.clone();
            for component in scope.components() {
                if let Component::Normal(part) = component {
                    path.push(part);
                }
            }
            #[cfg(windows)]
            {
                PathBuf::from(path.to_string_lossy().to_lowercase())
            }
            #[cfg(not(windows))]
            {
                path
            }
        })
        .collect()
}

fn conflicts(a: &Task, b: &Task) -> bool {
    scope_paths(a).iter().any(|a| {
        scope_paths(b)
            .iter()
            .any(|b| a.starts_with(b) || b.starts_with(a))
    })
}

fn total(state: &State) -> Result<u64> {
    state.tasks.values().try_fold(0_u64, |sum, task| {
        sum.checked_add(task.charged_microusd)
            .and_then(|s| s.checked_add(task.reserved_microusd))
            .context("cost ledger overflow")
    })
}

fn execute(plan: &Plan, state: &mut State, state_path: &Path) -> Result<()> {
    ensure!(
        !state.tasks.values().any(|s| s.status == Status::Running),
        "interrupted running tasks found; inspect work and reconcile the state before resuming (no automatic replay)"
    );
    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|threads| -> Result<()> {
        let mut active = BTreeSet::<String>::new();
        loop {
            let mut candidates: Vec<_> = plan
                .tasks
                .iter()
                .filter(|t| state.tasks[&t.id].status == Status::Pending)
                .collect();
            candidates.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
            for task in candidates {
                if active.len() >= plan.max_parallel {
                    break;
                }
                if task
                    .depends_on
                    .iter()
                    .any(|id| matches!(state.tasks[id].status, Status::Failed | Status::Blocked))
                {
                    let entry = state.tasks.get_mut(&task.id).unwrap();
                    entry.status = Status::Blocked;
                    entry.note = "dependency failed or blocked".into();
                    save(state_path, state)?;
                    continue;
                }
                if !task
                    .depends_on
                    .iter()
                    .all(|id| state.tasks[id].status == Status::Succeeded)
                {
                    continue;
                }
                if plan
                    .tasks
                    .iter()
                    .any(|other| active.contains(&other.id) && conflicts(task, other))
                {
                    continue;
                }
                let agent = plan.agents.iter().find(|agent| {
                    agent.capabilities.contains(&task.capability)
                        && !active
                            .iter()
                            .any(|id| state.tasks[id].agent.as_ref() == Some(&agent.id))
                });
                let Some(agent) = agent else {
                    continue;
                };
                if state.tasks[&task.id].route.is_none() {
                    let request = RouteRequest {
                        task_id: task.id.clone(),
                        capability: task.capability.clone(),
                        input_tokens: task.input_tokens,
                        output_tokens: task.output_tokens,
                    };
                    match router::route(&plan.router, &request, &task.cwd) {
                        Ok(decision) => {
                            state.tasks.get_mut(&task.id).unwrap().route = Some(decision)
                        }
                        Err(error) => {
                            let entry = state.tasks.get_mut(&task.id).unwrap();
                            entry.status = Status::Failed;
                            entry.note = format!("router failed: {error:#}");
                            save(state_path, state)?;
                            continue;
                        }
                    }
                }
                let decision = state.tasks[&task.id].route.as_ref().unwrap().clone();
                if total(state)?
                    .checked_add(decision.estimated_microusd)
                    .is_none_or(|n| n > plan.budget_microusd)
                {
                    continue;
                }
                let entry = state.tasks.get_mut(&task.id).unwrap();
                entry.status = Status::Running;
                entry.agent = Some(agent.id.clone());
                entry.reserved_microusd = decision.estimated_microusd;
                entry.note = "usage pending; reservation is not measured spending".into();
                save(state_path, state)?; // Durable reservation precedes the executable.
                active.insert(task.id.clone());
                let argv: Vec<_> = agent
                    .command
                    .iter()
                    .map(|arg| match arg.as_str() {
                        "{model}" => decision.model.clone(),
                        "{prompt}" => task.prompt.clone(),
                        "{task_id}" => task.id.clone(),
                        _ => arg.clone(),
                    })
                    .collect();
                let input = serde_json::to_vec(
                    &serde_json::json!({"task":task,"model":decision.model,"agent":agent.id}),
                )?;
                let sender = sender.clone();
                threads.spawn(move || {
                    let result = process::run(&argv, &task.cwd, Some(&input), task.timeout_ms);
                    let _ = sender.send((task.id.clone(), result));
                });
            }
            if active.is_empty() {
                for entry in state
                    .tasks
                    .values_mut()
                    .filter(|s| s.status == Status::Pending)
                {
                    entry.status = Status::Blocked;
                    entry.note = "budget exhausted or dependency blocked".into();
                }
                save(state_path, state)?;
                break;
            }
            let (id, result) = receiver.recv().context("task executor disconnected")?;
            active.remove(&id);
            let entry = state.tasks.get_mut(&id).unwrap();
            let parsed = result.and_then(|output| {
                ensure!(
                    output.status.success(),
                    "executor exit {:?}: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr)
                );
                serde_json::from_slice::<TaskOutput>(&output.stdout).context(
                    "executor must return JSON {usage:{input_tokens,output_tokens},summary}",
                )
            });
            match parsed {
                Ok(output) => {
                    let model = plan
                        .router
                        .models
                        .iter()
                        .find(|m| m.id == entry.route.as_ref().unwrap().model)
                        .unwrap();
                    match router::cost(model, &output.usage) {
                        Ok(cost) => {
                            entry.charged_microusd = cost;
                            entry.usage = Some(output.usage);
                            entry.status = Status::Succeeded;
                            entry.note = output.summary;
                        }
                        Err(error) => {
                            entry.charged_microusd = entry.reserved_microusd;
                            entry.status = Status::Failed;
                            entry.note = format!(
                                "unaccountable usage; retained estimate, inspect provider usage: {error:#}"
                            );
                        }
                    }
                }
                Err(error) => {
                    entry.charged_microusd = entry.reserved_microusd;
                    entry.status = Status::Failed;
                    entry.note = format!("usage unknown; retained estimate: {error:#}");
                }
            }
            entry.reserved_microusd = 0;
            save(state_path, state)?;
        }
        Ok(())
    })
}

pub fn run(args: &[String]) -> Result<()> {
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        println!("{}", super::HELP);
        return Ok(());
    }
    let [action, path, flags @ ..] = args else {
        bail!("usage: crew orchestrator plan|run|status FILE [--execute]");
    };
    ensure!(
        matches!(action.as_str(), "plan" | "run" | "status"),
        "unknown orchestrator command {action}"
    );
    ensure!(
        flags.is_empty() || (action == "run" && flags == ["--execute"]),
        "unexpected orchestrator option"
    );
    ensure!(
        action != "run" || flags == ["--execute"],
        "run requires --execute; use plan for a preview"
    );
    let (plan, mut state, state_path) = load(Path::new(path))?;
    // Lock the canonical state identity, including when a plan is reached through an alias.
    let lock_path = PathBuf::from(format!("{}.lock", state_path.display()));
    let _lock = if action == "run" {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .context("plan locked; inspect the owner process before removing a stale .lock")?;
        drop(file);
        Some(Lock(lock_path))
    } else {
        None
    };
    if action == "run" {
        // The prior process may have completed between our preview read and lock acquisition.
        state = load_state(&plan, &state.plan_sha256)?;
        execute(&plan, &mut state, &state_path)?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "state":state,"budget_microusd":plan.budget_microusd,"accounted_and_reserved_microusd":total(&state)?,
            "tasks":plan.tasks,"agents":plan.agents,"execution_requested":action == "run"
        }))?
    );
    if action == "run" && state.tasks.values().any(|s| s.status != Status::Succeeded) {
        bail!("orchestration incomplete; inspect task state");
    }
    ensure!(
        action != "run" || total(&state)? <= plan.budget_microusd,
        "reported usage exceeded budget; no further tasks were admitted; inspect cost ledger"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
