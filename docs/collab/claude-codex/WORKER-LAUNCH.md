# Codex worker bootstrap

`scripts/collab/codex_worker.ps1` supports `Prepare`, `Launch`, and `VerifyReceipt`. Use PowerShell 7 on Windows. The coordinator owns all routing and sends GO only after checking a successful receipt. Bootstrap is a prompt-level protocol, not an enforced lock against a worker that ignores instructions.

## Manifest and modes

Create a JSON template with `coordinator_thread_id`, `worker_name`, `model`, `effort` (`xhigh`), `role`, `task_id`, nonempty `owned_files`, `sandbox_mode` (`workspace-write`), unique `nonce`, absolute `report_path`, and existing absolute `working_directory`. `report_path` must be inside the working directory and resolve to one of the owned files; owned paths can be absolute or relative to the working directory. `crew` requires `binary`, `socket`, `state`, and `inbox_store`; `config` and `pane_id` are optional. Binary, state, inbox, and config paths must be absolute. Values are non-secret. For first-launch discovery, `worker_thread_id` may be omitted or null: verification then accepts any syntactically valid thread token and does not establish the worker's identity. If provided, it must be a nonempty string token and the receipt must match it exactly.

```powershell
./scripts/collab/codex_worker.ps1 -Mode Prepare -InputPath ./template.json -ManifestPath ./new-manifest.json
./scripts/collab/codex_worker.ps1 -Mode Launch -ManifestPath ./new-manifest.json
./scripts/collab/codex_worker.ps1 -Mode VerifyReceipt -ManifestPath ./new-manifest.json
```

Prepare validates before atomically creating a new file and refuses to overwrite an existing file. Launch runs the specified `-CodexPath` (default `codex`) in the foreground with an argument array, `-a never`, `-s workspace-write`, the requested model, `xhigh`, and a bootstrap-only prompt. It refuses an already-present receipt with the same nonce. After the worker appends the line and waits, verify returns JSON and exit code 0 only for exactly one standalone receipt for the nonce with the expected worker, task and, when supplied, thread. Wrong, absent or duplicate lines fail. Use a fresh, unpredictable nonce for each launch; a reused nonce in an old report cannot establish freshness. An early successful receipt does not prove the worker waited for GO.

Receipt format: `RECEIPT nonce=<nonce> worker=<name> thread=<thread> task=<task>`.

## Environment and limitations

Each launch uses per-invocation `shell_environment_policy.inherit="core"` and TOML `shell_environment_policy.set` values only for the explicitly supplied `CREW_SOCKET`, `CREW_PANE_ID`, `CREW_STATE`, `CREW_INBOX_STORE`, `CREW_CONFIG`, and `CREW_COOP_BINARY`. No global configuration is written. A prior model-free probe reported that `inherit=core` plus `include_only` left the Crew values null, while explicit `set` retained them, kept PATH, and omitted an unlisted sentinel. The launcher tests check the generated arguments, not the actual Codex sandbox environment.

The isolated preview named pipe was previously denied to restricted-token workers (Windows os error 5); a configured socket is not proof of IPC access. Do not widen ACLs or fall back to a production socket. Use the agreed file/queue handoff until access is resolved. Receipt file verification is not a Crew message ack and does not activate hooks, validate task results, or authorize merge or push.

Run `pwsh -NoProfile -File ./scripts/test_codex_worker.ps1` for fake-Codex tests; these do not invoke a model.
