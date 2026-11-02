//! Foreground automation engines. These commands do not require a Crew daemon.
pub mod orchestrator;
pub mod process;
pub mod router;
pub mod workflow;

pub const HELP: &str = "\
crew orchestrator plan FILE
crew orchestrator run FILE --execute
crew orchestrator status FILE
crew workflow plan FILE
crew workflow advance FILE [--execute] [--allow-ci]
crew workflow run FILE [--execute] [--allow-ci] [--poll-ms N] [--timeout-ms N]

Plans are explicit JSON files. Execution, GitHub writes and hosted CI are opt-in.
See docs/team/automation/README.md for adapters, budgets, state and recovery.
";
