//! Mouse input: click to focus and select, drag to copy, wheel to scroll.
//!
//! Click targets match what is drawn: sidebar lines come from `sidebar_rows`, window labels
//! from `status_hit_at`, the session chip and counters from `status_regions` (same layout), and
//! toolbar buttons from `toolbar_hit_at`.
//! Overlays (chooser, key popup) take the wheel; any click closes them, as before.
use crate::copy::{self, CopyState};
use crate::layout::Rect;
use crate::render::{
    SidebarKind, pack_chooser, sidebar_rows_lang, sidebar_text_width, status_hit_at,
    status_regions, toolbar_hit_at, unpack_chooser, visible_entries,
};
use crate::server::State;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

/// Lines one wheel tick moves a key popup or a pane.
const WHEEL_LINES: usize = 3;

impl State {
    /// Handle one mouse event from client `cid` on session index `si`.
    /// Returns a command line for the caller to run with `run_line` (session chip, counters
    /// and toolbar buttons); every other click or wheel step is applied here.
    pub(crate) fn handle_mouse(&mut self, cid: usize, si: usize, m: MouseEvent) -> Option<String> {
        if matches!(m.kind, MouseEventKind::Down(_)) {
            self.cancel_prefix(cid);
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click_left(cid, si, m),
            MouseEventKind::Drag(MouseButton::Left) => {
                self.drag_copy(cid, si, m);
                None
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.finish_drag(cid);
                None
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => self.wheel(cid, si, m),
            _ => None,
        }
    }

    /// Any click cancels a pending prefix key.
    fn cancel_prefix(&mut self, cid: usize) {
        let was = self
            .clients
            .get_mut(&cid)
            .is_some_and(|c| std::mem::replace(&mut c.prefix, false));
        if was {
            self.mark_dirty();
        }
    }

    fn click_left(&mut self, cid: usize, si: usize, m: MouseEvent) -> Option<String> {
        if let Some(c) = self.clients.get_mut(&cid) {
            c.drag = None;
            if c.chooser.is_some() || c.popup.is_some() {
                c.chooser = None;
                c.popup = None;
                self.mark_dirty();
                return None;
            }
        }
        // The status row spans the sidebar columns too, so it is checked before the sidebar.
        if self.sessions[si].rows.checked_sub(1) == Some(m.row) {
            return self.click_status(cid, si, m.column);
        }
        // the toolbar spans only the pane columns, so a sidebar click on row 0 falls through
        if let Some(line) = self.click_toolbar(cid, si, m.column, m.row) {
            return Some(line);
        }
        if self.click_sidebar(cid, si, m.column, m.row) {
            return None;
        }
        if let Some((pid, r)) = self.pane_at(si, m.column, m.row) {
            self.start_pane_drag(cid, si, pid, r, m);
            return None;
        }
        if let Some(pid) = self.title_at(si, m.column, m.row) {
            self.focus_pane(pid);
        }
        None
    }

    fn pane_at(&self, si: usize, col: u16, row: u16) -> Option<(usize, Rect)> {
        let s = &self.sessions[si];
        let w = &s.windows[s.active];
        self.visible_rects(s, w)
            .into_iter()
            .find(|(_, r)| r.contains(col, row))
    }

    /// Sidebar column: focus a pane or detail line, or switch to a collapsed session.
    /// Returns `true` when the click landed in the sidebar, even on a blank or footer line.
    fn click_sidebar(&mut self, cid: usize, si: usize, col: u16, row: u16) -> bool {
        let s = &self.sessions[si];
        let sw = self.sidebar_width(s);
        if sw == 0 || col <= s.cols - sw {
            return false;
        }
        let tw = sidebar_text_width(self, s);
        let target = sidebar_rows_lang(self, s, tw, s.rows, self.client_lang(cid))
            .into_iter()
            .find(|r| r.y == row);
        match target {
            Some(r) if matches!(r.kind, SidebarKind::Pane | SidebarKind::Detail) => {
                if let Some(pid) = r.pane {
                    self.focus_across_sessions(cid, pid);
                }
            }
            Some(r) if r.kind == SidebarKind::SessionClosed => {
                if let Some(nsi) = r.session {
                    self.switch_session(cid, nsi);
                    self.mark_dirty();
                }
            }
            _ => {}
        }
        true
    }

    /// Switch the client to the pane's session when needed, then focus the pane.
    fn focus_across_sessions(&mut self, cid: usize, pid: usize) {
        let Some((psi, _)) = self.locate(pid) else {
            return;
        };
        if self.client_si(cid) != Some(psi) {
            self.switch_session(cid, psi);
        }
        self.focus_pane(pid);
        self.mark_dirty();
    }

    /// Status row: the session chip opens the chooser, window labels switch windows, and the
    /// counters run `next-attention`.
    fn click_status(&mut self, cid: usize, si: usize, col: u16) -> Option<String> {
        let (regions, hit) = self.clients.get(&cid).map(|c| {
            let s = &self.sessions[si];
            (status_regions(self, c, s), status_hit_at(self, c, s, col))
        })?;
        let inside = |span: Option<(u16, u16)>| span.is_some_and(|(a, b)| col >= a && col < b);
        if inside(regions.session) {
            return Some("choose-tree".into());
        }
        if inside(regions.counts) {
            return Some("next-attention".into());
        }
        if let Some(wi) = hit {
            self.sessions[si].active = wi;
            self.relayout(si);
        }
        None
    }

    /// Toolbar row: the command line of the button under the pointer (kill commands come back
    /// wrapped in `confirm-before`, so they ask like the key bindings do). Nothing while a prompt
    /// is open, so a click cannot run a command under a half-typed rename or command line.
    fn click_toolbar(&self, cid: usize, si: usize, col: u16, row: u16) -> Option<String> {
        let c = self.clients.get(&cid).filter(|c| c.prompt.is_none())?;
        toolbar_hit_at(self, c, &self.sessions[si], col, row)
    }

    /// Pane whose title row (reserved above its rect, see `visible_rects`) is at `row`.
    fn title_at(&self, si: usize, col: u16, row: u16) -> Option<usize> {
        let s = &self.sessions[si];
        self.title_rows(s, &s.windows[s.active])
            .into_iter()
            .find(|(_, r)| r.y == row && col >= r.x && col < r.x + r.w)
            .map(|(pid, _)| pid)
    }

    fn start_pane_drag(&mut self, cid: usize, si: usize, pid: usize, r: Rect, m: MouseEvent) {
        let s = &mut self.sessions[si];
        s.windows[s.active].active = pid;
        if let Some(p) = self.panes.get_mut(&pid) {
            if let Some(cs) = &mut p.copy {
                cs.anchor = None;
            }
            let off = p.parser.screen().scrollback();
            let row = copy::abs_row(m.row - r.y, off);
            if let Some(c) = self.clients.get_mut(&cid) {
                c.drag = Some((pid, row, m.column - r.x, false));
            }
        }
        self.mark_dirty();
    }

    fn drag_copy(&mut self, cid: usize, si: usize, m: MouseEvent) {
        let Some((pid, row, col, _)) = self.clients.get(&cid).and_then(|c| c.drag) else {
            return;
        };
        let s = &self.sessions[si];
        let w = &s.windows[s.active];
        let Some(r) = self
            .visible_rects(s, w)
            .iter()
            .find(|(i, _)| *i == pid)
            .map(|x| x.1)
        else {
            return;
        };
        // a zero-sized pane has no cell to clamp to (and clamp panics when min > max)
        if r.w == 0 || r.h == 0 {
            return;
        }
        let Some(p) = self.panes.get_mut(&pid) else {
            return;
        };
        let mut cy = m.row.clamp(r.y, r.y + r.h - 1) - r.y;
        let cx = m.column.clamp(r.x, r.x + r.w - 1) - r.x;
        // auto-scroll when dragging past the edges
        let off = p.parser.screen().scrollback();
        if m.row < r.y {
            p.parser.screen_mut().set_scrollback(off + 1);
            cy = 0;
        } else if m.row >= r.y + r.h && off > 0 {
            p.parser.screen_mut().set_scrollback(off - 1);
        }
        let cs = p.copy.get_or_insert_with(CopyState::default);
        cs.anchor = Some((row, col));
        cs.line = false;
        cs.cx = cx;
        cs.cy = cy;
        if let Some(c) = self.clients.get_mut(&cid) {
            c.drag = Some((pid, row, col, true));
        }
        self.mark_dirty();
    }

    fn finish_drag(&mut self, cid: usize) {
        let Some((pid, _, _, moved)) = self.clients.get_mut(&cid).and_then(|c| c.drag.take())
        else {
            return;
        };
        if moved {
            self.copy_selection(cid, pid);
        }
    }

    /// Wheel: overlays first (chooser moves its selection, popup scrolls), then the pane
    /// under the pointer (app cursor keys on the alternate screen, else scrollback).
    fn wheel(&mut self, cid: usize, si: usize, m: MouseEvent) -> Option<String> {
        let up = m.kind == MouseEventKind::ScrollUp;
        if self.wheel_overlay(cid, up) {
            return None;
        }
        if let Some((pid, r)) = self.pane_at(si, m.column, m.row) {
            self.wheel_pane(pid, r, up);
        }
        None
    }

    /// Move the chooser selection by one entry (within the rows its filter shows) or scroll
    /// the key popup. `true` if an overlay was open.
    fn wheel_overlay(&mut self, cid: usize, up: bool) -> bool {
        let Some(c) = self.clients.get(&cid) else {
            return false;
        };
        if let Some(raw) = c.chooser {
            let (_, filter) = unpack_chooser(raw);
            let last = visible_entries(self, filter).len().saturating_sub(1);
            let next = wheel_chooser(raw, last, up);
            self.set_chooser_value(cid, next);
            return true;
        }
        if c.popup.is_none() {
            return false;
        }
        if let Some((lines, off)) = self.clients.get_mut(&cid).and_then(|c| c.popup.as_mut()) {
            let max = lines.len().saturating_sub(1);
            *off = if up {
                off.saturating_sub(WHEEL_LINES)
            } else {
                (*off + WHEEL_LINES).min(max)
            };
        }
        self.mark_dirty();
        true
    }

    fn set_chooser_value(&mut self, cid: usize, raw: usize) {
        if let Some(c) = self.clients.get_mut(&cid) {
            c.chooser = Some(raw);
        }
        self.mark_dirty();
    }

    fn wheel_pane(&mut self, pid: usize, _r: Rect, up: bool) {
        let Some(p) = self.panes.get_mut(&pid) else {
            return;
        };
        if p.parser.screen().alternate_screen() {
            let seq: &[u8] = if up { b"\x1b[A" } else { b"\x1b[B" };
            for _ in 0..WHEEL_LINES {
                let _ = p.writer.write_all(seq);
            }
            let _ = p.writer.flush();
            return;
        }
        let delta = if up {
            WHEEL_LINES as isize
        } else {
            -(WHEEL_LINES as isize)
        };
        self.scroll_pane(pid, delta);
    }

    /// Scroll pane `pid` back (`delta > 0`) or forward through its scrollback. A pane scrolled
    /// off the live screen enters scroll mode (copy mode without a selection); returning to the
    /// bottom leaves it again. Returns `false`, changing nothing, when the pane is on the
    /// alternate screen (full-screen apps own their own scrolling) or does not exist.
    pub(crate) fn scroll_pane(&mut self, pid: usize, delta: isize) -> bool {
        let Some(p) = self.panes.get_mut(&pid) else {
            return false;
        };
        let had_copy = p.copy.is_some();
        let scrolled = scroll_view(&mut p.parser, &mut p.copy, p.rect.h, delta);
        if scrolled || had_copy != p.copy.is_some() {
            self.mark_dirty();
        }
        scrolled
    }
}

/// Scrollback step behind [`State::scroll_pane`], kept free of the PTY so it can be tested.
/// On the alternate screen nothing scrolls and `false` is returned; a leftover scroll mode
/// (copy state without a selection, e.g. the app switched screens while the user was scrolled
/// back) is dropped there too, so the caller's key reaches the app instead of copy mode.
pub(crate) fn scroll_view(
    parser: &mut vt100::Parser,
    copy: &mut Option<CopyState>,
    rows: u16,
    delta: isize,
) -> bool {
    let unanchored = |c: &Option<CopyState>| c.as_ref().is_some_and(|c| c.anchor.is_none());
    if parser.screen().alternate_screen() {
        if unanchored(copy) {
            *copy = None;
            parser.screen_mut().set_scrollback(0);
        }
        return false;
    }
    let cur = parser.screen().scrollback();
    parser
        .screen_mut()
        .set_scrollback(cur.saturating_add_signed(delta));
    if parser.screen().scrollback() > 0 {
        copy.get_or_insert(CopyState {
            cx: 0,
            cy: rows.saturating_sub(1),
            anchor: None,
            line: false,
        });
    } else if unanchored(copy) {
        *copy = None;
    }
    true
}

/// Selection after one wheel tick over rows `0..=last`: up stops at the first row, down at
/// the last. A stale selection past `last` is pulled back first.
fn wheel_step(sel: usize, last: usize, up: bool) -> usize {
    let sel = sel.min(last);
    if up {
        sel.saturating_sub(1)
    } else {
        (sel + 1).min(last)
    }
}

/// Chooser value (`pack_chooser` form) after one wheel tick: the filter is kept, the selection
/// steps within the `last + 1` rows that filter shows.
fn wheel_chooser(raw: usize, last: usize, up: bool) -> usize {
    let (sel, filter) = unpack_chooser(raw);
    pack_chooser(wheel_step(sel, last, up), filter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Status;
    use crate::render::Filter;

    use crossterm::event::KeyModifiers;

    /// 5-row parser holding 30 numbered lines, so 25+ lines sit in scrollback.
    fn populated() -> vt100::Parser {
        let mut p = vt100::Parser::new(5, 20, 100);
        for i in 0..30 {
            p.process(format!("line {i}\r\n").as_bytes());
        }
        p
    }

    #[test]
    fn scroll_view_enters_scroll_mode_and_leaves_at_bottom() {
        let (mut p, mut copy) = (populated(), None);
        assert!(scroll_view(&mut p, &mut copy, 5, 1));
        assert_eq!(p.screen().scrollback(), 1);
        let cs = copy.as_ref().expect("scrolled back -> scroll mode");
        assert!(cs.anchor.is_none() && cs.cy == 4);
        assert!(scroll_view(&mut p, &mut copy, 5, 5));
        assert_eq!(p.screen().scrollback(), 6);
        assert!(p.screen().contents().contains("line 20"));
        assert!(
            scroll_view(&mut p, &mut copy, 5, -100),
            "clamps at the bottom"
        );
        assert_eq!(p.screen().scrollback(), 0);
        assert!(copy.is_none(), "back at the bottom leaves scroll mode");
    }

    #[test]
    fn scroll_view_keeps_a_selection_at_the_bottom() {
        let (mut p, mut copy) = (populated(), None);
        scroll_view(&mut p, &mut copy, 5, 3);
        copy.as_mut().unwrap().anchor = Some((-2, 0));
        scroll_view(&mut p, &mut copy, 5, -3);
        assert!(copy.is_some_and(|c| c.anchor.is_some()));
    }

    #[test]
    fn alternate_screen_drops_leftover_scroll_mode_so_keys_reach_the_app() {
        let (mut p, mut copy) = (populated(), None);
        scroll_view(&mut p, &mut copy, 5, 4);
        assert!(copy.is_some());
        // the program switches to the alternate screen while the user is scrolled back
        p.process(b"\x1b[?1049h");
        assert!(!scroll_view(&mut p, &mut copy, 5, 1), "nothing to scroll");
        assert!(
            copy.is_none(),
            "handle_key then encodes the key for the app"
        );
        // with a live selection the user keeps copy mode
        let mut copy = Some(CopyState {
            cx: 0,
            cy: 0,
            anchor: Some((0, 0)),
            line: false,
        });
        assert!(!scroll_view(&mut p, &mut copy, 5, 1));
        assert!(copy.is_some());
    }

    /// 120x30 session `t` with pane 7, client 1 attached, toolbar `on`.
    fn toolbar_state(on: bool) -> State {
        let mut st = crate::server::test_state();
        st.cfg.toolbar = on;
        st.sessions
            .push(crate::server::test_session("t", 120, 30, 7));
        st.clients.insert(1, crate::server::test_client("t"));
        st
    }

    fn click(st: &mut State, column: u16, row: u16) -> Option<String> {
        let m = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        st.handle_mouse(1, 0, m)
    }

    #[test]
    fn toolbar_click_returns_the_button_command() {
        let mut st = toolbar_state(true);
        // 120 columns fit all twelve default buttons:
        // Buttons use a single gap; hit ranges include the key and label.
        assert_eq!(click(&mut st, 0, 0).as_deref(), Some("list-keys"));
        assert_eq!(click(&mut st, 6, 0).as_deref(), Some("list-keys"));
        assert_eq!(click(&mut st, 7, 0), None, "gap between buttons");
        assert_eq!(click(&mut st, 8, 0).as_deref(), Some("new-window"));
        assert_eq!(click(&mut st, 15, 0).as_deref(), Some("split-window -h"));
        assert_eq!(click(&mut st, 35, 0).as_deref(), Some("resize-pane -Z"));
        assert_eq!(click(&mut st, 86, 0).as_deref(), Some("toggle-sidebar"));
        assert_eq!(click(&mut st, 100, 0).as_deref(), Some("detach-client"));
        assert_eq!(
            click(&mut st, 0, 1),
            None,
            "row 1 is the pane, not the toolbar"
        );
    }

    #[test]
    fn toolbar_close_asks_like_the_key_binding() {
        let mut st = toolbar_state(true);
        assert_eq!(
            click(&mut st, 76, 0).as_deref(),
            Some("confirm-before kill-pane")
        );
        assert_eq!(
            st.bindings.get("x").map(String::as_str),
            Some("confirm-before kill-pane"),
            "same command line as prefix x"
        );
    }

    #[test]
    fn toolbar_spans_only_pane_columns_and_hides_when_off() {
        let mut st = toolbar_state(true);
        st.sessions[0].sidebar = true;
        // Pane area is 92 columns: F1..F10 fit, F11 onward are dropped.
        assert_eq!(click(&mut st, 68, 0).as_deref(), Some("choose-tree"));
        assert_eq!(
            click(&mut st, 76, 0).as_deref(),
            Some("confirm-before kill-pane")
        );
        assert_eq!(click(&mut st, 86, 0), None, "overflow marker");
        assert_eq!(click(&mut st, 100, 0), None, "sidebar column");
        let mut off = toolbar_state(false);
        assert_eq!(click(&mut off, 0, 0), None);
        assert_eq!(click(&mut off, 119, 0), None);
    }

    #[test]
    fn toolbar_click_closes_an_open_overlay_first() {
        let mut st = toolbar_state(true);
        if let Some(c) = st.clients.get_mut(&1) {
            c.popup = Some((vec!["x".into()], 0));
        }
        assert_eq!(click(&mut st, 0, 0), None);
        assert!(st.clients[&1].popup.is_none());
        assert_eq!(click(&mut st, 0, 0).as_deref(), Some("list-keys"));
    }

    #[test]
    fn toolbar_click_does_nothing_while_a_prompt_is_open() {
        use crossterm::event::{KeyCode, KeyEvent};
        let mut st = toolbar_state(true);
        let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::server::test_state()));
        st.handle_key(
            &shared,
            1,
            0,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        );
        st.handle_key(
            &shared,
            1,
            0,
            KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE),
        );
        assert!(
            st.clients[&1].prompt.is_some(),
            "prefix : opens the command prompt"
        );
        assert_eq!(
            click(&mut st, 9, 0),
            None,
            "F2 new is not run under the prompt"
        );
        assert!(st.clients[&1].prompt.is_some(), "the prompt stays open");
    }

    #[test]
    fn wheel_step_moves_one_row_and_clamps_at_both_ends() {
        assert_eq!(wheel_step(0, 4, true), 0, "wheel up at the top stays at 0");
        assert_eq!(wheel_step(2, 4, true), 1);
        assert_eq!(wheel_step(2, 4, false), 3);
        assert_eq!(wheel_step(4, 4, false), 4, "wheel down at the end stays");
    }

    #[test]
    fn wheel_step_on_empty_or_stale_selection_stays_in_range() {
        assert_eq!(wheel_step(0, 0, false), 0);
        assert_eq!(
            wheel_step(9, 2, true),
            1,
            "stale selection is clamped first"
        );
        assert_eq!(wheel_step(9, 2, false), 2);
    }

    #[test]
    fn wheel_chooser_keeps_the_filter_while_stepping() {
        let blocked = Filter::Only(Status::Blocked);
        let raw = pack_chooser(1, blocked);
        let down = wheel_chooser(raw, 3, false);
        assert_eq!(unpack_chooser(down), (2, blocked));
        let up = wheel_chooser(down, 3, true);
        assert_eq!(unpack_chooser(up), (1, blocked));
    }

    #[test]
    fn wheel_chooser_at_top_with_all_filter_stays_at_zero() {
        let raw = pack_chooser(0, Filter::All);
        assert_eq!(
            unpack_chooser(wheel_chooser(raw, 5, true)),
            (0, Filter::All)
        );
    }
}
