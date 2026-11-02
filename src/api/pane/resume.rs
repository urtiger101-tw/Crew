//! Explicit restart recovery. A resume request submits the saved argv at most once.
use crate::api::{ApiError, ApiResult, Reply, lock, pane_id_str, pane_json};
use crate::server::{Shared, State};
use serde_json::json;

/// Shared by the JSON API and the focused pane's keyboard action. The state lock
/// owns both the duplicate check and the single write ending in Enter.
pub fn resume_pane(st: &mut State, pane: usize) -> Result<bool, ApiError> {
    let shell = crate::commands::tokenize(&st.cfg.default_shell());
    let program = shell.first().map(String::as_str).unwrap_or("sh");
    if program.rsplit(['/', '\\']).next().is_some_and(|name| {
        name.eq_ignore_ascii_case("cmd") || name.eq_ignore_ascii_case("cmd.exe")
    }) {
        return Err(ApiError::new(
            "resume_unavailable",
            "saved resume commands require PowerShell or a POSIX shell; cmd.exe is unsupported",
        ));
    }
    let p = st
        .panes
        .get_mut(&pane)
        .ok_or_else(|| ApiError::pane_not_found(pane))?;
    let agent = p
        .agent
        .as_ref()
        .filter(|agent| agent.source == crate::api::RESTORE_SOURCE)
        .ok_or_else(|| {
            ApiError::new(
                "resume_unavailable",
                "this pane has no restored agent to resume",
            )
        })?;
    if p.resume_requested {
        return Ok(false);
    }
    if p.detected_agent.is_some() {
        return Err(ApiError::new(
            "agent_already_running",
            "the pane already has a detected agent; no resume command was sent",
        ));
    }
    let line = crate::persist::quote_resume(&agent.resume).ok_or_else(|| {
        ApiError::new(
            "resume_unavailable",
            "no supported saved resume argv; inspect pane get and start the agent manually",
        )
    })?;
    let mut bytes = Vec::new();
    if p.parser.screen().bracketed_paste() {
        bytes.extend_from_slice(b"\x1b[200~");
    }
    bytes.extend_from_slice(line.as_bytes());
    if p.parser.screen().bracketed_paste() {
        bytes.extend_from_slice(b"\x1b[201~");
    }
    bytes.push(b'\r');
    p.writer
        .write_all(&bytes)
        .map_err(|error| ApiError::new("resume_failed", error.to_string()))?;
    // A completed write includes Enter; keep the duplicate guard even if flush fails.
    p.resume_requested = true;
    if let Some(agent) = p.agent.as_mut() {
        agent.message = Some("resume requested; waiting for a live agent report".into());
    }
    let flushed = p.writer.flush();
    st.events.record(
        "pane.resume_requested",
        Some(pane),
        json!({"pane_id": pane_id_str(pane)}),
    );
    st.mark_dirty();
    flushed.map_err(|error| ApiError::new("resume_failed", error.to_string()))?;
    Ok(true)
}

pub fn resume(shared: &Shared, pane: usize) -> ApiResult {
    let mut st = lock(shared);
    let requested = resume_pane(&mut st, pane)?;
    Ok(Reply::Json(json!({
        "resume_status": if requested { "requested" } else { "already-requested" },
        "pane": pane_json(&st, pane),
    })))
}
