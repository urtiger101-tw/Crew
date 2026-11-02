# Making Crew easier to use: Claude × Codex discussion conclusions

> The version written for people. For the message-by-message conversation see `TRANSCRIPT.md`; for the raw messages see `log/`; for the communication language the two agents created see `PROTOCOL.md`.
>
> Participants: Claude Code (Crew pane (窗格) p1) and Codex CLI v0.166.0 (pane p3), placed side by side in the same Crew window. They exchanged 8 messages in total.

## Fixed and committed this round

| # | Problem you hit | Cause | Fix | Verification |
|---|---|---|---|---|
| P8 | Cannot see Codex respond | After `pane run` pasted the text, it waited only 30 ms before pressing Enter. Codex treated that Enter as a newline inside the pasted content, so the command stayed in the input box unsent | When the pane holds an agent (reported or detected), wait 150 ms instead, the same as `agent prompt` | Unit tests pass; Codex review found no issues; not yet verified by actually sending into Codex, because the running server is still the old version |
| P9 | No colors in Crew | Every pane inherited `NO_COLOR=1` from the server environment, and `COLORTERM` was not set | New panes set `COLORTERM=truecolor` and remove the inherited `NO_COLOR`; `--env NO_COLOR=1` still lets individual panes opt out of color | Live-tested on a separately started server and passed; Codex review found no issues |
| P10 | Want to scroll back through earlier output in a pane | Previously you had to press the prefix key to enter copy mode first, or use the mouse wheel | No prefix key needed: `Shift+↑／↓` moves line by line, `Shift+PgUp／PgDn` moves page by page. Scrolling back to the bottom exits automatically. Full-screen programs (such as vim) still receive keys as usual. `set scroll-keys off` disables it | Codex review caught a bug (keys were swallowed when switching to a full-screen program during scrolling); fixed and covered by a test. Not yet acceptance-tested by pressing keys manually in a real terminal |

The `↑／↓` keys without Shift are deliberately kept for the shell to recall command history, so they are not used for scrolling.

**To make these fixes take effect, the Crew server must be restarted.** A restart closes all current panes, including this Claude and Codex. You can first shut it down with `crew kill-server`, rebuild the release version, and then start it again.

## Follow-up suggestions both sides agreed on (not implemented yet, by priority)

1. **P1 Installation and diagnostics**: `crew doctor` reports which executable is actually running, the client and server versions, the socket, and the hook status. It also provides an explicit "加入 PATH" (add to PATH) action, and reminds the user that already-open shells will not update automatically.
2. **P2 Specify the command directly when opening a pane**: `new-window`, `split-window`, and `pane split` can all take `--command`, behaving the same as `tab create --command`, and return the new pane ID.
3. **P5 Agent mailbox** (Codex considers this the most valuable for agents): `crew msg send|wait|read`, with sender, recipient, message ID, reply ID, reading the body from a file, a read cursor, and timeouts. It removes the need to communicate by typing text into the other side's TUI or capturing the screen. Also let `agent prompt` work with agents recognized only by process detection; currently it returns `agent_not_found`.
4. **P7 Output checkpoint**: `wait-output --after <checkpoint>`, so that old output or echoed text cannot make a wait succeed early.
5. **P3 Move panes**: `join-pane` / `break-pane` move a pane, together with the program running in it, to another window. The ID, process, and working directory all stay the same.
6. **P4 Place two agents side by side with one command**: `crew pair <command-A> <command-B> [--layout h|v]`; do this after P2 is done.

Withdrawn: P6 (`--workspace` accepts a session name). After Codex verified it, the code was found to already support this.

## What the two agents learned

- Do not write the completion marker verbatim in a notification: notification text echoes in the other pane and satisfies the wait condition early.
- The TUI prefixes the marker with a bullet (`• EOT-06`), so the wait regex must allow that prefix. Claude once thought Codex had stalled, but Codex had already finished.
- `IMP` (implemented) does not mean the work has passed its acceptance check (驗收). Any claim without actual execution evidence must be marked `NR` (not verified). These two terms were added to the language after both sides agreed.

## Change history

| Version | Date | Content | Scope |
|------|------|------|----------|
| — | 2026-11-02 | Translated to English; Chinese kept as glossary for keywords and literal prompts | This file |
| v1 | 2026-11-02 | Final: results of fixes P8–P10, follow-up suggestions, lessons learned | This file |
| v0 | 2026-11-02 | Initial draft | This file |
