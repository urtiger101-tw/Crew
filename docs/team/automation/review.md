# Scoped review and validation

Root implemented Orchestrator, process adapter and CLI; Sol xhigh implemented router; Terra xhigh implemented Workflow Engine. Authors used separate worktrees. The root integrates all changes.

## Independent review

- Sol reviewed root Orchestrator/process/CLI. Three concrete findings were fixed: locks now use canonical state identity with a fresh state read after acquisition; state updates use unique create_new temporary siblings; state fingerprints include resolved cwd/state paths. Regression tests cover all three triggers. Sol's final narrow reread reported no unresolved findings.
- Terra reviewed Sol's router: no actionable findings in allowlist validation, JSON/argv execution, cost rounding or overflow handling.
- Root reviewed Terra's workflow. Fixed: absent help entry, lack of polling driver, empty check lists, approval alongside unresolved changes requested, non-paginated statuses, config/state path collision and stale readiness after a failed evidence refresh. The final implementation has corresponding tests; tests were then moved into a separate module without behavior changes.

## Executed checks

`cargo test --locked` passed: 614 unit tests plus 26 integration tests; 4 existing ignored tests. After the final platform-clock timeout guard and test-module split, the affected `cargo test --locked --lib automation` passed all 29 tests. `cargo clippy --locked --all-targets -- -D warnings`, `cargo fmt --all --check`, and `git diff --check` passed.

Actual Windows subprocess tests cover direct stdin/argv forwarding, timeout termination, router JSON/allowlist, DAG ordering, reservation and usage accounting, dependency failures, resume, priority, real concurrent execution, concurrency limits, overlapping scope serialization, state locks and config fingerprints. Workflow tests use deterministic GitHub responses and a real configured fake-gh executable; they do not perform GitHub writes.

The compiled CLI ran the documented Windows example with an external router and two local demonstration adapters. Plan/run/status passed; a second run preserved the completed state hash. Offline workflow plan made no state mutation; missing --execute was rejected. These adapters return fixture usage and do not demonstrate provider spending or independent human review.

Logs, CLI outputs and binary are in `target/verification/automation/` and `target/debug/crew.exe`. `handoff.json` and `source-manifest.json` record the dirty-source identity and results. Local source HEAD is only a base identifier; these changes are not committed.

## Not run

Paid model/provider calls, live remote PR creation/review requests/workflow dispatch, hosted CI, Linux/macOS execution, release packaging and shared daemon restart: NOT_RUN. CI triggers and global settings were not modified. No commit or push was performed by this task.

## Operational limits

Task decomposition is supplied as a DAG. Capability/priority scheduling is automatic; natural-language planning is not included. State resumes finished tasks but intentionally requires inspection after interrupted in-flight work. Budget admission uses estimates; external adapters report usage, and actual overrun stops later admissions. Provider hard caps and the router's own spending require provider/adapter controls. File ownership scopes coordinate dispatch and do not sandbox executables. Workflow readiness covers configured current-head review/check gates, not every repository merge or deployment policy.
