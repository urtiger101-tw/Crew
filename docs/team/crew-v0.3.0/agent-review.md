# Independent AGENT review and R5 verification

## Decision and scope

**PASS for AGENT occupant identity, explicit hook diagnostics, and the R5 fix at tested head `a421a6bd3808f5930a00e4dd921298d98ddd449b`.** No open blocking findings remain in this review scope.

Reviewer: `/root/target_audit`. AGENT author: `/root/reference_audit`. R5 fix author: `/root`. These are separate agent instances. The reviewer authored CORE/R4; CORE, R4, GIT, bridge, and whole-PR approval are outside this review.

Reviewed files:

- `src/api/agent.rs`
- `src/integrations.rs`
- `scripts/agent_identity_smoke.py`

| Revision | Role |
|---|---|
| `1ece67877e3977e9c7a65ab7cd1b9c1f14b15233` | Original AGENT base |
| `6f85821dff5f108b707ed47eb41eba2837932cc0` | Original independent AGENT review and reproduction of R5 |
| `cb85172038b99265a1467f29dd8c9c983ec42f25` | R5 fix: printable path plus explicit `path_lossy` flag |
| `3e24e5c273c8243faf472f17e34c85020842d0d4` | Requested integrated review candidate |
| `a421a6bd3808f5930a00e4dd921298d98ddd449b` | Actual integrated build and final verification |

The branch advanced to `a421a6b` before the independent build. Its three reviewed file blobs exactly match `3e24e5c`. HEAD and source hashes were checked again after testing. The AGENT implementation is unchanged from `6f85821`; later changes in this scope are the R5 diagnostic conversion and case-insensitive environment cleanup in the smoke harness.

## Finding R5 — P2, resolved

Location: `src/integrations.rs:175`.

The original doctor command put a `PathBuf` directly into `json!`. A valid Unix PATH directory containing byte `0xff` caused serde to reject the path and `json!` to panic. The independent reproduction on `6f85821` returned exit **101**, no JSON, and `path contains invalid UTF-8 characters`.

The fix renders the path with `to_string_lossy()` and adds `path_lossy`, calculated from `path.to_str().is_none()`. It preserves valid Unicode paths and truthfully identifies when the diagnostic display replaced invalid bytes.

Independent final results:

| Fixture | Exit | Observed result |
|---|---:|---|
| Executable `codex` in a valid non-UTF-8 Unix directory | 0 | Valid JSON; `executable_found: true`, `path_lossy: true`, replacement character visible |
| Executable `codex` in a Chinese directory name containing spaces | 0 | Exact Unicode path; `path_lossy: false` |
| Empty PATH directory | 0 | `executable_found: false`, empty matches |

Each executable fixture would create a marker if run. The marker remained absent. Doctor kept `hook_status: unverified`, `evidence: path-file-check-only`, `agent_started: false`, and `server_probed: false`. No provider, coding agent, or server was started by this diagnostic regression.

## Behavioral review

The reviewer traced target resolution, reporting, release, sequence filtering, metadata merging, waits, prompt submission, and integration CLI transport.

- A wait resolves one occupant instance, checks that instance at each poll, and captures the matched status and pane snapshot under one State lock.
- Poll sleeps and the 150 ms prompt submission delay occur after releasing the State lock.
- Prompt paste and delayed Enter each verify the pinned occupant while holding the write lock. A reported replacement or release fails the old operation; a newly blocked occupant does not receive the delayed Enter.
- Accepted reports retain identity only for the same source and kind. Release/re-report allocates a new identity, including identical source/kind reports arriving between polls.
- Equal or lower sequence reports from the current source are ignored and emit no status event. Stale and foreign-source releases preserve the current occupant.
- Rename preserves occupant identity. An unrelated source does not inherit the old name or resume command; same-kind restored metadata retains the existing takeover behavior.
- Integration output distinguishes explicit reports from PTY activity heuristics and states that executable presence is not verified hook installation or process identity. It does not execute PATH candidates or install provider configuration.

