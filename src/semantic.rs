//! Shared visual state vocabulary. Canonical agent lifecycle and waits stay in `api::agent`.
use crate::api::{Status, agent};
use crate::config::Config;
use crate::i18n::{Lang, tr};
use crate::server::Pane;
use serde::Serialize;
use std::time::Duration;

/// Completion remains a recent visual signal for five seconds after its report,
/// or after the last output when completion is inferred without a new report.
/// Unseen completion/attention itself is not acknowledged or changed by this deadline.
pub const DONE_VISUAL_WINDOW: Duration = Duration::from_secs(5);

/// Visual states can describe host loss/restoration without extending the API status enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayState {
    Working,
    Blocked,
    Done,
    Idle,
    Unknown,
    Error,
    Restored,
    ResumeRequested,
}

/// Theme-independent color roles shared by TUI chrome and API display metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorRole {
    Working,
    Blocked,
    Done,
    Muted,
    Error,
}

/// The single state vocabulary: words, accessible shapes, emphasis and color role.
#[derive(Debug, Clone, Copy)]
pub struct StateDescriptor {
    pub state: DisplayState,
    pub word: &'static str,
    pub word_key: &'static str,
    pub glyph: &'static str,
    pub ascii_glyph: &'static str,
    pub color_role: ColorRole,
    pub bold: bool,
}

const STATES: [StateDescriptor; 8] = [
    StateDescriptor {
        state: DisplayState::Working,
        word: "working",
        word_key: "render.state.working",
        glyph: "◐",
        ascii_glyph: "@",
        color_role: ColorRole::Working,
        bold: false,
    },
    StateDescriptor {
        state: DisplayState::Blocked,
        word: "needs input",
        word_key: "render.state.blocked",
        glyph: "●",
        ascii_glyph: "!",
        color_role: ColorRole::Blocked,
        bold: true,
    },
    StateDescriptor {
        state: DisplayState::Done,
        word: "done",
        word_key: "render.state.done",
        glyph: "✓",
        ascii_glyph: "+",
        color_role: ColorRole::Done,
        bold: false,
    },
    StateDescriptor {
        state: DisplayState::Idle,
        word: "idle",
        word_key: "render.state.idle",
        glyph: "○",
        ascii_glyph: "-",
        color_role: ColorRole::Muted,
        bold: false,
    },
    StateDescriptor {
        state: DisplayState::Unknown,
        word: "unknown",
        word_key: "render.state.unknown",
        glyph: "?",
        ascii_glyph: "?",
        color_role: ColorRole::Muted,
        bold: false,
    },
    StateDescriptor {
        state: DisplayState::Error,
        word: "host lost",
        word_key: "render.state.host_lost",
        glyph: "!",
        ascii_glyph: "!",
        color_role: ColorRole::Error,
        bold: true,
    },
    StateDescriptor {
        state: DisplayState::Restored,
        word: "restored, not running",
        word_key: "render.restore.not_running",
        glyph: "○",
        ascii_glyph: "-",
        color_role: ColorRole::Muted,
        bold: false,
    },
    StateDescriptor {
        state: DisplayState::ResumeRequested,
        word: "resume requested, not running",
        word_key: "render.restore.requested",
        glyph: "○",
        ascii_glyph: "-",
        color_role: ColorRole::Muted,
        bold: false,
    },
];

/// Descriptor for a visual state; enum order is kept explicit at this boundary.
pub fn descriptor(state: DisplayState) -> &'static StateDescriptor {
    &STATES[match state {
        DisplayState::Working => 0,
        DisplayState::Blocked => 1,
        DisplayState::Done => 2,
        DisplayState::Idle => 3,
        DisplayState::Unknown => 4,
        DisplayState::Error => 5,
        DisplayState::Restored => 6,
        DisplayState::ResumeRequested => 7,
    }]
}

/// Shared descriptor for the existing five canonical API statuses.
pub fn status_descriptor(status: Status) -> &'static StateDescriptor {
    descriptor(match status {
        Status::Working => DisplayState::Working,
        Status::Blocked => DisplayState::Blocked,
        Status::Done => DisplayState::Done,
        Status::Idle => DisplayState::Idle,
        Status::Unknown => DisplayState::Unknown,
    })
}

/// A localized canonical status label, also usable by legacy overlays and templates.
pub fn status_word(lang: Lang, status: Status) -> &'static str {
    Presentation::from(status).word(lang)
}

/// Additive display metadata; `status` retains the canonical lifecycle value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Presentation {
    pub status: Status,
    pub state: DisplayState,
    pub word: &'static str,
    pub glyph: &'static str,
    pub ascii_glyph: &'static str,
    pub color_role: ColorRole,
    pub bold: bool,
    #[serde(skip)]
    word_key: &'static str,
}

