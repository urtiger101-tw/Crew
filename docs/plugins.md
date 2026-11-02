# Crew plugin marketplace

Crew 0.5.0 provides a working offline catalog, optional public GitHub discovery,
reviewable installation, local development links and explicit command actions.
Plugins use `crew-plugin.toml`. Herdr manifests and its full plugin API are not
wire-compatible with Crew.

## Browse and install

```sh
crew plugin marketplace list
crew plugin marketplace search worktree
crew plugin marketplace show crew.workspaces
crew plugin inspect crew.workspaces
crew plugin install crew.workspaces
crew plugin action list --plugin crew.workspaces
crew plugin action invoke crew.workspaces.snapshot
```

The two bundled entries are real manifests embedded in the binary. Workspace
Inspector lists workspaces/agents and reads a snapshot from an existing daemon.
Git Worktree Inspector lists real Git worktrees for the caller's directory.
Neither starts an agent, contacts an AI service, or implicitly starts Crew.

`marketplace search QUERY --online` additionally searches public GitHub
repositories with the `crew-plugin` topic. It uses `curl` (Windows: `curl.exe`)
with HTTPS verification, bounded output and a deadline. Search results are
unreviewed repository metadata. They do not prove the repository has a valid
manifest; use `inspect owner/repo[/subdir]` to fetch and validate the manifest.
No external marketplace service, account or API key is required.

```sh
crew plugin inspect owner/repo/plugin --ref FULL_COMMIT_SHA
crew plugin install owner/repo/plugin --ref FULL_COMMIT_SHA --yes
crew plugin update owner/repo/plugin --yes
```

GitHub installation requires Git and accepts repository shorthand only. The
fetch records the actual commit and uses a fresh managed directory. HTTPS is
the only allowed Git transport; checkout uses empty global/system Git config,
no hooks or submodules, and no interactive credentials. An explicit `--ref`
tracks that branch/tag/commit during update; an immutable commit stays pinned.
Updates skip an unchanged commit. Old directories are retained so an already
running action can finish. Automatic garbage collection is not implemented.

Installation and linking show the source, manifest and commands before asking
for approval. Noninteractive callers must pass `--yes`. Build commands run only
after approval. A failed build or a manifest changed by the build is not
registered. Actions never run just because a plugin is installed or enabled.

Plugins are ordinary programs with the invoking user's permissions. Crew does
not sandbox their code. Only install sources you intend to trust.

## agentcmd utilities

