//! One display projection for API snapshots and TUI rows; no semantic state changes.
use super::{Key, presentation};
use crate::i18n::Lang;
use crate::server::State;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

const MAX_TEMPLATE: usize = 512;
const MAX_OUTPUT: usize = 512;
const MAX_SEGMENTS: usize = 16;

/// A literal text template, optionally styled for one sidebar occurrence.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SidebarSegment {
    Template(String),
    Styled(StyledSegment),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyledSegment {
    pub template: String,
    pub fg: Option<String>,
    pub bold: Option<bool>,
    pub dim: Option<bool>,
}

/// Plain text shared by API and TUI. Styles never contain terminal escape codes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DisplaySpan {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<bool>,
}

impl DisplaySpan {
    fn plain(text: String) -> Self {
        Self {
            text,
            fg: None,
            bold: None,
            dim: None,
        }
    }
}

/// Strict ASCII CSS-style RGB; neither ANSI sequences nor theme token names are accepted.
pub fn segment_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    if !matches!(hex.len(), 3 | 6) || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let component = |i| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    if hex.len() == 6 {
        Some((component(0)?, component(2)?, component(4)?))
    } else {
        let mut values = hex.chars().map(|c| c.to_digit(16).map(|n| n as u8 * 17));
        Some((values.next()??, values.next()??, values.next()??))
    }
}

pub fn validate_segments(segments: &[SidebarSegment]) -> Result<(), String> {
    if segments.len() > MAX_SEGMENTS {
        return Err("sidebar_pane_segments must contain at most 16 segments".into());
    }
    for segment in segments {
        let pattern = match segment {
            SidebarSegment::Template(pattern) => pattern,
            SidebarSegment::Styled(style) => {
                if style
                    .fg
                    .as_deref()
                    .is_some_and(|fg| segment_rgb(fg).is_none())
                {
                    return Err("sidebar segment fg must be #RGB or #RRGGBB".into());
                }
                &style.template
            }
        };
        validate_template(pattern)?;
    }
    Ok(())
}

/// Drop empty values before adding separators; the combined text stays bounded.
fn render_segments(
    segments: &[SidebarSegment],
    values: &BTreeMap<String, String>,
) -> Vec<DisplaySpan> {
    let mut spans = Vec::new();
    let mut remaining = MAX_OUTPUT;
    for segment in segments.iter().take(MAX_SEGMENTS) {
        let (pattern, style) = match segment {
            SidebarSegment::Template(pattern) => (pattern, None),
            SidebarSegment::Styled(style) => (&style.template, Some(style)),
        };
        let text = template(pattern, values);
        if text.is_empty() {
            continue;
        }
        let separator = if spans.is_empty() { "" } else { " · " };
        let separator_len = separator.chars().count();
        if remaining <= separator_len {
            break;
        }
        if !separator.is_empty() {
            spans.push(DisplaySpan::plain(separator.into()));
            remaining -= separator_len;
        }
        let text: String = text.chars().take(remaining).collect();
        remaining -= text.chars().count();
        spans.push(DisplaySpan {
            text,
            fg: style.and_then(|s| s.fg.clone()),
            bold: style.and_then(|s| s.bold),
            dim: style.and_then(|s| s.dim),
        });
        if remaining == 0 {
            break;
        }
    }
    spans
}

pub fn validate_template(value: &str) -> Result<(), String> {
    if value.chars().count() > MAX_TEMPLATE
        || value.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return Err("display template must contain at most 512 characters without control or bidi formatting characters".into());
    }
    Ok(())
}

/// `$name` and `${name}` perform one literal lookup. `$$` is a literal dollar.
/// Values are never expanded again; shell commands and markup have no meaning here.
pub fn template(pattern: &str, values: &BTreeMap<String, String>) -> String {
    let input: Vec<char> = pattern.chars().take(MAX_TEMPLATE).collect();
    let mut out = String::new();
    let mut count = 0;
    let mut i = 0;
    let mut append = |text: &str| {
        for c in text.chars().filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        }) {
            if count >= MAX_OUTPUT {
                break;
            }
            out.push(c);
            count += 1;
        }
    };
    while i < input.len() {
        if input[i] != '$' {
            append(&input[i].to_string());
            i += 1;
            continue;
        }
        i += 1;
        if input.get(i) == Some(&'$') {
            append("$");
            i += 1;
            continue;
        }
        let braced = input.get(i) == Some(&'{');
        let start = i + usize::from(braced);
        let mut end = start;
        while input
            .get(end)
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            end += 1;
        }
        if end == start || (braced && input.get(end) != Some(&'}')) {
            append("$");
            continue;
        }
        let key: String = input[start..end].iter().collect();
        if let Some(value) = values.get(&key) {
            append(value);
        }
        i = end + usize::from(braced);
    }
    out.trim().to_string()
}

