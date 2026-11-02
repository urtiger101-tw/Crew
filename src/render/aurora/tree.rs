use super::{BOLD, Ctx, Grid, Hit, Lang, Run, State, Theme, UNDER, cut, paint, run, tr};
use crate::server::{Client, Session};
use unicode_width::UnicodeWidthStr;

pub(super) fn tree_runs(st: &State, c: &Client, s: &Session, t: &Theme) -> Vec<Run> {
    let width = st.tree_width(s);
    if width < 4 || s.rows < 4 {
        return Vec::new();
    }
    let lang = Lang::resolve(&st.cfg.lang, c.env.lang);
    let mut rows = Vec::new();
    for (si, session) in st.sessions.iter().enumerate() {
        let open = session.id == s.id;
        let marker = if open {
            if t.ascii { "v" } else { "▾" }
        } else if t.ascii {
            ">"
        } else {
            "▸"
        };
        rows.push((
            format!("{marker} {}", session.name),
            Hit::Workspace(si),
            open,
            None,
        ));
        if !open {
            continue;
        }
        for (wi, window) in session.windows.iter().enumerate() {
            rows.push((
                format!("  {wi} {}", window.name),
                Hit::Tab(si, wi),
                wi == session.active,
                None,
            ));
            for pid in window.panes() {
                let Some(pane) = st.panes.get(&pid) else {
                    continue;
                };
                let display = crate::metadata::display::pane(st, pid, lang);
                let name = display
                    .as_ref()
                    .map_or_else(|| format!("p{pid}"), |d| d.sidebar.clone());
                let state = crate::semantic::pane(pane, &st.cfg);
                let label = format!(
                    "    {}  {} {}",
                    state.glyph(t.ascii),
                    name,
                    state.word(lang)
                );
                rows.push((
                    label,
                    Hit::Pane(pid),
                    wi == session.active && pid == window.active,
                    Some(pid),
                ));
            }
        }
    }
    let limit = usize::from(s.rows.saturating_sub(5));
    let hidden = rows.len().saturating_sub(limit);
    let visible = if hidden > 0 {
        limit.saturating_sub(1)
    } else {
        limit
    };
    let mut runs = vec![run(
        1,
        0,
        cut(
            tr(lang, "render.aurora.workspaces"),
            usize::from(width - 2),
            t.g.ellipsis,
        ),
        (t.c(t.muted), t.c(t.surface_bg), BOLD),
        Some(Hit::Chooser),
    )];
    for (index, (label, hit, selected, pane)) in rows.iter().take(visible).enumerate() {
        let y = index as u16 + 2;
        let bg = t.c(if *selected { t.raised_bg } else { t.surface_bg });
        let fg = if matches!(hit, Hit::Workspace(_)) && *selected {
            t.c(t.palette[3])
        } else {
            t.c(t.sidebar_fg)
        };
        let attrs = if *selected {
            BOLD | if matches!(hit, Hit::Pane(_)) {
                UNDER
            } else {
                0
            }
        } else {
            0
        };
        runs.push(run(
            0,
            y,
            " ".repeat(usize::from(width - 1)),
            (fg, bg, attrs),
            Some(*hit),
        ));
        runs.push(run(
            1,
            y,
            cut(label, usize::from(width - 2), t.g.ellipsis),
            (fg, bg, attrs),
            Some(*hit),
        ));
        if let Some(pid) = pane {
            let p = &st.panes[pid];
            let state = crate::semantic::pane(p, &st.cfg);
            let identity = crate::semantic::identity(p);
            let accent = identity.map_or(t.c(t.muted), |name| t.agent_accent(name));
            runs.push(run(
                3,
                y,
                if t.ascii { "*" } else { "▪" },
                (accent, bg, attrs),
                Some(*hit),
            ));
            runs.push(run(
                5,
                y,
                state.glyph(t.ascii),
                (
                    t.state_color(state.color_role),
                    bg,
                    attrs | if state.bold { BOLD } else { 0 },
                ),
                Some(*hit),
            ));
        }
    }
    if hidden > 0 {
        runs.push(run(
            1,
            visible as u16 + 2,
            cut(
                &format!(
                    "+{} {}",
                    rows.len() - visible,
                    tr(lang, "render.aurora.more")
                ),
                usize::from(width - 2),
                t.g.ellipsis,
            ),
            (t.c(t.muted), t.c(t.surface_bg), 0),
            Some(Hit::Chooser),
        ));
    }
    let hint = format!(
        "{} b  {}",
        st.cfg.prefix_name(),
        tr(lang, "render.aurora.hide")
    );
    runs.push(run(
        1,
        s.rows - 2,
        cut(&hint, usize::from(width - 2), t.g.ellipsis),
        (t.c(t.muted), t.c(t.surface_bg), 0),
        None,
    ));
    runs
}

