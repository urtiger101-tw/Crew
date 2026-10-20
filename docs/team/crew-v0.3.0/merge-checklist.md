# Crew v0.3.0 — merge and release checklist

Prepared: **2026-10-20**. Task: `PR-final-handoff`, revision 1.
Author: `/root/worktree_worker`, acting as `pr_manager`.
Next owner: `/root`, coordinator and final PR/merge/release owner.
Write scope: this file and `pr-body.md` only. This author performed no remote
write, workflow dispatch, merge, tag creation, or release publication.

**Decision: local implementation evidence is PASS; remote CI, merge, and release
are NOT_RUN. This document does not authorize bypassing those gates.**

Use **PASS** for executed successful checks, **FAIL** for executed failures,
**BLOCKED** for a missing prerequisite that prevents an action, **NOT_RUN** for
unexecuted work, and **N/A** only with a reason. An unchecked box is pending, not
an implicit success. Update observations with the actual SHA, URL, and outcome.

## Candidate identity and evidence

| Item | Recorded value |
|---|---|
| Destination / PR direction | `urtiger101-tw/Crew`: `feat/crew-herdr-alignment` → `main` |
| Original repository | `stevenke1981/tmuxlike`, read-only |
| Source base | `12be9e307e47a56d3bfd669ab7bddb1ee7466e09` (tmuxlike 0.2.1) |
| Herdr behavior reference | `143bf5fd4781d164260c63fe28e7dc08054ad32f` (0.9.5) |
| Local documented candidate | `eaec779d2946de7424f5fad1d2a4a45eafbe9269` |
| Actual final Linux tested SHA | `efbf228127774bd25c7cf5904485eb27204d1632` |
| Actual final Linux tested tree | `54ab25e4b096519ac9d540f8efd49bf3af943851` |
| Product independent-review SHA | `a421a6bd3808f5930a00e4dd921298d98ddd449b` |
| Delivery independent-review SHA | `ec2329ed75d8e9674ed665f483183ee4584a9403`, integrated as `0d24000` |
| Remote import/upload observation | Coordinator reports full history imported; feature upload through GitHub UI in progress; PR not yet opened at handoff |
| Actual remote PR head / PR URL | **NOT_RUN** — record after upload and PR creation |
| Remote PR/merge-group CI run + attempt | **NOT_RUN** — record actual source and tested merge SHA |
| Merged `main` SHA / its push CI run + attempt | **NOT_RUN** |
| Release run / tag target / release URL | **NOT_RUN** |

The tested-to-documented diff adds only `local-validation.md` and
`local-validation.json`. Product sources and Cargo files are unchanged from the
product review baseline. The delivery files' hashes match the reviewed fix.
The final feature diff from the original base to `eaec779` has 61 files,
7,756 insertions and 522 deletions; the two PR handoff documents come afterward.

Evidence files in this directory: [local validation](local-validation.md),
[machine-readable local record](local-validation.json),
[CORE/GIT/API review](core-review.md), [AGENT/R5 review](agent-review.md),
[delivery review](delivery-review.md), [dispatch plan](dispatch-plan.md), and
[workflow rules](workflow-rules.md). Scope and behavior limits are in the
[parity matrix](../../herdr-parity.md) and
[automation guide](../../automation.md).

## 1. Completed local acceptance

The validation actor `/root/reference_audit` ran in
`/workspace/scratch/21396cda8665/tmuxlike` on Linux x86_64, Rust 1.96.1, Python
3.12.14, with a clean worktree before and after. Every command below exited **0**.
Exact commands, tool paths, timestamps, and hashes are in `local-validation.json`.

- [x] **PASS** — `cargo fmt --all -- --check`.
- [x] **PASS** — `cargo clippy --offline --locked --all-targets -- -D warnings`.
- [x] **PASS** — `cargo test --offline --locked`: **318 passed, 0 failed**.
- [x] **PASS** — `python3 scripts/package_release.py self-test`: **5 tests**,
  including 13 final-publication scenarios with mocked API responses.
