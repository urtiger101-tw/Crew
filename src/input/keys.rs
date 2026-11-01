//! Keyboard input: overlays (popup, chooser, prompt), the prefix key, and pane pass-through.
use crate::commands::{self, Cmd, key_name};
use crate::i18n::{Lang, tr, trf};
use crate::keys;
use crate::render::{
    Entry, Filter, kill_target_lang, pack_chooser, toolbar_command, unpack_chooser, visible_entries,
};
use crate::server::{Prompt, PromptKind, Shared, State};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fmt::Display;

/// Prompt history length (`State.history`, oldest first).
const HISTORY_MAX: usize = 50;
/// Lines moved by PageUp/PageDown and Space in the popup.
const PAGE: usize = 10;

impl State {
    pub(crate) fn handle_key(&mut self, shared: &Shared, cid: usize, si: usize, k: KeyEvent) {
        let Some(c) = self.clients.get_mut(&cid) else {
            return;
        };
        // overlays first
        if c.popup.is_some() {
            self.popup_key(cid, k);
            return;
        }
        if c.chooser.is_some() {
            self.chooser_key(shared, cid, k);
            return;
        }
        if c.prompt.is_some() {
            self.prompt_key(shared, cid, k);
            return;
        }
        if c.prefix {
            c.prefix = false;
            self.mark_dirty();
            if self.cfg.is_prefix(&k) {
                let _ = self.exec(shared, cid, Cmd::SendPrefix);
                return;
            }
            self.prefix_key(shared, cid, si, &k);
            return;
        }
        if self.cfg.is_prefix(&k) {
            c.prefix = true;
            self.mark_dirty();
            return;
        }
        // `toolbar_fkeys`: a plain F1..F12 runs its toolbar button instead of reaching the pane
        if self.cfg.toolbar_fkeys
            && let Some(line) = self.toolbar_fkey_line(si, &k)
        {
            self.run_line(shared, cid, &line);
            return;
        }
        let pid = self.active_pane(si);
        // `scroll-keys`: Shift+Up/Down (line) and Shift+PageUp/PageDown (page) browse the
        // pane's earlier output; on the alternate screen the key still reaches the app.
        if self.cfg.scroll_keys {
            let page = self.panes.get(&pid).map_or(1, |p| p.rect.h.max(1));
            if let Some(delta) = scroll_key_delta(&k, page)
                && self.scroll_pane(pid, delta)
            {
                return;
            }
        }
        if self.panes.get(&pid).is_some_and(|p| p.copy.is_some()) {
            self.copy_key(cid, pid, k);
            return;
        }
        let Some(p) = self.panes.get_mut(&pid) else {
            return;
        };
        let app = p.parser.screen().application_cursor();
        let b = keys::encode(&k, app);
        self.write_pane(pid, &b);
    }

    /// Command line of the toolbar button for a plain F1..F12 (kill commands wrapped in
    /// `confirm-before`); `None` for any other key, a missing button, or no toolbar drawn on
    /// session `si` (off, or too few rows).
    fn toolbar_fkey_line(&self, si: usize, k: &KeyEvent) -> Option<String> {
        if !self.sessions.get(si).is_some_and(|s| self.toolbar_shown(s)) {
            return None;
        }
        toolbar_command(&self.cfg, fkey_number(k)?.checked_sub(1)?)
    }

    /// The key after the prefix: run its binding (an F-key without one runs its toolbar
    /// button), or say it is unbound. Esc cancels silently.
    fn prefix_key(&mut self, shared: &Shared, cid: usize, si: usize, k: &KeyEvent) {
        if k.code == KeyCode::Esc {
            return;
        }
        let name = key_name(k);
        let line = name
            .as_ref()
            .and_then(|n| self.bindings.get(n).cloned())
            .or_else(|| self.toolbar_fkey_line(si, k));
        match line {
            Some(line) => self.run_line(shared, cid, &line),
            None => {
                let prefix = self.cfg.prefix_name();
                let key = name.unwrap_or_else(|| "key".into());
                let lang = self.client_lang(cid);
                self.error_message(cid, unbound_message(lang, &prefix, &key));
            }
        }
    }

    /// Popup overlay: j/k, Space, arrows and PageUp/PageDown scroll; Esc or q closes; any other
    /// key is ignored.
    pub(crate) fn popup_key(&mut self, cid: usize, k: KeyEvent) {
        let Some(c) = self.clients.get_mut(&cid) else {
            return;
        };
        if !plain_key(&k) {
            return;
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => c.popup = None,
            code => {
                if let Some((lines, off)) = &mut c.popup {
                    let max = lines.len().saturating_sub(1);
                    if let Some(next) = scrolled(*off, code, max) {
                        *off = next;
                    }
                }
            }
        }
        self.mark_dirty();
    }

