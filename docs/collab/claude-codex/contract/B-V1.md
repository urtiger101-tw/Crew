# B v1：具名 peer 與持久訊息契約草案

狀態：**DRAFT／尚未凍結**。依據 §62、§64、§65 與 §68 的修訂；§66 指定 hook probes 也必須等雙方凍結 B 後才開始。
本文件與 [b-v1.fixtures.json](b-v1.fixtures.json) 是待審查的輸入／輸出範例。`peer`／`msg` 命令、儲存、重啟恢復與 hook 行為都尚未實作，範例不得當成 runtime PASS。

## 首版範圍與分工

- Codex：B fixtures、C 的 store／API／CLI、`message.stored`／`message.acked`，以及中央 Request、bridge methods、routing。
- Claude：B 獨立審查；B 凍結後才做 hook probes、Claude Stop receiver、Codex notify notifier、AC4 fixture。
- C 首版只有 `send/list/read/ack`，加一個經 probe 證實的 receiver；`msg wait`、輸出游標、通知合併與未讀 UI 留待後續量測。
- 新命令沿用目前 CLI 的 `{"result": ...}`／`{"error":{"code","message"}}` 與 JSON-lines bridge 的 `id/method/params.args`。文件中的新 methods 仍是提案。

## 身份與責任

`peer list` 投影現有的已回報 agent，且必須有 `agent rename` 指定的唯一名稱；沒有第二份 peer registry。
程序偵測、presentation title、session title、`codex`／`claude` 種類名稱都不是收件者名稱。
peer snapshot 回傳目前 daemon `epoch`、peer 的 `name/pane_id/instance/source/agent/state`；不回傳推測的 `input_ready`。

訊息的 `from/to` 是穩定名稱；`epoch/instance` 是當次投遞綁定，不能把更換後的程序當成先前已讀者。
持久訊息不因 daemon epoch 改變消失；通知前重新解析 peer，不能重用舊 pane 號碼。
`--from`／`--by` 是同一本機使用者下的責任標記與語意檢查，不是新的驗證或授權機制。
send 的 caller pane 由 CLI／bridge 繼承的 `CREW_PANE_ID` 識別；server 以該 pane 的 live reported name 核對 `--from`，不能反過來用 `--from` 選擇 caller。缺 caller pane 或名稱不符回 invalid_argument；caller 沒有 live named report 回 agent_not_found。
新收件者名稱必須已知：目前的 live reported name，或此 store 任一已存訊息的 from/to。已知 peer 離線仍可收新訊息；只有未知名稱回 unknown_peer，沒有另外增加 peer registry。

## Message

| 欄位 | 型別與語意 |
|---|---|
| `id` | server 指派、不重用的 opaque string；client 不解析其格式。 |
| `key` | sender 選的1..128 ASCII bytes重試鍵，字元為 A-Z/a-z/0-9/`.`/`_`/`:`/`-`；去重範圍為同一 inbox store 的 `from + key`。 |
| `from/to` | from 是 caller 的 live reported name；to 是 live 或 store 歷史已知的名稱。不接受 pane ID 或程序種類替代。 |
| `kind` | `handoff/review/question/answer/done`。kind 不自動產生 ack。 |
| `reply_to` | 原訊息 ID 或 `null`；非 null 時須指向同 store 的已存訊息。 |
| `summary` | UTF-8 文字，最多 **512 bytes**，不是 512 個字。 |
| `artifacts` | 最多16個 Artifact，純文字訊息用空 array。 |
| `created_at` | server 指派的 UTC RFC3339 時間；retry 不改寫。 |
| `acks` | 依時間保存的明確 receiver 確認；新訊息為空 array。 |

相同 `from + key` 與相同 submitted content 回原 ID，`deduplicated=true`，不新增 `message.stored`。
比較內容為 `from/to/key/kind/reply_to/summary` 及按順序排列的 artifact `path/sha256/bytes`；不包含 repo_root、server 的 id/time/acks 或通知狀態。JSON object 的 key 排列不影響比較。
retry 的 repo_root 改變大小寫、斜線寫法或換到另一個 worktree 仍去重，保留首次保存的 repo_root，不改寫原訊息。其餘比較內容改變則回 message_conflict；要改內容請使用新 key。
send 處理順序固定為：驗證 caller/live sender 與 --from → 查既存 key 的去重／衝突 → 檢查新 recipient 是否已知 → 檢查容量 → 保存及發事件。去重先於 recipient/capacity 檢查，不略過 sender 身份核對。

