//! Semantic UI colors and glyphs (docs/team/ui-ux/ui-spec.md §2).
//!
//! The default studio preset uses quiet truecolor chrome; terminal follows ANSI-16. State is
//! always shape + word; color only reinforces it. `mono` drops chrome colors (NO_COLOR,
//! TERM=dumb, preset "mono"); `ascii` swaps every glyph for an ASCII one.
use crate::api::Status;
use crate::semantic::ColorRole;
use crossterm::style::Color;
use serde::Deserialize;
use std::collections::BTreeMap;

#[cfg(test)]
mod aurora_tests;

/// `[theme]` table in config.toml.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ThemeCfg {
    /// studio (default) | aurora | terminal (ANSI-16) | dark | light | mono (no color)
    pub preset: String,
    /// auto | true | false
    pub ascii_glyphs: String,
    /// per-token override: 0-255 | "#rrggbb" | "reset"
    pub colors: BTreeMap<String, String>,
    /// Stable agent name -> pastel name, palette index, #rrggbb, or reset.
    pub agent_accents: BTreeMap<String, String>,
}

impl Default for ThemeCfg {
    fn default() -> Self {
        ThemeCfg {
            preset: "studio".into(),
            ascii_glyphs: "auto".into(),
            colors: BTreeMap::new(),
            agent_accents: BTreeMap::new(),
        }
    }
}

/// Color capability of an attaching terminal. Old clients retain truecolor behavior.
#[derive(serde::Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ColorDepth {
    Ansi16,
    Ansi256,
    #[default]
    TrueColor,
}

fn color_depth(term: Option<&str>, colorterm: Option<&str>, modern_terminal: bool) -> ColorDepth {
    if modern_terminal
        || colorterm
            .is_some_and(|c| c.eq_ignore_ascii_case("truecolor") || c.eq_ignore_ascii_case("24bit"))
    {
        ColorDepth::TrueColor
    } else if term.is_some_and(|t| t.contains("256color")) {
        ColorDepth::Ansi256
    } else {
        ColorDepth::Ansi16
    }
}

/// What the attaching client's environment asks for (sent with `Command::Attach`).
#[derive(serde::Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientEnv {
    /// NO_COLOR is set and non-empty
    pub no_color: bool,
    /// TERM=dumb
    pub dumb: bool,
    /// CREW_ASCII=1 or a non-UTF-8 locale
    pub ascii: bool,
    /// UI language detected from the client's locale (None = not recognised)
    #[serde(default)]
    pub lang: Option<crate::i18n::Lang>,
    /// Aurora chrome capability; absent on older clients means the legacy truecolor path.
    #[serde(default)]
    pub color_depth: ColorDepth,
    /// Optional client version for a nonfatal daemon/client version-drift notice.
    #[serde(default)]
    pub version: Option<[u16; 3]>,
}

impl ClientEnv {
    /// Read the current process environment (client side).
    pub fn detect() -> ClientEnv {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let locale = var("LC_ALL")
            .or_else(|| var("LC_CTYPE"))
            .or_else(|| var("LANG"));
        let non_utf8 = locale.is_some_and(|l| {
            let l = l.to_ascii_lowercase();
            !(l.contains("utf-8") || l.contains("utf8")) && l != "c.utf8"
        });
        let term = var("TERM");
        let colorterm = var("COLORTERM");
        let modern_terminal = var("WT_SESSION").is_some()
            || var("TERM_PROGRAM")
                .is_some_and(|p| matches!(p.as_str(), "WezTerm" | "vscode" | "iTerm.app"));
        let version = (|| {
            Some([
                env!("CARGO_PKG_VERSION_MAJOR").parse().ok()?,
                env!("CARGO_PKG_VERSION_MINOR").parse().ok()?,
                env!("CARGO_PKG_VERSION_PATCH").parse().ok()?,
            ])
        })();
        ClientEnv {
            no_color: var("NO_COLOR").is_some(),
            dumb: term.as_deref() == Some("dumb"),
            ascii: crate::brand::env_var("ASCII")
                .ok()
                .is_some_and(|v| v == "1")
                || (cfg!(unix) && non_utf8),
            lang: crate::i18n::Lang::detect(),
            color_depth: color_depth(term.as_deref(), colorterm.as_deref(), modern_terminal),
            version,
        }
    }
}

