use crate::commands::{DEFAULT_BINDINGS, key_name, normalize_key, parse};
use crossterm::event::KeyEvent;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

/// Most toolbar buttons (one per function key, F1..F12).
pub const TOOLBAR_MAX_BUTTONS: usize = 12;
/// Widest toolbar label, in display columns.
const TOOLBAR_LABEL_MAX: usize = 24;

/// One `[[toolbar_buttons]]` entry: the label drawn after its F-key chip and the command line
/// a click (or the F-key) runs.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolbarButton {
    pub label: String,
    pub command: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    /// Prefix key, e.g. "C-b", "C-a"
    pub prefix: String,
    /// Shell to launch in new panes (default: $SHELL / powershell)
    pub shell: Option<String>,
    pub scrollback: usize,
    pub mouse: bool,
    /// Show herdr-like sidebar with pane/agent status
    pub sidebar: bool,
    pub sidebar_width: u16,
    /// Seconds after last output during which a pane is considered "busy"
    pub busy_secs: f32,
    /// Name windows after the foreground process (unless renamed manually)
    pub automatic_rename: bool,
    /// Copy selections to the system clipboard via OSC 52
    pub osc52: bool,
    /// Extra key bindings (after the prefix): key -> command
    pub bind: BTreeMap<String, String>,
    /// Keys to remove from the default bindings ("all" removes every default)
    pub unbind: Vec<String>,
    /// Commands run after the config is loaded
    pub commands: Vec<String>,
    /// Save the layout to a state file and restore it when the server restarts
    pub persist: bool,
    /// Show key hints (status-bar help, which-key bar, first-attach tip)
    pub hints: bool,
    /// Ask y/n before interactive kill-pane / kill-window / chooser kill
    pub confirm_close: bool,
    /// Pane title rows: off | auto | always
    pub pane_titles: String,
    /// Literal display templates; $name / ${name} substitute metadata or built-in fields.
    pub pane_template: String,
    pub sidebar_pane_template: String,
    /// Optional single-line pane label segments; empty values use the legacy template.
    pub sidebar_pane_segments: Vec<crate::metadata::display::SidebarSegment>,
    pub sidebar_workspace_template: String,
    /// Infer kind from live pane processes, without claiming a semantic lifecycle authority.
    pub agent_detection: bool,
    /// Colors and glyphs (`[theme]` table)
    pub theme: crate::theme::ThemeCfg,
    /// UI language: auto (from each client's locale) | en | zh-TW
    pub lang: String,
    /// Show the function-key toolbar on the top row (needs at least 4 rows)
    pub toolbar: bool,
    /// F1..F12 trigger toolbar buttons directly; when off only `prefix` then `F<n>` does
    pub toolbar_fkeys: bool,
    /// Custom toolbar buttons in F-key order (at most 12); empty = the default buttons
    pub toolbar_buttons: Vec<ToolbarButton>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prefix: "C-b".into(),
            shell: None,
            scrollback: 10000,
            mouse: true,
            sidebar: false,
            sidebar_width: 28,
            busy_secs: 1.5,
            automatic_rename: true,
            osc52: true,
            bind: BTreeMap::new(),
            unbind: vec![],
            commands: vec![],
            persist: true,
            hints: true,
            confirm_close: true,
            pane_titles: "auto".into(),
            pane_template: String::new(),
            sidebar_pane_template: String::new(),
            sidebar_pane_segments: Vec::new(),
            sidebar_workspace_template: String::new(),
            agent_detection: true,
            theme: crate::theme::ThemeCfg::default(),
            lang: "auto".into(),
            toolbar: true,
            toolbar_fkeys: false,
            toolbar_buttons: Vec::new(),
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    if let Ok(p) = crate::brand::env_var("CONFIG") {
        return Some(PathBuf::from(p));
    }
    dirs::config_dir().map(|d| crate::brand::compatible_file(&d, "config.toml"))
}

/// Control, bidi formatting, directional mark, line/paragraph separator, BOM and interlinear
/// annotation characters; never allowed in a label.
fn unsafe_label_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200e}'
                | '\u{200f}'
                | '\u{061c}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
        )
}

