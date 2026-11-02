//! Paste once, settle without the state lock, send one Enter, and report display evidence.
use super::{ensure_pane, read_text};
use crate::api::{ApiError, ApiResult, ReadSource, Reply, lock};
use crate::server::{Shared, State};
use serde_json::json;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

const DEFAULT_SETTLE_MS: u64 = 2000;
const MAX_SETTLE_MS: u64 = 10_000;
const OBSERVE_MS: u64 = 1000;

#[derive(Debug)]
struct Composer {
    row: u16,
    marker: char,
}

/// Only exact, single-row Codex/Claude-style composers with the cursor after the pasted text.
/// This is a display heuristic; it cannot prove that an agent accepted or executed anything.
fn composer_before(screen: &str, cursor: (u16, u16), text: &str) -> Option<Composer> {
    if text.trim().is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    let line = screen.lines().nth(cursor.0 as usize)?.trim_end();
    let marker = line.chars().next()?;
    if !matches!(marker, '›' | '❯') || line != format!("{marker} {text}") {
        return None;
    }
    if usize::from(cursor.1) != UnicodeWidthStr::width(line) {
        return None;
    }
    Some(Composer {
        row: cursor.0,
        marker,
    })
}

fn composer_cleared(screen: &str, cursor: (u16, u16), before: &Composer) -> bool {
    if cursor.0 != before.row {
        return false;
    }
    let Some(line) = screen.lines().nth(before.row as usize) else {
        return false;
    };
    let line = line.trim_end();
    let empty = line == before.marker.to_string()
        || (before.marker == '›' && line == "› Ask Codex to do anything");
    empty && cursor.1 == 2
}

fn capture(st: &mut State, pane: usize) -> Result<(String, (u16, u16)), ApiError> {
    let text = read_text(st, pane, ReadSource::Visible, None)?;
    let cursor = st.panes[&pane].parser.screen().cursor_position();
    Ok((text, cursor))
}

fn result(status: &str, stage: &str, evidence: &str) -> ApiResult {
    Ok(Reply::Json(
        json!({"status": status, "stage": stage, "evidence": evidence}),
    ))
}

