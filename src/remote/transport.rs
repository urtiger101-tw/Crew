//! Single-request SSH transport. API arguments only travel over stdin as JSON.
use super::store::Machine;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

pub const PROTOCOL: &str = "crew-ssh-v1";
const MAX_REQUEST: usize = 1024 * 1024;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_STDERR: usize = 64 * 1024;
pub const DEFAULT_TIMEOUT: u64 = 15_000;

#[derive(Clone)]
pub struct Budget {
    timeout_ms: u64,
    cancelled: Arc<AtomicBool>,
}

impl Budget {
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            timeout_ms,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl From<u64> for Budget {
    fn from(timeout_ms: u64) -> Self {
        Self::new(timeout_ms)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    protocol: String,
    socket: Option<String>,
    args: Vec<String>,
    summary: bool,
}

impl Request {
    pub fn command(args: Vec<String>, socket: Option<String>) -> Self {
        Self {
            protocol: PROTOCOL.into(),
            socket,
            args,
            summary: false,
        }
    }
    pub fn summary(socket: Option<String>) -> Self {
        Self {
            protocol: PROTOCOL.into(),
            socket,
            args: Vec::new(),
            summary: true,
        }
    }
}

/// POSIX remote shell quoting applies only to a saved binary path and fixed helper name.
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub fn ssh(machine: &Machine, timeout_ms: u64, tty: bool, interactive: bool) -> Command {
    let mut command = Command::new("ssh");
    if let Some(path) = &machine.ssh_config {
        command.arg("-F").arg(path);
    }
    command.arg(if tty { "-tt" } else { "-T" });
    if !interactive {
        command.arg("-oBatchMode=yes");
    }
    command
        .arg(format!(
            "-oConnectTimeout={}",
            (timeout_ms / 1000).clamp(1, 30)
        ))
        .arg("-oConnectionAttempts=1")
        .arg("-oServerAliveInterval=5")
        .arg("-oServerAliveCountMax=2")
        .arg("-oClearAllForwardings=yes")
        .arg("--")
        .arg(&machine.destination);
    command
}

pub fn remote(
    machine: &Machine,
    args: &[String],
    summary: bool,
    budget: impl Into<Budget>,
) -> Result<Value> {
    let budget = budget.into();
    machine.validate()?;
    let mut command = ssh(machine, budget.timeout_ms, false, false);
    command.arg(format!("exec {} '__remote-api'", quote(&machine.binary)));
    let request = if summary {
        Request::summary(machine.socket.clone())
    } else {
        Request::command(args.to_vec(), machine.socket.clone())
    };
    invoke(&mut command, &request, budget)
}

/// The local endpoint uses the same bounded subprocess path as SSH. One hung daemon
/// cannot prevent a healthy remote from reporting its result.
pub fn local(args: &[String], summary: bool, budget: impl Into<Budget>) -> Result<Value> {
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("__remote-api");
    let request = if summary {
        Request::summary(None)
    } else {
        Request::command(args.to_vec(), None)
    };
    invoke(&mut command, &request, budget.into())
}

fn invoke(command: &mut Command, request: &Request, budget: Budget) -> Result<Value> {
    let bytes = serde_json::to_vec(request)?;
    if bytes.len() > MAX_REQUEST {
        bail!("remote request exceeds 1 MiB");
    }
    let output = capture(command, &bytes, budget)?;
    let envelope: Value = serde_json::from_slice(&output)
        .context("invalid remote protocol reply; remote must support crew-ssh-v1")?;
    if envelope.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
        bail!("remote protocol mismatch; remote must support crew-ssh-v1");
    }
    if let Some(error) = envelope.get("error") {
        bail!("remote API error: {error}");
    }
    envelope
        .get("result")
        .cloned()
        .context("remote reply has no result")
}

fn reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut chunk = [0u8; 8192];
        let result = loop {
            match reader.read(&mut chunk) {
                Ok(0) => break Ok(output),
                Ok(n) if output.len() + n <= limit => output.extend_from_slice(&chunk[..n]),
                Ok(_) => {
                    overflow.store(true, Ordering::Release);
                    break Err(std::io::Error::other("output limit exceeded"));
                }
                Err(e) => break Err(e),
            }
        };
        let _ = tx.send(result);
    });
    rx
}