- [x] **PASS** — `cargo build --offline --locked --release --bins`: both binaries.
- [x] **PASS** — `scripts/smoke.py` against optimized `tmuxlike`, real PTY.
- [x] **PASS** — `scripts/api_smoke.py` against optimized `crew`.
- [x] **PASS** — `scripts/workspace_smoke.py` against optimized `crew`.
- [x] **PASS** — `scripts/worktree_smoke.py` against optimized `crew`: 13 checks
  and real Linux R3/R6 barrier regressions, invoked automatically.
- [x] **PASS** — `scripts/agent_identity_smoke.py` against optimized `crew`.
- [x] **PASS** — `scripts/bridge_events_smoke.py` against optimized `crew`.

The last six checks use isolated real servers/PTYs; worktree tests use temporary
real Git repositories. Both tested release executables have SHA256
`8382d48f1970ccedac7c33cf331c944f0c9d89f992935da57053521b86754ac2`.
This hash identifies the local binaries, not future hosted release artifacts.

- [x] **PASS** — Independent scoped reviews resolved R1, R2, R3, R4, R5, R6,
  and DELIVERY-1; no open P1/P2 remains in their recorded scopes.
- [x] **PASS** — Independent delivery review ran actionlint 1.7.12 with no
  diagnostics and tested complete publish paths offline, including retaining
  drafts on failed uploads, changed tags, incomplete assets, and changed CI.
- [ ] **NOT_RUN** — Windows and macOS execution.
- [ ] **NOT_RUN** — Hosted CI, four-target release build, live upload/publication.
- [ ] **NOT_RUN** — Interactive `scripts/e2e_linux.py` scenario and operational
  rollback rehearsal; neither is represented by the six completed smoke suites.

The AGENT author also ran final integration validation; independent AGENT approval
comes from `/root/target_audit`'s separate scoped review. Distinct agent instances
do not establish distinct GitHub account approvals.

## 2. Before opening or updating the PR — owner: coordinator

- [ ] Confirm the destination and base branch in GitHub are
  `urtiger101-tw/Crew` / `main`; keep the original repository read-only.
- [ ] Finish uploading the complete feature, evidence, and these two handoff
  documents. Record the actual remote branch head. GitHub UI commits can have
  different SHAs from the local commits even when file contents match.
- [ ] Compare the remote feature tree to the intended local integrated tree;
  account explicitly for any documentation-only additions. Verify all source,
  Cargo, toolchain, workflow, and test-script hashes against the reviewed/tested
  content in `local-validation.json`. Check added/deleted paths as well as hashes;
  sampling a few files is insufficient. Record the mapping and any differences.
- [ ] Inspect the actual PR diff against remote `main` to catch omitted files,
  unintended changes, or an incorrect history/base. Preserve history; no force
  replacement of the default branch.
- [ ] Open the PR using [pr-body.md](pr-body.md), record its URL and head SHA, and
  update its preparation-time status with actual remote observations.
- [ ] Confirm the evidence links resolve after upload. Replace mutable branch
  links in the PR with the actual remote revision where useful; do not invent
  GitHub URLs for local-only commits.

If the uploaded implementation differs, determine the affected review and test
scope before carrying evidence forward. A documentation-only delta can be
recorded directly; a changed implementation cannot inherit a passing result
without the appropriate renewed checks.

## 3. Before merge — owner: coordinator with independent reviewer

- [ ] Review the final remote candidate, its content mapping, and all changes
  after the recorded scoped reviews. Resolve new blocking findings and review
  conversations. Record the actual reviewer decision without treating an author's
  self-approval or multiple names on one GitHub account as separate approvals.
- [ ] Observe actual CI for the current PR head/tested merge revision. Required
  jobs: **`Linux x86_64`**, **`Windows x86_64`**, **`macOS arm64`**, and
  **`Crew CI Gate`**. Record run URL/ID, attempt, source/tested SHA, and conclusion.
- [ ] Confirm formatting, Clippy with warnings denied, unit tests, release
  tooling tests, both optimized executables, and all six smoke steps succeeded
  on every CI platform. Missing, skipped, cancelled, failed, or stale checks do
  not satisfy the gate. Logs remain evidence, not substitutes for job success.
- [ ] If a merge queue is used, require the same successful checks for its
  actual `merge_group` revision. Without a queue, mark this item **N/A** with
  that reason; keep the current PR checks mandatory.