impl Presentation {
    /// Visible word in the attaching client's language; API metadata remains English.
    pub fn word(&self, lang: Lang) -> &'static str {
        let translated = tr(lang, self.word_key);
        if translated == self.word_key {
            self.word
        } else {
            translated
        }
    }

    /// Accessible state shape for this client's Unicode/ASCII choice.
    pub fn glyph(&self, ascii: bool) -> &'static str {
        if ascii { self.ascii_glyph } else { self.glyph }
    }
}

impl From<Status> for Presentation {
    fn from(status: Status) -> Self {
        present(status, false, None, Duration::ZERO)
    }
}

fn present(
    status: Status,
    host_lost: bool,
    restored: Option<&str>,
    output_age: Duration,
) -> Presentation {
    let state = if host_lost {
        DisplayState::Error
    } else {
        match restored {
            Some("not-running" | "restored-shell") => DisplayState::Restored,
            Some("resume-requested") => DisplayState::ResumeRequested,
            _ if status == Status::Done && output_age >= DONE_VISUAL_WINDOW => DisplayState::Idle,
            _ => status_descriptor(status).state,
        }
    };
    let d = descriptor(state);
    let mut result = Presentation {
        status,
        state,
        word: d.word,
        glyph: d.glyph,
        ascii_glyph: d.ascii_glyph,
        color_role: d.color_role,
        bold: d.bold,
        word_key: d.word_key,
    };
    if restored == Some("restored-shell") && !host_lost {
        result.word = "restored shell";
        result.word_key = "render.restore.shell";
    }
    result
}

/// Resolve host/restoration notices and recent completion without mutating the pane.
pub fn pane(p: &Pane, cfg: &Config) -> Presentation {
    present(
        agent::status(p, cfg),
        p.host_lost.is_some(),
        crate::persist::restore_state(p),
        p.done_since.unwrap_or(p.last_output).elapsed(),
    )
}

/// Stable identity for color assignment; presentation titles and pane positions are excluded.
pub fn identity(p: &Pane) -> Option<&str> {
    p.agent
        .as_ref()
        .map(|a| a.name.as_deref().unwrap_or(&a.agent))
        .or_else(|| p.detected_agent.as_ref().map(|d| d.agent.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_preserves_api_values_and_accessible_shapes() {
        for status in [
            Status::Working,
            Status::Blocked,
            Status::Done,
            Status::Idle,
            Status::Unknown,
        ] {
            let p = present(status, false, None, Duration::ZERO);
            assert_eq!(p.status, status);
            assert_eq!(p.word(Lang::En), p.word);
            assert!(!p.word(Lang::ZhTw).is_empty());
            assert!(!p.glyph(false).is_empty());
            assert!(p.glyph(true).is_ascii());
            let json = serde_json::to_value(p).unwrap();
            assert_eq!(json["status"], status.as_str());
            assert_eq!(json["word"], p.word);
            assert_eq!(json["glyph"], p.glyph(false));
        }
        let blocked = present(Status::Blocked, false, None, Duration::ZERO);
        assert!(blocked.bold);
        assert_eq!(blocked.color_role, ColorRole::Blocked);
        assert_eq!(status_word(Lang::En, Status::Blocked), "needs input");
    }

    #[test]
    fn completion_decay_changes_only_visual_state() {
        let recent = present(
            Status::Done,
            false,
            None,
            DONE_VISUAL_WINDOW - Duration::from_millis(1),
        );
        let expired = present(Status::Done, false, None, DONE_VISUAL_WINDOW);
        assert_eq!(recent.state, DisplayState::Done);
        assert_eq!(expired.state, DisplayState::Idle);
        assert_eq!(expired.status, Status::Done);
        assert_eq!(expired.color_role, ColorRole::Muted);
        for status in [
            Status::Working,
            Status::Blocked,
            Status::Idle,
            Status::Unknown,
        ] {
            assert_eq!(
                present(status, false, None, Duration::from_secs(600)).state,
                status_descriptor(status).state
            );
        }
    }

    #[test]
    fn host_loss_and_restoration_have_explicit_visual_priority() {
        for restored in [
            None,
            Some("not-running"),
            Some("restored-shell"),
            Some("resume-requested"),
        ] {
            let lost = present(Status::Working, true, restored, Duration::ZERO);
            assert_eq!(lost.state, DisplayState::Error);
            assert_eq!(lost.color_role, ColorRole::Error);
            assert!(lost.bold);
            assert_eq!(lost.status, Status::Working);
        }
        assert_eq!(
            present(Status::Idle, false, Some("not-running"), Duration::ZERO).state,
            DisplayState::Restored
        );
        assert_eq!(
            present(
                Status::Idle,
                false,
                Some("resume-requested"),
                Duration::ZERO
            )
            .state,
            DisplayState::ResumeRequested
        );
        let shell = present(Status::Idle, false, Some("restored-shell"), Duration::ZERO);
        assert_eq!(shell.state, DisplayState::Restored);
        assert_eq!(shell.word(Lang::En), "restored shell");
    }
}
