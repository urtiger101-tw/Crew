# Metadata tokens

Crew 0.4.0 可以在 pane 或 workspace 保存短文字標記，例如任務摘要、建置進度或
分支名稱。標記可以自行到期。它們不會改變 agent 的 `working`、`blocked`、`idle`
狀態、occupant instance，或讓 `agent wait`／`agent prompt --wait` 提前完成。

```sh
crew workspace report-metadata w1 --source build:ci --token build=running --seq 1 --ttl-ms 60000
crew pane report-metadata p1 --source hook:task --token "task=修正登入流程" --token progress=3/5 --seq 10
crew pane report-metadata p1 --source hook:task --clear-token progress --seq 11
crew workspace get w1
crew pane get p1
```

`--token KEY=VALUE` 與 `--clear-token KEY` 可重複使用。每個請求只能提到同一鍵
一次；沒有提到的鍵維持原值及原期限。空白或經正規化後為空的值等同清除。
同一個資源共用一份 token map，不同來源寫入同一鍵時，最後接受的更新生效。

## 限制與期限

| 項目 | 契約 |
|---|---|
| Source | 1–80 個 ASCII 字母、數字、`:`、`.`、`_`、`-` |
| Token 名稱 | 1–32 個 ASCII 字母、數字、`_`、`-` |
| Token 值 | 去除前後空白與控制字元，最多 80 個 Unicode 字元 |
| 單次更新 | 1–16 個不同鍵 |
| 每個 pane／workspace | 最多 32 個同時有效的鍵 |
| `--ttl-ms` | 1–86400000 毫秒；省略則保留到覆寫、清除、關閉或重啟 |
| 有序來源 | 每個資源生命週期最多 32 個不同的 sequenced sources |

TTL 使用單調時鐘，每個鍵分別計時。刷新某一鍵不會延長其他鍵的期限；覆寫時
省略 TTL 會取消該鍵的舊期限。讀取回應立即排除到期值，背景計時迴圈負責清理
並發出到期事件，無須等待終端產生輸出。事件通常在下一個約 500 毫秒的 server
tick 發出；繁忙時可能較晚。

`--seq` 是非負整數。相同 source 的序號必須大於上次接受的序號；舊的或相同的
序號回傳 `accepted: false`、`changed: false`，不改值、不刷新 TTL、不發更新事件。
清除或到期都保留序號高水位，因此延遲回報不會使舊標記復活。

每個來源自行維護序號；不同來源之間沒有全域排序。省略 `--seq` 的更新仍然可用，
但不享有序號排序保護。無效輸入或超過容量的整份 patch 都會被拒絕，不會先套用
其中幾個鍵，也不會消耗該次序號。

Token 與序號只存在於執行中的 server。關閉 pane／workspace 會清除對應資料；
冷啟動恢復版面時不恢復 token，也不保留上次 server 的序號。

## API 與事件

`pane get/list`、`agent get/list`、`workspace get/list` 及 `api snapshot` 的資源
物件會包含 `tokens` map。成功回報回傳 `accepted`、`changed` 及最新的 `pane`
或 `workspace`。只有實際變更或到期才會發出：

- `pane.metadata_updated`
- `workspace.metadata_updated`

事件的 `data` 包含 `reason`（`report` 或 `expired`）、`changed_keys`，以及最新
資源 snapshot。清除資源本身使用既有 close event；不額外發空的 metadata event。

JSON-lines bridge 接受與 CLI 相同的獨立參數，沒有 shell 展開：

```json
{"id":"build-1","method":"workspace.report-metadata","params":{"args":["w1","--source","build:ci","--token","build=running","--seq","1","--ttl-ms","60000"]}}
```

Crew 使用 `crew-json-lines-v1` 的 CLI argument array；這不是 Herdr 的原生 socket
協定。此版對齊 Herdr 0.9.5 的 **token 子集**，未提供 presentation title、display
agent、state label 覆寫，也未新增 sidebar token 模板。既有 agent 狀態顯示照常運作。

參考行為：[Herdr 0.9.5 socket API metadata](https://github.com/herdrdev/herdr/blob/143bf5fd4781d164260c63fe28e7dc08054ad32f/docs/versions/0.9.5/website/src/content/docs/socket-api.mdx)。
Crew 為獨立實作。
