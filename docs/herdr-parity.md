# Crew / Herdr 功能對齊

本次對照固定於 Herdr `143bf5fd4781d164260c63fe28e7dc08054ad32f`（0.9.5），
Crew 由 tmuxlike `12be9e307e47a56d3bfd669ab7bddb1ee7466e09` 演進為 0.3.0。
來源：[Herdr](https://github.com/herdrdev/herdr/tree/143bf5fd4781d164260c63fe28e7dc08054ad32f)
的 README、CLI reference 與 socket API 文件。行為獨立實作，沒有搬入 Herdr 原始碼。

| 功能 | Crew 0.3.0 狀態 | 實作與範圍 |
|---|---|---|
| PTY、背景 daemon、detach/attach | 既有，保留 | Windows ConPTY/Named Pipe、Unix PTY/socket |
| 窗格分割、focus、zoom、read、keys、run、wait-output | 既有，保留 | `src/api/pane/`；原生 tmux 風格 resize 仍可用 |
| Workspace CRUD 與穩定 ID | 本版新增 | `src/workspace.rs`；wN、cwd、env、command、no-focus |
| Tab CRUD 與穩定 ID | 本版新增 | tN、workspace scope、caller current、env 繼承 |
| Git worktree | 本版新增 | list/create/open/remove；保留 branch；拒絕 dirty/active/locked/primary |
| Agent 狀態與等待 | 本版強化 | wait/prompt 綁定 occupant instance；stale 回報不推進等待 |
| Agent integrations | 部分 | generic report/release hooks、registry、PATH doctor、explain；不自動安裝官方 hooks |
| Event stream | 本版新增 | 512 筆 cursor journal、epoch、gap、long polling；CLI 及 JSON-lines bridge |
| JSON socket API | 部分 | Crew 本機長度前綴協定＋JSON-lines bridge；不是 Herdr wire compatibility |
| 版面保存 | 本版強化 | 保存 wN/tN 與配置高水位、cwd、明確 env；相容原 version 1 |
| 狀態 sidebar、blocked/done、繁中、theme | 既有，保留 | 原 TUI 與 API；新功能說明位於 README |
| SSH 多機與遠端 workspace UI | 尚未實作 | 沒有假 remote 命令或本機 fallback |
| Plugin / marketplace | 尚未實作 | 沒有 placeholder install 或自動執行第三方套件 |
| Ghostty / Kitty 進階圖形與鍵盤協定 | 尚未對齊 | 保留 vt100 後端；不是完整圖形終端 |
| Process/agent 自動種類偵測 | 尚未實作 | explicit report 優先，否則僅輸出活動 heuristic |
| Workspace/tab move、metadata TTL 等進階 API | 尚未對齊 | 本版提供已列出的 CRUD 子集 |
| 原生系統通知、歷史畫面、不中斷 server handoff | 尚未對齊 | 本版停止 server 仍會結束程序 |

## 行為邊界

- Workspace/tab ID 跨重啟保留；pane ID 需重新查詢。
- 狀態保存不是程序 checkpoint。任意 startup command 不在還原時重播。
- Agent instance 識別一次占用者，不是每一則 prompt 的 request ID。不要對同一
  agent 同時提交多個 prompt 並把 idle 當作個別 prompt 的完成證明。
- Release 會清除當下回報；舊 reporter 必須停止再回報。沒有跨 release 的 seq tombstone。
- Git remove 的使用中檢查包含已知 workspace/pane cwd，Linux 另查前景程序及 shell cwd。
  Windows/macOS 沒有 live cwd 支援。Git 操作與外部程序切換目錄無法形成原子鎖。
- Git hooks 在 checkout 時停用；Git filters 仍依儲存庫設定執行。Git 子程序沒有額外 timeout。
- integrations doctor 不會啟動付費 agent，也不會把 PATH 中的檔案視為 hook 已安裝。
- 新舊 state 並存時採用新檔後留下選擇標記，避免 clear/quarantine 後復活舊資料。

## 驗收來源

產品測試為 `cargo test`、clippy、format 與 `scripts/*smoke.py` 的實際 PTY/Git 回歸。
工作包與獨立 review 證據在 `docs/team/crew-v0.3.0/`。遠端結果以
[Crew Actions](https://github.com/urtiger101-tw/Crew/actions) 的當次 SHA 為準。
本表描述實作範圍，不代表所有平台已有成功的遠端驗證，也不宣稱百分之百 Herdr 相容。
