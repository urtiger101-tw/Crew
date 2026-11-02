# Claude Stop receiver：註冊與驗收界線

本文件描述 `scripts/adapters/claude_stop_receiver.py`。設定範本未寫入任何 Claude 設定，亦未啟用 hook。

## 明確設定執行環境

啟動 Claude 的程序必須提供以下環境，receiver 會繼承同一組值：

| 變數 | 設定與用途 |
|---|---|
| `CREW_RECEIVER_NAME` | 既有具名 peer，例如 `crew-claude`；格式 `[a-z][a-z0-9_-]{0,31}`。未設定時允許 Stop。 |
| `CREW_RECEIVER_CREW` | **絕對 Crew 執行檔路徑**。不接受 `crew`、`crew.exe`、相對路徑或 Windows drive-relative 路徑；不搜尋 cwd 或 PATH。 |
| `CREW_RECEIVER_REPO_ROOT` | **明確設定 receiver 自己的絕對 root**，例如 `D:/Crew-agentcmd`。所有 artifact 均在此 root 解析／核對，sender 的 `repo_root` 不作備援。若省略，現有相容行為會使用 hook stdin 的 cwd；多 worktree 場景不要依賴此預設。 |
| `CREW_RECEIVER_STATE` | adapter 自己的 JSON 絕對路徑；保存通知狀態與 ack 失敗計數。不可指向 Crew inbox store。省略時使用平台的使用者 state 目錄。 |
| `CREW_SOCKET` 等 | 明確設定目標 socket 與對應的 `CREW_STATE`、`CREW_INBOX_STORE`、`CREW_CONFIG`、`CREW_PLUGIN_HOME`、`CREW_LOG`；測試一律使用 fixture 建立的唯一隔離值。`CREW_PANE_ID` 繼承對應 caller。 |

隔離驗收可使用已複製的 `D:/Crew/target/crew-coop-20261102-121955/crew-preview.exe`；測試程式另建自己的 socket，不使用合作預覽 socket或預設 daemon。

提供上述環境後，Stop 註冊範本如下。Python 與腳本路徑應改成部署機器上已確認的絕對路徑；**command hook timeout 明確為 45 秒**：

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"C:/Python314/python.exe\" -X utf8 -B \"D:/Crew-agentcmd/scripts/adapters/claude_stop_receiver.py\"",
            "timeout": 45
          }
        ]
      }
    ]
  }
}
```

## 交付與失敗行為

- `stop_hook_active=true` 直接允許停止。格式錯誤 stdin、無效 peer 名稱、未設定／非絕對 Crew 路徑，不啟動 Crew 呼叫。
- 先以 `--unread` 分頁取得待交付訊息，再獨立查 `--open` 顯示 read-but-not-done 數量；只有已讀未完成時允許 Stop，不重新交付。這是目前 receiver 的策略，不表示原工作已恢復或完成。
- 每筆 summary、artifact path 與 metadata 以 JSON 字串呈現。移除 Unicode `Cc/Cf/Zl/Zp/Cs/Co` 字元，換行／tab／NEL 轉空格；每輪產生新 nonce，逐筆包在 `BEGIN/END CREW MESSAGE <nonce> <index>`。這是資料邊界提示，不是模型不受 prompt injection 影響的證明。
- 先組成並 ASCII-safe 編碼單一 block JSON，再 write、flush stdout，**兩步成功後才 ack read**。encode/write/flush 失敗會記 diagnostic，不嘗試任何 read ack；flush 失敗時已寫出的 bytes 無法撤回。receiver 從不自動 ack done。
- read ack 失敗每次計數，正常退出時持久保存，每 ID 上限 **3**。達上限後不再完整交付該訊息，以 stderr 一行 `stuck message "<id>"` 提示人工處理；若只有 stuck 訊息則允許 Stop。失敗訊息仍在 Crew 中 unread，不能當作 done。修復連線／store 後由處理者明確取件、完成工作並依契約確認；adapter 不自行跳過確認步驟。
- adapter state 最多保存 500 筆；遺失、損壞、寫入失敗或被淘汰的紀錄無法維持先前計數。此時可能再交付，會留下適用的診斷；沒有持久 exactly-once 保證。
- 長度或 SHA-256 不符、missing、越界或 Windows 尾端點／空格 alias 路徑會標示狀態，不能作 verified artifact 使用。失敗狀態仍可隨 summary 交付並明確 ack read；read 不代表 artifact 合格。單筆 reason 過長時只保留完整行，保留 nonce 尾界，並標示 `ARTIFACT LINES OMITTED`；使用前必須另取完整 manifest。

## 上限與尚未證明的情況

總工作 deadline 為 **28 秒**，每次 Crew 子程序 timeout 為剩餘預算與 10 秒的較小值。每個 artifact 最多 hash **64 MiB**，先檢查實際檔案大小，並在每個 chunk 讀取前後檢查共用 deadline；過大或逾時標 `UNVERIFIED`。檔案在讀取期間增長也會停止 hash。檔案系統 open/read、stdout flush 或 state 寫入的阻塞無法由此合作式 deadline 強制中斷，外層 45 秒 hook timeout 提供額外界線；未做斷電或慢檔案系統實測。

預設每個 selector 最多 **5 頁 × 20 = 100 headers**。讀取失敗或 stuck 的舊訊息仍占 unread 視窗；若占滿視窗，後面的新訊息可能無法被看到。這是 poison-read 的明確上限，沒有隱藏的跳過、刪除或無界掃描。頁數／大小環境變數只供隔離測試調整。

兩個 Claude sessions 若使用同一 receiver name，可能同時交付同一筆訊息；本 adapter **沒有跨 session 鎖**。各 session 應使用自己的具名 peer 與 state 路徑。atomic state 替換不代表交付互斥。

stdout flush 後、read 保存前中斷，可能重送；read 已保存但 host 尚未接收 reason 時中斷，仍可能未交付給模型。模型理解、真實 hook 啟用、pending composer／approval UI、queue delivery、merge／push，均不能從 synthetic Stop fixture 推得 PASS。

## 執行指定 fixture

```text
python -X utf8 -B scripts/test_claude_stop_receiver.py D:/Crew/target/crew-coop-20261102-121955/crew-preview.exe
```

fixture 使用唯一 socket、臨時設定／state／store，僅停止自己建立的 daemon。unit/fake cases 與真實隔離 daemon cases 的結果須分開記錄；遇到環境阻塞不能把它寫成 receiver PASS。