    /// Chooser overlay: j/k and arrows move, Tab/Shift+Tab move with wrap-around, Enter jumps, `x` kills, b/w/d/i/a filter, Esc or q
    /// closes. Any other key is ignored and keeps the chooser open.
    pub(crate) fn chooser_key(&mut self, shared: &Shared, cid: usize, k: KeyEvent) {
        let Some(raw) = self.clients.get(&cid).and_then(|c| c.chooser) else {
            return;
        };
        if !plain_key(&k) {
            return;
        }
        let (sel, filter) = unpack_chooser(raw);
        let entries = visible_entries(self, filter);
        let last = entries.len().saturating_sub(1);
        let sel = sel.min(last);
        match k.code {
            KeyCode::Enter => {
                self.set_chooser(cid, None);
                if let Some(&e) = entries.get(sel) {
                    self.chooser_jump(cid, e);
                }
            }
            KeyCode::Char('x') => {
                self.set_chooser(cid, None);
                if let Some(&e) = entries.get(sel) {
                    self.chooser_kill(shared, cid, e);
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => self.set_chooser(cid, None),
            KeyCode::Up | KeyCode::Char('k') => {
                self.set_chooser(cid, Some(pack_chooser(sel.saturating_sub(1), filter)))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.set_chooser(cid, Some(pack_chooser((sel + 1).min(last), filter)))
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let back = k.code == KeyCode::BackTab || k.modifiers.contains(KeyModifiers::SHIFT);
                let next = tab_step(sel, entries.len(), back);
                self.set_chooser(cid, Some(pack_chooser(next, filter)))
            }
            // In the window switcher `w` steps on, like holding Alt and tapping Tab
            KeyCode::Char('w') if filter == Filter::Windows => {
                let next = tab_step(sel, entries.len(), false);
                self.set_chooser(cid, Some(pack_chooser(next, filter)))
            }
            KeyCode::Char(ch) => {
                if let Some(f) = Filter::from_key(ch) {
                    self.set_chooser(cid, Some(pack_chooser(0, f)));
                }
            }
            _ => {}
        }
    }

    fn set_chooser(&mut self, cid: usize, v: Option<usize>) {
        if let Some(c) = self.clients.get_mut(&cid) {
            c.chooser = v;
        }
        self.mark_dirty();
    }

    /// Enter on a chooser row: switch session, then select the window or focus the pane.
    fn chooser_jump(&mut self, cid: usize, (si, wi, pi): Entry) {
        if si >= self.sessions.len() {
            return;
        }
        // same session: no switch, so the client is not redrawn in full
        if self.client_si(cid) != Some(si) {
            self.switch_session(cid, si);
        }
        match (wi, pi) {
            (Some(w), None) => {
                self.sessions[si].active = w;
                self.relayout(si);
            }
            (Some(w), Some(p)) => {
                let pid = self.sessions[si]
                    .windows
                    .get(w)
                    .and_then(|win| win.panes().get(p).copied());
                if let Some(pid) = pid {
                    self.focus_pane(pid);
                }
            }
            _ => {}
        }
        self.mark_dirty();
    }

    /// `x` on a chooser row: ask first when `confirm-close` is on, otherwise run the kill now.
    fn chooser_kill(&mut self, shared: &Shared, cid: usize, e: Entry) {
        let Some(target) = kill_target_lang(self, e, self.client_lang(cid)) else {
            let lang = self.client_lang(cid);
            self.message(cid, tr(lang, "messages.pane_close_hint"));
            return;
        };
        if self.cfg.confirm_close {
            if let Some(c) = self.clients.get_mut(&cid) {
                c.prompt = Some(Prompt::confirm(target.question, target.command));
            }
            self.mark_dirty();
        } else {
            self.run_reporting(shared, cid, &target.command);
        }
    }

    /// Prompt line: text kinds edit the buffer (UX-09); Confirm waits for y. Enter runs a text
    /// prompt, Esc or Ctrl-C cancels.
    pub(crate) fn prompt_key(&mut self, shared: &Shared, cid: usize, k: KeyEvent) {
        let Some(kind) = self
            .clients
            .get(&cid)
            .and_then(|c| c.prompt.as_ref())
            .map(|p| p.kind)
        else {
            return;
        };
        if kind == PromptKind::Confirm {
            self.confirm_key(shared, cid, &k);
        } else {
            self.text_prompt_key(shared, cid, kind, &k);
        }
        self.mark_dirty();
    }

    /// `Confirm`: `y` or `Y` runs the pending command; any other key cancels. A confirm bound
    /// to a pane (`expect_pane`) refuses `y` when that pane is no longer the active one.
    fn confirm_key(&mut self, shared: &Shared, cid: usize, k: &KeyEvent) {
        let Some(p) = self.take_prompt(cid) else {
            return;
        };
        let yes = matches!(k.code, KeyCode::Char('y' | 'Y')) && plain_key(k);
        let expect = p.expect_pane;
        let lang = self.client_lang(cid);
        match (yes, p.pending) {
            (true, Some(_)) if !self.is_expected_pane(cid, expect) => {
                self.error_message(cid, tr(lang, "messages.target_changed"));
            }
            (true, Some(cmd)) => self.run_reporting(shared, cid, &cmd),
            _ => self.message(cid, tr(lang, "messages.cancelled")),
        }
    }

    /// `true` when the confirm has no pane bound, or the bound pane still exists and is the
    /// active pane of the client's session.
    fn is_expected_pane(&self, cid: usize, expect: Option<usize>) -> bool {
        let Some(pid) = expect else {
            return true;
        };
        let Some(si) = self.client_si(cid) else {
            return false;
        };
        self.panes.contains_key(&pid) && self.active_pane(si) == pid
    }

    fn text_prompt_key(&mut self, shared: &Shared, cid: usize, kind: PromptKind, k: &KeyEvent) {
        let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            _ if ctrl_c => {
                self.take_prompt(cid);
            }
            KeyCode::Esc => {
                self.take_prompt(cid);
            }
            KeyCode::Enter => {
                let Some(p) = self.take_prompt(cid) else {
                    return;
                };
                let line = match p.kind {
                    PromptKind::Command => {
                        push_history(&mut self.history, &p.buf);
                        p.buf
                    }
                    PromptKind::RenameWindow => {
                        format!("rename-window \"{}\"", p.buf.replace('"', "\\\""))
                    }
                    PromptKind::RenameSession => {
                        format!("rename-session \"{}\"", p.buf.replace('"', "\\\""))
                    }
                    PromptKind::Confirm => return,
                };
                if !line.trim().is_empty() {
                    self.run_reporting(shared, cid, &line);
                }
            }
            _ => self.edit_prompt(cid, kind, k),
        }
    }

    /// Apply an editing key to the open text prompt. Up/Down walk the `:` history; a text
    /// change ends history browsing.
    fn edit_prompt(&mut self, cid: usize, kind: PromptKind, k: &KeyEvent) {
        let Some(p) = self.clients.get_mut(&cid).and_then(|c| c.prompt.as_mut()) else {
            return;
        };
        if kind == PromptKind::Command && matches!(k.code, KeyCode::Up | KeyCode::Down) {
            let up = k.code == KeyCode::Up;
            if let Some((hist, text)) = history_step(&self.history, p.hist, up) {
                p.hist = hist;
                p.cur = text.chars().count();
                p.buf = text;
            }
            return;
        }
        if let Some((buf, cur)) = line_edit(&p.buf, p.cur, k) {
            if buf != p.buf {
                p.hist = None;
            }
            p.buf = buf;
            p.cur = cur;
        }
    }

    fn take_prompt(&mut self, cid: usize) -> Option<Prompt> {
        self.clients.get_mut(&cid)?.prompt.take()
    }

    /// Run a command line; a failure is shown as an error (run_line shows it as info).
    fn run_reporting(&mut self, shared: &Shared, cid: usize, line: &str) {
        if let Err(e) = commands::parse(line).and_then(|c| self.exec(shared, cid, c)) {
            let text = commands::localize(self.client_lang(cid), &e);
            self.error_message(cid, text);
        }
    }
}

/// Whether a key carries no Ctrl or Alt chord.
/// Scrollback lines for a `scroll-keys` key: Shift+Up/Down one line, Shift+PageUp/PageDown
/// one pane height (`page` rows). Positive scrolls back. `None` for every other key.
fn scroll_key_delta(k: &KeyEvent, page: u16) -> Option<isize> {
    if k.modifiers != KeyModifiers::SHIFT {
        return None;
    }
    let page = page as isize;
    match k.code {
        KeyCode::Up => Some(1),
        KeyCode::Down => Some(-1),
        KeyCode::PageUp => Some(page),
        KeyCode::PageDown => Some(-page),
        _ => None,
    }
}

/// Chooser row after Tab (forward) or Shift+Tab (`back`): like a dialog's focus ring it wraps
/// around at both ends. An empty list stays on row 0.
fn tab_step(sel: usize, len: usize, back: bool) -> usize {
    if len == 0 {
        return 0;
    }
    let sel = sel.min(len - 1);
    if back {
        (sel + len - 1) % len
    } else {
        (sel + 1) % len
    }
}

fn plain_key(k: &KeyEvent) -> bool {
    !k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

/// Number of a function key pressed without modifiers (`F1` -> 1); `None` otherwise.
fn fkey_number(k: &KeyEvent) -> Option<usize> {
    match k.code {
        KeyCode::F(n) if k.modifiers.is_empty() => Some(usize::from(n)),
        _ => None,
    }
}

/// Message for a key after the prefix that has no binding (UX-04).
fn unbound_message(lang: Lang, prefix: &str, key: &str) -> String {
    trf(
        lang,
        "messages.unbound",
        &[("prefix", &prefix as &dyn Display), ("key", &key)],
    )
}

/// New popup offset for a scroll key; `None` when the key does not scroll.
fn scrolled(off: usize, code: KeyCode, max: usize) -> Option<usize> {
    Some(match code {
        KeyCode::Down | KeyCode::Char('j') => (off + 1).min(max),
        KeyCode::Up | KeyCode::Char('k') => off.saturating_sub(1),
        KeyCode::PageDown | KeyCode::Char(' ') => (off + PAGE).min(max),
        KeyCode::PageUp => off.saturating_sub(PAGE),
        _ => return None,
    })
}

/// Editing of a prompt line (UX-09). `buf` with the cursor at char index `cur`; returns the new
/// buffer and cursor, or `None` when the key is not an editing key. Ctrl-k/Ctrl-u/Ctrl-w
/// follow readline (kill to end, to start, previous word).
pub(crate) fn line_edit(buf: &str, cur: usize, k: &KeyEvent) -> Option<(String, usize)> {
    if k.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    let chars: Vec<char> = buf.chars().collect();
    let len = chars.len();
    let cur = cur.min(len);
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let (next, at): (Vec<char>, usize) = match (k.code, ctrl) {
        (KeyCode::Left, false) | (KeyCode::Char('b'), true) => (chars, cur.saturating_sub(1)),
        (KeyCode::Right, false) | (KeyCode::Char('f'), true) => (chars, (cur + 1).min(len)),
        (KeyCode::Home, false) | (KeyCode::Char('a'), true) => (chars, 0),
        (KeyCode::End, false) | (KeyCode::Char('e'), true) => (chars, len),
        (KeyCode::Backspace, false) | (KeyCode::Char('h'), true) => {
            if cur == 0 {
                (chars, 0)
            } else {
                let mut v = chars;
                v.remove(cur - 1);
                (v, cur - 1)
            }
        }
        (KeyCode::Delete, false) => {
            let mut v = chars;
            if cur < len {
                v.remove(cur);
            }
            (v, cur)
        }
        (KeyCode::Char('k'), true) => (chars[..cur].to_vec(), cur),
        (KeyCode::Char('u'), true) => (chars[cur..].to_vec(), 0),
        (KeyCode::Char('w'), true) => {
            let start = word_start(&chars, cur);
            let mut v = chars[..start].to_vec();
            v.extend_from_slice(&chars[cur..]);
            (v, start)
        }
        (KeyCode::Char(c), false) => {
            let mut v = chars[..cur].to_vec();
            v.push(c);
            v.extend_from_slice(&chars[cur..]);
            (v, cur + 1)
        }
        _ => return None,
    };
    Some((next.into_iter().collect(), at))
}

/// Start of the word before `cur` (readline unix-word-rubout): skip spaces, then non-spaces.
fn word_start(chars: &[char], cur: usize) -> usize {
    let mut i = cur;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// Up/Down in the `:` prompt over `history` (oldest first). `hist` is the browsing position
/// (`None` = a fresh line), so duplicate entries are walked one by one. Returns the new
/// position and the text it shows, or `None` when nothing moves. Up from a fresh line goes to
/// the newest entry; Down past the newest returns to an empty fresh line.
fn history_step(
    history: &[String],
    hist: Option<usize>,
    up: bool,
) -> Option<(Option<usize>, String)> {
    let newest = history.len().checked_sub(1)?;
    match (hist, up) {
        (None, true) => Some((Some(newest), history[newest].clone())),
        (None, false) => None,
        (Some(i), true) => {
            let prev = i.min(newest).checked_sub(1)?;
            Some((Some(prev), history[prev].clone()))
        }
        (Some(i), false) if i < newest => Some((Some(i + 1), history[i + 1].clone())),
        (Some(_), false) => Some((None, String::new())),
    }
}

/// Record a submitted `:` line: not blank, not a repeat of the newest entry, at most
/// `HISTORY_MAX` entries.
fn push_history(history: &mut Vec<String>, line: &str) {
    if line.trim().is_empty() || history.last().is_some_and(|h| h.as_str() == line) {
        return;
    }
    history.push(line.to_string());
    if history.len() > HISTORY_MAX {
        let excess = history.len() - HISTORY_MAX;
        history.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn plain(code: KeyCode) -> KeyEvent {
        key(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn edit(buf: &str, cur: usize, k: KeyEvent) -> (String, usize) {
        line_edit(buf, cur, &k).expect("editing key")
    }

    #[test]
    fn typing_inserts_at_cursor() {
        assert_eq!(edit("ad", 1, plain(KeyCode::Char('b'))), ("abd".into(), 2));
        assert_eq!(edit("", 0, plain(KeyCode::Char('x'))), ("x".into(), 1));
    }

    #[test]
    fn typing_handles_wide_chars_by_char_index() {
        assert_eq!(
            edit("中文", 1, plain(KeyCode::Char('x'))),
            ("中x文".into(), 2)
        );
    }

    #[test]
    fn arrows_and_home_end_clamp() {
        assert_eq!(edit("abc", 0, plain(KeyCode::Left)), ("abc".into(), 0));
        assert_eq!(edit("abc", 3, plain(KeyCode::Right)), ("abc".into(), 3));
        assert_eq!(edit("abc", 1, plain(KeyCode::Right)), ("abc".into(), 2));
        assert_eq!(edit("abc", 2, plain(KeyCode::Home)), ("abc".into(), 0));
        assert_eq!(edit("abc", 0, plain(KeyCode::End)), ("abc".into(), 3));
        assert_eq!(edit("abc", 1, ctrl('a')), ("abc".into(), 0));
        assert_eq!(edit("abc", 1, ctrl('e')), ("abc".into(), 3));
        assert_eq!(edit("abc", 2, ctrl('b')), ("abc".into(), 1));
        assert_eq!(edit("abc", 2, ctrl('f')), ("abc".into(), 3));
    }

    #[test]
    fn backspace_removes_before_cursor_and_noops_at_start() {
        assert_eq!(edit("abc", 2, plain(KeyCode::Backspace)), ("ac".into(), 1));
        assert_eq!(edit("abc", 0, plain(KeyCode::Backspace)), ("abc".into(), 0));
        assert_eq!(edit("abc", 1, ctrl('h')), ("bc".into(), 0));
    }

    #[test]
    fn delete_removes_at_cursor_and_noops_at_end() {
        assert_eq!(edit("abc", 1, plain(KeyCode::Delete)), ("ac".into(), 1));
        assert_eq!(edit("abc", 3, plain(KeyCode::Delete)), ("abc".into(), 3));
    }

    #[test]
    fn ctrl_k_kills_to_end_and_ctrl_u_kills_to_start() {
        assert_eq!(edit("hello world", 6, ctrl('k')), ("hello ".into(), 6));
        assert_eq!(edit("hello world", 6, ctrl('u')), ("world".into(), 0));
    }

    #[test]
    fn ctrl_w_deletes_previous_word_readline_style() {
        assert_eq!(
            edit("one two  three", 14, ctrl('w')),
            ("one two  ".into(), 9)
        );
        assert_eq!(
            edit("one two  three", 3, ctrl('w')),
            (" two  three".into(), 0)
        );
        assert_eq!(edit("abc", 0, ctrl('w')), ("abc".into(), 0));
    }

    #[test]
    fn non_editing_keys_return_none() {
        assert!(line_edit("abc", 1, &plain(KeyCode::Enter)).is_none());
        assert!(line_edit("abc", 1, &plain(KeyCode::Up)).is_none());
        assert!(line_edit("abc", 1, &ctrl('z')).is_none());
        assert!(line_edit("abc", 1, &key(KeyCode::Char('x'), KeyModifiers::ALT)).is_none());
    }

    fn hist_of(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn history_up_walks_newest_to_oldest_and_stops_at_oldest() {
        let h = hist_of(&["a", "b", "c"]);
        assert_eq!(history_step(&h, None, true), Some((Some(2), "c".into())));
        assert_eq!(history_step(&h, Some(2), true), Some((Some(1), "b".into())));
        assert_eq!(history_step(&h, Some(1), true), Some((Some(0), "a".into())));
        assert_eq!(history_step(&h, Some(0), true), None, "oldest stays");
    }

    #[test]
    fn history_down_walks_back_and_past_newest_clears_the_line() {
        let h = hist_of(&["a", "b", "c"]);
        assert_eq!(
            history_step(&h, Some(0), false),
            Some((Some(1), "b".into()))
        );
        assert_eq!(
            history_step(&h, Some(2), false),
            Some((None, String::new())),
            "past newest is a fresh empty line"
        );
        assert_eq!(
            history_step(&h, None, false),
            None,
            "fresh line: Down does nothing"
        );
    }

    #[test]
    fn history_with_duplicates_walks_each_position() {
        // [a, b, a]: the text "a" appears twice; the position, not the text, drives the walk
        let h = hist_of(&["a", "b", "a"]);
        assert_eq!(history_step(&h, None, true), Some((Some(2), "a".into())));
        assert_eq!(history_step(&h, Some(2), true), Some((Some(1), "b".into())));
        assert_eq!(history_step(&h, Some(1), true), Some((Some(0), "a".into())));
        assert_eq!(
            history_step(&h, Some(0), false),
            Some((Some(1), "b".into()))
        );
        assert_eq!(
            history_step(&h, Some(1), false),
            Some((Some(2), "a".into()))
        );
        assert_eq!(
            history_step(&h, Some(2), false),
            Some((None, String::new()))
        );
    }

    #[test]
    fn history_on_empty_list_never_moves() {
        assert_eq!(history_step(&[], None, true), None);
        assert_eq!(history_step(&[], None, false), None);
        assert_eq!(history_step(&[], Some(0), true), None);
    }

    #[test]
    fn push_history_skips_blank_and_repeat_of_last() {
        let mut h = vec![];
        push_history(&mut h, "   ");
        push_history(&mut h, "");
        assert!(h.is_empty());
        push_history(&mut h, "split-window -h");
        push_history(&mut h, "split-window -h");
        assert_eq!(h, vec!["split-window -h".to_string()]);
        push_history(&mut h, "kill-pane");
        push_history(&mut h, "split-window -h");
        assert_eq!(h.len(), 3, "non-consecutive repeats are kept");
    }

    #[test]
    fn push_history_is_capped() {
        let mut h = vec![];
        for i in 0..55 {
            push_history(&mut h, &format!("cmd {i}"));
        }
        assert_eq!(h.len(), HISTORY_MAX);
        assert_eq!(h.first().map(String::as_str), Some("cmd 5"));
        assert_eq!(h.last().map(String::as_str), Some("cmd 54"));
    }

    #[test]
    fn unbound_notice_uses_prefix_and_key_names() {
        assert_eq!(
            unbound_message(Lang::En, "C-b", "Q"),
            "C-b Q is not bound (C-b ? lists keys)"
        );
        let zh = unbound_message(Lang::ZhTw, "C-b", "Q");
        assert_eq!(zh, "C-b Q 未綁定（C-b ? 列出按鍵）");
    }

    #[test]
    fn popup_scroll_keys_clamp_and_others_do_not_scroll() {
        assert_eq!(scrolled(0, KeyCode::Down, 3), Some(1));
        assert_eq!(scrolled(3, KeyCode::Down, 3), Some(3));
        assert_eq!(scrolled(0, KeyCode::Up, 3), Some(0));
        assert_eq!(scrolled(0, KeyCode::Char(' '), 25), Some(PAGE));
        assert_eq!(scrolled(5, KeyCode::PageUp, 25), Some(0));
        assert_eq!(scrolled(0, KeyCode::Char('z'), 3), None);
        assert_eq!(scrolled(0, KeyCode::Esc, 3), None);
    }

    /// Session `t` with pane 7 and client 1; returns the state and a spare `Shared`.
    fn fkey_state(toolbar: bool, fkeys: bool) -> (State, Shared) {
        let mut st = crate::server::test_state();
        st.cfg.toolbar = toolbar;
        st.cfg.toolbar_fkeys = fkeys;
        st.sessions
            .push(crate::server::test_session("t", 120, 30, 7));
        st.clients.insert(1, crate::server::test_client("t"));
        let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::server::test_state()));
        (st, shared)
    }

    fn press(st: &mut State, shared: &Shared, k: KeyEvent) {
        st.handle_key(shared, 1, 0, k);
    }

    #[test]
    fn scroll_keys_need_exactly_shift() {
        let shift = |c| key(c, KeyModifiers::SHIFT);
        assert_eq!(scroll_key_delta(&shift(KeyCode::Up), 20), Some(1));
        assert_eq!(scroll_key_delta(&shift(KeyCode::Down), 20), Some(-1));
        assert_eq!(scroll_key_delta(&shift(KeyCode::PageUp), 20), Some(20));
        assert_eq!(scroll_key_delta(&shift(KeyCode::PageDown), 20), Some(-20));
        // plain arrows stay with the shell (history), other modifiers with the app
        assert_eq!(scroll_key_delta(&plain(KeyCode::Up), 20), None);
        assert_eq!(scroll_key_delta(&plain(KeyCode::PageUp), 20), None);
        let ctrl_shift = KeyModifiers::SHIFT | KeyModifiers::CONTROL;
        assert_eq!(scroll_key_delta(&key(KeyCode::Up, ctrl_shift), 20), None);
        assert_eq!(scroll_key_delta(&shift(KeyCode::Left), 20), None);
    }

    #[test]
    fn scroll_keys_without_a_pane_fall_through_harmlessly() {
        let (mut st, shared) = fkey_state(false, false);
        assert!(st.cfg.scroll_keys, "on by default");
        press(&mut st, &shared, key(KeyCode::Up, KeyModifiers::SHIFT));
        assert!(st.clients[&1].popup.is_none() && st.clients[&1].message.is_none());
    }

    #[test]
    fn fkey_number_only_for_plain_function_keys() {
        assert_eq!(fkey_number(&plain(KeyCode::F(1))), Some(1));
        assert_eq!(fkey_number(&plain(KeyCode::F(12))), Some(12));
        assert_eq!(fkey_number(&key(KeyCode::F(1), KeyModifiers::SHIFT)), None);
        assert_eq!(fkey_number(&plain(KeyCode::Char('1'))), None);
    }

    #[test]
    fn direct_fkeys_run_toolbar_buttons_when_enabled() {
        let (mut st, shared) = fkey_state(true, true);
        press(&mut st, &shared, plain(KeyCode::F(1)));
        assert!(st.clients[&1].popup.is_some(), "F1 runs list-keys");
        // F13 has no button and reaches the pane (there is none here): nothing happens
        let (mut st, shared) = fkey_state(true, true);
        press(&mut st, &shared, plain(KeyCode::F(13)));
        assert!(st.clients[&1].popup.is_none() && st.clients[&1].message.is_none());
    }

    #[test]
    fn direct_fkeys_pass_through_when_off() {
        for (toolbar, fkeys) in [(true, false), (false, true)] {
            let (mut st, shared) = fkey_state(toolbar, fkeys);
            press(&mut st, &shared, plain(KeyCode::F(1)));
            let c = &st.clients[&1];
            assert!(c.popup.is_none(), "toolbar={toolbar} fkeys={fkeys}");
        }
    }

    #[test]
    fn prefix_then_fkey_runs_the_toolbar_button() {
        let (mut st, shared) = fkey_state(true, false);
        press(&mut st, &shared, ctrl('b'));
        assert!(st.clients[&1].prefix);
        press(&mut st, &shared, plain(KeyCode::F(1)));
        assert!(st.clients[&1].popup.is_some());
        // with the toolbar off the key is unbound, as before
        let (mut st, shared) = fkey_state(false, false);
        press(&mut st, &shared, ctrl('b'));
        press(&mut st, &shared, plain(KeyCode::F(1)));
        let c = &st.clients[&1];
        assert!(c.popup.is_none());
        assert!(c.message.as_ref().is_some_and(|m| m.error));
    }

    #[test]
    fn fkey_close_asks_before_killing() {
        let (mut st, shared) = fkey_state(true, true);
        press(&mut st, &shared, plain(KeyCode::F(10)));
        let p = st.clients[&1].prompt.as_ref().expect("confirm prompt");
        assert_eq!(p.kind, PromptKind::Confirm);
        assert_eq!(p.expect_pane, Some(7));
        // `n` cancels: the pane is still the window's pane
        press(&mut st, &shared, plain(KeyCode::Char('n')));
        assert!(st.clients[&1].prompt.is_none());
        assert_eq!(st.sessions[0].windows[0].panes(), vec![7]);
    }

    #[test]
    fn user_binding_on_an_fkey_wins_over_the_toolbar() {
        let (mut st, shared) = fkey_state(true, false);
        st.bindings.insert("F1".into(), "display-message hi".into());
        press(&mut st, &shared, ctrl('b'));
        press(&mut st, &shared, plain(KeyCode::F(1)));
        assert!(st.clients[&1].popup.is_none());
    }

    #[test]
    fn fkeys_do_nothing_while_the_toolbar_is_hidden_for_lack_of_rows() {
        let (mut st, shared) = fkey_state(true, true);
        st.sessions[0].rows = 3;
        press(&mut st, &shared, plain(KeyCode::F(1)));
        assert!(st.clients[&1].popup.is_none(), "plain F1 reaches the pane");
        press(&mut st, &shared, ctrl('b'));
        press(&mut st, &shared, plain(KeyCode::F(1)));
        let c = &st.clients[&1];
        assert!(c.popup.is_none());
        assert!(
            c.message.as_ref().is_some_and(|m| m.error),
            "prefix F1 is unbound"
        );
    }

    #[test]
    fn tab_step_wraps_like_a_focus_ring() {
        assert_eq!(tab_step(0, 3, false), 1);
        assert_eq!(tab_step(2, 3, false), 0);
        assert_eq!(tab_step(0, 3, true), 2);
        assert_eq!(tab_step(1, 3, true), 0);
        assert_eq!(tab_step(9, 3, false), 0, "a stale row is clamped first");
        assert_eq!(tab_step(0, 0, false), 0);
        assert_eq!(tab_step(0, 0, true), 0);
    }

    #[test]
    fn tab_and_shift_tab_move_the_chooser_selection() {
        let (mut st, shared) = fkey_state(false, false);
        let rows = visible_entries(&st, Filter::All).len();
        assert!(rows >= 2, "fixture has a session row and a window row");
        st.clients.get_mut(&1).unwrap().chooser = Some(pack_chooser(0, Filter::All));
        let sel = |st: &State| unpack_chooser(st.clients[&1].chooser.unwrap()).0;
        press(&mut st, &shared, plain(KeyCode::Tab));
        assert_eq!(sel(&st), 1);
        press(&mut st, &shared, key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(sel(&st), 0);
        press(&mut st, &shared, key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(
            sel(&st),
            rows - 1,
            "Shift+Tab on the first row wraps to the last"
        );
        press(&mut st, &shared, plain(KeyCode::Tab));
        assert_eq!(sel(&st), 0, "Tab on the last row wraps to the first");
        assert!(
            st.clients[&1].chooser.is_some(),
            "Tab never closes the chooser"
        );
    }

    #[test]
    fn prefix_w_toggles_between_the_last_two_windows() {
        let (mut st, shared) = fkey_state(false, false);
        for i in 1..3 {
            st.sessions[0].windows.push(crate::server::Window {
                id: 50 + i,
                name: format!("w{i}"),
                manual_name: true,
                layout: crate::layout::Node::Leaf(20 + i),
                active: 20 + i,
                zoomed: false,
            });
        }
        st.track_window_mru();
        st.sessions[0].active = 2;
        st.track_window_mru();
        // C-b w opens the window switcher on window 0, the one used before window 2
        press(&mut st, &shared, ctrl('b'));
        press(&mut st, &shared, plain(KeyCode::Char('w')));
        let (sel, filter) = unpack_chooser(st.clients[&1].chooser.expect("switcher open"));
        assert_eq!(filter, Filter::Windows);
        assert_eq!(visible_entries(&st, Filter::Windows)[sel].1, Some(0));
        press(&mut st, &shared, plain(KeyCode::Enter));
        assert_eq!(st.sessions[0].active, 0);
        assert!(st.clients[&1].chooser.is_none());
        // and back again
        st.track_window_mru();
        press(&mut st, &shared, ctrl('b'));
        press(&mut st, &shared, plain(KeyCode::Char('w')));
        press(&mut st, &shared, plain(KeyCode::Enter));
        assert_eq!(st.sessions[0].active, 2);
    }

    #[test]
    fn w_inside_the_switcher_steps_forward() {
        let (mut st, shared) = fkey_state(false, false);
        st.sessions[0].windows.push(crate::server::Window {
            id: 60,
            name: "w1".into(),
            manual_name: true,
            layout: crate::layout::Node::Leaf(30),
            active: 30,
            zoomed: false,
        });
        st.clients.get_mut(&1).unwrap().chooser = Some(pack_chooser(0, Filter::Windows));
        press(&mut st, &shared, plain(KeyCode::Char('w')));
        assert_eq!(
            unpack_chooser(st.clients[&1].chooser.unwrap()),
            (1, Filter::Windows)
        );
        press(&mut st, &shared, plain(KeyCode::Char('w')));
        assert_eq!(
            unpack_chooser(st.clients[&1].chooser.unwrap()),
            (0, Filter::Windows)
        );
    }

    #[test]
    fn plain_key_rejects_ctrl_and_alt() {
        assert!(plain_key(&plain(KeyCode::Char('j'))));
        assert!(!plain_key(&ctrl('j')));
        assert!(!plain_key(&key(KeyCode::Char('j'), KeyModifiers::ALT)));
    }
}
