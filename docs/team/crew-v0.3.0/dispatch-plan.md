# Crew v0.3.0 — Herdr alignment

Date: 2026-10-20. User-selected product name: **Crew**; primary command: `crew`.
Target baseline: `12be9e307e47a56d3bfd669ab7bddb1ee7466e09` (tmuxlike 0.2.1).
Read-only Herdr reference: `143bf5fd4781d164260c63fe28e7dc08054ad32f` (0.9.5).

## Goal and decisions

Continue the existing multiplexer rather than discard it. Preserve the legacy executable,
environment variables, existing configuration/state, Traditional Chinese UI, Windows ConPTY
handling and MIT implementation. Herdr is a behavior reference; do not copy its source.
Existing sessions/windows map to workspaces/tabs. New stable identifiers must survive rename,
unrelated deletion and save/restore. Native socket framing remains length-prefixed JSON;
CLI behavior alignment does not claim Herdr wire compatibility.

## Actual team and ownership

All workers are real independent agent instances inheriting the authorized model/provider.
Each implementation author uses an isolated git worktree. Shared public wiring is coordinator
owned and established before work starts; integration and review are separate stages.

| Task | Owner | Write scope | Acceptance |
|---|---|---|---|
| DISC-1 | reference_audit | read only | Concrete feature matrix from pinned Herdr source |
| DISC-2 | target_audit | read only | Existing architecture, migration risks and interfaces |
| BRAND | root | Cargo, lib entry, brand/config paths, help, packaging | crew + tmuxlike use one implementation; CREW_* wins with legacy fallback |
| CORE | target_audit | workspace module, server model, persistence, affected test fixtures | workspace/tab CRUD, stable IDs, cwd/env, no-focus, compatible restore |
| GIT | worktree_worker | worktree module, dedicated smoke | list/create/open/remove, real isolated checkout, dirty removal refused, no shell interpolation |
| AGENT | reference_audit | agent implementation, integrations module, dedicated smoke | Wait/prompt pin occupant; explicit hook adapters and detection evidence |
| API | root | events, bridge, shared API/CLI wiring | Bounded event history, cursor gaps, nonblocking waits, structured JSON methods |
| VERIFY | root + independent review instance | tests, evidence, fixes | Local unit/clippy/format, real PTY smoke, latest-head independent review and CI |

## Validation

Baseline: Linux x86_64, Rust 1.96.1, `cargo test --locked`: **288 passed**.
Use unique sockets, temporary state/config directories and temporary Git repositories for
all integration tests. Never stop unrelated servers or remove user worktrees. Git/SSH I/O and
event waits must not hold the multiplexer State lock.

Required evidence: actual command, exit code, tested revision, platform and known limits.
Windows/macOS are NOT_RUN until their actual CI jobs pass. A source-level implementation,
a local Linux pass, a PR, a merge and a published release are separate completion levels.

## Full alignment tracking

Record the remaining advanced areas honestly in the parity document: federated SSH UI,
marketplace/plugins, Ghostty/Kitty protocols, native notifications, screen history and live
server handoff. Do not claim complete Herdr coverage from renamed commands or placeholder APIs.

## Delivery

Prepare the implementation branch, PR, CI and GitHub Release in **urtiger101-tw/Crew**.
The user explicitly authorized this destination and release on 2026-10-20 to avoid consuming
stevenke1981 Actions quota. The original repository is read-only; do not push or run CI there.
Preserve history in the new repository. No force push or destructive replacement of an existing
default branch. Publish the verified release only after the latest PR review and CI succeed.