fn with_fallback(pattern: &str, values: &BTreeMap<String, String>, fallback: &str) -> String {
    if pattern.is_empty() {
        return fallback.to_string();
    }
    let rendered = template(pattern, values);
    if rendered.is_empty() {
        fallback.to_string()
    } else {
        rendered
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PaneDisplay {
    pub title: String,
    /// Sanitized OSC 0/2 hint; never used to determine agent kind or completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_title: Option<String>,
    pub agent: Option<String>,
    pub state_label: String,
    /// Canonical status plus the common visual word, glyph, color role and emphasis.
    pub semantic: crate::semantic::Presentation,
    /// Stable agent identity used for pastel assignment; display titles do not affect it.
    pub identity: Option<String>,
    /// Theme-configured accent as #rrggbb, palette index or reset; no ANSI escape codes.
    pub identity_accent: Option<String>,
    pub sidebar: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sidebar_spans: Vec<DisplaySpan>,
    pub kind_source: &'static str,
    pub presentation: presentation::Presentation,
}

fn pane_title<'a>(
    presentation: &'a presentation::Presentation,
    agent_name: Option<&'a str>,
    label: Option<&'a str>,
    terminal_title: Option<&'a str>,
    kind: Option<&'a str>,
    window_name: &'a str,
) -> &'a str {
    presentation
        .title
        .as_deref()
        .or(presentation.display_agent.as_deref())
        .or(agent_name)
        .or(kind)
        .or(label)
        .or(terminal_title)
        .unwrap_or(window_name)
}

fn informative_terminal_title<'a>(
    title: Option<&'a str>,
    has_agent: bool,
    window_name: &str,
) -> Option<&'a str> {
    title.filter(|title| {
        if !has_agent || title.eq_ignore_ascii_case(window_name) {
            return false;
        }
        let value = title.trim_matches('"');
        let lower = value.to_ascii_lowercase();
        let executable = [".exe", ".com", ".cmd", ".bat"]
            .iter()
            .any(|ext| lower.ends_with(ext))
            && (value.contains(['/', '\\']) || !value.chars().any(char::is_whitespace));
        !executable
            && !matches!(
                lower.rsplit(['/', '\\']).next().unwrap_or(&lower),
                "crew"
                    | "tmuxlike"
                    | "powershell"
                    | "windows powershell"
                    | "pwsh"
                    | "cmd"
                    | "command prompt"
                    | "conhost"
                    | "sh"
                    | "bash"
                    | "zsh"
            )
    })
}

