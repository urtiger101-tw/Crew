//! Aurora chrome uses the same cell runs for painting and pointer targets.
use super::{BOLD, Ctx, Grid, INVERSE, UNDER, cut};
use crate::i18n::{Lang, tr};
use crate::layout::Rect;
use crate::server::{Client, Session, State};
use crate::theme::Theme;
use crossterm::style::Color;
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
mod tests;
mod tree;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    Workspace(usize),
    Tab(usize, usize),
    Pane(usize),
    Chooser,
    Attention,
    Doctor,
}

struct Run {
    x: u16,
    y: u16,
    text: String,
    style: (Color, Color, u8),
    hit: Option<Hit>,
}

fn run(
    x: u16,
    y: u16,
    text: impl Into<String>,
    style: (Color, Color, u8),
    hit: Option<Hit>,
) -> Run {
    Run {
        x,
        y,
        text: text.into(),
        style,
        hit,
    }
}

fn paint(g: &mut Grid, runs: &[Run]) {
    for r in runs {
        g.text(
            r.x,
            r.y,
            g.w.saturating_sub(r.x),
            &r.text,
            r.style.0,
            r.style.1,
            r.style.2,
        );
    }
}

fn tab_runs(st: &State, c: &Client, s: &Session, theme: &Theme) -> Vec<Run> {
    let y = st.tab_row(s);
    let si = st.sidx(&s.name).unwrap();
    let mut runs = Vec::new();
    let mut x: u16 = 0;
    let end = if s.cols >= 60 { s.cols / 2 } else { s.cols };
    let brand_width = end.saturating_sub(8).min(18);
    let brand = if brand_width >= 4 {
        format!(
            " {} ",
            cut(&s.name, usize::from(brand_width - 2), theme.g.ellipsis)
        )
    } else {
        String::new()
    };
    if !brand.is_empty() {
        let width = brand.width() as u16;
        runs.push(run(
            x,
            y,
            brand,
            (
                theme.c(theme.base_bg),
                theme.c(theme.palette[5]),
                if theme.mono { BOLD | INVERSE } else { BOLD },
            ),
            Some(Hit::Chooser),
        ));
        x += width;
    }
    x = x.saturating_add(1).min(end);
    let mut labels = Vec::new();
    for (wi, window) in s.windows.iter().enumerate() {
        labels.push(format!(
            "{} {}",
            wi,
            cut(&window.name, 22, theme.g.ellipsis)
        ));
    }
    let available = usize::from(end.saturating_sub(x));
    if labels.iter().map(|s| s.width() + 3).sum::<usize>() > available {
        labels = (0..s.windows.len()).map(|wi| wi.to_string()).collect();
    }
    // Keep the active tab in view when even numeric pills overflow.
    let start = if labels.iter().map(|s| s.width() + 3).sum::<usize>() > available {
        s.active
    } else {
        0
    };
    for (wi, label) in labels.iter().enumerate().skip(start) {
        let text = format!(" {label} ");
        let width = text.width() as u16;
        if x.saturating_add(width) > end {
            break;
        }
        let active = wi == s.active;
        let fg = theme.c(if active { theme.bar_fg } else { theme.muted });
        let bg = theme.c(if active {
            theme.surface_bg
        } else {
            theme.base_bg
        });
        let attrs = if active {
            BOLD | if theme.mono { INVERSE } else { 0 }
        } else {
            0
        };
        runs.push(run(x, y, text, (fg, bg, attrs), Some(Hit::Tab(si, wi))));
        x += width + 1;
    }
    let _ = c;
    runs
}

