# Independent follow-up: SSH selector daemon epoch

- Reviewer: `/root/v05_presentation`, not the SSH author.
- Product candidate: `923033dfaa79df448b37050dc0bc72ed59b98675`.
- Tested binary SHA-256: `f5f794745948b55bc2ac04741face70cc5f202f6702b4c4d9182fc3313c3b8b3`.
- Triggering integrated gate: source `276ad68338af7562add672a7b461d9772fbd4b1b`, `/tmp/crew-v05-native-evidence-03/logs/25-ssh-federation.log`.
- Current follow-up verdict: **FAIL pending new SSH-P2-3**. Previously fixed SSH-P2-1 and SSH-P2-2 remain resolved for their tested same-daemon conditions. Their historical review/test records are not rewritten.

## Distinguish the gate fixture failure from the product defect

The gate fails at `scripts/ssh_federation_smoke.py:383`, before pressing Enter. Its local log records the original daemon PID 135850, the final pane EOF, and a new daemon PID 136182. At integrated `src/server.rs:1755-1762`, closing every workspace exits the daemon, regardless of the `persist` setting. A new State starts its workspace counter at zero (`src/server.rs:1671`); with no restored layout, its first new workspace is w1. Therefore the fixture's implicit assumption that closing its only workspace leaves the same ID allocator running is invalid. A keeper workspace is the correct minimal repair for the intended same-daemon removed-ID regression. The reviewer's earlier independent removed-ID fixture already had a keeper and does not rely on that race.

That fixture repair does not address a second, real product path: a cached selector outlives the old daemon, and the replacement daemon can allocate the same workspace ID.

## SSH-P2-3: bind cached selection to the daemon epoch as well as workspace ID

Relevant locations at exact product candidate: `src/remote/dashboard.rs:58-63` and its attachment call at 313-314; `src/remote/transport.rs` summary response; `src/server.rs` AttachWorkspace ID lookup. The cached Target carries machine, workspace and name, with no daemon epoch. The attachment protocol likewise carries only workspace ID. Consequently the server cannot distinguish a fresh w1 from the w1 that was observed in an older daemon.

Native Linux PTY reproduction:

1. Start an isolated daemon with `persist = false`; create and select w1.
2. Close its only workspace and explicitly wait until `workspace list` fails, proving the old daemon is gone.
3. Create a new workspace with the same name in the same named socket. It is w1 in a new daemon.
4. Press Enter before the selector's fifteen-second refresh and type a fixture marker.

The marker reaches the replacement daemon. The run completed in about 0.5 seconds. Observed event epoch changed from `136892-1792498074716044348` to `136917-1792498075094770778`, while both old and new IDs were w1. This is a real stale-selection identity defect, independent of name reuse within one running daemon. The fixture was cleaned up and did not contact SSH or an external service.

Reproducer: `/tmp/crew-ssh-review-923033d-restart-repro.py`.
Evidence: `/tmp/crew-ssh-review-923033d-restart-repro.log`.
PTY transcript: `/tmp/crew-ssh-review-923033d-restart-transcript.bin`.
Reproducer exit 0 means it successfully demonstrated the defect; it is not a product PASS.

## Required correction and recheck

The summary must bind its workspace data to the observed daemon epoch. Carry that epoch with the selected workspace into the attach protocol, and compare it under the same daemon state lock used for ID resolution and client registration. Epoch mismatch must reject and require a fresh selection, without name/ID fallback. Merely reading the epoch before a separate unguarded attach leaves a race. Ensure older endpoints cannot silently ignore the new guard: a distinct protocol tag or equivalent fail-closed compatibility path is required.

Retain the same-daemon keeper regression and add this deterministic cold-restart case. Review the immutable successor, rerun the targeted epoch test, and rerun the required native SSH/PTY smoke. Windows/macOS remain NOT_RUN; root still owns the integrated full validation gate.

Next owner: `/root/v05_remote` for the authorized narrow fix, then this independent reviewer on the successor SHA.