/// Glyph table; `ascii` swaps the whole table.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyphs {
    pub working: &'static str,
    pub blocked: &'static str,
    pub done: &'static str,
    pub idle: &'static str,
    pub unknown: &'static str,
    pub vline: &'static str,
    pub hline: &'static str,
    pub vline_active: &'static str,
    pub hline_active: &'static str,
    /// ├ ┤ ┬ ┴ ┼
    pub tee_right: &'static str,
    pub tee_left: &'static str,
    pub tee_down: &'static str,
    pub tee_up: &'static str,
    pub cross: &'static str,
    /// ┌ ┐ └ ┘ (two perpendicular arms)
    pub corner_tl: &'static str,
    pub corner_tr: &'static str,
    pub corner_bl: &'static str,
    pub corner_br: &'static str,
    pub group_open: &'static str,
    pub group_closed: &'static str,
    pub pointer: &'static str,
    pub ellipsis: &'static str,
    pub tree_mid: &'static str,
    pub tree_last: &'static str,
    pub ok: &'static str,
    pub err: &'static str,
    /// neutral notice glyph (info toasts, "cancelled"); `ok` is only for successes
    pub info: &'static str,
    /// separator inside titles / notices (`claude · blocked`)
    pub dot: &'static str,
    /// dash used in prose (`session X ended — now in Y`)
    pub dash: &'static str,
}

pub const UNICODE: Glyphs = Glyphs {
    working: "●",
    blocked: "▲",
    done: "◆",
    idle: "○",
    unknown: "?",
    vline: "│",
    hline: "─",
    vline_active: "┃",
    hline_active: "━",
    tee_right: "├",
    tee_left: "┤",
    tee_down: "┬",
    tee_up: "┴",
    cross: "┼",
    corner_tl: "┌",
    corner_tr: "┐",
    corner_bl: "└",
    corner_br: "┘",
    group_open: "▾",
    group_closed: "▸",
    pointer: "›",
    ellipsis: "…",
    tree_mid: "├",
    tree_last: "└",
    ok: "✓",
    err: "✗",
    info: "•",
    dot: "·",
    dash: "—",
};

pub const ASCII: Glyphs = Glyphs {
    working: "@",
    blocked: "!",
    done: "+",
    idle: "-",
    unknown: "?",
    vline: "|",
    hline: "-",
    vline_active: "#",
    hline_active: "=",
    tee_right: "+",
    tee_left: "+",
    tee_down: "+",
    tee_up: "+",
    cross: "+",
    corner_tl: "+",
    corner_tr: "+",
    corner_bl: "+",
    corner_br: "+",
    group_open: "v",
    group_closed: ">",
    pointer: ">",
    ellipsis: "~",
    tree_mid: "|",
    tree_last: "\\",
    ok: "+",
    err: "x",
    info: "*",
    dot: ".",
    dash: "-",
};

/// Resolved theme for one client frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Aurora is opt-in; existing presets keep their original rendering behavior.
    pub aurora: bool,
    /// no chrome colors: use BOLD / INVERSE / DIM instead
    pub mono: bool,
    pub ascii: bool,
    pub accent: Color,
    pub muted: Color,
    pub border: Color,
    pub border_active: Color,
    pub working: Color,
    pub blocked: Color,
    pub done: Color,
    pub idle: Color,
    pub unknown: Color,
    pub bar_bg: Color,
    pub bar_fg: Color,
    pub selection: Color,
    pub sidebar_bg: Color,
    pub sidebar_fg: Color,
    pub pane_bg: Color,
    pub pane_fg: Color,
    /// Low-emphasis toolbar keys and status chips.
    pub quiet_chrome: bool,
    pub warn: Color,
    /// Chrome-only surfaces; never used as defaults for child terminal cells.
    pub base_bg: Color,
    pub surface_bg: Color,
    pub raised_bg: Color,
    pub error: Color,
    /// Pink, peach, yellow, mint, sky, lavender, in stable identity-hash order.
    pub palette: [Color; 6],
    pub color_depth: ColorDepth,
    pub(crate) agent_accents: BTreeMap<String, Color>,
    pub g: Glyphs,
}

