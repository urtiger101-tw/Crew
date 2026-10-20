# Crew v0.3.0 — independent CI and release review

**Result: PASS for the four reviewed delivery files after DELIVERY-1 was fixed.**
This is a source and local tooling review. GitHub-hosted CI, native release builds,
and a real draft/publication were **NOT_RUN by this reviewer**. This report is not
an assertion that a release exists or that every target platform has passed.

## Scope and independence

- Reviewer: `/root/target_audit`, acting as `independent_reviewer`.
- Implementation and fix author: `/root/worktree_worker`, a separate agent instance.
- Base: `2f01881f7268918879852eae4255e159d669d74d`.
- Original delivery commit: `f7a9dde5099c9f8b58570a91dfa753d221715c33`.
- Fix and independently tested head: `ec2329ed75d8e9674ed665f483183ee4584a9403`.
- Reviewed source: `.github/workflows/ci.yml`, `.github/workflows/release.yml`,
  `scripts/package_release.py`, and `rust-toolchain.toml`.
- Source worktree: `/workspace/scratch/21396cda8665/wt-worktree`, clean at verification.
- Reviewer-authored CORE/R4 code is excluded. This is not a whole-PR approval.

The original four delivery files were also inspected in the integrated source tree
at observed head `938d8522a6f3a06ad44fe9de75d66f8aaf085948`, where every referenced
smoke script exists and those four files matched `f7a9dde` byte for byte. The author branch is a scoped delivery
branch and does not independently contain all other workers' feature commits.
Root must integrate the reviewed delivery commits with those implementations.

The review followed the team workflow's independent-review, handoff, CI and release
contracts supplied under `/tmp/crew-workflow-reference`, and the project's
`dispatch-plan.md` and `workflow-rules.md`. No reviewed source was edited by the
reviewer. No GitHub mutation, workflow trigger, tag, PR, release, or remote push
was performed by the reviewer.

## Findings

### DELIVERY-1 · P2 · RESOLVED — CI could become stale during asset uploads

**Original location:** `scripts/package_release.py`, original `publish()` final
asset check and `draft=false` update, approximately lines 313–366 in `f7a9dde`.

The original publisher checked the latest successful CI run/attempt before creating
its draft, then uploaded six assets and published without checking CI again. A
rerun could change that exact source's latest attempt to a failed or running
attempt during uploads. The release would still become public using stale CI
success, contradicting the script's run/attempt binding.

The independent offline reproducer below simulates GitHub at the API boundary.
After the sixth upload, it changes CI from attempt 1/success to attempt 2/failure.
On the original code it observed `published=true`, `ci_reads=1`, and exited **1**.
No network call was made, and the archive payloads are explicitly synthetic.

Author fix `ec2329e` factors `verify_ci()` out of source verification and invokes
it again immediately before the public update. It verifies the current matching
run ID, attempt, completed/success status, all expected jobs, and their required
steps. The finalizer also checks every asset's uploaded state, size and SHA256
digest against the verified local files, and rechecks the tag's source SHA.

The unchanged reproducer on `ec2329e` observed `published=false`, `ci_reads=2`,
and the error `latest CI run for the source SHA is not successful; wait for or fix CI`.
The whole independent scenario suite exited **0**. The successful scenario still
uploads six assets and publishes once. **No unresolved P1/P2 findings remain in scope.**

## What was checked

### Source, CI and artifact identity

Release dispatch is limited to `main`; a tag push must match the requested stable
version tag. The checked-out HEAD, peeled trigger commit, Cargo package/version,
and Rust toolchain are checked. The selected source must be in `main` history.
Existing tags pointing elsewhere and existing releases or drafts are rejected.

The CI query is restricted to this repository's `main` push for the exact source
SHA. The latest matching run must succeed. Its three platform jobs and `Crew CI Gate`
must all be present and successful, with each named required step successful;
missing, skipped, cancelled or unsuccessful checks do not count as success.
The verified run ID and attempt are carried into package provenance and checked
again during publication, including after asset upload.

Build and publication checkouts use the verified SHA. Aggregation requires all
four distinct target archives and their manifests, verifies archive membership,
embedded manifests and file hashes, and writes the release manifest/checksums.
The publisher requires exactly the resulting six assets. It creates a draft,
checks every upload, then validates the entire remote asset set before making
one public-state update. It does not upload incrementally to an already public
release. Upload failure, a missing asset, a changed tag, and stale CI all retained
the draft in independent local simulations.

### Permissions and CI behavior

CI uses `contents: read`; release verification/build use `contents: read` and
`actions: read`. Only the publish job has `contents: write`, and its first step
requires successful verification and every matrix build. Checkout credentials
are not persisted. Every action reference is a fixed 40-character upstream
commit. The action commits were checked against the official action repositories.
No `pull_request_target`, self-hosted runner, `continue-on-error`, or ignored
required failure was present in the reviewed workflows.