The installer targets [agentcmd at commit
`617a90c73554ccb5a6c6eebc631e26c78948927a`](https://github.com/stevenke1981/linux-bash-command/tree/617a90c73554ccb5a6c6eebc631e26c78948927a).
At that revision, the installer creates 46 separate plugin modules, one per
utility. A newly installed module starts disabled and can be enabled
independently: for example, `agentcmd.wc` supplies `wc`, and the utility `[` uses
the plugin ID `agentcmd.bracket`. A fresh installation does not add any of the
module directories to a pane.

### Install and enable one utility

Git, tar and Cargo are required. The installer fetches the pinned source by default, presents
the source and build for review, and builds one shared agentcmd binary after
approval. It makes a separate executable directory for each module; the shared
binary directory itself is never added to a pane's PATH. For a noninteractive
installation, `--yes` supplies approval. Review the source and build before
using it.

```sh
crew plugin agentcmd install
crew plugin enable agentcmd.wc
crew plugin list --json
```

For a local source, `--source PATH` must point to a Git repository containing
the exact pinned commit. The installer exports that commit rather than building
uncommitted checkout changes. Use `--offline` with `--source` to make Cargo
work offline; the required dependencies must already be available locally.
Without `--offline`, Cargo may download dependencies even with a local source.

```sh
crew plugin agentcmd install --source /path/to/agentcmd-repository --offline
```

To reinstall or refresh the managed modules, rerun
`crew plugin agentcmd install`, with the same source options if needed. Generic `crew plugin update`
skips these managed local entries. Reinstallation preserves the enabled state
of unchanged managed modules; check `crew plugin list --json` rather than
assuming that every module is disabled afterward.
Older agentcmd installation stages remain on disk so open panes keep their
commands. Remove an old stage manually only after no pane still uses it.

Enabling a module adds its executable directory only to newly spawned panes,
including workspace-created, tab-created and split panes. Running panes keep
their original environment. An enabled, compatible, unchanged plugin's
validated directories are prepended in plugin-ID order to the pane's base PATH,
including any workspace PATH override. The base PATH is preserved, so existing
system commands remain available; an enabled module can take precedence over
an executable of the same name. A shell profile that changes PATH after startup
can move another executable ahead of the module. PowerShell aliases and shell built-ins may
still win command resolution: `[` is a built-in in many POSIX shells, `cat`
is commonly a PowerShell alias, and Windows may already provide `sort.exe`.
Check the resolved command and its path instead of assuming those names are
absent.

### Verify and disable

`crew plugin list --json` reports each module's `enabled`, `available`,
`diagnostic`, `resolved_path_dirs` and `effective_path_dirs`. A resolved path
alone does not mean the module is enabled; `effective_path_dirs` describes the
current directories eligible for new panes. To see what a particular pane
received when it was created, inspect `plugin_path_dirs` and
`plugin_path_diagnostics` in its `pane get` JSON. For an existing workspace,
replace the IDs below with the ID from `pane list` and the new ID returned by
`pane split`:

```sh
crew pane list
crew pane split PANE_ID --direction right --no-focus
crew pane get NEW_PANE_ID
```

In the new pane, compare the resolved `wc` path with that pane's injected
directories and run a small check. In a POSIX shell:

```sh
command -v wc
command -v sort
command -v cat
printf 'one two\n' | wc -w
```

In PowerShell:

```powershell
Get-Command wc, sort.exe, cat -All | Format-Table CommandType, Name, Source
'one two' | wc -w
```

Both examples should count two words. When only `agentcmd.wc` is enabled,
`sort` and `cat` must not resolve from agentcmd module directories; a shell
alias or system executable with either name may still exist.

```sh
crew plugin disable agentcmd.wc
crew plugin list --json
crew pane split PANE_ID --direction right --no-focus
crew pane get NEW_PANE_ID
```

After disabling, compare the newly created pane's directories with the earlier
pane's directories. The new pane should not include the `agentcmd.wc` module
directory; the earlier pane keeps its original environment.

## Author an action plugin

```toml
id = "example.tools"
name = "Project tools"
version = "1.0.0"
min_crew_version = "0.5.0"
description = "Inspect a running Crew workspace"
platforms = ["linux", "macos", "windows"]

[[actions]]
id = "snapshot"
title = "Read workspace snapshot"
command = ["{crew}", "api", "snapshot"]
```

Required metadata is validated, including minimum Crew version, portable IDs,
explicit platform support, unique action IDs and bounded argv lists. Unknown
fields are rejected. `[[build]]`, `[[actions]]` and the `[[path_dirs]]`
capability below are supported. A plugin must declare
at least one action or PATH directory. It does not silently accept unsupported
startup hooks, event hooks, plugin panes or URL handlers.

To expose executables to new panes, a manifest can also declare:

```toml
[[path_dirs]]
path = "bin"
platforms = ["windows"]
```

`path` is a portable relative directory under the approved plugin root;
`platforms` is optional and otherwise inherits the plugin's platforms. The
directory must exist, and absolute paths, traversal and symlink escapes are
rejected. Only enabled, compatible plugins with unchanged, valid manifests
contribute directories. Inspect `crew plugin list --json` for effective paths
and diagnostics.

```sh
crew plugin link ./my-plugin --yes
crew plugin action invoke example.tools.snapshot
crew plugin disable example.tools
crew plugin enable example.tools
crew plugin log list --plugin example.tools --limit 10
crew plugin config-dir example.tools
crew plugin state-dir example.tools
crew plugin unlink example.tools
```

`link` validates and registers the existing directory without running build
commands. If its manifest later changes, invocation is refused until it is
linked again after approval. A GitHub install cannot overwrite a local link.
`update` skips local links. `unlink`/`uninstall` unregister the plugin but keep
its source, configuration, state and logs. Reinstallation preserves the same
config and state directories.

Action commands are argv arrays passed directly to the operating system. Explicit
relative executable paths such as `./bin/action` resolve from the plugin root on
every platform; bare program names use the ordinary executable search path. Build
commands use the same path rule. Action discovery retains healthy plugins if a
different linked plugin changes or disappears. Entries expose `available`,
`manifest_unchanged`, and `diagnostic`; stale entries remain unavailable until
approved again, and invocation still rejects them.

No shell interpretation occurs unless the action explicitly invokes a shell.
Whole argument `{crew}` becomes the current executable and `{cwd}` becomes the
invoking directory. Runtime cwd is the plugin root. Output is bounded to 64 KiB
per stream, with truncation recorded. The default deadline is 30 seconds;
`--timeout-ms` accepts 1..300000. Nonzero exits, spawn errors and deadlines are
failed actions and remain in the log, with the actual exit code and output.

Actions receive `CREW_BIN_PATH`, `CREW_PLUGIN_ID`, `CREW_PLUGIN_ROOT`,
`CREW_PLUGIN_CONFIG_DIR`, `CREW_PLUGIN_STATE_DIR`, `CREW_PLUGIN_ACTION_ID` and
`CREW_PLUGIN_CONTEXT_JSON`. Existing socket/pane context is inherited; plugin
context records the invoking cwd and pane ID. Durable settings belong in the
config/state directories, not in managed installation directories.

The registry uses an operating-system file lock for changes and atomic file
replacement. `CREW_PLUGIN_HOME` overrides its user data directory, including
installations, config, state, build logs and command logs. JSON commands wrap
their result in `{"result": ...}`; an unsuccessful action returns exit code 1.
Use qualified `plugin.id.action` identifiers if local action names collide.

## Validation

`python scripts/plugin_smoke.py PATH_TO_CREW` runs without network. It exercises
the embedded catalog, approval, disabled plugins, a real Crew daemon, real
Python fixture actions, Unicode paths, literal argv, manifest changes,
reinstall/update, timeouts, failure logs and preserved user data. It never
downloads or executes a third-party plugin. Unit tests additionally cover
manifest bounds, source/ref validation, process groups and OS file locks.
