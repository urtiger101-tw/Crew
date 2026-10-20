# SSH federation

Crew can save SSH machines, route its API commands to an existing remote daemon,
and show local and remote workspaces and agents together. `crew federation` opens
a terminal selector; Enter attaches to the selected machine with native OpenSSH.
Each server continues to own its workspaces, panes, processes, and agent state.

## Requirements

- Install OpenSSH `ssh` on the client and a Crew build with `crew-ssh-v1` support
  on each remote machine. This protocol first ships with Crew 0.5.0; use the same
  Crew release on both ends.
- Remote execution currently requires a POSIX login shell, `env`, and a Linux or
  macOS Crew installation. A Windows client can use its installed OpenSSH client;
  a native Windows SSH server is not supported by this remote shell protocol.
- Start the remote daemon yourself before adding a machine. For example, log in
  normally and run `crew workspace create work`. Neither discovery nor a routed
  create command installs Crew, starts a daemon, or restarts one.
- Configure authentication, ports, jump hosts and host keys with OpenSSH. Verify
  the host normally before adding it. Read-only probes use batch authentication,
  so an encrypted private key must already be available through an SSH agent.

Crew does not copy credentials or edit SSH configuration. It invokes the `ssh`
found on the client's executable search path, retains OpenSSH host-key checking,
and accepts a saved SSH alias or `user@host`. Put port and identity settings in an
SSH config file instead of placing SSH flags in the destination.

For example, with an existing SSH alias `buildbox`:

```sh
crew machine add buildbox --label Build --binary /opt/crew/bin/crew
crew machine list
crew machine status Build --timeout-ms 5000
crew --machine Build workspace list
crew --machine Build agent list
crew federation
```

An absolute binary path is useful when a noninteractive SSH login has a different
`PATH`. `--binary crew` is the default. Relative paths containing `/`, `~` paths,
and Windows paths are rejected. Spaces and apostrophes in absolute POSIX paths
are supported.

## Saved machines

```text
crew machine add HOST [--label TEXT] [--binary PATH|crew]
                      [--socket NAME] [--ssh-config PATH] [--no-probe]
crew machine list [--json]
crew machine rename ID|LABEL --label TEXT
crew machine disable ID|LABEL
crew machine enable ID|LABEL
crew machine remove ID|LABEL
crew machine status [ID|LABEL] [--timeout-ms N] [--json]
crew machine probe [ID|LABEL] [--timeout-ms N] [--json]
crew machine reconnect ID|LABEL [--timeout-ms N]
```

Add normally verifies both remote workspace and agent API responses before
saving the profile. `--no-probe` explicitly saves a profile without connecting;
use this for an offline machine. Labels default to the SSH destination and must
be unique. Machine IDs such as `m1` are stable across renames and are never reused
after removal. `local` and names matching machine IDs are reserved labels.

`--ssh-config PATH` selects an existing config with OpenSSH's `-F` option. Crew
saves its absolute path. Without it, OpenSSH uses its normal configuration. This
does not import or modify that file. An SSH config can contain `ProxyJump` or
`ProxyCommand`; those are commands the user has chosen to trust. Crew clears
configured forwarding listeners for these command connections.

`--socket NAME` selects a named Crew daemon on the remote host, corresponding to
the remote server's `CREW_SOCKET`. This is a socket name, not a file path. Omit it
for the remote user's normal Crew namespace. It never selects a local socket.

Profiles contain the destination, label, binary path, optional socket/config
path, and enabled state. They are stored in `machines.json` beside the resolved
Crew configuration file, including a custom `CREW_CONFIG` path. Set
`CREW_MACHINES` to use a different catalog. Each catalog supports up to 32 saved
machines. Changes use an exclusive file lock and atomic replacement. Invalid or
unsupported JSON is reported and preserved, rather than replaced with defaults.

