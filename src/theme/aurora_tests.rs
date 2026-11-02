use super::*;
use crate::semantic::{ColorRole, Presentation};

fn config() -> ThemeCfg {
    ThemeCfg {
        preset: "aurora".into(),
        ..ThemeCfg::default()
    }
}

#[test]
fn aurora_is_opt_in_and_does_not_color_child_defaults() {
    let studio = Theme::resolve(&ThemeCfg::default(), &ClientEnv::default());
    assert_eq!(ThemeCfg::default().preset, "studio");
    assert!(!studio.aurora);
    assert_eq!(studio.pane_bg, parse_color("#14161b").unwrap());
    let mut cfg = config();
    cfg.colors.insert("pane_fg".into(), "#010203".into());
    cfg.colors.insert("pane_bg".into(), "#040506".into());
    let t = Theme::resolve(&cfg, &ClientEnv::default());
    assert!(t.aurora && t.quiet_chrome);
    assert_eq!(t.base_bg, parse_color("#111214").unwrap());
    assert_eq!(t.surface_bg, parse_color("#17181b").unwrap());
    assert_eq!(t.raised_bg, parse_color("#1d1f23").unwrap());
    assert_eq!(t.pane_fg, Color::Reset);
    assert_eq!(t.pane_bg, Color::Reset);
    let mut runtime = crate::config::Config::default();
    runtime.set("theme", "aurora").unwrap();
    assert_eq!(runtime.theme.preset, "aurora");
}

#[test]
fn aurora_respects_mono_and_ascii_without_changing_legacy_presets() {
    for env in [
        ClientEnv {
            no_color: true,
            ..Default::default()
        },
        ClientEnv {
            dumb: true,
            ..Default::default()
        },
    ] {
        let t = Theme::resolve(&config(), &env);
        assert!(t.mono);
        for color in t
            .palette
            .into_iter()
            .chain([t.base_bg, t.surface_bg, t.raised_bg, t.error])
        {
            assert_eq!(t.c(color), Color::Reset);
        }
        assert_eq!(t.agent_accent("codex"), Color::Reset);
        assert_eq!(t.state_color(ColorRole::Error), Color::Reset);
    }
    let env = ClientEnv {
        ascii: true,
        color_depth: ColorDepth::Ansi16,
        ..Default::default()
    };
    let t = Theme::resolve(&config(), &env);
    for status in [
        Status::Working,
        Status::Blocked,
        Status::Done,
        Status::Idle,
        Status::Unknown,
    ] {
        assert!(t.status(status).0.is_ascii());
        assert_eq!(t.status(status).0, Presentation::from(status).glyph(true));
    }
    let studio = Theme::resolve(&ThemeCfg::default(), &env);
    assert_eq!(studio.c(studio.accent), studio.accent);
    let dark = Theme::resolve(
        &ThemeCfg {
            preset: "dark".into(),
            ..Default::default()
        },
        &ClientEnv {
            no_color: true,
            ..Default::default()
        },
    );
    assert!(
        !dark.mono,
        "legacy explicit dark preset behavior is retained"
    );
}

#[test]
fn aurora_capability_fallbacks_are_bounded_idempotent_and_distinct() {
    let indexed = Theme::resolve(
        &config(),
        &ClientEnv {
            color_depth: ColorDepth::Ansi256,
            ..Default::default()
        },
    );
    for color in indexed.palette.into_iter().chain([
        indexed.base_bg,
        indexed.surface_bg,
        indexed.raised_bg,
        indexed.muted,
        indexed.bar_fg,
    ]) {
        let converted = indexed.c(color);
        assert!(matches!(converted, Color::AnsiValue(16..=255)));
        assert_eq!(indexed.c(converted), converted);
    }
    let classic = Theme::resolve(
        &config(),
        &ClientEnv {
            color_depth: ColorDepth::Ansi16,
            ..Default::default()
        },
    );
    let accents: Vec<_> = classic.palette.iter().map(|c| classic.c(*c)).collect();
    for (index, color) in accents.iter().enumerate() {
        assert!(!matches!(color, Color::Rgb { .. } | Color::AnsiValue(_)));
        assert!(
            !accents[..index].contains(color),
            "identity colors stay distinct in ANSI-16"
        );
        assert_eq!(classic.c(*color), *color);
    }
    assert_eq!(classic.c(Color::Reset), Color::Reset);
    assert_eq!(classic.c(classic.border), Color::DarkGrey);
    assert!(!matches!(
        classic.c(Color::AnsiValue(74)),
        Color::AnsiValue(_)
    ));
    assert_eq!(classic.state_color(ColorRole::Working), Color::Cyan);
    assert_eq!(classic.state_color(ColorRole::Blocked), Color::Yellow);
    assert_eq!(classic.state_color(ColorRole::Done), Color::Green);
    assert_eq!(classic.state_color(ColorRole::Error), Color::Magenta);
}