pub fn pane(st: &State, pid: usize, lang: Lang) -> Option<PaneDisplay> {
    let p = st.panes.get(&pid)?;
    let (si, wi) = st.locate(pid)?;
    let s = &st.sessions[si];
    let w = &s.windows[wi];
    let explicit = p.agent.as_ref();
    let detected = p.detected_agent.as_ref();
    let kind = explicit
        .map(|a| a.agent.as_str())
        .or_else(|| detected.map(|d| d.agent.as_str()));
    let now = Instant::now();
    let presentation = st.metadata.presentation(
        Key::Pane(pid),
        now,
        kind,
        explicit.map(|a| a.source.as_str()),
    );
    let effective_agent = presentation
        .display_agent
        .as_deref()
        .or_else(|| explicit.and_then(|a| a.name.as_deref()))
        .or(kind);
    let terminal_title = informative_terminal_title(
        p.parser.callbacks().terminal_title.as_deref(),
        explicit.is_some() || detected.is_some(),
        &w.name,
    );
    let title = pane_title(
        &presentation,
        explicit.and_then(|agent| agent.name.as_deref()),
        p.label.as_deref(),
        terminal_title,
        kind,
        &w.name,
    );
    let semantic = crate::semantic::pane(p, &st.cfg);
    let status = semantic.status;
    let default_state = semantic.word(lang);
    let default_state = if default_state.starts_with("render.") {
        status.as_str()
    } else {
        default_state
    };
    let state_label = presentation
        .state_labels
        .get(status.as_str())
        .map(String::as_str)
        .unwrap_or(default_state);
    let mut values = st.metadata.values(Key::Pane(pid), now);
    for (key, value) in [
        ("title", title),
        ("pane", title),
        ("agent", effective_agent.unwrap_or("")),
        ("workspace", s.name.as_str()),
        ("tab", w.name.as_str()),
        ("state", status.as_str()),
        ("state_text", state_label),
    ] {
        values.insert(key.into(), value.to_string());
    }
    values.insert("pane_id".into(), crate::api::pane_id_str(pid));
    values.insert(
        "workspace_id".into(),
        crate::workspace::workspace_id_str(s.id),
    );
    values.insert("tab_id".into(), crate::workspace::tab_id_str(w.id));
    let sidebar_spans = render_segments(&st.cfg.sidebar_pane_segments, &values);
    let sidebar = if sidebar_spans.is_empty() {
        with_fallback(&st.cfg.sidebar_pane_template, &values, title)
    } else {
        sidebar_spans
            .iter()
            .map(|span| span.text.as_str())
            .collect()
    };
    let identity = crate::semantic::identity(p);
    let identity_accent = identity.and_then(|name| {
        use crossterm::style::Color;
        let theme = crate::theme::Theme::resolve(&st.cfg.theme, &Default::default());
        match theme.agent_accent(name) {
            Color::Rgb { r, g, b } => Some(format!("#{r:02x}{g:02x}{b:02x}")),
            Color::AnsiValue(index) => Some(index.to_string()),
            Color::Reset => Some("reset".into()),
            _ => None,
        }
    });
    Some(PaneDisplay {
        title: with_fallback(&st.cfg.pane_template, &values, title),
        terminal_title: terminal_title.map(str::to_owned),
        agent: effective_agent.map(str::to_string),
        state_label: state_label.to_string(),
        semantic,
        identity: identity.map(str::to_owned),
        identity_accent,
        sidebar,
        sidebar_spans,
        kind_source: if explicit.is_some() {
            "explicit-report"
        } else if detected.is_some() {
            "process-detection"
        } else {
            "none"
        },
        presentation,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceDisplay {
    pub title: String,
    pub sidebar: String,
    pub presentation: presentation::Presentation,
}

pub fn workspace(st: &State, si: usize) -> WorkspaceDisplay {
    let s = &st.sessions[si];
    let now = Instant::now();
    let presentation = st
        .metadata
        .presentation(Key::Workspace(s.id), now, None, None);
    let title = presentation.title.as_deref().unwrap_or(&s.name);
    let mut values = st.metadata.values(Key::Workspace(s.id), now);
    values.insert("title".into(), title.to_string());
    values.insert("workspace".into(), s.name.clone());
    values.insert(
        "workspace_id".into(),
        crate::workspace::workspace_id_str(s.id),
    );
    WorkspaceDisplay {
        title: title.to_string(),
        sidebar: with_fallback(&st.cfg.sidebar_workspace_template, &values, title),
        presentation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additive_state_and_identity_metadata_preserve_canonical_authority() {
        use crate::api::{AgentInfo, Status};
        use crate::semantic::DisplayState;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        // This is an in-memory server with one real shell, no listener or shared endpoint.
        struct ShellFixture(crate::server::Shared);
        impl Drop for ShellFixture {
            fn drop(&mut self) {
                let mut st = crate::api::lock(&self.0);
                if !st.sessions.is_empty() {
                    st.kill_session(0);
                }
            }
        }
        let mut st = crate::server::test_state();
        st.cfg.shell = Some(
            if cfg!(windows) {
                "cmd.exe /Q"
            } else {
                "/bin/sh"
            }
            .into(),
        );
        st.cfg.theme.preset = "aurora".into();
        st.cfg.pane_template = "Dynamic display title".into();
        let shared = Arc::new(Mutex::new(st));
        let _shell = ShellFixture(shared.clone());
        let mut st = crate::api::lock(&shared);
        st.new_session(&shared, Some("semantic-fixture".into()), 80, 24, None)
            .unwrap();
        let pid = st.active_pane(0);
        let p = st.panes.get_mut(&pid).unwrap();
        p.agent = Some(AgentInfo {
            instance: 1,
            source: "test".into(),
            agent: "fixture".into(),
            state: Status::Idle,
            message: None,
            seq: Some(1),
            name: Some("codex".into()),
            resume: Vec::new(),
        });
        p.attention = true;
        p.last_output = Instant::now() - Duration::from_secs(10);
        let display = pane(&st, pid, Lang::En).unwrap();
        assert_eq!(display.title, "Dynamic display title");
        assert_eq!(display.identity.as_deref(), Some("codex"));
        assert_eq!(display.identity_accent.as_deref(), Some("#f2a7c3"));
        assert_eq!(display.semantic.status, Status::Done);
        assert_eq!(display.semantic.state, DisplayState::Idle);
        assert_eq!(
            display.state_label, "idle",
            "display follows visual decay while canonical status is retained"
        );
        let json = crate::api::pane_json(&st, pid).unwrap();
        assert_eq!(json["status"], "done");
        assert_eq!(json["display"]["semantic"]["state"], "idle");
        assert_eq!(json["display"]["semantic"]["word"], "idle");
        assert_eq!(json["display"]["semantic"]["color_role"], "muted");

        st.panes
            .get_mut(&pid)
            .unwrap()
            .agent
            .as_mut()
            .unwrap()
            .state = Status::Blocked;
        let display = pane(&st, pid, Lang::ZhTw).unwrap();
        assert_eq!(display.semantic.word(Lang::ZhTw), "待回應");
        assert!(display.semantic.bold);
        st.panes.get_mut(&pid).unwrap().host_lost = Some(crate::server::host::HostLost {
            since: Instant::now(),
            survivors: vec![],
        });
        let display = pane(&st, pid, Lang::En).unwrap();
        assert_eq!(display.semantic.state, DisplayState::Error);
        assert_eq!(display.semantic.status, Status::Blocked);
        let p = st.panes.get_mut(&pid).unwrap();
        p.host_lost = None;
        p.agent.as_mut().unwrap().source = crate::api::RESTORE_SOURCE.into();
        assert_eq!(
            pane(&st, pid, Lang::En).unwrap().semantic.state,
            DisplayState::Restored
        );
        st.panes.get_mut(&pid).unwrap().resume_requested = true;
        assert_eq!(
            pane(&st, pid, Lang::En).unwrap().semantic.state,
            DisplayState::ResumeRequested
        );
        st.panes.get_mut(&pid).unwrap().agent.as_mut().unwrap().name = None;
        assert_eq!(
            pane(&st, pid, Lang::En).unwrap().identity.as_deref(),
            Some("fixture")
        );
        let p = st.panes.get_mut(&pid).unwrap();
        p.agent = None;
        p.detected_agent = Some(crate::agent_detection::Detection {
            agent: "terra".into(),
            process_id: 123,
            executable: "terra".into(),
            evidence: "test",
        });
        assert_eq!(
            pane(&st, pid, Lang::En).unwrap().identity.as_deref(),
            Some("terra")
        );
    }

    #[test]
    fn terminal_hints_require_an_agent_and_ignore_host_shell_defaults() {
        for title in [
            "Review collaboration task",
            r"D:\Crew\crew.exe",
            "Windows PowerShell",
        ] {
            assert_eq!(
                informative_terminal_title(Some(title), false, "powershell"),
                None
            );
        }
        for title in [
            r"D:\Crew\crew.exe",
            r"\\?\D:\Crew\crew.exe",
            "bin/crew.exe",
            "crew.exe",
            "powershell.exe",
            "Windows PowerShell",
            "pwsh",
            "cmd",
            "conhost",
            "/bin/bash",
            "window label",
        ] {
            assert_eq!(
                informative_terminal_title(Some(title), true, "window label"),
                None,
                "{title}"
            );
        }
        assert_eq!(
            informative_terminal_title(Some("Review;中文"), true, "powershell"),
            Some("Review;中文")
        );
        assert_eq!(
            informative_terminal_title(Some("Fix crew.exe"), true, "powershell"),
            Some("Fix crew.exe")
        );
    }

    #[test]
    fn terminal_title_preserves_legacy_kind_and_label_precedence() {
        let mut presentation = presentation::Presentation::default();
        let title = |presentation: &presentation::Presentation, name, label, terminal| {
            pane_title(
                presentation,
                name,
                label,
                terminal,
                Some("codex"),
                "powershell",
            )
            .to_owned()
        };
        assert_eq!(
            title(&presentation, None, None, Some("Review collaboration task")),
            "codex"
        );
        assert_eq!(
            title(
                &presentation,
                None,
                Some("Manual label"),
                Some("Terminal title")
            ),
            "codex"
        );
        assert_eq!(
            title(
                &presentation,
                Some("Reviewer"),
                Some("Manual label"),
                Some("Terminal title")
            ),
            "Reviewer"
        );
        presentation.display_agent = Some("Custom agent".into());
        assert_eq!(
            title(
                &presentation,
                Some("Reviewer"),
                None,
                Some("Terminal title")
            ),
            "Custom agent"
        );
        presentation.title = Some("Explicit title".into());
        assert_eq!(
            title(
                &presentation,
                Some("Reviewer"),
                None,
                Some("Terminal title")
            ),
            "Explicit title"
        );
        presentation.title = Some(String::new());
        assert_eq!(
            title(
                &presentation,
                Some("Reviewer"),
                None,
                Some("Terminal title")
            ),
            ""
        );
        assert_eq!(
            pane_title(
                &presentation::Presentation::default(),
                None,
                None,
                None,
                Some("codex"),
                "powershell"
            ),
            "codex"
        );
        assert_eq!(
            pane_title(
                &presentation::Presentation::default(),
                None,
                None,
                None,
                None,
                "powershell"
            ),
            "powershell"
        );
    }

    #[test]
    fn substitutions_are_literal_bounded_and_do_not_recurse() {
        let values = BTreeMap::from([
            ("task".into(), "$other 中文".into()),
            ("other".into(), "wrong".into()),
        ]);
        assert_eq!(
            template("${task}/$missing/$$/$(command)", &values),
            "$other 中文//$/$(command)"
        );
        assert_eq!(template("${broken", &values), "${broken");
        assert_eq!(template(&"x".repeat(900), &values).len(), 512);
        let repeated = BTreeMap::from([("long".into(), "中".repeat(200))]);
        assert_eq!(
            template("$long $long $long", &repeated).chars().count(),
            512
        );
        assert_eq!(template("\u{1b}$task\n\u{202e}", &values), "$other 中文");
    }
    #[test]
    fn validation_and_empty_fallback() {
        assert!(validate_template("$task 中文").is_ok());
        assert!(validate_template("\u{1b}[31m").is_err());
        for value in [
            "\t$title",
            "$title\n",
            "\r$title\t",
            "\u{202e}$title",
            "$title\u{2066}",
        ] {
            assert!(
                validate_template(value).is_err(),
                "accepted control/bidi in {value:?}"
            );
        }
        assert!(validate_template(&"x".repeat(513)).is_err());
        assert_eq!(with_fallback("$missing", &BTreeMap::new(), "p1"), "p1");
    }

    #[test]
    fn sidebar_segments_drop_empty_values_and_keep_literal_styles() {
        let config: crate::config::Config = toml::from_str(
            r##"
sidebar_pane_segments = [
    "$missing",
    { template = "$left", fg = "#3aF", bold = false, dim = true },
    "${absent}",
    { template = "$right", bold = true, dim = false },
]
"##,
        )
        .unwrap();
        validate_segments(&config.sidebar_pane_segments).unwrap();
        let values = BTreeMap::from([
            ("left".into(), "$right\u{1b}台\u{202e}灣".into()),
            ("right".into(), "完成".into()),
        ]);
        let spans = render_segments(&config.sidebar_pane_segments, &values);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].text, "$right台灣");
        assert_eq!(spans[0].fg.as_deref(), Some("#3aF"));
        assert_eq!(spans[0].bold, Some(false));
        assert_eq!(spans[0].dim, Some(true));
        assert_eq!(spans[1], DisplaySpan::plain(" · ".into()));
        assert_eq!(spans[2].text, "完成");
        let json = serde_json::to_value(&spans).unwrap();
        assert_eq!(json[0]["bold"], false);
        assert_eq!(json[1], serde_json::json!({"text": " · "}));
        assert_eq!(
            spans.iter().map(|s| s.text.as_str()).collect::<String>(),
            "$right台灣 · 完成"
        );
        assert!(
            render_segments(&[SidebarSegment::Template("$missing".into())], &values).is_empty()
        );
    }

    #[test]
    fn sidebar_segment_limit_applies_to_the_complete_row_without_orphan_separator() {
        let values = BTreeMap::from([("long".into(), "界".repeat(510))]);
        let segments = vec![
            SidebarSegment::Template("$long".into()),
            SidebarSegment::Template("tail".into()),
        ];
        let spans = render_segments(&segments, &values);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text.chars().count(), 510);
        let values = BTreeMap::from([("long".into(), "界".repeat(508))]);
        let spans = render_segments(&segments, &values);
        let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(plain.chars().count(), 512);
        assert!(plain.ends_with(" · t"));
        assert_eq!(
            render_segments(&[SidebarSegment::Template("x".repeat(700))], &values)[0]
                .text
                .chars()
                .count(),
            512
        );
    }

    #[test]
    fn sidebar_segment_rgb_is_strict_ascii_and_expands_short_values() {
        assert_eq!(segment_rgb("#3aF"), Some((51, 170, 255)));
        assert_eq!(segment_rgb("#012aEF"), Some((1, 42, 239)));
        for bad in [
            "",
            "red",
            "reset",
            "#12",
            "#1234",
            "#1234567",
            "#ff00gg",
            " #123",
            "#１２３",
            "#123\n",
            "\u{1b}[31m",
        ] {
            assert_eq!(segment_rgb(bad), None, "accepted {bad:?}");
        }
    }
}