fn footer_runs(st: &State, c: &Client, s: &Session, theme: &Theme) -> Vec<Run> {
    let y = s.rows.saturating_sub(1);
    let lang = Lang::resolve(&st.cfg.lang, c.env.lang);
    let bg = theme.c(theme.base_bg);
    let mut runs = tab_runs(st, c, s, theme);
    let mut x = runs
        .iter()
        .map(|r| r.x + r.text.width() as u16)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let clock = clock_text();
    let warning = doctor_warning(st, c);
    let clock = format!(" {clock} ");
    let doctor = " ! doctor ";
    let right_width = clock.width() + if warning { doctor.width() + 1 } else { 0 };
    let right_x = s.cols.saturating_sub(right_width as u16);
    if right_x > x + 4 {
        if warning {
            runs.push(run(
                right_x,
                y,
                doctor,
                (theme.c(theme.base_bg), theme.c(theme.warn), BOLD),
                Some(Hit::Doctor),
            ));
        }
        runs.push(run(
            s.cols.saturating_sub(clock.width() as u16),
            y,
            clock,
            (theme.c(theme.muted), bg, 0),
            None,
        ));
    }
    let mut end = if right_x > x + 4 {
        right_x.saturating_sub(1)
    } else {
        s.cols
    };
    if st.cfg.hints {
        let help = format!(
            "{} {}",
            st.cfg.prefix_name(),
            tr(lang, "render.status.help")
        );
        let width = help.width() as u16;
        if end.saturating_sub(x) >= width + 2 {
            runs.push(run(
                end - width,
                y,
                help,
                (theme.c(theme.muted), bg, 0),
                None,
            ));
            end -= width + 2;
        }
    }
    if let Some(message) = &c.message {
        runs.push(run(
            x,
            y,
            cut(
                &message.text,
                usize::from(end.saturating_sub(x)),
                theme.g.ellipsis,
            ),
            (
                theme.c(if message.error {
                    theme.error
                } else {
                    theme.bar_fg
                }),
                bg,
                u8::from(message.error) * BOLD,
            ),
            None,
        ));
        return runs;
    }
    if st
        .panes
        .get(&s.windows[s.active].active)
        .is_some_and(|p| p.copy.is_some())
    {
        runs.push(run(
            x,
            y,
            cut(
                tr(lang, "render.aurora.copy"),
                usize::from(end.saturating_sub(x)),
                theme.g.ellipsis,
            ),
            (theme.c(theme.warn), bg, BOLD),
            None,
        ));
        return runs;
    }
    let mut agents: Vec<_> = st
        .sessions
        .iter()
        .flat_map(|session| &session.windows)
        .flat_map(|window| window.panes())
        .filter_map(|pid| {
            let pane = st.panes.get(&pid)?;
            (crate::persist::restore_state(pane).is_none()).then_some(())?;
            Some((pid, pane, crate::semantic::identity(pane)?))
        })
        .collect();
    agents.sort_by_key(|(pid, _, _)| *pid != s.windows[s.active].active);
    let mut shown = 0;
    for (pid, pane, identity) in &agents {
        let presentation = crate::semantic::pane(pane, &st.cfg);
        let name = cut(identity, 12, theme.g.ellipsis);
        let open = pane
            .agent
            .as_ref()
            .and_then(|a| a.name.as_ref())
            .and_then(|name| st.chrome.snapshot.inbox.get(name))
            .copied()
            .unwrap_or(0);
        let suffix = if open > 0 {
            format!(" [{}]", open)
        } else {
            String::new()
        };
        let text = format!(
            " {} {} {}{} ",
            presentation.glyph(theme.ascii),
            name,
            presentation.word(lang),
            suffix
        );
        let width = text.width() as u16;
        if x.saturating_add(width + 5) > end {
            break;
        }
        let active = *pid == s.windows[s.active].active;
        let chip_bg = theme.c(if active {
            theme.raised_bg
        } else {
            theme.surface_bg
        });
        let attrs = if presentation.bold { BOLD } else { 0 };
        runs.push(run(
            x,
            y,
            text,
            (theme.state_color(presentation.color_role), chip_bg, attrs),
            Some(Hit::Pane(*pid)),
        ));
        // Identity and lifecycle keep separate colors inside the same clickable chip.
        runs.push(run(
            x + 3,
            y,
            name,
            (theme.agent_accent(identity), chip_bg, attrs),
            Some(Hit::Pane(*pid)),
        ));
        x += width + 1;
        shown += 1;
    }
    if shown < agents.len() {
        let text = format!("+{}", agents.len() - shown);
        runs.push(run(
            x,
            y,
            cut(&text, usize::from(end.saturating_sub(x)), theme.g.ellipsis),
            (theme.c(theme.warn), bg, BOLD),
            Some(Hit::Attention),
        ));
    }
    runs
}

pub(crate) fn doctor_warning(st: &State, c: &Client) -> bool {
    let current: Vec<u16> = env!("CARGO_PKG_VERSION")
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    c.env
        .version
        .is_some_and(|version| version.as_slice() != current.as_slice())
        || c.env.no_color
        || c.env.dumb
        || st.diagnostics["environment_hazards"]
            .as_array()
            .is_some_and(|hazards| !hazards.is_empty())
        || st.chrome.snapshot.inbox_error.is_some()
}

pub(crate) fn doctor_lines(st: &State, c: &Client) -> Vec<String> {
    vec![
        format!(
            "Crew daemon {} / client {:?}",
            env!("CARGO_PKG_VERSION"),
            c.env.version
        ),
        format!(
            "Color: NO_COLOR={}, TERM=dumb={}",
            c.env.no_color, c.env.dumb
        ),
        format!(
            "Daemon color hazards: {}",
            st.diagnostics["environment_hazards"]
        ),
        st.chrome.snapshot.inbox_error.as_ref().map_or_else(
            || "Inbox: available".into(),
            |error| format!("Inbox: {error}"),
        ),
        "Esc / q: close".into(),
    ]
}

