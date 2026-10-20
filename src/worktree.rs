//! Git checkouts exposed as Crew workspaces. Git processes never hold the State mutex.
mod cli;
mod git;

use crate::api::{ApiError, ApiResult, Reply, Request as ApiRequest, lock};
use crate::server::Shared;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use cli::parse;
pub const HELP: &str = "crew worktree list [--repo PATH]\ncrew worktree create --path PATH --branch NAME [--repo PATH] [--base REF] [--name NAME] [--focus | --no-focus] [--no-open]\ncrew worktree open (--path PATH | --branch NAME) [--repo PATH] [--name NAME] [--focus | --no-focus]\ncrew worktree remove --path PATH [--repo PATH]\n\nCreates normal Git worktrees and opens them as workspaces by default. Removal refuses active, dirty, ignored-file, locked and primary checkouts; branches are kept. --cwd is an alias for --repo, --label for --name.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    List {
        repo: String,
    },
    Create {
        repo: String,
        path: String,
        branch: String,
        base: Option<String>,
        name: Option<String>,
        focus: bool,
        open: bool,
    },
    Open {
        repo: String,
        path: Option<String>,
        branch: Option<String>,
        name: Option<String>,
        focus: bool,
    },
    Remove {
        repo: String,
        path: String,
    },
}

// Serialize this module's mutations without blocking pane I/O or the event journal.
static OPERATIONS: Mutex<()> = Mutex::new(());