impl Theme {
    /// Default "terminal" preset (ANSI-16).
    pub fn terminal() -> Theme {
        let a = Color::AnsiValue;
        Theme {
            aurora: false,
            mono: false,
            ascii: false,
            accent: a(6),
            muted: a(8),
            border: a(8),
            border_active: a(6),
            working: a(3),
            blocked: a(1),
            done: a(2),
            idle: a(8),
            unknown: a(8),
            bar_bg: a(0),
            bar_fg: a(7),
            selection: a(6),
            sidebar_bg: Color::Reset,
            sidebar_fg: Color::Reset,
            pane_bg: Color::Reset,
            pane_fg: Color::Reset,
            quiet_chrome: false,
            warn: a(3),
            base_bg: a(0),
            surface_bg: a(0),
            raised_bg: a(8),
            error: a(1),
            palette: AURORA_PALETTE,
            color_depth: ColorDepth::TrueColor,
            agent_accents: BTreeMap::new(),
            g: UNICODE,
        }
    }

    /// Resolve config + client environment. Precedence: explicit preset > NO_COLOR/dumb -> mono.
    /// Presets "dark"/"light" and `colors` overrides are applied by `apply_preset`/`apply_overrides`.
    pub fn resolve(cfg: &ThemeCfg, env: &ClientEnv) -> Theme {
        let mut t = Theme::terminal();
        let explicit = !matches!(cfg.preset.as_str(), "terminal" | "studio" | "aurora");
        t.mono = cfg.preset == "mono" || (!explicit && (env.no_color || env.dumb));
        t.apply_preset(&cfg.preset);
        t.apply_overrides(&cfg.colors);
        if t.aurora {
            t.pane_fg = Color::Reset;
            t.pane_bg = Color::Reset;
        }
        t.color_depth = if t.aurora {
            env.color_depth
        } else {
            ColorDepth::TrueColor
        };
        const NAMES: [&str; 6] = ["pink", "peach", "yellow", "mint", "sky", "lavender"];
        t.agent_accents = cfg
            .agent_accents
            .iter()
            .filter_map(|(name, value)| {
                let color = NAMES
                    .iter()
                    .position(|accent| *accent == value.trim())
                    .map(|index| t.palette[index])
                    .or_else(|| parse_color(value))?;
                Some((name.clone(), color))
            })
            .collect();
        t.ascii = match cfg.ascii_glyphs.as_str() {
            "true" | "on" | "yes" => true,
            "false" | "off" | "no" => false,
            _ => env.ascii || env.dumb,
        };
        if t.ascii {
            t.g = ASCII;
        }
        t
    }

    /// Truecolor presets (ui-spec §2). "terminal" and "mono" keep the ANSI-16 defaults
    /// (mono is a flag set by `resolve`); unknown names are ignored.
    pub fn apply_preset(&mut self, preset: &str) {
        let Some(p) = PRESETS.iter().find(|(name, _)| *name == preset) else {
            return;
        };
        self.aurora = preset == "aurora";
        self.quiet_chrome = matches!(preset, "studio" | "aurora");
        for (token, value) in p.1 {
            if let (Some(slot), Some(c)) = (self.slot(token), parse_color(value)) {
                *slot = c;
            }
        }
    }