## Artifact：跨 worktree 規則

草案選擇把 `repo_root` 放在每個 artifact，保持 message 欄位清單不變：

```json
{
  "path": "docs/collab/claude-codex/contract/artifacts/handoff-中文.txt",
  "repo_root": "D:/Crew-agentcmd",
  "sha256": "b7244c63e27e859e587353f499b550454fab186247d6efdfd00372fe2365573b",
  "bytes": 64
}
```

此段數字與 digest 和 fixtures JSON 相同，由附帶 artifact 的原始 UTF-8 bytes 計算。

- `path` 相對於 sender 的 repository root，固定用 `/`；不接受絕對路徑、drive prefix、反斜線、空 segment、`.`／`..`、colon 或 NUL。
- `repo_root` 是 sender 提供的 absolute display context；**不參與 receiver 的檔案解析，不是 fallback root**。fixture 的 Windows root 是示例，不要求機器存在該路徑。
- receiver 使用自己明確設定的 repository root 解析 `path`，確認解析後仍在該 root 內；不能因收件者 cwd 改變而改讀別處。
- 先讀 bytes 做 `bytes`／SHA-256 核對，通過後才交給 agent 消費或使用。雜湊驗證本身必須讀檔，不宣稱 mismatch 前完全不讀 bytes。
- missing/mismatch 回報給 receiver；不得改讀 sender 的 `repo_root`、忽略 digest 或自動複製 sender worktree。需要另一份內容時，sender 用新 key 重寄。
- digest 只證明內容一致，不授予執行檔案、傳送憑證、合併或發布的權限。
- 附帶的 [handoff-中文.txt](artifacts/handoff-中文.txt) 沒有換行，避免 Git 的 LF/CRLF 轉換改變 fixture bytes。

## Ack 與通知

| stage | 必要欄位 | 意義 |
|---|---|---|
| `read` | `stage/by/at/note` | intended receiver／adapter 明確確認已取到訊息；不代表模型理解或接受責任。 |
| `done` | 上述欄位加 `result=ok\|failed` | receiver 明確回報處理結果；不是測試、merge 或 push 的自動證據。 |

`by` 必須等於該訊息的 `to`；`at` 由 server 指派，`note` 是最多512 UTF-8 bytes的文字或 null。
`msg read` 只取內容，沒有 ack 副作用。kind=answer/done 的回信也不偷偷修改原訊息的 acks。

草案採最小單向狀態：未確認 → read → done。done 前沒有 read 回 `invalid_argument`。
同一 stage、by、result、note 的 retry 回 `changed=false` 並保留原 at；同一 stage 改內容回 `ack_conflict`，不覆寫歷史。done 後 retry 既有 read/done 仍可去重，不再新增確認。

通知另外保存 `message_id/state/at/note`，state 為 `deferred/notified/failed`；不放入 acks。
首版由 Claude 所有的 adapter 保存這份通知狀態，不新增 msg notify API，也不讓 adapter 直接改寫 message store。
通知重試、貼上成功、`sent=true`、PTY 安靜、turn-complete 事件都不能自動升級為 read。
receiver 的 Stop/notify 能力與非空 composer、approval UI 等情況，留待 **B 凍結後**的隔離 probe 驗證。

## CLI 與 bridge 範例（尚未實作）

下列命令僅供審查，不能對目前 daemon 執行：

```text
crew peer list
crew msg send --from crew-codex --to crew-claude --key review-b-1 --kind review --summary "請審查 B v1 草案；核對相同 key 的重試與跨 worktree artifact。" --repo-root D:/Crew-agentcmd --file docs/collab/claude-codex/contract/artifacts/handoff-中文.txt
crew msg list --to crew-claude --open --limit 20
crew msg read msg_fixture_1
crew msg ack msg_fixture_1 --by crew-claude --stage read --note "Receiver fetched the message"
crew msg ack msg_fixture_1 --by crew-claude --stage done --result ok --note "Review recorded in a reply"
crew msg send --from crew-claude --to crew-codex --key answer-b-1 --kind answer --reply-to msg_fixture_1 --summary "接受範圍；請修正 fixture 的兩點。"
```

`--file` 可重複；有 artifact 時 `--repo-root` 必須明確指定。sender frontend 算 digest/bytes，daemon 保存 manifest，不從 display-only repo_root 自動取內容。
bridge 方法為 `peer.list`、`msg.send/list/read/ack`，params.args 就是 group/subcommand 後的同一份 CLI argv。
read/list 不需要進入 agent TUI；send/ack 不自動呼叫模型或送 Enter。所有 --help/-h 都必須無副作用且尊重文字值。

