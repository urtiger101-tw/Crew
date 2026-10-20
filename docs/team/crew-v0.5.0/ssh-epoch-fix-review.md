# Independent review: SSH daemon epoch correction

- Reviewer: `/root/v05_presentation`; author: `/root/v05_remote`.
- Exact author candidate: `e82676349ed1a7153efed8c8489b7eeeff1d5a70`.
- Parent: `52709b62711dca03d8131a829ffcf581fe4364e5` (same-daemon keeper fixture correction).
- Product predecessor with the reproduced epoch defect: `923033dfaa79df448b37050dc0bc72ed59b98675`.
- Read-only exact source: `/tmp/crew-review-ssh-e826763` (git archive).
- Tested executable: `/workspace/scratch/21396cda8665/target-v05-remote-epoch/debug/crew`.
- Independently checked executable SHA-256: `6fa48223bce24e0fe7bc5062d6a07b9161118a249a51517f9a89bd8fd7b8601d`.
- Verdict: **PASS for this SSH correction and its native Linux regression scope**. SSH-P2-3 is resolved. SSH-P2-1 and SSH-P2-2 remain resolved and their regression cases pass. This is not a full integrated-release or platform-matrix result.

## Separate fixture correction

The failure at integrated source `276ad68338af7562add672a7b461d9772fbd4b1b`, smoke line 383, occurred before pressing Enter. Its log proves that closing the only workspace exited the daemon and a new daemon reused w1. Keeper commit `52709b6` repairs the intended same-daemon removed-ID test by retaining another workspace and checking its ID, name and active pane before/after the operations. It preserves all prior rejection, attachment-count, and terminal-restoration assertions. I reviewed this narrow diff and its author's thirteen-group record, then executed the inherited keeper case as part of the final fourteen-group run. The original full-gate failure remains preserved and is not relabeled a pass.

## Resolved SSH-P2-3: cached identity includes the daemon epoch

The old product's ID-only selection could cross a cold restart and attach a different w1. My original independent native PTY reproduction and failure record remain at `/tmp/crew-ssh-review-923033d-restart-repro.py` and `.log`, with the follow-up review/findings at the same restart-prefixed paths.

The successor makes two atomic guarantees:

1. `Request::FederationSnapshot` in `src/api/mod.rs:362-371` collects workspace rows, the shared agent-list projection, and the event-journal epoch under one State lock. The helper now makes this one request, replacing separate workspace and agent reads. IDs cannot be paired with an epoch read from another daemon lifetime.
2. The selector retains that epoch in its Target and passes it through `__attach-workspace-epoch` and a distinct `Command::AttachWorkspaceEpoch` tag. `src/server.rs:1449-1459` checks the expected epoch before resolving the ID; its caller at 1596 onward retains the same lock through client registration. Mismatch rejects before attaching. There is no preflight/attach race and no fallback to another ID or name.

A missing epoch makes an old helper's rows unselectable. The new atomic request fails closed on an older daemon. The distinct attach tag and verb prevent an older endpoint from silently ignoring a new guard field. Explicit legacy name/ID commands retain their existing current-daemon semantics. Federation JSON now exposes `machine_epoch`, and documentation correctly scopes cached identity to `(machine_id, machine_epoch, resource_id)`.

## Independently executed verification

| Check | Result | Evidence |
| --- | --- | --- |
| Original cross-restart PTY scenario with expected rejection | PASS, exit 0 | `/tmp/crew-ssh-review-e826763-epoch-native.log` |
| Complete exact-candidate native OpenSSH/PTY regression | PASS, 14 groups, exit 0, simulation false | `/tmp/crew-ssh-review-e826763-native-01.log` and matching directory |
| New/old epoch compatibility, reviewed author fixture rerun with a separate report path | PASS, 4 groups, exit 0, simulation false | `/tmp/crew-ssh-review-e826763-compat.log` and `.json` |
| Windows/macOS native execution | NOT_RUN | Native environments unavailable |
| Complete integrated root runtime gate | NOT_RUN by this reviewer | Root owns the full local gate |

The independent restart fixture explicitly waits for the old daemon to exit before creating a replacement with the same ID and name. On the fix, old epoch `140683-1792498665615905654` and replacement epoch `140727-1792498665993261916` differed while both IDs were w1. Enter failed, replacement `attached` remained 0, and terminal settings were restored. Harness: `/tmp/crew-ssh-review-epoch-fix-check.py`; transcript: `/tmp/crew-ssh-review-epoch-fix-transcript.bin`.

The fourteen-group run retained the original routing, no-local-fallback, no-remote-auto-start, JSON/quoting, host-key rejection, timeout/no-replay, concurrent catalog, machine-ID isolation, proxy cleanup, and real remote attach/detach checks. It includes same-daemon rename/name reuse, the corrected same-daemon removed-ID case, and a separate deterministic cold-restart case. Its record is `/tmp/crew-ssh-review-e826763-native-01/result.json`; the cold-restart evidence is `restart-guard.json` in that directory.

The four compatibility groups independently confirmed new epoch attachment rejection by the older ID-capable daemon; old CLI rejection of the distinct verb; new snapshot failure without falling back to unbound reads; and continued acceptance of legacy Attach plus strict new-verb argument validation. The old comparison binary is the fixed-ID predecessor with SHA-256 `f5f794745948b55bc2ac04741face70cc5f202f6702b4c4d9182fc3313c3b8b3`. Reviewer execution used a copied, reviewed fixture that writes only `/tmp/crew-ssh-review-e826763-compat.json`, preserving the author's evidence.

## Narrow integration source check (not an integrated execution)

At root commit `1ea47032b60098f3e43bd8736212484a81c7191e`, I also checked the reported merge-conflict resolution read-only. `Request::Metadata` remains boxed and its handler still dereferences `*req`; the complete FederationSnapshot branch is present. The agent-list change is visibility only, preserving root's explicit-agent-or-detected-agent condition. `pane_json` still exposes `detected_agent`, `process_detection`, `display`, and tokens, and the background `agent_detection::refresh` call remains. This confirms the narrow integration preserved those existing paths; it is not a new independent approval of my own presentation implementation, nor a runtime test of the merged binary.

## Handoff

No blocking finding remains in the reviewed correction. All execution used authorized temporary local fixtures; the SSH smoke used pinned loopback keys and the existing `oai` fixture account. No third-party service, GitHub write or hosted CI was used, and no author's checkout was modified.

Next owner: `/root` for source-bound documentation, the complete local gate on its final integrated public SHA, packaging, and accurate platform limitations. Preserve the original failing gate, the 923033d epoch-defect addendum, and this successor record as separate historical evidence.
