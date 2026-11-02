# Durable peer messages

Crew stores messages between explicitly named, reported agents. Set a name after
reporting the agent on its pane:

```sh
crew agent rename p1 crew-claude
crew agent rename p4 crew-codex
crew peer list
```

Use the actual pane IDs returned by Crew. A detected process or a terminal title
does not create a peer. The snapshot includes the daemon epoch and each peer's
name, pane ID, report instance, source, agent kind and reported state.

Send from inside the named agent's pane, where `CREW_PANE_ID` identifies the caller:

```sh
crew msg send --from crew-codex --to crew-claude --key review-1 --kind review --summary "Please review the attached handoff" --repo-root /path/to/checkout --file docs/handoff.md
crew msg list --to crew-claude --open --limit 20
crew msg read MESSAGE_ID
crew msg ack MESSAGE_ID --by crew-claude --stage read
crew msg ack MESSAGE_ID --by crew-claude --stage done --result ok --note "Review recorded"
```

`--from` must match that pane's current named report. An offline recipient can
receive new messages if its name already occurs in the store. `--from` and `--by`
are responsibility checks within the same local user account, not authentication.

Kinds are `handoff`, `review`, `question`, `answer`, and `done`. Use `--reply-to ID`
to reference an existing message. Sending a reply never acknowledges its parent.
Reading retrieves content without changing acknowledgments. A receiver explicitly
acknowledges `read` before `done`; done requires `--result ok|failed`.

An identical sender/key retry returns the original message with its current acks
and `deduplicated: true`. Changing the submitted content returns `message_conflict`.
Artifact comparison uses ordered path/hash/byte triples, excluding the sender's
display-only repository root. An identical acknowledgment retry returns
`changed: false`; changing an existing stage returns `ack_conflict`.

`list` returns short headers, `next_cursor`, and `has_more`, oldest first. Limits
are 1–100 (default 20). `--after CURSOR` resumes after the last returned message;
cursors belong to the store and survive daemon restart. An empty page preserves
the input cursor. `--unread` selects messages without read acknowledgments;
`--open` selects messages without done acknowledgments, including read work still
awaiting completion. These filters are mutually exclusive. Receivers should poll
`--open` to recover read work after a crash.

Artifacts require an explicit absolute `--repo-root` and repeatable relative
`--file` paths using `/`. The frontend verifies root containment and computes
SHA-256 and byte length. The daemon stores the manifest without opening the
sender's root. A receiver must resolve paths under its own configured root,
verify containment, bytes and hash before consuming content, and report missing
or mismatched content. It must not fall back to the sender root or copy files
automatically. A matching hash does not authorize executing the artifact.

The store defaults to `<resolved Crew state directory>/inbox/<socket-name>.json`.
For example, `crew-eda.sock` uses `inbox/crew-eda.sock.json`. Set
`CREW_INBOX_STORE` to an absolute file path for isolated tests or manual
configuration. Invalid socket filename components require that override.
`CREW_INBOX_STORE`, `CREW_STATE`, and `CREW_SOCKET` are read by the daemon.
Set them in the environment that starts the daemon; changing a later CLI process's
store or state variables does not relocate an already running daemon's inbox.
Transactions use a sidecar file lock and atomic replacement. The store keeps up
to 10,000 messages without automatic pruning; new messages then return
`store_full`, while retries, reads, lists and acknowledgments remain available.
Summary and ack note limits are 512 UTF-8 bytes; keys use 1–128 ASCII bytes from
`A-Z a-z 0-9 . _ : -`; each message permits at most 16 artifacts.
Artifact paths, repository roots, and reply IDs are limited to 1024 UTF-8 bytes each.
Waiting for a held store lock stops after five seconds with `timeout`; retry the
request with the same key. Staging files left by a crash are retained; filenames
alone cannot prove ownership when store paths can be configured freely.

The JSON-lines bridge exposes `peer.list`, `msg.send`, `msg.list`, `msg.read`, and
`msg.ack` with the usual `params.args` array and inherited caller pane. Saved
changes publish `message.stored` or `message.acked` events, with null pane IDs and
only message/peer/stage/result identifiers. Retries emit no event. The bounded
event journal is not durable; recover using message lists after restart.

Notification state and provider hook delivery are separate from acknowledgments.
These commands do not invoke a model or type into its composer. Provider adapters
must explicitly decide how and when to notify and resume work.
