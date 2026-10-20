# Independent review: SSH federation (fix verification)

- Reviewer: `/root/v05_presentation`; SSH author is `/root/v05_remote`.
- Feature base: `5e7e208bca5bf388aded7631aff0d8846b47363f`.
- Original candidate: `a80e0f9a7308bd944139e045a7e961b468d833bc`.
- Reviewed successor: `923033dfaa79df448b37050dc0bc72ed59b98675`, parent is the original candidate.
- Exact read-only source copy: `/tmp/crew-review-ssh-923033d` (git archive).
- Tested binary: `/workspace/scratch/21396cda8665/target-v05-remote-fix/debug/crew`.
- Binary SHA-256 independently checked before execution: `f5f794745948b55bc2ac04741face70cc5f202f6702b4c4d9182fc3313c3b8b3`.
- Verdict: **PASS for the reviewed SSH federation slice on native Linux**. SSH-P2-1 and SSH-P2-2 are resolved. No full-platform or integrated-release pass is claimed.

## Resolved SSH-P2-1: process cleanup after early SSH exit

The original `src/remote/transport.rs:241-248` could return from post-exit pipe collection without killing a ProxyCommand descendant that still held stderr/stdout. The successor creates an owning `TransportChild` guard immediately after a successful spawn; its destructor invokes the existing process/group stop-and-wait operation. This covers normal completion and every subsequent error path, including `try_wait`, collection and request-write failures. The Unix process group contains the SSH transport and ordinary ProxyCommand descendants. The documentation explicitly limits process-group cleanup to Unix.

I reran the original real OpenSSH/temporary ProxyCommand scenario with the expected-correct assertion. It returned the intended transport error after 0.545 seconds; the descendant was in state `Z`, with `descendant_alive_after_return: false`. This establishes termination of the running helper. It does not claim that an orphan zombie had already been reaped by its OS parent. The source regression additionally exercises stdout and stderr inheritance. No network is used by this separate error fixture.

Evidence: `/tmp/crew-ssh-review-923033d-proxy.log`; harness `/tmp/crew-ssh-review-fix-proxy-check.py`. The original vulnerable reproduction and native failure evidence remain at the a80e0f9-prefixed paths.

## Resolved SSH-P2-2: stable selection identity through attachment

The original selector at `src/remote/dashboard.rs:301-313` kept a stable ID in its cached selection but passed its name to attachment. Rename/name reuse could send input to another workspace.

The successor sends `AttachTarget::Id(&target.workspace)` through a distinct `attach-workspace wN` CLI verb and `Command::AttachWorkspace` protocol variant. `client::attach_workspace` validates the exact positive canonical wN form and uses `connect(false)`. The daemon preserves the ID while normalizing transport fields, then finds the session and registers the client under the same state lock. Missing IDs return an error; there is no name fallback or implicit daemon/workspace creation. Existing name-based `Attach` has unchanged wire fields and behavior. An older binary rejects the new verb, and an older daemon rejects the distinct enum tag rather than ignoring an optional new field.

My independent native PTY fixture selected w1, renamed it, and created w2 with the old name before Enter. The typed marker went to w1, and w2 did not receive it. A second fixture removed w1 and reused the name as w3; the selector reported attachment failure, and the replacement was not entered. Both fixtures restored terminal settings and stopped their temporary daemons. The expanded real SSH fixture independently covered the rename/name-reuse case through actual `ssh -tt` too.

Evidence: `/tmp/crew-ssh-review-923033d-rename.log`, `/tmp/crew-ssh-review-923033d-removed.log`; harness `/tmp/crew-ssh-review-fix-workspace-check.py` with `rename` and `removed` modes. The source and author's four compatibility-check records were inspected; those four compatibility cases were not separately re-executed by this reviewer.

## Independently executed checks

| Check | Result | Evidence |
| --- | --- | --- |
| Original inherited-pipe ProxyCommand case with corrected expectation | PASS, harness exit 0; product expected error exit 1; child no longer live | `/tmp/crew-ssh-review-923033d-proxy.log` |
| Cached selector, rename and old-name reuse | PASS, exit 0; input stays with original w1 | `/tmp/crew-ssh-review-923033d-rename.log` |
| Cached selector, removed ID and old-name reuse | PASS, exit 0; replacement receives no attachment | `/tmp/crew-ssh-review-923033d-removed.log` |
| Exact successor `scripts/ssh_federation_smoke.py`, real OpenSSH and PTY | PASS, 13 groups including the original ten, exit 0, simulation false | `/tmp/crew-ssh-review-923033d-native-01.log` and matching directory |
| Read-only fixture-process audit after tests | PASS, no live matching owned fixture process | `/tmp/crew-ssh-review-923033d-cleanup-audit.json` |
| Native Windows/macOS | NOT_RUN | Native environments unavailable |
| Full integrated Crew Linux validator | NOT_RUN by this reviewer | Root owns integration validation |

The 13 groups retain same-numbered machine ID isolation, no local fallback or remote auto-start, JSON command data and POSIX helper quoting, offline-host partial availability, timeout behavior, changed-host-key refusal, explicit reconnect, concurrent catalog writes, disable/remove preserving remote work, and actual attach/input/detach/refresh/TTY restoration. They add early-exit cleanup, stable remote workspace selection, and deleted-ID refusal. The isolated loopback fixture ran under the existing `oai` account, used temporary pinned keys/configs and named sockets, and did not modify user SSH configuration or contact a third-party service.

## Scope and remaining limits

Review is tied to the exact successor and binary above. I inspected the complete original SSH slice and its narrow successor, including CLI routing, remote command construction, timeout/no-replay behavior, host-key configuration, the async selector, and host-scoped IDs. I did not modify the author's checkout or review my own metadata/detection code. Native Windows and macOS behavior, and the final merged source/binary, still require their respective validation. The current transport requires POSIX remote hosts; Windows SSH servers remain unsupported as documented.

No reproducible blocking issue remains in this reviewed slice. Next owner: `/root` for integration and the local validation gate. Root should preserve the original failure reports and this successor record rather than rewriting historical failures as passes.
