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
fields are rejected. This version supports `[[build]]` and `[[actions]]`; it
does not silently accept unsupported startup hooks, event hooks, plugin panes
or URL handlers.

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