    /// Per-token overrides from `[theme.colors]`: token = Theme field name, value =
    /// 0-255 | "#rrggbb" | "reset". Unknown tokens and unparsable values are ignored.
    pub fn apply_overrides(&mut self, colors: &BTreeMap<String, String>) {
        for (token, value) in colors {
            if let (Some(slot), Some(c)) = (self.slot(token), parse_color(value)) {
                *slot = c;
            }
        }
    }

    /// The color field named by a token (ui-spec §2 names).
    fn slot(&mut self, token: &str) -> Option<&mut Color> {
        Some(match token {
            "accent" => &mut self.accent,
            "muted" => &mut self.muted,
            "border" => &mut self.border,
            "border_active" => &mut self.border_active,
            "working" => &mut self.working,
            "blocked" => &mut self.blocked,
            "done" => &mut self.done,
            "idle" => &mut self.idle,
            "unknown" => &mut self.unknown,
            "bar_bg" => &mut self.bar_bg,
            "bar_fg" => &mut self.bar_fg,
            "selection" => &mut self.selection,
            "sidebar_bg" => &mut self.sidebar_bg,
            "sidebar_fg" => &mut self.sidebar_fg,
            "pane_bg" => &mut self.pane_bg,
            "pane_fg" => &mut self.pane_fg,
            "warn" => &mut self.warn,
            "base_bg" => &mut self.base_bg,
            "surface_bg" => &mut self.surface_bg,
            "raised_bg" => &mut self.raised_bg,
            "error" => &mut self.error,
            _ => return None,
        })
    }

    /// (glyph, color, word) for a status. Chrome callers must still honor `mono`.
    pub fn status(&self, s: Status) -> (&'static str, Color, &'static str) {
        if self.aurora {
            let visual = crate::semantic::Presentation::from(s);
            return (
                visual.glyph(self.ascii),
                self.state_color(visual.color_role),
                visual.word,
            );
        }
        match s {
            Status::Working => (self.g.working, self.working, "working"),
            Status::Blocked => (self.g.blocked, self.blocked, "blocked"),
            Status::Done => (self.g.done, self.done, "done"),
            Status::Idle => (self.g.idle, self.idle, "idle"),
            Status::Unknown => (self.g.unknown, self.unknown, "unknown"),
        }
    }

    /// Semantic chrome color, already adjusted for this client's mono/color capability.
    pub fn state_color(&self, role: ColorRole) -> Color {
        self.c(match role {
            ColorRole::Working => self.working,
            ColorRole::Blocked => self.blocked,
            ColorRole::Done => self.done,
            ColorRole::Muted => self.muted,
            ColorRole::Error => self.error,
        })
    }

    /// Stable name accent, already adjusted for this client. FNV-1a fixes the mapping
    /// across processes, restarts and Rust versions; pane IDs/titles do not enter the hash.
    pub fn agent_accent(&self, name: &str) -> Color {
        let hash = name
            .as_bytes()
            .iter()
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            });
        self.c(self
            .agent_accents
            .get(name)
            .copied()
            .unwrap_or(self.palette[(hash % 6) as usize]))
    }

    /// `c`, or `Color::Reset` in mono mode. Use for every chrome color.
    pub fn c(&self, c: Color) -> Color {
        if self.mono || c == Color::Reset {
            return Color::Reset;
        }
        if self.aurora && self.color_depth == ColorDepth::Ansi16 {
            const FALLBACK: [Color; 6] = [
                Color::Magenta,
                Color::Yellow,
                Color::DarkYellow,
                Color::Green,
                Color::Cyan,
                Color::Blue,
            ];
            if let Some(index) = self.palette.iter().position(|color| *color == c) {
                return FALLBACK[index];
            }
            if c == self.border || c == self.muted {
                return Color::DarkGrey;
            }
        }
        downgrade(c, self.color_depth)
    }
}

