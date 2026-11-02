use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    repo: String,
    repo_path: PathBuf,
    state_path: PathBuf,
    head: String,
    base: String,
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default = "yes")]
    draft: bool,
    pr_number: Option<u64>,
    #[serde(default)]
    reviewers: Vec<String>,
    required_checks: Vec<String>,
    ci: Option<Ci>,
    #[serde(default = "default_gh")]
    gh_command: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ci {
    workflow: String,
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    inputs: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}
fn default_gh() -> Vec<String> {
    vec!["gh".into()]
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    config_hash: String,
    pr_number: Option<u64>,
    head_sha: Option<String>,
    requested_for_head: Option<String>,
    dispatched_for_head: Option<String>,
    recovery_needed: Option<String>,
    status: String,
}

#[derive(Debug, Deserialize)]
struct Pull {
    number: u64,
    state: String,
    user: User,
    head: Head,
    base: Branch,
    #[serde(default)]
    requested_reviewers: Vec<User>,
}
#[derive(Debug, Deserialize)]
struct User {
    login: String,
}
#[derive(Debug, Deserialize)]
struct Head {
    sha: String,
    #[serde(rename = "ref")]
    reference: String,
}
#[derive(Debug, Deserialize)]
struct Branch {
    #[serde(rename = "ref")]
    reference: String,
}
#[derive(Debug, Deserialize)]
struct Review {
    user: Option<User>,
    state: String,
    commit_id: Option<String>,
}
#[derive(Debug, Deserialize)]
struct CheckPage {
    check_runs: Vec<Check>,
}
#[derive(Debug, Deserialize)]
struct Check {
    id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
}
#[derive(Debug, Deserialize)]
struct CommitStatus {
    id: u64,
    context: String,
    state: String,
}
#[derive(Debug, Deserialize)]
struct ListedPull {
    number: u64,
}

struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn save(path: &Path, state: &State) -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(
        ".tmp.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(name);
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let result = (|| -> Result<()> {
        file.write_all(&serde_json::to_vec_pretty(state)?)?;
        drop(file);
        fs::rename(&tmp, path).with_context(|| format!("replace state file {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}

fn call<F>(
    config: &Config,
    repo_dir: &Path,
    command: &[String],
    input: Option<&[u8]>,
    runner: &mut F,
) -> Result<Vec<u8>>
where
    F: FnMut(&[String], &Path, Option<&[u8]>, u64) -> Result<std::process::Output>,
{
    let mut argv = config.gh_command.clone();
    argv.extend_from_slice(command);
    let output = runner(&argv, repo_dir, input, 30_000)?;
    ensure!(
        output.status.success(),
        "gh command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}

fn api<F>(
    config: &Config,
    repo_dir: &Path,
    endpoint: &str,
    paginate: bool,
    runner: &mut F,
) -> Result<Vec<u8>>
where
    F: FnMut(&[String], &Path, Option<&[u8]>, u64) -> Result<std::process::Output>,
{
    let mut args = vec!["api".into(), endpoint.into()];
    if paginate {
        args.extend(["--paginate".into(), "--slurp".into()]);
    }
    call(config, repo_dir, &args, None, runner)
}

fn pull<F>(config: &Config, repo_dir: &Path, number: u64, runner: &mut F) -> Result<Pull>
where
    F: FnMut(&[String], &Path, Option<&[u8]>, u64) -> Result<std::process::Output>,
{
    let bytes = api(
        config,
        repo_dir,
        &format!("repos/{}/pulls/{number}", config.repo),
        false,
        runner,
    )?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn mutate<F>(
    config: &Config,
    repo_dir: &Path,
    state_path: &Path,
    state: &mut State,
    operation: (&str, &[String], Option<&[u8]>),
    runner: &mut F,
) -> Result<()>
where
    F: FnMut(&[String], &Path, Option<&[u8]>, u64) -> Result<std::process::Output>,
{
    let (description, args, input) = operation;
    state.recovery_needed = Some(description.into());
    save(state_path, state)?;
    call(config, repo_dir, args, input, runner).with_context(|| {
        format!("{description}: outcome may be ambiguous; inspect GitHub before recovery")
    })?;
    Ok(())
}

fn advance<F>(
    config: &Config,
    repo_dir: &Path,
    state_path: &Path,
    state: &mut State,
    execute: bool,
    allow_ci: bool,
    runner: &mut F,
) -> Result<()>
where
    F: FnMut(&[String], &Path, Option<&[u8]>, u64) -> Result<std::process::Output>,
{
    ensure!(
        state.recovery_needed.is_none(),
        "recovery required for {:?}; inspect GitHub and reconcile state before retrying",
        state.recovery_needed
    );
    state.status = "collecting current GitHub evidence".into();
    save(state_path, state)?;
    let repo_arg = vec!["--repo".into(), config.repo.clone()];
    if state.pr_number.is_none() {
        let mut args = vec![
            "pr".into(),
            "list".into(),
            "--state".into(),
            "open".into(),
            "--head".into(),
            config.head.clone(),
            "--base".into(),
            config.base.clone(),
            "--json".into(),
            "number".into(),
            "--limit".into(),
            "100".into(),
        ];
        args.extend(repo_arg.clone());
        let matches: Vec<ListedPull> =
            serde_json::from_slice(&call(config, repo_dir, &args, None, runner)?)?;
        ensure!(
            matches.len() <= 1,
            "multiple matching PRs; configure pr_number explicitly"
        );
        if let Some(pr) = matches.first() {
            state.pr_number = Some(pr.number);
            save(state_path, state)?;
        } else if !execute {
            state.status = "PR creation requires --execute; ensure the branch is pushed".into();
            save(state_path, state)?;
            println!("{}", state.status);
            return Ok(());
        } else {
            let mut args = vec![
                "pr".into(),
                "create".into(),
                "--head".into(),
                config.head.clone(),
                "--base".into(),
                config.base.clone(),
                "--title".into(),
                config.title.clone(),
                "--body".into(),
                config.body.clone(),
            ];
            if config.draft {
                args.push("--draft".into());
            }
            args.extend(repo_arg.clone());
            mutate(
                config,
                repo_dir,
                state_path,
                state,
                ("create PR", &args, None),
                runner,
            )?;
            // Discover the created PR before clearing the recovery marker.
            let mut query = vec![
                "pr".into(),
                "list".into(),
                "--state".into(),
                "open".into(),
                "--head".into(),
                config.head.clone(),
                "--base".into(),
                config.base.clone(),
                "--json".into(),
                "number".into(),
                "--limit".into(),
                "100".into(),
            ];
            query.extend(repo_arg);
            let found: Vec<ListedPull> =
                serde_json::from_slice(&call(config, repo_dir, &query, None, runner)?)?;
            ensure!(
                found.len() == 1,
                "created PR could not be uniquely identified; recovery required"
            );
            state.pr_number = Some(found[0].number);
            state.recovery_needed = None;
            save(state_path, state)?;
        }
    }
    let number = state.pr_number.unwrap();
    let pr = pull(config, repo_dir, number, runner)?;
    ensure!(
        pr.number == number
            && pr.state == "open"
            && pr.head.reference == config.head
            && pr.base.reference == config.base,
        "PR is not open or its head/base branches do not match config"
    );
    if state.head_sha.as_deref() != Some(&pr.head.sha) {
        state.head_sha = Some(pr.head.sha.clone());
        state.requested_for_head = None;
        state.dispatched_for_head = None;
        state.status = "head changed; approval and checks must be re-evaluated".into();
        save(state_path, state)?;
    }
    let pages: Vec<Vec<Review>> = serde_json::from_slice(&api(
        config,
        repo_dir,
        &format!("repos/{}/pulls/{number}/reviews?per_page=100", config.repo),
        true,
        runner,
    )?)?;
    let mut latest: HashMap<String, &Review> = HashMap::new();
    for review in pages.iter().flatten() {
        if let Some(user) = &review.user
            && review.state != "COMMENTED"
            && review.state != "PENDING"
        {
            latest.insert(user.login.to_ascii_lowercase(), review);
        }
    }
    let approved = latest.iter().any(|(login, r)| {
        !login.eq_ignore_ascii_case(&pr.user.login)
            && r.state == "APPROVED"
            && r.commit_id.as_deref() == Some(pr.head.sha.as_str())
    });
    let changes_requested = latest.iter().any(|(login, r)| {
        !login.eq_ignore_ascii_case(&pr.user.login) && r.state == "CHANGES_REQUESTED"
    });
    if changes_requested {
        state.status = "waiting for outstanding change requests to be resolved".into();
        save(state_path, state)?;
        println!("{}", state.status);
        return Ok(());
    }
    if !approved {
        let pending: Vec<String> = config
            .reviewers
            .iter()
            .filter(|r| {
                !r.eq_ignore_ascii_case(&pr.user.login)
                    && !pr
                        .requested_reviewers
                        .iter()
                        .any(|u| u.login.eq_ignore_ascii_case(r))
            })
            .cloned()
            .collect();
        if execute
            && !pending.is_empty()
            && state.requested_for_head.as_deref() != Some(&pr.head.sha)
        {
            let args = vec![
                "api".into(),
                "--method".into(),
                "POST".into(),
                format!("repos/{}/pulls/{number}/requested_reviewers", config.repo),
                "--input".into(),
                "-".into(),
            ];
            let body = serde_json::to_vec(&serde_json::json!({"reviewers": pending}))?;
            mutate(
                config,
                repo_dir,
                state_path,
                state,
                ("request reviewers", &args, Some(&body)),
                runner,
            )?;
            state.requested_for_head = Some(pr.head.sha.clone());
            state.recovery_needed = None;
        }
        state.status = "waiting for a non-author approval on the current head".into();
        save(state_path, state)?;
        println!("{}", state.status);
        return Ok(());
    }
    let checks: Vec<CheckPage> = serde_json::from_slice(&api(
        config,
        repo_dir,
        &format!(
            "repos/{}/commits/{}/check-runs?per_page=100",
            config.repo, pr.head.sha
        ),
        true,
        runner,
    )?)?;
    let statuses: Vec<Vec<CommitStatus>> = serde_json::from_slice(&api(
        config,
        repo_dir,
        &format!(
            "repos/{}/commits/{}/statuses?per_page=100",
            config.repo, pr.head.sha
        ),
        true,
        runner,
    )?)?;
    let mut results: HashMap<String, (u64, bool)> = HashMap::new();
    for check in checks.into_iter().flat_map(|p| p.check_runs) {
        let entry = results.entry(check.name).or_insert((0, false));
        if check.id >= entry.0 {
            *entry = (
                check.id,
                check.status == "completed" && check.conclusion.as_deref() == Some("success"),
            );
        }
    }
    // Status contexts and check runs are independent namespaces. A failure in either is not success.
    let mut contexts: HashMap<String, (u64, bool)> = HashMap::new();
    for status in statuses.into_iter().flatten() {
        let entry = contexts.entry(status.context).or_insert((0, false));
        if status.id >= entry.0 {
            *entry = (status.id, status.state == "success");
        }
    }
    let missing: Vec<&str> = config
        .required_checks
        .iter()
        .filter(|name| {
            let check = results.get(*name).map(|v| v.1);
            let status = contexts.get(*name).map(|v| v.1);
            !(check.is_some() || status.is_some()) || check == Some(false) || status == Some(false)
        })
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        state.status =
            "ready: current head has non-author approval and all required checks succeeded".into();
    } else {
        state.status = format!("waiting for required checks: {}", missing.join(", "));
        if let Some(ci) = &config.ci
            && execute
            && allow_ci
            && state.dispatched_for_head.as_deref() != Some(&pr.head.sha)
        {
            ensure!(
                ci.reference == config.head,
                "CI ref must match PR head branch"
            );
            let fresh = pull(config, repo_dir, number, runner)?;
            ensure!(
                fresh.state == "open"
                    && fresh.head.sha == pr.head.sha
                    && fresh.head.reference == config.head
                    && fresh.base.reference == config.base,
                "PR changed before CI dispatch; run advance again"
            );
            let body =
                serde_json::to_vec(&serde_json::json!({"ref": ci.reference, "inputs": ci.inputs}))?;
            let args = vec![
                "api".into(),
                "--method".into(),
                "POST".into(),
                format!(
                    "repos/{}/actions/workflows/{}/dispatches",
                    config.repo, ci.workflow
                ),
                "--input".into(),
                "-".into(),
            ];
            mutate(
                config,
                repo_dir,
                state_path,
                state,
                ("dispatch CI", &args, Some(&body)),
                runner,
            )?;
            state.dispatched_for_head = Some(pr.head.sha.clone());
            state.recovery_needed = None;
        }
    }
    // Confirm that the evidence still describes the current PR head before reporting readiness.
    let fresh = pull(config, repo_dir, number, runner)?;
    ensure!(
        fresh.state == "open"
            && fresh.head.sha == pr.head.sha
            && fresh.head.reference == config.head
            && fresh.base.reference == config.base,
        "PR changed during evidence collection; run advance again"
    );
    save(state_path, state)?;
    println!("{}", state.status);
    Ok(())
}

fn poll<F>(
    config: &Config,
    repo_dir: &Path,
    state_path: &Path,
    state: &mut State,
    options: (bool, bool),
    timing: (u64, u64),
    runner: &mut F,
) -> Result<()>
where
    F: FnMut(&[String], &Path, Option<&[u8]>, u64) -> Result<std::process::Output>,
{
    let (execute, allow_ci) = options;
    let (poll_ms, timeout_ms) = timing;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(timeout_ms))
        .context("workflow timeout is too large")?;
    loop {
        ensure!(
            Instant::now() < deadline,
            "workflow polling timed out; last status: {}",
            state.status
        );
        let mut bounded_runner = |argv: &[String], cwd: &Path, input: Option<&[u8]>, limit: u64| {
            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(!remaining.is_zero(), "workflow polling timed out");
            let millis = u64::try_from(remaining.as_millis())
                .unwrap_or(u64::MAX)
                .max(1);
            runner(argv, cwd, input, limit.min(millis))
        };
        advance(
            config,
            repo_dir,
            state_path,
            state,
            execute,
            allow_ci,
            &mut bounded_runner,
        )?;
        if state.status.starts_with("ready:") {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !remaining.is_zero(),
            "workflow polling timed out; last status: {}",
            state.status
        );
        std::thread::sleep(remaining.min(Duration::from_millis(poll_ms)));
    }
}

pub fn run(args: &[String]) -> Result<()> {
    const HELP: &str = "usage: crew workflow plan FILE\n       crew workflow advance FILE [--execute] [--allow-ci]\n       crew workflow run FILE [--execute] [--allow-ci] [--poll-ms N] [--timeout-ms N]\n\nrun polls until ready or timeout; --execute authorizes PR creation and reviewer requests; --allow-ci additionally authorizes configured CI dispatch.";
    if args == ["--help"]
        || args == ["-h"]
        || (args.len() == 2 && (args[1] == "--help" || args[1] == "-h"))
    {
        println!("{HELP}");
        return Ok(());
    }
    ensure!(args.len() >= 2, "{HELP}");
    let action = &args[0];
    ensure!(
        action == "plan" || action == "advance" || action == "run",
        "unknown workflow command: {action}"
    );
    let mut execute = false;
    let mut allow_ci = false;
    let mut poll_ms = None;
    let mut timeout_ms = None;
    let mut flags = args[2..].iter();
    while let Some(arg) = flags.next() {
        match arg.as_str() {
            "--execute" if !execute => execute = true,
            "--allow-ci" if !allow_ci => allow_ci = true,
            "--poll-ms" if action == "run" && poll_ms.is_none() => {
                poll_ms = Some(
                    flags
                        .next()
                        .context("--poll-ms requires a value")?
                        .parse::<u64>()
                        .context("invalid --poll-ms")?,
                )
            }
            "--timeout-ms" if action == "run" && timeout_ms.is_none() => {
                timeout_ms = Some(
                    flags
                        .next()
                        .context("--timeout-ms requires a value")?
                        .parse::<u64>()
                        .context("invalid --timeout-ms")?,
                )
            }
            _ => bail!("unknown or duplicate option: {arg}"),
        }
    }
    ensure!(
        action != "plan" || (!execute && !allow_ci),
        "plan does not accept execution flags"
    );
    ensure!(!allow_ci || execute, "--allow-ci also requires --execute");
    let poll_ms = poll_ms.unwrap_or(30_000);
    let timeout_ms = timeout_ms.unwrap_or(3_600_000);
    ensure!(
        poll_ms > 0 && timeout_ms > 0,
        "poll and timeout must be positive"
    );
    let config_path = fs::canonicalize(&args[1]).context("open workflow config")?;
    let dir = config_path
        .parent()
        .context("config has no parent directory")?;
    let bytes = fs::read(&config_path)?;
    let config: Config = serde_json::from_slice(&bytes).context("parse workflow JSON config")?;
    ensure!(
        !config.gh_command.is_empty() && !config.gh_command[0].is_empty(),
        "gh_command must contain an executable"
    );
    ensure!(
        config.repo.split('/').count() == 2
            && !config.repo.starts_with('/')
            && !config.repo.ends_with('/'),
        "repo must be owner/name"
    );
    ensure!(
        !config.head.is_empty() && !config.base.is_empty() && !config.title.is_empty(),
        "head, base, and title are required"
    );
    ensure!(
        !config.required_checks.is_empty()
            && config
                .required_checks
                .iter()
                .all(|name| !name.trim().is_empty()),
        "at least one non-empty required check is required"
    );
    let repo_dir = fs::canonicalize(dir.join(&config.repo_path)).context("open repo_path")?;
    let mut state_path = dir.join(&config.state_path);
    if state_path.exists() {
        state_path = fs::canonicalize(&state_path).context("resolve state_path")?;
        ensure!(
            state_path != config_path && state_path != repo_dir,
            "state_path must not be the config or repository directory"
        );
    }
    let _lock = if action != "plan" {
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent)?;
            state_path = fs::canonicalize(parent)?.join(
                state_path
                    .file_name()
                    .context("state_path needs a filename")?,
            );
        }
        let mut name = state_path.as_os_str().to_os_string();
        name.push(".lock");
        let path = PathBuf::from(name);
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path).with_context(|| format!("acquire workflow lock {}; another advance may be running, or a stale lock requires inspection", path.display()))?;
        let lock = Lock(path);
        writeln!(file, "{}", std::process::id())?;
        Some(lock)
    } else {
        None
    };
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let mut state: State = match fs::read(&state_path) {
        Ok(data) => serde_json::from_slice(&data).context("parse workflow state")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => State {
            config_hash: hash.clone(),
            pr_number: config.pr_number,
            ..State::default()
        },
        Err(e) => return Err(e.into()),
    };
    ensure!(
        state.config_hash == hash,
        "config changed since state was created; reconcile or use a new state_path"
    );
    if action == "plan" {
        println!(
            "PR: {:?}; head: {:?}; status: {}; recovery: {:?}",
            state.pr_number,
            state.head_sha,
            if state.status.is_empty() {
                "not started"
            } else {
                &state.status
            },
            state.recovery_needed
        );
        println!(
            "Next: advance to refresh GitHub evidence; --execute permits PR creation and reviewer requests; --allow-ci additionally permits configured CI dispatch."
        );
        return Ok(());
    }
    if action == "run" {
        return poll(
            &config,
            &repo_dir,
            &state_path,
            &mut state,
            (execute, allow_ci),
            (poll_ms, timeout_ms),
            &mut super::process::run,
        );
    }
    advance(
        &config,
        &repo_dir,
        &state_path,
        &mut state,
        execute,
        allow_ci,
        &mut super::process::run,
    )
}

#[cfg(test)]
mod tests;
