# Crew 0.5.0 review handoff

Date: 2026-10-20. Base: `5e7e208bca5bf388aded7631aff0d8846b47363f`.
Herdr reference: `143bf5fd4781d164260c63fe28e7dc08054ad32f`.
Reviewed integrated product head before these report files:
`0475da4faf910adf99bf1227d914a51650b4f3b1`.

The owner requested five additional alignment capabilities and local verification
because hosted Actions minutes are exhausted. The implementation scope and its
remaining differences are in [the parity table](../../herdr-parity.md).

## Independent review decision

No reproducible blocking finding remains in the reviewed feature slices.
Authors and reviewers were separate agent instances with separate worktrees.
These records describe real code review and focused native tests. They are not
GitHub human approvals, fabricated status checks, or a full-platform release
attestation.

| Capability | Author | Independent reviewer | Record |
| --- | --- | --- | --- |
| Metadata presentation/templates | v05_presentation; root for CLI literal fix | v05_transfer | [Presentation review](presentation-review.md), [machine record](presentation-review.json) |
| Live cross-workspace tab transfer | v05_transfer | root | [Transfer review](transfer-review.md) |
| SSH federation | v05_remote | v05_presentation | [Original review](ssh-initial-review.md), [successor review](ssh-fix-review.md) |
| Plugin marketplace/actions | root | v05_remote; v05_transfer for public download | [Original review](plugin-initial-review.md), [fix review](plugin-fix-review.md), [network check](plugin-network-review.md) |
| Agent-kind process detection | v05_presentation | v05_transfer | [Presentation/detection review](presentation-review.md) |
| Local validation/native packaging | v05_transfer | v05_presentation | [Packaging review](local-packaging-review.md) |
| Hosted-minute guards and existing release verifier | root | v05_remote | [Initial plugin/workflow review](plugin-initial-review.md) |

## Findings resolved before release validation

| Finding | Fix and independently verified behavior |
| --- | --- |
| Presentation guards disappeared when display fields became empty | Author `2424819fbe905a69c077d8ecce2f1a6caa6ab586`, integrated `496c750ebf1a7551dde6a9db5737e8f61eae5133`; guard-only state survives omitted-field patches |
| Boundary control characters were accepted in templates | Same presentation fix; direct validation rejects controls and bidi-format characters |
| CLI serialization changed literal quotes, backslashes or whitespace | `14a4f64608134f56022ea6abbe10cb209b11e692`; literal argv survives tokenization and invalid raw controls reach the validator |
| Relative plugin executables could depend on caller cwd | Same `14a4f64`; explicit relative commands are anchored to the plugin root before child cwd changes |
| One stale linked plugin broke all action discovery | Same `14a4f64`; per-plugin diagnostics preserve healthy action discovery, while stale invocation still fails |
| Installer bytes could change during staging copy | Author `a9a60b79d319e3f7a85d93f69ac8dcc2166e13f0`, integrated `5fd9423da2a7c2acf78a4b3b79514dab17646c85`; staged setup and sidecar hashes must match the verified originals |
| Early SSH exit could leave a helper holding inherited pipes | Author `923033dfaa79df448b37050dc0bc72ed59b98675`, integrated `0475da4faf910adf99bf1227d914a51650b4f3b1`; owning guard terminates Unix transport groups on every exit path |
| Cached workspace names could attach a renamed selection to a replacement | Same SSH fix; distinct stable-ID attach is resolved and registered under one daemon state lock; missing IDs fail without fallback |

Original failed reports and successor reports remain separate. Historical FAIL
records are not rewritten as PASS. Machine-readable finding records accompany
the reports. The SSH review explicitly distinguishes a stopped orphan zombie
from a still-running descendant.

## Focused evidence already obtained

- Transfer: seven independent native groups using live shell PTYs, unchanged
  PIDs/cwd/env, metadata/agent waits and persistence restart.
- Presentation: independent guard-loss reproductions, literal/control CLI cases,
  and code review of bounded token expansion and process-detection reconciliation.
- Plugins: independently built immutable source, 12 plugin unit tests, seven
  native smoke groups and nine independent path/discovery reproductions.
- Public plugin path: nine real subprocess checks against the owner's fixed
  GitHub manifest at `c6156b3d635fe9df016d77ba16cb4a5e5a07563f`; raw HTTPS bytes,
  inspect, disabled install and unchanged pinned update match. Live marketplace
  search returned HTTP 200 with zero public topic results. Third-party plugin
  validation and nonempty discovery transformation were not observed.
- Local tooling: 17 regression cases and 11 existing hosted-verifier self-tests;
  independent setup/sidecar copy races are both refused. Synthetic PE fixtures
  test evidence consistency only and never count as native Windows execution.
- SSH: independent original reproductions plus 13 real OpenSSH/PTY groups on the
  fixed source. Temporary pinned credentials and an explicit existing fixture
  account isolate the two local SSH/daemon instances. No production host or paid
  agent API is used.

## Final integrated validation and delivery

The release gate uses `scripts/validate_local.py` on the final clean public
candidate, after checking that its Git tree matches this reviewed source plus
reports. Browser-created commit SHAs differ from development commits; tree
identity is checked explicitly. The resulting `validation.json` identifies the
actual public commit, tree, complete command plan, exit codes, all source files,
logs and release binaries. `package_local.py` independently rejects changed,
missing, skipped or stale inputs. This document does not substitute focused
review tests for that final gate.

Linux has 25 required command records, including all 12 common product smokes
and the real loopback SSH smoke. Evidence is published with native scope and no
invented CI run ID. Supplemental cross-compilation checks are separate from
native runtime evidence. A release created with Linux-only execution must remain
a clearly labelled native Linux preview rather than a full-platform stable
release.

Native Windows/macOS execution and the new Windows Inno build plus all 14
installer lifecycle checks remain **NOT_RUN** unless a separate native report
proves them. The existing Windows local runner retains those checks; no step was
removed to manufacture a PASS. See [local validation](../../local-validation.md).

CI and Release workflows are manually disabled on the owner repository for this
quota-constrained delivery. Their source also has only manual dispatch, with
`allow_hosted_minutes` defaulting to false. No hosted jobs are dispatched, branch
protection is unchanged, and local validation is never posted as a fictitious
GitHub check. Compare the Actions run inventory before and after publication.

For source-review reproducibility, retain the separate development branch
history and the raw evidence archive alongside public release metadata. The
existing 0.4.0 release remains the rollback binary; program rollback does not
reverse application state or restore processes. The five features retain their
own documented limits rather than claiming full Herdr wire/UI/plugin parity.
