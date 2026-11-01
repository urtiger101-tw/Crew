# Brief: drop the legacy `tmuxlike` name, keep only `crew`

## Goal
The project was renamed from tmuxlike to Crew. Leftover `TMUXLIKE*` env vars, the
`tmuxlike-<user>.sock` default socket, the `tmuxlike` binary and `$TMUXLIKE` wording now
confuse users (e.g. the nested-attach error says "unset $TMUXLIKE to force", while panes
also export CREW=...). Remove the legacy name from live behaviour and live docs.

## Shared contract (decided by the lead, do not change)
1. Env vars: only `CREW` and `CREW_<SUFFIX>` are read. `TMUXLIKE*` is never read and never
   exported. `brand::env_os/env_var` keep their signatures but only look at `CREW*`.
2. Panes: server exports only `CREW`, `CREW_PANE_ID`, `CREW_SOCKET`; it must also
   `env_remove` every inherited `TMUXLIKE` / `TMUXLIKE_*` var from the pane command so stale
   values from an old parent shell cannot leak in.
3. Nested-attach error (`cli.err.nested`): en "sessions should be nested with care, unset $CREW
   to force"; zh-TW "工作階段不宜巢狀使用，請先取消設定 $CREW 以強制執行".
4. Default socket base name: `crew-<user>.sock` (was `tmuxlike-<user>.sock`). `CREW_SOCKET`
   still overrides it.
5. State migration: default state file is `state-<sanitized socket>.json`. When the socket was
   not overridden, a one-time fallback to the legacy-socket file
   `state-tmuxlike-<user>.sock.json` must keep the user's saved layout, with the SAME
   "never resurrect after the new file was selected/cleared/quarantined" semantics that
   `default_state_path` already implements for the `tmuxlike/` -> `crew/` directory move.
   Candidates checked when the new file was never used: `crew/<legacy file>`, then
   `tmuxlike/<legacy file>`, then `tmuxlike/<new file>`. Legacy files are never copied,
   moved or deleted.
6. Config/data DIRECTORY fallback (`brand::compatible_file`, `tmuxlike/` dirs) stays: it only
   reads an old user file, it is not a live name. Keep it and its tests.
7. Binary: remove the `tmuxlike` bin target (`Cargo.toml [[bin]]`, `src/bin/tmuxlike.rs`).
   Only `crew` is built, packaged, installed. Installer may still delete an old
   `tmuxlike.exe` on upgrade/uninstall (cleanup only) but must not ship it.
8. Reserved env keys for workspace `--env` and plugin build env stripping: `CREW`, `CREW_*`
   only (drop the TMUXLIKE entries).

## Ownership
- Agent A (runtime, Rust): everything under `src/` and `tests/`, plus `Cargo.toml`
  (bin removal only; Cargo.lock regenerates by itself if needed - report it).
- Agent B (build/docs/scripts): `README.md`, `docs/*.md` at top level of docs/ (NOT
  `docs/team/**`, `docs/collab/**`, `docs/releases/**` which are history), `scripts/**`,
  `skills/**`, `installer/**`, `.github/workflows/**`.
- Lead only: `CHANGELOG.md`, this brief, git.

## Hard rules
- Touch only your own files. No git add/commit/stash/checkout/reset.
- No new dependencies.
- Never run `crew kill-server`, never kill crew.exe processes: a live Crew server is in use.
  Any test that starts a server must use an isolated `CREW_SOCKET` and `CREW_STATE` and stop
  only that server.
- `cargo` builds: use `--target-dir target/drop-tmuxlike-<A|B>` so the live
  `target/release/crew.exe` is not overwritten/locked.
- Historical docs keep their wording.

## Acceptance
- Agent A: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  all pass (with your target dir). New/updated unit tests for: env only reads CREW*, state
  legacy-socket fallback (used when new never used; not resurrected after clear/quarantine;
  not used when CREW_SOCKET overrides), default socket name. `rg -i tmuxlike src tests` shows
  only the directory-fallback code/tests of rule 6 and the legacy-state candidates of rule 5.
- Agent B: `rg -i tmuxlike` over your owned paths shows only installer cleanup of an old
  `tmuxlike.exe`. Python scripts still parse (`python -m py_compile` each changed file).
  CI no longer runs the legacy binary smoke.

## Report (final message, <= 40 lines)
Files changed, what each change does, commands run with pass/fail, anything not verified,
anything you think the lead must decide.
