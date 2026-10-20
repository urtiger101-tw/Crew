# Metadata display and templates

Crew accepts ephemeral presentation metadata for panes and workspaces. The same
computed display appears in API snapshots and the TUI. Presentation changes labels
only: it does not rename stable IDs, replace an agent instance, set its semantic
state, or complete `agent wait`.

## Report a presentation

```sh
crew pane report-metadata p1 --source ui:build --seq 1 \
  --title 'Review login' --display-agent 'Reviewer' \
  --state-label working='Reviewing' --state-label blocked='Needs your input' \
  --token task='login' --ttl-ms 30000

crew workspace report-metadata w1 --source ui:project --seq 1 \
  --title 'Crew release' --token branch='release'
```

The source ID is required. Give independent publishers different source IDs.
Workspace presentation accepts `title`; agent labels and state labels belong to
panes. Repeat token or state-label flags for distinct keys.

| Option | Meaning |
| --- | --- |
| `--title TEXT` | Pane title or workspace display title |
| `--display-agent TEXT` | Pane agent label; does not change `agent.agent` or its name |
| `--state-label STATE=TEXT` | Label for `idle`, `working`, `blocked`, `done`, or `unknown` |
| `--agent KIND` | Apply this source's presentation while the effective agent kind matches |
| `--applies-to-source ID` | Apply while an explicit agent report has this source ID |
| `--clear-title` / `--clear-display-agent` | Clear that field from this publisher |
| `--clear-state-label STATE` | Clear that state's label from this publisher |
| `--clear-presentation` | Remove this publisher's presentation and guards; retain tokens |

Omitted fields remain unchanged within a source. Empty or whitespace-only values
clear a field. Native JSON request strings have the same semantics: explicit
`null` clears a text field, while an omitted field preserves it. Display text is
trimmed, stripped of control and bidirectional formatting characters, and limited
to 80 Unicode characters. This normalization prevents terminal escape injection.

The latest changed eligible source wins each overlapping presentation field.
Fields absent from that source can fall back to another eligible source. Clearing
or expiring a source can therefore reveal an older publisher's title. An identical
report without a TTL does not reorder publishers. Guards hide only presentation;
independent tokens remain visible. Guards persist when omitted; clear the source
presentation before reporting new unguarded fields.
Guard-only reports are retained, and clearing the last visible field preserves
its guards. Such entries consume the same bounded source capacity and expire by
the same TTL rules as entries with visible fields.

Explicitly reported kind takes priority over a process-detected kind for guard
matching. A process hint has no explicit source, so it cannot satisfy an
`--applies-to-source` guard. A label such as `working=Finished` is still the
`working` state; status colors, counts and waits use the actual state.

## Expiry and ordering

`--ttl-ms` accepts 1 through 86,400,000 milliseconds. A presentation patch updates
the expiry of that source's **whole presentation**, including its unchanged
fields. Omitting TTL in a presentation patch removes that source's expiry.
Token TTL remains per updated token, as described in [metadata.md](metadata.md).
A token-only report never extends or removes the source presentation's TTL.

`--seq` is a nonnegative integer with one high-water mark per resource and source,
shared by tokens and presentation. A report at or below the last accepted number
is rejected without changing data or emitting a change event. The high-water mark
survives clearing and expiry, so stale reports cannot resurrect expired labels.
Reports without a sequence do not claim ordering. Validation and capacity
rejections are atomic and do not consume a sequence number.

Expiry runs on the daemon timer even without terminal output; reads filter expired
values immediately. Each resource supports at most 32 active presentation sources
and 32 retained sequenced sources. Closing the resource or restarting the daemon
discards all metadata, guards, expiry and sequence state. Saved layouts do not
restore these transient values.

## Configure literal templates

Add these optional strings to the Crew configuration file:

```toml
pane_template = "$title · $task"
sidebar_pane_template = "$agent $task"
sidebar_workspace_template = "$title [$branch]"
```

They are also configurable in a running server:

```sh
crew set pane-template '$title · $task'
crew set sidebar-pane-template '$agent $task'
crew set sidebar-workspace-template '$title [$branch]'
```

`$name` and `${name}` perform one literal substitution. `$$` produces a dollar
sign. Unknown tokens produce an empty string; substituted values are never
interpreted again. `$(command)`, shell code and markup have no execution semantics.
Names contain ASCII letters, digits, `_` or `-`; use `${task}-suffix` when a
following hyphen should be literal text rather than part of the token name.
Templates accept at most 512 Unicode characters and reject control or bidi
formatting characters. Output is capped at 512 characters before existing terminal
column clipping, which handles wide characters.

| Built-in | Pane template | Workspace template |
| --- | --- | --- |
| `$title` | Presentation title, otherwise agent/pane/tab name | Presentation title, otherwise workspace name |
| `$pane` | Same input as `$title` | — |
| `$agent` | Presentation label, explicit name/kind, or detected kind | — |
| `$workspace` | Actual workspace name | Actual workspace name |
| `$tab` | Actual tab name | — |
| `$pane_id`, `$tab_id` | Stable resource IDs | — |
| `$workspace_id` | Stable workspace ID | Stable workspace ID |
| `$state` | Actual machine-readable status | — |
| `$state_text` | Presentation state label or localized status | — |
| Other names | This pane's metadata tokens | This workspace's metadata tokens |

Built-ins take precedence over identically named tokens. Pane and workspace token
namespaces are separate. Tokens never replace actual resource names or state.
The input `$title` is not the rendered template output. Empty configuration
preserves normal display; an entirely empty rendered result also falls back to a
normal name. Literal punctuation remains even if all substitutions are empty.

`pane_template` applies to pane title rows. Existing `pane_titles` and layout rules
still determine whether a title row fits. A split pane with title metadata or
tokens used by a configured pane template has display information for `auto` mode.
The sidebar templates apply to the corresponding rows when the sidebar is
enabled. The status bar's workspace/tab names remain stable navigation labels.

## API, events and verification

`pane get/list`, `agent get/list`, workspace/tab responses and `api snapshot`
include a `display` object. Pane display contains `title`, `sidebar`, `agent`,
`state_label`, `kind_source`, and the eligible raw `presentation`. Workspace display
contains `title`, `sidebar`, and `presentation`. Machine snapshots use English for
the default `state_label`; the TUI follows the client's language. User-supplied
labels are literal in both.

Existing metadata update events include `presentation_changed` alongside token
`changed_keys`. Source changes hidden by a guard can still emit an update; the
event contains the current computed display. Expiration has `reason: "expired"`.
The JSON-lines bridge accepts the same options through `pane.report-metadata` and
`workspace.report-metadata` argument arrays.

```sh
python scripts/presentation_smoke.py /path/to/native/crew
```

The smoke checks a real rendered terminal frame, API consistency, guards, clears,
sequence rejection, TTL, explicit wait identity, JSON-lines access and restart
cleanup. It requires local PTY/socket permission and launches no vendor agent.
The string template syntax is Crew's implementation; it does not implement
Herdr's full styled row/grid template configuration.
