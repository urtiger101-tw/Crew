# Local validation and native packaging

`scripts/validate_local.py` runs Crew's required checks on the current machine and
writes evidence with `provider: "local"`. It does not call GitHub, start Actions,
create a release, or supply a GitHub run ID. `scripts/package_local.py` accepts
that evidence only for the same clean checkout and the same tested binaries.

This is a separate path from `scripts/package_release.py`. The existing release
publisher continues to require its own hosted CI and release provenance.

## Run a complete native check

Use Python 3.11 or later, Git, and the Rust version in `rust-toolchain.toml`.
Install the toolchain and dependencies before requesting an offline run. Run from
a committed checkout; validation can diagnose a dirty checkout, but packaging
rejects it even when all commands pass.

```sh
python3 scripts/validate_local.py --offline \
  --output ../Crew-local-evidence \
  --target-dir ../Crew-local-target
```

On Windows, use the same arguments with the installed Python interpreter:

```powershell
python scripts/validate_local.py --offline `
  --output ..\Crew-local-evidence `
  --target-dir ..\Crew-local-target
```

The output directory must be new or empty. A build directory can be reused.
Both directories must be outside the checkout, or inside a Git-ignored directory
that contains no tracked files. `target/` is an allowed location. Source and Git
metadata directories are refused. There is no `--skip` or reduced check profile.

`--offline` passes `--offline` to Cargo, sets `CARGO_NET_OFFLINE=true`, and requires
cached pinned Windows installer dependencies. It does not skip the loopback SSH
test. Without this option, Cargo and the Windows preparation step can fetch their
required public dependencies. No step contacts an agent model provider.

Linux SSH validation requires `ssh`, `sshd`, `ssh-keygen`, and a usable native
PTY/loopback environment. A restricted execution environment must allow the real
local socket and PTY operations. A permission error is a failed or blocked check;
it never becomes a successful skip. In a Linux container where root lacks the
audit capability required by `sshd`, an authorized root caller can select an
existing non-root fixture account:

```sh
python3 scripts/validate_local.py --offline \
  --output ../Crew-local-evidence --target-dir ../Crew-local-target \
  --ssh-fixture-user oai
```

The option is forwarded as `--fixture-user` to the SSH smoke. It does not create
an account or change the account's home configuration. The harness records the
caller and actual fixture identity. Omit the option on an ordinary native Linux
user account. The validator records the selected option and exact command.

## Required commands and platform coverage

Every native run requires these commands, in order:

| Group | Required checks |
| --- | --- |
| Tools | `rustc -vV`, `cargo --version`, `git --version`, Python version |
| Rust | Format, Clippy with warnings denied, locked unit tests |
| Tooling | Release tooling self-tests and local validation/packaging regression tests |
| Build | Locked release build of `crew` and `tmuxlike`, both version commands |
| Existing smoke tests | Legacy PTY, API, workspace, workspace ordering, metadata, worktree, agent identity, bridge/events |
| New smoke tests | Cross-workspace tab transfer, metadata presentation, agent detection, plugin installation/lifecycle |

The exact script names and commands are in `SMOKES` and `command_plan()` in
`scripts/validate_local.py`. Changes to the list are source changes and require
new evidence. Missing scripts, duplicate or missing command records, nonzero
exits, `NOT_RUN`, and `SKIP`/`SKIPPED` results cannot satisfy a required check.
Zero-exit smoke output that explicitly starts a line with a skip marker is also
rejected. A failed build prevents smoke tests from accidentally using stale
binaries; their entries remain `NOT_RUN`.

| Native target | Additional required check | Explicitly unexecuted coverage |
| --- | --- | --- |
| Linux x86_64 GNU | Real loopback SSH federation smoke | Windows Inno build and all 14 installer lifecycle checks; Windows and macOS execution |
| Windows x86_64 MSVC | Inno build and all 14 native installer lifecycle checks | Linux-only SSH fixture; Linux and macOS execution |
| macOS arm64 or x86_64 | Common checks above | Linux-only SSH fixture; Windows installer checks; other native targets |

The Rust host must match the actual supported native platform. A cross-compiled
executable cannot satisfy a native run. The validator checks the binary format
and architecture, and checks static CRT imports on Windows. Windows builds add
`-C target-feature=+crt-static` to `RUSTFLAGS`; an inherited
`CARGO_ENCODED_RUSTFLAGS` is refused because it would override that requirement.

A successful result has `result: "PASS"` and `scope: "native-platform"`.
`matrix_result` remains `NOT_RUN`. Missing platform execution is listed under
`unavailable_checks`; a Linux result never claims an installer was built or
tested. Windows/macOS SSH behavior also remains unverified by this Linux-only
SSH harness. A complete multi-platform release needs separate native evidence
and its own release decision.

## What the evidence contains

`validation.json` records:

- The local provider, native target, OS, architecture, tool locations and
  versions, expected pinned Rust version, selected build flags, and timestamps.
