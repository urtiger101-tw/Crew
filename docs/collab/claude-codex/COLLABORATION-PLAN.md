# Crew Proactive Collaboration and Fast Data Exchange Plan

Version: 2026-11-02 draft. Requirement: let Codex and Claude Code in Crew proactively find each other and reliably hand off (交接) data, reducing manual relaying by humans and screen polling.
Scope: two agents in the same local project; first reuse the Crew daemon, JSON API and CRL files, then add a persistent inbox (信箱).
The peer/msg commands and the automatic-receive adapter in this document are proposals; they are not yet implemented, and no end-to-end acceptance check (驗收) with two real agents has been run. For G, terminal-title reception and progress sync from an explicitly specified source have completed a first version in this round.
§62–65 narrowed the first version to named peers, send/list/read/ack and one receiver; concrete fields and the CLI await review (審查) and freezing in the [B v1 draft](contract/B-V1.md). Per §66, hook probes also start only after the freeze.

## Confirmed starting point

| Status | Existing capability and limits | Source |
|---|---|---|
| PASS / source code | CRL files are the content basis; the pane (窗格) text carries the notification. Two-way proposals, assignment (派工) and wake-up are authorized (授權). | [PROTOCOL.md:8](PROTOCOL.md#傳輸-transport), [PROTOCOL.md:55](PROTOCOL.md#協作角色-rolesv3-新增) |
| PASS / source code | `agent rename` can create a unique name for a reported (回報) agent; an exact name is resolved, and a duplicate name is rejected. An agent detected only by process cannot use this identity. | [src/api/agent.rs:167](../../../src/api/agent.rs#L167), [413](../../../src/api/agent.rs#L413) |
| PASS / source code | Explicit reports already have `source`, `seq` and `instance`; waiting recognizes when the occupant has been replaced, so a new process is not mistaken for finished work on the old task. | [src/api/mod.rs:216](../../../src/api/mod.rs#L216), [src/api/agent.rs:34](../../../src/api/agent.rs#L34), [228](../../../src/api/agent.rs#L228) |
| PASS / source code | `crew api serve` can keep one JSON-lines child process, matching asynchronous replies by request ID; up to 16 requests may be in flight at once, with a 1 MiB limit per line. | [src/bridge.rs:11](../../../src/bridge.rs#L11), [69](../../../src/bridge.rs#L69), [151](../../../src/bridge.rs#L151) |
| PASS / source code | `events list/wait` already have epoch, cursor, timeout and loss hints; the log holds only 512 entries in memory, and `pane.output` records only the byte count. | [src/events.rs:9](../../../src/events.rs#L9), [67](../../../src/events.rs#L67), [173](../../../src/events.rs#L173), [src/server.rs:551](../../../src/server.rs#L551) |
| PASS / source code | `pane run` sends text and Enter and then returns only `sent=true`; an agent's delay is 150 ms, which cannot prove that the model received the message or started processing it. | [src/api/pane/mod.rs:28](../../../src/api/pane/mod.rs#L28), [121](../../../src/api/pane/mod.rs#L121), [src/api/pane/keys.rs:160](../../../src/api/pane/keys.rs#L160) |
| PASS / source code | `agent prompt` fixes the instance and checks for blocked; when waiting, it observes working/blocked, then waits for the specified state. This is still process state, with no per-message receipt confirmation. | [src/api/agent.rs:554](../../../src/api/agent.rs#L554) |
| PASS / source code | `wait-output` searches the currently existing terminal logical lines each time and has no output checkpoint; an old EOT or an echo of a notification can match early. | [src/api/pane/mod.rs:159](../../../src/api/pane/mod.rs#L159), [src/api/pane/read.rs:171](../../../src/api/pane/read.rs#L171), [PROTOCOL.md:62](PROTOCOL.md#實務教訓-lessonsv1-新增) |
| PASS / source code | Display metadata already has title, state-label, token, TTL and source ordering; these are kept separate from agent work state and from the wait completion condition. | [src/metadata.rs:18](../../../src/metadata.rs#L18), [metadata-display.md:24](../../metadata-display.md#report-a-presentation), [metadata.md:6](../../metadata.md) |

§57 records that a Codex pane exited while app-server work continued and later moved to another pane; this shows that the pane and the agent session (工作階段) must be identified separately, and the reason for the exit is still unknown.
§58–60 show that the current handoff still requires the literal request "請讀某號檔案" (please read file N), and that review and completion status depend on text conventions; these records are not test evidence for the new inbox feature.

## Suggested minimal usage flow

1. Each agent's actual start/resume flow reports a unique `source`, sets a project-local peer name, and publishes its capability to accept messages.
2. The sender first saves the CRL or another handoff file, then sends one structured message containing a short summary, the file path and its hash; it gets the message ID immediately.
3. Crew writes the message to the inbox and then emits an event; the receiver's integration program reads new messages from the cursor, and either wakes the agent up proactively or waits until the current work ends.
4. After retrieving the content, the receiver explicitly reports `read`; on completion it reports `done` with an ok/failed result, replying with the original message ID and the verification evidence. Acceptance of responsibility is expressed through the reply; the first version has no accepted ack.
5. Users can see at any time what is pending reading, in progress, needing help, or completed; neither side needs to guess from the whole terminal screen whether work has been handed off.

This flow requires a receiver integration program. Crew has one inbox, which by itself will not give a model that is still reasoning any new content automatically; the receiving mode actually supported by each CLI/app-server must be verified.

## Priorities and delivery slices

| Priority | Work | Dependencies | Minimum delivery and acceptance check (驗收) |
|---|---|---|---|
| P0 | A: improve the existing collaboration flow first | None | Explicit reports, named peers, file handoff and a single notifier; after a pane is replaced, the binding can be re-established, and if a target is missing or ambiguous, delivery stops and an error is reported. |
| P0 | G: session title and trustworthy current progress | Can run in parallel with A | The Crew title can show the real discussion or session name, for example Review collaboration task; next to Working, it can show the current step and the source time, without inventing a percentage. |
| P1 | B: freeze the message and identity contract | Actual flow data from A | Clearly distinguish logical peer, daemon epoch, agent instance, message ID, reply-to and the various confirmation states; the CLI/JSON fixtures can be reviewed together. |
| P1 | C: persistent inbox and query | B frozen | A `send/list/read/ack` vertical slice with one receiver; still readable after restart (重啟), and retries do not produce a second message; msg wait is deferred. |
| P1 | D: proactive notification and adapters on both ends | Probes after B is frozen; integration after C is complete | Verify receiver capability; send one notification when ready, and keep pending items while busy or blocked; never resend Enter because of a timeout; notification coalescing is deferred. |
| P2 | E: fast data exchange and visible status | C, D | A resident JSON bridge, waiting per recipient, short summaries and file pointers; the TUI shows the unread count and the count needing a response, without putting full message text into the title. |
| P2 | F: output checkpoints and compatibility bridge | B; can run in parallel with C | Add `wait-output --after` for integrations that still need to read the terminal; limit new output by pane, epoch and output position, so old markers no longer match. |

Recommended order: deliver A→B→C→D first; schedule E and F according to measured bottlenecks. The first version reuses the existing daemon and does not introduce an extra broker, cloud service or cross-host routing.

## G: Let the user see what we are doing

- The user pointed out that the Codex footer already shows Review collaboration task, while the Crew header showed only codex; in this round, OSC 0/2 title reception, cleanup and name priority were added, and verified with an isolated daemon using Chinese and semicolon-containing titles.
- Added a [codex-progress compatibility bridge](CODEX-PROGRESS.md) that explicitly targets a rollout/pane and publishes only real assistant commentary; sync has been started on the local p4 pane, and both the API and actual debug-frame output show the session name and current step. The default daemon was not restarted; the new OSC parser is enabled with the binary built after Claude integration.
- Codex's local `tui.terminal_title` has been set to thread, task-progress and spinner; newly started CLIs will adopt it, and existing CLIs can select it immediately via /title. The setting is based on the [official example](https://learn.chatgpt.com/docs/config-file/config-sample); automatic selection and binding of new sessions is still pending stage D.
- Discussion titles, routing peer names and process kinds are stored separately; displaying a thread title must not change the recipient's identity, and an old title must not attach to a replaced agent.
- First reuse the existing presentation title, then connect the normalized terminal OSC title or the verified adapter thread title; a user's explicit override priority must be preserved.
- Progress shows the current step with its source, for example "正在讀取第 60 則" (reading message 60), "修正 API help" (fixing API help), "執行測試 3/5" (running tests 3/5), or "等待 Claude 審查" (waiting for Claude review); a fraction or percentage is shown only when the real total is available.
- The adapter reports task/step/activity/updated_at; when there are no real step events yet, show Working and the last activity time, and do not estimate completion from the spinner, output volume or elapsed time.
- Progress display is wired through the metadata source, seq and TTL; after expiry or disconnection it shows unknown or the last update time, so it does not look as if it is stuck on the same step forever.
- Work progress, message delivered/read status and work completion are three separate signals; progress text must not satisfy the message ack, agent wait or test completion conditions.

## A: Working methods we can improve right now

- After each real process start, use the existing `integration report ... --source ... --seq ...`, then set names such as `crew-codex` and `crew-claude` with `agent rename`; first confirm that the hook is actually connected.
- The name is used for lookup; `epoch + instance` is used to verify this receiver; after a resume, re-obtain the binding and do not reuse old pane numbers.
- The current `agent prompt` requires an explicit report; if there is only `detected_agent`, the display shows insufficient capability, and the manually confirmed pane notification method is kept.
- Write the handoff file to a temporary file first, verify the CRL header/END, and send the notification only after completion; the notification carries only a summary and a path, and does not include a raw EOT that a wait condition could misjudge.
- Each receiving agent has only one notification deliverer at a time, to avoid interleaved paste/Enter sequences from two sources; reply content stays in the CRL until the inbox feature passes acceptance.
- First measure ten actual handoffs: the manual steps, the number of CLI process starts, the number of screen reads, and the time from sending to an explicit reply; there is currently no baseline data.

## B / C: Message contract and persistent inbox

The proposed command interface is as follows; names and parameters are frozen only after B review, and these commands do not exist yet.

```text
crew peer list
crew msg send --from crew-codex --to crew-claude --kind review --summary "請審查交接" --repo-root D:/Crew --file docs/collab/claude-codex/log/61-codex.crl --key review-61
crew msg list --to crew-claude --open --limit 20
crew msg read MESSAGE_ID
crew msg ack MESSAGE_ID --by crew-claude --stage read
crew msg ack MESSAGE_ID --by crew-claude --stage done --result ok
```

In the example above, the `--summary` value "請審查交接" (Please review the handoff) is literal text sent to the peer and is kept unchanged.

- `peer` belongs to an explicit project/collaboration namespace; the name resolves to the current reporter. A detected claude.exe must not be treated as a confirmed recipient or as authorization.
- The first-version fields are `id/key/from/to/kind/reply_to/created_at/summary/artifacts/acks`; an artifact records a relative path, size, SHA-256 and a display-only sender repo_root, while the full test/diff remains in the file.
- `send` accepts a retry key created by the sender, scoped to from + key within the same store; the same key with the same content returns the same ID, and the same key with different content reports a conflict explicitly.
- §68 revision: `artifact.repo_root` does not participate in deduplication; a recipient may be an offline peer already known from store history; the caller's live reported name must match `--from`. The receiver queries `--open`, so work interrupted after `read` can still be resumed.
- `stored`: Crew has saved the message. `read`: the receiver/adapter has explicitly confirmed that it obtained the content. `done`, together with result=ok|failed: the receiver reports the processing result.
- `read` does not prove that the model understood; `done` also does not mean tests passed, that the change was merged or published; a completion reply still needs the IMP/TST/NR entries and evidence in the CRL.
- Notification status is stored separately as `deferred/notified/failed`; a successful TUI paste may be counted only as notified and must not be upgraded to read.
- Long content defaults to an artifact; the body of a short message has an explicit limit; lists return only summaries and can be filtered by kind/task/peer, avoiding repeated transfer of all history.
- Version 1 uses one bounded local JSON store per socket: state dir/inbox/socket-name.json, overridable with `CREW_INBOX_STORE`; at most 10000 messages, 16 artifacts per message, summary/note each 512 UTF-8 bytes, and keys 1..128 ASCII bytes. It reuses file-lock and atomic updates; when full it returns `store_full`, and v1 does not delete automatically; it is not mixed into the plugin registry. See [B v1](contract/B-V1.md) for details.
- A reply of `stored` may be returned only after the save succeeds; after a program restart, the message, ack and send-deduplication records can be recovered. Retention and cleanup must be set explicitly; unread messages are not deleted silently.
- The API additionally provides a persistent inbox cursor; the existing events cursor serves only as a fast notification and does not carry message storage. When `events_lost` or the epoch changes, backfill from the inbox cursor.
- Retryable transport and deduplication provide an explainable at-least-once pickup; it does not claim that model work is executed exactly once.

## D: Proactive wake-up without interrupting work

- Each end's adapter provides `capabilities` and explicit `input_ready/busy/blocked/offline`, kept separate from Crew's working/idle and from PTY quietness; without a ready report, Enter is never pressed automatically.
- If the CLI/app-server has a verified receiving interface, prefer notifying by message ID; otherwise use only a single-deliverer TUI compatibility path, and let the adapter report a verifiable read.
- While busy, store all messages and defer notification; when the agent next becomes ready to receive, it reads the inbox, and resending the same prompt message by message is forbidden. In the first version, notification coalescing is deferred until the minimal receiver has been measured.
- For blocked or confirmation screens, do not automatically send y, Esc or Ctrl-C; keep the message and let the user see that action is needed. This directly prevents the notification from being typed into a permission or login prompt.
- Before sending, check the destination's epoch/instance; when delivery is deferred until ready, re-resolve the logical peer. After the old occupant exits, an old delivery action must not send Enter to the new occupant.
- The message deadline only affects whether notification continues; it must not be treated as cancelling work the other side has already accepted. Cancellation uses a separate message, and the receiver confirms the result.
- Deduplicating same-kind events, limited backoff and a single watcher are enough to start; the existing 25 ms event polling is kept for now, and the notification primitive is changed only if a CPU or latency bottleneck is measured.
- Progress updates, proposals, questions, handoffs and completions each use an explicit kind; while waiting for review, the rules allow proposing independent work, but must not automatically expand permissions, start paid agents, or approve on someone's behalf.

## E / F: Faster exchange while keeping compatibility

- The adapter may reuse a single `crew api serve` and use request IDs to match replies; if a msg wait is added later, at most one time-limited wait is kept, so it does not fill the current 16 concurrent slots with many long waits.
- New msg/peer methods reuse the existing request/reply envelope; extend the Crew API first, and only then consider each agent's tool interface, rather than creating a separate, mutually exclusive protocol.
- Record `message.stored`, `message.acked` and `peer.changed` in the existing event log; events carry only IDs and status, and details are read back through the inbox.
- The existing `api snapshot` returns neither epoch nor cursor ([src/api/pane/query.rs:39](../../../src/api/pane/query.rs#L39)); initialization must not assume that it provides a seamless event entry point.
- A new peer/inbox snapshot should return the matching cursors together; an old API can first take the event position, then read state, then process subsequent events, and re-read on loss, so changes between snapshots are not missed.
- The output checkpoint must have a pane and an epoch, and must clearly define whether it counts raw output or logical line numbers; design the semantics first, then add the interface, and do not pass an event's byte count off as a terminal text cursor.
- Even with `--after`, the echo of a new notification may still match a text condition; a formal handoff completion relies on the message ID's reply/ack, and EOT is kept only for compatibility.

## Acceptance checks and measurement

The following are acceptance targets that have not yet been run; they are not current performance results. Tests first use an isolated socket/state and a free fixture receiver.

| ID | Acceptance condition | Evidence required |
|---|---|---|
| AC1 | When a pane is replaced, the daemon restarts, or two agents of the same kind exist, no delivery goes to the wrong one; ambiguity is reported explicitly. | Binding and reconnect integration tests, and records of the destination instance. |
| AC2 | After a message is sent, kill and restart the test daemon: unread messages, acks and deduplication results remain consistent. | Restart and same-key retry tests; not extrapolated as a power-loss guarantee. |
| AC3 | Old EOT, notification echoes, more than 512 events, or an event epoch change do not create false completions or miss persistent messages. | Tests that explicitly insert old markers, event overflow, and recovery. |
| AC4 | Enter is not sent carelessly while busy, blocked, when the input box has a half-typed draft, or when the instance is replaced; nothing is shown as read before the read step. | A receiver fixture whose ready state can be controlled, followed by a real two-end acceptance check. |
| AC5 | A local fixture of 1,000 short messages: p95 of the stored reply < 100 ms; p95 of notification for a ready adapter < 500 ms. | Record OS, machine, concurrency and payload; separately report the model's pickup time, and do not mix the two. |
| AC6 | Across ten identical handoffs, reduce manual relaying, extra CLI starts and full-page terminal reads; accuracy must not decline. | Compare with the A baseline; token savings require real records and must not be based on estimates. |
| AC7 | The existing CRL flow works when no adapter is enabled; the new features' `--help` has no side effects. | Compatibility tests, native Windows runs, fmt/clippy/test, and an independent review. |
| AC8 | Crew shows the real thread title and the current step; when an agent is replaced or expires, old data is cleared; an unknown total never shows a fabricated percentage. | Codex/Claude native TUI screenshots and state transitions; also confirm that metadata does not change wait/ack. |

## Work split, risks and next steps

- Contract B is proposed by one party and reviewed by the other; after freezing, Codex can build C's storage/API and Claude can build D's receiving adapter, each in its own worktree with non-overlapping file ownership (所有權).
- `api::Request`, the bridge method list and server routes are modified in sequence by the integrator; test agents may build AC fixtures in parallel, to avoid several parties editing the entry files at once.
- The largest unknown is the actual receiving/acceptable events of the Claude CLI and the Codex app-server; first run a controlled capability probe, and only promise automatic wake-up after it is confirmed.
- The main risk is treating "the process is still running / no output on screen" as ready, and treating `sent=true` as successful receipt; these are addressed by explicit capabilities, message IDs and staged acks.
- An artifact hash only verifies that content is consistent; it is not authorization. Message content is work data and cannot by itself grant publish, delete or credential-access permissions.
- The persistent inbox may store content that includes project information; version 1 stores only the necessary summaries and pointers, and the content scope, retention period and cleanup behavior are listed for contract review.
- Basic functions confirmed from the source code are PASS; new inbox, adapter, performance targets and cross-platform execution are NOT_RUN; the real provider receiving capability and the current latency baseline are unknown.
- Next step: both sides vote item by item on B's fields, ack semantics, identity rebinding and adapter capabilities, then choose the smallest end-to-end experiment for C+D. This plan does not equal implementation or deployment authorization.
