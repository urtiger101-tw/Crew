# Turn-complete hook probes (Claude Code 2.1.316 / Codex CLI 0.166.0) — 2026-11-02

Runs used: Claude 3/8 (all --model haiku; C3 had 2 extra accidental Haiku turns), Codex 1/4 (gpt-6-luna). Nothing left running. Real settings untouched.

| ID | Verdict | Finding |
|---|---|---|
| C1 | PASS | Stop hook from a temp `--settings` file runs when `claude -p` ends. stdin JSON: session_id, transcript_path, cwd, prompt_id, permission_mode, effort, hook_event_name=Stop, stop_hook_active, last_assistant_message, background_tasks, session_crons. |
| C2 | PASS | `{"decision":"block","reason":"INBOX: ..."}` makes Claude continue; it sees "Stop hook feedback:\n<reason>" as a user message and answers (PINEAPPLE). Second Stop has stop_hook_active=true; hook allows; run ends. No loop. |
| C3 | PASS / UNKNOWN | Interactive TUI: Stop fires at turn end; does NOT fire while unsent draft text sits in the composer (20 s). External `send-text` appends to a human draft (mixing observed). Whether a block reason mixes with a pending draft: UNKNOWN. |
| X1 | PASS | `codex exec` runs `notify` once at turn end; argv JSON: type=agent-turn-complete, thread-id, turn-id, cwd, client=codex_exec, input-messages, last-assistant-message. |
| X2 | PASS / UNKNOWN | notify stdout/stderr never reach the session (one-way). `codex queue --thread <id> --message <text> [--resume]` exists in 0.166 ("Queue a message for an existing session"); delivery timing UNTESTED. `codex features list` shows `hooks stable true`; its semantics UNTESTED. |

Incident (C3, isolated server only): `send-keys C-u` did not clear the draft; `pane run "/exit"` then submitted "C:/Program Files/Git/exit" (Git Bash path conversion) as a prompt, causing 2 extra Haiku turns. Ended with `crew server stop` on the isolated socket.

Side effects: trust entry for the temp dir $P/c3/work recorded by Claude; `~/.claude/projects/*-probes-*-work` dirs created; codex exec may have written logs under ~/.codex.

Implications: Claude receiver = Stop hook + decision=block, guarded by stop_hook_active and a pending marker (idle-only delivery, no Enter injection). Never deliver via send-text into an interactive Claude TUI. Codex: notify only signals turn end; injection candidate = `codex queue` (to be probed) — fallback = single notifier with pane run only when composer state is known empty.

Evidence: $P/{c1,c2,c3,x1}/..., $P/hooks/stop-probe.js ($P = this probes dir).
