# UPF / scrollback / sidebar 交付紀錄

## 1. Baseline / current state

2026-11-02，Windows 10 Pro 19045，Rust 1.96.1，x86_64-pc-windows-msvc。
基準 `69375220e60ee2238f0567a470187a13ec743a74`；獨立 branch
`fix/scrollback-20261102`，worktree `D:/Crew/target/worktrees/scrollback-20261102`。
原 main 的未提交功能列修正保留，交付分支已包含同一修正。輸入交接包原件未刪除。
既有 integration 24 項、bridge 59 methods，沒有 task/contract method；
本輪採本機 CLI，不改 wire protocol。現況與替代方案詳 ADR。

## 2. Design decisions and architecture

[ADR](task-contract-adr.md) 定義 S/C/I/E/F、V/R、版本、信任邊界、流程圖、
狀態機、失敗與回滾。`crew task validate/store/inspect` 可離線運作。
不新增模型呼叫／provider dependency；沒有私有推理輸出需求。
上下文、例子、指令與 evidence_ref 都是資料，不產生執行或檔案讀取。

## 3. Changes actually implemented

- `src/task_contract.rs`：有界 JSON、strict serde、交叉參照與有序依賴、
  獨立 reviewer 宣告、完整 check 集合、有限 attempt、到期與重複完成檢查。
  create_new 儲存與 SHA-256、唯讀 inspect；不覆寫、不自動建父目錄。
- `src/lib.rs`、CLI help、README：選用 task 命令及 exit/status 契約。
- `tests/task_contract_cli.rs`：真正執行 binary 的七組 acceptance tests。
- `docs/design/` 與 `docs/references/`：設計、範例與使用者參考來源。
- scrollback：無 mouse reporting 時 wheel 進 Crew history；有 reporting
  時正確轉送；alternate-screen 使用 retained main snapshot；未聚焦 pane
  捲動後同步聚焦、可 q/Esc 離開。見 [scrollback 報告](../team/ui-ux/scrollback-fix.md)。
- sidebar：預設 true；明確 false、runtime off、保存 session 值仍保留；
  Aurora toolbar 恢復預設顯示，切換主題尊重工具列狀態。
- `scripts/stage_windows_conpty.ps1`：固定 Microsoft runtime、雜湊、簽章及 MIT
  授權證據。一般 Cargo build 不自動帶 runtime，release candidate 必須另行 stage。

## 4. Test evidence (PASS / FAIL / NOT_RUN)

證據目錄為本 worktree 的 `target/verification/`。

| 檢查 | 結果 | 證據 |
|---|---|---|
| 基準 scrollback regression | RED，2 failed，捕獲3次Up及漏送SGR | scrollback-baseline-red.log |
| 基準 task CLI | RED，0 passed / 7 failed，unknown command: task | Sol 獨立基準執行，target D:/Crew/target/upf-tests-red-build |
| `cargo test --locked --offline --lib` | PASS，589 passed / 0 failed / 3 ignored | sidebar-lib.log |
| `cargo test --locked --offline --test task_contract_cli` | PASS，7 passed | upf-cli.log |
| `CREW_TEST_MODERN_CONPTY=1 cargo test --locked --offline --test pane_scrollback -- --nocapture` | PASS，1 passed / 1 ignored helper；完整alt/SGR/default/UTF8斷言執行 | final-scrollback.log |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | PASS，exit 0 | final-clippy.log |
| `cargo fmt --all -- --check`、`git diff --check` | PASS，exit 0 | 本機工具回傳 |
| 隔離 sidebar daemon / debug-frame smoke | PASS，新session120欄→pane92；off→120；換主題仍120 | sidebar-smoke/result.json、default-on.txt |
| Windows Terminal 真實 GUI、實際 Claude/Codex 對話捲動 | NOT_RUN | 工具未暴露測試視窗；純render不是截圖驗收 |
| Linux/macOS、hosted CI、安裝程式生命週期 | NOT_RUN | 未執行／未授權hosted CI |
| paid API、真實模型品質改善、自動V/R scheduler | NOT_RUN | MVP不實作 |

sidebar smoke 初次把 detached 預設誤認為80欄，第二次發現 toggle-sidebar 需要
attached client；這兩次是 harness 假設錯誤，已改以120欄及 `set sidebar off` 驗證。
GUI toggle 未被這個 headless smoke 驗證。測試只關閉自己的獨立 socket。

## 5. Independent review and follow-up fixes

實際可用：GPT-6.1 Sol xhigh、GPT-6.1 Terra xhigh。主代理擁有 production code、設計、
整合；Sol 在獨立 `test/upf-contract-20261102` worktree 只撰寫 CLI 測試；
Terra 未撰寫 UPF，執行限定 Rust/安全/相容性審查，也獨立審查 Sol 測試。
沒有作者自審或 reviewer chain。

UPF findings：任意位置的 --help 誤回成功、文件 assessment.state 欄位錯誤。
均已修正；七組CLI測試包含兩種 help 負面形狀與 result.state 精確斷言。
sidebar default、明確關閉／持久化與80/120/48欄幾何補審無具體阻斷。
scrollback 的焦點與 Windows runtime 發現也已修正並驗證。

## 6. Risks, blockers, and deferred scope

`declared_pass` 只是 supplied records 推導，`evidence_verified=false` 固定不變。
沒有身份認證、artifact hash外部驗真、secret redaction、模型排程；不應把合法紀錄
當成部署／付款／權限核准。部分寫入失敗會保留新檔並回報錯誤。
vt100 沒保存過去 alternate-screen 重繪，不能重建它；可看已保存的一般畫面。
共用daemon目前仍未更新。本機release build不代表安裝包、production GUI已驗收。

## 7. Git commit / branch / PR status

主人已授權工作完成後 commit、push、build release；不含合併或重啟共用daemon。
本文件隨實作提交；最終 SHA、PR URL、release候選執行檔與雜湊由
`docs/collab/claude-codex/log/110-codex.crl` 及本機release manifest記錄。
既有 hosted workflows 只有手動授權入口，未 dispatch。
