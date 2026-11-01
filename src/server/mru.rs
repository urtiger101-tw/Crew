//! Most-recently-used window per session, for the `choose-window` switcher (`prefix w`): like
//! Alt+Tab, the switcher opens on the window used before the current one, so `prefix w Enter`
//! toggles between the last two windows.
use super::State;
use crate::render::{Filter, pack_chooser, visible_entries};

impl State {
    /// Record each session's active window; when it changed, the old one becomes "previous".
    /// Called once per render pass, so every way of switching windows (keys, mouse, chooser,
    /// API) is covered without hooking each of them. Entries of closed sessions are dropped.
    pub(crate) fn track_window_mru(&mut self) {
        let live: Vec<(usize, usize)> = self
            .sessions
            .iter()
            .filter_map(|s| s.windows.get(s.active).map(|w| (s.id, w.id)))
            .collect();
        self.window_mru
            .retain(|sid, _| live.iter().any(|(id, _)| id == sid));
        for (sid, wid) in live {
            let entry = self.window_mru.entry(sid).or_insert((wid, None));
            if entry.0 != wid {
                *entry = (wid, Some(entry.0));
            }
        }
    }

    /// Index of the window used before the current one in session `si`, if it still exists.
    pub(crate) fn previous_window(&self, si: usize) -> Option<usize> {
        let s = self.sessions.get(si)?;
        let (_, prev) = *self.window_mru.get(&s.id)?;
        let prev = prev?;
        s.windows
            .iter()
            .position(|w| w.id == prev)
            .filter(|&wi| wi != s.active)
    }

    /// Packed chooser value for the window switcher of session `si`: window rows only, with
    /// the previous window selected (else the next one, else the current one).
    pub(crate) fn window_switcher(&mut self, si: usize) -> usize {
        self.track_window_mru();
        let Some(s) = self.sessions.get(si) else {
            return pack_chooser(0, Filter::Windows);
        };
        let n = s.windows.len().max(1);
        let target = self.previous_window(si).unwrap_or((s.active + 1) % n);
        let entries = visible_entries(self, Filter::Windows);
        let sel = entries
            .iter()
            .position(|e| e.0 == si && e.1 == Some(target))
            .or_else(|| {
                entries
                    .iter()
                    .position(|e| e.0 == si && e.1 == Some(s.active))
            })
            .unwrap_or(0);
        pack_chooser(sel, Filter::Windows)
    }
}

#[cfg(test)]
mod tests {
    use crate::layout::Node;
    use crate::render::{Filter, unpack_chooser, visible_entries};
    use crate::server::{Window, test_session, test_state};

    fn state_with_windows(n: usize) -> crate::server::State {
        let mut st = test_state();
        let mut s = test_session("main", 80, 24, 1);
        for i in 1..n {
            s.windows.push(Window {
                id: 100 + i,
                name: format!("w{i}"),
                manual_name: true,
                layout: Node::Leaf(10 + i),
                active: 10 + i,
                zoomed: false,
            });
        }
        st.sessions.push(s);
        st
    }

    fn selected_window(st: &crate::server::State, raw: usize) -> Option<usize> {
        let (sel, filter) = unpack_chooser(raw);
        assert_eq!(filter, Filter::Windows);
        visible_entries(st, Filter::Windows)
            .get(sel)
            .and_then(|e| e.1)
    }

    #[test]
    fn switcher_lists_only_window_rows() {
        let st = state_with_windows(3);
        let entries = visible_entries(&st, Filter::Windows);
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.1.is_some() && e.2.is_none()));
    }

    #[test]
    fn switcher_opens_on_the_previously_used_window() {
        let mut st = state_with_windows(4);
        st.track_window_mru();
        st.sessions[0].active = 2;
        st.track_window_mru();
        st.sessions[0].active = 3;
        st.track_window_mru();
        assert_eq!(st.previous_window(0), Some(2));
        let raw = st.window_switcher(0);
        assert_eq!(selected_window(&st, raw), Some(2));
    }

    #[test]
    fn switcher_without_history_opens_on_the_next_window() {
        let mut st = state_with_windows(3);
        let raw = st.window_switcher(0);
        assert_eq!(selected_window(&st, raw), Some(1));
        st.sessions[0].active = 2;
        st.window_mru.clear();
        let raw = st.window_switcher(0);
        assert_eq!(
            selected_window(&st, raw),
            Some(0),
            "wraps to the first window"
        );
    }

    #[test]
    fn a_closed_previous_window_is_forgotten() {
        let mut st = state_with_windows(3);
        st.track_window_mru();
        st.sessions[0].active = 1;
        st.track_window_mru();
        st.sessions[0].windows.remove(0);
        st.sessions[0].active = 0;
        assert_eq!(st.previous_window(0), None);
    }

    #[test]
    fn single_window_switcher_selects_it() {
        let mut st = state_with_windows(1);
        let raw = st.window_switcher(0);
        assert_eq!(selected_window(&st, raw), Some(0));
    }
}
