# Crew 主動協作與快速交換資料計畫

版本：2026-11-02 草案。需求：讓 Codex 與 Claude Code 在 Crew 中主動找對對方、可靠交接資料，減少人類轉述與畫面輪詢。
範圍：本機同一專案的兩個 agent；先沿用 Crew daemon、JSON API 與 CRL 檔案，再增加持久信箱。
本文件的 peer／msg 命令與自動收件 adapter 均為提案，尚未實作或進行兩個真實 agent 的端到端驗收；G 的終端標題接收與明確指定來源的進度同步已在本輪完成首版。
§62–65 已把首版縮小為具名 peer、send/list/read/ack 與一個 receiver；具體欄位和 CLI 待 [B v1 草案](contract/B-V1.md) 審查凍結。依 §66，hook probes 也等凍結後才開始。

## 已確認的起點

| 狀態 | 現有能力與限制 | 來源 |
|---|---|---|
| PASS／原始碼 | CRL 檔案是內容依據；窗格文字負責通知。已授權雙向提案、派工與喚醒。 | [PROTOCOL.md:8](PROTOCOL.md#傳輸-transport)、[PROTOCOL.md:48](PROTOCOL.md#協作角色-rolesv3-新增) |
| PASS／原始碼 | `agent rename` 可建立唯一的已回報 agent 名稱；解析精確名稱，重複名稱拒絕。僅有程序偵測的 agent 不能使用此身份。 | [src/api/agent.rs:167](../../../src/api/agent.rs#L167)、[413](../../../src/api/agent.rs#L413) |
| PASS／原始碼 | 明確回報已有 `source`、`seq`、`instance`；等待會辨識 occupant 被替換，避免把新程序當作舊工作完成。 | [src/api/mod.rs:216](../../../src/api/mod.rs#L216)、[src/api/agent.rs:34](../../../src/api/agent.rs#L34)、[228](../../../src/api/agent.rs#L228) |
| PASS／原始碼 | `crew api serve` 可維持一個 JSON-lines 子程序，以 request ID 對應非同步回覆；最多 16 個同時請求，單行上限 1 MiB。 | [src/bridge.rs:11](../../../src/bridge.rs#L11)、[69](../../../src/bridge.rs#L69)、[151](../../../src/bridge.rs#L151) |
| PASS／原始碼 | `events list/wait` 已有 epoch、cursor、逾時與遺失提示；日誌只有記憶體內的 512 筆，`pane.output` 只記位元組數。 | [src/events.rs:9](../../../src/events.rs#L9)、[67](../../../src/events.rs#L67)、[173](../../../src/events.rs#L173)、[src/server.rs:551](../../../src/server.rs#L551) |
| PASS／原始碼 | `pane run` 送文字與 Enter 後只回 `sent=true`；agent 的延遲是 150 ms，不能證明模型收到或開始處理。 | [src/api/pane/mod.rs:28](../../../src/api/pane/mod.rs#L28)、[121](../../../src/api/pane/mod.rs#L121)、[src/api/pane/keys.rs:160](../../../src/api/pane/keys.rs#L160) |
| PASS／原始碼 | `agent prompt` 會固定 instance、檢查 blocked；有等待時觀察 working/blocked，再等指定狀態。這仍是程序狀態，沒有每則訊息的收件確認。 | [src/api/agent.rs:554](../../../src/api/agent.rs#L554) |
| PASS／原始碼 | `wait-output` 每次搜尋現存終端邏輯行，沒有輸出檢查點；舊 EOT 或通知回顯可提早命中。 | [src/api/pane/mod.rs:159](../../../src/api/pane/mod.rs#L159)、[src/api/pane/read.rs:171](../../../src/api/pane/read.rs#L171)、[PROTOCOL.md:55](PROTOCOL.md#實務教訓-lessonsv1-新增) |
| PASS／原始碼 | 顯示 metadata 已有 title、state-label、token、TTL 與來源排序；它們與 agent 工作狀態及 wait 完成條件分開。 | [src/metadata.rs:18](../../../src/metadata.rs#L18)、[metadata-display.md:24](../../metadata-display.md#report-a-presentation)、[metadata.md:6](../../metadata.md) |

§57 記錄 Codex 窗格退出、app-server 工作仍繼續，後來改到另一窗格；這證明窗格與 agent 工作階段需要分開識別，退出原因仍未知。
§58–60 顯示目前交接仍需「請讀某號檔案」，審查和完成狀態靠文字約定；這些記錄不是新信箱功能的測試證據。

## 建議的最小使用流程

1. 每個 agent 的實際啟動／恢復流程回報唯一 `source`，設定專案內的 peer 名稱，並發布可接受訊息的能力。
2. 寄件端先保存 CRL 或其他交接檔，再送一個包含短摘要、檔案路徑與雜湊的結構化訊息；立即取得訊息 ID。
3. Crew 寫入信箱後發出事件；收件端整合程式從游標取新訊息，主動喚醒 agent 或留待當前工作結束。
4. 收件端取到內容後明確回報 `read`，完成時回報 `done` 的 ok/failed 結果並回覆原訊息 ID 與驗證依據；接受責任透過回信表達，首版沒有 accepted ack。
5. 使用者隨時看到待讀、處理中、需要協助及完成；雙方不必用整個終端畫面猜測工作是否已交接。

此流程需要收件端整合程式。Crew 有一個信箱，不會因此讓正在推論的模型自動取得新內容；必須驗證各 CLI／app-server 實際支援的接收方式。

## 優先順序與交付切片

| 優先 | 工作 | 依賴 | 最小交付與驗收 |
|---|---|---|---|
| P0 | A：先改善現有協作流程 | 無 | 明確回報、具名 peer、檔案交接與單一通知者；更換窗格後能重新綁定，找不到或有歧義就停止投遞並報錯。 |
| P0 | G：工作階段標題與可信的當前進度 | 可與 A 平行 | Crew 標題能顯示真實討論／工作階段名，例如 Review collaboration task；Working 旁能顯示目前步驟及來源時間，不編造百分比。 |
| P1 | B：凍結訊息與身份契約 | A 的實際流程資料 | 明確區分 logical peer、daemon epoch、agent instance、message ID、reply-to 與各種確認狀態；CLI/JSON fixture 可共同審查。 |
| P1 | C：持久信箱與查詢 | B 凍結 | `send/list/read/ack` 垂直切片與一個 receiver；重啟後仍能讀到，重試不產生第二則訊息；msg wait 延後。 |
| P1 | D：主動通知與兩端 adapter | B 凍結後 probes；C 完成後整合 | 驗證收件能力；ready 時送一次通知，busy/blocked 時保留待辦；不可因超時重送 Enter，通知合併延後。 |
| P2 | E：快速資料交換與可見狀態 | C、D | 常駐 JSON bridge、依收件人等待、短摘要與檔案指標；TUI 顯示未讀與需回應數，不把訊息全文塞入標題。 |
| P2 | F：輸出檢查點與相容橋接 | B；可與 C 平行 | 為仍需讀終端的整合補 `wait-output --after`；以 pane/epoch/輸出位置限定新輸出，舊標記不再命中。 |

建議先交付 A→B→C→D；E 與 F 按實測瓶頸安排。第一版沿用現有 daemon，不引入額外 broker、雲端服務或跨主機路由。

## G：讓使用者看得懂我們正在做什麼

- 使用者指出 Codex footer 已有 Review collaboration task，而 Crew header 只有 codex；本輪已補 OSC 0/2 標題接收、清理與名稱優先序，並以隔離 daemon 驗證中文與含分號標題。
- 已新增明確指定 rollout／pane 的 [codex-progress 相容橋接](CODEX-PROGRESS.md)，只發布真實 assistant commentary；本機 p4 已啟動同步，API 與實際 debug-frame 輸出均顯示 session 名稱與目前步驟。未重啟預設 daemon，新的 OSC parser 隨 Claude 整合後的 binary 啟用。
- Codex 本機 tui.terminal_title 已設為 thread、task-progress、spinner；新啟動的 CLI 會採用，現有 CLI 可透過 /title 即時選取。設定依據為 [官方範例](https://learn.chatgpt.com/docs/config-file/config-sample)；自動選取／綁定新 session 仍待 D 階段。
- 討論標題、路由 peer 名稱與程序種類分開保存；顯示 thread title 不得改變收件者身份，也不得讓舊標題套到替換後的 agent。
- 先沿用現有 presentation title，再接經正規化的終端 OSC title／已驗證的 adapter thread title；明確使用者覆寫的優先權需保留。
- 進度顯示「正在讀取第 60 則」「修正 API help」「執行測試 3/5」「等待 Claude 審查」等有來源的當前步驟；只有能取得真實總量時才顯示分數或百分比。
- adapter 回報 task/step/activity/updated_at；尚無真實步驟事件時就顯示 Working 與最後活動時間，不根據 spinner、輸出量或耗時推估完成度。
- 顯示進度透過 metadata 的來源、seq 與 TTL 接線；過期／斷線顯示未知或最後更新時間，避免看起來永遠仍在同一步。
- 工作進度、訊息已送達／已讀與工作完成是三種訊號；進度文字不可以滿足 message ack、agent wait 或測試完成條件。

## A：現在就能改進的工作方式

- 在每次真實程序啟動後使用既有 `integration report ... --source ... --seq ...`，再以 `agent rename` 設定例如 `crew-codex`、`crew-claude`；先確認 hook 確實接上。
- 名稱用於查找，`epoch + instance` 用於檢查這次接收者；恢復後重新取得綁定，不沿用舊窗格號碼。
- 目前 `agent prompt` 需要明確回報；若只有 detected_agent，顯示能力不足，保留經人工確認的 pane 通知方式。
- 交接檔先寫暫存檔、驗證 CRL header/END，完成後才發通知；通知只帶摘要及路徑，不包含可被等待條件誤判的原樣 EOT。
- 每個收件 agent 同時只有一個通知投遞者，避免兩段貼上／Enter 交錯；回覆內容保留在 CRL，直到信箱功能驗收。
- 先量十次實際交接的人工步驟、CLI 程序啟動次數、讀畫面次數、寄出到明確回覆的時間；目前沒有基準數據。

## B／C：訊息契約與持久信箱

建議命令介面如下，名稱與參數需在 B 審查後才凍結；這些命令目前不存在。

```text
crew peer list
crew msg send --from crew-codex --to crew-claude --kind review --summary "請審查交接" --repo-root D:/Crew --file docs/collab/claude-codex/log/61-codex.crl --key review-61
crew msg list --to crew-claude --open --limit 20
crew msg read MESSAGE_ID
crew msg ack MESSAGE_ID --by crew-claude --stage read
crew msg ack MESSAGE_ID --by crew-claude --stage done --result ok
```

- `peer` 屬於明確的專案／協作 namespace，名稱解析到當前回報者；不能把「偵測到 claude.exe」當作確定收件人或授權。
- 首版欄位為 `id/key/from/to/kind/reply_to/created_at/summary/artifacts/acks`；artifact 記相對路徑、大小、SHA-256 與 display-only sender repo_root，完整測試／diff 仍留在檔案。
- `send` 接受由寄件端建立的重試鍵，範圍為同 store 的 from + key；同鍵同內容回同一 ID，同鍵不同內容明確報衝突。
- §68 修訂：artifact.repo_root 不參與去重；recipient 可為 store 歷史已知的離線 peer；caller 的 live reported name 必須與 --from 相符。receiver 查 --open，讓 read 後中斷的工作仍可恢復。
- `stored`：Crew 已保存訊息。`read`：receiver／adapter 明確確認已取得內容。`done` 配合 result=ok|failed：receiver 回報處理結果。
- `read` 不能證明模型理解；`done` 也不等於測試通過、已合併或已發布，完成回覆仍需 CRL 的 IMP/TST/NR 與證據。
- 通知狀態另存 `deferred/notified/failed`；TUI 貼上成功只能算 notified，不能升級為 read。
- 長內容預設用 artifact；短訊息本體設定明確上限，列表只回摘要，可依 kind/task/peer 篩選，避免反覆搬運全部歷史。
- 第一版使用每 socket 一個有界本機 JSON store：state dir/inbox/socket-name.json，可用 CREW_INBOX_STORE 覆寫；最多10000筆、16 artifacts/message、summary/note各512 UTF-8 bytes、key為1..128 ASCII bytes。沿用 file-lock／原子更新，滿時 store_full，v1不自動刪除；不混入 plugin registry。詳見 [B v1](contract/B-V1.md)。
- 保存回覆成功後才能回 stored；程序重啟可恢復訊息／確認／寄件去重記錄。保留期限與清理需明確設定，不默默刪除未讀訊息。
- API 額外提供持久的 inbox cursor；現有 events cursor 只當快速通知，不承擔訊息保存。events_lost 或 epoch 改變時，從 inbox cursor 補讀。
- 可重試的傳輸與去重提供可解釋的至少一次取件；不宣稱模型工作恰好執行一次。

## D：主動喚醒與不打斷工作

- 每端 adapter 提供 `capabilities` 與明確 `input_ready/busy/blocked/offline`，與 Crew 的 working/idle、PTY 安靜程度分開；未回報 ready 就不自動按 Enter。
- 若 CLI/app-server 有已驗證的接收介面，優先以訊息 ID 通知；否則只採單一投遞者的 TUI 相容途徑，並讓 adapter 回報可驗證的 read。
- busy 時保存所有訊息並延後通知；agent 下個可接收時讀取信箱，禁止逐則重送相同提示。首版通知合併延後，待最小 receiver 量測後再做。
- blocked／確認畫面不自動送 y、Esc 或 Ctrl-C；保留訊息並讓使用者看到需要處理。這直接避免把通知打進許可／登入提示。
- 送出前檢查目的 epoch/instance；延後到 ready 時重新解析邏輯 peer。舊占用者退出後，舊的投遞動作不得對新占用者補送 Enter。
- 訊息期限只影響是否繼續通知，不能等同取消對方已接受的工作；取消使用獨立訊息，由收件端確認結果。
- 同類事件去重、有限退避與單一 watcher 即可起步；現有 25 ms 事件輪詢先保留，只有量到 CPU／延遲瓶頸才改通知原語。
- 進度更新、提案、問題、交接、完成各用明確 kind；規則允許在等待審查時提出獨立工作，但不得自動擴權、啟動付費 agent 或替人核准。

## E／F：讓交換更快，也保留相容性

- adapter 可沿用一個 `crew api serve`，用 request ID 接回覆；未來加入 msg wait 時最多保留一個有期限等待，不以大量長等待佔滿目前 16 個並行名額。
- 新增 msg/peer 方法沿用既有 request/reply envelope；先擴充 Crew API，再考慮各 agent 的工具介面，不另造一套互斥協定。
- 向現有事件日誌記錄 `message.stored`、`message.acked`、`peer.changed`，事件只含 ID 與狀態；詳細內容透過信箱補讀。
- 現有 `api snapshot` 沒有同時回 epoch/cursor（[src/api/pane/query.rs:39](../../../src/api/pane/query.rs#L39)）；初始化不能假設它提供無縫事件接點。
- 新 peer/inbox snapshot 應一起回對應游標；舊 API 可先取事件位置、再讀狀態、再處理後續事件，遇遺失就重讀，避免漏掉快照間的變動。
- 輸出檢查點須有 pane 與 epoch，清楚定義按原始輸出或邏輯行編號；先設計語意再補介面，不能拿事件的 bytes 數冒充終端文字游標。
- 即使補了 `--after`，新通知的回顯仍可能符合文字條件；正式交接完成以 message ID 的回覆／ack 為依據，EOT 僅留相容用途。

## 驗收與量測

下列是尚未執行的驗收目標，不是目前效能結果；測試先使用隔離 socket/state 與免費 fixture receiver。

| ID | 驗收條件 | 證據要求 |
|---|---|---|
| AC1 | 更換 pane、重啟 daemon、出現兩個同種類 agent 時，不誤投遞；歧義明確報錯。 | 綁定與重連整合測試、目的 instance 記錄。 |
| AC2 | 寄出成功後殺掉／重啟測試 daemon，未讀訊息、ack、去重結果仍一致。 | 重啟與相同 key 重試測試；不外推為斷電保證。 |
| AC3 | 舊 EOT、通知回顯、超過 512 個事件或事件 epoch 更換，不造成假完成或漏取持久訊息。 | 明確插入舊標記、事件溢位及恢復測試。 |
| AC4 | busy、blocked、輸入框半成品、instance 替換時不亂送 Enter；read 前不顯示已讀。 | 可控制 ready 狀態的 receiver fixture，再做真實兩端驗收。 |
| AC5 | 本機 1,000 則短訊 fixture：stored 回覆 p95 < 100 ms；ready adapter 的通知 p95 < 500 ms。 | 記 OS、機器、併發與 payload；另報模型接單時間，不混算。 |
| AC6 | 相同十次交接中降低人類轉述、額外 CLI 啟動與整頁 terminal read；正確率不下降。 | 與 A 的基準比較；token 節省需真實紀錄，不能用估算當成實績。 |
| AC7 | 未啟用 adapter 時既有 CRL 流程可用；新功能的 --help 無副作用。 | 相容測試、Windows 原生實測、fmt/clippy/test 與獨立審查。 |
| AC8 | Crew 顯示真實 thread title 與當前步驟；替換／過期時清除舊資料；未知總量不出現虛構百分比。 | Codex／Claude 原生 TUI 截圖與狀態切換；另確認 metadata 不改變 wait／ack。 |

## 分工、風險與下一步

- 契約 B 先由一方提出、另一方審查；凍結後可讓 Codex 做 C 的儲存／API，Claude 做 D 的接收 adapter，各自 worktree、檔案所有權不重疊。
- `api::Request`、bridge method 清單與 server 路由由整合者序列修改；測試 agent 可平行做 AC fixture，避免多方同改入口檔。
- 最大未知是 Claude CLI 與 Codex app-server 的實際收件／可接收事件；先做一個受控能力探測，證實後才承諾自動喚醒。
- 最主要風險是把「程序還在／畫面沒輸出」當成 ready，以及把 `sent=true` 當成收件成功；用明確能力、訊息 ID 與分階段 ack 化解。
- artifact 雜湊只檢查內容一致，不能當作授權；訊息內容是工作資料，不能自行授予發佈、刪除或憑證存取權限。
- 持久信箱保存的內容可能含專案資訊；第一版只保存必要摘要與指標，內容範圍、保留期限和清理行為列入契約審查。
- 依原始碼可確認的基礎功能為 PASS；新增信箱、adapter、效能目標及跨平台執行為 NOT_RUN；真正 provider 收件能力與目前延遲基準為 unknown。
- 下一步：雙方對 B 的欄位、ack 語意、身份重綁與 adapter 能力逐項投票，再選 C＋D 的最小端到端實驗。這份計畫不等於實作或部署授權。