/// Check `[[toolbar_buttons]]`: at most [`TOOLBAR_MAX_BUTTONS`], every command parses, every
/// label is non-blank, at most `TOOLBAR_LABEL_MAX` columns and has no control or bidi
/// characters.
pub fn validate_toolbar(buttons: &[ToolbarButton]) -> Result<(), String> {
    if buttons.len() > TOOLBAR_MAX_BUTTONS {
        return Err(format!(
            "toolbar_buttons: at most {TOOLBAR_MAX_BUTTONS} buttons (F1..F12), got {}",
            buttons.len()
        ));
    }
    for (i, b) in buttons.iter().enumerate() {
        let n = i + 1;
        let label = &b.label;
        // width 0 catches labels made only of zero-width characters, which `trim` keeps
        if label.trim().is_empty()
            || label.chars().any(unsafe_label_char)
            || !(1..=TOOLBAR_LABEL_MAX).contains(&label.width())
        {
            return Err(format!(
                "toolbar_buttons F{n}: label must be 1-{TOOLBAR_LABEL_MAX} columns without control or bidi characters"
            ));
        }
        parse(&b.command).map_err(|e| format!("toolbar_buttons F{n}: command: {e}"))?;
    }
    Ok(())
}

fn parse_bool(v: &str) -> Result<bool, String> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(format!("expected on/off, got {v}")),
    }
}

impl Config {
    pub fn load() -> Config {
        match config_path().filter(|p| p.exists()) {
            Some(p) => Config::load_from(&p).unwrap_or_else(|e| {
                eprintln!("crew: {e}");
                Config::default()
            }),
            None => Config::default(),
        }
    }

    pub fn load_from(p: &std::path::Path) -> Result<Config, String> {
        let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let config: Config = toml::from_str(&s).map_err(|e| format!("config error: {e}"))?;
        for template in [
            &config.pane_template,
            &config.sidebar_pane_template,
            &config.sidebar_workspace_template,
        ] {
            crate::metadata::display::validate_template(template)?;
        }
        crate::metadata::display::validate_segments(&config.sidebar_pane_segments)?;
        validate_toolbar(&config.toolbar_buttons)?;
        Ok(config)
    }

    pub fn default_shell(&self) -> String {
        if let Some(s) = &self.shell {
            return s.clone();
        }
        if cfg!(windows) {
            "powershell.exe".into()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
        }
    }

    pub fn prefix_name(&self) -> String {
        normalize_key(&self.prefix).unwrap_or_else(|_| "C-b".into())
    }

    pub fn is_prefix(&self, k: &KeyEvent) -> bool {
        key_name(k).is_some_and(|n| n == self.prefix_name())
    }

    /// Build the effective key table: defaults - unbind + bind.
    pub fn bindings(&self) -> (BTreeMap<String, String>, Vec<String>) {
        let mut errs = vec![];
        let mut m: BTreeMap<String, String> = BTreeMap::new();
        if !self.unbind.iter().any(|u| u == "all") {
            for (k, c) in DEFAULT_BINDINGS {
                m.insert(k.to_string(), c.to_string());
            }
        }
        for u in &self.unbind {
            if let Ok(k) = normalize_key(u) {
                m.remove(&k);
            }
        }
        for (k, c) in &self.bind {
            match (normalize_key(k), parse(c)) {
                (Ok(k), Ok(_)) => {
                    m.insert(k, c.clone());
                }
                (Err(e), _) | (_, Err(e)) => errs.push(format!("bind {k}: {e}")),
            }
        }
        (m, errs)
    }

