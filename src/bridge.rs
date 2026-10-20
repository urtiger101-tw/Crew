//! Newline JSON bridge for local automation. Native daemon framing stays unchanged.
use crate::api::{ApiError, ApiResult, Reply};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

const MAX_LINE: usize = 1024 * 1024;
const MAX_PENDING: usize = 16;
pub const HELP: &str = "crew api serve   Newline JSON on stdin/stdout; replies carry the request id\ncrew api schema  Describe supported methods and the params.args array\ncrew events list|wait [--after N] [--epoch E] [--limit N] [--timeout MS]";

const METHODS: &[&str] = &[
    "api.snapshot",
    "session.snapshot",
    "session.list",
    "server.stop",
    "workspace.list",
    "workspace.get",
    "workspace.current",
    "workspace.create",
    "workspace.rename",
    "workspace.focus",
    "workspace.close",
    "tab.list",
    "tab.get",
    "tab.current",
    "tab.create",
    "tab.rename",
    "tab.focus",
    "tab.close",
    "worktree.list",
    "worktree.create",
    "worktree.open",
    "worktree.remove",
    "pane.list",
    "pane.get",
    "pane.current",
    "pane.split",
    "pane.focus",
    "pane.zoom",
    "pane.rename",
    "pane.close",
    "pane.read",
    "pane.send-text",
    "pane.send-keys",
    "pane.run",
    "pane.wait-output",
    "pane.report-agent",
    "pane.release-agent",
    "agent.list",
    "agent.get",
    "agent.read",
    "agent.send-keys",
    "agent.prompt",
    "agent.wait",
    "agent.rename",
    "agent.focus",
    "events.list",
    "events.wait",
];

pub fn schema() -> Value {
    json!({"protocol": "crew-json-lines-v1", "version": env!("CARGO_PKG_VERSION"),
        "methods": METHODS, "local_methods": ["ping", "api.schema"],
        "request": {"id": "string or integer", "method": "listed method", "params": {"args": ["CLI argument", "..."]}},
        "reply": {"id": "request id", "result": "JSON value or text string", "error": "code/message on failure only"},
        "max_line_bytes": MAX_LINE, "max_pending": MAX_PENDING,
        "ordering": "concurrent; correlate by id; wait requests should use a timeout",
        "compatibility": "Crew methods use CLI argument arrays; this is not the Herdr socket protocol"})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    id: Value,
    method: String,
    #[serde(default)]
    params: Params,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    #[serde(default)]
    args: Vec<String>,
}

fn execute(request: Envelope) -> ApiResult {
    if matches!(request.method.as_str(), "ping" | "api.schema") {
        if !request.params.args.is_empty() {
            return Err(ApiError::invalid("local methods take no arguments"));
        }
        return Ok(Reply::Json(if request.method == "ping" {
            json!({"bridge": true, "version": env!("CARGO_PKG_VERSION")})
        } else {
            schema()
        }));
    }
    if !METHODS.contains(&request.method.as_str()) {
        return Err(ApiError::new(
            "method_not_found",
            "method is not listed by crew api schema",
        ));
    }
    let method = if request.method == "session.snapshot" {
        "api.snapshot"
    } else {
        &request.method
    };
    let (group, sub) = method.split_once('.').unwrap();
    let mut args = vec![group.to_string(), sub.to_string()];
    args.extend(request.params.args);
    let current = crate::brand::env_var("PANE_ID")
        .ok()
        .and_then(|p| crate::api::parse_pane_id(&p));
    let req = crate::api::cli::parse(&args, current).map_err(ApiError::invalid)?;
    crate::client::request(req)
}

fn response(id: Value, result: ApiResult) -> Value {
    match result {
        Ok(Reply::Json(value)) => json!({"id": id, "result": value}),
        Ok(Reply::Text(text)) => json!({"id": id, "result": text}),
        Err(error) => json!({"id": id, "error": error}),
    }
}
fn parse(line: &[u8]) -> Result<Envelope, ApiError> {
    let req: Envelope = serde_json::from_slice(line)
        .map_err(|e| ApiError::new("invalid_request", e.to_string()))?;
    if !(req.id.is_string() || req.id.is_i64() || req.id.is_u64()) {
        return Err(ApiError::new(
            "invalid_request",
            "id must be a string or integer",
        ));
    }
    Ok(req)
}
fn send(output: &Mutex<std::io::Stdout>, value: &Value) -> std::io::Result<()> {
    let mut out = output.lock().unwrap_or_else(|e| e.into_inner());
    serde_json::to_writer(&mut *out, value)?;
    writeln!(out)?;
    out.flush()
}

/// Bound memory and concurrent requests; a long wait does not block subsequent commands.
pub fn serve() -> anyhow::Result<()> {
    let output = Arc::new(Mutex::new(std::io::stdout()));
    let pending = Arc::new(AtomicUsize::new(0));
    let mut input = std::io::stdin().lock();
    let mut workers = Vec::new();
    loop {
        let mut line = Vec::new();
        let mut oversized = false;
        loop {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                break;
            }
            let length = buffer
                .iter()
                .position(|&b| b == b'\n')
                .map_or(buffer.len(), |n| n + 1);
            let done = buffer[length - 1] == b'\n';
            if line.len() + length > MAX_LINE {
                oversized = true;
            }
            if !oversized {
                line.extend_from_slice(&buffer[..length]);
            }
            input.consume(length);
            if done {
                break;
            }
        }
        if oversized {
            send(
                &output,
                &response(
                    Value::Null,
                    Err(ApiError::new("request_too_large", "maximum line is 1 MiB")),
                ),
            )?;
            continue;
        }
        if line.is_empty() {
            break;
        }
        let request = match parse(&line) {
            Ok(r) => r,
            Err(e) => {
                send(&output, &response(Value::Null, Err(e)))?;
                continue;
            }
        };
        workers.retain(|w: &std::thread::JoinHandle<()>| !w.is_finished());
        if pending.load(Ordering::SeqCst) >= MAX_PENDING {
            send(
                &output,
                &response(
                    request.id,
                    Err(ApiError::new("busy", "16 requests are already in flight")),
                ),
            )?;
            continue;
        }
        pending.fetch_add(1, Ordering::SeqCst);
        let output = output.clone();
        let pending = pending.clone();
        workers.push(std::thread::spawn(move || {
            let id = request.id.clone();
            let result = std::panic::catch_unwind(|| execute(request))
                .unwrap_or_else(|_| Err(ApiError::new("internal", "request handler failed")));
            let _ = send(&output, &response(id, result));
            pending.fetch_sub(1, Ordering::SeqCst);
        }));
    }
    for worker in workers {
        let _ = worker.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_envelope_rejects_accidental_api_mismatches() {
        for value in [
            json!({"id": null,"method":"ping"}),
            json!({"id":1,"method":"ping","params":{"cwd":"/tmp"}}),
            json!({"id":1,"method":"ping","params":{"args":[1]}}),
        ] {
            assert!(parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(parse(br#"{"id":"hello","method":"ping"}"#).is_ok());
    }
    #[test]
    fn text_and_errors_preserve_id_without_stdout_noise() {
        assert_eq!(
            response(json!(7), Ok(Reply::Text("a\nb".into()))),
            json!({"id":7,"result":"a\nb"})
        );
        let reply = response(json!("x"), Err(ApiError::new("timeout", "expired")));
        assert_eq!(reply["id"], "x");
        assert_eq!(reply["error"]["code"], "timeout");
        assert!(reply.get("result").is_none());
    }
}