- The starting and ending commit SHA, Git tree SHA, index digest, dirty status,
  and SHA-256/size/type for every tracked and nonignored untracked source file.
  Symlink targets are recorded as links. `assume-unchanged` and `skip-worktree`
  flags cannot make a checkout eligible for packaging. Submodule source entries
  are rejected because this inventory does not claim to verify their contents.
- Every required command's argument array, working directory, start/end time,
  exit code, result, error, and retained log path/size/SHA-256. Missing tools and
  exceptions are recorded; they cannot be treated as exit zero.
- Both release binary paths, sizes, and SHA-256 hashes immediately after the
  build and at the end of validation. A source or binary change makes the run
  fail. The original failed logs remain in that run's output directory.
- The full log inventory and, on Windows, the installer artifact inventory and
  its native lifecycle record.

The runner writes incremental evidence after each command. It runs the remaining
independent checks after a failure, while preserving blocked dependencies. Use a
new output directory for a rerun instead of overwriting the earlier evidence.

These hashes establish consistency between a local run and its artifacts. They
are not a digital signature or an independent attestation; an actor who controls
the checkout, binaries, and evidence can fabricate a complete local record.
Keep the original logs and obtain independent review before publication. Build
environment variables unrelated to the selected build flags and secret values
are not copied into the report.

## Native Windows Inno validation

The validator calls `windows_installer_local.py` to prepare the current native
portable payload and compile the existing Inno script with the pinned compiler.
This preparation step alone is not successful validation. Its package and
installer manifests identify the local provider, source snapshot, and exact
binary/payload hashes, with no `ci_run_id` or `ci_attempt`.

Next, `windows_installer_smoke.py --local-provider` verifies that provenance and
runs the existing native lifecycle code. The default CI/release branch remains
separate. Local records are not accepted as CI records, and hosted records are
not accepted through the local branch. The shared contract still requires all
14 lifecycle checks, including actual installation, both installed commands,
payload digests, shortcut and HKCU registration, PATH/config/state preservation,
foreign-directory rejection, busy upgrade/uninstall rejection, daemon survival,
real pinned 0.3.0 upgrade, downgrade rejection, and safe uninstall behavior.

Use an isolated Windows user or runner without an existing Crew installation or
shortcut. The existing smoke refuses to overwrite either. It performs real
current-user installs and uninstalls under temporary directories. Linux tests of
the evidence validator use synthetic PE fixtures only; they do not count as
Windows installation execution.

For an offline Windows run, these public dependencies must already be present
and match the sizes and SHA-256 values in `windows_installer_contract.py`:

| Dependency | Cache path relative to `%RUNNER_TEMP%`, or the Python temporary directory when unset |
| --- | --- |
| Inno Setup 7.1.0 compiler installer | `crew-inno-7.1.0/innosetup-7.1.0-x64.exe` |
| Real Crew 0.3.0 portable ZIP for upgrade validation | `crew-local-dependencies/Crew-v0.3.0-windows-portable.zip` |

A prior online local run populates these caches. Missing or altered cached files
fail an offline run; there is no download fallback. The compiler's Authenticode
publisher and engine version checks remain active.

## Produce a native local package

After the complete run passes against a clean committed checkout:

```sh
python3 scripts/package_local.py \
  --evidence ../Crew-local-evidence/validation.json \
  --target-dir ../Crew-local-target \
  --output ../Crew-local-dist
```

`--target-dir` is an optional additional match check, not a way to substitute
another binary. Packaging is always local and performs no network requests.
The evidence must remain in its original output directory with its logs and
installer files. A copied JSON file alone is not accepted as complete input.

The packager independently reconstructs the current required command list,
verifies each exact argument array and zero exit, rehashes logs and binaries,
checks the current clean source against both recorded snapshots, and repeats
the checks after assembling the archive. Missing tests, changed source or
binary bytes, edited logs, stale commit evidence, or an incomplete Windows
lifecycle prevent output creation.

The output contains:

- One `Crew-vVERSION-TARGET-local.tar.gz` or `-local.zip` for the actual native
  target, with `crew`, the `tmuxlike` compatibility command, README, LICENSE, and
  a local package manifest. Unix command executable permissions are retained.
- `local-manifest.json`, the source/evidence/artifact relationships and explicit
  native coverage; `local-validation.json`, a copy of the original report; and
  `SHA256SUMS` for the generated artifacts.
- On native Windows only, the exact Inno installer and lifecycle sidecar from
  the verified run. Its embedded payload manifest matches the portable package.

The report copy is for distribution and inspection, not a replacement for the
original log directory. The local manifest declares `published: false`. No
package creation step pushes a branch, opens a PR, or publishes a GitHub release.

## Tooling regression tests

```sh
python3 -B scripts/test_local_validation.py
python3 -B scripts/package_release.py self-test
```

The local suite uses temporary Git repositories and synthetic evidence to test
rejection of missing/duplicate/skipped checks, changed source/commit/binaries,
log drift, unexpected commands, output paths that could hide sources, invalid
provider/platform claims, source changes during packaging, offline cache misses,
and separation of local versus CI installer provenance. Native product smokes
and real Windows lifecycle execution remain separate required evidence.
