# Crew v0.4.0 — Herdr continuation and Windows installer

## Goal and baseline

Continue the user's Herdr feature alignment and publish an Inno Setup installer
alongside Crew's portable packages. All remote work belongs to
`urtiger101-tw/Crew`; `stevenke1981/tmuxlike` remains a read-only source.

Crew baseline: `bf2190f8b15849fb1ad703110fdc53981fa0455c` (v0.3.0).
Herdr reference: `143bf5fd4781d164260c63fe28e7dc08054ad32f` (0.9.5),
confirmed as the upstream HEAD on 2026-10-20. Implement behavior independently;
do not copy Herdr source or claim Herdr wire-protocol compatibility.

The user's existing authorization covers code, CI, PR and GitHub Release in the
new repository. No purchases, new credentials, paid services or permissions are
part of this task. Existing local worktrees and published v0.3.0 assets remain
intact.

## Owners and isolated write scopes

| Task | Author instance / worktree | Scope | Independent review |
|---|---|---|---|
| CORE-04 | target_audit / wt-v04-core | workspace/tab ordering, relevant persistence/identity regressions, workspace smoke and feature documentation | root |
| META-04 | root / tmuxlike | token metadata model, CLI/API/bridge, TTL/events, metadata smoke; shared server/workspace wiring after CORE handoff | reference_audit |
| INSTALL-04 | worktree_worker / wt-v04-installer | Inno script, build/install smoke, package/release tooling, CI/release workflows, Windows installation guide | reference_audit and root |
| DELIVERY-04 | root / tmuxlike | version, README, parity matrix, integration, PR/CI/release and final evidence | reference_audit public-download verification |

The instances are distinct agents, not separate human GitHub approvers. Each
author uses one worktree. Shared server/workspace edits are serialized. Only
root performs remote mutations through the authorized owner session.

## Acceptance

### CORE-04

- `workspace move` and `tab move` reorder existing resources using documented
  zero-based insertion slots, with raw API validation and CLI/bridge access.
- Preserve workspace, tab and pane IDs, running PTYs, agent occupant identity,
  current workspace and active tab/pane. Reject out-of-range requests atomically.
- Publish authoritative ordered IDs in move events. Persist order across a cold
  restart. This release does not move tabs between workspaces.
- Verify boundary slots, adjacent/no-op moves, a background workspace, live PTY
  continuity, caller context, event data and restored ordering.

### META-04

- `pane report-metadata` and `workspace report-metadata` patch bounded named
  string tokens. Clear selected keys, retain omitted keys and expose tokens in
  the existing resource snapshots and agent projections.
- Use per-key TTL (1..86400000 ms), monotonic time and per-source sequence
  high-water marks. Ignore stale sequenced reports without refreshing TTL.
- Preserve sequence high-water marks after clearing or expiry; cap patch size,
  retained keys and sequenced sources. Validate before mutating state.
- Emit metadata update/expiry events. Remove closed-resource metadata and do
  not restore tokens after restart. Metadata must never alter agent lifecycle,
  status or the completion of wait/prompt operations.
- This slice covers token metadata. Herdr's presentation title/state-label
  overrides and configurable token templates remain outside this release.

### INSTALL-04

- Build a real Windows x64 setup executable with a fixed official Inno Setup
  compiler version and verified download digest. Preserve the portable ZIP.
- Install per user without elevation; use a stable AppId. Support upgrade and
  uninstall while preserving configuration, saved state and unrelated files.
- Do not silently stop a running Crew server. Detect or refuse an in-use binary
  safely; describe the user's upgrade action accurately.
- In native Windows CI, compile and run silent fresh installation, actual
  version replacement, and uninstall in disposable directories. Use released
  v0.3.0 binaries in a test-only predecessor installer when testing upgrade;
  do not imply that this fixture was a previously published installer.
- Include the installer in the release manifest, SHA256SUMS, source/CI
  provenance, and GitHub uploaded-asset digest checks. Missing installer,
  invalid contents or failed installation tests must block publication.

### DELIVERY-04

- Run format, clippy, unit and relevant native smoke tests; independently review
  the final candidate. Preserve the first failure and correct its cause without
  weakening gates or claiming skipped checks passed.
- Create a PR, verify the exact tested tree, merge only after required CI and
  review are satisfied, then run main CI and publish v0.4.0 from that source.
- Independently download every published asset and verify names, sizes,
  digests, manifests and binary identities. Report unsigned status honestly.

## Evidence and remaining scope

Use `PASS`, `FAIL`, `BLOCKED`, `NOT_RUN` and `N/A` with exact SHA, commands,
exit codes and platform. Local Linux success does not establish Windows/macOS
success; merge does not establish release. Native installation tests are
NOT_RUN until a Windows runner provides actual evidence.

SSH federation, marketplace/plugins, advanced terminal graphics, automatic
agent detection, cross-workspace tab/pane moves, native notifications and live
server handoff remain separate gaps. No percentage-complete estimate is used.