const AURORA_PALETTE: [Color; 6] = [
    Color::Rgb {
        r: 0xf2,
        g: 0xa7,
        b: 0xc3,
    },
    Color::Rgb {
        r: 0xf5,
        g: 0xc0,
        b: 0x8a,
    },
    Color::Rgb {
        r: 0xe8,
        g: 0xd2,
        b: 0x7a,
    },
    Color::Rgb {
        r: 0x8f,
        g: 0xd9,
        b: 0xa8,
    },
    Color::Rgb {
        r: 0x8a,
        g: 0xb4,
        b: 0xf8,
    },
    Color::Rgb {
        r: 0xb5,
        g: 0x9c,
        b: 0xf2,
    },
];

const ANSI16_RGB: [[u8; 3]; 16] = [
    [0, 0, 0],
    [128, 0, 0],
    [0, 128, 0],
    [128, 128, 0],
    [0, 0, 128],
    [128, 0, 128],
    [0, 128, 128],
    [192, 192, 192],
    [128, 128, 128],
    [255, 0, 0],
    [0, 255, 0],
    [255, 255, 0],
    [0, 0, 255],
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 255],
];

fn palette_rgb(index: u8) -> [u8; 3] {
    match index {
        0..=15 => ANSI16_RGB[index as usize],
        16..=231 => {
            const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let i = usize::from(index - 16);
            [CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6]]
        }
        232..=255 => [8 + (index - 232) * 10; 3],
    }
}

fn downgrade(color: Color, depth: ColorDepth) -> Color {
    if depth == ColorDepth::TrueColor {
        return color;
    }
    let rgb = match color {
        Color::Rgb { r, g, b } => [r, g, b],
        Color::AnsiValue(index) if depth == ColorDepth::Ansi16 => palette_rgb(index),
        _ => return color,
    };
    let range = if depth == ColorDepth::Ansi16 {
        0..=15u8
    } else {
        16..=255u8
    };
    let mut closest = 0;
    let mut distance = u32::MAX;
    for index in range {
        let candidate = palette_rgb(index);
        let candidate_distance = rgb
            .iter()
            .zip(candidate)
            .map(|(a, b)| (i32::from(*a) - i32::from(b)).unsigned_abs().pow(2))
            .sum();
        if candidate_distance < distance {
            closest = index;
            distance = candidate_distance;
        }
    }
    if depth == ColorDepth::Ansi256 {
        Color::AnsiValue(closest)
    } else {
        [
            Color::Black,
            Color::DarkRed,
            Color::DarkGreen,
            Color::DarkYellow,
            Color::DarkBlue,
            Color::DarkMagenta,
            Color::DarkCyan,
            Color::Grey,
            Color::DarkGrey,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::White,
        ][closest as usize]
    }
}

