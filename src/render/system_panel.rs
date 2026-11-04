//! Compact host and active-pane launch information for the lower sidebar section.

use super::{BOLD, Ctx, DIM, Grid, cut};
#[cfg(test)]
use crate::i18n::Lang;
use crate::i18n::tr;
use crossterm::style::Color;
use unicode_width::UnicodeWidthStr;

/// Rows reserved for the system panel, leaving the final terminal row to the status bar.
pub(super) fn section_height(terminal_rows: u16) -> u16 {
    if terminal_rows < 12 {
        0
    } else {
        10.min((terminal_rows - 1) / 2)
    }
}

/// Draw the bounded lower sidebar panel. `Pane::cwd` is the launch directory recorded when
/// that pane was created; this deliberately does not claim to know the shell's current cwd.
pub(super) fn draw(g: &mut Grid, cx: &Ctx, x: u16, width: u16, top: u16, height: u16, bg: Color) {
    let width = width.min(g.w.saturating_sub(x));
    let height = height.min(g.h.saturating_sub(top));
    if width == 0 || height == 0 {
        return;
    }

    let theme = &cx.theme;
    for row in top..top.saturating_add(height) {
        g.text(
            x,
            row,
            width,
            &" ".repeat(usize::from(width)),
            theme.c(theme.sidebar_fg),
            bg,
            0,
        );
    }

    let text_x = x.saturating_add(2);
    let text_width = width.saturating_sub(3).min(g.w.saturating_sub(text_x));
    if height >= 1 {
        let title = cut(
            tr(cx.lang, "render.system.title"),
            usize::from(text_width),
            theme.g.ellipsis,
        );
        g.text(
            text_x,
            top,
            text_width,
            &title,
            theme.c(theme.accent),
            bg,
            BOLD,
        );
    }
    if height >= 2 {
        g.text(
            x,
            top + 1,
            width,
            &theme.g.hline.repeat(usize::from(width)),
            theme.c(theme.border),
            bg,
            0,
        );
    }
    if height <= 2 {
        return;
    }

    let active_pane =
        cx.s.windows
            .get(cx.s.active)
            .and_then(|window| cx.st.panes.get(&window.active));
    let launch_cwd = active_pane.and_then(|pane| pane.cwd.as_deref());
    let launch_label = tr(cx.lang, "render.system.launch_dir");
    if let Some(path) = launch_cwd {
        draw_path_field(
            g,
            text_x,
            top + 2,
            text_width,
            launch_label,
            &crate::api::display_cwd(path),
            theme.g.ellipsis,
            bg,
            theme.c(theme.muted),
            theme.c(theme.sidebar_fg),
        );
    } else {
        draw_field(
            g,
            text_x,
            top + 2,
            text_width,
            launch_label,
            tr(cx.lang, "render.system.na"),
            theme.g.ellipsis,
            bg,
            theme.c(theme.muted),
            theme.c(theme.sidebar_fg),
        );
    }

    let disk = launch_cwd.and_then(|cwd| cx.st.system_status.snapshot.disks.get(cwd));
    let disk_value = match (launch_cwd, disk) {
        (None, _) | (Some(_), Some(None)) => tr(cx.lang, "render.system.na").to_owned(),
        (Some(_), None) => tr(cx.lang, "render.system.pending").to_owned(),
        (Some(_), Some(Some(sample))) => byte_pair(sample.available, sample.total),
    };
    if height >= 4 {
        draw_field(
            g,
            text_x,
            top + 3,
            text_width,
            tr(cx.lang, "render.system.disk_available"),
            &disk_value,
            theme.g.ellipsis,
            bg,
            theme.c(theme.muted),
            theme.c(theme.sidebar_fg),
        );
    }

    let memory_value = cx
        .st
        .system_status
        .snapshot
        .memory
        .as_ref()
        .map(|sample| byte_pair(sample.used, sample.total))
        .unwrap_or_else(|| tr(cx.lang, "render.system.na").to_owned());
    if height >= 5 {
        draw_field(
            g,
            text_x,
            top + 4,
            text_width,
            tr(cx.lang, "render.system.memory_used"),
            &memory_value,
            theme.g.ellipsis,
            bg,
            theme.c(theme.muted),
            theme.c(theme.sidebar_fg),
        );
    }

    if height >= 6 {
        draw_field(
            g,
            text_x,
            top + 5,
            text_width,
            tr(cx.lang, "render.system.os"),
            std::env::consts::OS,
            theme.g.ellipsis,
            bg,
            theme.c(theme.muted),
            theme.c(theme.sidebar_fg),
        );
    }
    if height >= 7 {
        draw_field(
            g,
            text_x,
            top + 6,
            text_width,
            tr(cx.lang, "render.system.panes"),
            &cx.st.panes.len().to_string(),
            theme.g.ellipsis,
            bg,
            theme.c(theme.muted),
            theme.c(theme.sidebar_fg),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_field(
    g: &mut Grid,
    x: u16,
    y: u16,
    width: u16,
    label: &str,
    value: &str,
    ellipsis: &str,
    bg: Color,
    label_fg: Color,
    value_fg: Color,
) {
    let room = draw_label(g, x, y, width, label, ellipsis, bg, label_fg);
    let value_x = x.saturating_add(width.saturating_sub(room));
    let safe_value = clean_display(value);
    let value = cut(&safe_value, usize::from(room), ellipsis);
    g.text(value_x, y, room, &value, value_fg, bg, 0);
}

#[allow(clippy::too_many_arguments)]
fn draw_path_field(
    g: &mut Grid,
    x: u16,
    y: u16,
    width: u16,
    label: &str,
    path: &str,
    ellipsis: &str,
    bg: Color,
    label_fg: Color,
    value_fg: Color,
) {
    let room = draw_label(g, x, y, width, label, ellipsis, bg, label_fg);
    let value_x = x.saturating_add(width.saturating_sub(room));
    let path = crop_path(&clean_display(path), usize::from(room), ellipsis);
    g.text(value_x, y, room, &path, value_fg, bg, 0);
}

#[allow(clippy::too_many_arguments)]
fn draw_label(
    g: &mut Grid,
    x: u16,
    y: u16,
    width: u16,
    label: &str,
    ellipsis: &str,
    bg: Color,
    fg: Color,
) -> u16 {
    let prefix = format!("{}: ", clean_display(label));
    let prefix = cut(&prefix, usize::from(width), ellipsis);
    let used = prefix.width().min(usize::from(width)) as u16;
    g.text(x, y, used, &prefix, fg, bg, DIM);
    width - used
}

/// Keep a path's final component visible when its full spelling cannot fit the sidebar row.
fn crop_path(path: &str, max_width: usize, ellipsis: &str) -> String {
    if path.width() <= max_width {
        return path.to_owned();
    }
    let ellipsis_width = ellipsis.width();
    if max_width < ellipsis_width {
        return String::new();
    }
    let basename = path
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path);
    let with_separator = format!("{ellipsis}/{basename}");
    if with_separator.width() <= max_width {
        return with_separator;
    }

    let suffix_width = max_width.saturating_sub(ellipsis_width);
    let mut suffix = String::new();
    let mut used = 0;
    for cluster in super::display_clusters(basename).into_iter().rev() {
        let width = cluster.width();
        if used + width > suffix_width {
            break;
        }
        suffix.insert_str(0, cluster);
        used += width;
    }
    format!("{ellipsis}{suffix}")
}

fn clean_display(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_control()
                || matches!(
                    ch,
                    '\u{061c}'
                        | '\u{200e}'..='\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                )
            {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn byte_pair(first: u64, second: u64) -> String {
    let (first_value, first_unit) = byte_parts(first);
    let (second_value, second_unit) = byte_parts(second);
    if first_unit == second_unit {
        format!("{first_value} / {second_value} {first_unit}")
    } else {
        format!("{first_value} {first_unit} / {second_value} {second_unit}")
    }
}

fn byte_parts(bytes: u64) -> (String, &'static str) {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let bytes = u128::from(bytes);
    let mut unit = 0;
    let mut divisor = 1_u128;
    while unit + 1 < UNITS.len() && bytes >= divisor * 1024 {
        divisor *= 1024;
        unit += 1;
    }
    let tenths = (bytes * 10 + divisor / 2) / divisor;
    let value = if tenths < 100 {
        if tenths.is_multiple_of(10) {
            (tenths / 10).to_string()
        } else {
            format!("{}.{:01}", tenths / 10, tenths % 10)
        }
    } else {
        ((tenths + 5) / 10).to_string()
    };
    (value, UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Cell;
    use crate::server::{test_client, test_session, test_state};
    use crate::theme::Theme;

    fn blank(width: u16, height: u16) -> Grid {
        Grid {
            w: width,
            h: height,
            cells: vec![Cell::default(); width as usize * height as usize],
        }
    }

    fn row_text(g: &Grid, y: u16) -> String {
        g.cells[y as usize * g.w as usize..(y as usize + 1) * g.w as usize]
            .iter()
            .filter(|cell| !cell.skip)
            .map(|cell| cell.s.as_str())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn section_height_keeps_the_status_row_and_caps_the_panel() {
        assert_eq!(section_height(0), 0);
        assert_eq!(section_height(11), 0);
        assert_eq!(section_height(12), 5);
        assert_eq!(section_height(16), 7);
        assert_eq!(section_height(30), 10);
    }

    #[test]
    fn byte_pairs_are_compact_and_share_matching_units() {
        assert_eq!(byte_pair(1_536, 8_192), "1.5 / 8 KiB");
        assert_eq!(byte_pair(1, 1_024), "1 B / 1 KiB");
        assert_eq!(byte_pair(0, 0), "0 / 0 B");
    }

    #[test]
    fn path_crop_keeps_the_basename_and_unicode_cluster_boundary() {
        let path = r"C:\Users\crew\Projects\repository";
        let short = crop_path(path, 16, "…");
        assert!(short.ends_with("/repository"), "{short:?}");
        assert!(short.width() <= 16, "{short:?}");

        let cjk = crop_path("/a/b/目錄名稱", 8, "…");
        assert!(cjk.ends_with("名稱"), "{cjk:?}");
        assert!(cjk.width() <= 8, "{cjk:?}");
    }

    #[test]
    fn controls_and_bidi_overrides_are_replaced_before_rendering() {
        let clean = clean_display("C:\\tmp\u{001b}[2J\u{202e}evil\tname");
        assert_eq!(clean, "C:\\tmp [2J evil name");
        assert!(!clean.chars().any(char::is_control));
    }

    #[test]
    fn short_draw_prioritizes_directory_disk_and_memory_rows() {
        let st = test_state();
        let session = test_session("panel", 50, 12, 7);
        let client = test_client("panel");
        let cx = Ctx {
            st: &st,
            s: &session,
            c: &client,
            w: 50,
            h: 12,
            rects: vec![],
            area: st.pane_area(&session),
            theme: Theme::terminal(),
            lang: Lang::En,
        };
        let mut grid = blank(50, 5);
        draw(&mut grid, &cx, 0, 50, 0, 5, Color::Reset);

        assert!(row_text(&grid, 0).contains("SYSTEM"));
        assert!(row_text(&grid, 2).contains("Launch dir: N/A"));
        assert!(row_text(&grid, 3).contains("Disk free: N/A"));
        assert!(row_text(&grid, 4).contains("RAM used: N/A"));
        assert!(!row_text(&grid, 4).contains("OS:"));
    }

    #[test]
    fn panel_drawing_stays_within_grid_and_emits_no_controls_from_values() {
        let st = test_state();
        let session = test_session("panel", 10, 7, 7);
        let client = test_client("panel");
        let cx = Ctx {
            st: &st,
            s: &session,
            c: &client,
            w: 10,
            h: 7,
            rects: vec![],
            area: st.pane_area(&session),
            theme: Theme::terminal(),
            lang: Lang::ZhTw,
        };
        let mut grid = blank(10, 7);
        draw(&mut grid, &cx, 6, 20, 4, 10, Color::DarkBlue);
        assert_eq!(grid.cells.len(), 70);
        assert!(row_text(&grid, 4).contains("…"));
        assert!(grid.cells.iter().all(|cell| !cell.s.contains('\u{001b}')));
    }
}
