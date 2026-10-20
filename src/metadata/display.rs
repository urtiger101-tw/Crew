//! One display projection for API snapshots and TUI rows; no semantic state changes.
use super::{Key, presentation};
use crate::api::agent;
use crate::i18n::{Lang, tr};
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
    pub agent: Option<String>,
    pub state_label: String,
    pub sidebar: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sidebar_spans: Vec<DisplaySpan>,
    pub kind_source: &'static str,
    pub presentation: presentation::Presentation,
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
    let native_name = effective_agent
        .or(p.label.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or(&w.name);
    let title = presentation.title.as_deref().unwrap_or(native_name);
    let status = agent::status(p, &st.cfg);
    let default_state = tr(
        lang,
        match status {
            crate::api::Status::Working => "render.state.working",
            crate::api::Status::Blocked => "render.state.blocked",
            crate::api::Status::Done => "render.state.done",
            crate::api::Status::Idle => "render.state.idle",
            crate::api::Status::Unknown => "render.state.unknown",
        },
    );
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
    Some(PaneDisplay {
        title: with_fallback(&st.cfg.pane_template, &values, title),
        agent: effective_agent.map(str::to_string),
        state_label: state_label.to_string(),
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
