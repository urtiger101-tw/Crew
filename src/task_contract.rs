//! Offline task records. Validation checks declarations, never executes their contents.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1024 * 1024;
const HELP: &str = "crew task validate FILE\ncrew task store FILE --output FILE\ncrew task inspect FILE\nOffline schema v1; never starts a daemon or executes contract contents.\nExit 0 validates the record, not the evidence or task completion.";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    schema_version: u32,
    task_id: String,
    objective: String,
    role: String,
    authorized_actions: Vec<String>,
    constraints: Vec<String>,
    non_goals: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<Model>,
    #[serde(default)]
    context: Vec<Context>,
    instructions: Vec<Instruction>,
    #[serde(default)]
    examples: Vec<Example>,
    output_contract: OutputContract,
    verification: Vec<Criterion>,
    revision_policy: RevisionPolicy,
    #[serde(default)]
    attempts: Vec<Attempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at_unix_ms: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Model {
    provider: String,
    name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Trust {
    RepositoryData,
    ToolOutput,
    UntrustedReference,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Context {
    uri: String,
    provenance: String,
    trust: Trust,
    revision: String,
    byte_budget: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Instruction {
    id: String,
    action: String,
    owner: String,
    depends_on: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Example {
    input: String,
    output: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OutputContract {
    format: String,
    artifacts: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Criterion {
    id: String,
    criterion: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RevisionPolicy {
    max_attempts: u32,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
enum Status {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
    #[serde(rename = "NOT_RUN")]
    NotRun,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Check {
    id: String,
    status: Status,
    evidence_ref: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Review {
    reviewer: String,
    status: Status,
    evidence_ref: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    number: u32,
    author: String,
    summary: String,
    checks: Vec<Check>,
    review: Review,
}

fn required(value: &str, field: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{field} must not be blank");
    Ok(())
}

impl Contract {
    fn validate(&self, now_ms: u128) -> Result<&'static str> {
        ensure!(
            self.schema_version == 1,
            "unsupported schema_version; expected 1"
        );
        for (field, value) in [
            ("task_id", &self.task_id),
            ("objective", &self.objective),
            ("role", &self.role),
            ("output_contract.format", &self.output_contract.format),
        ] {
            required(value, field)?;
        }
        for value in self
            .authorized_actions
            .iter()
            .chain(&self.constraints)
            .chain(&self.non_goals)
            .chain(&self.output_contract.artifacts)
        {
            required(value, "action, constraint, non-goal or artifact")?;
        }
        ensure!(
            !self.output_contract.artifacts.is_empty(),
            "output_contract.artifacts must not be empty"
        );
        if let Some(model) = &self.model {
            required(&model.provider, "model.provider")?;
            required(&model.name, "model.name")?;
        }
        for context in &self.context {
            required(&context.uri, "context.uri")?;
            required(&context.provenance, "context.provenance")?;
            required(&context.revision, "context.revision")?;
            ensure!(
                (1..=MAX_BYTES).contains(&u64::from(context.byte_budget)),
                "context.byte_budget must be 1..=1048576"
            );
        }
        for example in &self.examples {
            required(&example.input, "example.input")?;
            required(&example.output, "example.output")?;
        }
        ensure!(
            !self.instructions.is_empty(),
            "instructions must not be empty"
        );
        let mut instruction_ids = HashSet::new();
        for instruction in &self.instructions {
            required(&instruction.id, "instruction.id")?;
            required(&instruction.action, "instruction.action")?;
            required(&instruction.owner, "instruction.owner")?;
            let mut dependencies = HashSet::new();
            for dependency in &instruction.depends_on {
                ensure!(
                    instruction_ids.contains(dependency),
                    "instruction dependency must reference an earlier instruction"
                );
                ensure!(
                    dependencies.insert(dependency),
                    "duplicate instruction dependency"
                );
            }
            ensure!(
                instruction_ids.insert(&instruction.id),
                "duplicate instruction id"
            );
        }
        ensure!(
            !self.verification.is_empty(),
            "verification must not be empty"
        );
        let mut criteria = HashSet::new();
        for criterion in &self.verification {
            required(&criterion.id, "verification.id")?;
            required(&criterion.criterion, "verification.criterion")?;
            ensure!(criteria.insert(&criterion.id), "duplicate verification id");
        }
        ensure!(
            (1..=100).contains(&self.revision_policy.max_attempts),
            "max_attempts must be 1..=100"
        );
        ensure!(
            self.attempts.len() <= self.revision_policy.max_attempts as usize,
            "retry budget exceeded"
        );
        let mut declared_pass = false;
        for (index, attempt) in self.attempts.iter().enumerate() {
            ensure!(
                !declared_pass,
                "cannot append an attempt after declared_pass"
            );
            ensure!(
                attempt.number as usize == index + 1,
                "attempt numbers must be consecutive from 1"
            );
            required(&attempt.author, "attempt.author")?;
            required(&attempt.summary, "attempt.summary")?;
            required(&attempt.review.reviewer, "review.reviewer")?;
            ensure!(
                attempt.review.reviewer != attempt.author
                    && !self
                        .instructions
                        .iter()
                        .any(|instruction| instruction.owner == attempt.review.reviewer),
                "reviewer must be independent of author and instruction owners"
            );
            if attempt.review.status != Status::NotRun {
                required(&attempt.review.evidence_ref, "review.evidence_ref")?;
            }
            let mut seen = HashSet::new();
            for check in &attempt.checks {
                ensure!(
                    criteria.contains(&check.id),
                    "unknown verification check id"
                );
                ensure!(seen.insert(&check.id), "duplicate verification check id");
                if check.status != Status::NotRun {
                    required(&check.evidence_ref, "check.evidence_ref")?;
                }
            }
            ensure!(
                seen.len() == criteria.len(),
                "every attempt must report all verification checks"
            );
            declared_pass = attempt.review.status == Status::Pass
                && attempt
                    .checks
                    .iter()
                    .all(|check| check.status == Status::Pass);
        }
        Ok(
            if self
                .expires_at_unix_ms
                .is_some_and(|expiry| u128::from(expiry) <= now_ms)
            {
                "expired"
            } else if declared_pass {
                "declared_pass"
            } else if self.attempts.is_empty() {
                "ready"
            } else if self.attempts.len() == self.revision_policy.max_attempts as usize {
                "exhausted"
            } else {
                "revision_required"
            },
        )
    }
}

pub(crate) fn run(args: &[String]) -> i32 {
    let help = match args {
        [] => true,
        [flag] => matches!(flag.as_str(), "--help" | "-h"),
        [operation, flag] => {
            matches!(operation.as_str(), "validate" | "store" | "inspect")
                && matches!(flag.as_str(), "--help" | "-h")
        }
        _ => false,
    };
    if help {
        println!("{HELP}");
        return 0;
    }
    let (operation, source, output) = match args {
        [operation, source]
            if matches!(operation.as_str(), "validate" | "inspect") && !source.starts_with('-') =>
        {
            (operation.as_str(), source, None)
        }
        [operation, source, flag, output]
            if operation == "store"
                && flag == "--output"
                && !source.starts_with('-')
                && !output.starts_with('-') =>
        {
            (operation.as_str(), source, Some(output))
        }
        _ => {
            eprintln!(
                "{}",
                json!({"error":{"code":"invalid_argument","message":HELP}})
            );
            return 2;
        }
    };
    // Categorize failures at the CLI boundary; no user-provided text is executed.
    let mut error_code = "io_error";
    let result = (|| -> Result<serde_json::Value> {
        let input = File::open(source)?;
        let mut bytes = Vec::new();
        input.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        error_code = "invalid_contract";
        ensure!(bytes.len() as u64 <= MAX_BYTES, "contract exceeds 1 MiB");
        let contract: Contract = serde_json::from_slice(&bytes)?;
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let state = contract.validate(now_ms)?;
        let mut canonical = serde_json::to_vec_pretty(&contract)?;
        canonical.push(b'\n');
        // Canonical expansion must remain readable by this same CLI.
        ensure!(
            canonical.len() as u64 <= MAX_BYTES,
            "canonical contract exceeds 1 MiB"
        );
        let mut result = json!({
            "schema_version":1, "task_id":contract.task_id,
            "contract_valid":true, "evidence_verified":false, "state":state,
            "sha256":format!("{:x}", Sha256::digest(&canonical))
        });
        if let Some(output) = output {
            error_code = "io_error";
            let mut file = match OpenOptions::new().write(true).create_new(true).open(output) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    error_code = "already_exists";
                    bail!("output already exists; choose a new path");
                }
                Err(error) => return Err(error.into()),
            };
            file.write_all(&canonical)?;
            file.sync_all()?;
            result["path"] = json!(output);
        }
        if operation == "inspect" {
            result["contract"] = serde_json::to_value(&contract)?;
        }
        Ok(result)
    })();
    match result {
        Ok(result) => {
            println!("{}", json!({"result":result}));
            0
        }
        Err(error) => {
            eprintln!(
                "{}",
                json!({"error":{"code":error_code,"message":error.to_string()}})
            );
            1
        }
    }
}