/// Token values of the truecolor presets (ui-spec §2 "truecolor dark / light").
/// `border_active` and `selection` equal `accent`; `idle`/`unknown` equal `muted`;
/// `warn` equals `working`.
const PRESETS: &[(&str, &[(&str, &str)])] = &[
    (
        "aurora",
        &[
            ("base_bg", "#111214"),
            ("surface_bg", "#17181b"),
            ("raised_bg", "#1d1f23"),
            ("accent", "#b59cf2"),
            ("muted", "#6b7079"),
            ("border", "#2a2c31"),
            ("border_active", "#b59cf2"),
            ("working", "#8ab4f8"),
            ("blocked", "#f5c08a"),
            ("done", "#8fd9a8"),
            ("error", "#f2a7c3"),
            ("idle", "#6b7079"),
            ("unknown", "#6b7079"),
            ("bar_bg", "#111214"),
            ("bar_fg", "#c9ccd3"),
            ("sidebar_bg", "#17181b"),
            ("sidebar_fg", "#c9ccd3"),
            ("pane_bg", "reset"),
            ("pane_fg", "reset"),
            ("selection", "#b59cf2"),
            ("warn", "#f5c08a"),
        ],
    ),
    (
        "studio",
        &[
            ("accent", "#a7bdcc"),
            ("muted", "#a4a8b1"),
            ("border", "#60646e"),
            ("border_active", "#a7bdcc"),
            ("working", "#e5c07b"),
            ("blocked", "#e89aa1"),
            ("done", "#a9c997"),
            ("idle", "#a4a8b1"),
            ("unknown", "#a4a8b1"),
            ("bar_bg", "#1b1d22"),
            ("bar_fg", "#d0d2d7"),
            ("sidebar_bg", "#1b1d22"),
            ("sidebar_fg", "#d0d2d7"),
            ("pane_bg", "#14161b"),
            ("pane_fg", "#d0d2d7"),
            ("selection", "#a7bdcc"),
            ("warn", "#e5c07b"),
        ],
    ),
    (
        "dark",
        &[
            ("accent", "#56b6c2"),
            ("muted", "#7f8796"),
            ("border", "#6b7280"),
            ("border_active", "#56b6c2"),
            ("working", "#e5c07b"),
            ("blocked", "#e06c75"),
            ("done", "#98c379"),
            ("idle", "#7f8796"),
            ("unknown", "#7f8796"),
            ("bar_bg", "#1f2430"),
            ("bar_fg", "#c8ccd4"),
            ("selection", "#56b6c2"),
            ("warn", "#e5c07b"),
        ],
    ),
    (
        "light",
        &[
            ("accent", "#0e7490"),
            ("muted", "#6b7280"),
            ("border", "#6b7280"),
            ("border_active", "#0e7490"),
            ("working", "#a16207"),
            ("blocked", "#b91c1c"),
            ("done", "#15803d"),
            ("idle", "#6b7280"),
            ("unknown", "#6b7280"),
            ("bar_bg", "#e5e7eb"),
            ("bar_fg", "#1f2937"),
            ("selection", "#0e7490"),
            ("warn", "#a16207"),
        ],
    ),
];

