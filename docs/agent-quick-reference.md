# Agent quick reference

Compact reference for scripts and agents that drive Crew from the shell or over the
JSON-lines bridge. The human guide is [README.md](../README.md); pane orchestration
for agents is in [skills/crew/SKILL.md](../skills/crew/SKILL.md).

## 1. Output contract

- Success: stdout `{"result": ...}`, exit 0. Text replies (`pane read`, `agent read`) print raw text.
- Failure: stderr `{"error":{"code","message"}}`, exit 1. Codes are stable; messages are for people.
  `crew api schema` lists every code with a one-line meaning under `error_codes`.
- Usage error (unknown subcommand or option, bad argument count): exit 2, plain `crew: ...` text.
  Exception: `peer` and `msg` usage errors print `{"error":{"code":"invalid_argument",...}}`
  on stderr, still with exit 2.
- Bridge errors come back inside the reply object (section 3), not as exit codes.

## 2. Targeting

- Pane ID: `p<N>` (tmux-style `%<N>` is also accepted). IDs are stable while one server
  runs; run `pane list` again after a restart.
- `--current` targets the calling pane (`$CREW_PANE_ID`): `pane current`, `pane split`,
  `pane focus --direction` and `pane zoom`. Outside a pane it is a usage error.
- Only `pane current`, `pane split`, `pane focus --direction`, `pane zoom` and `pane layout`
  fall back to a default pane when no ID is given. Every other pane command
  (`get`, `read`, `send-text`, `send-keys`, `run`, `submit`, `wait-output`, `close`, ...)
  requires an explicit `p<N>`.
- `agent` commands accept a pane ID or a unique agent name. Names match
  `[a-z][a-z0-9_-]{0,31}`, cannot look like a pane ID (such as `p12`), and are set with
  `agent rename` on a pane that already has an agent report.
- `pane` commands take pane IDs only.

## 3. Bridge: `crew api serve`

Newline-delimited JSON on stdin and stdout, one request per line. Replies can arrive out
of order, so match them by `id`.

```text
request : {"id": <string or integer>, "method": "group.sub", "params": {"args": ["CLI argument", "..."]}}
reply   : {"id": ..., "result": ...}   or   {"id": ..., "error": {"code": "...", "message": "..."}}
```

```text
-> {"id":1,"method":"ping"}
<- {"id":1,"result":{"bridge":true,"version":"0.6.0"}}
-> {"id":"s","method":"api.schema"}
<- {"id":"s","result":{"protocol":"crew-json-lines-v1","methods":["api.snapshot", "..."],"local_methods":["ping","api.schema"],...}}
```

- `ping` and `api.schema` are local and take no arguments. Any other method must be listed
  by `api.schema`, or it fails with `method_not_found`. The bridge runs the CLI with the
  argument array `[group, sub, ...params.args]`, so the rules in sections 2 and 5 apply.
- `session.snapshot` is an alias of `api.snapshot`.
- Unknown request fields are rejected with `invalid_request`. `id` must be a string or an integer.
  Every envelope failure (`invalid_request`, `request_too_large`) is replied with `"id": null`,
  so treat a null-id error as "one of my unanswered requests was rejected". Only `busy` keeps the id.
- Limits: 1 MiB per line (`request_too_large`) and 16 requests in flight (`busy`).
- A wait holds its in-flight slot until it ends, so always pass a timeout (section 5).

## 4. Methods

Each row is one method from `api.schema`. Bridge method = the command with a dot
between group and sub (`pane read` is `pane.read`).

