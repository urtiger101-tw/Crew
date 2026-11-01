# Changelog

## Unreleased

- Drop the legacy `tmuxlike` name. The `tmuxlike` executable is removed, and
  `TMUXLIKE*` environment variables are no longer read or set for panes. Workspace
  `--env` rejects `TMUXLIKE` keys. The default socket is now `crew-<user>.sock`.
  On its first start, a new server adopts the state file of the old
  `tmuxlike-<user>.sock` socket, once. While an old server still listens on that
  socket, the new server neither adopts nor saves state, and the client warns once.
  Stop the old server with `crew server stop` from that version first. Do not use
  `kill-server`, which deletes the state file being migrated. Then restart Crew.
- Keep a pane whose console host died while programs in it still run. The pane
  shows what survives until you close it. If only the root process exits, the
  pane stays live. Known limit: when the host and the shell die together before
  the server has scanned the process tree once, a running child can be orphaned.
  An ignored test in `tests/pane_host_lost.rs` records this.

- Add a function-key toolbar on the top row over the pane columns: twelve localized
  default buttons (help, new, split, zoom, next, copy, paste, tree, close, sidebar,
  detach) that run on click or `prefix` then `F1`..`F12`; `toolbar_fkeys` lets plain
  F-keys run them. Kill commands ask through `confirm-before`; clicks are ignored
  while a prompt is open. `toolbar` (default on) and `toolbar-fkeys` can be set at
  runtime; panes move down one row while the toolbar is shown (sessions of 4+ rows).
- Allow up to 12 custom `[[toolbar_buttons]]` with validated labels (1-24 columns,
  no control, bidi or invisible separator characters) and parsed commands.
- Add `scripts/toolbar_smoke.py`, which drives the toolbar through a real attached
  client (clicks, F-keys, y/n confirmation) and checks geometry and config errors.

## 0.5.1 — Windows preview (2026-10-21)

- Run PowerShell/pwsh workspace and tab startup commands through one-time native
  shell arguments, avoiding the ConPTY initialization/input race. Keep the shell
  interactive and preserve cwd/env; split and cold restore do not replay commands.
- Load signature verification from Windows PowerShell's own security module so
  an inherited PowerShell 7 module path cannot break pinned Inno preparation.
- Add optional single-line sidebar pane segments with literal metadata expansion,
  strict RGB foregrounds and per-segment bold/dim overrides. Share plain text and
  spans with the API; preserve semantic status, layout and monochrome clients.
- Require complete nonce-framed PID, marker and environment fields in the live
  tab-transfer smoke, so partial PTY reads or line wrapping cannot satisfy a probe.

This is a Windows preview with native runtime tests and an Inno-built installer.
Full installer lifecycle acceptance remains blocked by Windows App Control on
the pinned 0.3.0 compatibility fixture. It is not certified as a fully validated
installer release; the 0.4.0 stable release and prior assets remain available.

## 0.5.0 — Presentation, federation and local validation

- Add source-scoped pane/workspace presentation, literal token templates, guards,
  field clearing, TTL and sequence protection. Share computed display with the
  TUI and API without changing agent lifecycle or resource identities.
- Transfer a complete live tab to another workspace while preserving PTYs,
  pane/tab IDs, agent instances, metadata, focus and persistent layout order.
- Save SSH machines, route API requests over native OpenSSH and combine scoped
  workspace/agent views. Add an interactive federation selector with real remote
  attach/detach, bounded probes, partial failures and explicit reconnection.
- Add a bundled plugin marketplace, optional public GitHub discovery, manifest
  inspection, install/link/update, enable/disable, bounded argv actions and logs.
  Require installation trust and keep config/state and previous installations.
- Detect agent kinds from live process evidence with platform-specific native
  backends. Explicit reports remain authoritative; hints do not complete waits.
- Replace automatic hosted push/PR/tag runs with an opt-in manual workflow.
  Add local validation and native packaging with source, command, log and binary
  hashes. Native Windows local runs retain the full Inno installer lifecycle.

This release implements bounded behavior subsets. It does not add Herdr's full
styled row/grid templates, plugin pane/event engine, screen-state detection,
remote pane embedding or cross-host PTY transfer. See `docs/herdr-parity.md`.
Local evidence covers its named native platform only. Windows/macOS and installer
execution must be reported from actual native runs; no CI run ID is synthesized.

## 0.4.0 — Ordering, metadata and Windows setup

- Add workspace/tab ordering with zero-based insertion slots. Preserve stable IDs,
  running PTYs, agent instances, active resources and pending UI selections.
- Add bounded pane/workspace token patches, per-key TTL and per-source sequence
  protection. Expose metadata in resource snapshots and update/expiry events;
  keep it independent from agent lifecycle and clear it on cold restart.
- Add a per-user Windows x64 Inno Setup installer with a stable upgrade identity,
  Start Menu entry and uninstall support. Preserve configuration/state, unrelated
  files and PATH, and refuse replacement while installed executables are in use.
- Build Windows release executables with a static CRT and verify their PE imports.
  Keep the portable ZIP alongside the installer and three Unix archives.
- Gate releases on native ordering/metadata smokes and Windows installer build,
  installation, version replacement, busy-binary refusal and uninstall checks.
  Include the setup executable in source/CI provenance and release digest checks.

This is an additional Herdr behavior subset: cross-workspace tab/pane moves,
metadata presentation overrides/templates and the other gaps in
`docs/herdr-parity.md` remain separate work. Setup executables are unsigned.
An upgrade test fixture wraps released 0.3.0 binaries; no 0.3.0 installer was
previously published. Actual native validation is recorded by the release's CI.

## 0.3.0 — Crew

- Rename the product to Crew and move PR/CI/releases to urtiger101-tw/Crew.
  Preserve `tmuxlike` executable and environment compatibility.
- Add workspace/tab CRUD with stable IDs, focus control, cwd/env and startup commands.
- Add real Git worktree list/create/open/remove and guarded removal.
- Bind agent wait/prompt to an occupant instance; provide generic hooks and diagnostics.
- Add bounded events and a concurrent, request-correlated JSON-lines bridge.
- Preserve workspace/tab IDs and environment overrides across layout restore; prevent
  stale legacy state from returning after the new state is cleared or quarantined.
- Isolate all smoke tests from the caller's Crew server. Fix active tab indexing after
  deleting a preceding tab and current-tab lookup from background panes.
- Package Crew and its legacy executable together with checksums and source provenance.

See `docs/herdr-parity.md` for the implemented subset and remaining Herdr differences.
Processes do not survive server restart. Upgrade by saving work and stopping the old server
before using the new API. Windows/macOS are verified by the release's actual CI run.

Earlier tmuxlike history is preserved in Git.