fn stop(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        // Each transport has its own group, including an SSH ProxyCommand child.
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Own cleanup from the moment spawning succeeds, including error returns after
/// the direct child exits. A ProxyCommand descendant can still hold a pipe then;
/// terminating the group closes those pipe owners and lets the I/O workers exit.
struct TransportChild(std::process::Child);

impl Drop for TransportChild {
    fn drop(&mut self) {
        stop(&mut self.0);
    }
}

pub(super) fn capture(
    command: &mut Command,
    input: &[u8],
    budget: impl Into<Budget>,
) -> Result<Vec<u8>> {
    let budget = budget.into();
    let timeout_ms = budget.timeout_ms;
    if budget.cancelled.load(Ordering::Acquire) {
        bail!("transport cancelled");
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut owned = TransportChild(
        command
            .spawn()
            .context("start transport (OpenSSH must be installed)")?,
    );
    let child = &mut owned.0;
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = reader(child.stdout.take().unwrap(), MAX_OUTPUT, overflow.clone());
    let stderr = reader(child.stderr.take().unwrap(), MAX_STDERR, overflow.clone());
    let mut stdin = child.stdin.take().unwrap();
    let data = input.to_vec();
    let (sent_tx, sent_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = stdin.write_all(&data).and_then(|_| stdin.flush());
        drop(stdin);
        let _ = sent_tx.send(result);
    });
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let status = loop {
        if budget.cancelled.load(Ordering::Acquire) {
            bail!("transport cancelled");
        }
        if overflow.load(Ordering::Acquire) {
            bail!("transport output limit exceeded");
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            bail!(
                "transport timed out after {timeout_ms} ms; a remote mutation may have completed, so it was not retried"
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // A transport exiting with an inherited pipe still open must not hang the caller.
    let collect = Duration::from_millis(500);
    let out = stdout
        .recv_timeout(collect)
        .context("transport stdout did not close")??;
    let err = stderr
        .recv_timeout(collect)
        .context("transport stderr did not close")??;
    if !status.success() {
        let detail = String::from_utf8_lossy(&err);
        let detail: String = detail
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(2000)
            .collect();
        bail!(
            "SSH/transport exit {}: {}",
            status.code().map_or("signal".into(), |n| n.to_string()),
            detail.trim()
        );
    }
    sent_rx
        .recv_timeout(collect)
        .context("request write did not complete")??;
    Ok(out)
}

fn native_request(req: crate::api::Request, socket: Option<&str>) -> crate::api::ApiResult {
    use crate::api::ApiError;
    use crate::proto::{ClientMsg, Command, ServerMsg, read_msg, write_msg};
    use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Stream, prelude::*};
    let stream = match socket {
        None => crate::client::connect(false, crate::i18n::Lang::En),
        Some(name) => {
            let result = if GenericNamespaced::is_supported() {
                name.to_owned()
                    .to_ns_name::<GenericNamespaced>()
                    .and_then(Stream::connect)
            } else {
                std::env::temp_dir()
                    .join(name)
                    .to_fs_name::<GenericFilePath>()
                    .and_then(Stream::connect)
            };
            result.map_err(anyhow::Error::from)
        }
    }
    .map_err(|e| ApiError::new("server_not_running", e.to_string()))?;
    let (mut reader, mut writer) = stream.split();
    write_msg(&mut writer, &ClientMsg::Hello(Command::Api(req)))
        .map_err(|e| ApiError::new("transport_error", e.to_string()))?;
    match read_msg::<ServerMsg>(&mut reader)
        .map_err(|e| ApiError::new("transport_error", e.to_string()))?
    {
        ServerMsg::Api(result) => result,
        ServerMsg::Error(error) => Err(ApiError::new("server_error", error)),
        _ => Err(ApiError::new(
            "transport_error",
            "unexpected daemon response",
        )),
    }
}

fn execute(request: Request) -> crate::api::ApiResult {
    use crate::api::ApiError;
    if request.protocol != PROTOCOL {
        return Err(ApiError::new("protocol_mismatch", "expected crew-ssh-v1"));
    }
    if let Some(socket) = &request.socket
        && (super::store::validate_text(socket, 160, "remote socket").is_err()
            || socket.contains(['/', '\\']))
    {
        return Err(ApiError::invalid("invalid socket name"));
    }
    if request.summary {
        if !request.args.is_empty() {
            return Err(ApiError::invalid("summary takes no command arguments"));
        }
        return native_request(
            crate::api::Request::FederationSnapshot,
            request.socket.as_deref(),
        );
    }
    // Never inherit a pane id from the SSH login environment.
    let req = crate::api::cli::parse(&request.args, None).map_err(ApiError::invalid)?;
    native_request(req, request.socket.as_deref())
}

pub fn serve() -> Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_REQUEST as u64 + 1)
        .read_to_end(&mut bytes)?;
    let result = if bytes.len() > MAX_REQUEST {
        Err(crate::api::ApiError::new(
            "request_too_large",
            "maximum request is 1 MiB",
        ))
    } else {
        serde_json::from_slice::<Request>(&bytes)
            .map_err(|e| crate::api::ApiError::new("invalid_request", e.to_string()))
            .and_then(execute)
    };
    let reply = match result {
        Ok(crate::api::Reply::Json(value)) => json!({"protocol": PROTOCOL,"result":value}),
        Ok(crate::api::Reply::Text(value)) => json!({"protocol": PROTOCOL,"result":value}),
        Err(error) => json!({"protocol": PROTOCOL,"error":error}),
    };
    println!("{reply}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_is_not_part_of_remote_shell_command() {
        assert_eq!(
            quote("/tmp/a b'$(touch /tmp/pwn)"),
            "'/tmp/a b'\"'\"'$(touch /tmp/pwn)'"
        );
        let request = Request::command(
            vec![
                "pane".into(),
                "run".into(),
                "p1".into(),
                "$(anything); & ' \"".into(),
            ],
            None,
        );
        let encoded = serde_json::to_vec(&request).unwrap();
        let decoded: Request = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.args, request.args);
    }

    #[test]
    fn helper_rejects_protocol_and_unknown_envelope_fields() {
        let request = Request {
            protocol: "other".into(),
            socket: None,
            args: Vec::new(),
            summary: false,
        };
        assert!(execute(request).is_err());
        assert!(
            serde_json::from_str::<Request>(
                r#"{"protocol":"crew-ssh-v1","socket":null,"args":[],"summary":true,"shell":"x"}"#
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn bounded_transport_kills_stalls_and_rejects_excess_output() {
        let start = Instant::now();
        let mut stalled = Command::new("/bin/sh");
        stalled.args(["-c", "sleep 10"]);
        assert!(
            capture(&mut stalled, b"{}", 80_u64)
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        let mut noisy = Command::new("/bin/sh");
        noisy.args(["-c", "yes x"]);
        assert!(capture(&mut noisy, b"{}", 2000_u64).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_a_dashboard_check_stops_its_transport() {
        let budget = Budget::new(10_000);
        let cancel = budget.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            cancel.cancel();
        });
        let start = Instant::now();
        let mut stalled = Command::new("/bin/sh");
        stalled.args(["-c", "sleep 10"]);
        assert_eq!(
            capture(&mut stalled, b"{}", budget)
                .unwrap_err()
                .to_string(),
            "transport cancelled"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exited_transport_cleans_descendants_that_keep_either_output_pipe_open() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        for pipe in ["stdout", "stderr"] {
            let pid_file = std::env::temp_dir().join(format!(
                "crew-transport-pipe-{}-{}-{pipe}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let mut command = Command::new("/bin/sh");
            let script = if pipe == "stdout" {
                "sleep 30 2>/dev/null & printf '%s' $! > \"$1\"; exit 7"
            } else {
                "sleep 30 >/dev/null & printf '%s' $! > \"$1\"; exit 7"
            };
            command.args(["-c", script, "crew-pipe-fixture"]);
            command.arg(&pid_file);
            let start = Instant::now();
            let error = capture(&mut command, b"", 2000_u64).unwrap_err();
            let pid: i32 = fs::read_to_string(&pid_file).unwrap().parse().unwrap();
            fs::remove_file(pid_file).unwrap();
            let alive = || {
                fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        stat.rsplit_once(')')
                            .map(|(_, tail)| tail.trim().to_owned())
                    })
                    .is_some_and(|tail| !tail.starts_with(['Z', 'X']))
            };
            let cleanup_deadline = Instant::now() + Duration::from_millis(500);
            while alive() && Instant::now() < cleanup_deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            // On a regression clean up the fixture before failing the assertion.
            let leaked = alive();
            if leaked {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
            assert_eq!(error.to_string(), format!("transport {pipe} did not close"));
            assert!(start.elapsed() < Duration::from_secs(2));
            assert!(!leaked, "descendant {pid} survived a closed transport");
        }
    }
}
