# Brief: translate Crew agent rules and skills to English

## Goal
Every rule, skill, protocol and agent-facing instruction file used in Crew is written in English.
Chinese stays only as an attached gloss for keywords and for prompt text that agents or the
owner type literally. Meaning, structure, commands, code, paths, IDs and tables must not change.

## Style contract (all agents)
1. Translate prose to clear, plain technical English. Keep headings, list structure, tables,
   code blocks, inline code, links and anchors exactly. Do not add or drop rules.
2. Keywords: on the FIRST occurrence in a file, write `English term (中文)`; later occurrences
   are English only. Use the shared glossary below so all files agree.
3. Prompt text (a string an agent sends to another agent or the owner types, e.g. a `send-text`
   payload, a copy-paste prompt): keep the original Chinese string and add the English
   translation next to it, e.g. `"請讀 log N" (Read log N)`. Never change the Chinese string
   itself, because tools may match it.
4. Existing English text, code, commands, file names, model names, `crl` operators and keys:
   unchanged.
5. A `## Change history` table (named `變更歷史` before): translate its header and rows, and add
   one new row dated 2026-11-02: `Translated to English; Chinese kept as glossary for keywords
   and literal prompts`. Files without such a table: do not add one (exception: none).
6. Keep line endings as they are. UTF-8 without BOM.

## Shared glossary
| Chinese | English |
|---|---|
| 主人 | the owner (主人) |
| 主代理 | lead agent (主代理) |
| 子代理／代理 | sub-agent / agent |
| 派工 | assignment (派工) |
| 審查／獨立審查 | review / independent review |
| 作者不得自審 | an author must not review their own change |
| 交接 | handoff (交接) |
| 窗格 | pane (窗格) |
| 分頁 | tab (分頁) |
| 工作階段 | session (工作階段) |
| 工作區 | workspace (工作區) |
| 收件匣 | inbox (收件匣) |
| 證據 | evidence (證據) |
| 未執行／未驗證 | not run / not verified |
| 授權 | authorization (授權) |
| 重啟 | restart |
| 提交／推送 | commit / push |
| 所有權 | ownership |
| 驗收 | acceptance check (驗收) |
| 回報 | report |

## Ownership (one owner per file; touch nothing else)
- W1: `AGENTS.md`, `skills/crew/SKILL.md`, `docs/team/crew-v0.3.0/workflow-rules.md`
- W2: `docs/collab/claude-codex/PROTOCOL.md`, `docs/collab/claude-codex/CODEX-PROGRESS.md`,
  `docs/collab/claude-codex/CLAUDE-STOP-RECEIVER.md`, `docs/collab/claude-codex/SUMMARY.md`
- W3: `docs/collab/claude-codex/COLLABORATION-PLAN.md`
- W4: `crew-agent-handoff-20261102/README_使用方式.md`, `crew-agent-handoff-20261102/COPY_TO_AGENT.txt`
  (keep the file names)

## Out of scope (do not edit)
`docs/collab/claude-codex/log/*.crl` (history), `contract/B-V1.md` and fixtures (frozen by
SHA-256), `TRANSCRIPT.md`, release notes, review records under `docs/team/*` except the one
listed, README, UI strings and tests in `src/` (zh-TW localization is a product feature).

## Hard rules
- Edit only your own files. No git add/commit/stash/checkout. No new dependencies.
- If `SKILL.md` has YAML frontmatter, keep keys; translate `description` to English and keep
  any trigger keywords in Chinese appended, e.g. `... Triggers: pane, 窗格, 工作區`.
- W2 only: in PROTOCOL.md also add the agreed rule WAKE-PEER (from `log/119-claude.crl`,
  accepted in `log/120-codex.crl`) to the Transport section as v4, and add a v4 change-history
  row. Do not otherwise change behavior.

## Acceptance check
- `grep -P '[\x{4e00}-\x{9fff}]'` on each file shows Chinese only inside glossary parentheses,
  literal prompt strings, file names, or quoted UI labels.
- Every command, code block, path and table row count is the same as before (compare with
  `git diff`; for W4 files, which are untracked, compare with your own first read).

## Report format (final message, under 200 words)
Files changed; remaining Chinese and why each stays; anything uncertain; checks you ran.
