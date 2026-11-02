use std::{collections::HashSet, path::Path, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub input_microusd_per_million: u64,
    pub output_microusd_per_million: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRequest {
    pub task_id: String,
    pub capability: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterConfig {
    #[serde(default)]
    pub command: Vec<String>,
    pub models: Vec<Model>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteDecision {
    pub model: String,
    pub estimated_microusd: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Prices are USD millionths per million tokens. Round the combined charge up
/// once, so a nonzero fractional microdollar never becomes a free request.
pub fn cost(model: &Model, usage: &Usage) -> Result<u64> {
    let input = u128::from(model.input_microusd_per_million)
        .checked_mul(u128::from(usage.input_tokens))
        .context("input token cost overflow")?;
    let output = u128::from(model.output_microusd_per_million)
        .checked_mul(u128::from(usage.output_tokens))
        .context("output token cost overflow")?;
    let total = input
        .checked_add(output)
        .context("combined token cost overflow")?;
    let rounded = total / 1_000_000 + u128::from(total % 1_000_000 != 0);
    u64::try_from(rounded).context("token cost exceeds u64 microdollar range")
}

/// Validate configuration without executing its adapter or estimating a task.
pub fn validate(config: &RouterConfig) -> Result<()> {
    ensure!(
        !config.models.is_empty(),
        "router requires configured models"
    );
    let mut ids = HashSet::with_capacity(config.models.len());
    for model in &config.models {
        ensure!(!model.id.trim().is_empty(), "model ID must not be empty");
        ensure!(ids.insert(&model.id), "duplicate model ID: {}", model.id);
    }
    ensure!(config.timeout_ms > 0, "router timeout must be positive");
    ensure!(
        std::time::Instant::now()
            .checked_add(Duration::from_millis(config.timeout_ms))
            .is_some(),
        "router timeout is too large"
    );
    if let Some(executable) = config.command.first() {
        ensure!(
            !executable.trim().is_empty(),
            "router executable must not be empty"
        );
    }
    Ok(())
}

/// Run a configured JSON router directly, or choose the cheapest configured
/// model. Only exact model IDs from the configuration can be selected.
pub fn route(config: &RouterConfig, request: &RouteRequest, cwd: &Path) -> Result<RouteDecision> {
    validate(config)?;
    let usage = Usage {
        input_tokens: request.input_tokens,
        output_tokens: request.output_tokens,
    };

    if config.command.is_empty() {
        let mut cheapest: Option<RouteDecision> = None;
        for model in &config.models {
            let candidate = RouteDecision {
                model: model.id.clone(),
                estimated_microusd: cost(model, &usage)?,
            };
            if cheapest.as_ref().is_none_or(|current| {
                (candidate.estimated_microusd, &candidate.model)
                    < (current.estimated_microusd, &current.model)
            }) {
                cheapest = Some(candidate);
            }
        }
        return cheapest.context("router requires configured models");
    }

    let input = serde_json::to_vec(request).context("serialize router request")?;
    let output = super::process::run(&config.command, cwd, Some(&input), config.timeout_ms)
        .context("run router command")?;
    if !output.status.success() {
        bail!(
            "router command failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Selection {
        model: String,
    }
    let selection: Selection = serde_json::from_slice(&output.stdout)
        .context("router stdout must be JSON {\"model\":\"...\"}")?;
    let model = config
        .models
        .iter()
        .find(|model| model.id == selection.model)
        .with_context(|| format!("router selected unknown model: {}", selection.model))?;
    Ok(RouteDecision {
        model: model.id.clone(),
        estimated_microusd: cost(model, &usage)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, input: u64, output: u64) -> Model {
        Model {
            id: id.into(),
            input_microusd_per_million: input,
            output_microusd_per_million: output,
        }
    }

    fn request() -> RouteRequest {
        RouteRequest {
            task_id: "task-1".into(),
            capability: "analysis".into(),
            input_tokens: 1_000_000,
            output_tokens: 500_000,
        }
    }

    fn config(models: Vec<Model>) -> RouterConfig {
        RouterConfig {
            command: Vec::new(),
            models,
            timeout_ms: 30_000,
        }
    }

    #[test]
    fn cost_rounds_the_combined_charge_once() {
        let pricing = model("known", 1, 1);
        assert_eq!(
            cost(
                &pricing,
                &Usage {
                    input_tokens: 1,
                    output_tokens: 1
                }
            )
            .unwrap(),
            1
        );
        assert_eq!(
            cost(
                &pricing,
                &Usage {
                    input_tokens: 500_000,
                    output_tokens: 500_000
                }
            )
            .unwrap(),
            1
        );
        assert_eq!(
            cost(
                &pricing,
                &Usage {
                    input_tokens: 0,
                    output_tokens: 0
                }
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn cost_uses_wide_intermediates_and_reports_overflow() {
        let pricing = model("known", u64::MAX, u64::MAX);
        assert_eq!(
            cost(
                &pricing,
                &Usage {
                    input_tokens: 1_000_000,
                    output_tokens: 0
                }
            )
            .unwrap(),
            u64::MAX
        );
        assert!(
            cost(
                &pricing,
                &Usage {
                    input_tokens: 1_000_001,
                    output_tokens: 0
                }
            )
            .is_err()
        );
        assert!(
            cost(
                &pricing,
                &Usage {
                    input_tokens: u64::MAX,
                    output_tokens: u64::MAX
                }
            )
            .is_err()
        );
    }

    #[test]
    fn builtin_chooses_lowest_estimate_then_exact_id() {
        let cfg = config(vec![
            model("expensive", 10, 10),
            model("zeta", 2, 2),
            model("alpha", 1, 4),
        ]);
        assert_eq!(
            route(&cfg, &request(), Path::new(".")).unwrap(),
            RouteDecision {
                model: "alpha".into(),
                estimated_microusd: 3
            }
        );
    }

    #[test]
    fn configuration_rejects_ambiguous_models_and_invalid_execution_limits() {
        let req = request();
        for models in [
            vec![],
            vec![model(" ", 1, 1)],
            vec![model("same", 1, 1), model("same", 2, 2)],
        ] {
            assert!(route(&config(models), &req, Path::new(".")).is_err());
        }
        let mut cfg = config(vec![model("known", 1, 1)]);
        cfg.timeout_ms = 0;
        assert!(route(&cfg, &req, Path::new(".")).is_err());
        cfg.timeout_ms = 30_000;
        cfg.command = vec![" ".into()];
        assert!(route(&cfg, &req, Path::new(".")).is_err());
    }

    #[test]
    fn config_defaults_to_builtin_routing_and_finite_timeout() {
        let cfg: RouterConfig = serde_json::from_str(
            r#"{"models":[{"id":"known","input_microusd_per_million":1,"output_microusd_per_million":2}]}"#,
        ).unwrap();
        assert!(cfg.command.is_empty());
        assert_eq!(cfg.timeout_ms, 30_000);
    }

    fn external_command(stdout: &str, exit_code: i32) -> Vec<String> {
        #[cfg(windows)]
        {
            vec![
                "powershell".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                format!(
                    "$request = [Console]::In.ReadToEnd() | ConvertFrom-Json; if ($request.task_id -ne 'task-1' -or $request.capability -ne 'analysis' -or $request.input_tokens -ne 1000000 -or $request.output_tokens -ne 500000) {{ exit 17 }}; [Console]::Write('{}'); exit {exit_code}",
                    stdout.replace('\'', "''")
                ),
            ]
        }
        #[cfg(not(windows))]
        {
            vec![
                "sh".into(),
                "-c".into(),
                format!(
                    "cat >/dev/null; printf '%s' '{}'; exit {exit_code}",
                    stdout.replace('\'', "'\\''")
                ),
            ]
        }
    }

    #[test]
    fn external_router_receives_request_and_estimates_only_known_selection() {
        let mut cfg = config(vec![model("known", 7, 5)]);
        cfg.command = external_command(r#"{"model":"known"}"#, 0);
        assert_eq!(
            route(&cfg, &request(), Path::new(".")).unwrap(),
            RouteDecision {
                model: "known".into(),
                estimated_microusd: 10
            }
        );
    }

    #[test]
    fn external_router_cannot_select_an_unknown_model() {
        let mut cfg = config(vec![model("known", 7, 5)]);
        cfg.command = external_command(r#"{"model":"guessed-provider/model"}"#, 0);
        let error = route(&cfg, &request(), Path::new(".")).unwrap_err();
        assert!(error.to_string().contains("unknown model"), "{error:#}");
    }

    #[test]
    fn external_router_requires_only_a_json_model_object_and_successful_status() {
        let mut cfg = config(vec![model("known", 7, 5)]);
        for stdout in [
            "log line\n{\"model\":\"known\"}",
            r#"{"model":"known","estimated_microusd":0}"#,
            r#"{"model":"known"} {"model":"known"}"#,
        ] {
            cfg.command = external_command(stdout, 0);
            assert!(route(&cfg, &request(), Path::new(".")).is_err());
        }
        cfg.command = external_command(r#"{"model":"known"}"#, 9);
        let error = route(&cfg, &request(), Path::new(".")).unwrap_err();
        assert!(
            error.to_string().contains("router command failed"),
            "{error:#}"
        );
    }
}