pub(super) fn draw_tree(g: &mut Grid, cx: &Ctx) {
    let width = cx.st.tree_width(cx.s);
    if width == 0 {
        return;
    }
    let t = &cx.theme;
    for y in 0..cx.h.saturating_sub(1) {
        g.text(
            0,
            y,
            width,
            &" ".repeat(usize::from(width)),
            t.c(t.sidebar_fg),
            t.c(t.surface_bg),
            0,
        );
        g.text(
            width - 1,
            y,
            1,
            t.g.vline,
            t.c(t.border),
            t.c(t.surface_bg),
            0,
        );
    }
    paint(g, &tree_runs(cx.st, cx.c, cx.s, t));
}

pub(super) fn draw_files(g: &mut Grid, cx: &Ctx) {
    let width = cx.st.files_width(cx.s);
    if width == 0 {
        return;
    }
    let x = cx.w - cx.st.sidebar_width(cx.s) - width;
    let t = &cx.theme;
    for y in 0..cx.h.saturating_sub(1) {
        g.text(
            x,
            y,
            width,
            &" ".repeat(usize::from(width)),
            t.c(t.sidebar_fg),
            t.c(t.surface_bg),
            0,
        );
        g.text(x, y, 1, t.g.vline, t.c(t.border), t.c(t.surface_bg), 0);
    }
    let text_width = width.saturating_sub(3);
    g.text(
        x + 2,
        0,
        text_width,
        &cut(
            tr(cx.lang, "render.aurora.files"),
            usize::from(text_width),
            t.g.ellipsis,
        ),
        t.c(t.muted),
        t.c(t.surface_bg),
        BOLD,
    );
    let cwd = cx
        .st
        .panes
        .get(&cx.s.windows[cx.s.active].active)
        .and_then(|p| p.cwd.as_ref());
    let snapshot = cwd.and_then(|cwd| cx.st.chrome.snapshot.files.get(cwd));
    g.text(
        x + 2,
        1,
        text_width,
        &cut(
            tr(cx.lang, "render.aurora.start_directory"),
            usize::from(text_width),
            t.g.ellipsis,
        ),
        t.c(t.muted),
        t.c(t.surface_bg),
        0,
    );
    g.text(
        x + 2,
        2,
        text_width,
        &cut(
            cwd.map_or("?", String::as_str),
            usize::from(text_width),
            t.g.ellipsis,
        ),
        t.c(t.sidebar_fg),
        t.c(t.surface_bg),
        0,
    );
    let lines = match snapshot {
        Some(Ok(files)) if !files.is_empty() => {
            let limit = usize::from(cx.h.saturating_sub(7));
            for (index, file) in files.iter().take(limit).enumerate() {
                let color = if file.status.contains('D') {
                    t.error
                } else if file.status == "??" {
                    t.palette[3]
                } else {
                    t.palette[4]
                };
                g.text(
                    x + 2,
                    index as u16 + 4,
                    2,
                    &file.status,
                    t.c(color),
                    t.c(t.surface_bg),
                    0,
                );
                g.text(
                    x + 5,
                    index as u16 + 4,
                    width.saturating_sub(6),
                    &cut(
                        &file.path,
                        usize::from(width.saturating_sub(6)),
                        t.g.ellipsis,
                    ),
                    t.c(t.sidebar_fg),
                    t.c(t.surface_bg),
                    0,
                );
            }
            if files.len() > limit {
                g.text(
                    x + 2,
                    cx.h - 3,
                    text_width,
                    &format!("+{}", files.len() - limit),
                    t.c(t.muted),
                    t.c(t.surface_bg),
                    0,
                );
            }
            Vec::new()
        }
        Some(Ok(_)) => vec![tr(cx.lang, "render.aurora.clean").to_string()],
        Some(Err(error)) => {
            let safe: String = error
                .chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .collect();
            vec![
                tr(cx.lang, "render.aurora.files_error").into(),
                cut(&safe, usize::from(text_width), t.g.ellipsis),
            ]
        }
        None => vec![tr(cx.lang, "render.aurora.loading").to_string()],
    };
    for (index, line) in lines.iter().enumerate() {
        g.text(
            x + 2,
            index as u16 + 4,
            text_width,
            &cut(line, usize::from(text_width), t.g.ellipsis),
            t.c(t.muted),
            t.c(t.surface_bg),
            0,
        );
    }
    let footer = tr(cx.lang, "render.aurora.read_only");
    g.text(
        x + 2,
        cx.h - 2,
        text_width.min(footer.width() as u16),
        footer,
        t.c(t.muted),
        t.c(t.surface_bg),
        0,
    );
}