list 的草案回覆為 `messages/next_cursor/has_more`；messages 只含簡短 header 與 read/done 狀態，完整 artifacts/acks 用 read 取得。
`--unread` 表示尚無 read ack；`--open` 表示尚無 done ack，包含 read-but-not-done。兩個 filter 不可同時使用，否則 invalid_argument；未指定時列出全部符合收件者的訊息。
`--after` 使用 opaque inbox cursor，與 daemon epoch 分離；cursor 對應 store，重啟後仍可用，錯 store/非法 cursor 回 invalid_argument。
預設 oldest-first，limit 預設20、範圍1..100。`next_cursor` 只前進到本頁最後回傳訊息，不跳過仍未回傳的符合項目；空頁保留傳入 cursor。
最小 receiver 每次查 `--open`，不能只靠 --unread 或 cursor：read 後 crash 的工作仍必須列出。adapter 依 read ack 判斷是否已交付，再決定恢復或處理；不能把 read 當 done，也不能宣稱模型工作恰好執行一次。

## Store 位置與上限

一個 Crew socket 對應一個 store，預設 `<crew state dir>/inbox/<socket-name>.json`。state dir 是已解析的 Crew state file 所在目錄，socket-name 使用當前 IPC base name，例如 crew-eda.sock；不移除 .sock，故檔名是 crew-eda.sock.json。
`CREW_INBOX_STORE` 可改成明確的 absolute file path，供隔離測試或手動設定；不改變該 socket 的 peer 身份。socket-name 若不是合法的單一檔名 component，須使用此 override，不做會碰撞的截斷／取 basename。
儲存沿用 plugins/store.rs 的 file lock 與原子檔案替換慣例；鎖涵蓋讀取、去重／界限檢查與保存。成功回覆及 message event 都在保存成功之後；這不外推為實測過的斷電保證。

| 項目 | v1 上限 |
|---|---|
| summary | 512 UTF-8 bytes |
| ack note | 512 UTF-8 bytes，或 null |
| artifacts | 每訊息16個 |
| key | 1..128 bytes，`[A-Za-z0-9._:-]+` |
| messages | 每 store 10000個 |

未通過欄位界限回 invalid_argument。10000筆時的新 key 回 store_full，訊息與事件不改變；既存相同 retry 仍正常去重，read/list/ack 仍可用。v1 永不自動刪除訊息，pruning 留待未來明確命令；不會為騰空間偷偷丟掉已讀或未讀資料。

## Events

沿用既有的 `cursor/timestamp_ms/kind/pane_id/data` envelope，message 事件的 pane_id 為 null，因為收件者是 logical peer。
持久更新成功後才發出 `message.stored` 或 `message.acked`；去重 retry 不新增事件。
stored 的 data 為 message_id/from/to；acked 為 message_id/stage/by，done 才加 result。事件不含 summary 或 artifact 內容。
event cursor／epoch 仍只屬於目前記憶體 journal，不能當作 durable inbox cursor；重啟／遺失事件後以 msg list 補讀。

## Fixtures 的用途

[b-v1.fixtures.json](b-v1.fixtures.json) 包含 peer snapshot、stored/read/done 分支、回信、bridge envelope、去重／ack／UTF-8 邊界／通知狀態／跨 worktree artifact scenarios。
所有 expected 值都是 **未來實作的 acceptance oracle**；不是已執行命令的輸出。
實際檢查只涵蓋 JSON、跨參照、狀態一致性與 artifact digest/bytes；C 的持久化、重啟、CLI 和 hooks 均為 NOT_RUN。

## §68 修訂與凍結

B1 排除 display-only repo_root 的去重比較並保留首份值；B2 ack 順序維持；B3 增加 --open 恢復 read-but-not-done；B4 允許離線已知 recipient；B5 固定位置與容量；NIT 將 --from 與 caller 的 live reported name 核對。
fixtures 的 caller context 是 test harness 的環境／前置狀態，不是新增的 bridge params 或可由 --from 覆寫的身份欄位。歷史 read/list 不要求 peer online；send retry 仍先要求 live sender，只有 recipient 可以離線。
雙方未明確核准相同雜湊的本文件及 fixtures 前，B 不凍結，C 與 hook probes 都不啟動。
