//! Top toolbar: a row of function-key buttons (`F1 help  F2 new ...`) over the pane columns,
//! each running a Crew command when clicked or when its F-key is pressed.
//!
//! Geometry is pure ([`toolbar_layout`]). The drawer paints the buttons that layout returns
//! and the mouse code hit-tests the same output ([`toolbar_hit_at`]), so a click lands on the
//! columns that were drawn.
use super::{BOLD, Cell, Ctx, DIM, Grid, INVERSE};
use crate::commands::{self, Cmd};
use crate::config::{Config, TOOLBAR_MAX_BUTTONS};
use crate::i18n::{Lang, tr};
use crate::server::{Client, Session, State};
use crate::theme::Theme;
use crossterm::style::Color;
use unicode_width::UnicodeWidthStr;

/// Columns between two buttons.
const GAP: u16 = 1;

/// Default buttons in F-key order: (label catalog key, ASCII label catalog key, command).
/// The ASCII key is used when the theme swaps glyphs for ASCII (labels with box glyphs).
const DEFAULT_BUTTONS: &[(&str, Option<&str>, &str)] = &[
    ("render.toolbar.help", None, "list-keys"),
    ("render.toolbar.new", None, "new-window"),
    (
        "render.toolbar.split_h",
        Some("render.toolbar.split_h_ascii"),
        "split-window -h",
    ),
    (
        "render.toolbar.split_v",
        Some("render.toolbar.split_v_ascii"),
        "split-window -v",
    ),
    ("render.toolbar.zoom", None, "resize-pane -Z"),
    ("render.toolbar.next", None, "next-pane"),
    ("render.toolbar.copy", None, "copy-mode"),
    ("render.toolbar.paste", None, "paste-buffer"),
    ("render.toolbar.tree", None, "choose-tree"),
    ("render.toolbar.close", None, "kill-pane"),
    ("render.toolbar.sidebar", None, "toggle-sidebar"),
    ("render.toolbar.detach", None, "detach-client"),
];

/// One button as the layout needs it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    /// key chip text, `F1`..`F12`
    pub key: String,
    /// label drawn after the chip (without the padding spaces)
    pub label: String,
    /// command line as configured
    pub command: String,
}

/// One placed button: item `index` spans columns `[x0, x1)` (chip and label).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Btn {
    pub index: usize,
    pub x0: u16,
    pub x1: u16,
}

/// Result of [`toolbar_layout`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolbarLayout {
    pub btns: Vec<Btn>,
    /// column of the ellipsis that marks dropped buttons; `None` when all fit or no room
    pub more: Option<u16>,
}

/// Command line of button `index` as configured (defaults when none are set).
fn command_of(cfg: &Config, index: usize) -> Option<&str> {
    if cfg.toolbar_buttons.is_empty() {
        DEFAULT_BUTTONS.get(index).map(|&(_, _, c)| c)
    } else {
        cfg.toolbar_buttons
            .get(index)
            .filter(|_| index < TOOLBAR_MAX_BUTTONS)
            .map(|b| b.command.as_str())
    }
}

/// The buttons for one client: configured ones, or the localized defaults.
pub(crate) fn toolbar_items(cfg: &Config, lang: Lang, ascii: bool) -> Vec<Item> {
    let pairs: Vec<(String, String)> = if cfg.toolbar_buttons.is_empty() {
        DEFAULT_BUTTONS
            .iter()
            .map(|&(key, ascii_key, command)| {
                let key = match ascii_key {
                    Some(a) if ascii => a,
                    _ => key,
                };
                (tr(lang, key).to_string(), command.to_string())
            })
            .collect()
    } else {
        cfg.toolbar_buttons
            .iter()
            .take(TOOLBAR_MAX_BUTTONS)
            .map(|b| (b.label.clone(), b.command.clone()))
            .collect()
    };
    pairs
        .into_iter()
        .enumerate()
        .map(|(i, (label, command))| Item {
            key: format!("F{}", i + 1),
            label,
            command,
        })
        .collect()
}

/// Command line for a click on, or the F-key of, a toolbar button. Bare kill commands are
/// wrapped in `confirm-before`, so they ask y/n exactly like the `x` / `&` bindings (and run
/// at once when `confirm-close` is off).
pub(crate) fn guarded(command: &str) -> String {
    match commands::parse(command) {
        Ok(Cmd::KillPane | Cmd::KillWindow { .. } | Cmd::KillSession { .. } | Cmd::KillServer) => {
            format!("confirm-before {command}")
        }
        _ => command.to_string(),
    }
}

