# META-04 independent review

**PASS for the implemented review scope; no new P1/P2 findings. Final integration remains NOT_RUN.**

- Author: `/root`; independent reviewer: `/root/reference_audit`.
- Base: `bf2190f8b15849fb1ad703110fdc53981fa0455c`.
- Tested source: `36e8aa5766abb99053873d76f7f9e6656fd745f1` in an isolated detached worktree.
- Platform: Linux x86_64. Repository source remained unchanged and clean.

## Source review

Validation and source/key capacity checks precede patch and sequence commit. The native API and CLI use the same validation. Token deadlines use monotonic time; read-time filtering leaves physical entries for one expiry event from the timer. The handler sweeps before applying a report, and the timer sweeps without requiring PTY activity. Clear/expiry retain source sequence high-water marks until resource closure. A replacement changes its own deadline; omitting TTL cancels that deadline. The State store is not part of persistence.

Pane and workspace keys have separate enum variants. Resource lookup requires a live target. The sweep collects actual live IDs and removes dead-resource metadata before expiry events. Pane metadata event snapshots include the current token map. Full server snapshots expose workspace IDs and tokens. Metadata never writes AgentInfo, instance, lifecycle state, or the agent wait/prompt state machines.

The three `(last_used, id)` tie-break changes in server.rs use the same deterministic selector for default target, attach, and fallback after closing a session. This removes vector-order dependence when timestamps tie. An explicit session target and non-tied recency retain their original behavior. The test-state constructor is cfg(test)-only and starts no listener, PTY, or background thread.

The new smoke checks actual server responses, stale report events, idle-time expiry, 32-key atomic rejection, agent wait independence, bridge literal arguments and restart behavior. It uses isolated environment/config/state/socket paths; the Windows prompt guard is present and its 15-second server timeout is shorter than the 30-second subprocess timeout. The script was reviewed but not represented as a successful complete run because its disclosed workspace projection dependency is unfinished at this SHA.

## Executed evidence

1. `cargo test --offline --locked metadata::`: exit 0; 11 passed.
2. `cargo build --offline --locked --bins`: exit 0.
3. `metadata-live.py` against this worktree's freshly built debug crew: exit 0; 9 real-server assertion groups passed. These cover pane event payloads, stale suppression, autonomous/per-key TTL, deadline replacement, 32-key and 32-source bounds, unconsumed sequence on rejection, agent occupant/wait independence, literal bridge input, closed-pane late-report rejection/no later expiry, and cold-restart clearing.

The probe uses no paid agent. All PTY/server data belongs to a disposable fixture. Exact binary/source/script/log hashes are recorded in review.json and the evidence index.

## Disclosed integration dependencies

- `workspace_json` needs the author-disclosed tokens field; workspace get/list/report/event projections omit it at this SHA. The independent live probe used the already wired full server snapshot for the workspace store check and explicitly printed KNOWN_PENDING for the missing projection. This is not counted as a new finding or as a passed projection test.
- Bridge workspace.move/tab.move await CORE integration.
- Run the unchanged original metadata smoke on the final combined candidate, then review the small integration diff.

Windows/macOS execution and final combined CI are NOT_RUN. No product-source edit or remote mutation was performed by this reviewer.
