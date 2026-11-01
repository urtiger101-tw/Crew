# 讓 Crew 更好用：Claude × Codex 討論結論

> 給人看的版本。逐則對話見 `TRANSCRIPT.md`，原始訊息見 `log/`，兩個 agent 自創的溝通語言見 `PROTOCOL.md`。
>
> 參與者：Claude Code（Crew 窗格 p1）與 Codex CLI v0.166.0（窗格 p3），在同一個 Crew 視窗左右並排，共交換 8 則訊息。

## 本次已修好並提交

| # | 你遇到的問題 | 原因 | 修正 | 驗證 |
|---|---|---|---|---|
| P8 | 看不到 Codex 有反應 | `pane run` 貼上文字後只等 30ms 就按 Enter，Codex 把它當成貼上內容裡的換行，指令卡在輸入框沒送出 | 窗格裡是 agent（已回報或已偵測到）時改等 150ms，與 `agent prompt` 相同 | 單元測試通過；Codex 審查無問題；實際送進 Codex 尚未驗證，因為執行中的伺服器還是舊版 |
| P9 | Crew 裡沒有顏色 | 每個窗格都繼承了伺服器環境裡的 `NO_COLOR=1`，也沒有設定 `COLORTERM` | 新窗格設定 `COLORTERM=truecolor`，並移除繼承來的 `NO_COLOR`；仍可用 `--env NO_COLOR=1` 讓個別窗格不上色 | 在另開的獨立伺服器實測通過；Codex 審查無問題 |
| P10 | 想在窗格裡往回看先前的輸出 | 原本要先按前綴鍵進入複製模式，或使用滑鼠滾輪 | 不必按前綴鍵：`Shift+↑／↓` 逐行、`Shift+PgUp／PgDn` 整頁；捲回底部自動結束；全螢幕程式（vim 等）照常收到按鍵；可用 `set scroll-keys off` 關閉 | Codex 審查時抓到一個漏洞（捲動中切換到全螢幕程式時，按鍵會被吃掉），已修正並補測試；尚未在實際終端機手動按鍵驗收 |

不加 Shift 的 `↑／↓` 刻意保留給 shell 叫出指令歷史，所以不拿來捲動。

**要讓這些修正生效，需要重啟 Crew 伺服器。** 重啟會關閉目前所有窗格，包括這個 Claude 與 Codex。可先用 `crew kill-server` 關閉，重新編譯 release 版後再開啟。

## 雙方同意的後續建議（尚未實作，依優先順序）

1. **P1 安裝與診斷**：`crew doctor` 回報實際執行的是哪個執行檔、客戶端與伺服器版本、socket、hook 狀態；另提供明確的「加入 PATH」動作，並提醒已開啟的 shell 不會自動更新。
2. **P2 開窗格時直接指定指令**：`new-window`、`split-window`、`pane split` 都能加 `--command`，行為與 `tab create --command` 相同，並回傳新窗格 ID。
3. **P5 agent 信箱**（Codex 認為對 agent 最有價值）：`crew msg send|wait|read`，具備寄件者、收件者、訊息 ID、回覆 ID、從檔案讀入內文、讀取游標與逾時；不再靠把字打進對方 TUI、擷取畫面來溝通。同時讓 `agent prompt` 也能用於只靠程序偵測認出的 agent，目前會回 `agent_not_found`。
4. **P7 輸出檢查點**：`wait-output --after <檢查點>`，避免舊輸出或回顯的文字讓等待提早成立。
5. **P3 搬移窗格**：`join-pane`／`break-pane`，把窗格連同執行中的程式一起搬到別的視窗，ID、程序、工作目錄都不變。
6. **P4 一鍵並排兩個 agent**：`crew pair <指令A> <指令B> [--layout h|v]`，先做好 P2 後再做。

已撤回：P6（`--workspace` 接受 session 名稱）。Codex 查證後發現程式碼原本就支援。

## 兩個 agent 學到的事

- 完成標記不能原樣寫進通知：通知文字會回顯在對方窗格，提早滿足等待條件。
- TUI 會在標記前加上項目符號（`• EOT-06`），等待的 regex 要容許這個前綴。Claude 一度以為 Codex「停住」，其實 Codex 早已完成。
- 「已實作」（`IMP`）不等於「已驗收」，沒有實際執行證據的說法一律標 `NR`（未驗證）。這兩個詞是雙方協議後加進語言的。

## 變更歷史

| 版本 | 日期 | 內容 | 影響範圍 |
|------|------|------|----------|
| v1 | 2026-11-02 | 定稿：P8–P10 修正結果、後續建議、經驗 | 本檔 |
| v0 | 2026-11-02 | 初稿 | 本檔 |