fn clock_text() -> String {
    #[cfg(windows)]
    {
        let mut time = windows_sys::Win32::Foundation::SYSTEMTIME::default();
        unsafe {
            windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut time);
        }
        format!("{:02}:{:02}", time.wHour, time.wMinute)
    }
    #[cfg(unix)]
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as libc::time_t;
        let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
        if unsafe { libc::localtime_r(&now, local.as_mut_ptr()) }.is_null() {
            return "--:--".into();
        }
        let local = unsafe { local.assume_init() };
        format!("{:02}:{:02}", local.tm_hour, local.tm_min)
    }
}

pub(crate) fn tab_at(st: &State, c: &Client, s: &Session, col: u16) -> Option<usize> {
    let theme = Theme::resolve(&st.cfg.theme, &c.env);
    tab_runs(st, c, s, &theme)
        .into_iter()
        .find_map(|r| match r.hit {
            Some(Hit::Tab(_, wi)) if col >= r.x && usize::from(col - r.x) < r.text.width() => {
                Some(wi)
            }
            _ => None,
        })
}

pub(crate) fn hit_at(st: &State, c: &Client, s: &Session, col: u16, row: u16) -> Option<Hit> {
    if c.prompt.is_some() {
        return None;
    }
    let theme = Theme::resolve(&st.cfg.theme, &c.env);
    let runs = if row == s.rows.saturating_sub(1) {
        footer_runs(st, c, s, &theme)
    } else {
        tree::tree_runs(st, c, s, &theme)
    };
    runs.into_iter().find_map(|r| {
        (r.y == row && col >= r.x && usize::from(col - r.x) < r.text.width())
            .then_some(r.hit)
            .flatten()
    })
}

pub(super) fn draw_chrome(g: &mut Grid, cx: &Ctx) {
    tree::draw_tree(g, cx);
    tree::draw_files(g, cx);
}

pub(super) fn draw_footer(g: &mut Grid, cx: &Ctx) {
    g.text(
        0,
        cx.h.saturating_sub(1),
        cx.w,
        &" ".repeat(usize::from(cx.w)),
        cx.theme.c(cx.theme.muted),
        cx.theme.c(cx.theme.base_bg),
        0,
    );
    paint(g, &footer_runs(cx.st, cx.c, cx.s, &cx.theme));
}

pub(super) fn draw_title(g: &mut Grid, cx: &Ctx, pid: usize, r: Rect) {
    let p = &cx.st.panes[&pid];
    let t = &cx.theme;
    let active = pid == cx.s.windows[cx.s.active].active;
    let accent = t.c(t.palette[4]);
    let state = crate::semantic::pane(p, &cx.st.cfg);
    let bg = t.c(if active { t.raised_bg } else { t.base_bg });
    let fg = t.c(if active { t.bar_fg } else { t.muted });
    let attrs = if active { BOLD } else { 0 };
    g.text(r.x, r.y, r.w, &" ".repeat(usize::from(r.w)), fg, bg, attrs);
    let display = crate::metadata::display::pane(cx.st, pid, cx.lang);
    let window = &cx.s.windows[cx.s.active];
    let shell = cx.st.shell_name();
    let name = display.as_ref().map_or(shell.as_str(), |d| {
        if d.title != window.name
            || p.label.is_some()
            || !cx.st.cfg.pane_template.is_empty()
            || d.presentation.title.is_some()
            || d.agent.is_some()
        {
            d.title.as_str()
        } else {
            shell.as_str()
        }
    });
    let id = format!(" p{pid} ");
    let budget = usize::from(r.w).saturating_sub(id.width() + 5);
    let suffix = if crate::persist::restore_state(p) == Some("restored-shell") {
        format!(" {} x", cx.st.cfg.prefix_name())
    } else {
        String::new()
    };
    let cwd = p
        .cwd
        .as_deref()
        .and_then(|cwd| std::path::Path::new(cwd).file_name())
        .map(|name| name.to_string_lossy());
    let mut label = format!("{name} {} {}{suffix}", t.g.dot, state.word(cx.lang));
    if let Some(cwd) = cwd {
        label.push_str(&format!(" {} {cwd}", t.g.dot));
    }
    let label = cut(&label, budget, t.g.ellipsis);
    let pointer = if active {
        if t.ascii { "|" } else { "▍" }
    } else {
        " "
    };
    g.text(
        r.x.saturating_add(1),
        r.y,
        r.w.saturating_sub(1),
        pointer,
        accent,
        bg,
        attrs,
    );
    g.text(
        r.x.saturating_add(3),
        r.y,
        r.w.saturating_sub(3),
        state.glyph(t.ascii),
        if active {
            t.state_color(state.color_role)
        } else {
            fg
        },
        bg,
        attrs | if state.bold { BOLD } else { 0 },
    );
    g.text(
        r.x.saturating_add(5),
        r.y,
        r.w.saturating_sub(5),
        &label,
        fg,
        bg,
        attrs,
    );
    if r.w as usize >= id.width() + 8 {
        g.text(
            r.x + r.w - id.width() as u16,
            r.y,
            id.width() as u16,
            &id,
            t.c(t.muted),
            bg,
            attrs,
        );
    }
}
