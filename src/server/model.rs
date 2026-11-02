//! Runtime objects owned by the multiplexer server.
use crate::config::Config;
use crate::copy::CopyState;
use crate::layout::{Node, Rect};
use crate::render::Frame;
use portable_pty::{Child, MasterPty};
use std::collections::BTreeMap;
use std::io::Write;
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
    /// Validated plugin directories actually prepended when this pane was started.
    pub plugin_path_dirs: Vec<String>,
    /// Problems encountered while resolving this pane's plugin directories.
    pub plugin_path_diagnostics: Vec<String>,
    /// Set when the pane's console host ended while processes under it still run.
    pub host_lost: Option<crate::server::host::HostLost>,
    /// Set when the pane's root process exited while its console host still serves processes
    /// under it: their last seen (pid, executable). The pane stays live, without a banner.
    pub root_exited: Option<Vec<(u32, String)>>,
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
