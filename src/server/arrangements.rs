//! Apply one rebuilt layout and resize existing PTYs once through relayout.
use super::State;
use crate::layout::Preset;

impl State {
    pub(crate) fn select_layout(&mut self, si: usize, wi: usize, preset: Preset) -> bool {
        let w = &mut self.sessions[si].windows[wi];
        let panes = w.panes();
        if panes.len() < 2 {
            return false;
        }
        let layout = preset.build(&panes).expect("an existing window has panes");
        if w.layout == layout && !w.zoomed && w.layout_preset == Some(preset) {
            return false;
        }
        w.layout = layout;
        w.layout_preset = Some(preset);
        // Reveal the new layout; active pane, parser/history and process handles stay owned.
        w.zoomed = false;
        let session = self.sessions[si].name.clone();
        for client in self.clients.values_mut().filter(|c| c.session == session) {
            client.drag = None;
        }
        self.relayout(si);
        true
    }
}

#[cfg(test)]
mod tests;