Disable only changes future routing and discovery. Remove only deletes the local
profile. Neither operation stops a remote daemon, kills an agent, or closes an
already attached SSH session. Reconnect makes a fresh bounded probe; it does not
restart services or replay a previous command.

## Route API commands to one machine

Place `--machine` before the ordinary API command. Machine-local IDs keep their
ordinary form:

```sh
crew --machine Build workspace list
crew --machine Build workspace rename w1 review
crew --machine Build tab list --workspace w1
crew --machine Build pane read p1
crew --machine Build pane run p1 'cargo test --locked'
crew --machine Build agent list
crew --machine Build --timeout-ms 30000 api snapshot
```

Workspace, tab, pane, agent, worktree, events and `api snapshot` commands use the
same parser and daemon API as local commands. Commands that normally infer the
current pane require an explicit remote ID; a local `CREW_PANE_ID` is never used.
The `--timeout-ms` option belongs immediately after the machine selector. All
timeouts accept 250–300000 ms; the default is 15000 ms.

Successful output retains the ordinary API result shape. JSON uses a `result`
envelope; text replies such as pane contents remain text. A failed route exits 1
and reports a JSON error on stderr. Malformed local machine-management options
are ordinary CLI errors. A remote error never triggers a local retry or a local
daemon start.

Requests cross SSH stdin as a versioned JSON envelope. User API arguments do not
become part of the SSH shell command. A command intentionally passed to
`pane run` still executes in that selected remote pane as its API specifies.
Remote helper paths and terminal-attach workspace names use POSIX quoting.

Timeouts stop the client-side transport. A request already delivered to a remote
daemon may have completed, including a mutation. Crew never retries it
automatically; inspect remote state before deciding whether to repeat it. A
transport request is limited to 1 MiB, stdout to 4 MiB, and stderr to 64 KiB. Large
queries fail explicitly rather than consuming unlimited memory.
On Unix, every completion and error path also cleans the transport's process
group, including a ProxyCommand descendant that retains a pipe after SSH exits.

## Combined terminal selector

```sh
crew federation
crew federation watch --timeout-ms 5000
crew machine attach Build --workspace review
```

With a terminal, `crew federation` opens the selector. It displays a Local group
and each saved machine, connection state, workspace names, and each workspace's
reported agents and status. Healthy machines appear as their independent probes
finish; one unavailable host does not hide another host's workspaces.

| Key | Action |
| --- | --- |
| Up / Down | Select a reachable workspace. |
| Enter | Attach to that workspace on its owning server. |
| `r` | Re-read saved profiles and probe all machines again. |
| `q`, Escape, Ctrl+C | Leave the selector and restore the terminal. |

The selector refreshes every 15 seconds when no probe is pending. A pending probe
does not block keyboard input; pressing `r` during it does not launch duplicate
requests. Offline, disabled and checking rows cannot be attached. If a selected
workspace disappears, Enter reports that it is unavailable until you select
another reachable workspace. It does not switch silently to Local.

Enter temporarily leaves the selector and runs native
`ssh -tt ... crew attach-workspace wN` for a remote workspace, or local
`crew attach-workspace wN` for a local workspace. The daemon resolves that stable
ID while registering the attached client, so a rename or reuse of the old name
cannot redirect input. A removed ID fails; it never selects the newest workspace.
This separate command and protocol message also fail with older Crew versions
that do not support ID attachment. Direct `machine attach --workspace NAME`
retains the explicit name-based behavior. The native
SSH client can ask for authentication or host-key confirmation according to its
configuration. Crew's normal detach key is **Ctrl+B, then d**; detach returns to
the selector and leaves the selected server's processes alive. An attach error
also returns to the selector with a visible failure status. Direct
`machine attach` exits with the SSH process's status instead.

This is a federation selector, not a cross-host PTY transfer or a remote-pane
embedding layer. The local server's selected workspace and layout remain intact
while you attach to a remote server. Tab movement stays within one owning
machine. There is no automatic remote deployment, version upgrade, shared
filesystem, persistent SSH tunnel, or cross-host workspace transaction.