#[test]
fn client_capabilities_are_per_client_and_legacy_wire_defaults_are_preserved() {
    assert_eq!(
        color_depth(Some("xterm-256color"), None, false),
        ColorDepth::Ansi256
    );
    assert_eq!(
        color_depth(Some("vt100"), Some("TRUECOLOR"), false),
        ColorDepth::TrueColor
    );
    assert_eq!(color_depth(None, None, true), ColorDepth::TrueColor);
    assert_eq!(color_depth(Some("vt100"), None, false), ColorDepth::Ansi16);
    let legacy: ClientEnv = serde_json::from_value(
        serde_json::json!({"no_color": false, "dumb": false, "ascii": false}),
    )
    .unwrap();
    assert_eq!(legacy.color_depth, ColorDepth::TrueColor);
    assert_eq!(legacy.version, None);
    let modern = ClientEnv {
        color_depth: ColorDepth::Ansi256,
        version: Some([0, 6, 0]),
        ..Default::default()
    };
    let roundtrip: ClientEnv =
        serde_json::from_value(serde_json::to_value(modern).unwrap()).unwrap();
    assert_eq!(roundtrip.color_depth, ColorDepth::Ansi256);
    assert_eq!(roundtrip.version, Some([0, 6, 0]));
    assert!(ClientEnv::detect().version.is_some());
}

#[test]
fn identity_hash_is_stable_and_overrides_are_resolved_once() {
    let cfg: ThemeCfg = toml::from_str("preset = 'aurora'\n[agent_accents]\nclaude = 'mint'\ncodex = '#010203'\nsol = '74'\ninvalid = 'invalid color'\n").unwrap();
    let t = Theme::resolve(&cfg, &ClientEnv::default());
    assert_eq!(t.agent_accent("claude"), t.palette[3]);
    assert_eq!(t.agent_accent("codex"), parse_color("#010203").unwrap());
    assert_eq!(t.agent_accent("sol"), Color::AnsiValue(74));
    let plain = Theme::resolve(&config(), &ClientEnv::default());
    assert_eq!(t.agent_accent("invalid"), plain.agent_accent("invalid"));
    for name in ["claude", "codex", "sol", "terra", "reviewer_2", "中文"] {
        assert_eq!(
            plain.agent_accent(name),
            Theme::resolve(&config(), &ClientEnv::default()).agent_accent(name)
        );
        assert!(plain.palette.contains(&plain.agent_accent(name)));
    }
    // Lock the name-to-color contract, rather than merely comparing two identical calls.
    assert_eq!(plain.agent_accent("claude"), plain.palette[3]);
    assert_eq!(plain.agent_accent("codex"), plain.palette[0]);
}

#[test]
fn aurora_body_and_muted_text_meet_requested_surface_contrast() {
    let t = Theme::resolve(&config(), &ClientEnv::default());
    let luminance = |color: Color| {
        let Color::Rgb { r, g, b } = color else {
            panic!("contrast requires RGB preset tokens")
        };
        let channel = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    };
    let contrast = |fg, bg| {
        let (a, b) = (luminance(fg), luminance(bg));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    for surface in [t.base_bg, t.surface_bg, t.raised_bg] {
        assert!(contrast(t.bar_fg, surface) >= 4.5);
        assert!(contrast(t.muted, surface) >= 3.0);
    }
}
