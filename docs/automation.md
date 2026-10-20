# Crew automation

## Workspace and Git isolation

```sh
crew workspace create backend --cwd /path/to/project --env ROLE=backend
crew tab create tests --workspace backend --command "cargo test"
crew worktree create --repo /path/to/project --path /path/to/project-review --branch review
crew workspace list
crew tab list --workspace backend
crew pane list
```

Each create returns JSON containing the allocated ID and pane. Creation defaults to no-focus.
Use the returned IDs instead of guessing numbers. `--command` intentionally executes text in
the configured shell; ordinary arguments are never reparsed as a shell command.
Workspace and tab names are exact; `name:NAME` disambiguates legacy ID-shaped names.

Close the associated workspace before removing its checkout. Remove refuses modified,
untracked and ignored files and does not delete the branch. A failed terminal launch preserves
the created worktree and reports its path. Check `git status` and preserve desired files before
retrying; there is no force removal option.

## Explicit agent lifecycle

Workspace and tab ordering is available through `workspace move TARGET --index N`
and `tab move TARGET --index N [--workspace TARGET]`. `N` is a slot in the original
ordered list, from zero through its length. The operation preserves IDs and live
processes; it does not transfer a tab to another workspace. See
[ordering](workspace-move.md) for examples, focus and event semantics.

Display-only progress is separate from lifecycle reports:

```sh
crew pane report-metadata p1 --source task:hook --token "task=review changes" --seq 1 --ttl-ms 60000
crew workspace report-metadata w1 --source build:ci --token build=running --seq 1
```

These bounded patches appear in `tokens` maps and metadata update/expiry events.
They do not complete agent waits or change agent state. See [metadata](metadata.md)
for expiry, clearing, source sequencing and restart behavior.

Inside a Crew pane, a hook can use:

```sh
crew integration report claude start --current --source my-hook --seq 1
crew integration report claude blocked --current --source my-hook --seq 2 --message "Review requested"
crew integration report claude idle --current --source my-hook --seq 3
crew integration report claude exit --current --source my-hook --seq 4
```

Use `crew integration --help` for the complete contract. Events are generic and must be wired
to the provider's actual supported lifecycle. These examples do not install provider hooks.
Doctor checks PATH files; explain shows explicit-report versus output-activity evidence.

`agent wait` and `agent prompt` require a reported agent. They pin its instance: replacement
or release fails the old wait. Prompt checks the same identity while pasting and again before
Enter. If the occupant changes or becomes blocked between these steps, Enter is cancelled.
Text already pasted cannot be recalled. Use one prompt producer per agent.

## JSON lines

Launch `crew api serve` as a subprocess. Write one JSON object per line to stdin, and read
one JSON object per line from stdout. Do not assume response order; correlate by `id`.
All live operations use Crew's local daemon; `ping` only verifies the bridge itself.

```json
{"id":"create","method":"workspace.create","params":{"args":["worker","--cwd","/path/to/repo"]}}
{"id":"list","method":"agent.list"}
{"id":"read","method":"pane.read","params":{"args":["p1","--source","recent","--lines","80"]}}
```

Responses have either `result` or `error: {code,message}`. Text reads return a JSON string.
Unknown methods/params return explicit errors. `crew api schema` lists all supported methods.
The bridge permits 16 concurrent in-flight requests and a maximum 1 MiB input line. Further
requests return `busy`; oversized input returns `request_too_large`. EOF waits for pending
requests, so always set a finite `--timeout` on wait operations. Terminating the bridge does
not stop the daemon. This bridge is not a drop-in replacement for Herdr's socket protocol.

## Events and recovery

```sh
crew events list --limit 100
crew events wait --after 42 --epoch SERVER_EPOCH --timeout 30000
```

Carry forward the returned `cursor`, not `latest_cursor`, while paginating. Each server run
has a new `epoch`. `events_lost: true` means the 512-entry journal overran the caller; obtain
a fresh `api snapshot` and resume from the current event position. `server_restarted` requires
a fresh snapshot and epoch. Events are in memory and do not survive restart. `pane.output`
contains a byte count, not terminal text. Workspace/tab/pane lifecycle and accepted agent
reports are recorded; this is not a log of every keystroke or every UI state change.

## Running tests safely

The smoke tests clear inherited Crew/tmuxlike environment variables, then use their own
socket and temporary config/state/Git paths. They do not connect to the caller's pane server.
Run `cargo build --bins`, then pass `target/debug/crew` (Windows: `crew.exe`) to each script.
`tmuxlike` remains a compatibility alias and supports the same commands.
