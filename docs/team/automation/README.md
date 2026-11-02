# Agent 自動化

Orchestrator 與 Workflow Engine 是前景 CLI，不依賴 Crew daemon。以 `cargo build --locked` 建置後使用 `target/debug/crew.exe`；安裝中的舊 binary 不會自動取得此功能。

## 排程與路由

```powershell
New-Item -ItemType Directory -Force target/verification/automation | Out-Null
target/debug/crew.exe orchestrator plan docs/team/automation/example-orchestrator.windows.json
target/debug/crew.exe orchestrator run docs/team/automation/example-orchestrator.windows.json --execute
target/debug/crew.exe orchestrator status docs/team/automation/example-orchestrator.windows.json
```

此範例使用本機 PowerShell 測試執行器與測試模型，沒有模型 API 呼叫。兩個 agent 按 `capability` 接任務；高 `priority` 先派，平手按 task ID，agent 按設定順序選擇。`depends_on` 全部成功後才可派工。每個 agent 同時一件，總數受 `max_parallel` 限制（1–64）。同一實體 cwd 下有重疊 `write_scopes` 的工作會序列化；失敗／預算不足會阻擋依賴者。

任務 cwd 與 state_file 相對設定檔目錄，cwd 及 state_file 的父目錄須事先存在。請透過現有 `crew worktree create` 為各作者備妥獨立工作樹。`write_scopes` 是 cwd 相對路徑，接受目錄前綴，不接受 `..` 或絕對路徑；scope 是派工協調，不會限制子程序的 OS 權限，也不解析 scope 下的 symlink。

設定欄位見 JSON 範例。模型及價格必須明確填入，單位是 **micro-USD per million tokens**，1 USD = 1,000,000 micro-USD。內建 fallback 對任務預估 token 數選最低價模型，平手按模型 ID。

`router.command` 非空時啟動使用者指定的 executable/argv，stdin 為：

```json
{"task_id":"implement","capability":"code","input_tokens":1000,"output_tokens":2000}
```

stdout 必須僅含 `{"model":"設定中的精確模型ID"}`。非零退出、逾時、未知模型或多餘欄位都會使該任務失敗，不會默默切換 provider。`plan` 僅檢查設定，不執行 router；`run --execute` 才呼叫 router 與 agent。外部路由器自己的費用不納入 token ledger，請在路由器端控管。

agent command 接受 exact-argument placeholders `{model}`、`{prompt}`、`{task_id}`，不在其他字串內展開，不經過 shell。需要 shell 時由設定檔明確指定。stdin 另提供完整 `{"task":{...},"model":"...","agent":"..."}` JSON；adapter 可將它轉成任意 agent CLI 的參數。stdout 必須僅有：

```json
{"usage":{"input_tokens":100,"output_tokens":200},"summary":"實際完成的工作摘要"}
```

stderr 可輸出診斷。原生 CLI 若輸出文字或不同 usage 格式，須加 adapter，不可直接把文字輸出當作成功。排程器不推測任務的正文、驗收結果或 token 使用量。

每次派工前將估價預留並寫入狀態；成功後按回報 usage 與固定價格換算，合計後向上取整。失敗／缺少 usage 時保留估價，`usage:null` 與 note 明確標示未知，不能當成真實帳單或節省證據。預算是**派工預估額度**：實際 token 可超出估算；超額會停止後續派工並回傳非零，已啟動任務可能仍完成。需要模型硬上限時，應由 adapter/provider 同時設定 max tokens 與帳戶上限。

## 狀態與恢復

狀態綁定設定檔 SHA256，已成功工作不重跑。變更設定（含價格或任務）須使用新的 state_file，或恢復原設定。run 在 canonical state_file 上取得 `.lock`，避免兩個命令重複派工；每次更新先寫 `.pending` 再替換原狀態。

中斷時如果存在 running 任務或 stale lock，CLI 會停止，要求先查驗原程序及工作樹。確認所有子程序結束、核對實際輸出與帳單後，才能備份並人工更正該任務狀態／成本及移除 stale lock。不得直接刪除 state 後重跑可能已產生副作用的工作。程序逾時會嘗試終止 adapter 的子程序樹，仍應查驗 adapter 在外部服務已提交的工作。

自動分工依據明確的 task DAG、capability 與 priority；此版本不會從自然語言需求自行產生子任務。支援可觀察、可恢復的前景排程，不是常駐系統服務。

## PR → Review → CI

見 [Workflow Engine 設定與範例](workflow.md)。`advance` 單次推進；`run` 持續輪詢至完成或逾時。GitHub PR 建立／要求 reviewer 需明確執行選項，hosted CI dispatch 另需 `--allow-ci`。引擎不 push、不 merge；現有 repository workflow_dispatch 與 runner minutes 授權條件保留。

僅對設定的 reviewer/check gates 評估 readiness，不代表 GitHub branch protection、merge queue 或部署已通過。參照 GitHub 官方的 [review 回應](https://docs.github.com/en/rest/pulls/reviews#list-reviews-for-a-pull-request)、[check runs](https://docs.github.com/en/rest/checks/runs#list-check-runs-for-a-git-reference) 與 [workflow dispatch](https://docs.github.com/en/rest/actions/workflows#create-a-workflow-dispatch-event)。

本機驗證及 NOT_RUN 邊界記錄於 handoff.json；使用測試 adapter 的證據不代表真實付費模型或 GitHub 寫入已驗收。
