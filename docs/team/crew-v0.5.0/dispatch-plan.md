# Crew 0.5.0 — Herdr alignment without hosted CI

Date: 2026-10-20. Base: `5e7e208bca5bf388aded7631aff0d8846b47363f`.
Reference: Herdr `143bf5fd4781d164260c63fe28e7dc08054ad32f`, confirmed by a
read-only remote HEAD lookup. Implement behavior independently; Crew keeps its
own command and wire contracts.

## User constraints

The owner requested metadata presentation, cross-workspace tab transfer, SSH
federation, a plugin marketplace, and automatic agent-kind detection. GitHub
Actions minutes are exhausted. Run validation locally, do not trigger hosted
jobs or purchase another CI service. All remote work remains confined to
`urtiger101-tw/Crew`; the original repository remains read-only.

## Ownership and integration

| Owner instance | Worktree | Deliverable |
| --- | --- | --- |
| v05_transfer | wt-v05-transfer | Live tab transfer, identity/persistence/PTY regression |
| v05_presentation | wt-v05-presentation | Metadata presentation/templates and evidence-based agent identity |
| v05_remote | wt-v05-remote | Saved SSH machines, scoped API routing, federation selector and real SSH tests |
| root | tmuxlike | Plugin marketplace/runtime, local validation, version/docs and final integration |

Every author uses a separate branch based on the same source. Shared module
registration and API additions are limited to the author's feature. Root owns
their serial integration: transfer, presentation, remote, then plugin routing.
No author changes another worktree or GitHub. Separate instances review the
integrated candidate; the author does not approve their own change.

## Acceptance

- Metadata: visible pane/workspace templates, bounded/sanitized token expansion,
  clear/TTL/sequence behavior and explicit lifecycle isolation.
- Transfer: stable tab/pane IDs, live PTYs/PIDs, focus/zoom, client targets and
  persistence survive; invalid requests have no partial side effects.
- SSH: real OpenSSH transport with ordinary host-key verification, saved
  machines, no implicit local fallback, per-machine errors/timeouts, combined
  listing and an interactive workspace selector. Loopback tests use temporary
  credentials and isolated daemons, never a user's production SSH host.
- Plugins: searchable bundled catalog and optional public discovery, inspect,
  install/link/update, enable/disable, action execution and logs. Reviewed argv
  manifests, explicit install trust, stable config/state and retained old files.
  Tests execute only repository-owned fixtures and bundled commands.
- Agent identity: observed running processes identify agent kind; explicit
  reports win and retain their instance/wait semantics. No paid agent is started.
- Validation: format, clippy, unit/tooling tests, release builds and real local
  integration tests with exact source/tree/binary hashes and per-check results.
  Missing native Windows/macOS validation is NOT_RUN, never a synthetic PASS.

## Delivery policy

Keep existing CI jobs available for explicitly requested future use, but remove
automatic push/PR/tag triggers for this quota-constrained delivery. Never weaken
branch-protection rules or fabricate GitHub checks. Local evidence records its
provider as local, including failures and platform limits. A local artifact is
not called a fully validated four-platform release.