    /// `set OPTION VALUE`
    pub fn set(&mut self, key: &str, val: &str) -> Result<(), String> {
        let num = |v: &str| {
            v.parse::<f64>()
                .map_err(|_| format!("expected number, got {v}"))
        };
        match key.replace('_', "-").as_str() {
            "prefix" => {
                normalize_key(val)?;
                self.prefix = val.to_string()
            }
            "shell" | "default-shell" | "default-command" => self.shell = Some(val.to_string()),
            "mouse" => self.mouse = parse_bool(val)?,
            "sidebar" => self.sidebar = parse_bool(val)?,
            "sidebar-width" => self.sidebar_width = num(val)?.clamp(10.0, 100.0) as u16,
            "agent-detection" => self.agent_detection = parse_bool(val)?,
            "pane-template" | "sidebar-pane-template" | "sidebar-workspace-template" => {
                crate::metadata::display::validate_template(val)?;
                match key.replace('_', "-").as_str() {
                    "pane-template" => self.pane_template = val.to_string(),
                    "sidebar-pane-template" => self.sidebar_pane_template = val.to_string(),
                    _ => self.sidebar_workspace_template = val.to_string(),
                }
            }
            "scrollback" | "history-limit" => self.scrollback = num(val)? as usize,
            "busy-secs" => self.busy_secs = num(val)? as f32,
            "automatic-rename" => self.automatic_rename = parse_bool(val)?,
            "osc52" | "set-clipboard" => self.osc52 = parse_bool(val)?,
            "persist" => self.persist = parse_bool(val)?,
            "hints" => self.hints = parse_bool(val)?,
            "lang" | "language" => match val {
                "auto" => self.lang = "auto".into(),
                v => match crate::i18n::Lang::from_locale(v) {
                    Some(l) => self.lang = l.code().into(),
                    None => return Err(format!("lang: expected auto|en|zh-TW, got {v}")),
                },
            },
            "confirm-close" => self.confirm_close = parse_bool(val)?,
            "toolbar" => self.toolbar = parse_bool(val)?,
            "toolbar-fkeys" => self.toolbar_fkeys = parse_bool(val)?,
            "pane-titles" => match val {
                "off" | "auto" | "always" => self.pane_titles = val.to_string(),
                _ => return Err(format!("pane-titles: expected off|auto|always, got {val}")),
            },
            "theme" | "theme-preset" => match val {
                "terminal" | "dark" | "light" | "mono" => self.theme.preset = val.to_string(),
                _ => {
                    return Err(format!(
                        "theme: expected terminal|dark|light|mono, got {val}"
                    ));
                }
            },
            "ascii-glyphs" => match val {
                "auto" | "true" | "false" | "on" | "off" => {
                    self.theme.ascii_glyphs = val.to_string()
                }
                _ => return Err(format!("ascii-glyphs: expected auto|on|off, got {val}")),
            },
            other => return Err(format!("unknown option: {other}")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_text(text: &str) -> Result<Config, String> {
        use std::io::Write;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "crew-sidebar-config-{}-{stamp}.toml",
            std::process::id()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
        drop(file);
        let result = Config::load_from(&path);
        std::fs::remove_file(path).unwrap();
        result
    }

    #[test]
    fn sidebar_segments_config_validates_before_use_and_preserves_legacy_templates() {
        let config = load_text(
            r##"
sidebar_pane_template = "legacy:$title"
sidebar_pane_segments = ["$workspace", {template="$task", fg="#abc", bold=false, dim=true}]
"##,
        )
        .unwrap();
        assert_eq!(config.sidebar_pane_template, "legacy:$title");
        assert_eq!(config.sidebar_pane_segments.len(), 2);
        assert!(
            load_text("sidebar_pane_template='old'")
                .unwrap()
                .sidebar_pane_segments
                .is_empty()
        );
        for input in [
            r##"sidebar_pane_segments = [{template="$task", fg="red"}]"##.to_string(),
            r##"sidebar_pane_segments = [{template="$task", bould=true}]"##.to_string(),
            r##"sidebar_pane_segments = [{template="$task", bold="false"}]"##.to_string(),
            "sidebar_pane_segments = [\"\\u001b[31m\"]".into(),
            "sidebar_pane_segments = [\"\\u202e$task\"]".into(),
            format!("sidebar_pane_segments = [{}]", vec!["'x'"; 17].join(",")),
            format!("sidebar_pane_segments = ['{}']", "x".repeat(513)),
        ] {
            assert!(
                load_text(&input).is_err(),
                "accepted invalid config {input}"
            );
        }
    }

    #[test]
    fn toolbar_defaults_parse_and_set() {
        let c = Config::default();
        assert!(c.toolbar && !c.toolbar_fkeys && c.toolbar_buttons.is_empty());
        let c = load_text(
            "toolbar = false\ntoolbar_fkeys = true\n\
             [[toolbar_buttons]]\nlabel = \"build\"\ncommand = \"display built\"\n\
             [[toolbar_buttons]]\nlabel = \"關閉\"\ncommand = \"kill-pane\"\n",
        )
        .unwrap();
        assert!(!c.toolbar && c.toolbar_fkeys);
        assert_eq!(
            c.toolbar_buttons,
            vec![
                ToolbarButton {
                    label: "build".into(),
                    command: "display built".into()
                },
                ToolbarButton {
                    label: "關閉".into(),
                    command: "kill-pane".into()
                },
            ]
        );
        let mut c = Config::default();
        c.set("toolbar", "off").unwrap();
        c.set("toolbar_fkeys", "on").unwrap();
        assert!(!c.toolbar && c.toolbar_fkeys);
        c.set("toolbar", "on").unwrap();
        assert!(c.toolbar);
        assert!(c.set("toolbar", "maybe").is_err());
        assert!(c.set("toolbar-fkeys", "2").is_err());
    }

    #[test]
    fn toolbar_buttons_are_validated_at_load() {
        // TOML basic string with every non-ASCII or control char as a \uXXXX escape, so the
        // invalid labels reach the validator instead of failing the TOML parse
        let quote = |s: &str| {
            let body: String = s
                .chars()
                .map(|c| match c {
                    '"' | '\\' => format!("\\{c}"),
                    ' '..='~' => c.to_string(),
                    _ => format!("\\u{:04X}", c as u32),
                })
                .collect();
            format!("\"{body}\"")
        };
        let button = |label: &str, command: &str| {
            format!(
                "[[toolbar_buttons]]\nlabel = {}\ncommand = {}\n",
                quote(label),
                quote(command)
            )
        };
        assert!(
            toml::from_str::<Config>(&button("a\u{1b}[31m", "new-window")).is_ok(),
            "the escape helper produces valid TOML"
        );
        assert!(load_text(&button(&"x".repeat(24), "new-window")).is_ok());
        assert!(load_text(&button("中文標籤", "split-window -h")).is_ok());
        for bad in [
            button("", "new-window"),
            button("   ", "new-window"),
            button(&"x".repeat(25), "new-window"),
            button(&"中".repeat(13), "new-window"),
            button("a\u{1b}[31m", "new-window"),
            button("\u{202e}abc", "new-window"),
            button("a\u{200f}", "new-window"),
            button("\u{200b}", "new-window"),
            button("a\u{2028}b", "new-window"),
            button("\u{feff}ok", "new-window"),
            button("ok", "no-such-command"),
            button("ok", ""),
            "[[toolbar_buttons]]\nlabel = \"ok\"\n".into(),
            "[[toolbar_buttons]]\nlabel = \"ok\"\ncommand = \"new-window\"\nkey = \"F1\"\n".into(),
            button("ok", "new-window").repeat(13),
        ] {
            assert!(load_text(&bad).is_err(), "accepted invalid toolbar {bad:?}");
        }
        assert!(load_text(&button("ok", "new-window").repeat(12)).is_ok());
    }

    #[test]
    fn parse_config() {
        let c: Config = toml::from_str(
            "prefix = \"C-a\"\nsidebar = true\nunbind = [\"x\"]\n[bind]\n\"C-k\" = \"kill-pane\"\nv = \"split-window -h\"\n",
        )
        .unwrap();
        assert!(c.sidebar);
        assert_eq!(c.prefix_name(), "C-a");
        assert_eq!(c.scrollback, 10000);
        let (b, errs) = c.bindings();
        assert!(errs.is_empty());
        assert!(!b.contains_key("x"));
        assert_eq!(b["C-k"], "kill-pane");
        assert_eq!(b["v"], "split-window -h");
        assert_eq!(b["c"], "new-window");
        let mut c2: Config = toml::from_str("unbind = [\"all\"]\n[bind]\nq = \"nope\"").unwrap();
        let (b2, e2) = c2.bindings();
        assert!(b2.is_empty());
        assert_eq!(e2.len(), 1);
        c2.set("mouse", "off").unwrap();
        assert!(!c2.mouse);
        c2.set("history-limit", "500").unwrap();
        assert_eq!(c2.scrollback, 500);
        assert!(c2.set("prefix", "C-").is_err() || c2.prefix == "C-");
        assert!(c2.set("bogus", "1").is_err());
    }
}
