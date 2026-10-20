//! State-file format (version 1): data types, validation, and conversion between live layout
//! trees and their saved form.
use crate::api;
use crate::layout::{Dir, Node};
use crate::server::{Pane, Session, State, Window};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub(super) const VERSION: u32 = 1;
/// Maximum tree depth (root leaf = 1). Deeper snapshots are rejected.
pub(super) const MAX_DEPTH: usize = 64;
const RATIO_MIN: f32 = 0.05;
const RATIO_MAX: f32 = 0.95;

/// Whole state file. `version` must be 1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub version: u32,
    #[serde(default)]
    pub next_workspace_id: usize,
    #[serde(default)]
    pub next_tab_id: usize,
    #[serde(default)]
    pub focused_workspace_id: Option<usize>,
    pub sessions: Vec<SessionSnap>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SessionSnap {
    /// Zero denotes a legacy snapshot without stable identifiers.
    #[serde(default)]
    pub id: usize,
    pub name: String,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub sidebar: bool,
    pub active_window: usize,
    pub windows: Vec<WindowSnap>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WindowSnap {
    #[serde(default)]
    pub id: usize,
    pub name: String,
    pub manual_name: bool,
    pub zoomed: bool,
    /// index of the focused pane in leaf order
    pub active: usize,
    pub layout: LayoutSnap,
}

/// Per-pane data kept for a leaf. Serialized as the `Leaf` variant's body.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct LeafSnap {
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub label: Option<String>,
    /// agent kind label (`AgentInfo::agent`)
    pub agent: Option<String>,
    /// unique live agent name (`AgentInfo::name`)
    pub agent_name: Option<String>,
    /// command that resumes the agent after a restart
    #[serde(default)]
    pub resume: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum LayoutSnap {
    Leaf(LeafSnap),
    Split {
        dir: Dir,
        ratio: f32,
        a: Box<LayoutSnap>,
        b: Box<LayoutSnap>,
    },
}

pub(super) fn serialize(snap: &Snapshot) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(snap)
}

/// Decode raw state-file bytes. Non-UTF-8 content is an error, as is anything `parse_snapshot`
/// rejects.
pub(super) fn decode_snapshot(bytes: &[u8]) -> Result<Snapshot, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not UTF-8: {e}"))?;
    parse_snapshot(text)
}

/// Parse and validate a state file. Pure.
pub(super) fn parse_snapshot(text: &str) -> Result<Snapshot, String> {
    let snap: Snapshot = serde_json::from_str(text).map_err(|e| format!("parse: {e}"))?;
    validate(&snap)?;
    Ok(snap)
}

/// Structural checks on a whole snapshot. Ratios must be finite; out-of-range finite values are
/// clamped later by `rebuild`.
pub(super) fn validate(snap: &Snapshot) -> Result<(), String> {
    if snap.version != VERSION {
        return Err(format!("unsupported version {}", snap.version));
    }
    let mut names: Vec<&str> = Vec::with_capacity(snap.sessions.len());
    let mut workspace_ids = HashSet::new();
    let mut tab_ids = HashSet::new();
    for s in &snap.sessions {
        if s.id != 0 && !workspace_ids.insert(s.id) {
            return Err(format!("duplicate workspace id {}", s.id));
        }
        crate::workspace::validate_env(&s.env).map_err(|e| e.message)?;
        if s.name.is_empty() {
            return Err("empty session name".into());
        }
        if names.contains(&s.name.as_str()) {
            return Err(format!("duplicate session name {:?}", s.name));
        }
        names.push(&s.name);
        if s.windows.is_empty() {
            return Err(format!("session {:?} has no windows", s.name));
        }
        if s.active_window >= s.windows.len() {
            return Err(format!("session {:?}: active_window out of range", s.name));
        }
        for w in &s.windows {
            if w.id != 0 && !tab_ids.insert(w.id) {
                return Err(format!("duplicate tab id {}", w.id));
            }
            let mut leaves = 0;
            check_layout(&w.layout, 1, &mut leaves)?;
            if w.active >= leaves {
                return Err(format!("window {:?}: active pane out of range", w.name));
            }
        }
    }
    if snap
        .focused_workspace_id
        .is_some_and(|id| id != 0 && !workspace_ids.contains(&id))
    {
        return Err("focused workspace id does not exist".into());
    }
    Ok(())
}