- [ ] Inspect the repository's existing Rulesets/branch-protection and review
  requirements, including the exact `Crew CI Gate` name. Record what is actually
  enforced. Workflow files alone do not configure these settings; changes to
  repository settings are outside this documentation task.
- [ ] Check the PR has no unresolved conflicts and no newer head invalidating
  CI or review. Merge only after all applicable evidence above is current.
- [ ] Record the resulting `main` commit and merge URL. **Merge remains NOT_RUN
  until that event is observed.**

CI intentionally runs for PRs targeting `main`, pushes to `main`, and merge groups.
Feature-branch pushes alone do not duplicate PR runs. Earlier PR runs may be
cancelled by concurrency; only the latest relevant completed run counts.

## 4. Before release — owner: coordinator / release manager

- [ ] Wait for the **main-push CI** on the exact selected release source to
  complete successfully, including the three platform jobs and `Crew CI Gate`.
  A green pre-merge PR run does not substitute for this source's main-push run.
  Record its run ID and attempt. If a partial rerun omits expected jobs, rerun
  the full required job set; do not weaken source verification.
- [ ] Confirm Cargo version `0.3.0`, requested tag `v0.3.0`, and Rust `1.96.1`.
  The selected SHA must be in `main` history. Check that no existing tag points
  elsewhere and no release/draft conflicts with the intended publication.
- [ ] Start `Release` using `workflow_dispatch` on `main` with tag `v0.3.0`, or
  the supported version-tag push, under the existing delivery authorization.
  Record the immutable source selected by `Verify source and CI`.
- [ ] Require successful native build and six smoke steps for all four targets:
  `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`,
  `aarch64-apple-darwin`, and `x86_64-apple-darwin`. macOS Intel execution is an
  additional release target beyond the three-platform PR CI matrix.
- [ ] Require aggregation to verify archive members, both executables, README,
  LICENSE, embedded manifests, file digests, source SHA, and CI run/attempt
  provenance. Confirm the final six assets: four platform archives,
  `release-manifest.json`, and `SHA256SUMS`.
- [ ] Require the publish job to revalidate source/CI, create a draft, upload
  all assets, and recheck tag, latest CI run/attempt/jobs/steps, and every remote
  asset's uploaded state, size, and SHA256 immediately before publication.
  Missing asset digests or any changed gate must retain the draft and fail.
- [ ] If a run fails after creating a tag/draft, inspect the actual failure and
  retained state before retrying. The publisher refuses to overwrite drafts;
  do not delete assets or tags merely to force a green result. Changed CI
  run/attempt requires refreshed package provenance.
- [ ] Observe the completed public release and record its URL, source SHA, CI
  and release run IDs/attempts, target results, manifest/checksum hashes, and
  asset list. Only then change release status from **NOT_RUN** to **PASS**.

The publisher's final verification closes the upload-time stale-CI gap. Separate
GitHub REST reads and the final update still do not form an atomic transaction
across CI, tags, and releases. Artifacts are unsigned; do not describe them as
signed/notarized or claim installer upgrade validation.

## 5. Compatibility and user-facing handoff — owner: coordinator

- [ ] Include upgrade instructions: save work, stop the old daemon before new
  APIs, preserve config/state backups, and disclose that processes do not survive
  server stop. The same default socket is used by both command names.
- [ ] Retain rollback instructions from the PR body: preserve the current state,
  restore the prior binary plus pre-upgrade backups, and use a reviewed source
  revert for a merged regression. Do not assume old binaries understand new
  state or use selection-marker deletion as a migration repair.
- [ ] Preserve the documented limits: stable workspace/tab IDs but new pane IDs
  after restart; persisted env overrides; generic provider hooks; occupant rather
  than per-prompt identity; Linux-only live cwd checks; external Git/process races;
  bounded in-memory events; Crew rather than Herdr wire format; remaining advanced
  Herdr differences in the parity matrix.
- [ ] Report implementation, local validation, hosted CI, PR, merge, and release
  as separate outcomes. Supply actual links for completed remote actions and
  specific blockers or NOT_RUN status for anything pending.

No external action is required from this document author. The coordinator owns
source upload, independent final review, remote checks, merge, and release under
the existing user authorization. No further user confirmation is implied by
this handoff; technical gates require evidence before execution proceeds.