## Commands and evidence

Platform: **Linux x86_64**, `rustc 1.96.1 (31fca3adb 2026-06-26)`.

Cargo used these session-local toolchains:

```text
CARGO_HOME=/workspace/scratch/21396cda8665/toolchain/cargo
RUSTUP_HOME=/workspace/scratch/21396cda8665/toolchain/rustup
cargo=/workspace/scratch/21396cda8665/toolchain/cargo/bin/cargo
```

| SHA | Command / cwd | Exit | Status |
|---|---|---:|---|
| `6f85821` | `cargo test --offline --locked --quiet` in `wt-agent`, target directory `target-agent` | 0 | PASS: 295 tests |
| `6f85821` | `cargo build --offline --locked --quiet` in `wt-agent` | 0 | PASS |
| `6f85821` | `python3 scripts/agent_identity_smoke.py /workspace/scratch/21396cda8665/target-agent/debug/crew` in `wt-agent` | 0 | PASS |
| `6f85821` | Supplementary independent PTY checks in `/tmp/crew-agent-review-6f85821.py` | 0 | PASS for release, responsiveness, rename, and timeout cases; R5's child process failure was recorded separately |
| `a421a6b` | `cargo build --offline --locked --quiet` in `tmuxlike`, default target directory | 0 | PASS |
| `a421a6b` | `python3 /tmp/crew-r5-recheck.py /workspace/scratch/21396cda8665/tmuxlike/target/debug/crew` in `tmuxlike` | 0 | PASS: all three R5 cases |
| `a421a6b` | `python3 scripts/agent_identity_smoke.py /workspace/scratch/21396cda8665/tmuxlike/target/debug/crew` in `tmuxlike` | 0 | PASS |
| `a421a6b` | `git status --short`, before and after validation | 0 | PASS: no source modifications |

Actual checkout directories above are under `/workspace/scratch/21396cda8665/`. The real-server smoke used its own socket and temporary config/state, and cleaned up only that server. Unix socket/PTY execution needed the host's approved local-test sandbox escalation. No remote CI, PR, merge, publication, account setting, model, provider, or billing operation was performed by this review.

Final identity smoke passed ordinary-terminal rejection, stable identity on accepted updates, stale-report rejection without events, source replacement, release/re-report, kind replacement, generic hooks, blocked prompt rejection, and raw PTY checks that replacement/block races receive no delayed Enter.

Supplementary original checks also verified that another API responds during a pending wait, rename keeps a wait attached to the same occupant, timeout 0 writes nothing, and timeout 50 ms pastes text without Enter. Two preliminary supplementary-script runs had reviewer-harness `KeyError`s from assuming private `resume`/`seq` fields appeared in public JSON. After correcting the script against `pane_json`, its final run passed; those errors were not product findings.

## Evidence hashes

| Artifact at `a421a6b` | SHA-256 |
|---|---|
| `src/api/agent.rs` | `a73c001cd2d5cfb8acd50b6bedd0267a6dfc6d895e02aff2067a52faf3347027` |
| `src/integrations.rs` | `eb4296e8abde6229a01d5c57f89afd5be527100eb2183c2d7bd1a81649d2569e` |
| `scripts/agent_identity_smoke.py` | `6631f0211c69808d82cff1def3214e082065f95516f38ce9e1077bb5c1f8c0c8` |
| Tested `target/debug/crew` binary | `ca376dbbf3db8f871bb1b9cc9a1e4304578db3d36a4ad5d5a44030c4f1676869` |

## Limits and unexecuted checks