Both workflows specify `shell: bash`; GitHub documents that explicit Bash includes
`pipefail`, so piping CI output through `tee` retains failure status [S2]. User
release-tag input is passed through quoted environment variables and the script
validates its literal version format. The API helper rejects paths outside
`urtiger101-tw/Crew` before any network operation; the independent test exercised
that boundary using the original source repository's path.

### Platform feasibility and evidence boundary

The official runner reference documents `ubuntu-24.04` and `windows-2025` as x64,
`macos-15` as arm64, and `macos-15-intel` as Intel [S1]. This matches the three-platform
CI matrix and four native release targets. Windows steps use Bash provided by
Git for Windows [S2], Python 3.12 and `.exe` paths. The integrated smoke scripts
contain explicit Windows handling and isolate their configuration/state/socket
settings. The release build runs the smoke scripts against the native target
binaries before packaging. Rust is pinned to 1.96.1.

This establishes workflow configuration and a plausible native execution path.
It does **not** establish that hosted runners have actually built or passed those
checks. Those outcomes remain for the coordinator to collect from the real CI run.

## Independent commands and results

Final commands ran on Linux x86_64 (`Linux-6.18.55-x86_64-with-glibc2.39`), Python
3.12.14. Unless indicated otherwise, cwd was
`/workspace/scratch/21396cda8665/wt-worktree` at `ec2329e`.

| Check | Command | Exit | Result |
|---|---|---:|---|
| Release validator tests | `python3 scripts/package_release.py self-test` | 0 | PASS: 5 tests, including 13 final-publication scenarios |
| Independent complete publish-path scenarios | `python3 /tmp/crew-delivery-review.py /workspace/scratch/21396cda8665/wt-worktree/scripts/package_release.py` | 0 | PASS: success, upload failure, missing asset, moved tag, failed CI rerun, initial changed attempt, repository fence |
| Workflow syntax and expression lint | `/tmp/crew-actionlint/actionlint .github/workflows/ci.yml .github/workflows/release.yml` | 0 | PASS: actionlint 1.7.12, no diagnostics |
| Source cleanliness | `git status --short` | 0 | PASS: empty output |
| Hosted CI / native four-target build / live release | Not executed by reviewer | — | NOT_RUN |

The author self-test's 13 final-publication cases cover success; failed/running CI;
a new run or attempt; skipped job/step; missing asset; wrong asset state, size or
digest; absent digest; and a changed tag. These are real executed tests of mocked
GitHub responses, not live GitHub success evidence.

Independent complete-path output after the fix:

```text
success: PASS, published=true, uploads=6, ci_reads=2
upload_failure: PASS, published=false, uploads=2, ci_reads=1
asset_missing: PASS, published=false, uploads=6, ci_reads=1
tag_changed: PASS, published=false, uploads=6, ci_reads=1
ci_rerun_failed: PASS, published=false, uploads=6, ci_reads=2
ci_attempt_changed_before_publish: PASS
repository_fence: PASS
All scenarios: remote_calls=0; synthetic archive payloads.
```

## Operational limits and next owner

- Root owns integration, remote CI and release actions. Actual green CI must be
  for the final chosen source SHA, with the expected run and attempt. This scoped
  review does not configure or verify repository branch-protection rules.
- Draft publication is one API state update after checks; GitHub's separate reads
  and update are not a cross-resource transaction. External changes after the last
  check are not eliminated by a local script. The fix removes the long upload-time
  gap without claiming a server-side lock.
- A failed upload or rejected final check can leave a tag and draft. The script
  intentionally refuses to overwrite an existing draft; an authorized owner must
  inspect and resolve that state before retrying.
- The final listing must expose correct SHA256 digests. A missing digest fails
  closed and preserves the draft. GitHub documents the digest/state/size fields
  in the release-asset API [S3]. No live upload behavior was tested here.
- The CI validator requires the complete expected job set for its pinned attempt.
  If a partial rerun does not supply that full set, rerun all required jobs rather
  than weakening the gate. A changed attempt requires rebuilding/reverifying the
  candidate's provenance.
- Packages explicitly declare `signed: false`; native signing/notarization and
  installer/upgrade validation are outside this change.

## Primary sources checked