| Command | Purpose |
|---|---|
| `api snapshot` | Full JSON snapshot of sessions, windows, panes and agents |
| `session snapshot` | Bridge-only alias of `api snapshot` |
| `session list` | List sessions |
| `server stop` | Save the layout, then stop the server |
| `workspace list` | List workspaces |
| `workspace get` | Show one workspace by ID or name |
| `workspace current` | Show the current workspace |
| `workspace create` | Create a workspace (starts the server if needed) |
| `workspace rename` | Rename a workspace |
| `workspace move` | Move a workspace to an index |
| `workspace report-metadata` | Set source-tagged metadata tokens on a workspace |
| `workspace focus` | Focus a workspace |
| `workspace close` | Close a workspace |
| `tab list` | List tabs |
| `tab get` | Show one tab |
| `tab current` | Show the current tab |
| `tab create` | Create a tab in a workspace |
| `tab rename` | Rename a tab |
| `tab move` | Move a tab to an index or to another workspace |
| `tab focus` | Focus a tab |
| `tab close` | Close a tab |
| `worktree list` | List Git worktrees of a repository |
| `worktree create` | Create a Git worktree and open it as a workspace |
| `worktree open` | Open an existing worktree as a workspace |
| `worktree remove` | Remove a clean worktree (the branch is kept) |
| `pane list` | List panes, optionally for one session |
| `pane get` | Show one pane |
| `pane resume` | Submit a restored pane's saved resume command once |
| `pane current` | Resolve the pane for `--current` or the caller |
| `pane split` | Split a pane right or down |
| `pane swap` | Swap two panes in the same tab, keeping processes and focus |
| `pane focus` | Focus a pane by ID or by direction |
| `pane zoom` | Zoom, unzoom or toggle a pane |
| `pane layout` | Apply one of five layout presets |
| `pane auto-focus` | Opt a pane in or out of auto-focus |
| `pane rename` | Set or clear a pane label |
| `pane close` | Close a pane |
| `pane read` | Read the visible or recent text of a pane |
| `pane send-text` | Type text without pressing Enter |
| `pane send-keys` | Send named keys such as `enter`, `ctrl+c` or `up` |
| `pane run` | Type a command and press Enter |
| `pane submit` | Paste text, send one Enter, and report `submitted` or `unconfirmed` |
| `pane wait-output` | Wait for a literal or regex match in pane output |
| `pane report-agent` | Report an agent's `idle`, `working`, `blocked` or `unknown` state |
| `pane report-metadata` | Set source-tagged metadata tokens on a pane |
| `pane release-agent` | Release a previously reported agent state |
| `agent list` | List agents and their status |
| `agent get` | Show one agent's status and details |
| `agent read` | Read an agent pane's text |
| `agent send-keys` | Send keys to an agent's pane |
| `agent prompt` | Send a prompt, optionally waiting for a state |
| `agent wait` | Wait for an agent to reach a state |
| `agent rename` | Set or clear an agent's unique name |
| `agent focus` | Focus an agent's pane |
| `events list` | Read the event journal after a cursor |
| `events wait` | Like `events list`, but waits for new events |
| `peer list` | List inbox peers |
| `msg send` | Send a durable message to a peer |
| `msg list` | List messages for a recipient (`--open` or `--unread`) |
| `msg read` | Read one message |
| `msg ack` | Mark a message read or done |

Events: `events list --after N` returns `{"epoch","events":[{"cursor","timestamp_ms","kind","pane_id","data"}],"cursor","latest_cursor","events_lost"}`.
Pass `cursor` as the next `--after` (unchanged when nothing new arrived) and `epoch` as
`--epoch E`. `--limit` defaults to 100 (1..512), so loop until `cursor` reaches `latest_cursor`.
The journal keeps 512 events; `events_lost: true` means events after your cursor were
dropped before you read them: resync with `api snapshot`. With `--epoch`, a restarted
server fails with `server_restarted`; a cursor ahead of the server fails with
`invalid_cursor`. Either way take `api snapshot` and restart from cursor 0.

## 5. Waits and timeouts

- A timed-out wait fails with code `timeout`. Pane and agent waits poll every 100 ms;
  `events wait` polls every 25 ms.
- `pane wait-output`, `agent wait` and `agent prompt --wait` wait indefinitely without
  `--timeout MS`; scripts should always pass one.
- `events wait` defaults to 30000 ms (maximum 600000, else usage error). `events list` never waits.
- `agent wait` and `agent prompt --wait` default to `--until idle --until done --until blocked`
  when no `--until` is given. `idle` also matches a `done` agent. `--until` and
  `--timeout` on `agent prompt` need `--wait`.
- `agent prompt --wait` fails with `agent_prompt_stalled` if the agent neither starts working
  nor blocks within 5000 ms; with a shorter caller timeout it fails with `timeout` instead.
- `agent prompt` on a blocked agent fails with `agent_blocked` and sends nothing.
- A timeout between paste and Enter leaves the text in the agent's input and sends no
  Enter. Read the pane before retrying so the prompt is not duplicated.
- `pane submit --settle-ms` takes 0 to 10000 ms (default 2000). It sends one Enter and
  never retries.

## 6. Where to read next

- [docs/inbox.md](../docs/inbox.md): durable peer messages (`peer list`, `msg`).
- [docs/automation.md](../docs/automation.md): automation recipes, snapshots and layout details.
- [skills/crew/SKILL.md](../skills/crew/SKILL.md): pane orchestration for agents.
- [README.md](../README.md#自動化-api-automation-api-herdr-style): the full automation API section.
