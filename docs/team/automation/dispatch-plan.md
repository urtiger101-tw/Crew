# Crew 自動化實作範圍

基準：e534f96。主代理負責 Orchestrator、CLI、程序執行與整合；Sol xhigh 在獨立 worktree 負責 router.rs，Terra xhigh 在獨立 worktree 負責 workflow.rs。各作者不得批准自己的修改。保留開始時既有的文件與全域規則變更。

## 驗收

- AC1：`crew orchestrator plan/run/status` 能按 DAG、capability、priority、檔案所有權與並行上限派工；失敗的任務阻擋依賴者；狀態可恢復，已完成任務不重跑。
- AC2：外部路由器以 stdin/stdout JSON 選擇已設定模型；拒絕未知模型；以明確價格及 token 使用量計算整數 micro-USD，啟動前預留預算，缺少 usage 時保留估價並標示未知。
- AC3：`crew workflow plan/advance` 支援 PR 建立、請求獨立 review、觀察 CI 與明確授權的手動 CI dispatch；舊 head 的證據不得通關。失敗／缺少／略過的必要檢查不得視為成功。
- AC4：CLI 可發現、有實際用法與可執行範例；真實本機 adapter／排程測試、模擬 GitHub 回應測試及獨立審查通過。

## 邊界

通用 router 介面已由使用者確認。模型執行器與路由器都是使用者設定的本機 executable + argv，不內建價格猜測、不呼叫付費 provider。執行任務需 `--execute`；GitHub 寫入與 hosted CI 有獨立明確開關。不自動 merge、push、重啟共用 daemon 或修改現有 CI 觸發器。

本次自動分工以已提交的任務 DAG 及 agent capabilities 為依據，不宣稱能從任意自然語言需求自動推導正確的所有子任務。執行器應在已備妥的 worktree 工作；檔案 scope 是排程協調，不是 OS 沙箱。

## 依賴與證據

router 與 workflow 可獨立實作；主代理完成共享 process adapter 與 scheduler 後整合。Sol 審查主代理的 scheduler/process；Terra 審查 Sol router；主代理審查 Terra workflow。測試與命令輸出存於 target/verification/automation，交付用 handoff.json 記錄 PASS、FAIL、NOT_RUN。
