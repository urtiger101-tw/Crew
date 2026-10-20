# Workspace ordering and live tab transfer

Crew can reorder workspaces and tabs, and transfer a live tab to another workspace.
Reordering within one list preserves the selected workspace, tab and pane. Both
operations preserve stable tab/pane IDs, running processes, shell state, agent
occupants, pane metadata, and the moved tab's active pane and zoom state.

```sh
crew workspace move w3 --index 0
crew workspace move 'project.alpha:中文' --index 3
crew tab move t7 --index 0
crew tab move 'review.second:中文' --index 3 --workspace w2
crew tab move t7 --to-workspace w3
crew tab move 'review.second:中文' --workspace w2 --to-workspace w3 --index 0
```

Targets use the existing exact-name and stable-ID rules. `--workspace` scopes a
tab lookup; it does not identify a destination workspace. Without that option, a
`tN` ID resolves globally, while a name resolves in the caller pane's workspace,
or the most recently used workspace when there is no caller pane. Names with
periods and colons are matched in full. `name:NAME` still disambiguates legacy
names that resemble an ID.

## Transfer a tab between workspaces

`--to-workspace ID|NAME` selects an existing destination workspace. `--workspace`
continues to constrain **source lookup**. An omitted `--index` appends to the
destination; an explicit index is a slot from zero through its current tab count.
The daemon moves the existing tab object under one state lock. It does not copy
or restart its shells, allocate new pane/tab IDs, release the agent occupant, or
cancel a pending agent wait. All panes in a split layout move together.

The destination keeps its previously selected tab. The source also keeps its
selected tab if that tab remains. If the moved tab was selected, the source
selects the following tab, or the preceding tab when the moved one was last.
Use `crew tab focus tN` afterward if you want to follow the moved tab.

Moving the source's **last tab closes the now-empty source workspace**. This
does not close the moved tab or kill any of its panes. Clients attached to that
source switch to the destination, whose selected tab stays unchanged. Other
clients remain attached to their workspaces. When the removed source was the
globally focused workspace, the destination becomes the focused workspace;
otherwise the previously focused workspace stays focused. Source workspace
metadata is removed; pane metadata follows the stable pane IDs and retains its
TTL and sequence high-water marks. Destination workspace metadata stays intact.

The transferred layout is resized for the destination's terminal dimensions.
Open chooser rows and pending tab-close confirmations follow the same stable
tab/pane IDs. A chooser row for a workspace that was removed closes the chooser.
In-progress drags in affected workspaces are cleared because their old geometry
no longer identifies the same boundary.

Caller context uses `CREW_PANE_ID` (or its legacy alias) to resolve current
membership each time a request reaches the daemon. A command run by a moved
pane therefore sees the new workspace even though that process's environment
was created earlier. Existing processes retain their original working directory
and environment; a transfer cannot rewrite a running shell's environment.
Splits of those panes inherit the original pane environment. New tabs created
in the destination inherit the destination workspace's environment instead.

Validation completes before any move: missing destinations, mismatched source
scope, invalid insertion slots, and conflicting **manually assigned** tab names
are rejected without changing focus, membership or events. Automatically named
shell tabs may share a name, just as they can on creation; use a `tN` ID when a
name is ambiguous. Transferring to the current workspace uses the existing
ordering/no-op rules below, including append when no index is supplied.

This is a Crew operation using Crew's stable IDs and protocol. It does not claim
that Herdr's `tab.move` wire method supports the same cross-workspace request.

## Insertion slots

`--index N` is a zero-based **insertion slot in the original list**. For a list
with `count` items, the permitted range is `0..count`, inclusive. Slot `0` places
the item first and slot `count` places it last. If the source occurs before the
requested slot, its removal reduces the final index by one.

For the original order `[A, B, C]`:

| Command | Result | Changed |
|---|---|---|
| Move B to slot 0 | `[B, A, C]` | Yes |
| Move B to slot 1 | `[A, B, C]` | No |
| Move B to slot 2 | `[A, B, C]` | No |
| Move B to slot 3 | `[A, C, B]` | Yes |
| Move C to slot 1 | `[A, C, B]` | Yes |

The slots immediately before and after an item are both no-ops. In a one-item
list, slots `0` and `1` are both valid no-ops. Negative numbers, non-integers,
missing or repeated `--index`, additional arguments and unknown options are
rejected by the CLI. The server also rejects an insertion slot beyond the
current list length with `invalid_argument`. Validation completes before any
ordering, focus, UI selection, persistence or event state changes.

