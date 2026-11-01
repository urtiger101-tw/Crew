//! Pane host loss. On Windows each pane is a ConPTY served by its own `conhost.exe`; when
//! that host dies (it can crash, for example while the window is resized) the pane's output
//! stream ends although the shell or the programs it started may still be running. Removing
//! the pane then would orphan them out of sight, so the pane is kept with a banner instead,
//! and closing it ends the surviving process tree.
use super::{Pane, State};
use std::time::Instant;

/// Why a pane is kept after its output stream ended: the processes still alive under it.
#[derive(Debug, Clone, PartialEq)]
pub struct HostLost {
    pub since: Instant,
    /// (pid, executable), the pane's root process first when it is still running
    pub survivors: Vec<(u32, String)>,
}

impl State {
    /// The reader of pane `id` reached EOF. Close the pane as before when nothing under it
    /// is still running; otherwise keep it, marked host-lost.
    pub(crate) fn pane_host_ended(&mut self, id: usize) {
        let Some(p) = self.panes.get(&id) else {
            return;
        };
        let survivors = survivors(p);
        if survivors.is_empty() {
            self.pane_exited(id);
            return;
        }
        crate::log!("pane {id}: console host ended; still running {survivors:?}");
        self.events.record(
            "pane.host_lost",
            Some(id),
            serde_json::json!({ "processes": processes_json(&survivors) }),
        );
        if let Some(p) = self.panes.get_mut(&id) {
            p.host_lost = Some(HostLost {
                since: Instant::now(),
                survivors,
            });
        }
        self.mark_dirty();
    }

    /// Housekeeping for host-lost panes: refresh the survivor list, and close a pane once
    /// nothing under it runs any more.
    pub(crate) fn reap_host_lost(&mut self) {
        let mut gone = vec![];
        let mut changed = false;
        for (id, p) in self.panes.iter_mut() {
            let Some(lost) = &p.host_lost else { continue };
            let now = refreshed(p, &lost.survivors);
            if now.is_empty() {
                // Nothing is left to end: forget the stale list so the close below cannot
                // terminate a pid the system has since handed to an unrelated process.
                p.host_lost = None;
                gone.push(*id);
            } else if now != lost.survivors {
                changed = true;
                if let Some(lost) = p.host_lost.as_mut() {
                    lost.survivors = now;
                }
            }
        }
        for id in gone {
            self.pane_exited(id);
        }
        if changed {
            self.mark_dirty();
        }
    }
}

fn root_pid(p: &Pane) -> Option<u32> {
    p.child.process_id()
}

fn survivors(p: &Pane) -> Vec<(u32, String)> {
    refreshed(p, &[])
}

/// The pane's processes still running now: its tree from the root, plus everything under
/// processes seen running before (`known`), whose parents may have exited since.
fn refreshed(p: &Pane, known: &[(u32, String)]) -> Vec<(u32, String)> {
    match root_pid(p) {
        Some(root) => crate::agent_detection::live_tree_with(root, known),
        // u32::MAX is never a real process id, so only `known` seeds the walk
        None => crate::agent_detection::live_tree_with(u32::MAX, known),
    }
}

pub(crate) fn processes_json(survivors: &[(u32, String)]) -> serde_json::Value {
    survivors
        .iter()
        .map(|(pid, exe)| serde_json::json!({ "pid": pid, "executable": exe }))
        .collect()
}

/// Closing a host-lost pane: end every surviving process of its tree. Killing only the
/// root (what a normal close does) would leave the descendants running without a console.
pub(crate) fn kill_tree(p: &Pane) {
    let Some(lost) = &p.host_lost else { return };
    // Rescan right before ending anything: the last list may be up to one tick old.
    for (pid, exe) in &refreshed(p, &lost.survivors) {
        crate::log!("ending host-lost survivor {exe} ({pid})");
        terminate(*pid);
    }
}

#[cfg(windows)]
fn terminate(pid: u32) {
    // `taskkill /T` also ends children that appeared after the last survivor scan.
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(windows))]
fn terminate(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processes_json_lists_pid_and_executable() {
        let v = processes_json(&[(4, "codex.exe".into()), (9, "node.exe".into())]);
        assert_eq!(
            v,
            serde_json::json!([
                {"pid": 4, "executable": "codex.exe"},
                {"pid": 9, "executable": "node.exe"}
            ])
        );
    }
}
