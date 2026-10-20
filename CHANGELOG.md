# Changelog

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
