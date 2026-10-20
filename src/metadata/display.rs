//! One display projection for API snapshots and TUI rows; no semantic state changes.
use super::{Key, presentation};
use crate::api::agent;
use crate::i18n::{Lang, tr};
use crate::server::State;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Instant;

const MAX_TEMPLATE: usize = 512;
const MAX_OUTPUT: usize = 512;

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
    Some(PaneDisplay {
        title: with_fallback(&st.cfg.pane_template, &values, title),
        agent: effective_agent.map(str::to_string),
        state_label: state_label.to_string(),
        sidebar: with_fallback(&st.cfg.sidebar_pane_template, &values, title),
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
}
