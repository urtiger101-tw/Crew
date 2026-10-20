# Crew v0.3.0: rename the project and add workspace, worktree, and agent automation

## Problem and outcome

The existing tmuxlike multiplexer provides terminal sessions and agent status, but
automation needs stable workspace/tab identifiers, isolated Git checkouts, and
reliable agent lifecycle tracking. This change renames the product to **Crew** and
implements a defined subset of Herdr's behavior on the existing Rust terminal
backend. It preserves the original Git history and the `tmuxlike` compatibility
executable.

The destination is **`urtiger101-tw/Crew`**, with
`feat/crew-herdr-alignment` targeting `main`. The original
`stevenke1981/tmuxlike` repository remains read-only. At preparation time on
2026-10-20, the coordinator reported that history import was complete and feature
files were being uploaded through GitHub's UI; a PR had not yet been opened.
Remote CI, merge, and release are **NOT_RUN** in this handoff.

## What changes

- **Brand and compatibility:** `crew` is the primary command; `tmuxlike` runs the
  same implementation. `CREW_*` variables take precedence with `TMUXLIKE_*`
  fallback. Existing configuration/state paths, Traditional Chinese UI, and
  platform terminal backends are retained.
- **Workspace and tab automation:** CRUD, stable `wN`/`tN` IDs, caller-aware current
  tab selection, exact names, cwd/env/startup command, and creation without focus
  changes by default. IDs and explicit environment overrides survive layout
  restore; legacy version 1 state remains readable.
- **Git worktrees:** real list/create/open/remove operations, direct Git argument
  arrays, and Unicode/space-safe paths. Opening creates or reuses a workspace at
  the actual checkout. Removal preserves branches and refuses dirty, untracked,
  ignored, primary, locked, or known active checkouts. Linux checks live pane
  process directories; reservations reject new Crew panes below a checkout being
  removed. External Git I/O runs outside the shared State lock.
- **Agent lifecycle:** waits and prompt submission bind to one reported occupant.
  Replacement, release, or a blocked replacement prevents stale delayed Enter.
  Generic report/release hooks, a registry, and PATH diagnostics expose their
  evidence without launching providers or claiming official hooks are installed.
- **Automation transport and events:** a concurrent JSON-lines bridge with
  correlated responses, structured errors, and bounded requests; a 512-entry
  event journal with cursors, restart epochs, gap reporting, and long polling.
- **Delivery:** Rust 1.96.1; PR, main-push, and merge-group CI on Linux, Windows,
  and macOS; four native release targets. Packages include both executables,
  README, LICENSE, provenance, and checksums. Publication verifies the source,
  latest CI run/attempt (which must have completed successfully), and every
  uploaded asset before exposing one complete release. Only its publish job has
  `contents: write`.

The implementation refers to Herdr **0.9.5** at
`143bf5fd4781d164260c63fe28e7dc08054ad32f`; its source code was not imported.
See the [parity matrix][parity] and [automation contract][automation].

## Acceptance and verification

The local comparison base is tmuxlike 0.2.1,
`12be9e307e47a56d3bfd669ab7bddb1ee7466e09`. The documentation snapshot is
`eaec779d2946de7424f5fad1d2a4a45eafbe9269`.
The commands below actually ran against
**`efbf228127774bd25c7cf5904485eb27204d1632`**, tree
`54ab25e4b096519ac9d540f8efd49bf3af943851`, on Linux x86_64 with Rust 1.96.1
and Python 3.12.14. The only files added between that tested SHA and `eaec779`
are `local-validation.md` and `local-validation.json`; the tested implementation,
workflows, and test scripts are unchanged. These local SHAs do not identify a
GitHub UI-created commit automatically: the coordinator must bind the uploaded
contents and actual remote head to this evidence before merging.

| Acceptance | Executed check | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | **PASS**, exit 0 |
| Lint | `cargo clippy --offline --locked --all-targets -- -D warnings` | **PASS**, exit 0 |
| Unit regression | `cargo test --offline --locked` | **PASS**, 318 passed, 0 failed |
| Release tooling | `python3 scripts/package_release.py self-test` | **PASS**, 5 tests, including 13 final-publication scenarios using mocked GitHub responses |
| Optimized executables | `cargo build --offline --locked --release --bins` | **PASS**, both `crew` and `tmuxlike` |
| Existing terminal compatibility | `scripts/smoke.py` with release `tmuxlike` | **PASS**, real PTY |
| Automation API | `scripts/api_smoke.py` with release `crew` | **PASS**, real server/PTY |
| Workspace/tab behavior | `scripts/workspace_smoke.py` with release `crew` | **PASS**, stable IDs, cwd/env, restore, caller context |
| Git isolation and removal | `scripts/worktree_smoke.py` with release `crew` | **PASS**, 13 checks plus Linux R3/R6 race regressions |
| Agent identity/hooks | `scripts/agent_identity_smoke.py` with release `crew` | **PASS**, real PTY identity and delayed-Enter cases |
| Bridge/events | `scripts/bridge_events_smoke.py` with release `crew` | **PASS**, response correlation, waits, limits, and recovery |
| Windows/macOS execution; hosted CI; merge; published release | No execution evidence in this handoff | **NOT_RUN**, coordinator gate |

All 11 local commands exited 0. The six smoke suites used optimized release
executables, isolated sockets/state/config, and temporary repositories. Full
argv, timestamps, exit codes, source hashes, and binary hashes are in the
[local validation report][local] and its [JSON record][local-json]. The interactive
`scripts/e2e_linux.py` scenario was **NOT_RUN**; it is separate from those six
completed smoke suites. Mocked release tests are not evidence of a live release.

## Independent review and resolved findings

