# Independent plugin fix revalidation

Reviewer: `/root/v05_remote`; author: `/root`.

## Verdict: PASS for PLUGIN-R1 and PLUGIN-R2

Both previously reported P2 findings are **CLOSED at `14a4f64608134f56022ea6abbe10cb209b11e692`**. No additional defect was found in this focused successor diff. The original FAIL review, findings and reproduction remain unchanged in the parent evidence directory.

- Immutable base: `496c750ebf1a7551dde6a9db5737e8f61eae5133`.
- Immutable successor: `14a4f64608134f56022ea6abbe10cb209b11e692`.
- Exported source: `/workspace/scratch/21396cda8665/review-plugin-14a4f64`.
- Independently built binary: `/workspace/scratch/21396cda8665/target-v05-plugin-review/debug/crew`.
- Binary SHA-256: `d4b177c72f8c5c07cfa3cf23757cc2a4e7a69ce8d1ea8711635687c2098a92b8`.
- Production worktrees were not edited; no GitHub or hosted workflow operation was performed.

## Finding resolution

### PLUGIN-R1 — CLOSED

`src/plugins/process.rs:30–40` chooses the executable before assigning child cwd. Explicit relative paths with directory components are joined to the stored absolute plugin root; bare names remain ordinary PATH commands. Both action (`src/plugins/mod.rs:343`) and build (`src/plugins/install.rs:214`) call this helper.

The new unit test deliberately changes the child cwd after helper construction and still executes the plugin executable. The build test executes an actual relative `./bin/build` program and retains its log. Independent CLI reproduction creates valid same-named executable files in caller and plugin directories: only the plugin file runs. Bare `printf` continues to resolve through PATH. A missing relative executable returns failure with a retained `spawn_error` record instead of losing command history.

This closes the reviewed ambiguity through explicit absolute-path construction and Linux runtime evidence. Native Windows/macOS execution remains NOT_RUN; this is not a Windows installer or cross-platform runtime sign-off.

### PLUGIN-R2 — CLOSED

`src/plugins/mod.rs:291–298` lists actions from the approved manifest snapshot and records the current manifest error per entry. The strict `store::loaded` check remains in invocation at line 324.

Independent CLI reproduction verifies enabled and disabled changed manifests, invalid TOML, and missing manifests. The healthy bundled plugin always retains three available actions, the healthy local fixture remains executable, and every stale action exposes `available=false`, `manifest_unchanged=false`, and a nonempty diagnostic. Stale invocation fails. Restoring the exact approved manifest restores availability. The author's updated smoke independently passes changed enabled/disabled isolation while actually invoking the bundled action against a Crew daemon.

## Executed evidence

| Check | Exit | Result |
| --- | ---: | --- |
| `cargo build --locked --offline --bins` | 0 | PASS |
| `cargo test --locked --offline plugins:: --quiet` | 0 | PASS: 12 tests |
| `python3 -B scripts/plugin_smoke.py <review binary>` | 0 | PASS: 7 real fixture groups |
| Independent CLI fix reproduction | 0 | PASS: 9 checks; raw argv, cwd, exit code and output retained |

The unit/build observed logs transcribe the actual tool output. The smoke logs and independent reproduction JSON are captured directly by their runners. All runtime fixtures use temporary local profiles and reviewer/authored programs, with no third-party plugin download.

## Scope and limits

- Focused revalidation covers PLUGIN-R1/R2 plus their direct build/action/list behavior.
- The previous quota/workflow review at `3a00299c4f667cf5a755c5f2497c589a76ed621f` is unchanged; hosted workflows were not run.
- Native Windows/macOS, public GitHub plugin download, Windows installer: NOT_RUN.
- This report does not approve the reviewer's own SSH code or other changes included in the successor commit. SSH has a separate independent reviewer; that reviewer has reported a transport cleanup issue for the remote author to fix.

Next owner: `/root` integrates this focused PASS and handles remaining independent review findings.