/// Allocate only missing legacy ids, after reserving every explicitly saved id. This also
/// retains high-water marks so removing the highest id cannot cause reuse after restart.
pub(super) fn assign_legacy_ids(snap: &mut Snapshot) -> Result<(), String> {
    validate(snap)?;
    let mut workspace = snap
        .next_workspace_id
        .max(snap.sessions.iter().map(|s| s.id).max().unwrap_or(0));
    let mut tab = snap.next_tab_id.max(
        snap.sessions
            .iter()
            .flat_map(|s| &s.windows)
            .map(|w| w.id)
            .max()
            .unwrap_or(0),
    );
    for s in &mut snap.sessions {
        if s.id == 0 {
            workspace = workspace
                .checked_add(1)
                .ok_or("workspace identifiers exhausted")?;
            s.id = workspace;
        }
        for w in &mut s.windows {
            if w.id == 0 {
                tab = tab.checked_add(1).ok_or("tab identifiers exhausted")?;
                w.id = tab;
            }
        }
    }
    snap.next_workspace_id = workspace;
    snap.next_tab_id = tab;
    Ok(())
}

fn check_ratio(ratio: f32) -> Result<(), String> {
    if ratio.is_finite() {
        Ok(())
    } else {
        Err(format!("non-finite split ratio {ratio}"))
    }
}

fn check_layout(layout: &LayoutSnap, depth: usize, leaves: &mut usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("layout deeper than {MAX_DEPTH}"));
    }
    match layout {
        LayoutSnap::Leaf(leaf) => {
            crate::workspace::validate_env(&leaf.env).map_err(|e| e.message)?;
            *leaves += 1;
            Ok(())
        }
        LayoutSnap::Split { ratio, a, b, .. } => {
            check_ratio(*ratio)?;
            check_layout(a, depth + 1, leaves)?;
            check_layout(b, depth + 1, leaves)
        }
    }
}

/// Build a `Node` tree from a snapshot tree. `leaf` is called once per leaf, in leaf order,
/// and returns the new pane id. The first error aborts the build.
pub(super) fn rebuild(
    layout: &LayoutSnap,
    leaf: &mut impl FnMut(&LeafSnap) -> Result<usize, String>,
) -> Result<Node, String> {
    rebuild_at(layout, leaf, 1)
}

fn rebuild_at(
    layout: &LayoutSnap,
    leaf: &mut impl FnMut(&LeafSnap) -> Result<usize, String>,
    depth: usize,
) -> Result<Node, String> {
    if depth > MAX_DEPTH {
        return Err(format!("layout deeper than {MAX_DEPTH}"));
    }
    match layout {
        LayoutSnap::Leaf(s) => Ok(Node::Leaf(leaf(s)?)),
        LayoutSnap::Split { dir, ratio, a, b } => {
            check_ratio(*ratio)?;
            let a = rebuild_at(a, leaf, depth + 1)?;
            let b = rebuild_at(b, leaf, depth + 1)?;
            Ok(Node::Split {
                dir: *dir,
                ratio: ratio.clamp(RATIO_MIN, RATIO_MAX),
                a: Box::new(a),
                b: Box::new(b),
            })
        }
    }
}

/// Keep a restored agent name only when it is a valid live name and not already claimed.
/// Kept names are recorded in `taken`. A rejected name is logged and becomes `None`.
pub(super) fn keep_agent_name(raw: Option<&str>, taken: &mut HashSet<String>) -> Option<String> {
    let name = raw?;
    if !api::is_agent_name(name) {
        crate::log!("restore: agent name {name:?} is invalid; dropped");
        return None;
    }
    if !taken.insert(name.to_string()) {
        crate::log!("restore: agent name {name:?} is already taken; dropped");
        return None;
    }
    Some(name.to_string())
}