| Finding | Resolution and independent evidence |
|---|---|
| **R1 / P1** — cleared or quarantined new state could resurrect stale legacy state | Persist a state-selection marker. Restore chooses the selected state without modifying or reviving the old file. [CORE review][core-review] |
| **R2 / P1** — inherited Crew environment could direct smoke cleanup at a caller's server | Clear both Crew and legacy environment families before assigning isolated test paths; sentinel tests preserved the unrelated server. [CORE review][core-review] |
| **R3 / P2** — a pane that changed directory into a checkout could be missed by removal | Inspect Linux foreground/shell live cwd outside State; removal refuses active checkout and succeeds after the process leaves. [CORE review][core-review] |
| **R4 / P2** — an inactive caller pane could resolve the foreground tab | Resolve current tab from the caller's pane with explicit-target precedence, including the legacy variable. [CORE review][core-review] |
| **R5 / P2** — doctor panicked on a valid non-UTF-8 Unix PATH | Emit a printable path and explicit `path_lossy`; independent invalid-byte, Chinese/space, and missing-executable fixtures returned valid JSON without launching anything. [AGENT review][agent-review] |
| **R6 / P2** — new workspace/tab/split creation could race checkout removal | Reserve the canonical removal path, reject new panes beneath it, permit unrelated work, and release reservations on success and errors. Deterministic real-Git regression passed. [CORE review][core-review] |
| **DELIVERY-1 / P2** — CI could change while release assets uploaded | Recheck the exact latest successful CI run/attempt, jobs, and steps immediately before publication; also verify tag and final uploaded asset state/size/SHA256. Independent failure cases keep the release as a draft. [Delivery review][delivery-review] |

No open blocking findings remain in these recorded review scopes. CORE/GIT/API
and AGENT reviews target product source at `a421a6b`; those product sources are
unchanged in the final local validation. Delivery review targets author commit
`ec2329e`, integrated as `0d24000`, with matching file hashes. Reviewers are
separate agent instances from the respective authors. These scoped records do
not constitute distinct GitHub account approvals or a completed whole-PR merge
decision; the coordinator owns the final review and CI gate.

## Compatibility risks and remaining Herdr differences

Before using the new APIs, save current work and stop the old daemon with its
`server stop` command. Both executable names use the same default socket; a new
client does not upgrade an already running old daemon. Stopping the server ends
processes. Layout persistence preserves workspace/tab IDs, but pane IDs must be
queried again and arbitrary startup commands are not replayed. Explicit `--env`
overrides are stored in the local state file.

Agent instance identity describes an occupant, not each submitted prompt. Use one
prompt producer per agent, unique sources per reporting process, and stop reports
after release; there is no sequence tombstone across releases. Already pasted
text cannot be recalled when a later Enter is cancelled.

Worktree protection uses known start directories on Windows/macOS; live process
cwd inspection is Linux-specific. It cannot atomically lock other processes'
directory changes or writes. Checkout hooks are disabled, but Git filters still
follow repository configuration and Git subprocesses have no added timeout.

Herdr coverage remains partial: no federated SSH workspace UI, plugin marketplace,
Ghostty/Kitty advanced protocols, native notifications, screen-history feature,
live server handoff, automatic process/agent kind detection, workspace/tab moves,
or metadata TTL. The bridge is Crew's versioned CLI-argument interface and the
native socket stays length-prefixed; **Herdr wire compatibility is not provided**.
Events are bounded and in memory. Packages are unsigned; native signing,
notarization, and installer/upgrade validation are outside this change.

## Rollback

1. Before upgrading, save work, stop the old server, and retain the prior binary
   and copies of the actual config/state paths, including any selection marker.
   Account for path overrides instead of assuming platform defaults.
2. If reverting, save work and stop the Crew server with `crew server stop`.
   Preserve its state separately, reinstall the prior binary, and restore a
   pre-upgrade state/config backup. Reading new-format state with an older binary
   is not a verified downgrade path. Do not use `kill-server` or delete the
   selection marker as a migration repair.
3. Git checkouts and branches remain user data. Closing a workspace does not
   remove its worktree; inspect and preserve checkout changes separately.
4. For a source rollback after merge, submit a revert through the same Crew
   review/CI process and issue a corrected version if required. Do not rewrite
   published history or repoint an existing release tag. Operational rollback
   execution is **NOT_RUN** in this handoff.

The [merge and release checklist][checklist] records the outstanding remote gates.
Until they have real run URLs, exact SHAs, and reviewer decisions, this PR is
prepared and locally validated, with merge/release still pending.

[parity]: https://github.com/urtiger101-tw/Crew/blob/feat/crew-herdr-alignment/docs/herdr-parity.md
[automation]: https://github.com/urtiger101-tw/Crew/blob/feat/crew-herdr-alignment/docs/automation.md
[local]: https://github.com/urtiger101-tw/Crew/blob/feat/crew-herdr-alignment/docs/team/crew-v0.3.0/local-validation.md
[local-json]: https://github.com/urtiger101-tw/Crew/blob/feat/crew-herdr-alignment/docs/team/crew-v0.3.0/local-validation.json
[core-review]: https://github.com/urtiger101-tw/Crew/blob/feat/crew-herdr-alignment/docs/team/crew-v0.3.0/core-review.md
[agent-review]: https://github.com/urtiger101-tw/Crew/blob/feat/crew-herdr-alignment/docs/team/crew-v0.3.0/agent-review.md
[delivery-review]: https://github.com/urtiger101-tw/Crew/blob/feat/crew-herdr-alignment/docs/team/crew-v0.3.0/delivery-review.md
[checklist]: https://github.com/urtiger101-tw/Crew/blob/feat/crew-herdr-alignment/docs/team/crew-v0.3.0/merge-checklist.md
