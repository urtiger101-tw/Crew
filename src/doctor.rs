//! Read-only diagnostics from the daemon actually serving this socket.
use crate::api::{Reply, Request};
use serde_json::{Value, json};

pub const HELP: &str =
    "crew doctor [--json]  Show daemon version, start time and color environment hazards";

pub fn snapshot() -> Value {
    let hazards: Vec<_> = ["NO_COLOR", "TERM", "CLICOLOR", "FORCE_COLOR"]
        .into_iter()
        .filter_map(|name| {
            let value = std::env::var(name).ok()?;
            let hazardous = match name {
                "NO_COLOR" => !value.is_empty(),
                "TERM" => value == "dumb",
                _ => value == "0",
            };
            hazardous.then(
                || json!({"name": name, "value": if name == "NO_COLOR" { "set" } else { &value }}),
            )
        })
        .collect();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "executable": std::env::current_exe().ok(),
        "started_at_unix_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
        "environment_hazards": hazards,
        "pane_color_policy": "inherited NO_COLOR removed; TERM=xterm-256color; COLORTERM=truecolor; explicit pane overrides win",
        "chrome_color_policy": "resolved from each attaching client's environment and configured theme"
    })
}

pub fn run(args: &[String]) -> anyhow::Result<()> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        println!("{HELP}");
        return Ok(());
    }
    if !args.is_empty() && args != ["--json"] {
        anyhow::bail!("{HELP}");
    }
    let Reply::Json(value) = crate::client::request(Request::ServerDoctor)? else {
        anyhow::bail!("unexpected doctor response; update the daemon");
    };
    if args.is_empty() {
        println!(
            "Client {} / daemon {} (PID {})",
            env!("CARGO_PKG_VERSION"),
            value["version"],
            value["pid"]
        );
        println!("Daemon executable: {}", value["executable"]);
        println!(
            "Daemon start (Unix milliseconds): {}",
            value["started_at_unix_ms"]
        );
        println!(
            "Daemon environment hazards: {}",
            value["environment_hazards"]
        );
        println!("Pane colors: {}", value["pane_color_policy"]);
        println!("Chrome colors: {}", value["chrome_color_policy"]);
    } else {
        println!("{}", json!({"result": value}));
    }
    Ok(())
}
