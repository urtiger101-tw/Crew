# Crew v0.3.0 independent review — stage 2

## Verdict

**PASS for the specified non-AGENT implementation scope at
`a421a6bd3808f5930a00e4dd921298d98ddd449b`.** No unresolved P1/P2 findings remain in this
reviewer's scope. This is a Linux source review and local verification result, not a CI,
cross-platform, PR, merge or release approval.

- Reviewer instance: `/root/reference_audit`; recipient and next owner: `/root`.
- Base: `12be9e307e47a56d3bfd669ab7bddb1ee7466e09`.
- Final reviewed and independently built/tested head: `a421a6bd3808f5930a00e4dd921298d98ddd449b`.
- Original worker deliveries: CORE `b2a691866a357e5fdcaaf78d115edf2c039e6d0d`,
  GIT `2f01881f7268918879852eae4255e159d669d74d`.
- Integration/fixes examined: `cb85172`, `3e24e5c`, `a421a6b`.
- Working directory: `/workspace/scratch/21396cda8665/wt-review` (clean detached checkout).
- Platform: Linux x86_64; real Unix sockets, PTYs and local Git checkouts.

Scope: brand and compatibility paths, workspace/tab stable IDs and caller context,
cwd/environment inheritance and persistence, Git worktree operations, events, JSON-lines
bridge, client request transport and the two previously unsafe smoke scripts. The reviewer
did not modify these product sources. The separately written regression script is supplied
for the coordinator to integrate.

**Excluded:** `src/api/agent.rs`, `src/integrations.rs`, and their authored identity behavior.
This reviewer authored their original implementation and does not independently approve it.
R5 belongs to `/root/target_audit`'s independent AGENT review. Provider processes were never
launched. No external repository, Actions run, PR or release was written by this reviewer.

## Finding dispositions

| ID | Severity | Disposition and evidence |
|---|---|---|
| R1 | P1 | Fixed by CORE state selection marker. Real-server clear/restart now restores only `fresh`; the unselected legacy file remains byte-identical. Corrupt new state is quarantined without reverting to or overwriting legacy data. Fixed-state implementation remains unchanged at the final head. |
| R2 | P1 | Fixed in `cb85172`. Both `smoke.py` and `api_smoke.py` were run with an inherited disposable CREW_SOCKET containing `sentinel`; each exited 0 and the original server still contained only `sentinel`. Both scripts now clear the new and legacy environment families before assigning independent config/state/socket paths. |
| R3 | P2 | Fixed in `cb85172`: Linux live shell/foreground cwd is inspected outside State. Exact-head regression confirms a shell that cd'd into the checkout blocks removal with `worktree_in_use`, preserves tracked content, and permits removal after leaving. |
| R4 | P2 | Fixed in `3e24e5c` (worker `ce345c7`). An inactive caller pane now resolves its own tab through both CREW_PANE_ID and TMUXLIKE_PANE_ID. Explicit workspace scope or tab ID still takes precedence. Custom reproduction and exact-head workspace smoke pass. |
| R6 | P2 | Fixed in `a421a6b`. A deterministic Git-status barrier reproduced the old race on `3e24e5c`: a workspace opened while removal was pending, and its checkout was deleted. The fixed version reserves the canonical path under State and rejects workspace/tab/split creation beneath it. Git I/O stays outside State. Success and error paths release the reservation. |

An additional lifecycle observation was also resolved: workspace close now records
`pane.closed` for every removed child pane. A two-pane workspace verified both events.

## Final-head verification

Independent build, exit **0**:

```sh
env CARGO_HOME=/workspace/scratch/21396cda8665/toolchain/cargo \
  RUSTUP_HOME=/workspace/scratch/21396cda8665/toolchain/rustup \
  CARGO_TARGET_DIR=/workspace/scratch/21396cda8665/target-review \
  /workspace/scratch/21396cda8665/toolchain/cargo/bin/cargo build --offline --locked --bins
```

The following commands all ran from the clean final checkout and exited **0**:

