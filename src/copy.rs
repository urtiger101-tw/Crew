//! Copy mode: cursor, selection and text extraction over a vt100 screen + scrollback.

use unicode_width::UnicodeWidthStr;

/// ConPTY emits CRLF at full rows, so on Windows a full-width row is assumed to continue.
pub const HARD_WRAP_AT_WIDTH: bool = cfg!(windows);

#[derive(Debug, Clone, Default)]
pub struct CopyState {
    /// cursor in viewport coordinates
    pub cx: u16,
    pub cy: u16,
    /// selection anchor in absolute coordinates (row relative to the live screen top, may be negative)
    pub anchor: Option<(i64, u16)>,
    pub line: bool,
}

pub fn abs_row(view_row: u16, offset: usize) -> i64 {
    view_row as i64 - offset as i64
}

/// Is (row, col) inside the selection spanning a..b?
pub fn in_selection(a: (i64, u16), b: (i64, u16), line: bool, p: (i64, u16)) -> bool {
    let (s, e) = if a <= b { (a, b) } else { (b, a) };
    if line {
        p.0 >= s.0 && p.0 <= e.0
    } else {
        p >= s && p <= e
    }
}

/// Retrieve the text of a single row from the screen as displayed.
pub fn row_text(screen: &vt100::Screen, row: u16) -> String {
    let width = screen.size().1;
    screen.rows(0, width).nth(row as usize).unwrap_or_default()
}

/// Does `row` continue onto the next row for text reading/search?
/// True if vt100 marked it soft-wrapped, or if (on Windows) its display width fills the pane width.
pub fn row_continues(screen: &vt100::Screen, row: u16, row_text: &str) -> bool {
    screen.row_wrapped(row)
        || (HARD_WRAP_AT_WIDTH && UnicodeWidthStr::width(row_text) >= usize::from(screen.size().1))
}

/// A rule row is one where all non-space cells are the same character, or all non-space cells
/// are box-drawing (U+2500..=U+257F), block elements (U+2580..=U+259F), or '-', '=', '_', '─'.
fn is_rule_row(text: &str) -> bool {
    let non_space: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if non_space.is_empty() {
        return true;
    }
    if non_space.iter().all(|&c| c == non_space[0]) {
        return true;
    }
    non_space.iter().all(|&c| {
        matches!(
            c,
            '\u{2500}'..='\u{257f}' | '\u{2580}'..='\u{259f}' | '-' | '=' | '_'
        )
    })
}

fn next_row_first_cell_non_blank(screen: &vt100::Screen, next_row: u16) -> bool {
    screen
        .cell(next_row, 0)
        .is_some_and(|cell| !cell.is_wide_continuation() && !cell.contents().trim().is_empty())
}

/// Stricter continuation predicate for copy mode.
/// Soft-wrap flag OR (HARD_WRAP_AT_WIDTH && full width && row is NOT a rule row && next row's first cell is non-blank).
pub fn copy_row_continues(screen: &vt100::Screen, row: u16, row_text: &str) -> bool {
    if screen.row_wrapped(row) {
        return true;
    }
    if !HARD_WRAP_AT_WIDTH {
        return false;
    }
    let width = usize::from(screen.size().1);
    if UnicodeWidthStr::width(row_text) < width || row_text.trim().is_empty() {
        return false;
    }
    if is_rule_row(row_text) {
        return false;
    }
    next_row_first_cell_non_blank(screen, row.saturating_add(1))
}

/// Extract text between two absolute positions (inclusive).
pub fn extract<CB: vt100::Callbacks>(
    parser: &mut vt100::Parser<CB>,
    a: (i64, u16),
    b: (i64, u16),
    line: bool,
) -> String {
    extract_opts(parser, a, b, line, false)
}

/// Extract text between two absolute positions (inclusive), joining rows into a single line.
pub fn extract_joined<CB: vt100::Callbacks>(
    parser: &mut vt100::Parser<CB>,
    a: (i64, u16),
    b: (i64, u16),
    line: bool,
) -> String {
    extract_opts(parser, a, b, line, true)
}

