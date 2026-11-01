# CRL — Crew Relay Lingo

Claude Code (`p1`) 與 Codex CLI (`p3`) 透過 Crew 窗格互相溝通所用的精簡語言。
此語言給 agent 用，不求人類可讀；人類版本見 `TRANSCRIPT.md` 與 `SUMMARY.md`。

## 傳輸 Transport

1. 每則訊息是一個檔案：`log/NN-<from>.crl`，`NN` 為兩位數流水號，奇數 Claude、偶數 Codex。
2. 發送方寫好檔案後，經 Crew 通知對方：
   - Claude → Codex：`crew pane run p3 "<短指令>"`（輸入到 Codex 提示列並按 Enter）。
   - Codex → Claude：在自己的窗格輸出單獨一行 `EOT-NN`；Claude 以
     `crew pane wait-output p3 --match EOT-NN` 等待。
3. 訊息檔是唯一真實來源；窗格文字只是通知。

## 語法 Syntax

```
§NN from>to re:MM            # 標頭；re 可省略
OP k=v k=v :: 自由文字       # 一行一句；`::` 之後可為任何語言
...
END                          # 回合結束
```

## 運算子 Operators（v0）

| OP | 意義 | 常用鍵 |
|---|---|---|
| `OBS` | 觀察到的事實 | `ev=`（證據：`f:path:line`、`cmd:...`） |
| `PRO` | 提案 | `id=Pn` `area=cli\|ui\|api\|agent\|doc\|inst` `pri=1..3`（1 最高）`cost=S\|M\|L` |
| `VOT` | 對提案投票 | `Pn` `+`／`-`／`~`（贊成／反對／保留）`why=` |
| `AMD` | 修改提案 | `Pn` 加上要改的鍵 |
| `ASK` / `ANS` | 提問／回答 | `q=Qn` |
| `LEX` | 為本語言新增或修改詞彙 | `w=新詞` `m=意義` |
| `FIN` | 提議結案；列出共識的提案 id | `ids=P1,P3` |
| `END` | 回合結束 | — |

鍵值中的空白改用 `_`。雙方都同意 `FIN` 的同一組 `ids` 時，討論結束。

## 實務教訓 Lessons（v1 新增）

- 完成標記不可原樣寫進通知文字：通知會回顯在對方窗格，`wait-output --match` 會先比對到回顯。改為描述標記（「E,O,T 以 -NN 連接」），並以錨定 regex `^\s*EOT-NN\s*$` 等待。
- 等到標記後仍須確認訊息檔存在且以 `END` 結尾；標記只是通知。
- 標記行可能被 TUI 加上項目符號（如 `• EOT-06`），錨定 regex 要允許前導符號：`^\W*EOT-NN\s*$`（v2）。
- 送進 agent TUI 時，文字與 Enter 分開送（`send-text` → 確認文字已出現 → `send-keys Enter`）。

## 詞彙增修 Lexicon log

`LEX` 被對方 `VOT +` 後即生效，並追加至下表。

| 版本 | 詞 | 意義 | 提出者 | 訊息 |
|---|---|---|---|---|
| v1 | `IMP` | 已在工作樹實作並附證據鍵；**不代表**測試或實機驗收通過 | Claude | §03（Codex §04 附議） |
| v1 | `NR` | 未執行／未驗證；凡缺執行證據的主張必須標註，並指出缺哪一項檢查 | Claude | §03（Codex §04 附議並補充） |

## 變更歷史

| 版本 | 日期 | 內容 | 影響範圍 |
|------|------|------|----------|
| v2 | 2026-11-02 | 標記行允許 TUI 項目符號（EOT-anchor） | 本檔 |
| v1 | 2026-11-02 | 新增 IMP、NR 詞彙與傳輸實務教訓 | 本檔 |
| v0 | 2026-11-02 | 初版，由 Claude 起草 | 本檔 |
