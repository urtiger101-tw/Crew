# Independent review: SSH federation (original candidate)

- Reviewer: `/root/v05_presentation`, not the SSH author.
- Base: `5e7e208bca5bf388aded7631aff0d8846b47363f`.
- Immutable head: `a80e0f9a7308bd944139e045a7e961b468d833bc`.
- Read-only source copy: `/tmp/crew-review-ssh-a80e0f9`.
- Tested executable: `/workspace/scratch/21396cda8665/target-v05-remote/debug/crew`.
- Executable SHA-256: `b11e020ff3e50444ee80939464db9de30b03013f1104cb66aee29066d52f6fcc`.
- Current verdict: **FAIL pending SSH-P2-1 and SSH-P2-2**. The normal native Linux OpenSSH/PTY regression passes. Windows/macOS native checks are **NOT_RUN**.

## SSH-P2-1: clean up descendants after an exited transport leaves a pipe open

Location: `src/remote/transport.rs:241-248` (also other error propagation after spawning).

`capture()` creates a separate Unix process group and calls `stop()` on timeout, cancellation, or a polled output overflow. Once `try_wait()` reports the direct SSH child has exited, however, the final pipe collection uses `?`. If a ProxyCommand descendant inherited stderr and outlives that direct child, the 500 ms collection returns `transport stderr did not close` without stopping the process group. The descendant remains alive. Inside a running federation selector the detached reader thread remains blocked too. Repeated refreshes can accumulate those resources.

This is reproduced with actual `/usr/bin/ssh`, a private temporary SSH config, and a local Python ProxyCommand. The proxy creates a sleep helper with only stderr inherited, then exits. No remote service or network connection is used. The exact Crew binary returns exit 1 after 0.554 seconds, but the helper remains in state `S` after the call: PID 114695, process group 114690. The fixture kills its helper in `finally`.

Reproducer: `/tmp/crew-ssh-review-a80e0f9-proxy-repro.py`.
Native evidence: `/tmp/crew-ssh-review-a80e0f9-proxy-native-repro.log`.
The first sandboxed attempt `/tmp/crew-ssh-review-a80e0f9-proxy-repro.log` was inconclusive because PID namespaces did not match `/proc`; it is preserved and is not treated as a product failure.

Request the author to protect all post-spawn return paths with cleanup, including early child exit and stdout/stderr/stdin collection errors, and add a regression that proves a pipe-holding descendant no longer lives after return. Recheck the original reproducer on an immutable successor and retain the existing real SSH/PTY regression.

## SSH-P2-2: attach the selected workspace ID instead of a cached name

Location: `src/remote/dashboard.rs:301-313`, especially the call to `attach(machine.as_ref(), Some(&target.name))` at line 313.

The selector's `Target` already contains a stable workspace ID, but attachment discards it and uses the cached display name. The list refresh interval is fifteen seconds. If another client renames the selected workspace and creates a different workspace with its former name during that interval, Enter silently attaches to the new workspace. Rechecking `targets.contains()` only checks the same cached snapshot; the catalog reload checks machine configuration, not workspace identity.

Native Linux daemon/PTY reproduction on the exact binary selected `w1 / review-original`, renamed that workspace to `review-renamed`, and created `w2 / review-original`. In less than a second, before any scheduled refresh, Enter attached to w2. A typed fixture command appeared in w2 and did not appear in w1. This is user input delivered to a different workspace than the selected stable ID, not just a stale label. The temporary daemon and PTY were cleaned up.

Reproducer: `/tmp/crew-ssh-review-a80e0f9-workspace-repro.py`.
Evidence: `/tmp/crew-ssh-review-a80e0f9-workspace-native-02.log` and `/tmp/crew-ssh-review-a80e0f9-workspace-transcript.bin`.
The first fixture attempt used an incorrect pane JSON path and failed before opening the selector; `/tmp/crew-ssh-review-a80e0f9-workspace-repro.log` is preserved as a harness failure, not product evidence.

The selector should pass the stable workspace ID through attachment and have the destination daemon resolve it under its normal state lock, or reject if that ID no longer exists. A preflight name query alone still leaves a check/attach race. Direct user-requested `machine attach --workspace NAME` can keep its documented name semantics. Add rename/name-reuse and removed-ID tests for chooser attachment.

## Independently executed native Linux regression

Command: `python3 scripts/ssh_federation_smoke.py /workspace/scratch/21396cda8665/target-v05-remote/debug/crew --fixture-user oai --logs /tmp/crew-ssh-review-a80e0f9-native-01` from the read-only candidate.

Result: **PASS**, exit 0, all ten groups, `simulation: false`. The fixture uses the existing unprivileged `oai` account because this container's root SSH interactive session lacks the required audit capability. It creates only temporary keys/configs/sockets, pins the temporary host key, leaves the user's SSH configuration unchanged, and cleans up its local daemon and SSH processes.

The ten groups cover actual SSH and quoted executable paths; stdin JSON remaining data; independent same-numbered local/remote IDs; no remote auto-start; partial availability beside offline hosts; bounded operation timeout with later independent requests; changed host-key rejection and explicit reconnect; concurrent atomic catalog updates; disable/remove preserving remote work; and a real selector PTY that attaches, routes input to the remote pane, detaches, refreshes, and restores terminal state. They also preserve a malformed catalog.

Logs: `/tmp/crew-ssh-review-a80e0f9-native-01.log` and `/tmp/crew-ssh-review-a80e0f9-native-01/` including the terminal transcript and result record.

## Reviewed paths and limits

Routing validates commands before invoking exactly one remote transport and has no local fallback. Remote arguments use JSON stdin, while helper paths and attach arguments use POSIX quoting. Timeouts do not replay possibly delivered mutations. OpenSSH retains host-key policy and chosen config. Machine catalog IDs are never reused, results carry machine IDs, and async selector probes share cancellation plus worker joins. Native attach intentionally inherits the terminal and uses the existing remote daemon. Supported remotes are POSIX Linux/macOS; native Windows SSH servers are explicitly unsupported.

This record is not a full platform matrix or release approval. It does not review this reviewer's own metadata/detection work. Root has assigned the SSH author to fix SSH-P2-1, and SSH-P2-2 has been reported to root and the author. The next review must identify the successor SHA and separately verify cleanup, stable selection identity, and the ten native Linux regression groups.
