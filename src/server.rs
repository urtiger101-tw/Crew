//! The multiplexer server: owns sessions, windows, panes (ptys) and renders frames for clients.
use crate::commands::{self, Cmd};
use crate::config::{Config, config_path};
use crate::copy::{self, CopyState};
use crate::ipc;
use crate::keys;
use crate::layout::{Dir, Move, Node, Rect, neighbor};
use crate::proto::{ClientMsg, Command, ServerMsg, read_msg, write_msg};
use crate::render::{Frame, chooser_entries, render_client};
use anyhow::{Result, anyhow};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use interprocess::local_socket::prelude::*;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// One-shot process launch options; never persisted or inherited by split panes.
#[derive(Default)]
pub(crate) struct Launch {
    pub env: BTreeMap<String, String>,
    pub shell: Option<Vec<String>>,
}

pub struct Pane {
    pub parser: vt100::Parser,
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub rect: Rect,
    pub last_output: Instant,
    pub attention: bool,
    /// when the pane was spawned; startup output does not count as unseen work
    pub started: Instant,
    pub copy: Option<CopyState>,
    /// reported agent state (herdr-style `pane report-agent`)
    pub agent: Option<crate::api::AgentInfo>,
    /// Inferred process identity; never substitutes for a reported lifecycle authority.
    pub detected_agent: Option<crate::agent_detection::Detection>,
    /// user label (`pane rename`)
    pub label: Option<String>,
    /// working directory the pane was started in
    pub cwd: Option<String>,
    /// Explicit environment overrides, inherited by splits and persisted with the pane.
    pub env: BTreeMap<String, String>,
}

impl Pane {
    /// Run a vt100 operation on this pane, containing its panics (see [`contain_vt`]).
    pub fn vt<R>(
        &mut self,
        scrollback: usize,
        f: impl FnOnce(&mut vt100::Parser) -> R,
    ) -> Option<R> {
        let (rows, cols) = (self.rect.h, self.rect.w);
        let r = contain_vt(&mut self.parser, rows, cols, scrollback, f);
        if r.is_none() {
            self.copy = None;
        }
        r
    }

    pub fn busy(&self, cfg: &Config) -> bool {
        self.last_output.elapsed().as_secs_f32() < cfg.busy_secs
    }

    /// Name of the foreground process in this pane, if the platform lets us find it.
    pub fn foreground_name(&self) -> Option<String> {
        #[cfg(unix)]
        {
            let pid = self.master.process_group_leader()?;
            #[cfg(target_os = "linux")]
            {
                return std::fs::read_to_string(format!("/proc/{pid}/comm"))
                    .ok()
                    .map(|s| s.trim().to_string());
            }
            #[cfg(target_os = "macos")]
            {
                let mut buf = [0u8; 256];
                let n = unsafe {
                    libc::proc_name(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32)
                };
                if n > 0 {
                    return Some(String::from_utf8_lossy(&buf[..n as usize]).into_owned());
                }
                return None;
            }
            #[allow(unreachable_code)]
            {
                let _ = pid;
                None
            }
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
}

/// vt100 0.16.2 can panic on wide characters at the last column after a resize. Without
/// containment the panic would poison the server lock and every later request would fail.
/// On panic the terminal state is reset to an empty `rows`x`cols` screen; the shell keeps running.
pub fn contain_vt<R>(
    parser: &mut vt100::Parser,
    rows: u16,
    cols: u16,
    scrollback: usize,
    f: impl FnOnce(&mut vt100::Parser) -> R,
) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(parser))) {
        Ok(r) => Some(r),
        Err(_) => {
            crate::log!("vt100 panic contained; terminal reset to {cols}x{rows}");
            *parser = vt100::Parser::new(rows.max(2), cols.max(2), scrollback);
            None
        }
    }
}

pub struct Window {
    /// Stable tab identifier, independent of its display name and vector index.
    pub id: usize,
    pub name: String,
    pub manual_name: bool,
    pub layout: Node,
    pub active: usize,
    pub zoomed: bool,
}

impl Window {
    pub fn panes(&self) -> Vec<usize> {
        let mut v = vec![];
        self.layout.leaves(&mut v);
        v
    }
}

pub struct Session {
    /// Stable workspace identifier, preserved when renamed and restored.
    pub id: usize,
    pub name: String,
    pub windows: Vec<Window>,
    pub active: usize,
    pub cols: u16,
    pub rows: u16,
    pub sidebar: bool,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub last_used: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PromptKind {
    Command,
    RenameWindow,
    RenameSession,
    /// y/n question; `Prompt::pending` runs on `y`, anything else cancels
    Confirm,
}

pub struct Prompt {
    pub kind: PromptKind,
    /// input text; for `Confirm` the question shown (without the " (y/n) " suffix)
    pub buf: String,
    /// cursor position in `buf`, in chars (0..=buf.chars().count())
    pub cur: usize,
    /// command line run when a `Confirm` prompt is answered `y`
    pub pending: Option<String>,
    /// `:` history position while browsing with Up/Down (index into `State.history`)
    pub hist: Option<usize>,
    /// for `Confirm`: the pane that was active when asked; `y` refuses if it changed
    pub expect_pane: Option<usize>,
}

impl Prompt {
    /// Text prompt with the cursor at the end.
    pub fn new(kind: PromptKind, buf: String) -> Prompt {
        let cur = buf.chars().count();
        Prompt {
            kind,
            buf,
            cur,
            pending: None,
            hist: None,
            expect_pane: None,
        }
    }

    /// y/n prompt that runs `command` on `y`.
    pub fn confirm(question: String, command: String) -> Prompt {
        Prompt {
            kind: PromptKind::Confirm,
            buf: question,
            cur: 0,
            pending: Some(command),
            hist: None,
            expect_pane: None,
        }
    }
}

/// Transient status-bar message.
pub struct Message {
    pub text: String,
    pub at: Instant,
    /// errors render with the error token and stay longer
    pub error: bool,
    /// a success (copied, config reloaded): shown with the ok glyph; language independent
    pub ok: bool,
}

impl Message {
    pub fn ttl(&self) -> Duration {
        Duration::from_secs(if self.error { 5 } else { 3 })
    }
}

pub struct Client {
    pub session: String,
    pub sender: Box<dyn Write + Send>,
    pub prefix: bool,
    pub frame: Frame,
    pub prompt: Option<Prompt>,
    pub chooser: Option<usize>,
    pub popup: Option<(Vec<String>, usize)>,
    pub message: Option<Message>,
    /// mouse drag start: (pane, abs row, col), and whether we moved
    pub drag: Option<(usize, i64, u16, bool)>,
    /// terminal capabilities reported at attach (theme resolution)
    pub env: crate::theme::ClientEnv,
}

pub struct State {
    pub events: crate::events::Journal,
    pub metadata: crate::metadata::Store,
    /// Canonical checkouts reserved by an in-flight removal; pane creation shares this lock.
    pub removing_worktrees: std::collections::HashSet<std::path::PathBuf>,
    pub cfg: Config,
    pub bindings: BTreeMap<String, String>,
    pub buffer: Option<String>,
    /// `:` prompt history, oldest first (max 50)
    pub history: Vec<String>,
    pub panes: HashMap<usize, Pane>,
    pub sessions: Vec<Session>,
    pub clients: HashMap<usize, Client>,
    pub next_id: usize,
    /// Last allocated stable workspace/tab ids; also persisted after deletions.
    pub next_workspace_id: usize,
    pub next_tab_id: usize,
    pub dirty: bool,
    notify: mpsc::Sender<()>,
}

/// Output within this time after spawn (shell banner, prompt) is not "unseen work".
const ATTENTION_GRACE: Duration = Duration::from_secs(2);

pub type Shared = Arc<Mutex<State>>;

/// In-memory state for handler tests; no listener, PTY or background thread is started.
#[cfg(test)]
pub(crate) fn test_state() -> State {
    // legacy geometry (panes from row 0); toolbar tests turn it on explicitly
    let cfg = Config {
        toolbar: false,
        ..Config::default()
    };
    let (bindings, _) = cfg.bindings();
    let (notify, _) = mpsc::channel();
    State {
        events: crate::events::Journal::default(),
        metadata: crate::metadata::Store::default(),
        removing_worktrees: Default::default(),
        cfg,
        bindings,
        buffer: None,
        history: Vec::new(),
        panes: HashMap::new(),
        sessions: Vec::new(),
        clients: HashMap::new(),
        next_id: 0,
        next_workspace_id: 0,
        next_tab_id: 0,
        dirty: false,
        notify,
    }
}

/// Session `name` of `cols`x`rows` with one window holding pane id `pane` (no PTY behind it).
#[cfg(test)]
pub(crate) fn test_session(name: &str, cols: u16, rows: u16, pane: usize) -> Session {
    Session {
        id: 1,
        name: name.into(),
        windows: vec![Window {
            id: 1,
            name: "w".into(),
            manual_name: true,
            layout: Node::Leaf(pane),
            active: pane,
            zoomed: false,
        }],
        active: 0,
        cols,
        rows,
        sidebar: false,
        cwd: None,
        env: BTreeMap::new(),
        last_used: Instant::now(),
    }
}

/// Client attached to `session` that writes nowhere.
#[cfg(test)]
pub(crate) fn test_client(session: &str) -> Client {
    Client {
        session: session.into(),
        sender: Box::new(std::io::sink()),
        prefix: false,
        frame: Frame::default(),
        prompt: None,
        chooser: None,
        popup: None,
        message: None,
        drag: None,
        env: crate::theme::ClientEnv::default(),
    }
}

/// `pane-titles` policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleMode {
    Off,
    /// only in split, unzoomed windows, for panes at least 6 rows high that have an agent
    /// report or a label
    Auto,
    /// every pane of a split, unzoomed window
    Always,
}

