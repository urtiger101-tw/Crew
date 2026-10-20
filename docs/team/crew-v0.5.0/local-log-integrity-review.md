# Independent local evidence log diagnosis

Reviewer: `/root/v05_remote`; scope: read-only source and existing evidence inspection. No validation suite was rerun for this diagnosis, and no source, original evidence, Git reference or GitHub state was modified.

## Assessment

The `crew-v05-local-evidence-02` report is **not valid packaging evidence**, despite its recorded 25 command PASS statuses. `package_local.py` correctly rejects its command log mismatch. The exact writer that replaced/truncated the log is **not established** by this inspection.

Reviewed source commit: `276ad68338af7562add672a7b461d9772fbd4b1b`; tree `469d052463131646a4de907584634817186edd9f`.

Evidence: `/workspace/scratch/21396cda8665/crew-v05-local-evidence-02/validation.json`; SHA-256 `b80f0a226b4f7d6be9c62b42a3f128d97f81e6e38a929773b711aa885d8c2551`.

Affected log: `/workspace/scratch/21396cda8665/crew-v05-local-evidence-02/logs/19-agent-identity.log`.

## Direct observations

| Observation | Value |
| --- | --- |
| Per-command bytes | 516 |
| Per-command SHA-256 | `5989d6537fa98ea1dade9c976531ea7d683a9deec2cff2ab9244a2261ef79745` |
| Current/final-manifest bytes | 411 |
| Current/final-manifest SHA-256 | `d718d5f31ed2a2edab29a6230822ed293ac466b01a3c2ad703f52f874e0f8e22` |
| Command ended | `2026-10-20T12:01:44.727722+00:00` |
| Current log mtime | `2026-10-20T12:01:44.281579+00:00` |
| Current log ctime | `2026-10-20T12:01:46.965624+00:00` |
| Missing suffix | 105 bytes, final two expected output lines |

A static AST extraction of the nine successful literal `print` statements in the immutable `agent_identity_smoke.py` produces exactly 516 bytes and SHA-256 `5989d6537fa98ea1dade9c976531ea7d683a9deec2cff2ab9244a2261ef79745`. This matches the per-command record exactly. The current log is an exact prefix containing the first seven lines. The missing suffix is:

```text
ok an agent becoming blocked during paste never receives Enter
PASS agent identity and integration smoke
```

This calculation reconstructs expected text only in memory; it does **not** restore or alter the evidence log. Only this log differs from its per-command digest among the 25 command records. Its final `log_files` manifest records the shortened 411-byte file, so the evidence contradicts itself.

## Source trace

At the reviewed commit, `scripts/validate_local.py:235–261` opens a unique indexed log in `wb`, runs and waits for the child, closes the file, then records its digest. `validate():393–394` immediately calls `read_log` to verify that digest. The outer command loop is sequential and does not reuse this log path. No later writer of this completed step log appears in the validator.

The agent identity smoke uses flushed `print` calls for its own stdout. Its API calls capture stdout/stderr, pending processes use `PIPE`, and its daemon log is confined to its temporary fixture. No source path that deliberately truncates the validator log was found in that smoke.

At `validate_local.py:430`, finalization separately snapshots every log. At line 432, final PASS depends on command statuses and source/binary errors, without rechecking each recorded command log against this final snapshot. Thus a later external change can produce recorded command PASS entries while making the resulting evidence internally inconsistent. `verify_evidence` calls `read_log` for every command during packaging and refuses this mismatch; it does not downgrade the integrity gate.

## Inference and limit

The current content is an earlier exact prefix; its mtime predates command completion by roughly 0.446 seconds, while its ctime is roughly 2.238 seconds after completion. The later logs inspected have equal mtime and ctime. These facts fit a later restoration/copy of an earlier snapshot that preserved the old mtime more closely than delayed ordinary stdout writes. They do not identify a process or prove that a synchronization service caused it. The root agent separately reported an active synchronization/inotify process; that observation alone is not causal proof.

## Disposition

Preserve evidence-02 as rejected, retain the original mismatch, and do not reconstruct or rehash it into a PASS. Root is rerunning the frozen candidate into isolated `/tmp` storage. The next evidence/package must independently pass command-log/final-manifest/actual-byte consistency checks. Source behavior of the failing original record and package refusal are confirmed; the external writer remains undetermined.

A later evidence-03 SSH fixture failure is a separate issue and is outside this log-drift diagnosis.
