# Crew / Herdr 功能對齊

本次對照固定於 Herdr `143bf5fd4781d164260c63fe28e7dc08054ad32f`（0.9.5），
於 2026-10-20 再次查核仍為 upstream HEAD。
Crew 由 tmuxlike `12be9e307e47a56d3bfd669ab7bddb1ee7466e09` 演進；0.5.0 以
已發行 0.4.0 的 `5e7e208bca5bf388aded7631aff0d8846b47363f` 為基礎繼續對齊。
來源：[Herdr](https://github.com/herdrdev/herdr/tree/143bf5fd4781d164260c63fe28e7dc08054ad32f)
的 README、CLI reference 與 socket API 文件。行為獨立實作，沒有搬入 Herdr 原始碼。

| 功能 | Crew 0.5.0 狀態 | 實作與範圍 |
|---|---|---|
| PTY、背景 daemon、detach/attach | 既有，保留 | Windows ConPTY/Named Pipe、Unix PTY/socket |
| 窗格分割、focus、zoom、read、keys、run、wait-output | 既有，保留 | `src/api/pane/`；原生 tmux 風格 resize 仍可用 |
| Workspace CRUD 與穩定 ID | 0.3.0 起提供 | `src/workspace.rs`；wN、cwd、env、command、no-focus |
| Tab CRUD 與穩定 ID | 0.3.0 起提供 | tN、workspace scope、caller current、env 繼承 |
| Workspace／tab 排序 | 0.4.0 起提供 | `workspace move`、`tab move`；0..len 插入槽；保留 PTY、ID、focus、chooser／confirm 目標；排序持久化 |
| 跨 workspace 搬移整個 tab | 本版新增 | `tab move --to-workspace`；保留所有 pane、PTY、PID、agent instance、metadata 與 zoom；空來源 workspace 關閉；同機限定 |
| Pane／workspace metadata tokens | 0.4.0 起提供 | patch／clear、per-key TTL、seq high-water、容量限制、snapshot 與更新／到期事件；不接管 agent 狀態 |
| Metadata presentation 與 sidebar template | 本版新增，子集 | title／display_agent／state_labels、來源 guards、clear／TTL／seq、`$token`／`${token}`／`$$`；TUI 與 API 共用；尚無完整 styled row/grid 模板 |
| Git worktree | 0.3.0 起提供 | list/create/open/remove；保留 branch；拒絕 dirty/active/locked/primary |
| Agent 狀態與等待 | 0.3.0 起強化 | wait/prompt 綁定 occupant instance；stale 回報不推進等待 |
| Agent integrations | 部分 | generic report/release hooks、registry、PATH doctor、explain；加入程序偵測證據；不自動安裝官方 hooks |
| Event stream | 本版擴充 | 512 筆 cursor journal、epoch、gap、long polling；tab transfer、presentation 更新與 `agent.detected` 事件 |
| JSON socket API | 部分 | Crew 本機長度前綴協定＋JSON-lines bridge；不是 Herdr wire compatibility |
| 版面保存 | 本版擴充 | 保存移動後歸屬與排序、wN/tN 與配置高水位、cwd、明確 env；metadata／偵測提示不持久化 |
| Windows Inno Setup 發行 | 0.4.0 起提供，本版支援本機流程 | per-user installer、portable ZIP、全部 14 個原生 lifecycle 檢查；本版須在 Windows 實測後才可標成功 |
| 狀態 sidebar、blocked/done、繁中、theme | 既有，保留 | 原 TUI 與 API；新功能說明位於 README |
| SSH 多機與遠端 workspace UI | 本版新增，子集 | saved machines、原生 SSH、API 路由、合併查詢、健康狀態、重連、鍵盤 selector 與 SSH attach/detach；遠端需 Linux/macOS POSIX shell；尚無 pane 嵌入 |
| Plugin / marketplace | 本版新增，子集 | 兩個可用內建外掛、離線目錄與 GitHub topic 搜尋、inspect／install／link／update、enable／disable、argv actions、config/state、日誌；Crew 自有 manifest；尚無 startup/events/pane renderers |
| Ghostty / Kitty 進階圖形與鍵盤協定 | 尚未對齊 | 保留 vt100 後端；不是完整圖形終端 |
| Process/agent 自動種類偵測 | 本版新增，子集 | Linux 前景程序與有界 script 位置；macOS 原生程序名稱；Windows Toolhelp child tree；explicit report 優先；不是畫面規則或語義狀態偵測 |
| 單個 pane 跨 workspace 搬移、批次 workspace move_block | 尚未對齊 | 本版跨 workspace 搬移的單位是整個 tab；不新增單 pane／批次 workspace 交易 |
| 原生系統通知、歷史畫面、不中斷 server handoff | 尚未對齊 | 本版停止 server 仍會結束程序 |

## 行為邊界

- Workspace/tab ID 跨重啟保留；pane ID 需重新查詢。
- 排序採 Herdr API 的移動前插入槽語意；Crew 新增自己的 CLI，並非聲稱 Herdr
  現有 CLI 有相同子命令。排序 no-op 不產生 moved event。
- Token 與 presentation metadata 均與 agent 語義分離；TTL 到期和 clear 保留
  source 序號高水位。Token／presentation／序號不跨 server restart 保存。
- 模板只做有界字串替換，不執行 shell 或遞迴解讀。顯示文字可以與實際狀態不同；
  顏色、counts、wait 與 prompt 仍以實際狀態及明確 agent instance 為準。
- 搬移不重啟程序，因此已存在程序的環境變數與 cwd 不會被目的 workspace 改寫。
  不支援跨主機 PTY 搬移；遠端資源以 `(machine_id, resource_id)` 區分。
- SSH 路由不會 fallback 到本機、啟動遠端 daemon 或重播失敗命令。Timeout 只終止
  客戶端 transport，已送達遠端的 mutation 可能已完成，需先查詢狀態再決定重試。
- 自動偵測只辨識名稱證據，不能驗證 vendor 身分或任務完成；Windows descendant
  可含背景 child，macOS／Windows 不解析 Node/Python package arguments。
- 外掛執行使用者信任的普通程序，不是 sandbox。需先檢視 manifest 並同意安裝；
  marketplace 搜尋結果不是審核或信任背書。停用不會終止已在執行的 action。
- 狀態保存不是程序 checkpoint。任意 startup command 不在還原時重播。
- Agent instance 識別一次占用者，不是每一則 prompt 的 request ID。不要對同一
  agent 同時提交多個 prompt 並把 idle 當作個別 prompt 的完成證明。
- Release 會清除當下回報；舊 reporter 必須停止再回報。沒有跨 release 的 seq tombstone。
- Git remove 的使用中檢查包含已知 workspace/pane cwd，Linux 另查前景程序及 shell cwd。
  Windows/macOS 沒有 live cwd 支援。Git 操作與外部程序切換目錄無法形成原子鎖。
- Worktree 的 Git hooks 在 checkout 時停用；其 filters 仍依儲存庫設定執行，
  worktree Git 子程序沒有額外 timeout。外掛下載另用隔離 Git config 與有界 timeout。
- integrations doctor 不會啟動付費 agent，也不會把 PATH 中的檔案視為 hook 已安裝。
- 新舊 state 並存時採用新檔後留下選擇標記，避免 clear/quarantine 後復活舊資料。

## 驗收來源

產品測試為 `cargo test`、clippy、format 與 `scripts/*smoke.py` 的實際 PTY/Git 回歸。
工作包與獨立 review 證據在 `docs/team/crew-v0.3.0/`、`docs/team/crew-v0.4.0/`
及 `docs/team/crew-v0.5.0/`。

0.5.0 採用 `scripts/validate_local.py` 與 `scripts/package_local.py`：
固定 required checks、原生 PTY／loopback SSH、exact source／binary／logs hashes；
不呼叫 GitHub Actions，不以假的 CI run ID 替代本機證據。Windows 的本機流程仍
實際執行 Inno build 與全部 14 項安裝 lifecycle 檢查，未執行時明列 `NOT_RUN`。
詳細使用方式見 [本機驗證](local-validation.md)。

歷史 0.3／0.4 結果以 [Crew Actions](https://github.com/urtiger101-tw/Crew/actions)
的當次 SHA 為準。各版本的發行證據不可互相代用。本表描述實作範圍，
不代表所有平台均已驗證，也不宣稱百分之百 Herdr 相容。