/// Snapshot of the given sessions. `leaf` supplies the per-pane data for each pane id.
/// Sessions without windows are skipped.
pub(super) fn snapshot_of(sessions: &[Session], leaf: &impl Fn(usize) -> LeafSnap) -> Snapshot {
    let sessions: Vec<SessionSnap> = sessions
        .iter()
        .filter(|s| !s.windows.is_empty())
        .map(|s| SessionSnap {
            id: s.id,
            name: s.name.clone(),
            cwd: s.cwd.clone(),
            env: s.env.clone(),
            cols: s.cols,
            rows: s.rows,
            sidebar: s.sidebar,
            active_window: s.active.min(s.windows.len() - 1),
            windows: s.windows.iter().map(|w| window_snap(w, leaf)).collect(),
        })
        .collect();
    Snapshot {
        version: VERSION,
        next_workspace_id: sessions
            .iter()
            .map(|s: &SessionSnap| s.id)
            .max()
            .unwrap_or(0),
        next_tab_id: sessions
            .iter()
            .flat_map(|s| &s.windows)
            .map(|w| w.id)
            .max()
            .unwrap_or(0),
        focused_workspace_id: None,
        sessions,
    }
}

fn window_snap(w: &Window, leaf: &impl Fn(usize) -> LeafSnap) -> WindowSnap {
    let mut ids = vec![];
    w.layout.leaves(&mut ids);
    WindowSnap {
        id: w.id,
        name: w.name.clone(),
        manual_name: w.manual_name,
        zoomed: w.zoomed,
        active: ids.iter().position(|&id| id == w.active).unwrap_or(0),
        layout: layout_snap(&w.layout, leaf),
    }
}

/// Convert a live `Node` tree to its snapshot form.
fn layout_snap(node: &Node, leaf: &impl Fn(usize) -> LeafSnap) -> LayoutSnap {
    match node {
        Node::Leaf(id) => LayoutSnap::Leaf(leaf(*id)),
        Node::Split { dir, ratio, a, b } => LayoutSnap::Split {
            dir: *dir,
            ratio: *ratio,
            a: Box::new(layout_snap(a, leaf)),
            b: Box::new(layout_snap(b, leaf)),
        },
    }
}

pub(super) fn current_snapshot(st: &State) -> Snapshot {
    let mut snapshot = snapshot_of(&st.sessions, &|id| leaf_of(st.panes.get(&id)));
    snapshot.next_workspace_id = st.next_workspace_id;
    snapshot.next_tab_id = st.next_tab_id;
    snapshot.focused_workspace_id = st.target_session(None).ok().map(|si| st.sessions[si].id);
    snapshot
}

fn leaf_of(pane: Option<&Pane>) -> LeafSnap {
    let Some(p) = pane else {
        return LeafSnap::default();
    };
    let agent = p.agent.as_ref();
    LeafSnap {
        cwd: live_cwd(p),
        env: p.env.clone(),
        label: p.label.clone(),
        agent: agent.map(|a| a.agent.clone()),
        agent_name: agent.and_then(|a| a.name.clone()),
        resume: agent.map(|a| a.resume.clone()).unwrap_or_default(),
    }
}

/// Current directory of the pane's foreground process (Linux, via `/proc`). Falls back to the
/// process pid, then to the directory the pane was started in.
#[cfg(target_os = "linux")]
fn live_cwd(p: &Pane) -> Option<String> {
    let leader = p
        .master
        .process_group_leader()
        .and_then(|v| u32::try_from(v).ok());
    [leader, p.child.process_id()]
        .into_iter()
        .flatten()
        .find_map(|pid| std::fs::read_link(format!("/proc/{pid}/cwd")).ok())
        .map(|d| d.to_string_lossy().into_owned())
        .or_else(|| p.cwd.clone())
}

/// Other platforms: the directory the pane was started in.
#[cfg(not(target_os = "linux"))]
fn live_cwd(p: &Pane) -> Option<String> {
    p.cwd.clone()
}
