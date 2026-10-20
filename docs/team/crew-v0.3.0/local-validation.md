# Crew v0.3.0 — final local Linux validation

## Result: PASS

All 11 required local validation commands exited **0** against candidate
`efbf228127774bd25c7cf5904485eb27204d1632`. Rust reported **318 passed / 0 failed**;
release tooling reported **5 passed**. All **six smoke suites passed** using the newly built
optimized executables. Worktree smoke also ran its Linux R3/R6 regressions automatically.

This record reports local validation. Windows/macOS execution, GitHub Actions, publication,
and validation of a later code revision are **NOT_RUN by this actor**. Running the agent smoke
is regression evidence; it does not replace the separate independent review of this actor's
previously authored agent/integration implementation.

## Candidate and environment

- Validation actor: `/root/reference_audit`; coordinator: `/root`.
- Tested SHA and ending SHA: `efbf228127774bd25c7cf5904485eb27204d1632`.
- Git tree: `54ab25e4b096519ac9d540f8efd49bf3af943851`.
- Product source review baseline: `a421a6bd3808f5930a00e4dd921298d98ddd449b`.
  `git diff --name-only a421a6b efbf228 -- src Cargo.toml Cargo.lock` was empty.
- Root checkout: `/workspace/scratch/21396cda8665/tmuxlike`.
- `git status --porcelain=v1` was **empty before and after** all checks. The root checkout was
  not edited by validation. Logs and build outputs used separate directories.
- Platform: Linux x86_64; kernel 6.18.55.
- Compiler: `rustc 1.96.1 (31fca3adb 2026-06-26)`; `cargo 1.96.1 (356927216 2026-06-26)`.
- Python: 3.12.14.
- Start: 2026-10-20T05:50:57.999172+00:00; finish: 2026-10-20T05:52:39.168382+00:00.
- Build target directory: `/workspace/scratch/21396cda8665/target-final-validation`.
- Cargo dependency resolution used `--offline --locked` for Clippy, tests and build.
  Format checking performed no dependency update.

## Commands and exits

| Check | Status | Exit | Seconds | Log |
|---|---|---|---|---|
| Formatting (`cargo fmt --all -- --check`) | PASS | 0 | 0.265 | `01-format.log` |
| Clippy (`--all-targets -- -D warnings`) | PASS | 0 | 10.715 | `02-clippy.log` |
| Rust unit tests — 318 passed, 0 failed | PASS | 0 | 11.304 | `03-unit.log` |
| Release tooling self-test — 5 passed | PASS | 0 | 0.165 | `04-release-tooling.log` |
| Optimized build — both crew and tmuxlike | PASS | 0 | 45.084 | `05-build-release-bins.log` |
| Legacy PTY smoke — tmuxlike executable | PASS | 0 | 0.164 | `06-legacy-pty.log` |
| Automation API smoke — crew executable | PASS | 0 | 6.436 | `07-api.log` |
| Workspace/tab smoke | PASS | 0 | 1.669 | `08-workspace.log` |
| Worktree smoke — 13 checks plus Linux R3/R6 regressions | PASS | 0 | 2.271 | `09-worktree-and-races.log` |
| Agent identity/integration smoke | PASS | 0 | 2.471 | `10-agent-identity.log` |
| Bridge/events smoke | PASS | 0 | 0.465 | `11-bridge-events.log` |

Every log is preserved under `/tmp/crew-final-validation/`. The accompanying
`local-validation.json` records exact argv, timestamps, exit codes, log SHA256 values,
source-file SHA256 values and binary SHA256 values.

The Cargo environment used:

```sh
CARGO_HOME=/workspace/scratch/21396cda8665/toolchain/cargo
RUSTUP_HOME=/workspace/scratch/21396cda8665/toolchain/rustup
CARGO_TARGET_DIR=/workspace/scratch/21396cda8665/target-final-validation
```

The build command was `cargo build --offline --locked --release --bins`.
`smoke.py` received `target-final-validation/release/tmuxlike`; the other five smoke suites
received `target-final-validation/release/crew`. All fixture servers used independent sockets
and temporary state/configuration; Git tests used disposable repositories. Socket/PTY tests
ran with the explicitly authorized local execution permission. No paid agent, external
repository, CI run, PR, merge or release was started by this validation.

## R3/R6 evidence from the worktree smoke tail

```text
PASS R3: live shell cwd protects checkout and tracked data
PASS R3: in-use error releases reservation; removal succeeds after leaving
PASS R6: workspace/tab/split rejected under reserved path; unrelated work proceeds
PASS R6: reservation released after successful removal and dirty-error return
PASS worktree race checks (Linux)
```

The tests hold a completed real Git status call at a deterministic test barrier. They verify
that a concurrent workspace, tab or split cannot enter a path being removed, while unrelated
workspace creation remains responsive. The tests also verify reservation cleanup after both
success and dirty-file rejection, and preserve the dirty fixture's contents.

## Release binary digests

Both executables have SHA256:

`8382d48f1970ccedac7c33cf331c944f0c9d89f992935da57053521b86754ac2`

Paths:

- `/workspace/scratch/21396cda8665/target-final-validation/release/crew`
- `/workspace/scratch/21396cda8665/target-final-validation/release/tmuxlike`

## Handoff

The coordinator can use this result with the independent implementation/delivery reviews.
Required remote platform checks must still be associated with the exact PR/release candidate.
This documentation-only commit does not change the tested Rust source or test scripts.
