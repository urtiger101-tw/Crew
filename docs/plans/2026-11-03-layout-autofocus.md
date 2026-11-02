# Crew layout / auto-focus 執行 brief

來源：D:/Crew/docs/collab/claude-codex/log/132-claude.crl、134-claude.crl、136-claude.crl。基準c9b72d35cb3eb78398dd122f915baafd11a60f09。原D:/Crew的untracked/logs保留。

## 授權

本機提交可用 urtiger101-tw <urtiger101@gmail.com>，訊息尾加入 Codex (GPT-6.1) <noreply@openai.com> 及 Claude Opus 5.5 <noreply@anthropic.com> 的 Co-Authored-By。只有lead提交。禁止push、live daemon restart、kill-server、終止live pane。用專用測試socket/暫存狀態可啟動測試daemon，但不能連到/改動live daemon；測試須能證明隔離端點，無法保證則不執行並標明NOT_RUN。

## 第一部分：select-layout

D:/Crew-layout，feat/select-layout，Sol xhigh author獨占此工作樹產品src/tests/scripts/docs（除本共享brief和lead交接report）。一位作者一次；其他人只讀。實作132完整規格：tmux5presets與next-layout、prefix Space/M-1..5、API pane layout、JSON/help/docs；只重建layout tree，保留process/paneid/focus/scrollback，明示zoom策略；每pane現有ConPTY resize路徑一次，MAX_DEPTH及snapshot相容；1pane no-op、unknown usageerror。先查鍵衝突。

驗收：1..6pane每個preset、leaf order、ratios與40x10無零size；API/server保留ids/pids的隔離測試、snapshotroundtrip。Rust1.96.1（Crew CI）、fmt、clippy --locked --all-targets -- -D warnings、cargo test --locked；相關local-validation/release-tooling及隔離API smoke按實際風險執行。避免觸碰正式live daemon。handoff寫SHA/命令exit/log/NOT_RUN/rollback，lead獨立整合並交review。

## 第二部分：auto-focus（依賴第一部分本機commit）

Terra xhigh先只讀調查134的status/client/input/UI/config邊界，提供必要文件所有權與接線設計，不改產品；第一部分source frozen並由lead commit後，建立feat/auto-focus（堆疊在feat/select-layout）及新工作樹，再交給Terra獨占產品實作，Sol停止寫Crew。預設off，不改主人config；主人restart時由Claude設定。

規格完整依134：既有status pipeline的Blocked/Done edge；client個別typing guard2500ms、dwell4000ms、copy/menu/confirm/prompt禁止搶焦點；priorityqueue blocked先、同window優先、scopewindow/session/all；drop stale、optout、no-clientno-op；jump只換focus不送keys/text；zoom/last-pane行為需明示；prefixF切換及prefixa手動attention需先查衝突；AF狀態列en/zh-TW。純決策函式與tabletests涵蓋所有guard、edge/level、scope/zoom/mode；隔離fakeagent server測試focus與toast。不得新增status detectionheuristics。

## Review / 交付

獨立非作者Rust reviewer檢查兩部分，具體findings回作者修復；不重複整個專案review。lead取得實際gates並本機提交。log133逐部分追加（auto-focus可同檔），每次以END收尾並WAKE-PEER；交Claude依136自行review/CI/commit/push。live acceptance、Linux/macOS或未跑gates明列NOT_RUN；新binary不會自動替換live daemon。