This behavior follows the `insert_index` semantics of Herdr 0.9.5 at
[`143bf5fd4781d164260c63fe28e7dc08054ad32f`](https://github.com/herdrdev/herdr/tree/143bf5fd4781d164260c63fe28e7dc08054ad32f),
specifically its workspace and tab move APIs. The Crew CLI commands shown here
are Crew commands; that Herdr revision does not expose identical CLI commands.
Crew retains its own API transport and ID scheme.

## Replies and events

The workspace reply contains `workspace_id`, `insert_index`, `from_index`,
`to_index`, `moved`, and the complete new `workspaces` array. The tab reply
contains `workspace_id`, `tab_id`, the same index and change fields, and the
complete new `tabs` array for that workspace. `from_index` is the old item
position; `to_index` is its final position after removal and insertion.

Only an actual order change emits `workspace.moved` or `tab.moved`. Event data
contains the resource IDs, `insert_index`, `from_index`, `to_index`, and the
authoritative new `workspace_ids` or `tab_ids` array. Tab events also carry the
parent `workspace_id`. No-op replies contain `moved: false` and emit no move
event. The journal's existing cursor, epoch and lost-event rules still apply.

For example, moving `t2` to the last slot in `[t1, t2, t3]` produces this event
data:

```json
{
  "workspace_id": "w1",
  "tab_id": "t2",
  "insert_index": 3,
  "from_index": 1,
  "to_index": 2,
  "tab_ids": ["t1", "t3", "t2"]
}
```

Raw typed API requests use the existing `Workspace` envelope:

```json
{"Workspace":{"WorkspaceMove":{"workspace":"w3","insert_index":0}}}
```

```json
{"Workspace":{"TabMove":{"tab":"t7","workspace":"w2","insert_index":0,"current":null}}}
```

A cross-workspace move uses a separate request so existing `TabMove` requests
remain compatible:

```json
{"Workspace":{"TabTransfer":{"tab":"t7","workspace":"w2","to_workspace":"w3","insert_index":0,"current":null}}}
```

`insert_index: null` appends. A cross-workspace response and its `tab.moved`
event add `from_workspace_id`, `to_workspace_id`, `source_workspace_closed`,
`source_tab_ids` and destination `tab_ids`; `workspace_id` is the destination.
The response additionally contains destination `tabs` and remaining
`source_tabs` arrays. An empty source emits `workspace.closed` **after**
`tab.moved`, with `reason: "last_tab_transferred"` and `to_workspace_id`.
There are no `pane.closed`, `tab.closed`, `pane.created` or `tab.created`
events for a transfer. Consumers should reread authoritative state after the
events, particularly when an empty source disappears.

Each move is applied under the server state lock, including selection repair
and its event. Concurrent requests are serialized. An insertion slot refers to
the order when the server applies that request; there is no expected-version
argument. Consumers that need a specific relative order should inspect the
returned ordered list before issuing their next move.

## Identity and persistence

Ordering repositions existing workspace/tab objects. It does not create,
stop, replace or resize a PTY. Transfer may resize a PTY for the destination,
but retains its process. Reported agent instances remain unchanged, so a
pending `agent wait` continues to observe the same occupant. Existing client
attachments remain on the same workspace during ordinary reordering. The empty
source exception for transfers is described above. Open chooser selections and pending
tab-close confirmations continue to refer to the same resource after its display
position changes. Legacy window indices intentionally reflect the new order.

The existing version 1 snapshot already stores ordered workspace/tab arrays,
stable IDs and active positions. Normal autosave and `server stop` save the new
order and transferred membership without a schema migration. A cold restart
restores that order, stable workspace/tab IDs and the selected tabs; closed empty
sources do not reappear. Restart still launches new processes;
pane IDs and agent occupant identity are only preserved by a move during the
same server lifetime.

## Verification

```sh
cargo test --locked workspace::
python scripts/workspace_move_smoke.py target/release/crew
python scripts/workspace_transfer_smoke.py target/release/crew
python scripts/workspace_smoke.py target/release/crew
```

On Windows, pass `target/release/crew.exe`. The move smoke uses a unique endpoint
and temporary configuration/state, waits for the PowerShell prompt before its
first command, checks a live shell variable after ordering, holds an agent wait
across moves, checks event order and invalid requests, and performs a cold
restart. It does not contact an existing user server. Unit tests cover every
insertion slot in a three-item list, recency ties, singleton no-ops, exact caller
scope, selection repair and pending close targets without spawning a PTY.

The transfer smoke additionally compares actual shell process IDs and shell
variables before and after several moves, checks original cwd/environment and
new destination-tab inheritance, holds an agent wait across the transfer, checks
metadata sequence rejection, moves the final tab out of a workspace, and performs
a cold restart. Tests execute on the local platform; running the script on Linux
does not count as native Windows or macOS validation.
