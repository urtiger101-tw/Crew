# Changelog

## Unreleased

- Windows: restrict DLL lookup to the application directory and System32 before
  any PTY is created, so a `conpty.dll` on `PATH` or in the current directory
  (for example an installed Crew) is never loaded; a sideloaded `conpty.dll`
  beside `crew.exe` is still used. Host-lost tests accept either `conhost.exe`
  or `OpenConsole.exe` as the terminal host.

- Add opt-in edge-triggered auto-focus with off/blocked/on modes, configurable
  idle/dwell/scope guards, per-pane opt-out and stale occupant/generation checks.
  Shared-session focus changes also guard every other client whose view changes.
  Add localized indicators, queued notices, prefix F/a/; navigation and persistent
  `pane auto-focus ID on|off`. Done jumps require a reported or detected agent;
  ordinary shell/build output does not steal focus. Keep recent messages until
  their normal timeout, and let prefix a select the queued target first.
  Prefix a replaces the former sidebar shortcut; studio keeps prefix b for the
  sidebar. Aurora restores prefix b for the workspace tree and uses prefix B
  for the sidebar; custom bindings still win. Auto jumps unzoom
  the target and never send keys or text.

- Add five tmux layout presets, `select-layout`, `next-layout`, prefix Space and
  M-1..M-5, plus JSON `pane layout NAME [--window N] [--session NAME]` and
  `pane.layout` bridge requests. Preserve pane IDs, processes, focus and history;
  multi-pane layout selection unzooms the window. Persist the selected tree and
  cycle position in backward-compatible version-1 snapshots.
- Clear the saved layout cycle position after a manual split, pane close or
  swap, so next-layout starts again at even-horizontal.
- Keep pane bodies nonzero at constrained sizes by omitting only necessary split
  separators. A single-pane layout request is a complete no-op.
- JSON change: pane `cwd` now omits Windows verbatim drive/UNC prefixes
  (`\\?\D:\...` becomes `D:\...`, `\\?\UNC\...` becomes `\\...`).
  Display uses the same form; native paths for process/file I/O are retained.

## 0.6.0 — Agent recovery and terminal UX (2026-11-02)

- Add the studio theme, native Windows clipboard image paste, UTF-8 pane console
  setup and working Ctrl-C delivery to native child programs.
- Reject missing Windows pane shells before starting the UTF-8 helper so worktree
  creation reports the open failure and preserves the completed checkout.
- Add per-command agentcmd PATH plugins, terminal-title and Codex progress display,
  a durable peer/message inbox and bounded worker/receiver integration helpers.
- Avoid stale nesting markers after a daemon restart, preserve wide terminal cells,
  and add Shift-key scrollback navigation and window MRU switching.
- Publish native Windows assets from a local clean build. Hosted CI is NOT_RUN;
  CI/Release workflows stay disabled. U4 submit and U5 lifecycle vocabulary remain
  deferred. Installer compilation does not certify its blocked full lifecycle.

- Keep restored agent panes visibly pending with their saved argv. Resume explicitly
  with `crew pane resume pN` or `prefix R`; repeated requests do not send the command
  again. Duplicate session names include an attach hint.
- Add read-only `crew doctor [--json]` diagnostics for the connected daemon's version,
  executable, PID, start time and color-related environment hazards. Pane environment
  isolation and per-client color behavior retain explicit user overrides.
- Show titles for all panes by default, including single and zoomed panes. Strengthen
  focused borders, show agent names in automatic window labels, and spell out status
  counts. Keep emoji ZWJ sequences together in chrome and exclude terminal controls.
- The reported Claude Code input-only CJK missing/overlapping-text issue remains
  unreproduced. Programmatic split-pane wrap/edit and ANSI replay checks pass;
  Windows IME preedit and native visual acceptance remain unverified.

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
  default buttons (hide, new, split, zoom, next, copy, paste, tree, close, sidebar,
  detach) that run on click or `prefix` then `F1`..`F12`; plain F1 always toggles
  visibility, while `toolbar_fkeys` enables plain F2..F12. Help remains `prefix ?`.
  Kill commands ask through `confirm-before`; clicks are ignored
  while a prompt is open. `toolbar` (default on) and `toolbar-fkeys` can be set at
  runtime; a thin divider is drawn directly below the toolbar, and panes move down
  two rows while both are shown (sessions of 5+ rows). Narrow toolbars show at most
  six buttons.
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