/// Whether a pane gets a title row (UX-10 / ui-spec §3.4). Pure: `split` = the window has at
/// least 2 panes, `rect_h` = the pane's layout height including the title row.
pub fn wants_title(
    mode: TitleMode,
    split: bool,
    zoomed: bool,
    rect_h: u16,
    has_info: bool,
) -> bool {
    if !split || zoomed {
        return false;
    }
    match mode {
        TitleMode::Off => false,
        TitleMode::Always => rect_h >= 2,
        TitleMode::Auto => rect_h >= 6 && has_info,
    }
}

impl State {
    fn id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    pub fn sidebar_width(&self, s: &Session) -> u16 {
        if s.sidebar && s.cols > self.cfg.sidebar_width + 20 {
            self.cfg.sidebar_width
        } else {
            0
        }
    }

    /// Whether the toolbar row is drawn for `s`: the option is on and the session has at least
    /// 4 rows (toolbar, 2 pane rows, status bar).
    pub fn toolbar_shown(&self, s: &Session) -> bool {
        self.cfg.toolbar && s.rows >= 4
    }

    /// Columns and rows the panes share: below the toolbar (when shown), above the status
    /// row, left of the sidebar.
    pub fn pane_area(&self, s: &Session) -> Rect {
        let top = u16::from(self.toolbar_shown(s));
        Rect {
            x: 0,
            y: top,
            w: s.cols - self.sidebar_width(s),
            h: s.rows.saturating_sub(1 + top).max(1),
        }
    }

    /// Pane rects from the layout tree, before title rows are reserved. Zoom gives the active
    /// pane the whole area.
    fn layout_rects(&self, s: &Session, w: &Window) -> Vec<(usize, Rect)> {
        let area = self.pane_area(s);
        if w.zoomed {
            return vec![(w.active, area)];
        }
        let mut v = vec![];
        w.layout.layout(area, &mut v);
        v
    }

    fn title_mode(&self) -> TitleMode {
        match self.cfg.pane_titles.as_str() {
            "off" => TitleMode::Off,
            "always" => TitleMode::Always,
            _ => TitleMode::Auto,
        }
    }

    fn pane_has_info(&self, pid: usize) -> bool {
        self.panes.get(&pid).is_some_and(|p| {
            p.agent.is_some()
                || p.detected_agent.is_some()
                || p.label.as_deref().is_some_and(|l| !l.is_empty())
                || (!self.cfg.pane_template.is_empty()
                    && !self
                        .metadata
                        .values(crate::metadata::Key::Pane(pid), Instant::now())
                        .is_empty())
                || crate::metadata::display::pane(self, pid, crate::i18n::Lang::En).is_some_and(
                    |display| {
                        display.presentation.title.is_some()
                            || display.presentation.display_agent.is_some()
                    },
                )
        })
    }

    /// Panes whose layout rect starts with a title row.
    fn titled_panes(&self, w: &Window, rects: &[(usize, Rect)]) -> Vec<usize> {
        let mode = self.title_mode();
        let split = w.panes().len() >= 2;
        rects
            .iter()
            .filter(|(pid, r)| wants_title(mode, split, w.zoomed, r.h, self.pane_has_info(*pid)))
            .map(|(pid, _)| *pid)
            .collect()
    }

    /// Content rects of the visible panes. A titled pane's first row is its title, so its
    /// rect starts one row lower (PTY size and mouse hit-tests use this rect).
    pub fn visible_rects(&self, s: &Session, w: &Window) -> Vec<(usize, Rect)> {
        let rects = self.layout_rects(s, w);
        let titled = self.titled_panes(w, &rects);
        rects
            .into_iter()
            .map(|(pid, r)| {
                if titled.contains(&pid) {
                    (
                        pid,
                        Rect {
                            y: r.y + 1,
                            h: r.h - 1,
                            ..r
                        },
                    )
                } else {
                    (pid, r)
                }
            })
            .collect()
    }

    /// The title row of each titled pane (1 row, same columns as the pane).
    pub fn title_rows(&self, s: &Session, w: &Window) -> Vec<(usize, Rect)> {
        let rects = self.layout_rects(s, w);
        let titled = self.titled_panes(w, &rects);
        rects
            .into_iter()
            .filter(|(pid, _)| titled.contains(pid))
            .map(|(pid, r)| (pid, Rect { h: 1, ..r }))
            .collect()
    }

    pub(crate) fn sidx(&self, name: &str) -> Option<usize> {
        self.sessions.iter().position(|s| s.name == name)
    }

    pub(crate) fn client_si(&self, cid: usize) -> Option<usize> {
        self.clients.get(&cid).and_then(|c| self.sidx(&c.session))
    }

    pub(crate) fn active_pane(&self, si: usize) -> usize {
        let s = &self.sessions[si];
        s.windows[s.active].active
    }

    pub(crate) fn message(&mut self, cid: usize, msg: impl Into<String>) {
        self.set_message(cid, msg.into(), false);
    }

    pub(crate) fn error_message(&mut self, cid: usize, msg: impl Into<String>) {
        self.set_message(cid, msg.into(), true);
    }

    /// Success notice (copied, sourced): shown with the ok glyph.
    pub(crate) fn ok_message(&mut self, cid: usize, msg: impl Into<String>) {
        self.set_message(cid, msg.into(), false);
        if let Some(m) = self.clients.get_mut(&cid).and_then(|c| c.message.as_mut()) {
            m.ok = true;
        }
    }

    fn set_message(&mut self, cid: usize, text: String, error: bool) {
        if let Some(c) = self.clients.get_mut(&cid) {
            c.message = Some(Message {
                text,
                at: Instant::now(),
                error,
                ok: false,
            });
        }
        self.mark_dirty();
    }

    pub(crate) fn spawn_pane(
        &mut self,
        shared: &Shared,
        rows: u16,
        cols: u16,
        cwd: Option<&str>,
    ) -> Result<usize> {
        self.spawn_pane_with_env(shared, rows, cols, cwd, &BTreeMap::new())
    }

    pub(crate) fn spawn_pane_with_env(
        &mut self,
        shared: &Shared,
        rows: u16,
        cols: u16,
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
    ) -> Result<usize> {
        self.spawn_pane_with_shell(shared, rows, cols, cwd, env, None)
    }