- [S1: GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
- [S2: Workflow syntax — shells and permissions](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)
- [S3: Release asset REST API](https://docs.github.com/en/rest/releases/assets)
- [S4: Workflow run REST API](https://docs.github.com/en/rest/actions/workflow-runs)
- [S5: Release REST API](https://docs.github.com/en/rest/releases/releases)
- [actions/checkout pinned commit](https://github.com/actions/checkout/commit/de0fac2e4500dabe0009e67214ff5f5447ce83dd)
- [actions/setup-python pinned commit](https://github.com/actions/setup-python/commit/e797f83bcb11b83ae66e0230d6156d7c80228e7c)
- [actions/upload-artifact pinned commit](https://github.com/actions/upload-artifact/commit/b7c566a772e6b6bfb58ed0dc250532a479d7789f)
- [actions/download-artifact pinned commit](https://github.com/actions/download-artifact/commit/37930b1c2abaa49bbe596cd826c3c89aef350131)

## Reviewed file identities

| File | SHA256 |
|---|---|
| `.github/workflows/ci.yml` | `322bdf47665cfccf15b8a22ab41e6d6604d84192848c2b0254ee7a25b2078c06` |
| `.github/workflows/release.yml` | `afa0503a689e3298f055af101a9e8a901352eef3ac77d480cf31fe28f3574f7e` |
| `scripts/package_release.py` | `e2cc1ccdc94263baa7bcb921279faeed6a3e0cd963a2f7ff26bc61ad4f3f8f5b` |
| `rust-toolchain.toml` | `080d1c422a980a506912db5c2cc9fbedcf1df1fe4c8de676934ab7c421c5d768` |

## Appendix: independent offline reproducer

Save the following code as `crew-delivery-review.py`. It accepts a path to the
reviewed `scripts/package_release.py`, replaces the entire GitHub API boundary,
and asserts that no network access occurs for the repository-fence test. It uses
synthetic archive payloads and does not launch native binaries.

The same script exits 1 for the original `f7a9dde` implementation and 0 for the
fixed `ec2329e` implementation. This appendix makes the original `/tmp` evidence
reproducible without relying on the temporary file's continued existence.

Reproducer SHA256: `e0c25d0d8b1da921856acb1d0296303344d720597c46cddd460d56dc1c97bb57`.

```python
#!/usr/bin/env python3
"""Independent offline scenarios; GitHub is fully mocked and never called."""
import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
from unittest.mock import patch
import urllib.parse

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("crew_release", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
SHA = "a" * 40
META = dict(repository=m.REPOSITORY, tag="v0.3.0", version="0.3.0", sha=SHA,
            ci_run_id=7, ci_attempt=1)
ENV = dict(GITHUB_REPOSITORY=m.REPOSITORY, GITHUB_EVENT_NAME="workflow_dispatch",
           GITHUB_REF="refs/heads/main", GITHUB_SHA=SHA)


class GitHubFixture:
    def __init__(self, mode="success"):
        self.mode, self.calls, self.assets = mode, [], []
        self.attempt, self.conclusion, self.tag, self.release = 1, "success", None, None
        self.run_lookups = 0

    def api(self, path, *, method="GET", value=None, raw=None, missing_ok=False, upload=False):
        prefix = "/repos/" + m.REPOSITORY
        assert path.startswith(prefix)
        route = path[len(prefix):]
        self.calls.append((method, route))
        if route == "/git/ref/heads/main":
            return {"object": {"type": "commit", "sha": SHA}}
        if route.startswith("/compare/"):
            return {"status": "identical"}
        if route.startswith("/git/ref/tags/"):
            return None if self.tag is None else {"object": {"type": "commit", "sha": self.tag}}
        if route.startswith("/releases?"):
            return [] if self.release is None else [copy.deepcopy(self.release)]
        if route.startswith("/actions/workflows/ci.yml/runs?"):
            self.run_lookups += 1
            return {"workflow_runs": [{"id": 7, "head_sha": SHA, "head_branch": "main",
                "event": "push", "head_repository": {"full_name": m.REPOSITORY},
                "status": "completed", "conclusion": self.conclusion, "run_attempt": self.attempt}]}
        if route.startswith("/actions/runs/7/attempts/"):
            jobs = [{"name": name, "status": "completed", "conclusion": "success", "steps": [
                {"name": step, "status": "completed", "conclusion": "success"}
                for step in (m.CI_STEPS if name in m.CI_PLATFORMS else {"Require every platform"})]}
                for name in sorted(m.CI_PLATFORMS | {"Crew CI Gate"})]
            return {"total_count": len(jobs), "jobs": jobs}
        if route == "/git/refs" and method == "POST":
            self.tag = value["sha"]
            return {"object": {"type": "commit", "sha": self.tag}}
        if route == "/releases" and method == "POST":
            assert value["draft"] is True
            self.release = {**value, "id": 11, "html_url": "https://example.invalid/release"}
            return copy.deepcopy(self.release)
        if route.startswith("/releases/11/assets?name=") and method == "POST":
            assert upload
            if self.mode == "upload_failure" and len(self.assets) == 2:
                raise ValueError("simulated HTTP 502 upload failure")
            name = urllib.parse.parse_qs(urllib.parse.urlsplit(route).query)["name"][0]
            asset = {"name": name, "size": len(raw), "state": "uploaded", "digest": "sha256:" + m.digest(raw)}
            self.assets.append(asset)
            if len(self.assets) == 6:
                if self.mode == "tag_changed":
                    self.tag = "b" * 40
                if self.mode == "ci_rerun_failed":
                    self.attempt, self.conclusion = 2, "failure"
            return copy.deepcopy(asset)
        if route.startswith("/releases/11/assets?"):
            return copy.deepcopy(self.assets[:-1] if self.mode == "asset_missing" else self.assets)
        if route == "/releases/11" and method == "PATCH":
            assert value == {"draft": False}
            self.release["draft"] = False
            return copy.deepcopy(self.release)
        raise AssertionError((method, route))


def build_fixtures(root):
    packages, output = root / "packages", root / "release"
    packages.mkdir()
    for target in m.TARGETS:
        ext = ".exe" if target.endswith("windows-msvc") else ""
        payloads = {"crew" + ext: b"synthetic binary", "tmuxlike" + ext: b"synthetic alias",
                    "README.md": b"synthetic readme", "LICENSE": b"synthetic license"}
        meta = {**META, "target": target, "rust": "1.96.1", "signed": False,
                "files": {name: {"sha256": m.digest(data), "bytes": len(data)} for name, data in payloads.items()}}
        payloads["manifest.json"] = m.json_bytes(meta)
        archive = packages / m.archive_name(META["tag"], target)
        m.write_archive(archive, "Crew-" + META["tag"] + "-" + target, payloads)
        data = archive.read_bytes()
        (packages / f"Crew-{META['tag']}-{target}.json").write_bytes(m.json_bytes({"package": meta,
            "archive": {"name": archive.name, "sha256": m.digest(data), "bytes": len(data)}}))
    args = SimpleNamespace(tag=META["tag"], sha=SHA, ci_run_id=7, ci_attempt=1,
                           input=str(packages), output=str(output))
    with contextlib.redirect_stdout(io.StringIO()):
        m.aggregate(args)
    args.input = str(output)
    return args


results = []
with tempfile.TemporaryDirectory(prefix="crew-delivery-review-") as directory:
    args = build_fixtures(Path(directory))
    for mode in ["success", "upload_failure", "asset_missing", "tag_changed", "ci_rerun_failed"]:
        fake = GitHubFixture(mode)
        error = None
        with patch.dict(os.environ, ENV), patch.object(m, "api", fake.api), \
             patch.object(m, "run", return_value=SHA), \
             patch.object(m, "checked_source", return_value="0.3.0"), \
             contextlib.redirect_stdout(io.StringIO()):
            try:
                m.publish(args)
            except ValueError as exc:
                error = str(exc)
        published = fake.release is not None and fake.release["draft"] is False
        expected = mode == "success"
        result = {"case": mode, "result": "PASS" if published == expected else "FAIL",
                  "published": published, "uploads": len(fake.assets), "latest_ci_attempt": fake.attempt,
                  "latest_ci_conclusion": fake.conclusion, "ci_reads": fake.run_lookups,
                  "error": error, "remote_calls": 0, "simulation": True}
        results.append(result)
        print(json.dumps(result))

    fake = GitHubFixture()
    fake.attempt = 2
    with patch.dict(os.environ, ENV), patch.object(m, "api", fake.api), \
         patch.object(m, "run", return_value=SHA), patch.object(m, "checked_source", return_value="0.3.0"):
        try:
            m.verify_source("v0.3.0", SHA, 7, 1)
            raise AssertionError("changed CI attempt was accepted at initial verification")
        except ValueError as exc:
            assert "changed" in str(exc), str(exc)
    print(json.dumps({"case": "ci_attempt_changed_before_publish", "result": "PASS", "simulation": True}))

    # Validate API repository fencing before any network operation can occur.
    with patch.dict(os.environ, {"GH_TOKEN": "synthetic-local-test-token"}), \
         patch.object(m.urllib.request, "urlopen", side_effect=AssertionError("network must not occur")):
        try:
            m.api("/repos/stevenke1981/tmuxlike/releases", method="POST", value={})
            raise AssertionError("foreign repository was accepted")
        except ValueError as exc:
            assert "outside" in str(exc), str(exc)
    print(json.dumps({"case": "repository_fence", "result": "PASS", "remote_calls": 0}))

sys.exit(1 if any(item["result"] == "FAIL" for item in results) else 0)
```
