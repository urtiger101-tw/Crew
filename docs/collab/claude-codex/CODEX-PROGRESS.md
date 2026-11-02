# Codex progress display sync

`crew integration codex-progress` is a local compatibility bridge with an explicitly specified source. It reads the assistant commentary from the selected Codex rollout JSONL and, through the existing presentation metadata, shows the latest steps on Crew's working label. It does not send keyboard input, does not call a model, and does not change the agent lifecycle, completion conditions, or recipient identity.

```powershell
crew integration codex-progress --session-file C:\path\to\rollout.jsonl --pane p4 --source codex.progress.own-session --title "Review collaboration task"
```

- You must specify `--session-file`, `--pane`, and a `--source` that is unique to each watcher; do not search for or guess other sessions.
- The target pane (窗格) must already have Codex detected, or be explicitly reported as codex by its owner. At startup, record the daemon epoch, the detected process PID, and the reported instance; re-check them on every poll, and stop when any of them changes.
- `--once` reads the full data once, returns the number of updates, and suits an isolated acceptance check (驗收). Normal mode checks every 500 ms.
- At startup, read back at most 1 MiB; each line is capped at 256 KiB. A JSON line that is not yet complete waits for its newline; over-long or incompatible records are skipped.
- Use only `output_text` from `response_item/message` where `role=assistant` and `phase=commentary`. Do not publish the user prompt, analysis, tool arguments, or tool results, and do not also adopt `event_msg/agent_message` as a duplicate source.
- Published commentary becomes the `state_label`. Every attached client connected to that Crew and every federation querier can see it; it is not text private to the synchronizer or to the current pane's user.
- The summary strips control characters and bidirectional text controls, and is at most 72 characters. It does not estimate a percentage from elapsed time, the spinner, or output volume.
- `task_started`, `task_complete`, `turn_aborted`, and assistant final_answer only clear this source's display; they do not report Crew work as complete.
- Each real commentary update's presentation has a 60-second TTL; without new data it expires and returns to the original display. `--title` belongs to the same short-lived presentation; if it is not specified, the name is not overwritten.

This JSONL format is supported based on actual records from the local Codex 0.166 series; it is not a stable cross-version API. The process check and the metadata submission are two separate local requests and are not an atomic identity guard. This tool stops at later checks, and the agent kind/source guard and the TTL bound the residual risk. A stricter app-server event adapter, an explicit startup binding, and an atomic instance guard are deferred to the contract phase of the collaboration plan.

For isolated acceptance, first create a fixture pane on a separate socket, use `integration report codex start --source fixture --pane pN` to create an explicit producer, then run `--once` with a JSONL file containing only test commentary. Check the pane's `display.state_label` and `display.presentation`, and confirm that `agent.instance/state` was not rewritten by the synchronizer. For real use, choose the rollout that matches the Codex instance actually running. A fixture acceptance check must not be treated as automatic source identification, or as evidence that two agents completed the work.
