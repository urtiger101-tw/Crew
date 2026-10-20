# Workspace and tab ordering

Crew 0.4.0 can reorder workspaces and reorder tabs within their current workspace.
Moving an item preserves its stable ID, running panes, shell state, agent occupant,
active tab and pane, zoom state, and focus. A tab stays in its original workspace.

```sh
crew workspace move w3 --index 0
crew workspace move 'project.alpha:中文' --index 3
crew tab move t7 --index 0
crew tab move 'review.second:中文' --index 3 --workspace w2
```

Targets use the existing exact-name and stable-ID rules. `--workspace` scopes a
tab lookup; it does not identify a destination workspace. Without that option, a
`tN` ID resolves globally, while a name resolves in the caller pane's workspace,
or the most recently used workspace when there is no caller pane. Names with
periods and colons are matched in full. `name:NAME` still disambiguates legacy
names that resemble an ID.

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

Each move is applied under the server state lock, including selection repair
and its event. Concurrent requests are serialized. An insertion slot refers to
the order when the server applies that request; there is no expected-version
argument. Consumers that need a specific relative order should inspect the
returned ordered list before issuing their next move.

## Identity and persistence

The operation repositions existing workspace/tab objects. It does not create,
stop, replace or resize a PTY. Reported agent instances remain unchanged, so a
pending `agent wait` continues to observe the same occupant. Existing client
attachments remain on the same workspace. Open chooser selections and pending
tab-close confirmations continue to refer to the same resource after its display
position changes. Legacy window indices intentionally reflect the new order.

The existing version 1 snapshot already stores ordered workspace/tab arrays,
stable IDs and active positions. Normal autosave and `server stop` save the new
order without a schema migration. A cold restart restores that order, stable
workspace/tab IDs and the selected tabs. Restart still launches new processes;
pane IDs and agent occupant identity are only preserved by a move during the
same server lifetime.

## Verification

```sh
cargo test --locked workspace::
python scripts/workspace_move_smoke.py target/release/crew
python scripts/workspace_smoke.py target/release/crew
```

On Windows, pass `target/release/crew.exe`. The move smoke uses a unique endpoint
and temporary configuration/state, waits for the PowerShell prompt before its
first command, checks a live shell variable after ordering, holds an agent wait
across moves, checks event order and invalid requests, and performs a cold
restart. It does not contact an existing user server. Unit tests cover every
insertion slot in a three-item list, recency ties, singleton no-ops, exact caller
scope, selection repair and pending close targets without spawning a PTY.
