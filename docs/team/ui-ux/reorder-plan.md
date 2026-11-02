# Pane swapping and drag ordering

Base: fd0696d7de44a3a0d0dc893283365b347ae09510. Main owns integration.

User requested previously missing pane-position swaps and mouse dragging, then
explicitly requested GPT-6.1 Sol xhigh + Terra xhigh assistance and a progress
notification to Claude. Progress was delivered in log/98-codex.crl and submitted
to Claude's live pane. Shared-daemon restart remains Claude's responsibility.

## Scope and acceptance

- Swap two panes in one tab by title drag, prefix `{`/`}`, or CLI/API. Preserve
  process identity, shell state, pane IDs and focused pane; keep split geometry.
- Reorder tabs by dragging status labels or prefix Shift+Left/Right. Reuse the
  existing insertion-slot operation so chooser/confirmation/MRU state stays valid.
- Commit drag moves on release after a two-cell threshold; drop outside or Escape
  cancels. Content dragging remains copy selection. Resize/surface changes cancel.
- Persist order with the existing format. Cross-tab pane swapping is rejected.
- Validate parser boundaries, state/gesture behavior, and isolated real-client
  events with live shell PIDs and markers before/after moves and cold restoration.
- Run format, Clippy and relevant/full Rust tests; one independent scoped review.
  Native visual mouse verification must be reported separately from event injection.

## Work and dependencies

1. Main: core layout/state operation, command/JSON CLI, bindings, docs and integration.
2. Terra xhigh: mouse/drag enum and tests in isolated target/drag-reorder-ui worktree;
   changes integrated after author freeze. Main now owns subsequent fixes.
3. Sol xhigh: read-only wiring/risk analysis completed; independent review after
   integrated implementation and tests. No reviewer authored the reviewed code.

No new dependency, public release or shared-daemon restart is needed for this task.
Preserve unrelated `.sites-runtime/` and all earlier release evidence.

## Evidence (2026-11-02)

- PASS: full `cargo test --locked`: 558 unit and 26 integration tests; two
  existing ignored tests. Log: `target/reorder-tests.log`.
- PASS: `cargo clippy --locked --all-targets -- -D warnings`, formatting and
  release build. The new `tests/pane_reorder.rs` uses an isolated daemon, real
  PTYs and client-protocol mouse/key events. It proves swaps, Escape, tab drag,
  PID/shell-memory preservation and cold-restored label order/focused positions.
- Independent Sol review found ordinary agent reports canceled gestures through
  unconditional relayout invalidation. Fixed to compare pane geometry; the test
  waits for title Down focus, sends a higher-sequence identical report, then
  finishes the gesture. Limited independent recheck: PASS, no remaining finding.
- PASS: workspace move and bridge/events smoke. Concurrent debug-binary API and
  workspace-transfer smoke encountered terminal text-matching failures. Both
  passed when run serially against the new release binary; baseline v0.6.0 API
  smoke also passed. Full native gate is the final source-bound acceptance.
- NOT_RUN: native physical mouse acceptance and non-Windows checks. The mouse
  integration test injects client events and does not claim native UI evidence.
- Shared daemon not restarted. No release published. Claude retains runtime
  switching ownership; the next UI wave is requested in `log/99-claude.crl`.
