# Claude Stop receiver: registration and acceptance-check boundaries

This document describes `scripts/adapters/claude_stop_receiver.py`. The configuration template does not write to any Claude settings and does not enable a hook.

## Set the runtime environment explicitly

The process that starts Claude must provide the following environment; the receiver inherits the same values:

| Variable | Setting and purpose |
|---|---|
| `CREW_RECEIVER_NAME` | An existing named peer, for example `crew-claude`; format `[a-z][a-z0-9_-]{0,31}`. If unset, Stop is allowed. |
| `CREW_RECEIVER_CREW` | **Absolute path to the Crew executable.** Bare `crew`, `crew.exe`, relative paths, and Windows drive-relative paths are not accepted; the cwd and PATH are not searched. |
| `CREW_RECEIVER_REPO_ROOT` | **Explicitly set the receiver's own absolute root**, for example `D:/Crew-agentcmd`. All artifacts are resolved and checked against this root; the sender's `repo_root` is not used as a fallback. If omitted, the existing compatible behavior uses the cwd from the hook stdin; do not rely on this default in multi-worktree setups. |
| `CREW_RECEIVER_STATE` | Absolute path of the adapter's own JSON file; stores notification state and ack failure counts. It must not point to the Crew inbox (收件匣) store. If omitted, the platform's user state directory is used. |
| `CREW_SOCKET` etc. | Explicitly set the target socket and the matching `CREW_STATE`, `CREW_INBOX_STORE`, `CREW_CONFIG`, `CREW_PLUGIN_HOME`, and `CREW_LOG`; tests always use the unique isolated values created by the fixture. `CREW_PANE_ID` is inherited from the matching caller. |

Isolated acceptance check (驗收) may use the copied `D:/Crew/target/crew-coop-20261102-121955/crew-preview.exe`. The test program creates its own socket and does not use the collaboration preview socket or the default daemon.

After providing the environment above, register the Stop template as follows. Change the Python and script paths to the absolute paths confirmed on the deployment machine. **The command hook timeout is explicitly 45 seconds**:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"C:/Python314/python.exe\" -X utf8 -B \"D:/Crew-agentcmd/scripts/adapters/claude_stop_receiver.py\"",
            "timeout": 45
          }
        ]
      }
    ]
  }
}
```

## Delivery and failure behavior

- If `stop_hook_active=true`, Stop is allowed immediately. For malformed stdin, an invalid peer name, or an unset or non-absolute Crew path, no Crew call is started.
- First fetch pending messages page by page with `--unread`, then separately query `--open` for the count of read-but-not-done messages. Stop is allowed only for read-but-not-done items, and nothing is re-delivered. This is the receiver's current strategy; it does not mean the original work has resumed or completed.
- Each summary, artifact path, and metadata is presented as a JSON string. Unicode `Cc/Cf/Zl/Zp/Cs/Co` characters are removed, and newline, tab, and NEL are converted to spaces. Each round generates a new nonce, and each message is wrapped in `BEGIN/END CREW MESSAGE <nonce> <index>`. This is a data-boundary hint; it is not proof that the model is immune to prompt injection.
- First build and ASCII-safe encode a single block JSON, then write and flush stdout, and **only after both steps succeed, ack read**. If encode, write, or flush fails, a diagnostic is recorded and no read ack is attempted. Bytes already written cannot be retracted if flush fails. The receiver never acks done automatically.
- Read-ack failures are counted each time and persisted on normal exit, with a limit of **3** per ID. Once the limit is reached, the message is no longer delivered in full; a single stderr line `stuck message "<id>"` asks for manual handling, and if only stuck messages remain, Stop is allowed. A failed message remains unread in Crew and must not be treated as done. After the connection or store is repaired, the handler explicitly fetches the message, completes the work, and confirms per the contract. The adapter does not skip the confirmation step on its own.
- The adapter state keeps at most 500 records. Records that are lost, corrupted, fail to write, or are evicted cannot preserve the previous counts. In that case a message may be delivered again and the applicable diagnostic is left behind; there is no persistent exactly-once guarantee.
- If the length or SHA-256 does not match, or the path is missing, out of bounds, or a Windows trailing-dot or trailing-space alias, the status is marked and the artifact cannot be used as verified. A failed status is still delivered with the summary and explicitly read-acked; a read does not mean the artifact passed. When a single reason is too long, only complete lines are kept, the nonce closing boundary is preserved, and `ARTIFACT LINES OMITTED` is marked. A complete manifest must be fetched separately before use.

## Limits and unproven cases

The total work deadline is **28 seconds**; each Crew subprocess timeout is the smaller of the remaining budget and 10 seconds. Each artifact is hashed for at most **64 MiB**: the actual file size is checked first, and the shared deadline is checked before and after each chunk read. An artifact that is too large or times out is marked `UNVERIFIED`. Hashing also stops if the file grows during reading. Blocking in filesystem open/read, stdout flush, or state writes cannot be forcibly interrupted by this cooperative deadline; the outer 45-second hook timeout provides an additional bound. Power-loss and slow-filesystem tests have not been performed.

By default each selector reads at most **5 pages × 20 = 100 headers**. Old messages that failed to read or are stuck still occupy the unread window; if they fill the window, newer messages behind them may not be seen. This is the explicit limit for poison reads; there is no hidden skipping, deletion, or unbounded scan. The page-count and size environment variables are only for adjusting isolated tests.

If two Claude sessions use the same receiver name, they may deliver the same message at the same time; this adapter has **no cross-session lock**. Each session should use its own named peer and state path. Atomic state replacement does not mean delivery is mutually exclusive.

If the process is interrupted after stdout flush but before the read is saved, the message may be resent. If it is interrupted after the read is saved but before the host receives the reason, the message may still not have reached the model. Model understanding, real hook activation, pending composer/approval UI, queue delivery, and merge/push cannot be concluded as PASS from the synthetic Stop fixture.

## Run a specific fixture

```text
python -X utf8 -B scripts/test_claude_stop_receiver.py D:/Crew/target/crew-coop-20261102-121955/crew-preview.exe
```

The fixture uses a unique socket and temporary config, state, and store, and stops only the daemon it created itself. Results for unit/fake cases and for real isolated-daemon cases must be recorded separately. An environment block must not be recorded as a receiver PASS.
