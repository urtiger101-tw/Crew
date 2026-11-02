# CRL — Crew Relay Lingo

Claude Code and Codex CLI use this compact language to talk to each other through Crew panes (窗格). Pane IDs change with each session (工作階段); treat `detected_agent` from `crew pane list` as the source of truth (currently Claude `p1`, Codex `p2`).
This language is for agents and is not meant to be human-readable. For the human version, see `TRANSCRIPT.md` and `SUMMARY.md`.

## 傳輸 Transport

1. Each message is one file: `log/NN-<from>.crl`, where `NN` is a two-digit sequence number; odd numbers are Claude, even numbers are Codex.
2. After the sender writes the file, it notifies the other side through Crew:
   - Claude → Codex: `crew pane run p3 "<短指令>"` (types the short command "<短指令>" into the Codex prompt line and presses Enter).
   - Codex → Claude: Codex prints a line containing only `EOT-NN` in its own pane; Claude waits for it with
     `crew pane wait-output <codex> --match EOT-NN`.
   - Codex may also wake Claude proactively (v3): `crew pane run <claude> "<短指令>"` (the same short command "<短指令>"), for example when a proposal, an assignment, or a question needs an immediate reply.
     While Claude is busy, this text is queued into its next input and is not lost.
3. The message file is the only source of truth; pane text is only a notification.
4. **WAKE-PEER (v4)**: this rule applies to both sides. After you write a log that contains `END`/`DONE` and needs the peer to take it over, you must wake the peer's pane:
   - Find the peer's pane with `crew pane list`, matching `detected_agent`. Do not hard-code the pane ID.
   - Run `crew pane send-text <id> "請讀 log N"` (Read log N).
   - Wait 2 seconds, then run `crew pane send-keys <id> Enter`.
   - Run `crew pane read <id>` to confirm that the peer has started processing, or that the message is queued.
   - If the peer is busy, the message is queued; it must still be sent.
   - An ACK-only reply needs no wake-back.

## 語法 Syntax

```
§NN from>to re:MM            # 標頭；re 可省略
OP k=v k=v :: 自由文字       # 一行一句；`::` 之後可為任何語言
...
END                          # 回合結束
```

## 運算子 Operators（v0）

| OP | Meaning | Common keys |
|---|---|---|
| `OBS` | Observed fact | `ev=` (evidence (證據): `f:path:line`, `cmd:...`) |
| `PRO` | Proposal | `id=Pn` `area=cli\|ui\|api\|agent\|doc\|inst` `pri=1..3` (1 is highest) `cost=S\|M\|L` |
| `VOT` | Vote on a proposal | `Pn` `+` / `-` / `~` (for / against / reserve) `why=` |
| `AMD` | Amend a proposal | `Pn` plus the keys to change |
| `ASK` / `ANS` | Question / answer | `q=Qn` |
| `LEX` | Add or change a term in this language | `w=<new term>` `m=<meaning>` |
| `ASG` | Assignment (派工); two-way since v3: either side may assign work to the other | `who=` `id=Pn` |
| `OWN` | File ownership list; one file belongs to one side at a time | `<who> :: files…` |
| `RULE` | Hard constraint | — |
| `DONE` | Completion criteria and report format | — |
| `REV` | Independent review result; an author must not review their own change | `id=Pn` `PASS\|FAIL` |
| `FIN` | Propose closing; lists the agreed proposal IDs | `ids=P1,P3` |
| `END` | End of turn | — |

Use `_` in place of spaces in values. The discussion ends when both sides agree to the same `ids` in `FIN`.

## 協作角色 Roles（v3 新增）

- Both sides are **peers**: either side may `PRO` new work, `VOT` or `AMD` the other side's proposals, assign work to the other side with `ASG`, or reply to a received `ASG` with `AMD` or `VOT -` and a reason.
- Before assigning work, confirm that the `OWN` lists do not overlap. If you need the other side's file, use `ASK`; do not modify it yourself.
- The decision of the owner (主人) takes precedence over the peers' consensus. git commit / push is still executed only by Claude during integration (unless the owner instructs otherwise). An implementer must not review their own work; the other side runs `REV` before integration.
- While waiting, proactively `PRO` work that can run in parallel instead of sitting idle.

## 實務教訓 Lessons（v1 新增）

- Do not write the completion marker verbatim in notification text: notifications echo back in the other pane, and `wait-output --match` would match the echo first. Describe the marker instead (for example, "E, O, T joined with -NN" (「E,O,T 以 -NN 連接」)), and wait with the anchored regex `^\s*EOT-NN\s*$`.
- After the marker is seen, still confirm that the message file exists and ends with `END`; the marker is only a notification.
- The TUI may add a bullet in front of the marker line (for example `• EOT-06`), so the anchored regex must allow a leading symbol: `^\W*EOT-NN\s*$` (v2).
- When sending into an agent TUI, send the text and the Enter key separately: `send-text` → confirm the text has appeared → `send-keys Enter`.

## 詞彙增修 Lexicon log

`LEX` takes effect once the other side gives `VOT +`, and is appended to the table below.

| Version | Term | Meaning | Proposed by | Message |
|---|---|---|---|---|
| v1 | `IMP` | Implemented in the working tree with evidence keys attached; **does not mean** that tests or the live acceptance check (驗收) passed | Claude | §03 (Codex §04 concurs) |
| v1 | `NR` | Not run / not verified; any claim without execution evidence must be marked, and must name which check is missing | Claude | §03 (Codex §04 concurs and adds to it) |

## Change history

| Version | Date | Content | Scope |
|------|------|------|----------|
| v4 | 2026-11-02 | Added the WAKE-PEER rule to Transport: after a handoff log that needs the peer, find the peer's pane by `detected_agent` and wake it (send-text → wait 2 s → send-keys Enter → pane read) | This file |
| — | 2026-11-02 | Translated to English; Chinese kept as glossary for keywords and literal prompts | This file |
| v3 | 2026-11-02 | Two-way assignment (ASG/OWN/RULE/DONE/REV added to the vocabulary), Codex may proactively wake Claude, peer role rules | This file |
| v2 | 2026-11-02 | Marker lines may carry a TUI bullet (EOT-anchor) | This file |
| v1 | 2026-11-02 | Added the IMP and NR vocabulary and transport lessons learned | This file |
| v0 | 2026-11-02 | Initial version, drafted by Claude | This file |
