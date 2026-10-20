# Automatic agent-kind detection

Crew can show an agent kind from a live pane process without requiring a hook.
This is a process-name hint, not verification of a vendor binary or knowledge of
the agent's task state. The daemon never launches an agent or a shell probe to
detect it. An installed executable or printed terminal text is insufficient.

Detection is enabled by default and can be disabled without restarting:

```toml
agent_detection = false
```

```sh
crew set agent-detection off
crew set agent-detection on
crew integration explain p1
```

Disabling removes detected hints on the next sample. Re-enabling observes the
current process afresh. Existing explicit reports and their waits are preserved.

## Evidence by platform

| Native platform | Evidence | Coverage and limits |
| --- | --- | --- |
| Linux | PTY foreground process-group leader, `/proc/PID/exe` and bounded `/proc/PID/cmdline` | Recognized native executable, or interpreter script position/package path. An unrelated background process is ignored. |
| macOS | PTY foreground process-group leader and native `proc_name` | Recognized native executable names. A CLI visible only as `node` or `python` needs an explicit report. |
| Windows | Native read-only Toolhelp snapshot and pane-child descendant relationships | Recognized `.exe` names in the pane tree. One unambiguous kind is required. Background children can qualify; this is descendant evidence, not a foreground claim. Node/Python arguments are not inspected. |
| Other platforms | `unsupported` | Explicit reports and existing PTY activity status remain available. |

The sampler runs roughly twice per second. It collects pane identities under the
state mutex, performs process I/O outside that mutex, and checks pane identity
again before applying a hint. Linux rechecks process start time and executable;
command-line input is capped at 16 KiB. Windows takes one process-only snapshot
per sample, caps it at 16,384 entries, and bounds parent walks to 64 steps with
cycle detection. Snapshot errors, unavailable foreground process IDs, inaccessible
process data, ambiguous kinds, or an exited process yield no hint. A successful
sample can restore a hint. No external `ps`, PowerShell or WMI process is launched
by the sampler.

Recognized executable names are `claude`, `codex`, `opencode`, `gemini`, `grok`,
`copilot`, `cursor-agent` (kind `cursor`), `pi`, `omp`, `mastracode`, `kiro-cli`
(kind `kiro`), `droid`, `hermes`, `kilo`, `kimi`, `amp`, `qwen`, `letta`, `maki`,
`muse`, and `aider`. Windows matching is case-insensitive and accepts `.exe`.
The generic name `agent` is deliberately ambiguous and is not matched. The
integration registry may list other hook/diagnostic commands; that list does not
promise every command has native process detection on every platform.

Linux also recognizes scripts in the immediate script argument of `node`,
`nodejs`, `bun`, `python`, or `python3` when their basename matches a known name.
Known `node_modules` package paths include `@anthropic-ai/claude-code`,
`@openai/codex`, `@google/gemini-cli`, `@github/copilot` and
`@mariozechner/pi-coding-agent`. Arbitrary arguments are never searched. Flags
before the script, version-suffixed interpreters, unusual wrappers, containers or
remote SSH commands can therefore need explicit reporting. Neither text banners
nor screen contents are used as agent evidence.

Executable names can be chosen by any local program. A harmless executable named
`codex` receives the same hint; Crew does not verify code signing, ownership or
provider authentication. Detected kind is not an authorization decision.

## Explicit lifecycle remains authoritative

An explicitly reported `AgentInfo` always takes priority over a detected kind.
The detector stores only `detected_agent`; it never changes the explicit agent's
source, kind, name, instance ID, sequence, resume command or semantic state.
Appearing, exiting, replacing, disabling or clearing a detected process cannot
finish or replace an explicit `agent wait`. Restored reports retain existing
restored-report semantics; a hint does not upgrade a restored state.

Without a report, detection supplies a name in the sidebar/title and includes the
pane in `agent list`. Status still uses Crew's existing PTY activity heuristic.
A quiet process is not evidence that a task completed. Commands that need an
agent instance, including `agent wait` and `agent prompt`, still require an
explicit report. Use `pane` commands for an inferred-only terminal and generic
integration hooks for reliable lifecycle reporting.

## Inspect the result

Pane API snapshots expose these separate fields:

```json
{
  "agent": null,
  "detected_agent": {
    "agent": "codex",
    "process_id": 1234,
    "executable": "codex",
    "evidence": "linux-foreground-process"
  },
  "process_detection": {
    "enabled": true,
    "backend": "linux-foreground-process",
    "kind_only": true
  }
}
```

Only the basename is exposed, not process arguments, environment variables or
full paths. `display.kind_source` is `explicit-report`, `process-detection`, or
`none`. With an explicit report, `detected_agent` can still show secondary process
evidence while `display` uses explicit identity. `agent.detected` events carry a
changed hint or `null` when it disappears. They do not emit `agent.status` or
`agent.released` lifecycle transitions.

`integration list` reports the compiled backend and its kind-only scope; daemon
configuration may disable it. `integration explain` separates explicit reports,
live process evidence and activity heuristics. `integration doctor` still checks
executable files on PATH only, does not start them, and does not claim they are
running or that hooks were installed.

## Local verification

```sh
cargo test --locked --offline --lib
python scripts/agent_detection_smoke.py /path/to/native/crew
```

The smoke copies system `sleep` on Linux/macOS or `cmd.exe` running built-in
`pause` on Windows. It exercises actual pane process evidence, exit/replacement,
printed-name false positives, explicit priority, enable/disable and wait isolation
without paid agents. Unix also checks an argv-name spoof; Linux checks an
unrelated background fixture.

Linux native behavior was exercised during implementation. The Windows selector's
descendant/ambiguity/cycle rules have host-independent unit tests; these are not
native Toolhelp execution. Windows and macOS runtime verification require those
operating systems and must be reported separately. This feature supplies
process-based kind hints, not Herdr's full screen-rule or plugin-manifest state
detection engine.