pub fn handle(shared: &Shared, req: Request) -> ApiResult {
    match req {
        Request::List { repo } => {
            let repo = git::repository(&repo)?;
            let entries = git::list(&repo)?;
            Ok(Reply::Json(
                json!({"repository": repo, "worktrees": entries}),
            ))
        }
        other => {
            let _operation = OPERATIONS.lock().unwrap_or_else(|e| e.into_inner());
            match other {
                Request::Create {
                    repo,
                    path,
                    branch,
                    base,
                    name,
                    focus,
                    open,
                } => create(
                    shared,
                    &repo,
                    &path,
                    &branch,
                    base.as_deref(),
                    name,
                    focus,
                    open,
                ),
                Request::Open {
                    repo,
                    path,
                    branch,
                    name,
                    focus,
                } => open_worktree(
                    shared,
                    &repo,
                    path.as_deref(),
                    branch.as_deref(),
                    name,
                    focus,
                ),
                Request::Remove { repo, path } => remove(shared, &repo, &path),
                Request::List { .. } => unreachable!(),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create(
    shared: &Shared,
    repo: &str,
    path: &str,
    branch: &str,
    base: Option<&str>,
    name: Option<String>,
    focus: bool,
    open: bool,
) -> ApiResult {
    validate_name(name.as_deref())?;
    let repo = git::repository(repo)?;
    let target = git::new_target(path)?;
    git::validate_branch(&repo, branch)?;
    if open
        && name
            .as_deref()
            .is_some_and(|n| lock(shared).sidx(n).is_some())
    {
        return Err(ApiError::new(
            "name_taken",
            "workspace name is already in use",
        ));
    }
    git::create(&repo, &target, branch, base).map_err(|e| {
        ApiError::new(
            &e.code,
            format!(
                "{}; destination: {}. Any files left there have been preserved",
                e.message,
                target.display()
            ),
        )
    })?;
    // A successful checkout is kept if opening a terminal fails. Never roll back by deleting
    // files that another process or checkout filter may already have changed.
    lock(shared).events.record(
        "worktree.created",
        None,
        json!({"path": target, "branch": branch}),
    );
    let entry = git::list(&repo)
        .and_then(|entries| find_path(&entries, &target).cloned())
        .map_err(|e| {
            ApiError::new(
                "worktree_created_unverified",
                format!(
                    "worktree created and kept at {}; cannot verify checkout: {}",
                    target.display(),
                    e.message
                ),
            )
        })?;
    let workspace = if open {
        Some(
            open_workspace(shared, &target, name, focus)
                .map_err(|e| {
                    ApiError::new(
                        "workspace_open_failed",
                        format!(
                            "worktree created and kept at {}; opening workspace failed: {}",
                            target.display(),
                            e.message
                        ),
                    )
                })?
                .0,
        )
    } else {
        None
    };
    Ok(Reply::Json(
        json!({"created": true, "worktree": entry, "workspace": workspace}),
    ))
}

fn open_worktree(
    shared: &Shared,
    repo: &str,
    path: Option<&str>,
    branch: Option<&str>,
    name: Option<String>,
    focus: bool,
) -> ApiResult {
    validate_name(name.as_deref())?;
    let repo = git::repository(repo)?;
    let entries = git::list(&repo)?;
    let entry = match (path, branch) {
        (Some(path), None) => find_path(&entries, &git::existing_dir(path)?)?.clone(),
        (None, Some(branch)) => {
            git::validate_branch(&repo, branch)?;
            entries
                .iter()
                .find(|e| e.branch.as_deref() == Some(branch))
                .cloned()
                .ok_or_else(|| {
                    ApiError::new(
                        "worktree_not_found",
                        format!("no worktree for branch {branch:?}"),
                    )
                })?
        }
        _ => {
            return Err(ApiError::invalid(
                "worktree open requires exactly one of path or branch",
            ));
        }
    };
    let target = git::existing_dir(&entry.path)?;
    if entry.bare {
        return Err(ApiError::invalid(
            "cannot open a bare repository as a worktree",
        ));
    }
    git::validate_checkout(&repo, &target, entry.primary)?;
    let (workspace, reused) = open_workspace(shared, &target, name, focus)?;
    Ok(Reply::Json(
        json!({"opened": true, "reused": reused, "worktree": entry, "workspace": workspace}),
    ))
}

fn validate_name(name: Option<&str>) -> Result<(), ApiError> {
    if name.is_some_and(|s| {
        s.trim().is_empty() || s.chars().count() > 80 || s.chars().any(char::is_control)
    }) {
        return Err(ApiError::invalid(
            "workspace name must be 1..80 characters without control characters",
        ));
    }
    Ok(())
}

/// Git filesystem work is completed before entering workspace handlers.
fn open_workspace(
    shared: &Shared,
    path: &Path,
    name: Option<String>,
    focus: bool,
) -> Result<(Value, bool), ApiError> {
    let path_text = git::path_text(path)?;
    let candidates: Vec<(usize, String)> = {
        lock(shared)
            .sessions
            .iter()
            .filter_map(|s| s.cwd.as_ref().map(|cwd| (s.id, cwd.clone())))
            .collect()
    };
    let existing = candidates
        .into_iter()
        .find(|(_, cwd)| same_path(Path::new(cwd), path));
    let existing = {
        let st = lock(shared);
        existing
            .and_then(|(id, cwd)| {
                st.sessions
                    .iter()
                    .position(|s| s.id == id && s.cwd.as_ref() == Some(&cwd))
            })
            .map(|si| crate::workspace::workspace_json(&st, si))
    };
    let reused = existing.is_some();
    let workspace = if let Some(existing) = existing {
        if focus {
            let id = existing["workspace_id"]
                .as_str()
                .ok_or_else(|| ApiError::new("internal", "missing workspace ID"))?;
            crate::workspace::handle(
                shared,
                crate::workspace::Request::WorkspaceFocus {
                    workspace: id.into(),
                },
            )?;
            let st = lock(shared);
            let si = crate::workspace::workspace_index(&st, id)?;
            crate::workspace::workspace_json(&st, si)
        } else {
            existing
        }
    } else {
        match crate::workspace::handle(
            shared,
            crate::workspace::Request::WorkspaceCreate {
                name,
                cwd: Some(path_text.to_string()),
                env: Default::default(),
                command: None,
                focus,
            },
        )? {
            Reply::Json(v) => v["workspace"].clone(),
            _ => return Err(ApiError::new("internal", "unexpected workspace response")),
        }
    };
    lock(shared).events.record(
        "worktree.opened",
        None,
        json!({"path": path, "workspace_id": workspace["workspace_id"], "reused": reused}),
    );
    Ok((workspace, reused))
}

fn find_path<'a>(entries: &'a [git::Entry], path: &Path) -> Result<&'a git::Entry, ApiError> {
    entries
        .iter()
        .find(|entry| same_path(Path::new(&entry.path), path))
        .ok_or_else(|| {
            ApiError::new(
                "worktree_not_found",
                format!(
                    "{} is not a registered worktree of this repository",
                    path.display()
                ),
            )
        })
}
fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
fn active_paths(shared: &Shared) -> Vec<PathBuf> {
    let st = lock(shared);
    let paths: Vec<PathBuf> = st
        .sessions
        .iter()
        .filter_map(|s| s.cwd.as_ref())
        .chain(st.panes.values().filter_map(|p| p.cwd.as_ref()))
        .map(PathBuf::from)
        .collect();
    #[cfg(target_os = "linux")]
    let pids: Vec<u32> = st
        .panes
        .values()
        .flat_map(|p| {
            [
                p.master
                    .process_group_leader()
                    .and_then(|pid| u32::try_from(pid).ok()),
                p.child.process_id(),
            ]
        })
        .flatten()
        .collect();
    drop(st);
    // Filesystem lookups happen after releasing State, including foreground processes
    // whose shell changed directory after the workspace was originally opened.
    #[cfg(target_os = "linux")]
    let paths = paths
        .into_iter()
        .chain(
            pids.into_iter()
                .filter_map(|pid| std::fs::read_link(format!("/proc/{pid}/cwd")).ok()),
        )
        .collect();
    paths
}

fn remove(shared: &Shared, repo: &str, path: &str) -> ApiResult {
    let repo = git::repository(repo)?;
    let target = git::existing_dir(path)?;
    let entries = git::list(&repo)?;
    let entry = find_path(&entries, &target)?;
    if entry.primary {
        return Err(ApiError::new(
            "worktree_primary",
            "the primary checkout cannot be removed",
        ));
    }
    if entry.locked.is_some() {
        return Err(ApiError::new(
            "worktree_locked",
            "this checkout is locked; unlock it explicitly with Git first",
        ));
    }
    git::validate_checkout(&repo, &target, false)?;
    // Reserve before checking users. Every pane creation checks this set while holding
    // State, so a concurrent Crew request cannot enter the checkout during Git removal.
    let _reservation = RemovalReservation::new(shared, &target)?;
    if active_paths(shared)
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .any(|p| p.starts_with(&target))
    {
        return Err(ApiError::new(
            "worktree_in_use",
            "close every workspace and pane using this worktree before removing it",
        ));
    }
    if git::dirty(&target)? {
        return Err(ApiError::new(
            "worktree_dirty",
            "worktree contains modified, untracked or ignored files; preserve or clean them explicitly before removal",
        ));
    }
    git::remove(&repo, &target)?;
    lock(shared).events.record(
        "worktree.removed",
        None,
        json!({"path": target, "branch": entry.branch, "branch_deleted": false}),
    );
    Ok(Reply::Json(
        json!({"removed": true, "path": target, "branch": entry.branch, "branch_deleted": false}),
    ))
}

struct RemovalReservation {
    shared: Shared,
    path: PathBuf,
}
impl RemovalReservation {
    fn new(shared: &Shared, path: &Path) -> Result<Self, ApiError> {
        let mut st = lock(shared);
        if !st.removing_worktrees.insert(path.to_path_buf()) {
            return Err(ApiError::new(
                "worktree_busy",
                "worktree removal is already in progress",
            ));
        }
        Ok(Self {
            shared: shared.clone(),
            path: path.to_path_buf(),
        })
    }
}
impl Drop for RemovalReservation {
    fn drop(&mut self) {
        lock(&self.shared).removing_worktrees.remove(&self.path);
    }
}
