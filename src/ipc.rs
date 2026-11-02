//! Local socket naming: named pipes on Windows, abstract/unix sockets elsewhere.
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, Stream, prelude::*,
};
use std::io;
use std::sync::mpsc;
use std::time::Duration;

/// Endpoint name: `CREW_SOCKET` when set, otherwise `crew-<user>.sock`.
pub fn base_name() -> String {
    crate::brand::env_var("SOCKET").unwrap_or_else(|_| default_base_name())
}

/// True when `CREW_SOCKET` picks the endpoint, so the default names are not in use.
pub fn socket_overridden() -> bool {
    crate::brand::env_var("SOCKET").is_ok()
}

/// Default endpoint name for this user.
pub fn default_base_name() -> String {
    format!("crew-{}.sock", user())
}

/// Endpoint name used before the rename. A server started by an older build may still listen
/// there, but this build never connects to it. It only names the old state file (`persist`).
pub fn legacy_base_name() -> String {
    format!("tmuxlike-{}.sock", user())
}

fn user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "default".into())
}

fn name_for(base: &str) -> io::Result<Name<'static>> {
    if GenericNamespaced::is_supported() {
        base.to_owned().to_ns_name::<GenericNamespaced>()
    } else {
        std::env::temp_dir()
            .join(base)
            .to_fs_name::<GenericFilePath>()
    }
}

fn connect_named(base: &str) -> io::Result<Stream> {
    Stream::connect(name_for(base)?)
}

pub fn connect() -> io::Result<Stream> {
    connect_named(&base_name())
}

/// Only a missing/refused endpoint permits automatic server creation.
pub(crate) fn server_is_absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

pub(crate) fn connection_error(socket: &str, error: &io::Error) -> crate::api::ApiError {
    let (code, message) = if error.kind() == io::ErrorKind::PermissionDenied {
        (
            "ipc_permission_denied",
            format!(
                "access denied connecting to Crew socket '{socket}'; the server may be running ({error})"
            ),
        )
    } else if server_is_absent(error) {
        (
            "server_not_running",
            format!("no server running on Crew socket '{socket}' ({error})"),
        )
    } else {
        (
            "transport_error",
            format!("cannot connect to Crew socket '{socket}' ({error})"),
        )
    };
    crate::api::ApiError::new(code, message)
}

/// How long a liveness probe waits for its connect. A connect to a busy named pipe waits in the
/// OS without a timeout, so a probe that has not returned by then counts as live.
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// True when a server accepts connections on the endpoint `base`. A missing or stale endpoint is
/// not live. Any other connect error (a busy pipe, say) and a connect that does not return within
/// `PROBE_TIMEOUT` count as live, so state is never migrated out from under a server that may
/// still be running.
pub fn endpoint_is_live(base: &str) -> bool {
    let (tx, rx) = mpsc::channel();
    let base = base.to_owned();
    // The connect runs on its own thread so the probe can give up. A thread left blocked after
    // the timeout is leaked on purpose: it holds no state, and it ends when the pipe frees up or
    // the process exits.
    std::thread::spawn(move || {
        let outcome = connect_named(&base).map(|_stream| ());
        let _ = tx.send(outcome);
    });
    match rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(Ok(())) => true,
        Ok(Err(e)) => !server_is_absent(&e),
        Err(_) => true,
    }
}

pub fn listen() -> io::Result<interprocess::local_socket::Listener> {
    ListenerOptions::new()
        .name(name_for(&base_name())?)
        .try_overwrite(true)
        .create_sync()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_failures_distinguish_absence_permission_and_transport() {
        for (kind, code, may_start) in [
            (io::ErrorKind::NotFound, "server_not_running", true),
            (io::ErrorKind::ConnectionRefused, "server_not_running", true),
            (
                io::ErrorKind::PermissionDenied,
                "ipc_permission_denied",
                false,
            ),
            (io::ErrorKind::TimedOut, "transport_error", false),
            (io::ErrorKind::InvalidInput, "transport_error", false),
        ] {
            let error = io::Error::from(kind);
            let result = connection_error("crew-test.sock", &error);
            assert_eq!(result.code, code);
            assert!(result.message.contains("crew-test.sock"));
            assert_eq!(server_is_absent(&error), may_start);
            if kind == io::ErrorKind::PermissionDenied {
                assert!(result.message.contains("may be running"));
            }
        }
        #[cfg(windows)]
        for (raw, code) in [(2, "server_not_running"), (5, "ipc_permission_denied")] {
            assert_eq!(
                connection_error("test", &io::Error::from_raw_os_error(raw)).code,
                code
            );
        }
        #[cfg(unix)]
        for raw in [libc::EACCES, libc::EPERM] {
            assert_eq!(
                connection_error("test", &io::Error::from_raw_os_error(raw)).code,
                "ipc_permission_denied"
            );
        }
    }

    #[test]
    fn default_endpoint_is_crew_named() {
        let name = default_base_name();
        assert!(
            name.starts_with("crew-") && name.ends_with(".sock"),
            "{name}"
        );
        assert_ne!(name, legacy_base_name());
    }

    #[test]
    fn legacy_endpoint_keeps_the_pre_rename_name() {
        let name = legacy_base_name();
        assert!(
            name.starts_with("tmuxlike-") && name.ends_with(".sock"),
            "{name}"
        );
    }

    #[test]
    fn endpoint_is_live_only_while_a_listener_holds_it() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let base = format!("crew-probe-test-{}-{nanos}.sock", std::process::id());
        assert!(!endpoint_is_live(&base), "nothing listens yet");
        let listener = ListenerOptions::new()
            .name(name_for(&base).unwrap())
            .create_sync()
            .unwrap();
        assert!(endpoint_is_live(&base), "a listener is live");
        drop(listener);
        assert!(!endpoint_is_live(&base), "a closed endpoint is not live");
    }

    /// interprocess keeps one free pipe instance after each accept. Two clients that are never
    /// accepted use up the free instances, and a plain connect then waits on the busy pipe. The
    /// probe must answer "live" within its timeout instead of waiting with it.
    #[cfg(windows)]
    #[test]
    fn busy_endpoint_probe_returns_live_without_blocking() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let base = format!("crew-busy-probe-test-{}-{nanos}.sock", std::process::id());
        let listener = ListenerOptions::new()
            .name(name_for(&base).unwrap())
            .create_sync()
            .unwrap();
        let first = connect_named(&base).unwrap();
        let _accepted = listener.accept().unwrap();
        let second = connect_named(&base).unwrap();
        // Precondition: a plain connect really waits on this busy pipe.
        let (tx, rx) = mpsc::channel();
        let plain_name = base.clone();
        std::thread::spawn(move || {
            let _ = tx.send(connect_named(&plain_name).map(|_| ()));
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "a plain connect must still be waiting on the busy pipe"
        );
        let started = std::time::Instant::now();
        assert!(endpoint_is_live(&base), "a busy endpoint is live");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the probe must not wait for the busy pipe"
        );
        drop((first, second));
    }
}
