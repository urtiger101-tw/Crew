//! Copy mode: keyboard selection in a pane, and copying the selection to the clipboard.
use crate::copy;
use crate::server::State;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl State {
    pub(crate) fn copy_selection(&mut self, cid: usize, pid: usize) {
        let Some(p) = self.panes.get_mut(&pid) else {
            return;
        };
        let Some(cs) = p.copy.clone() else { return };
        let Some(anchor) = cs.anchor else { return };
        let off = p.view().screen().scrollback();
        let cur = (copy::abs_row(cs.cy, off), cs.cx);
        let text = copy::extract(p.view_mut(), anchor, cur, cs.line);
        p.leave_copy();
        self.set_clipboard(cid, text);
    }

    #[allow(unused_assignments)]
    pub(crate) fn copy_key(&mut self, cid: usize, pid: usize, k: KeyEvent) {
        let Some(p) = self.panes.get_mut(&pid) else {
            return;
        };
        let (h, w) = (p.rect.h, p.rect.w);
        // a zero-sized pane has no cell to move to; keys then only close copy mode
        if h == 0 || w == 0 {
            p.leave_copy();
            self.mark_dirty();
            return;
        }
        let mut off = p.view().screen().scrollback();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let mut cs = p.copy.clone().unwrap();
        let scroll_to = |n: usize, parser: &mut crate::server::PaneParser| -> usize {
            parser.screen_mut().set_scrollback(n);
            parser.screen().scrollback()
        };
        let row_text = |parser: &crate::server::PaneParser, y: u16| -> String {
            (0..w)
                .map(|x| {
                    parser
                        .screen()
                        .cell(y, x)
                        .map(|c| c.contents().to_string())
                        .unwrap_or_default()
                })
                .map(|s| if s.is_empty() { " ".into() } else { s })
                .collect()
        };
        let mut done = false;
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc
                if cs.anchor.is_none() || k.code == KeyCode::Char('q') =>
            {
                done = true
            }
            KeyCode::Esc => cs.anchor = None,
            KeyCode::Char('c') if ctrl => done = true,
            KeyCode::Up | KeyCode::Char('k') => {
                if cs.cy > 0 {
                    cs.cy -= 1
                } else {
                    off = scroll_to(off + 1, p.view_mut())
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if cs.cy + 1 < h {
                    cs.cy += 1
                } else if off > 0 {
                    off = scroll_to(off - 1, p.view_mut())
                }
            }
            KeyCode::Left | KeyCode::Char('h') => cs.cx = cs.cx.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => cs.cx = (cs.cx + 1).min(w - 1),
            KeyCode::Char('0') | KeyCode::Home => cs.cx = 0,
            KeyCode::Char('$') | KeyCode::End => {
                cs.cx =
                    (row_text(p.view(), cs.cy).trim_end().chars().count() as u16).saturating_sub(1)
            }
            KeyCode::Char('w') => {
                let t: Vec<char> = row_text(p.view(), cs.cy).chars().collect();
                let mut x = cs.cx as usize;
                while x < t.len() && !t[x].is_whitespace() {
                    x += 1;
                }
                while x < t.len() && t[x].is_whitespace() {
                    x += 1;
                }
                cs.cx = (x.min(t.len().saturating_sub(1))) as u16;
            }
            KeyCode::Char('b') if !ctrl => {
                let t: Vec<char> = row_text(p.view(), cs.cy).chars().collect();
                let mut x = cs.cx as usize;
                while x > 0 && t.get(x - 1).is_some_and(|c| c.is_whitespace()) {
                    x -= 1;
                }
                while x > 0 && t.get(x - 1).is_some_and(|c| !c.is_whitespace()) {
                    x -= 1;
                }
                cs.cx = x as u16;
            }
            KeyCode::PageUp | KeyCode::Char('b') => off = scroll_to(off + h as usize, p.view_mut()),
            KeyCode::PageDown | KeyCode::Char('f') => {
                off = scroll_to(off.saturating_sub(h as usize), p.view_mut())
            }
            KeyCode::Char('u') => off = scroll_to(off + h as usize / 2, p.view_mut()),
            KeyCode::Char('d') => off = scroll_to(off.saturating_sub(h as usize / 2), p.view_mut()),
            KeyCode::Char('g') => {
                off = scroll_to(usize::MAX, p.view_mut());
                cs.cy = 0;
            }
            KeyCode::Char('G') => {
                off = scroll_to(0, p.view_mut());
                cs.cy = h - 1;
            }
            KeyCode::Char('H') => cs.cy = 0,
            KeyCode::Char('L') => cs.cy = h - 1,
            KeyCode::Char('v') | KeyCode::Char(' ') => {
                cs.line = false;
                cs.anchor = if cs.anchor.is_some() && !cs.line {
                    None
                } else {
                    Some((copy::abs_row(cs.cy, off), cs.cx))
                };
            }
            KeyCode::Char('V') => {
                cs.line = true;
                cs.anchor = Some((copy::abs_row(cs.cy, off), cs.cx));
            }
            KeyCode::Char('y') | KeyCode::Enter => {
                if cs.anchor.is_some() {
                    p.copy = Some(cs);
                    self.copy_selection(cid, pid);
                    self.mark_dirty();
                    return;
                }
                done = true;
            }
            _ => {}
        }
        if done {
            p.leave_copy();
        } else {
            p.copy = Some(cs);
        }
        self.mark_dirty();
    }
}
