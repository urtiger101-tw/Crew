# Codex 進度顯示同步

`crew integration codex-progress` 是明確指定來源的本機相容橋接。它讀取所選 Codex rollout JSONL 的 assistant commentary，透過現有 presentation metadata 在 Crew 的 working 標籤顯示最近步驟。它不送鍵盤輸入、不呼叫模型，也不修改 agent lifecycle、完成條件或收件身份。

```powershell
crew integration codex-progress --session-file C:\path\to\rollout.jsonl --pane p4 --source codex.progress.own-session --title "Review collaboration task"
```

- 必須指定 `--session-file`、`--pane` 與每個 watcher 唯一的 `--source`；不搜尋或猜測其他 session。
- 目的窗格須已偵測到 Codex，或由擁有者明確回報為 codex。啟動時記錄 daemon epoch、偵測程序 PID 與已回報 instance；每次輪詢重新確認，變更後停止。
- `--once` 只讀一次完整資料並回傳更新數，適合隔離驗收；一般模式每 500 ms 檢查一次。
- 起始只回讀最多 1 MiB；單行上限 256 KiB。未寫完的 JSON 行會等待換行，超長或不相容記錄會跳過。
- 僅使用 `response_item/message` 中 `role=assistant` 且 `phase=commentary` 的 `output_text`；不發布 user prompt、分析、工具參數或工具結果，也不重複採用 `event_msg/agent_message`。
- 發布後的 commentary 會成為 `state_label`，所有連線到該 Crew 的 attached clients 與 federation 查詢者都能看到；它不是只對同步器或目前窗格使用者私有的文字。
- 摘要清除控制字元與雙向文字控制符，最多 72 字元；不根據耗時、spinner 或輸出量推估百分比。
- `task_started`、`task_complete`、`turn_aborted` 與 assistant final_answer 只清除本來源的顯示，並不回報 Crew 工作完成。
- 每次真實 commentary 更新的 presentation 有 60 秒 TTL；沒有新資料便過期回到原顯示。`--title` 也屬於同一份短期 presentation；未指定時不覆蓋名稱。

此 JSONL 格式依本機 Codex 0.166 系列實際記錄支援，並非穩定的跨版本 API。程序對照與 metadata 提交是兩次本機請求，並非原子身份 guard；本工具會在後續檢查停止，agent kind/source guard 與 TTL 限制殘留。更嚴格的 app-server event adapter、明確啟動綁定和原子 instance guard 留待協作計畫的契約階段。

隔離驗收可先在獨立 socket 建立 fixture pane，使用 `integration report codex start --source fixture --pane pN` 建立明確 producer，再以只含測試 commentary 的 JSONL 執行 `--once`；查閱 pane 的 `display.state_label` 與 `display.presentation`，並確認 `agent.instance/state` 未被同步器改寫。真實使用應選擇當前 Codex 實際對應的 rollout；不能把 fixture 驗收當成來源自動識別或雙 agent 完成證據。
