# Changelog

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
