# Independent plugin and hosted-quota review

Reviewer: `/root/v05_remote`; author: `/root`.

## Scope and immutable source

- Plugin candidate: `f615aada7bca0ad1a32e853a6032fadb63ea0385` against `e03ac1872cffca2c67daf91912636ba57a5e3e7a`.
- Added workflow/release-contract/docs delta: `c9b9216..3a00299c4f667cf5a755c5f2497c589a76ed621f`.
- The plugin source, bundled manifests, plugin smoke and plugin documentation are unchanged between the two reviewed heads.
- Production worktrees were not edited. The candidate was exported to an isolated source snapshot and built in a separate target directory.
- This review does not approve the reviewer's own SSH implementation, and does not claim GitHub workflow execution or native Windows/macOS validation.

## Verdict: changes requested

Two P2 findings remain open. See `findings.json` for exact source locations and expected behavior.

1. **PLUGIN-R1 — Relative executable path resolution.** Both build and action commands call `Command::new(argv[0])` then set child cwd. An explicit relative executable is not resolved against the approved plugin root before spawning. Rust documents this combination as platform specific and unstable. Resolve paths such as `./bin/action.exe` against the plugin root, leaving bare PATH commands unchanged. This finding is based on source and the official process contract; no Windows runtime pass is claimed. [Rust contract](https://doc.rust-lang.org/std/process/struct.Command.html#method.current_dir).
2. **PLUGIN-R2 — Disabled stale link blocks unrelated action discovery.** Actual reproduction installs a healthy bundled plugin, links another plugin disabled, and adds a comment to its manifest. Global `plugin action list --json` exits 1 without any healthy results; `--plugin crew.workspaces` still lists three valid actions. Diagnostics should remain local to the broken plugin. Invocation should continue refusing changed manifests. Full command/output evidence is in `disabled-link-isolation.json`.

## Executed evidence

All commands ran on Linux with Rust 1.96.1 and the isolated candidate source, using no third-party plugin code.

| Check | Exit | Result |
| --- | ---: | --- |
| `cargo build --locked --offline --bins` at f615aad | 0 | PASS |
| `cargo test --locked --offline plugins:: --quiet` at f615aad | 0 | PASS: 11 tests |
| `python3 -B scripts/plugin_smoke.py <candidate binary>` | 0 | PASS: 7 actual fixture groups, simulation=false |
| `python3 -B scripts/package_release.py self-test` at f615aad | 0 | PASS: 11 tests |
| `python3 -B scripts/package_release.py self-test` at 3a00299 | 0 | PASS: 11 tests |
| Parsed both workflow trigger/guard declarations and compared required CI step names at 3a00299 | 0 | PASS |
| Disabled-link isolation reproduction | 0 | CONFIRMED the failing product behavior; actual action-list exit=1 |

The unchanged root audit failure from SSH testing belongs to the SSH implementation handoff, not to this review.

## Quota and release-gate assessment

PASS for the reviewed source: CI and Release only expose `workflow_dispatch`; each usage input defaults false. CI test/gate and Release verify/publish have explicit authorization guards. Release build depends on guarded verify and has no `always()` override. There are no automatic push, pull_request, merge_group or tag triggers.

The release verifier still requires exactly all three native platform jobs plus the CI gate, every required step completed successfully, exact source SHA/repository/main branch, latest run/attempt, complete artifacts and installer provenance. It now accepts main workflow_dispatch as well as historical main push runs. The wrong event/branch/repository, skipped job/step and stale run cases remain rejection fixtures. The additional five common checks and Linux SSH check match actual YAML step names. Windows retains all 14 installer lifecycle requirements.

The reviewed README/parity changes distinguish local native evidence from hosted CI and explicitly mark unexecuted OS coverage. They describe SSH as a selector with POSIX remote requirements, plugins as explicit argv actions with their own manifest, and remaining Herdr gaps. No unsupported full compatibility or all-platform release validation claim was found in this scope.

## Limits and next action

- Windows/macOS command behavior and Windows installer execution: NOT_RUN here.
- Public GitHub discovery/fetch: NOT_RUN; source/ref/path/transport behavior was inspected, but no remote plugin was downloaded or executed.
- Hosted workflows and GitHub writes: NOT_RUN / not performed, as requested.
- No parent-owned source changes were made. Root should fix the two findings and provide an immutable successor SHA for focused revalidation.