    fn spawn_pane_with_shell(
        &mut self,
        shared: &Shared,
        rows: u16,
        cols: u16,
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
        startup_shell: Option<&[String]>,
    ) -> Result<usize> {
        let id = self.id();
        let pty = native_pty_system();
        let size = PtySize {
            rows: rows.max(2),
            cols: cols.max(2),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty.openpty(size).map_err(|e| anyhow!("{e}"))?;
        let shell = self.cfg.default_shell();
        let words = commands::tokenize(&shell);
        let words = startup_shell.unwrap_or(&words);
        let mut cmd = CommandBuilder::new(words.first().map(|s| s.as_str()).unwrap_or("sh"));
        for a in words.iter().skip(1) {
            cmd.arg(a);
        }
        cmd.env("TERM", "xterm-256color");
        for (key, value) in env {
            cmd.env(key, value);
        }
        cmd.env("CREW", format!("{id}"));
        cmd.env("CREW_PANE_ID", crate::api::pane_id_str(id));
        cmd.env("CREW_SOCKET", ipc::base_name());
        cmd.env("TMUXLIKE", format!("{id}"));
        cmd.env("TMUXLIKE_PANE_ID", crate::api::pane_id_str(id));
        cmd.env("TMUXLIKE_SOCKET", ipc::base_name());
        let start_dir = match cwd {
            Some(d) if std::path::Path::new(d).is_dir() => Some(d.to_string()),
            Some(d) => return Err(anyhow!("cwd is not a directory: {d}")),
            None => dirs::home_dir().map(|h| h.to_string_lossy().into_owned()),
        };
        if let Some(d) = &start_dir {
            let canonical = std::path::Path::new(d).canonicalize()?;
            if self
                .removing_worktrees
                .iter()
                .any(|path| canonical.starts_with(path))
            {
                return Err(anyhow!(
                    "worktree removal is in progress: {}",
                    canonical.display()
                ));
            }
            let process_cwd = crate::process_path::for_child(std::path::Path::new(d))
                .map_err(|error| anyhow!(error.message))?;
            cmd.cwd(process_cwd);
        }
        crate::log!("pane {id}: spawning {shell:?} {}x{}", size.cols, size.rows);
        let child = pair.slave.spawn_command(cmd).map_err(|e| {
            crate::log!("pane {id}: spawn failed: {e}");
            anyhow!("{e}")
        })?;
        crate::log!("pane {id}: spawned pid {:?}", child.process_id());
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| anyhow!("{e}"))?;
        let writer = pair.master.take_writer().map_err(|e| anyhow!("{e}"))?;
        let pane = Pane {
            parser: vt100::Parser::new(size.rows, size.cols, self.cfg.scrollback),
            master: pair.master,
            writer,
            child,
            rect: Rect {
                x: 0,
                y: 0,
                w: size.cols,
                h: size.rows,
            },
            last_output: Instant::now(),
            attention: false,
            started: Instant::now(),
            copy: None,
            agent: None,
            detected_agent: None,
            label: None,
            cwd: start_dir,
            env: env.clone(),
        };
        self.panes.insert(id, pane);
        self.events
            .record("pane.created", Some(id), serde_json::json!({}));
        let shared = shared.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16384];
            let mut logged = 0usize;
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        crate::log!("pane {id}: EOF");
                        break;
                    }
                    Err(e) => {
                        crate::log!("pane {id}: read error {e}");
                        break;
                    }
                    Ok(n) => {
                        if logged < 8 {
                            logged += 1;
                            crate::log!(
                                "pane {id}: read {n} bytes: {:?}",
                                String::from_utf8_lossy(&buf[..n.min(300)])
                            );
                        }
                        let mut st = crate::api::lock(&shared);
                        let scrollback = st.cfg.scrollback;
                        let Some(p) = st.panes.get_mut(&id) else {
                            return;
                        };
                        p.vt(scrollback, |vt| vt.process(&buf[..n]));
                        // Answer terminal queries (ConPTY and many apps block waiting for these).
                        let replies = crate::keys::query_replies(
                            &buf[..n],
                            p.parser.screen().cursor_position(),
                        );
                        if !replies.is_empty() {
                            crate::log!(
                                "pane {id}: answering queries {:?}",
                                String::from_utf8_lossy(&replies)
                            );
                            let _ = p.writer.write_all(&replies);
                            let _ = p.writer.flush();
                        }
                        p.last_output = Instant::now();
                        if p.started.elapsed() >= ATTENTION_GRACE {
                            p.attention = true;
                        }
                        st.events
                            .record("pane.output", Some(id), serde_json::json!({"bytes": n}));
                        st.dirty = true;
                        let _ = st.notify.send(());
                    }
                }
            }
            let mut st = crate::api::lock(&shared);
            st.pane_exited(id);
        });
        Ok(id)
    }

    fn shell_name(&self) -> String {
        let s = self.cfg.default_shell();
        let first = commands::tokenize(&s)
            .into_iter()
            .next()
            .unwrap_or_else(|| "sh".into());
        std::path::Path::new(&first)
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or(first)
    }

    pub(crate) fn new_session(
        &mut self,
        shared: &Shared,
        name: Option<String>,
        cols: u16,
        rows: u16,
        cwd: Option<String>,
    ) -> Result<String> {
        self.new_workspace(shared, name, (cols, rows), cwd, Launch::default(), true)
    }

    pub(crate) fn allocate_workspace_id(&mut self) -> Result<usize> {
        self.next_workspace_id = self
            .next_workspace_id
            .checked_add(1)
            .ok_or_else(|| anyhow!("workspace identifiers exhausted"))?;
        Ok(self.next_workspace_id)
    }

    pub(crate) fn allocate_tab_id(&mut self) -> Result<usize> {
        self.next_tab_id = self
            .next_tab_id
            .checked_add(1)
            .ok_or_else(|| anyhow!("tab identifiers exhausted"))?;
        Ok(self.next_tab_id)
    }

    pub(crate) fn new_workspace(
        &mut self,
        shared: &Shared,
        name: Option<String>,
        size: (u16, u16),
        cwd: Option<String>,
        launch: Launch,
        focus: bool,
    ) -> Result<String> {
        let Launch { env, shell } = launch;
        let name = match name {
            Some(n) => {
                if self.sidx(&n).is_some() {
                    return Err(anyhow!("duplicate session: {n}"));
                }
                n
            }
            None => (0..)
                .map(|i: u32| i.to_string())
                .find(|n| self.sidx(n).is_none())
                .unwrap(),
        };
        let id = self.allocate_workspace_id()?;
        let old_recent = self.sessions.iter().map(|s| s.last_used).min();
        let sidebar = self.cfg.sidebar;
        self.sessions.push(Session {
            id,
            name: name.clone(),
            windows: vec![],
            active: 0,
            cols: size.0.max(10),
            rows: size.1.max(3),
            sidebar,
            cwd,
            env,
            last_used: Instant::now(),
        });
        let si = self.sessions.len() - 1;
        if let Err(e) = self.new_tab(
            shared,
            si,
            None,
            None,
            Launch {
                shell,
                ..Launch::default()
            },
            true,
        ) {
            self.sessions.pop();
            return Err(e);
        }
        if !focus && let Some(old) = old_recent {
            self.sessions[si].last_used = old.checked_sub(Duration::from_nanos(1)).unwrap_or(old);
        }
        self.events.record(
            "workspace.created",
            None,
            serde_json::json!({
                "workspace_id": crate::workspace::workspace_id_str(id), "name": name,
            }),
        );
        Ok(name)
    }

    pub(crate) fn new_window(
        &mut self,
        shared: &Shared,
        si: usize,
        name: Option<String>,
    ) -> Result<()> {
        self.new_tab(shared, si, name, None, Launch::default(), true)
            .map(|_| ())
    }

    /// Create a tab with optional cwd/env overrides without changing another tab's identity.
    pub(crate) fn new_tab(
        &mut self,
        shared: &Shared,
        si: usize,
        name: Option<String>,
        cwd: Option<String>,
        launch: Launch,
        focus: bool,
    ) -> Result<usize> {
        let area = self.pane_area(&self.sessions[si]);
        let cwd = cwd.or_else(|| self.sessions[si].cwd.clone());
        let env = crate::workspace::merged_env(&self.sessions[si].env, launch.env);
        let tid = self.allocate_tab_id()?;
        let id = if env.is_empty() && launch.shell.is_none() {
            self.spawn_pane(shared, area.h, area.w, cwd.as_deref())?
        } else {
            self.spawn_pane_with_shell(
                shared,
                area.h,
                area.w,
                cwd.as_deref(),
                &env,
                launch.shell.as_deref(),
            )?
        };
        let manual = name.is_some();
        let name = name.unwrap_or_else(|| self.shell_name());
        let s = &mut self.sessions[si];
        s.windows.push(Window {
            id: tid,
            name,
            manual_name: manual,
            layout: Node::Leaf(id),
            active: id,
            zoomed: false,
        });
        if focus {
            s.active = s.windows.len() - 1;
        }
        self.events.record(
            "tab.created",
            Some(id),
            serde_json::json!({
                "workspace_id": crate::workspace::workspace_id_str(s.id),
                "tab_id": crate::workspace::tab_id_str(tid),
            }),
        );
        self.relayout(si);
        Ok(tid)
    }

    fn split(&mut self, shared: &Shared, si: usize, dir: Dir) -> Result<()> {
        let pid = self.active_pane(si);
        self.split_pane(shared, pid, dir, None, None, true)
            .map(|_| ())
    }

    /// Split pane `pid` (the new pane goes right/below). `ratio` = share of the existing pane.
    /// cwd defaults to the source pane's start dir. Returns the new pane id.
    pub(crate) fn split_pane(
        &mut self,
        shared: &Shared,
        pid: usize,
        dir: Dir,
        ratio: Option<f32>,
        cwd: Option<String>,
        focus: bool,
    ) -> Result<usize> {
        let (si, wi) = self.locate(pid).ok_or_else(|| anyhow!("pane not found"))?;
        let cur = self.panes.get(&pid).map(|p| p.rect).unwrap_or_default();
        let (h, wd) = if dir == Dir::H {
            (cur.h, cur.w / 2)
        } else {
            (cur.h / 2, cur.w)
        };
        let cwd = cwd
            .or_else(|| self.panes.get(&pid).and_then(|p| p.cwd.clone()))
            .or_else(|| self.sessions[si].cwd.clone());
        let env = self
            .panes
            .get(&pid)
            .map(|p| p.env.clone())
            .unwrap_or_default();
        let id = self.spawn_pane_with_env(shared, h, wd, cwd.as_deref(), &env)?;
        let w = &mut self.sessions[si].windows[wi];
        w.layout
            .split_with(pid, id, dir, ratio.unwrap_or(0.5).clamp(0.05, 0.95));
        w.zoomed = false;
        if focus {
            self.focus_pane(id);
        }
        self.relayout(si);
        Ok(id)
    }

    /// Make `pid` the focused pane of its window/session and mark it seen.
    pub(crate) fn focus_pane(&mut self, pid: usize) -> bool {
        let Some((si, wi)) = self.locate(pid) else {
            return false;
        };
        let s = &mut self.sessions[si];
        s.active = wi;
        s.last_used = Instant::now();
        let w = &mut s.windows[wi];
        if w.active != pid {
            w.zoomed = false;
        }
        w.active = pid;
        if let Some(p) = self.panes.get_mut(&pid) {
            p.attention = false;
        }
        self.relayout(si);
        true
    }

    /// Close one pane (kills its process) and clean up its window/session.
    pub(crate) fn kill_pane(&mut self, pid: usize) -> bool {
        if !self.panes.contains_key(&pid) {
            return false;
        }
        self.pane_exited(pid);
        true
    }

    pub fn relayout(&mut self, si: usize) {
        let s = &self.sessions[si];
        let mut all = vec![];
        for w in &s.windows {
            all.extend(self.visible_rects(s, w));
        }
        let scrollback = self.cfg.scrollback;
        for (id, r) in all {
            if let Some(p) = self.panes.get_mut(&id)
                && p.rect != r
                && r.w > 0
                && r.h > 0
            {
                p.rect = r;
                p.vt(scrollback, |vt| vt.screen_mut().set_size(r.h, r.w));
                let _ = p.master.resize(PtySize {
                    rows: r.h,
                    cols: r.w,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                if let Some(c) = &mut p.copy {
                    c.cx = c.cx.min(r.w - 1);
                    c.cy = c.cy.min(r.h - 1);
                }
            }
        }
        self.mark_dirty();
    }

    fn relayout_all(&mut self) {
        for si in 0..self.sessions.len() {
            self.relayout(si);
        }
        for c in self.clients.values_mut() {
            c.frame.invalidate();
        }
    }

    /// Recompute the layout of `pid`'s session (titles may appear/disappear after an agent
    /// report or rename, which changes the pane's rect and PTY size). Marks dirty either way.
    pub(crate) fn relayout_pane(&mut self, pid: usize) {
        match self.locate(pid) {
            Some((si, _)) => self.relayout(si),
            None => self.mark_dirty(),
        }
    }

    /// UI language for client `cid` (config `lang` + its locale); English when unknown.
    pub(crate) fn client_lang(&self, cid: usize) -> crate::i18n::Lang {
        let env = self.clients.get(&cid).and_then(|c| c.env.lang);
        crate::i18n::Lang::resolve(&self.cfg.lang, env)
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
        let _ = self.notify.send(());
    }

    pub(crate) fn locate(&self, pid: usize) -> Option<(usize, usize)> {
        for (si, s) in self.sessions.iter().enumerate() {
            for (wi, w) in s.windows.iter().enumerate() {
                if w.layout.contains(pid) {
                    return Some((si, wi));
                }
            }
        }
        None
    }

    /// Remove a pane (after its process exited or it was killed) and clean up empty windows/sessions.
    pub fn pane_exited(&mut self, pid: usize) {
        if let Some(mut p) = self.panes.remove(&pid) {
            self.events
                .record("pane.closed", Some(pid), serde_json::json!({}));
            let _ = p.child.kill();
        }
        let Some((si, wi)) = self.locate(pid) else {
            return;
        };
        let s = &mut self.sessions[si];
        let w = &mut s.windows[wi];
        if matches!(w.layout, Node::Leaf(_)) {
            let closed = s.windows.remove(wi);
            self.events.record(
                "tab.closed",
                Some(pid),
                serde_json::json!({
                    "workspace_id": crate::workspace::workspace_id_str(s.id),
                    "tab_id": crate::workspace::tab_id_str(closed.id),
                }),
            );
            if (wi < s.active || s.active >= s.windows.len()) && s.active > 0 {
                s.active -= 1;
            }
        } else {
            w.layout.remove(pid);
            if w.active == pid {
                w.active = w.panes()[0];
            }
            w.zoomed = false;
        }
        if self.sessions[si].windows.is_empty() {
            let closed = self.sessions.remove(si);
            self.events.record("workspace.closed", None, serde_json::json!({
                "workspace_id": crate::workspace::workspace_id_str(closed.id), "name": closed.name,
            }));
            self.drop_clients_of(&closed.name, "exited");
        } else {
            self.relayout(si);
        }
        self.mark_dirty();
    }

    fn drop_clients_of(&mut self, session: &str, reason: &str) {
        // Move clients to another session if one exists (like tmux detach-on-destroy off)
        let other = self
            .sessions
            .iter()
            .max_by_key(|s| (s.last_used, s.id))
            .map(|s| s.name.clone());
        let ids: Vec<usize> = self
            .clients
            .iter()
            .filter(|(_, c)| c.session == session)
            .map(|(i, _)| *i)
            .collect();
        for id in ids {
            let lang = self.client_lang(id);
            if let Some(o) = &other {
                let c = self.clients.get_mut(&id).unwrap();
                c.session = o.clone();
                c.frame.invalidate();
                c.message = Some(Message {
                    text: commands::session_ended_text(
                        &crate::theme::Theme::resolve(&self.cfg.theme, &c.env),
                        lang,
                        session,
                        o,
                    ),
                    at: Instant::now(),
                    error: false,
                    ok: false,
                });
                continue;
            }
            if let Some(mut c) = self.clients.remove(&id) {
                let text = commands::localize(lang, reason);
                let _ = write_msg(&mut c.sender, &ServerMsg::Exit(text));
            }
        }
    }

    pub(crate) fn kill_session(&mut self, si: usize) {
        let s = self.sessions.remove(si);
        for w in &s.windows {
            self.events.record(
                "tab.closed",
                None,
                serde_json::json!({
                    "workspace_id": crate::workspace::workspace_id_str(s.id),
                    "tab_id": crate::workspace::tab_id_str(w.id),
                }),
            );
            for pid in w.panes() {
                if let Some(mut p) = self.panes.remove(&pid) {
                    self.events
                        .record("pane.closed", Some(pid), serde_json::json!({}));
                    let _ = p.child.kill();
                }
            }
        }
        self.events.record(
            "workspace.closed",
            None,
            serde_json::json!({
                "workspace_id": crate::workspace::workspace_id_str(s.id), "name": s.name,
            }),
        );
        self.drop_clients_of(&s.name, "session killed");
        self.mark_dirty();
    }

    pub(crate) fn write_pane(&mut self, pid: usize, bytes: &[u8]) {
        if let Some(p) = self.panes.get_mut(&pid) {
            if p.copy.is_some() {
                p.copy = None;
                p.parser.screen_mut().set_scrollback(0);
                self.dirty = true;
            }
            let _ = p.writer.write_all(bytes);
            let _ = p.writer.flush();
        }
    }

    pub(crate) fn paste(&mut self, pid: usize, text: &str) {
        let bp = self
            .panes
            .get(&pid)
            .map(|p| p.parser.screen().bracketed_paste())
            .unwrap_or(false);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let data = if bp {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        self.write_pane(pid, data.as_bytes());
    }

    pub(crate) fn set_clipboard(&mut self, cid: usize, text: String) {
        let n = text.chars().count();
        if self.cfg.osc52 {
            let seq = copy::osc52(&text);
            for c in self.clients.values_mut() {
                let _ = write_msg(&mut c.sender, &ServerMsg::Output(seq.clone()));
            }
        }
        self.buffer = Some(text);
        let lang = self.client_lang(cid);
        let copied = crate::i18n::trf(
            lang,
            "messages.copied",
            &[("n", &n as &dyn std::fmt::Display)],
        );
        self.ok_message(cid, copied);
    }

    pub(crate) fn switch_session(&mut self, cid: usize, ni: usize) {
        let Some(si) = self.client_si(cid) else {
            return;
        };
        let (cols, rows) = (self.sessions[si].cols, self.sessions[si].rows);
        let s = &mut self.sessions[ni];
        s.cols = cols;
        s.rows = rows;
        s.last_used = Instant::now();
        let name = s.name.clone();
        if let Some(c) = self.clients.get_mut(&cid) {
            c.session = name;
            c.frame.invalidate();
        }
        self.relayout(ni);
    }

    pub fn reload_config(&mut self, path: Option<&str>) -> Result<String, String> {
        let p = match path {
            Some(p) => std::path::PathBuf::from(p),
            None => config_path().ok_or("no config path")?,
        };
        let is_toml = p.extension().is_some_and(|e| e == "toml");
        if is_toml {
            let cfg = if p.exists() {
                Config::load_from(&p)?
            } else {
                Config::default()
            };
            self.cfg = cfg;
            let (b, errs) = self.cfg.bindings();
            self.bindings = b;
            let mut errs = errs;
            for c in self.cfg.commands.clone() {
                if let Err(e) = commands::parse(&c).and_then(|cmd| self.exec_global(cmd)) {
                    errs.push(e);
                }
            }
            self.relayout_all();
            if errs.is_empty() {
                Ok(format!("sourced {}", p.display()))
            } else {
                Err(errs.join("; "))
            }
        } else {
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let mut errs = vec![];
            for line in text.lines() {
                if commands::tokenize(line).is_empty() {
                    continue;
                }
                if let Err(e) = commands::parse(line).and_then(|cmd| self.exec_global(cmd)) {
                    errs.push(e);
                }
            }
            self.relayout_all();
            if errs.is_empty() {
                Ok(format!("sourced {}", p.display()))
            } else {
                Err(errs.join("; "))
            }
        }
    }

    pub(crate) fn target_session(&self, target: Option<&str>) -> Result<usize, String> {
        match target {
            Some(t) => {
                let name = t.split([':', '.']).next().unwrap_or(t);
                self.sidx(name)
                    .ok_or_else(|| format!("can't find session: {name}"))
            }
            None => self
                .sessions
                .iter()
                .enumerate()
                .max_by_key(|(_, s)| (s.last_used, s.id))
                .map(|(i, _)| i)
                .ok_or_else(|| "no sessions".to_string()),
        }
    }

    /// Visible text of the active pane of a session (like `tmux capture-pane -p`).
    pub fn capture(&self, target: Option<&str>) -> Result<String, String> {
        let si = self.target_session(target)?;
        let p = self.panes.get(&self.active_pane(si)).ok_or("no pane")?;
        let mut out = String::new();
        for line in p.parser.screen().contents().lines() {
            out.push_str(line.trim_end());
            out.push('\n');
        }
        while out.ends_with("\n\n") {
            out.pop();
        }
        Ok(out)
    }

    /// Commands that do not need a client/session.
    fn exec_global(&mut self, cmd: Cmd) -> Result<(), String> {
        match cmd {
            Cmd::Set(k, v) => {
                self.cfg.set(&k, &v)?;
                if k == "persist" && !self.cfg.persist {
                    crate::persist::clear();
                }
                if k == "sidebar" {
                    let sb = self.cfg.sidebar;
                    for s in &mut self.sessions {
                        s.sidebar = sb;
                    }
                }
                self.relayout_all();
            }
            Cmd::BindKey(k, c) => {
                self.bindings.insert(k, c);
            }
            Cmd::UnbindKey(k) => {
                if k == "*" {
                    self.bindings.clear();
                } else {
                    self.bindings.remove(&k);
                }
            }
            Cmd::SourceFile(p) => {
                self.reload_config(p.as_deref())?;
            }
            Cmd::SendKeys {
                target,
                literal,
                keys,
            } => {
                let si = self.target_session(target.as_deref())?;
                let pid = self.active_pane(si);
                let bytes: Vec<u8> = keys
                    .iter()
                    .flat_map(|k| commands::send_key_bytes(k, literal))
                    .collect();
                self.write_pane(pid, &bytes);
            }
            Cmd::CapturePane { target, .. } => {
                let text = self.capture(target.as_deref())?;
                self.buffer = Some(text);
            }
            Cmd::KillSession { target: Some(t) } => {
                let si = self.session_index_for_kill(&t)?;
                self.kill_session(si);
            }
            Cmd::KillWindow { target: Some(t) } => {
                let (si, wi) = self.window_for(None, Some(t.as_str()))?;
                self.kill_window_at(si, wi);
            }
            Cmd::NextAttention { backward } => {
                commands::headless_attention(self, backward)?;
            }
            Cmd::ConfirmBefore { command, .. } => {
                // no client to ask: run the command as typed
                self.exec_global(commands::parse(&command)?)?;
            }
            other => return Err(format!("\"{}\" needs an attached client", other.name())),
        }
        Ok(())
    }

    pub fn exec(&mut self, shared: &Shared, cid: usize, cmd: Cmd) -> Result<(), String> {
        let Some(si) = self.client_si(cid) else {
            return self.exec_global(cmd);
        };
        let e = |e: anyhow::Error| e.to_string();
        match cmd {
            Cmd::SplitWindow { horizontal } => self
                .split(shared, si, if horizontal { Dir::H } else { Dir::V })
                .map_err(e)?,
            Cmd::NewWindow { name } => self.new_window(shared, si, name).map_err(e)?,
            Cmd::RenameWindow(None) | Cmd::RenameSession(None) | Cmd::CommandPrompt => {
                let s = &self.sessions[si];
                let (kind, buf) = match cmd {
                    Cmd::RenameWindow(_) => {
                        (PromptKind::RenameWindow, s.windows[s.active].name.clone())
                    }
                    Cmd::RenameSession(_) => (PromptKind::RenameSession, s.name.clone()),
                    _ => (PromptKind::Command, String::new()),
                };
                if let Some(c) = self.clients.get_mut(&cid) {
                    c.prompt = Some(Prompt::new(kind, buf));
                }
            }
            Cmd::RenameWindow(Some(n)) => {
                let s = &mut self.sessions[si];
                let w = &mut s.windows[s.active];
                w.manual_name = !n.is_empty();
                if !n.is_empty() {
                    w.name = n;
                }
            }
            Cmd::RenameSession(Some(n)) => {
                if n.is_empty() || n.contains(':') {
                    return Err("invalid session name".into());
                }
                if self.sidx(&n).is_some_and(|i| i != si) {
                    return Err(format!("duplicate session: {n}"));
                }
                let old = std::mem::replace(&mut self.sessions[si].name, n.clone());
                for c in self.clients.values_mut() {
                    if c.session == old {
                        c.session = n.clone();
                    }
                }
            }
            Cmd::KillPane => {
                let pid = self.active_pane(si);
                self.pane_exited(pid);
            }
            Cmd::KillWindow { target } => {
                let (s, w) = self.window_for(Some(si), target.as_deref())?;
                self.kill_window_at(s, w);
            }
            Cmd::KillSession { target } => {
                let s = self.kill_session_target(si, target.as_deref())?;
                self.kill_session(s);
            }
            Cmd::NextAttention { backward } => self.next_attention(cid, si, backward),
            Cmd::ConfirmBefore { prompt, command } => {
                if !self.cfg.confirm_close {
                    // confirm-close off: the command runs at once, as if typed
                    return self.exec(shared, cid, commands::parse(&command)?);
                }
                // the target is bound now: `y` later runs `pending`, and kill-pane also
                // refuses when the active pane has moved (`expect_pane`)
                let target = self.confirm_target(si, &command)?;
                let lang = self.client_lang(cid);
                let text = match prompt {
                    Some(p) => p,
                    None => self.confirm_prompt_text(si, &command, lang)?,
                };
                if let Some(c) = self.clients.get_mut(&cid) {
                    let mut p = Prompt::confirm(text, target.pending);
                    p.expect_pane = target.expect_pane;
                    c.prompt = Some(p);
                }
            }
            Cmd::KillServer => {
                crate::persist::clear();
                while !self.sessions.is_empty() {
                    self.kill_session(0);
                }
                std::process::exit(0);
            }
            Cmd::SelectPane(m) => {
                let s = &self.sessions[si];
                let w = &s.windows[s.active];
                let rects = self.visible_rects(s, w);
                if let Some(n) = neighbor(&rects, w.active, m) {
                    let s = &mut self.sessions[si];
                    let w = &mut s.windows[s.active];
                    w.active = n;
                    w.zoomed = false;
                    self.relayout(si);
                }
            }
            Cmd::NextPane => {
                let s = &mut self.sessions[si];
                let w = &mut s.windows[s.active];
                let panes = w.panes();
                let i = panes.iter().position(|p| *p == w.active).unwrap_or(0);
                w.active = panes[(i + 1) % panes.len()];
                if w.zoomed {
                    self.relayout(si);
                }
            }
            Cmd::ResizePane(m, n) => {
                let area = self.pane_area(&self.sessions[si]);
                let s = &mut self.sessions[si];
                let w = &mut s.windows[s.active];
                let (dir, sign, total) = match m {
                    Move::Left => (Dir::H, -1.0, area.w),
                    Move::Right => (Dir::H, 1.0, area.w),
                    Move::Up => (Dir::V, -1.0, area.h),
                    Move::Down => (Dir::V, 1.0, area.h),
                };
                let target = w.active;
                w.layout
                    .resize(target, dir, sign * n as f32 / total.max(1) as f32);
                self.relayout(si);
            }
            Cmd::Zoom => {
                let s = &mut self.sessions[si];
                let w = &mut s.windows[s.active];
                w.zoomed = !w.zoomed && w.panes().len() > 1;
                self.relayout(si);
            }
            Cmd::NextWindow | Cmd::PrevWindow | Cmd::SelectWindow(_) => {
                let s = &mut self.sessions[si];
                let n = s.windows.len();
                s.active = match cmd {
                    Cmd::NextWindow => (s.active + 1) % n,
                    Cmd::PrevWindow => (s.active + n - 1) % n,
                    Cmd::SelectWindow(i) if i < n => i,
                    _ => return Err("no such window".into()),
                };
                self.relayout(si);
            }
            Cmd::CopyMode { page_up } => {
                let pid = self.active_pane(si);
                if let Some(p) = self.panes.get_mut(&pid) {
                    let (cy, cx) = p.parser.screen().cursor_position();
                    let c = p.copy.get_or_insert(CopyState {
                        cx,
                        cy,
                        anchor: None,
                        line: false,
                    });
                    if page_up {
                        let off = p.parser.screen().scrollback() + p.rect.h as usize;
                        p.parser.screen_mut().set_scrollback(off);
                        c.cy = 0;
                    }
                }
            }
            Cmd::PasteBuffer => {
                if let Some(b) = self.buffer.clone() {
                    let pid = self.active_pane(si);
                    self.paste(pid, &b);
                } else {
                    return Err("no buffer".into());
                }
            }
            Cmd::Detach => {
                let lang = self.client_lang(cid);
                if let Some(mut c) = self.clients.remove(&cid) {
                    let text = crate::i18n::trf(
                        lang,
                        "messages.detached",
                        &[("session", &c.session as &dyn std::fmt::Display)],
                    );
                    let _ = write_msg(&mut c.sender, &ServerMsg::Exit(text));
                }
            }
            Cmd::ChooseTree => {
                let entries = chooser_entries(self);
                let cur = self.sessions[si].active;
                let sel = entries
                    .iter()
                    .position(|e| e.0 == si && e.1 == Some(cur))
                    .unwrap_or(0);
                if let Some(c) = self.clients.get_mut(&cid) {
                    c.chooser = Some(sel);
                }
            }
            Cmd::ToggleSidebar => {
                let s = &mut self.sessions[si];
                s.sidebar = !s.sidebar;
                self.relayout(si);
                if let Some(c) = self.clients.get_mut(&cid) {
                    c.frame.invalidate();
                }
            }
            Cmd::SourceFile(p) => {
                let lang = self.client_lang(cid);
                let m = self.reload_config(p.as_deref())?;
                self.ok_message(cid, commands::localize(lang, &m));
            }
            Cmd::SwitchClient { next } => {
                let n = self.sessions.len();
                if n > 1 {
                    let ni = if next { (si + 1) % n } else { (si + n - 1) % n };
                    self.switch_session(cid, ni);
                }
            }
            Cmd::SendPrefix => {
                let (c, ctrl) = {
                    let p = self.cfg.prefix_name();
                    match p.strip_prefix("C-") {
                        Some(r) if r.len() == 1 => (r.chars().next().unwrap(), true),
                        _ => (p.chars().next().unwrap_or('b'), false),
                    }
                };
                let k = KeyEvent::new(
                    KeyCode::Char(c),
                    if ctrl {
                        KeyModifiers::CONTROL
                    } else {
                        KeyModifiers::NONE
                    },
                );
                let pid = self.active_pane(si);
                self.write_pane(pid, &keys::encode(&k, false));
            }
            Cmd::DisplayMessage(m) => self.message(cid, m),
            Cmd::ListKeys => {
                let lines = commands::key_lines(self, self.client_lang(cid));
                if let Some(c) = self.clients.get_mut(&cid) {
                    c.popup = Some((lines, 0));
                }
            }
            other => self.exec_global(other)?,
        }
        self.mark_dirty();
        Ok(())
    }
}

// The caller must keep this State lock through client registration. A daemon
// restart may reuse an ID even when the replacement also has the old name.
fn workspace_attach_name(st: &State, id: usize, epoch: Option<&str>) -> Result<String> {
    if epoch.is_some_and(|expected| expected != st.events.epoch()) {
        return Err(anyhow!(
            "server changed since selection; refresh federation and select again"
        ));
    }
    st.sessions
        .iter()
        .find(|session| session.id == id)
        .map(|s| s.name.clone())
        .ok_or_else(|| anyhow!("can't find workspace: w{id}"))
}

fn handle_conn(shared: Shared, cid: usize, stream: interprocess::local_socket::Stream) {
    let (mut r, mut w) = stream.split();
    let Ok(ClientMsg::Hello(cmd)) = read_msg::<ClientMsg>(&mut r) else {
        return;
    };
    // Keep both the stable ID and the observed daemon epoch until the same lock
    // that registers the client. Separate preflight reads could race a restart.
    let (cmd, workspace_id, expected_epoch) = match cmd {
        Command::AttachWorkspaceEpoch {
            workspace_id,
            epoch,
            cols,
            rows,
            env,
        } => (
            Command::Attach {
                session: None,
                create: false,
                cols,
                rows,
                cwd: None,
                detached: false,
                env,
            },
            Some(workspace_id),
            Some(epoch),
        ),
        Command::AttachWorkspace {
            workspace_id,
            cols,
            rows,
            env,
        } => (
            Command::Attach {
                session: None,
                create: false,
                cols,
                rows,
                cwd: None,
                detached: false,
                env,
            },
            Some(workspace_id),
            None,
        ),
        other => (other, None, None),
    };
    match cmd {
        Command::List => {
            let st = crate::api::lock(&shared);
            let mut out = String::new();
            for s in &st.sessions {
                let n = st.clients.values().filter(|c| c.session == s.name).count();
                out += &format!(
                    "{}: {} windows ({}x{}){}\n",
                    s.name,
                    s.windows.len(),
                    s.cols,
                    s.rows,
                    if n > 0 { " (attached)" } else { "" }
                );
            }
            let _ = write_msg(&mut w, &ServerMsg::Text(out));
        }
        Command::Kill { session } => {
            let mut st = crate::api::lock(&shared);
            match st.sidx(&session) {
                Some(si) => {
                    st.kill_session(si);
                    let _ = write_msg(&mut w, &ServerMsg::Text(String::new()));
                }
                None => {
                    let _ = write_msg(
                        &mut w,
                        &ServerMsg::Error(format!("can't find session: {session}")),
                    );
                }
            }
        }
        Command::KillServer => {
            let mut st = crate::api::lock(&shared);
            // under the lock so the render tick cannot autosave it back before exit
            crate::persist::clear();
            while !st.sessions.is_empty() {
                st.kill_session(0);
            }
            let _ = write_msg(&mut w, &ServerMsg::Text(String::new()));
            std::process::exit(0);
        }
        Command::Api(req) => {
            let stop = matches!(req, crate::api::Request::ServerStop);
            // handlers lock `shared` themselves so waits can release it between polls
            let reply = crate::api::handle(&shared, req);
            let _ = write_msg(&mut w, &ServerMsg::Api(reply));
            if stop {
                // the state file was saved by the handler; pane processes do not survive
                let mut st = crate::api::lock(&shared);
                while !st.sessions.is_empty() {
                    st.kill_session(0);
                }
                std::process::exit(0);
            }
        }
        Command::Run { line } => {
            let mut st = crate::api::lock(&shared);
            let res = commands::parse(&line).and_then(|c| match c {
                Cmd::CapturePane {
                    target,
                    print: true,
                } => st.capture(target.as_deref()),
                Cmd::DebugFrame { target, opts } => {
                    commands::debug_frame(&mut st, target.as_deref(), &opts)
                }
                // headless: the CLI gets the chosen pane on its own line
                Cmd::NextAttention { backward } => {
                    commands::headless_attention(&mut st, backward).map(|t| format!("{t}\n"))
                }
                c => st.exec_global(c).map(|_| String::new()),
            });
            let msg = match res {
                Ok(t) => ServerMsg::Text(t),
                Err(e) => ServerMsg::Error(e),
            };
            let _ = write_msg(&mut w, &msg);
        }
        Command::Attach {
            session,
            create,
            cols,
            rows,
            cwd,
            detached,
            env,
        } => {
            let mut st = crate::api::lock(&shared);
            let name = match workspace_id {
                Some(id) => workspace_attach_name(&st, id, expected_epoch.as_deref()),
                None => match session {
                    Some(n) if st.sidx(&n).is_some() && !create => Ok(n),
                    Some(n) if !create => Err(anyhow!("can't find session: {n}")),
                    None if !create => st
                        .sessions
                        .iter()
                        .max_by_key(|s| (s.last_used, s.id))
                        .map(|s| s.name.clone())
                        .ok_or_else(|| anyhow!("no sessions")),
                    n => st.new_session(&shared, n, cols, rows, cwd),
                },
            };
            let name = match name {
                Ok(n) => n,
                Err(e) => {
                    let _ = write_msg(&mut w, &ServerMsg::Error(e.to_string()));
                    return;
                }
            };
            let si = st.sidx(&name).unwrap();
            if detached {
                let _ = write_msg(&mut w, &ServerMsg::Text(String::new()));
                return;
            }
            {
                let s = &mut st.sessions[si];
                s.cols = cols.max(10);
                s.rows = rows.max(3);
                s.last_used = Instant::now();
            }
            st.relayout(si);
            st.clients.insert(
                cid,
                Client {
                    session: name,
                    sender: Box::new(w),
                    prefix: false,
                    frame: Frame::default(),
                    prompt: None,
                    chooser: None,
                    popup: None,
                    message: None,
                    drag: None,
                    env,
                },
            );
            let first = commands::first_attach();
            if first && st.cfg.hints {
                commands::attach_hint(&mut st, cid);
            }
            st.mark_dirty();
            drop(st);
            while let Ok(msg) = read_msg::<ClientMsg>(&mut r) {
                if let ClientMsg::Event(ev) = msg {
                    let mut st = crate::api::lock(&shared);
                    if !st.clients.contains_key(&cid) {
                        break;
                    }
                    st.handle_event(&shared, cid, ev);
                }
            }
            crate::api::lock(&shared).clients.remove(&cid);
        }
        Command::AttachWorkspace { .. } | Command::AttachWorkspaceEpoch { .. } => {
            unreachable!("workspace attachment was normalized above")
        }
    }
}

pub fn run() -> Result<()> {
    crate::log!("server starting, pid {}", std::process::id());
    let listener = ipc::listen().inspect_err(|e| crate::log!("listen failed: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let cfg = Config::load();
    let (bindings, _) = cfg.bindings();
    let shared: Shared = Arc::new(Mutex::new(State {
        events: crate::events::Journal::default(),
        metadata: crate::metadata::Store::default(),
        removing_worktrees: Default::default(),
        cfg,
        bindings,
        buffer: None,
        history: Vec::new(),
        panes: HashMap::new(),
        sessions: vec![],
        clients: HashMap::new(),
        next_id: 0,
        next_workspace_id: 0,
        next_tab_id: 0,
        dirty: false,
        notify: tx,
    }));
    {
        let mut st = crate::api::lock(&shared);
        st.events.record(
            "server.started",
            None,
            serde_json::json!({"version": env!("CARGO_PKG_VERSION")}),
        );
        for c in st.cfg.commands.clone() {
            let _ = commands::parse(&c).and_then(|cmd| st.exec_global(cmd));
        }
    }
    if crate::api::lock(&shared).cfg.persist {
        match crate::persist::restore_on_start(&shared) {
            Ok(n) => crate::log!("restored {n} sessions"),
            Err(e) => crate::log!("restore failed: {e}"),
        }
    }

    // Render / housekeeping thread.
    let sh = shared.clone();
    std::thread::spawn(move || {
        let mut ever_had_session = false;
        let mut last_tick = Instant::now();
        let mut last_saved: Option<String> = None;
        loop {
            let _ = rx.recv_timeout(Duration::from_millis(500));
            std::thread::sleep(Duration::from_millis(8));
            while rx.try_recv().is_ok() {}
            if last_tick.elapsed() > Duration::from_millis(500) {
                crate::agent_detection::refresh(&sh);
            }
            let mut st = crate::api::lock(&sh);
            let tick = last_tick.elapsed() > Duration::from_millis(500);
            if tick {
                last_tick = Instant::now();
                crate::metadata::sweep(&mut st, last_tick);
                // detect exited children (ConPTY may not deliver EOF)
                let dead: Vec<usize> = st
                    .panes
                    .iter_mut()
                    .filter_map(|(id, p)| match p.child.try_wait() {
                        Ok(Some(status)) => {
                            crate::log!("pane {id}: child exited {status:?}");
                            Some(*id)
                        }
                        _ => None,
                    })
                    .collect();
                for id in dead {
                    st.pane_exited(id);
                }
                // automatic window names
                if st.cfg.automatic_rename {
                    let st = &mut *st;
                    for s in &mut st.sessions {
                        for w in &mut s.windows {
                            if w.manual_name {
                                continue;
                            }
                            if let Some(n) =
                                st.panes.get(&w.active).and_then(|p| p.foreground_name())
                                && !n.is_empty()
                                && n != w.name
                            {
                                w.name = n;
                                st.dirty = true;
                            }
                        }
                    }
                }
                if st.cfg.persist {
                    crate::persist::autosave(&st, &mut last_saved);
                }
                for c in st.clients.values_mut() {
                    if c.message.as_ref().is_some_and(|m| m.at.elapsed() > m.ttl()) {
                        c.message = None;
                    }
                }
            }
            if !st.sessions.is_empty() {
                ever_had_session = true;
            } else if ever_had_session {
                // every session was closed by the user: nothing to restore next time
                if st.cfg.persist {
                    crate::persist::clear();
                }
                std::process::exit(0);
            }
            if !st.dirty && !tick {
                continue;
            }
            st.dirty = false;
            // clear attention for panes being looked at
            let viewed: Vec<usize> = st
                .clients
                .values()
                .filter_map(|c| st.sessions.iter().find(|s| s.name == c.session))
                .map(|s| s.windows[s.active].active)
                .collect();
            for id in viewed {
                if let Some(p) = st.panes.get_mut(&id) {
                    p.attention = false;
                }
            }
            let mut clients = std::mem::take(&mut st.clients);
            let mut gone = vec![];
            for (id, c) in clients.iter_mut() {
                if let Some(out) = render_client(&st, c)
                    && !out.is_empty()
                    && write_msg(&mut c.sender, &ServerMsg::Output(out)).is_err()
                {
                    gone.push(*id);
                }
            }
            for id in gone {
                clients.remove(&id);
            }
            st.clients = clients;
        }
    });

    for (cid, conn) in listener.incoming().flatten().enumerate() {
        let sh = shared.clone();
        let id = cid + 1;
        std::thread::spawn(move || handle_conn(sh, id, conn));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TitleMode, contain_vt, wants_title};

    #[test]
    fn epoch_attachment_rejects_reused_ids_and_preserves_same_daemon_ids() {
        let mut st = super::test_state();
        let epoch = st.events.epoch().to_owned();
        st.sessions.push(super::Session {
            id: 1,
            name: "original".into(),
            windows: vec![],
            active: 0,
            cols: 80,
            rows: 24,
            sidebar: false,
            cwd: None,
            env: Default::default(),
            last_used: std::time::Instant::now(),
        });
        assert_eq!(
            super::workspace_attach_name(&st, 1, Some(&epoch)).unwrap(),
            "original"
        );
        st.sessions[0].name = "renamed".into();
        assert_eq!(
            super::workspace_attach_name(&st, 1, Some(&epoch)).unwrap(),
            "renamed"
        );
        assert!(super::workspace_attach_name(&st, 2, Some(&epoch)).is_err());

        // A new daemon can have the same ID and name; neither makes an old
        // cached selection valid in the new lifetime.
        st.events = crate::events::Journal::default();
        st.sessions[0].name = "original".into();
        assert_ne!(st.events.epoch(), epoch);
        let error = super::workspace_attach_name(&st, 1, Some(&epoch)).unwrap_err();
        assert!(error.to_string().contains("server changed since selection"));
        assert!(st.clients.is_empty());
        assert_eq!(
            super::workspace_attach_name(&st, 1, Some(st.events.epoch())).unwrap(),
            "original"
        );
        // Explicit legacy ID attachment retains its existing contract.
        assert_eq!(
            super::workspace_attach_name(&st, 1, None).unwrap(),
            "original"
        );
    }

    #[test]
    fn pane_area_leaves_row_zero_to_the_toolbar() {
        use crate::layout::Rect;
        let mut st = super::test_state();
        let s = super::test_session("t", 80, 24, 7);
        // test_state keeps the legacy geometry
        assert!(!st.toolbar_shown(&s));
        assert_eq!(
            st.pane_area(&s),
            Rect {
                x: 0,
                y: 0,
                w: 80,
                h: 23
            }
        );
        st.cfg.toolbar = true;
        assert!(st.toolbar_shown(&s));
        assert_eq!(
            st.pane_area(&s),
            Rect {
                x: 0,
                y: 1,
                w: 80,
                h: 22
            }
        );
        let rects = st.visible_rects(&s, &s.windows[0]);
        assert_eq!(
            rects,
            vec![(
                7,
                Rect {
                    x: 0,
                    y: 1,
                    w: 80,
                    h: 22
                }
            )]
        );
        // the sidebar keeps its columns; the pane area (and toolbar) narrows
        let mut side = super::test_session("t", 80, 24, 7);
        side.sidebar = true;
        assert_eq!(
            st.pane_area(&side),
            Rect {
                x: 0,
                y: 1,
                w: 52,
                h: 22
            }
        );
    }

    #[test]
    fn toolbar_needs_four_rows() {
        use crate::layout::Rect;
        let mut st = super::test_state();
        st.cfg.toolbar = true;
        let s = super::test_session("t", 40, 4, 1);
        assert_eq!(
            st.pane_area(&s),
            Rect {
                x: 0,
                y: 1,
                w: 40,
                h: 2
            }
        );
        let s = super::test_session("t", 40, 3, 1);
        assert!(!st.toolbar_shown(&s));
        assert_eq!(
            st.pane_area(&s),
            Rect {
                x: 0,
                y: 0,
                w: 40,
                h: 2
            }
        );
    }

    #[test]
    fn split_layout_starts_below_the_toolbar() {
        use crate::layout::{Dir, Node, Rect};
        let mut st = super::test_state();
        st.cfg.toolbar = true;
        let mut s = super::test_session("t", 80, 24, 1);
        s.windows[0].layout = Node::Split {
            dir: Dir::V,
            ratio: 0.5,
            a: Box::new(Node::Leaf(1)),
            b: Box::new(Node::Leaf(2)),
        };
        let rects = st.visible_rects(&s, &s.windows[0]);
        assert_eq!(
            rects[0],
            (
                1,
                Rect {
                    x: 0,
                    y: 1,
                    w: 80,
                    h: 11
                }
            )
        );
        assert_eq!(
            rects[1],
            (
                2,
                Rect {
                    x: 0,
                    y: 13,
                    w: 80,
                    h: 10
                }
            )
        );
        assert_eq!(rects[1].1.y + rects[1].1.h, 23, "the status row stays free");
    }

    #[test]
    fn title_policy_auto_needs_split_height_and_info() {
        // auto: split, unzoomed, >= 6 rows, and an agent report or label
        assert!(wants_title(TitleMode::Auto, true, false, 6, true));
        assert!(
            !wants_title(TitleMode::Auto, true, false, 5, true),
            "too short"
        );
        assert!(
            !wants_title(TitleMode::Auto, true, false, 20, false),
            "plain pane"
        );
        assert!(
            !wants_title(TitleMode::Auto, false, false, 20, true),
            "single pane"
        );
        assert!(
            !wants_title(TitleMode::Auto, true, true, 20, true),
            "zoomed"
        );
    }

    #[test]
    fn title_policy_always_and_off() {
        assert!(wants_title(TitleMode::Always, true, false, 2, false));
        assert!(
            !wants_title(TitleMode::Always, true, false, 1, true),
            "no room for content"
        );
        assert!(!wants_title(TitleMode::Always, false, false, 20, true));
        assert!(!wants_title(TitleMode::Off, true, false, 20, true));
    }

    #[test]
    fn vt100_wide_char_resize_panic_is_contained() {
        let mut p = vt100::Parser::new(5, 4, 0);
        p.process("中中中中".as_bytes());
        p.screen_mut().set_size(5, 3);
        let r = contain_vt(&mut p, 5, 3, 0, |vt| vt.process("x中y".as_bytes()));
        assert!(
            r.is_none(),
            "vt100 no longer panics here; the guard can be revisited"
        );
        assert_eq!(p.screen().size(), (5, 3));
        p.process(b"ok");
        assert!(p.screen().contents().contains("ok"));
        assert_eq!(contain_vt(&mut p, 5, 3, 0, |_| 7), Some(7));
    }
}