/// Command line of button `index` (0 = F1), or `None` when there is no such button.
pub fn toolbar_command(cfg: &Config, index: usize) -> Option<String> {
    command_of(cfg, index).map(guarded)
}

/// Columns of one button: the key chip, then ` label`.
fn button_width(it: &Item) -> u16 {
    let w = it.key.width() + it.label.width() + 1;
    u16::try_from(w).unwrap_or(u16::MAX)
}

/// Pure toolbar layout for `w` columns. Buttons are placed left to right with a one-column
/// gap; the first one that does not fit and every later one are dropped whole (a label is
/// never cut), and an ellipsis `ell` marks the drop when there is room for it.
pub(crate) fn toolbar_layout(items: &[Item], w: u16, ell: &str) -> ToolbarLayout {
    let widths: Vec<u32> = items.iter().map(|it| u32::from(button_width(it))).collect();
    let gaps = u32::from(GAP) * (items.len().saturating_sub(1) as u32);
    let all_fit = widths.iter().sum::<u32>() + gaps <= u32::from(w);
    let ell_w = ell.width() as u32;
    // room kept for ` …` after the last placed button when some are dropped
    let limit = if all_fit {
        u32::from(w)
    } else {
        u32::from(w).saturating_sub(u32::from(GAP) + ell_w)
    };
    let mut btns = Vec::new();
    let mut end = 0u32;
    for (i, bw) in widths.iter().enumerate() {
        let start = if i == 0 { 0 } else { end + u32::from(GAP) };
        if start + bw > limit {
            break;
        }
        // start + bw <= limit <= w, so both fit in u16
        btns.push(Btn {
            index: i,
            x0: start as u16,
            x1: (start + bw) as u16,
        });
        end = start + bw;
    }
    let more = if all_fit {
        None
    } else {
        let at = if btns.is_empty() {
            0
        } else {
            end + u32::from(GAP)
        };
        (ell_w > 0 && at + ell_w <= u32::from(w)).then_some(at as u16)
    };
    ToolbarLayout { btns, more }
}

/// Command line of the toolbar button under (`col`, `row`) for client `c`, computed with the
/// same items and layout as the drawer. `None` when the toolbar is hidden or nothing is there.
pub fn toolbar_hit_at(st: &State, c: &Client, s: &Session, col: u16, row: u16) -> Option<String> {
    if !st.toolbar_shown(s) || row != 0 {
        return None;
    }
    let area = st.pane_area(s);
    if col < area.x || col >= area.x.saturating_add(area.w) {
        return None;
    }
    let theme = Theme::resolve(&st.cfg.theme, &c.env);
    let lang = Lang::resolve(&st.cfg.lang, c.env.lang);
    let items = toolbar_items(&st.cfg, lang, theme.ascii);
    let x = col - area.x;
    toolbar_layout(&items, area.w, theme.g.ellipsis)
        .btns
        .into_iter()
        .find(|b| x >= b.x0 && x < b.x1)
        .map(|b| guarded(&items[b.index].command))
}

/// Foreground, background and attributes of one styled run.
type Style = (Color, Color, u8);

/// Bar foreground on the bar background (the row and the labels).
fn bar_style(t: &Theme) -> Style {
    (t.c(t.bar_fg), t.c(t.bar_bg), 0)
}

/// Key chip: black on the accent token, INVERSE in mono so it still stands out.
fn chip_style(t: &Theme) -> Style {
    if t.mono {
        (Color::Reset, Color::Reset, BOLD | INVERSE)
    } else {
        (Color::Black, t.c(t.accent), BOLD)
    }
}