## Combined JSON views and partial failures

```sh
crew federation list --json
crew federation workspaces --timeout-ms 5000 --json
crew federation agents --json
```

Without an interactive terminal, plain `crew federation` defaults to `list`.
Machine commands and explicit federation list commands currently always emit
JSON; `--json` makes script intent explicit. `federation watch` requires a TTY.

The `result` object includes `machines`, `complete`, and the requested merged
`workspaces` and/or `agents` arrays. Every merged resource has `machine_id` and
`machine_label`. Treat **(machine_id, resource_id)** as its identity: `local/w1`
and `m1/w1` are different workspaces.

Each machine has its own `reachable`, `unavailable`, or `disabled` state. A
reachable entry includes its summary in `data`; an unavailable entry includes an
error. Authentication errors, changed host keys, missing Crew binaries, protocol
mismatches, stopped daemons and transport timeouts remain attached to that
machine. Both workspace and agent reads must succeed for its summary to be
reachable.

`complete` is false when any enabled machine is unavailable. Disabled machines
do not make an otherwise complete result incomplete. A federation list exits 0
when at least one machine is reachable, so scripts that require every enabled
machine must also check `complete`. It exits 1 when none are reachable. A machine
status/probe exits 1 if one of the queried enabled machines is unavailable.
Reads run concurrently and do not create local or remote daemons.

## Local validation without GitHub Actions

```sh
cargo test --locked remote::
python3 scripts/ssh_federation_smoke.py /absolute/path/to/crew --logs /tmp/crew-ssh-evidence
```

The smoke test requires Linux, Python 3, OpenSSH `ssh`/`sshd`/`ssh-keygen`, PTYs,
the distribution's standard `/run/sshd` directory, and permission to listen on
loopback. It creates temporary host and user keys, a pinned known-host entry,
an isolated SSH config, two actual Crew daemons and an offline profile. It binds
only `127.0.0.1`, requires public-key authentication, retains strict host-key and
SSH strict-mode checks, and never changes an existing account or SSH config.

It checks real SSH API routing, shell quoting, same-ID isolation, offline results,
wrong-host-key refusal, timeout recovery, catalog concurrency, no implicit daemon
start, cleanup after an OpenSSH ProxyCommand exits with an inherited pipe, and an
actual terminal attach/input/detach/refresh cycle. It renames the selected remote
workspace and reuses its old name before Enter, then verifies that input still
reaches the original ID. A second selector checks that deleting an ID and reusing
its name cannot redirect attachment. Terminal settings must be restored in both
cases. `--logs` preserves results, binary hash, SSH/server logs, the ProxyCommand
cleanup record and the terminal transcript. Linux prerequisites or failures
return nonzero. Other platforms return `NOT_RUN` with a nonzero exit; they do not
masquerade as a pass.

Some containers run root without `CAP_AUDIT_WRITE`. Their system sshd can accept
API requests but reject a root PTY with `linux_audit_write_entry failed:
Operation not permitted`. In that environment, an authorized root caller can
explicitly choose an **existing unprivileged user**:

```sh
python3 scripts/ssh_federation_smoke.py /absolute/path/to/crew --fixture-user oai --logs /tmp/crew-ssh-evidence
```

Use a suitable existing account on your machine; `oai` is only an example. This
option requires `runuser`, copies the binary and harness into a disposable
directory owned by that account, and executes the same real test there. It does
not create or unlock an account, edit a home directory, change SSH policy, or
grant capabilities. The default never switches identities. Both caller and
fixture identity and argv appear in the output; retained result JSON identifies
the actual fixture user. Preserve the initial environment failure separately.

The real SSH smoke is Linux evidence. macOS and Windows client behavior still
requires validation on those operating systems; Rust compilation alone is not
native SSH/terminal validation.
