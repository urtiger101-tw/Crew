//! Move layout occupants without replacing their PTYs, buffers or agent identities.
use super::State;
use crate::workspace::order::{remember_clients, restore_clients};

impl State {
    pub(crate) fn swap_panes(&mut self, first: usize, second: usize) -> Result<bool, String> {
        let (si, wi) = self
            .locate(first)
            .ok_or_else(|| format!("no such pane: p{first}"))?;
        let other = self
            .locate(second)
            .ok_or_else(|| format!("no such pane: p{second}"))?;
        if other != (si, wi) {
            return Err("pane swap requires two panes in the same tab".into());
        }
        if first == second {
            return Ok(false);
        }
        let clients = remember_clients(self);
        let w = &mut self.sessions[si].windows[wi];
        w.layout = w.layout.swapped(first, second);
        let tab = w.id;
        let session = self.sessions[si].name.clone();
        for c in self.clients.values_mut().filter(|c| c.session == session) {
            c.drag = None;
        }
        restore_clients(self, clients);
        self.relayout(si);
        self.events.record(
            "pane.swapped",
            Some(first),
            serde_json::json!({
                "pane_id": crate::api::pane_id_str(first),
                "other_pane_id": crate::api::pane_id_str(second),
                "workspace_id": crate::workspace::workspace_id_str(self.sessions[si].id),
                "tab_id": crate::workspace::tab_id_str(tab),
            }),
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::layout::{Dir, Node};
    use crate::server::{test_session, test_state};

    #[test]
    fn swap_preserves_focus_geometry_and_rejects_cross_tab_before_mutation() {
        let mut st = test_state();
        let mut s = test_session("main", 120, 30, 1);
        s.windows[0].layout = Node::Split {
            dir: Dir::H,
            ratio: 0.3,
            a: Box::new(Node::Leaf(1)),
            b: Box::new(Node::Leaf(2)),
        };
        st.sessions.push(s);
        st.sessions.push(test_session("other", 120, 30, 3));
        assert!(st.swap_panes(1, 3).is_err());
        assert!(st.swap_panes(1, 99).is_err());
        assert_eq!(st.sessions[0].windows[0].panes(), vec![1, 2]);
        assert_eq!(st.swap_panes(1, 1), Ok(false));
        let before = st.visible_rects(&st.sessions[0], &st.sessions[0].windows[0]);
        assert_eq!(st.swap_panes(1, 2), Ok(true));
        assert_eq!(st.sessions[0].windows[0].panes(), vec![2, 1]);
        assert_eq!(st.sessions[0].windows[0].active, 1);
        let after = st.visible_rects(&st.sessions[0], &st.sessions[0].windows[0]);
        assert_eq!(before[0].1, after[0].1);
        assert_eq!(before[1].1, after[1].1);
    }
}
