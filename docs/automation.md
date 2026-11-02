# Crew automation

For named agents exchanging persistent handoffs, see [durable peer messages](inbox.md).

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

On Windows, a configured `powershell(.exe)` or `pwsh(.exe)` executes a workspace/tab
startup command through its native `-NoExit -Command` arguments, after shell
initialization. Quotes, Unicode and multiline text remain one command argument.
The shell stays available for subsequent input. Configurations already selecting
a command, script or server execution mode are rejected before creating the pane
when `--command` is supplied. Other shells retain their existing input submission.
Startup commands are one-time launch options; splits and cold restore do not
replay them. Wait for the application's actual readiness before sending more input.

Close the associated workspace before removing its checkout. Remove refuses modified,
untracked and ignored files and does not delete the branch. A failed terminal launch preserves
the created worktree and reports its path. Check `git status` and preserve desired files before
retrying; there is no force removal option.

## Layout selection and snapshots

```sh
crew pane layout even-horizontal --session main --window 0
crew pane layout tiled
```

The five names are `even-horizontal` (equal side-by-side columns), `even-vertical`
(equal rows), `main-horizontal` (first pane above a row of other panes),
`main-vertical` (first pane left of a column of other panes), and `tiled` (near-square
rows, longest rows first). Main layouts give the first pane a 60% share. Existing
leaf order is retained. `--window` is a zero-based index and `--session` an exact
session name. With no explicit target, the caller's `CREW_PANE_ID` selects its
window; otherwise the most recently used session's current window is selected.
An explicit session overrides the caller pane without moving focus.

The JSON reply contains `session`, `window`, `layout`, `changed`, `active_pane`,
`zoomed` and `panes`. The JSON-lines bridge accepts method `pane.layout` with
the same CLI arguments in `params.args`. A one-pane window returns `changed:false`
without changing zoom or cycle state. Unknown names fail as usage errors before
connecting. Multi-pane selection unzooms once, then runs the existing PTY resize
path at most once per affected pane. No pane is respawned and no input is sent.
Pane identity, root PID, active pane and the parser/history stay owned by the
same objects. Tight dimensions may omit separators so six rows still have
nonzero pane bodies in a 40x10 pane area; dimensions too small even for the pane
cells cannot show every pane simultaneously.

`api snapshot` keeps its existing `layout` binary tree (`pane` leaves;
`split:"right"|"down"`, `ratio`, `a`, `b` branches). Windows additionally expose
`layout_name`, the last selected preset or null. `select-layout NAME` and prefix
M-1..M-5 choose presets in the order above; prefix Space / `next-layout` advances
in that order and wraps. New windows start with no selected preset, so their first
cycle chooses `even-horizontal`. Custom bindings still take precedence.

State snapshots remain version 1. Their optional `layout_preset` field preserves
the cycle position; old snapshots omit it and remain readable. Trees use balanced
strips and retain the existing depth limit of 64. Cold restore rebuilds shells as
before: persistence does not preserve processes across a server restart. Live
relayout does preserve them. Installing a new binary does not update an already
running daemon; restart remains an explicitly authorized operation.

## Explicit agent lifecycle

### Optional automatic focus

Auto-focus defaults to off. Configure `auto_focus = "off"|"blocked"|"on"`,
`auto_focus_scope = "window"|"session"|"all"` (default session),
`auto_focus_idle_ms = 2500`, and `auto_focus_dwell_ms = 4000`.
The corresponding `set auto-focus`, `set auto-focus-scope`,
`set auto-focus-idle-ms`, and `set auto-focus-dwell-ms` options take effect at
runtime. Config files also accept the hyphenated spellings. No owner config is
changed automatically. `prefix F` toggles off/on; `prefix a` invokes manual
next-attention, `prefix ;` / `last-pane` returns to the previous pane, including
across sessions. `prefix b` retains the sidebar binding in both themes. Custom
bindings override these defaults.

Only transitions into the existing effective Blocked or Done state enqueue work.
Mode blocked accepts Blocked only; mode on accepts both. There are no new status
heuristics. Repeated levels, enabling the mode, opting a pane back in, or attaching
after a headless event do not replay old edges. Each queued item pins an occupant
and generation; changes of status/occupant, closure, opt-out or scope remove stale
items. Priority is Blocked, then same-window preference, then oldest edge. A
queued notice reuses attention text and shows the `prefix a` manual action.

Keyboard presses/repeats, paste and mouse activity reset the client's idle guard;
resize and key-release events do not. Copy mode, prefix mode, chooser, popup,
prompt/confirmation and active drag gestures defer automatic jumps. The dwell
clock starts after a successful automatic jump. A focused target is ignored.
Jumps switch focus/session and unzoom the target window; no keys or text are
sent, no process is respawned, and scrollback remains owned by its pane. Manual
next-attention/last-pane are explicit requests and bypass automatic guards.

Crew's current pane/window focus is shared by a session, not an independent
viewport per client. Decisions and queues are per client, but a proposed jump
also checks typing, dwell and view-mode guards on **every other attached client
whose shared view would change**. Otherwise the proposal waits. A jump updates
the affected clients' dwell/prior-pane state together. A client in an unrelated
session is not disturbed. Without clients the scheduler changes no focus and
stores no pending jumps.

`crew pane auto-focus p2 off` opts a noisy pane out; `on` opts it back in. The
setting appears as `auto_focus` in pane JSON and persists as an optional
`auto_focus_disabled` field in version-1 leaf snapshots; absent means enabled.
The bridge method is `pane.auto-focus` with the same CLI arguments. Runtime
queues, clocks and prior-pane pointers are not persisted. Status bars in both
studio and Aurora show AF / AF:blocked (自焦 / 自焦:待回應 in zh-TW). Existing
status/event API semantics remain unchanged; no new protocol events are added.

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

The smoke tests clear inherited `CREW*` environment variables, then use their own
socket and temporary config/state/Git paths. They do not connect to the caller's pane server.
Run `cargo build --bins`, then pass `target/debug/crew` (Windows: `crew.exe`) to each script.