```sh
python3 scripts/workspace_smoke.py /workspace/scratch/21396cda8665/target-review/debug/crew
python3 scripts/worktree_smoke.py /workspace/scratch/21396cda8665/target-review/debug/crew
python3 scripts/bridge_events_smoke.py /workspace/scratch/21396cda8665/target-review/debug/crew
python3 /tmp/crew-review-stage2/worktree_race_linux.py /workspace/scratch/21396cda8665/target-review/debug/crew
```

- Workspace: stable IDs across rename/deletion/restart, ID high-water marks, caller tabs,
  explicit scope, focus/no-focus, cwd, environment override/inheritance, split inheritance,
  startup command and legacy version-1 migration.
- Worktree: 13 real checks covering Unicode/spaces, create/open/reuse, real PTY cwd, hook
  suppression, dirty/untracked/ignored-file guards, locked/primary/foreign checkout guards,
  changed `.git` pointer, branch retention, malformed input and preserved checkout after
  terminal spawn failure.
- Bridge/events: strict envelopes and errors, IDs, text replies, concurrent waits, restart
  epoch rejection, 1 MiB limit and recovery after an oversized request.
- R3/R6: live cwd refusal and later release; workspace/tab/split refusal while a real Git
  status result is held at a deterministic barrier; unrelated workspace progress; no leaked
  panes; path reuse after successful removal; reservation cleanup after dirty rejection;
  dirty data preserved.

Earlier journal-specific tests also passed: 512-entry retention, loss flag, contiguous
cursor pagination, timeout cursor preservation, future cursor rejection and old epoch
rejection. A session query completed in 2.5 ms during a pending event wait. Journal source
has not changed since that test.

## Source and artifact hashes (SHA256)

| Source/artifact | SHA256 |
|---|---|
| `src/workspace.rs` | `696f86a36b6596507c58eee97543da361c04b03dc0e88830ab47ca871adc9f32` |
| `src/worktree.rs` | `3aa223b2501e4cd5bbd3b8c5cedb713f846bcd5f265905e7279377fcb8a054fa` |
| `src/bridge.rs` | `6c7558e0453354779f349da7bac34d743489bbb86ecd29051b94d13fe4f17b03` |
| `src/events.rs` | `76aa23af523ba1bd762e96d0555e2ef9db3e1d5d4f958c1d175ad326d05b2349` |
| `src/persist/mod.rs` | `f75ce65ca45e14e5bf8ebf2f958eda0d7708992fc849cef3506667357734b22f` |
| `scripts/smoke.py` | `1d9f52627659aad495653bcf2225544a75a7da65beba981769a2645adf7d5a2d` |
| `scripts/api_smoke.py` | `3eb305ba365b247dfd7663a4512b4331b0bd7d56fb2f8e67529a9289175c0f0e` |
| Independent binary `target-review/debug/crew` | `a7b7bd7a6491fc3beaa4d55a17047bd64aacc25ef4da6bbf3f5eb14be19f27cc` |
| Supplied `worktree_race_linux.py` | `6f3c10369e268fc7e1285a1e81e81add2a2f53b260ce69704832db93980e2bd1` |

## Known limits / NOT_RUN

- Windows/macOS execution and GitHub CI: **NOT_RUN by this reviewer**; actual matrix jobs
  must determine those statuses. Live process cwd protection is Linux-specific.
- `scripts/e2e_linux.py` interactive scenario: **NOT_RUN**; its environment isolation repair
  was inspected statically. The inherited-server regression executed smoke.py and api_smoke.py.
- Final CI/release/package workflow changes: outside this reviewed implementation head and
  scope; the delivery reviewer must assess those files and the resulting latest candidate.
- A separate process can change cwd or alter files while Git is running. The reservation
  closes Crew's own pane-creation race; it is not a filesystem lock on arbitrary processes.
- The newline bridge intentionally uses Crew CLI argument arrays, not Herdr's native wire
  protocol. EOF waits for pending requests, so wait callers should set finite timeouts.
- Herdr advanced gaps remain explicitly described in `docs/herdr-parity.md`; this review
  does not claim full Herdr feature parity.
- No full unit/clippy matrix was rerun by this reviewer at the final head; the coordinator
  owns those required gates. The exact-head checks above are the independent evidence.

Next owner: `/root` integrates the Linux regression, combines independent AGENT and delivery
reviews, runs required gates against the latest candidate and records actual CI results.
