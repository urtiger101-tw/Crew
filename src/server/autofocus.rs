//! Edge-triggered focus scheduling. Status detection remains owned by api::agent.
use super::State;
use crate::api::Status;
use crate::i18n::{Lang, tr, trf};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Off,
    Blocked,
    On,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Window,
    #[default]
    Session,
    All,
}

pub fn indicator(mode: Mode, lang: Lang) -> &'static str {
    match mode {
        Mode::Off => "",
        Mode::On => tr(lang, "render.auto_focus.on"),
        Mode::Blocked => tr(lang, "render.auto_focus.blocked"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub pane: usize,
    pub generation: u64,
    pub status: Status,
}

#[derive(Default)]
pub struct ClientFocus {
    pub last_input: Option<Instant>,
    pub last_jump: Option<Instant>,
    pub queue: Vec<Edge>,
    pub prior: Option<usize>,
    pub notified: Option<(usize, u64)>,
}

#[derive(Debug, Clone, Copy)]
struct Observed {
    status: Status,
    occupant: Option<u64>,
    generation: u64,
}

#[derive(Default)]
pub struct Tracker {
    observed: BTreeMap<usize, Observed>,
    next_generation: u64,
}

impl Tracker {
    fn observe(&mut self, pane: usize, status: Status, occupant: Option<u64>) -> Option<Edge> {
        if self
            .observed
            .get(&pane)
            .is_some_and(|old| old.status == status && old.occupant == occupant)
        {
            return None;
        }
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        self.observed.insert(
            pane,
            Observed {
                status,
                occupant,
                generation,
            },
        );
        matches!(status, Status::Blocked | Status::Done).then_some(Edge {
            pane,
            status,
            generation,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub edge: Edge,
    pub session: usize,
    pub window: usize,
    pub enabled: bool,
    pub agent: bool,
    pub changes_shared_view: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    pub id: usize,
    pub session: usize,
    pub window: usize,
    pub pane: usize,
    pub last_input: Option<Instant>,
    pub last_jump: Option<Instant>,
    pub guarded: bool,
}

#[derive(Clone, Copy)]
pub struct Policy {
    pub mode: Mode,
    pub scope: Scope,
    pub idle: Duration,
    pub dwell: Duration,
}

pub fn eligible(edge: Edge, candidate: &Candidate, viewer: &Viewer, policy: Policy) -> bool {
    edge == candidate.edge
        && candidate.enabled
        && (edge.status != Status::Done || candidate.agent)
        && edge.pane != viewer.pane
        && match policy.mode {
            Mode::Off => false,
            Mode::Blocked => edge.status == Status::Blocked,
            Mode::On => matches!(edge.status, Status::Blocked | Status::Done),
        }
        && match policy.scope {
            Scope::Window => {
                candidate.window == viewer.window && candidate.session == viewer.session
            }
            Scope::Session => candidate.session == viewer.session,
            Scope::All => true,
        }
}

/// Shared queue ranking for automatic scheduling and explicit attention navigation.
fn queued_target<'a>(
    queue: &[Edge],
    panes: &'a [Candidate],
    viewer: &Viewer,
    policy: Policy,
) -> Option<&'a Candidate> {
    queue
        .iter()
        .filter_map(|edge| panes.iter().find(|p| eligible(*edge, p, viewer, policy)))
        .min_by_key(|p| {
            (
                usize::from(p.edge.status != Status::Blocked),
                usize::from(p.session != viewer.session || p.window != viewer.window),
                p.edge.generation,
            )
        })
}

/// Pure guard boundary. The second result is the queued toast target.
pub fn decide(
    queue: &[Edge],
    panes: &[Candidate],
    viewer: &Viewer,
    viewers: &[Viewer],
    policy: Policy,
    now: Instant,
) -> (Option<usize>, Option<Edge>) {
    let Some(target) = queued_target(queue, panes, viewer, policy) else {
        return (None, None);
    };
    let held = viewers
        .iter()
        .filter(|other| {
            other.id == viewer.id || (target.changes_shared_view && other.session == target.session)
        })
        .any(|other| {
            other.guarded
                || other
                    .last_input
                    .is_some_and(|at| now.saturating_duration_since(at) < policy.idle)
                || other
                    .last_jump
                    .is_some_and(|at| now.saturating_duration_since(at) < policy.dwell)
        });
    if held {
        (None, Some(target.edge))
    } else {
        (Some(target.edge.pane), None)
    }
}

impl State {
    /// Observe report/output/tick changes before selecting queued targets. A
    /// generation pins both effective status and occupant, so stale work drops.
    pub(crate) fn autofocus_update(&mut self, now: Instant) {
        self.autofocus
            .observed
            .retain(|id, _| self.panes.contains_key(id));
        let mut ids: Vec<_> = self.panes.keys().copied().collect();
        ids.sort_unstable();
        let mut fresh = vec![];
        for id in ids {
            let pane = &self.panes[&id];
            let status = crate::api::agent::status(pane, &self.cfg);
            let occupant = pane.agent.as_ref().map(|a| a.instance);
            if let Some(edge) = self.autofocus.observe(id, status, occupant) {
                fresh.push(edge);
            }
        }
        if self.clients.is_empty() {
            return;
        }
        if self.cfg.auto_focus == Mode::Off {
            for client in self.clients.values_mut() {
                client.focus.queue.clear();
                client.focus.notified = None;
            }
            return;
        }
        let policy = Policy {
            mode: self.cfg.auto_focus,
            scope: self.cfg.auto_focus_scope,
            idle: Duration::from_millis(self.cfg.auto_focus_idle_ms),
            dwell: Duration::from_millis(self.cfg.auto_focus_dwell_ms),
        };
        let panes = self.autofocus_candidates();
        let mut client_ids: Vec<_> = self.clients.keys().copied().collect();
        client_ids.sort_unstable();
        let viewers = self.autofocus_viewers();
        for &cid in &client_ids {
            let Some(viewer) = viewers.iter().find(|v| v.id == cid) else {
                continue;
            };
            let focus = &mut self.clients.get_mut(&cid).unwrap().focus;
            focus
                .queue
                .retain(|edge| panes.iter().any(|p| eligible(*edge, p, viewer, policy)));
            for edge in &fresh {
                if panes.iter().any(|p| eligible(*edge, p, viewer, policy)) {
                    focus.queue.push(*edge);
                }
            }
        }
        for cid in client_ids {
            let viewers = self.autofocus_viewers();
            let Some(viewer) = viewers.iter().find(|v| v.id == cid) else {
                continue;
            };
            let focus = &mut self.clients.get_mut(&cid).unwrap().focus;
            let (target, notice) = decide(&focus.queue, &panes, viewer, &viewers, policy, now);
            if let Some(pid) = target {
                self.autofocus_jump(cid, pid, true, now);
                // Focus acknowledges Done. Re-observe that effective status
                // before considering another client on the next update.
                break;
            } else if let Some(edge) = notice {
                let client = self.clients.get_mut(&cid).unwrap();
                if client
                    .message
                    .as_ref()
                    .is_some_and(|m| now.saturating_duration_since(m.at) < m.ttl())
                {
                    continue;
                }
                let focus = &mut client.focus;
                if focus.notified != Some((edge.pane, edge.generation)) {
                    focus.notified = Some((edge.pane, edge.generation));
                    let lang = self.client_lang(cid);
                    let theme = self.client_theme(cid);
                    let attention =
                        crate::commands::attention_text(self, edge.pane, edge.status, &theme, lang);
                    let prefix = self.cfg.prefix_name();
                    let hint = trf(
                        lang,
                        "messages.auto_focus_queued",
                        &[
                            ("pane", &crate::api::pane_id_str(edge.pane)),
                            ("attention", &attention),
                            ("prefix", &prefix),
                        ],
                    );
                    self.message(cid, hint);
                }
            }
        }
    }

    fn autofocus_candidates(&self) -> Vec<Candidate> {
        self.autofocus
            .observed
            .iter()
            .filter_map(|(&id, observed)| {
                let (si, wi) = self.locate(id)?;
                let pane = self.panes.get(&id)?;
                let session = &self.sessions[si];
                let window = &session.windows[wi];
                Some(Candidate {
                    edge: Edge {
                        pane: id,
                        generation: observed.generation,
                        status: observed.status,
                    },
                    session: session.id,
                    window: window.id,
                    enabled: !pane.auto_focus_disabled
                        && pane.host_lost.is_none()
                        && crate::persist::restore_state(pane).is_none()
                        && crate::api::agent::status(pane, &self.cfg) == observed.status,
                    agent: pane.agent.is_some() || pane.detected_agent.is_some(),
                    changes_shared_view: session.active != wi
                        || window.active != id
                        || window.zoomed,
                })
            })
            .collect()
    }

    pub(crate) fn queued_autofocus_target(&self, cid: usize) -> Option<(usize, Status)> {
        let viewer = self.autofocus_viewers().into_iter().find(|v| v.id == cid)?;
        let panes = self.autofocus_candidates();
        let policy = Policy {
            mode: self.cfg.auto_focus,
            scope: self.cfg.auto_focus_scope,
            idle: Duration::from_millis(self.cfg.auto_focus_idle_ms),
            dwell: Duration::from_millis(self.cfg.auto_focus_dwell_ms),
        };
        queued_target(
            &self.clients.get(&cid)?.focus.queue,
            &panes,
            &viewer,
            policy,
        )
        .map(|p| (p.edge.pane, p.edge.status))
    }

    fn autofocus_viewers(&self) -> Vec<Viewer> {
        self.clients
            .iter()
            .filter_map(|(&id, c)| {
                let si = self.sidx(&c.session)?;
                let s = &self.sessions[si];
                let w = s.windows.get(s.active)?;
                Some(Viewer {
                    id,
                    session: s.id,
                    window: w.id,
                    pane: w.active,
                    last_input: c.focus.last_input,
                    last_jump: c.focus.last_jump,
                    guarded: c.prefix
                        || c.prompt.is_some()
                        || c.chooser.is_some()
                        || c.popup.is_some()
                        || c.drag.is_some()
                        || self.panes.get(&w.active).is_some_and(|p| p.copy.is_some()),
                })
            })
            .collect()
    }

    pub(crate) fn autofocus_jump(&mut self, cid: usize, pid: usize, automatic: bool, now: Instant) {
        let Some(si) = self.client_si(cid) else {
            return;
        };
        let Some((target_si, target_wi)) = self.locate(pid) else {
            return;
        };
        let origin = self.active_pane(si);
        if origin == pid {
            return;
        }
        let target = &self.sessions[target_si];
        let changes = target.active != target_wi
            || target.windows[target_wi].active != pid
            || target.windows[target_wi].zoomed;
        let affected: Vec<_> = self
            .autofocus_viewers()
            .into_iter()
            .filter(|v| v.id == cid || (changes && v.session == target.id))
            .collect();
        for viewer in affected {
            let focus = &mut self.clients.get_mut(&viewer.id).unwrap().focus;
            focus.prior = Some(viewer.pane);
            if automatic {
                focus.last_jump = Some(now);
            }
            focus.queue.retain(|e| e.pane != pid);
            focus.notified = None;
        }
        if target_si != si {
            self.switch_session(cid, target_si);
        }
        self.sessions[target_si].windows[target_wi].zoomed = false;
        self.focus_pane(pid); // Existing path unzooms; never writes to a pane.
        self.mark_dirty();
    }

    pub(crate) fn last_pane(&mut self, cid: usize) {
        let prior = self.clients.get(&cid).and_then(|c| c.focus.prior);
        if let Some(pid) = prior.filter(|pid| self.locate(*pid).is_some()) {
            self.autofocus_jump(cid, pid, false, Instant::now());
        } else {
            self.message(cid, tr(self.client_lang(cid), "messages.no_prior_pane"));
        }
    }
}

#[cfg(test)]
mod tests;