pub(super) fn submit_checked(
    shared: &Shared,
    pane: usize,
    text: &str,
    settle_ms: Option<u64>,
) -> ApiResult {
    let settle_ms = settle_ms.unwrap_or(DEFAULT_SETTLE_MS);
    if settle_ms > MAX_SETTLE_MS {
        return Err(ApiError::invalid("settle_ms must be in 0..=10000"));
    }
    let agent_instance = {
        let mut st = lock(shared);
        ensure_pane(&st, pane)?;
        let instance = st.panes[&pane].agent.as_ref().map(|a| a.instance);
        let wrote = st.paste_checked(pane, text).is_ok();
        st.mark_dirty();
        if !wrote {
            return result("unconfirmed", "paste_attempted", "paste_write_failed");
        }
        instance
    };
    std::thread::sleep(Duration::from_millis(settle_ms));
    let before = {
        let mut st = lock(shared);
        let Some(p) = st.panes.get(&pane) else {
            return result("unconfirmed", "pasted", "pane_closed_before_enter");
        };
        if agent_instance.is_some() && p.agent.as_ref().map(|a| a.instance) != agent_instance {
            return result("unconfirmed", "pasted", "agent_changed_before_enter");
        }
        let before = capture(&mut st, pane)
            .ok()
            .and_then(|(screen, cursor)| composer_before(&screen, cursor, text));
        let wrote = st.write_pane_checked(pane, b"\r").is_ok();
        st.mark_dirty();
        if !wrote {
            return result("unconfirmed", "enter_attempted", "enter_write_failed");
        }
        before
    };
    let Some(before) = before else {
        return result(
            "unconfirmed",
            "enter_attempted",
            "composer_not_recognized_before_enter",
        );
    };
    let deadline = Instant::now() + Duration::from_millis(OBSERVE_MS);
    loop {
        {
            let mut st = lock(shared);
            let Some(p) = st.panes.get(&pane) else {
                return result("unconfirmed", "enter_attempted", "pane_closed_after_enter");
            };
            if agent_instance.is_some() && p.agent.as_ref().map(|a| a.instance) != agent_instance {
                return result(
                    "unconfirmed",
                    "enter_attempted",
                    "agent_changed_after_enter",
                );
            }
            if let Ok((screen, cursor)) = capture(&mut st, pane)
                && composer_cleared(&screen, cursor, &before)
            {
                return result(
                    "submitted",
                    "enter_attempted",
                    "composer_cleared_display_heuristic",
                );
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return result(
                "unconfirmed",
                "enter_attempted",
                "composer_clear_not_observed",
            );
        }
        std::thread::sleep(Duration::from_millis(30).min(deadline - now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_clear_requires_exact_pre_and_post_cursor_row() {
        let before = composer_before("old message\n› hello\n", (1, 7), "hello").unwrap();
        assert!(composer_cleared("old message\n›\n", (1, 2), &before));
        assert!(composer_cleared(
            "old message\n› Ask Codex to do anything\n",
            (1, 2),
            &before
        ));
        assert!(!composer_cleared("old message\n› hello\n", (1, 7), &before));
        assert!(!composer_cleared("›\nold message\n", (0, 2), &before));
        assert!(composer_before("› drafthello\n", (0, 12), "hello").is_none());
        assert!(composer_before("› hello\n", (0, 7), "hello\nnext").is_none());
        assert!(composer_before("historical hello\n›\n", (1, 2), "hello").is_none());
    }
    fn fixture(swallow: bool) {
        use std::io::{Read, Write};
        crossterm::terminal::enable_raw_mode().unwrap();
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Console::{
                ENABLE_VIRTUAL_TERMINAL_INPUT, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE,
                SetConsoleMode,
            };
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            assert_ne!(GetConsoleMode(handle, &mut mode), 0);
            assert_ne!(
                SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_INPUT),
                0
            );
        }
        let mut out = std::io::stdout();
        write!(out, "\x1b[?2004h\x1b[2J\x1b[H› ").unwrap();
        out.flush().unwrap();
        let mut input = std::io::stdin().lock();
        let mut pending = Vec::new();
        let mut count = 0;
        loop {
            let mut byte = [0];
            if input.read_exact(&mut byte).is_err() {
                break;
            }
            pending.push(byte[0]);
            if cfg!(windows) && byte[0] != b'\r' {
                write!(out, "\r\x1b[2K› {}", String::from_utf8_lossy(&pending)).unwrap();
                out.flush().unwrap();
            } else if pending.ends_with(b"\x1b[201~") {
                let payload = &pending[6..pending.len() - 6];
                write!(out, "\r\x1b[2K› {}", String::from_utf8_lossy(payload)).unwrap();
                out.flush().unwrap();
                pending.clear();
            } else if byte[0] == b'\r' {
                count += 1;
                if !swallow {
                    write!(out, "\r\x1b[2K› ").unwrap();
                }
                let cursor = if swallow { 8 } else { 3 };
                write!(out, "\x1b[2;1HENTERS={count}\x1b[1;{cursor}H").unwrap();
                out.flush().unwrap();
                pending.clear();
            }
        }
    }

    #[test]
    #[ignore = "subprocess PTY fixture"]
    fn fixture_clear() {
        fixture(false);
    }

    #[test]
    #[ignore = "subprocess PTY fixture"]
    fn fixture_swallow() {
        fixture(true);
    }

    struct RecordingWriter {
        inner: Box<dyn std::io::Write + Send>,
        bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl std::io::Write for RecordingWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            let count = self.inner.write(data)?;
            self.bytes.lock().unwrap().extend_from_slice(&data[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    fn spawn_fixture(name: &str) -> (Shared, usize, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let mut st = crate::server::test_state();
        let executable = std::env::current_exe().unwrap();
        st.cfg.shell = Some(format!(
            "\"{}\" --ignored --nocapture --exact api::pane::submit::tests::{name}",
            executable.to_string_lossy().replace('\\', "/")
        ));
        let shared = std::sync::Arc::new(std::sync::Mutex::new(st));
        let pane = lock(&shared).spawn_pane(&shared, 10, 80, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            {
                let st = lock(&shared);
                let screen = st.panes[&pane].parser.screen();
                if screen.bracketed_paste() && screen.contents().contains('›') {
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "fixture did not enable bracketed paste"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut st = lock(&shared);
        let writer = std::mem::replace(
            &mut st.panes.get_mut(&pane).unwrap().writer,
            Box::new(std::io::sink()),
        );
        st.panes.get_mut(&pane).unwrap().writer = Box::new(RecordingWriter {
            inner: writer,
            bytes: bytes.clone(),
        });
        drop(st);
        (shared, pane, bytes)
    }

    #[test]
    fn bracketed_composer_clear_and_swallowed_enter() {
        for (fixture_name, expected) in [
            ("fixture_clear", "submitted"),
            ("fixture_swallow", "unconfirmed"),
        ] {
            let (shared, pane, bytes) = spawn_fixture(fixture_name);
            let Reply::Json(reply) = crate::api::handle(
                &shared,
                crate::api::Request::PaneSubmit {
                    pane,
                    text: "hello".into(),
                    settle_ms: Some(250),
                },
            )
            .unwrap() else {
                panic!("expected json")
            };
            assert_eq!(reply["status"], expected, "{reply}");
            assert_eq!(*bytes.lock().unwrap(), b"\x1b[200~hello\x1b[201~\r");
            let mut st = lock(&shared);
            let screen = read_text(&mut st, pane, ReadSource::Visible, None).unwrap();
            assert!(screen.contains("ENTERS=1"), "{screen}");
            assert!(!screen.contains("ENTERS=2"), "{screen}");
            let _ = st.panes.get_mut(&pane).unwrap().child.kill();
        }
    }

    struct FailOnWrite {
        inner: Box<dyn std::io::Write + Send>,
        remaining: usize,
    }
    impl std::io::Write for FailOnWrite {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Err(std::io::Error::other("fixture write failed"));
            }
            self.remaining -= 1;
            self.inner.write(data)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    #[test]
    fn write_failures_report_ambiguous_delivery_and_never_retry() {
        for (successful_writes, expected, expected_bytes) in [
            (0, "paste_write_failed", b"".as_slice()),
            (
                1,
                "enter_write_failed",
                b"\x1b[200~hello\x1b[201~".as_slice(),
            ),
        ] {
            let (shared, pane, bytes) = spawn_fixture("fixture_clear");
            {
                let mut st = lock(&shared);
                let p = st.panes.get_mut(&pane).unwrap();
                let writer = std::mem::replace(&mut p.writer, Box::new(std::io::sink()));
                p.writer = Box::new(FailOnWrite {
                    inner: writer,
                    remaining: successful_writes,
                });
            }
            let Reply::Json(reply) = submit_checked(&shared, pane, "hello", Some(250)).unwrap()
            else {
                panic!("expected json")
            };
            assert_eq!(reply["status"], "unconfirmed");
            assert_eq!(reply["evidence"], expected);
            assert_eq!(*bytes.lock().unwrap(), expected_bytes);
            let _ = lock(&shared).panes.get_mut(&pane).unwrap().child.kill();
        }
    }

    #[test]
    fn closing_pane_after_paste_cancels_enter_without_holding_lock() {
        let (shared, pane, bytes) = spawn_fixture("fixture_clear");
        let worker_shared = shared.clone();
        let worker = std::thread::spawn(move || {
            submit_checked(&worker_shared, pane, "hello", Some(1000)).unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while bytes.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "paste did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut removed = lock(&shared).panes.remove(&pane).unwrap();
        let _ = removed.child.kill();
        let Reply::Json(reply) = worker.join().unwrap() else {
            panic!("expected json")
        };
        assert_eq!(reply["status"], "unconfirmed");
        assert_eq!(reply["evidence"], "pane_closed_before_enter");
        assert_eq!(*bytes.lock().unwrap(), b"\x1b[200~hello\x1b[201~");
    }

    #[test]
    fn daemon_rejects_out_of_range_before_paste() {
        let (shared, pane, bytes) = spawn_fixture("fixture_clear");
        assert!(submit_checked(&shared, pane, "hello", Some(10001)).is_err());
        assert!(bytes.lock().unwrap().is_empty());
        let _ = lock(&shared).panes.get_mut(&pane).unwrap().child.kill();
    }

    #[test]
    fn plain_shell_fallback_delivers_without_bracketed_mode() {
        let mut st = crate::server::test_state();
        st.cfg.shell = Some(
            if cfg!(windows) {
                "cmd.exe /Q"
            } else {
                "/bin/sh"
            }
            .into(),
        );
        let shared = std::sync::Arc::new(std::sync::Mutex::new(st));
        let pane = lock(&shared).spawn_pane(&shared, 12, 100, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if !lock(&shared).panes[&pane]
                .parser
                .screen()
                .contents()
                .trim()
                .is_empty()
            {
                break;
            }
            assert!(Instant::now() < deadline, "shell did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        let Reply::Json(reply) =
            submit_checked(&shared, pane, "echo B1_SHELL_OK", Some(100)).unwrap()
        else {
            panic!("expected json")
        };
        assert_eq!(reply["status"], "unconfirmed");
        loop {
            let mut st = lock(&shared);
            let screen = read_text(&mut st, pane, ReadSource::Visible, None).unwrap();
            if screen.lines().any(|line| line.trim() == "B1_SHELL_OK") {
                let _ = st.panes.get_mut(&pane).unwrap().child.kill();
                break;
            }
            assert!(
                Instant::now() < deadline,
                "shell did not echo marker: {screen}"
            );
            drop(st);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