/// Parse a theme color value: "reset", a 0-255 palette index, or "#rrggbb".
pub fn parse_color(value: &str) -> Option<Color> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("reset") {
        return Some(Color::Reset);
    }
    if let Some(hex) = v.strip_prefix('#') {
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return Some(Color::Rgb {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
        });
    }
    v.parse::<u8>().ok().map(Color::AnsiValue)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn studio_default_is_quiet_and_respects_accessibility_environment() {
        let cfg = ThemeCfg::default();
        assert_eq!(cfg.preset, "studio");
        let t = Theme::resolve(&cfg, &ClientEnv::default());
        assert!(t.quiet_chrome);
        assert_ne!(t.g.vline_active, t.g.vline);
        assert_ne!(t.g.hline_active, t.g.hline);
        assert_eq!(t.sidebar_bg, parse_color("#1b1d22").unwrap());
        assert_eq!(t.sidebar_fg, parse_color("#d0d2d7").unwrap());
        let mono = Theme::resolve(
            &cfg,
            &ClientEnv {
                no_color: true,
                ..Default::default()
            },
        );
        assert!(mono.mono);
        assert_eq!(mono.c(mono.sidebar_bg), Color::Reset);
        assert_ne!(mono.g.vline_active, mono.g.vline);
        let ascii = Theme::resolve(
            &cfg,
            &ClientEnv {
                ascii: true,
                ..Default::default()
            },
        );
        assert_eq!(ascii.g.vline_active, "#");
    }

    #[test]
    fn parse_color_accepts_index_hex_and_reset_only() {
        assert_eq!(parse_color("3"), Some(Color::AnsiValue(3)));
        assert_eq!(parse_color("255"), Some(Color::AnsiValue(255)));
        assert_eq!(parse_color("256"), None);
        assert_eq!(
            parse_color("#e5c07b"),
            Some(Color::Rgb {
                r: 0xe5,
                g: 0xc0,
                b: 0x7b
            })
        );
        assert_eq!(parse_color("#E5C07"), None);
        assert_eq!(parse_color("#zz0000"), None);
        assert_eq!(parse_color("reset"), Some(Color::Reset));
        assert_eq!(parse_color("Reset"), Some(Color::Reset));
        assert_eq!(parse_color("red"), None);
    }

    #[test]
    fn dark_and_light_presets_set_truecolor_tokens() {
        let mut t = Theme::terminal();
        t.apply_preset("dark");
        assert_eq!(
            t.accent,
            Color::Rgb {
                r: 0x56,
                g: 0xb6,
                b: 0xc2
            }
        );
        assert_eq!(t.border_active, t.accent);
        assert_eq!(t.idle, t.muted);
        let mut l = Theme::terminal();
        l.apply_preset("light");
        assert_eq!(
            l.bar_bg,
            Color::Rgb {
                r: 0xe5,
                g: 0xe7,
                b: 0xeb
            }
        );
        assert_ne!(l.accent, t.accent);
    }

    #[test]
    fn terminal_and_unknown_presets_keep_ansi16() {
        let base = Theme::terminal();
        let mut t = Theme::terminal();
        t.apply_preset("terminal");
        t.apply_preset("nope");
        assert_eq!(t, base);
        assert_eq!(base.accent, Color::AnsiValue(6));
        assert_eq!(base.border, Color::AnsiValue(8));
    }

    #[test]
    fn overrides_apply_by_field_name_and_ignore_bad_input() {
        let mut t = Theme::terminal();
        t.apply_overrides(&map(&[
            ("border_active", "#e5c07b"),
            ("accent", "74"),
            ("muted", "reset"),
            ("no_such_token", "1"),
            ("warn", "999"),
            ("blocked", "#12"),
        ]));
        assert_eq!(
            t.border_active,
            Color::Rgb {
                r: 0xe5,
                g: 0xc0,
                b: 0x7b
            }
        );
        assert_eq!(t.accent, Color::AnsiValue(74));
        assert_eq!(t.muted, Color::Reset);
        assert_eq!(t.warn, Color::AnsiValue(3), "out-of-range value ignored");
        assert_eq!(t.blocked, Color::AnsiValue(1), "bad hex ignored");
    }

    #[test]
    fn resolve_applies_preset_then_overrides() {
        let cfg = ThemeCfg {
            preset: "dark".into(),
            ascii_glyphs: "auto".into(),
            colors: map(&[("accent", "#000001")]),
            ..ThemeCfg::default()
        };
        let t = Theme::resolve(&cfg, &ClientEnv::default());
        assert_eq!(t.accent, Color::Rgb { r: 0, g: 0, b: 1 });
        assert_eq!(
            t.blocked,
            Color::Rgb {
                r: 0xe0,
                g: 0x6c,
                b: 0x75
            }
        );
    }

    #[test]
    fn resolve_mono_and_ascii() {
        let cfg = ThemeCfg::default();
        let plain = Theme::resolve(&cfg, &ClientEnv::default());
        assert!(!plain.mono && !plain.ascii);
        assert_eq!(plain.status(Status::Blocked).0, "▲");
        let nc = Theme::resolve(
            &cfg,
            &ClientEnv {
                no_color: true,
                ..Default::default()
            },
        );
        assert!(nc.mono && !nc.ascii, "NO_COLOR keeps unicode shapes");
        assert_eq!(nc.c(nc.accent), Color::Reset);
        let dumb = Theme::resolve(
            &cfg,
            &ClientEnv {
                dumb: true,
                ..Default::default()
            },
        );
        assert!(dumb.mono && dumb.ascii);
        assert_eq!(dumb.status(Status::Working).0, "@");
        let forced = ThemeCfg {
            preset: "dark".into(),
            ..ThemeCfg::default()
        };
        assert!(
            !Theme::resolve(
                &forced,
                &ClientEnv {
                    no_color: true,
                    ..Default::default()
                }
            )
            .mono
        );
    }
}