fn extract_opts<CB: vt100::Callbacks>(
    parser: &mut vt100::Parser<CB>,
    a: (i64, u16),
    b: (i64, u16),
    line: bool,
    join: bool,
) -> String {
    let (s, e) = if a <= b { (a, b) } else { (b, a) };
    let saved = parser.screen().scrollback();
    let (rows, cols) = parser.screen().size();
    let mut raw_rows = Vec::new();
    let mut continues = Vec::new();

    for r in s.0..=e.0 {
        let off = if r < 0 { (-r) as usize } else { 0 };
        parser.screen_mut().set_scrollback(off);
        if parser.screen().scrollback() != off {
            continue;
        }
        let v = (r + off as i64) as u16;
        if v >= rows {
            continue;
        }
        let c0 = if r == s.0 && !line { s.1 } else { 0 };
        let c1 = if r == e.0 && !line {
            (e.1 + 1).min(cols)
        } else {
            cols
        };
        let mut text = String::new();
        for c in c0..c1 {
            if let Some(cell) = parser.screen().cell(v, c) {
                if cell.is_wide_continuation() {
                    continue;
                }
                let t = cell.contents();
                text.push_str(if t.is_empty() { " " } else { t });
            }
        }
        let full_text = row_text(parser.screen(), v);
        let cont = copy_row_continues(parser.screen(), v, &full_text);
        raw_rows.push(text);
        continues.push(cont);
    }
    parser.screen_mut().set_scrollback(saved);

    if join {
        let mut out = String::new();
        let mut in_continuation = false;
        let n = raw_rows.len();

        for (i, (text, cont)) in raw_rows.into_iter().zip(continues).enumerate() {
            let is_last = i + 1 == n;
            let continues_next = cont && !is_last;

            if in_continuation {
                if continues_next {
                    out.push_str(&text);
                } else {
                    out.push_str(text.trim_end());
                }
            } else {
                let s = if i == 0 {
                    text.as_str()
                } else {
                    text.trim_start()
                };
                let s = if continues_next { s } else { s.trim_end() };
                if !s.is_empty() {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(s);
                }
            }
            in_continuation = continues_next;
        }
        out
    } else {
        let mut out = String::new();
        let n = raw_rows.len();
        for (i, (text, cont)) in raw_rows.into_iter().zip(continues).enumerate() {
            let is_last = i + 1 == n;
            if cont && !is_last {
                out.push_str(&text);
            } else {
                out.push_str(text.trim_end());
                if !is_last || line {
                    out.push('\n');
                }
            }
        }
        out
    }
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if ch.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if ch.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn selection_and_extract() {
        assert!(in_selection((0, 5), (1, 2), false, (0, 7)));
        assert!(!in_selection((0, 5), (1, 2), false, (1, 3)));
        assert!(in_selection((1, 2), (0, 5), true, (1, 9)));
        let mut p = vt100::Parser::new(3, 10, 100);
        p.process(b"line1\r\nline2\r\nline3\r\nline4\r\nhello");
        // live screen: line3, line4, hello ; line1,line2 in scrollback
        assert_eq!(extract(&mut p, (0, 0), (0, 4), false), "line3");
        assert_eq!(extract(&mut p, (-2, 2), (0, 1), false), "ne1\nline2\nli");
        assert_eq!(extract(&mut p, (-1, 3), (-1, 3), true), "line2\n");
        assert_eq!(p.screen().scrollback(), 0);
    }

    #[test]
    fn extract_soft_wrap_joins_without_newline() {
        let mut p = vt100::Parser::new(3, 10, 100);
        p.process(b"abcdefghijklmno\r\n");
        assert!(p.screen().row_wrapped(0));
        assert_eq!(extract(&mut p, (0, 0), (1, 4), false), "abcdefghijklmno");
    }

    #[test]
    fn extract_full_width_without_wrap_flag_conpty() {
        let mut p = vt100::Parser::new(3, 10, 100);
        // ConPTY emits explicit \r\n at row end instead of soft wrap
        p.process(b"abcdefghij\r\nklmno\r\n");
        assert!(!p.screen().row_wrapped(0));
        let text = extract(&mut p, (0, 0), (1, 4), false);
        if HARD_WRAP_AT_WIDTH {
            assert_eq!(text, "abcdefghijklmno");
        } else {
            assert_eq!(text, "abcdefghij\nklmno");
        }
    }

    #[test]
    fn extract_short_row_followed_by_indented_row() {
        let mut p = vt100::Parser::new(3, 15, 100);
        p.process(b"cmd --flag\r\n  --subflag\r\n");
        // Plain copy must keep the newline and indentation
        assert_eq!(
            extract(&mut p, (0, 0), (1, 10), false),
            "cmd --flag\n  --subflag"
        );
        // Joined copy must join rows with a single space and strip continuation row indentation
        assert_eq!(
            extract_joined(&mut p, (0, 0), (1, 10), false),
            "cmd --flag --subflag"
        );
    }

    #[test]
    fn extract_wide_cjk_at_row_end() {
        let mut p = vt100::Parser::new(3, 10, 100);
        // 8 ASCII chars + 1 CJK char (width 2) = 10 cols
        p.process("12345678你\r\nabc\r\n".as_bytes());
        let text = extract(&mut p, (0, 0), (1, 2), false);
        if HARD_WRAP_AT_WIDTH {
            assert_eq!(text, "12345678你abc");
        } else {
            assert_eq!(text, "12345678你\nabc");
        }
    }

    #[test]
    fn extract_selection_ending_mid_row() {
        let mut p = vt100::Parser::new(3, 10, 100);
        p.process(b"0123456789\r\nabcdefghij\r\n");
        // Selection ends mid-row at (1, 3) -> "abcd"
        let text = extract(&mut p, (0, 0), (1, 3), false);
        if HARD_WRAP_AT_WIDTH {
            assert_eq!(text, "0123456789abcd");
        } else {
            assert_eq!(text, "0123456789\nabcd");
        }

        // Selection starts mid-row at (0, 5) -> "56789" and ends at (1, 3) -> "abcd"
        let text_mid_start = extract(&mut p, (0, 5), (1, 3), false);
        if HARD_WRAP_AT_WIDTH {
            assert_eq!(text_mid_start, "56789abcd");
        } else {
            assert_eq!(text_mid_start, "56789\nabcd");
        }
    }

    #[test]
    fn extract_joined_soft_wrap_long_url() {
        let mut p = vt100::Parser::new(3, 40, 100);
        let url = "https://example.com/very/long/url/that/spans/multiple/lines/and/should/not/have/any/spaces/inserted";
        p.process(url.as_bytes());
        assert!(p.screen().row_wrapped(0));
        assert!(p.screen().row_wrapped(1));
        let text = extract_joined(&mut p, (0, 0), (2, (url.len() - 80 - 1) as u16), false);
        assert_eq!(text, url);
    }

    #[test]
    fn extract_joined_conpty_full_width_long_url() {
        let mut p = vt100::Parser::new(4, 20, 100);
        // ConPTY wraps 45 chars across 20-col rows with explicit \r\n:
        // row 0 (20): https://example.com/
        // row 1 (20): path/to/resource/123
        // row 2 (5):  45678
        p.process(b"https://example.com/\r\npath/to/resource/123\r\n45678\r\n");
        assert!(!p.screen().row_wrapped(0));
        assert!(!p.screen().row_wrapped(1));
        let text = extract_joined(&mut p, (0, 0), (2, 4), false);
        if HARD_WRAP_AT_WIDTH {
            assert_eq!(text, "https://example.com/path/to/resource/12345678");
        } else {
            assert_eq!(text, "https://example.com/ path/to/resource/123 45678");
        }
    }

    #[test]
    fn extract_plain_full_width_rule_row_keeps_newline() {
        let mut p = vt100::Parser::new(5, 10, 100);
        p.process(b"----------\r\ntext 1\r\n");
        p.process("┌────────┐\r\ntext 2\r\n".as_bytes());
        let text = extract(&mut p, (0, 0), (3, 5), false);
        assert_eq!(text, "----------\ntext 1\n┌────────┐\ntext 2");
    }

    #[test]
    fn extract_plain_full_width_text_row_followed_by_text_joins() {
        let mut p = vt100::Parser::new(3, 10, 100);
        p.process(b"abcdefghij\r\nklmno\r\n");
        let text = extract(&mut p, (0, 0), (1, 4), false);
        if HARD_WRAP_AT_WIDTH {
            assert_eq!(text, "abcdefghijklmno");
        } else {
            assert_eq!(text, "abcdefghij\nklmno");
        }
    }

    #[test]
    fn extract_plain_full_width_text_followed_by_indented_row_keeps_newline() {
        let mut p = vt100::Parser::new(3, 10, 100);
        p.process(b"abcdefghij\r\n  klmno\r\n");
        let text = extract(&mut p, (0, 0), (1, 6), false);
        assert_eq!(text, "abcdefghij\n  klmno");
    }
}