- **Inherited ordering limitation, observed:** working report seq 2, release seq 3, then late idle report seq 2 from the same source recreates metadata. Release discards the sequence high-water mark. This behavior exists in the original base and is outside the identity fix. Unique sources per process and no reports after exit are required by the generic hook contract; stronger delivery ordering would require tombstones or generations.
- **Inherited prompt semantics:** when the occupant is already Working, the start phase is considered complete. The API does not correlate completion to a particular prompt generation; an existing task may turn idle during the submission delay.
- Same source and kind without a release represent one occupant. Crew cannot infer an unreported real-process replacement from these fields.
- Windows and macOS actual execution: **NOT_RUN by this reviewer**. Real Claude/Codex/provider hooks: **NOT_RUN**. The delivery is a generic Crew hook contract, with no official provider-format compatibility claim.
- Full latest-head unit/clippy/CI matrix and CORE/R4/GIT/bridge review: **outside this review scope**; use their respective owners' evidence.

Read the team-review skill, reviewer role, handoff contract/template, and task dispatch/workflow rules. The provided temporary skill reference had no root `AGENTS.md`; the package resource was unavailable, so the already-provided project workflow rules were followed and the coordinator was informed. Source files under review were never edited by this reviewer.

## Reproducible R5 regression

Save this script as `crew-r5-recheck.py` and invoke it with an absolute path to the freshly built Crew binary. It requires Unix only because arbitrary non-UTF-8 filenames are the regression fixture. It creates and removes only its own temporary files.

```python
#!/usr/bin/env python3
"""Independent R5 regression: read-only doctor, no server or provider launched."""
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
assert os.name == "posix", "This regression covers valid non-UTF-8 Unix pathnames"
with tempfile.TemporaryDirectory(prefix="crew-r5-review-") as directory:
    root = Path(directory)
    marker = root / "must-not-execute"
    base_env = {k: v for k, v in os.environ.items() if not k.startswith(("CREW", "TMUXLIKE"))}
    cases = [(os.fsencode(directory) + b"/invalid-\xff", True),
             (os.fsencode(root / "路徑 with spaces"), False)]
    for path, lossy in cases:
        os.mkdir(path)
        executable = path + b"/codex"
        with open(executable, "wb") as stream:
            stream.write(("#!/bin/sh\nprintf ran > " + shlex.quote(str(marker)) + "\n").encode())
        os.chmod(executable, 0o700)
        completed = subprocess.run([binary, "integration", "doctor", "codex"],
                                   env=base_env | {"PATH": os.fsdecode(path)},
                                   capture_output=True, text=True, errors="strict", timeout=5)
        assert completed.returncode == 0, (completed.returncode, completed.stdout, completed.stderr)
        assert not completed.stderr, completed.stderr
        result = json.loads(completed.stdout)["result"]
        agent = result["agents"][0]
        match = agent["matches"][0]
        assert match["path_lossy"] is lossy, match
        assert match["command"] == "codex", match
        assert match["path"].endswith("/codex"), match
        if lossy:
            assert "\ufffd" in match["path"], match
        else:
            assert match["path"] == os.fsdecode(executable), match
        assert agent["executable_found"] is True, agent
        assert agent["hook_status"] == "unverified", agent
        assert agent["evidence"] == "path-file-check-only", agent
        assert agent["agent_started"] is False, agent
        assert result["server_probed"] is False, result
        assert not marker.exists(), "doctor executed the diagnostic executable"
        print(json.dumps({"case": "non-utf8" if lossy else "utf8-spaces",
                          "exit_code": completed.returncode, "result": "PASS",
                          "path_lossy": match["path_lossy"], "executable_found": True,
                          "agent_executed": marker.exists()}, ensure_ascii=False))
    empty = root / "empty"
    empty.mkdir()
    completed = subprocess.run([binary, "integration", "doctor", "codex"],
                               env=base_env | {"PATH": str(empty)},
                               capture_output=True, text=True, timeout=5)
    assert completed.returncode == 0, completed
    agent = json.loads(completed.stdout)["result"]["agents"][0]
    assert agent["executable_found"] is False and agent["matches"] == [], agent
    assert not marker.exists()
    print(json.dumps({"case": "missing-executable", "exit_code": 0, "result": "PASS"}))
```