/// Draw the toolbar on row 0 over the pane columns (nothing when it is hidden).
pub(super) fn draw_toolbar(g: &mut Grid, cx: &Ctx) {
    if !cx.st.toolbar_shown(cx.s) {
        return;
    }
    let t = &cx.theme;
    let (x0, w) = (cx.area.x, cx.area.w);
    let (fg, bg, attrs) = bar_style(t);
    for x in x0..x0.saturating_add(w) {
        g.put(
            x,
            0,
            Cell {
                fg,
                bg,
                attrs,
                ..Cell::default()
            },
        );
    }
    let items = toolbar_items(&cx.st.cfg, cx.lang, t.ascii);
    let layout = toolbar_layout(&items, w, t.g.ellipsis);
    let chip = chip_style(t);
    for b in &layout.btns {
        let it = &items[b.index];
        let x = x0 + b.x0;
        let used = g.text(x, 0, b.x1 - b.x0, &it.key, chip.0, chip.1, chip.2);
        let label = format!(" {}", it.label);
        g.text(x + used, 0, b.x1 - b.x0 - used, &label, fg, bg, attrs);
    }
    if let Some(mx) = layout.more {
        let ell = t.g.ellipsis;
        let room = w.saturating_sub(mx);
        g.text(x0 + mx, 0, room, ell, fg, bg, DIM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ToolbarButton;

    fn item(key: &str, label: &str, command: &str) -> Item {
        Item {
            key: key.into(),
            label: label.into(),
            command: command.into(),
        }
    }

    fn spans(l: &ToolbarLayout) -> Vec<(usize, u16, u16)> {
        l.btns.iter().map(|b| (b.index, b.x0, b.x1)).collect()
    }

    #[test]
    fn layout_places_buttons_with_one_column_gap() {
        let items = [
            item("F1", "help", "list-keys"),
            item("F2", "new", "new-window"),
        ];
        // "F1 help" = 7 cols, gap, "F2 new" = 6 cols -> 14
        let l = toolbar_layout(&items, 14, "…");
        assert_eq!(spans(&l), vec![(0, 0, 7), (1, 8, 14)]);
        assert_eq!(l.more, None);
    }

    #[test]
    fn layout_drops_whole_buttons_and_marks_them_with_an_ellipsis() {
        let items = [
            item("F1", "help", "list-keys"),
            item("F2", "new", "new-window"),
            item("F3", "zoom", "resize-pane -Z"),
        ];
        // F2 cannot fit at 13 columns; at 15 it leaves no room for the marker.
        let l = toolbar_layout(&items, 13, "…");
        assert_eq!(spans(&l), vec![(0, 0, 7)]);
        assert_eq!(l.more, Some(8));
        let l = toolbar_layout(&items, 15, "…");
        assert_eq!(spans(&l), vec![(0, 0, 7)]);
        assert_eq!(l.more, Some(8));
        // At 16 columns F2 and the marker both fit.
        let l = toolbar_layout(&items, 16, "…");
        assert_eq!(spans(&l), vec![(0, 0, 7), (1, 8, 14)]);
        assert_eq!(l.more, Some(15));
    }

    #[test]
    fn layout_on_tiny_or_zero_width_never_cuts_a_label() {
        let items = [
            item("F1", "help", "list-keys"),
            item("F2", "new", "new-window"),
        ];
        let l = toolbar_layout(&items, 3, "~");
        assert!(l.btns.is_empty());
        assert_eq!(l.more, Some(0));
        let l = toolbar_layout(&items, 0, "~");
        assert!(l.btns.is_empty());
        assert_eq!(l.more, None, "no room for the ellipsis");
        assert_eq!(
            toolbar_layout(&[], 10, "…"),
            ToolbarLayout {
                btns: vec![],
                more: None
            }
        );
    }

    #[test]
    fn layout_counts_cjk_labels_as_two_columns() {
        let items = [
            item("F1", "說明", "list-keys"),
            item("F2", "新視窗", "new-window"),
        ];
        // CJK characters occupy two columns, with one gap between buttons.
        let l = toolbar_layout(&items, 17, "…");
        assert_eq!(spans(&l), vec![(0, 0, 7), (1, 8, 17)]);
        let l = toolbar_layout(&items, 16, "…");
        assert_eq!(spans(&l), vec![(0, 0, 7)]);
        assert_eq!(l.more, Some(8));
    }

    #[test]
    fn default_items_cover_f1_to_f12_and_ascii_swaps_box_glyphs() {
        let cfg = Config::default();
        let en = toolbar_items(&cfg, Lang::En, false);
        assert_eq!(en.len(), 12);
        assert_eq!(en[0], item("F1", "help", "list-keys"));
        assert_eq!(en[2].label, "split│");
        assert_eq!(en[11], item("F12", "detach", "detach-client"));
        let ascii = toolbar_items(&cfg, Lang::En, true);
        assert_eq!(ascii[2].label, "split|");
        assert_eq!(ascii[3].label, "split-");
        assert!(ascii.iter().all(|it| it.label.is_ascii()), "{ascii:?}");
        let zh = toolbar_items(&cfg, Lang::ZhTw, false);
        assert_eq!(zh[0].label, "說明");
        assert_eq!(zh[9].label, "關閉");
        for (_, _, command) in DEFAULT_BUTTONS {
            assert!(commands::parse(command).is_ok(), "{command}");
        }
    }

    #[test]
    fn configured_buttons_replace_the_defaults() {
        let cfg = Config {
            toolbar_buttons: vec![ToolbarButton {
                label: "build".into(),
                command: "display hi".into(),
            }],
            ..Config::default()
        };
        assert_eq!(
            toolbar_items(&cfg, Lang::ZhTw, true),
            vec![item("F1", "build", "display hi")]
        );
        assert_eq!(toolbar_command(&cfg, 0).as_deref(), Some("display hi"));
        assert_eq!(toolbar_command(&cfg, 1), None);
    }

    #[test]
    fn kill_commands_go_through_confirm_before() {
        let cfg = Config::default();
        assert_eq!(
            toolbar_command(&cfg, 9).as_deref(),
            Some("confirm-before kill-pane")
        );
        assert_eq!(
            guarded("kill-window -t 1"),
            "confirm-before kill-window -t 1"
        );
        assert_eq!(guarded("kill-session"), "confirm-before kill-session");
        assert_eq!(guarded("new-window"), "new-window");
        assert!(commands::parse(&guarded("kill-window -t 1")).is_ok());
        let items = toolbar_items(&cfg, Lang::En, false);
        let l = toolbar_layout(&items, 200, "…");
        assert_eq!(l.btns.len(), 12);
        assert_eq!(
            guarded(&items[l.btns[9].index].command),
            "confirm-before kill-pane"
        );
        assert_eq!(guarded("kill-server"), "confirm-before kill-server");
    }

    /// Row 0 of a `cols`x10 frame drawn by `draw_toolbar` (the text and each cell's attrs).
    fn drawn_row(cols: u16, sidebar: bool, env: crate::theme::ClientEnv) -> (String, Vec<u8>) {
        let mut st = crate::server::test_state();
        st.cfg.toolbar = true;
        let mut s = crate::server::test_session("t", cols, 10, 7);
        s.sidebar = sidebar;
        let mut c = crate::server::test_client("t");
        c.env = env;
        let area = st.pane_area(&s);
        let cx = Ctx {
            st: &st,
            s: &s,
            c: &c,
            w: cols,
            h: 10,
            rects: vec![],
            area,
            theme: Theme::resolve(&st.cfg.theme, &c.env),
            lang: Lang::En,
        };
        let mut g = Grid {
            w: cols,
            h: 10,
            cells: vec![Cell::default(); usize::from(cols) * 10],
        };
        draw_toolbar(&mut g, &cx);
        let row = &g.cells[..usize::from(cols)];
        let text = row
            .iter()
            .filter(|c| !c.skip)
            .map(|c| c.s.as_str())
            .collect();
        (text, row.iter().map(|c| c.attrs).collect())
    }

    #[test]
    fn drawer_paints_chips_labels_and_the_ellipsis() {
        let env = crate::theme::ClientEnv::default();
        let (text, attrs) = drawn_row(40, false, env);
        assert_eq!(text, "F1 help F2 new F3 split│ F4 split─ …    ");
        assert_eq!(
            &attrs[..3],
            &[BOLD, BOLD, 0],
            "chip is bold, label is plain"
        );
        assert_eq!(attrs[35], DIM, "ellipsis is faint");
        // sidebar on: the toolbar stops before the sidebar columns
        let (text, _) = drawn_row(80, true, env);
        assert_eq!(
            text.trim_end(),
            "F1 help F2 new F3 split│ F4 split─ F5 zoom F6 next …"
        );
    }

    #[test]
    fn drawer_in_ascii_and_mono() {
        let env = crate::theme::ClientEnv {
            ascii: true,
            no_color: true,
            ..Default::default()
        };
        let (text, attrs) = drawn_row(40, false, env);
        assert_eq!(text, "F1 help F2 new F3 split| F4 split- ~    ");
        assert!(text.is_ascii());
        assert_eq!(attrs[0], BOLD | INVERSE, "mono chip is inverse");
    }

    #[test]
    fn mono_chip_uses_inverse_and_no_color() {
        let mut t = Theme::terminal();
        t.mono = true;
        assert_eq!(chip_style(&t), (Color::Reset, Color::Reset, BOLD | INVERSE));
        assert_eq!(bar_style(&t), (Color::Reset, Color::Reset, 0));
    }
}
