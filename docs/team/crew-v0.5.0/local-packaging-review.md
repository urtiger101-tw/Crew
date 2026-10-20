# Independent review: local validation and packaging (fix verification)

- Reviewer: `/root/v05_presentation`; local tooling author is `/root/v05_transfer`.
- Feature base: `3e498edbbc7ccb4b7f71f7c5daf76c4f3212e495`.
- Original candidate: `93c4bca9242f60d983088193820c4fb6779cb620`.
- Reviewed successor: `a9a60b79d319e3f7a85d93f69ac8dcc2166e13f0`, parent is the original candidate.
- Exact read-only source: `/tmp/crew-review-local-a9a60b7` (git archive).
- Verdict: **PASS for the reviewed local-tooling slice**. The sole P2 finding is resolved. Full integrated native Linux validation and native Windows execution are **NOT_RUN by this reviewer**.

## Resolved LOCAL-P2-1: staged Windows installer drift

Original `scripts/package_local.py:92-94` copied a verified setup and JSON sidecar without comparing the staged bytes to the verified inventory. The original-file check after copying could miss a change restricted to the copy interval. The original reproduction emitted a different setup while leaving the manifest tied to the old digest.

The successor adds `file_record(staging / name) == report["installer_files"][name]` immediately after each setup/sidecar copy (lines 94-95). Both byte length and SHA-256 must match the verified report before a manifest or final output is published. Existing temporary-directory cleanup removes a failed staging directory. The new regression covers each file, restores the source after the copy, confirms the source evidence remains valid, and confirms no output package is produced.

I independently repeated the original change-copy-restore model for the setup and the sidecar. Both returned exit 1 with `ValueError: installer artifact changed while copying`. These are expected rejection results. The script uses synthetic PE/evidence fixtures only; it does not compile or execute an installer. The original repro needed only `git commit --allow-empty` because the successor's fixture now pre-creates Windows helper scripts; the adapted copy adds a setup/sidecar selector. The original repro and original failure evidence remain preserved.

## Independent verification

| Check | Result | Evidence |
| --- | --- | --- |
| `python3 scripts/test_local_validation.py` | PASS, 17 tests, exit 0 | `/tmp/crew-local-review-a9a60b7-17-tests.log` |
| `python3 scripts/package_release.py self-test` | PASS, 11 tests, exit 0 | `/tmp/crew-local-review-a9a60b7-hosted-11-tests.log` |
| Setup copy-interval drift | PASS: expected rejection, process exit 1 | `/tmp/crew-local-review-a9a60b7-setup-race.log` |
| Sidecar copy-interval drift | PASS: expected rejection, process exit 1 | `/tmp/crew-local-review-a9a60b7-sidecar-race.log` |
| Native Windows compilation and 14 lifecycle checks | NOT_RUN | No native Windows environment |
| Complete integrated native Linux validation | NOT_RUN by reviewer | Root owns subsequent integrated validation |

## Scope and retained guarantees

The initial review examined the exact required command list and exact argv/cwd/count matching; source commit/tree/index/dirty state; binary type and digest matching; log and artifact inventories; rejection of zero-exit SKIP/NOT_RUN claims; explicit non-native unavailable checks; local-versus-hosted provider isolation; and the retained fourteen native Windows lifecycle requirements. The narrow successor changes only the staged installer copy verification and its tests. No hosted workflow, release publication, existing install, or remote service was changed by this review. The existing installer smoke's isolated-user and existing-install refusal contract remains in place; it was reviewed, not executed on Windows.

The hash report binds local files and evidence; it is not a third-party trust attestation. No full platform matrix pass is claimed. No remaining reproducible blocking issue was found in this slice.

Next owner: `/root` for integration and native Linux validation, with native Windows lifecycle validation still required before a verified Windows installer claim.
