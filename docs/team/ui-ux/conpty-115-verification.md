# §115：ConPTY 殘字與符號輸入驗證

2026-11-02，Windows 10 Pro 19045；本次追加要求收到時，§109 已交付。
本報告補齊 §115 的獨立對照驗證，沒有倒填先前工作的執行順序。

## 結論

同一個 Crew release 執行檔在 app-local Microsoft.Windows.Console.ConPTY
**1.25.260930003** 下，修正本次兩個 CJK 覆寫案例，以及 PSReadLine 2.0.0、
Codex CLI 0.166.0 草稿中的 `→`、`—`、`•` 遺失。使用 Windows 10 inbox runtime
的對照組仍可重現問題。這些案例暫無理由新增 win32-input-mode 編碼或改寫
`pane send-text` 的語意。

既有 release 已包含配對 runtime：
`D:/Crew/target/releases/crew-0.6.0-d66ac1f-windows-x64/`。
必須保留 `crew.exe`、`conpty.dll`、`OpenConsole.exe` 的相鄰配置與授權檔。
portable-pty 0.9.0 已支援 app-local runtime loading，不需要另改 CreatePseudoConsole
載入程式。只有 crew.exe 的部署仍可能使用舊系統 runtime。

## 證據與對照

Crew code commit：`d66ac1ff824929019159eeaafdce7bc92bb3ba60`。
兩個 input arm 的 crew.exe SHA-256 均為
`ecb5aeeb4dd23e97066e7864aa18b2a1fee856dded65aa592783c567fb5fad07`。

| 測試 | Inbox / legacy | Modern |
|---|---|---|
| `ESC[3;1H派工兩件測試\|ESC[3;6HXY` | `派工兩Y件測試\|` | `派工 XY 測試\|` |
| `ESC[5;1H兩兩兩兩\|ESC[5;2H(ab` | `兩ab兩兩\|`（後接空白） | ` (ab兩兩\|` |
| PSReadLine 原始 send-text 草稿 | `ABCD中E\|` | `A→B—C•D中E\|` |
| PSReadLine 實際 eval 碼點 | `65,66,67,68,20013,69,124` | `65,8594,66,8212,67,8226,68,20013,69,124` |
| Codex 原始 send-text 草稿 | `› ABCD中E\|` | `› A→B—C•D中E\|` |

Glyph 使用同一個既有 Claude probe（SHA-256
`6c0c2d7e7339f9832271d75b812211ca546098f4f583decb0e76c48cfd6b488f`），
portable-pty 0.9.0、vt100 0.16.2，10×40 PTY。Legacy 的 raw output 已經損壞；
modern raw 保留原始 CUP 與完整 CJK 操作，vt100 呈現上述正確結果。
兩臂 exit 0、stderr 空、probe 已退出。

Input 使用真正隔離的 Crew daemon 與 `pane send-text`，不是 paste/submit：
先在 PowerShell 讀取 PSReadLine 版本，再輸入
`([int[]][char[]]'A→B—C•D中E|')-join','` 並在該 shell 送 Enter，確認實際碼點。
另開 Codex tab，等待 idle composer 後只送 `A→B—C•D中E|`，讀回 visible。
完整 calls 顯示沒有對 Codex p2 送 Enter；沒有提交該草稿。
兩個獨立 socket 的 cleanup exit 都是 0；未操作共用 daemon。

本 worktree 的持久證據：

- `target/verification/issue115-glyph/summary.json`：兩臂退出碼、來源與執行檔 hashes、raw 及畫面列。
- 同目錄 `legacy/stdout.txt`、`modern/stdout.txt`、`source.rs`、`package-provenance.json`。
- `target/verification/issue115-input/probe.py`、`summary.json`。
- 同目錄 `legacy/`、`modern/`：完整 calls.json、PSReadLine version/draft/evaluated、Codex ready/draft。

## Runtime 與替代方案

NuGet `.nuspec` 實際宣告作者 Microsoft、license **MIT**；package SHA-256：
`02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee`。
Modern pair hashes：

- conpty.dll：`feeef341d891643c62d30b6b07800bc70f0bc148f44cb8c3bee84aa557ae805a`
- OpenConsole.exe：`3d66b23d0a71bb8eed2b77edc8b9df9bf54ce6c8fb74c863a30e760997f80586`

既有 staging script 已驗 Microsoft Authenticode 與固定檔案 hashes，帶入
ConPTY-LICENSE.txt 與來源記錄；不宣稱安裝包已自動整合。

`pane send-text` 直接寫 UTF-8。`paste_data` 是另一條路徑，只有 app 宣告
bracketed paste 才包裝序列，且會正規化換行；不能用它的成功替代 send-text 驗收。
Win32 input 序列亦非普通 Unix PTY 通用文字格式。這次 modern 已通過原始 send-text，
因此保留現有行為，避免為本機舊 runtime 特例改變所有 app 的輸入方式。

## 獨立審查與限制

Sol xhigh 負責 glyph 對照執行；主代理負責 input 測試；未撰寫測試的 Terra xhigh
審查 probe、raw/calls、碼點與草稿畫面。確認指定案例結論有據。
Terra 發現原摘要的 `model_prompt_calls=0` 只是硬編紀錄，不能證明網路活動；
已刪除，改由 calls 計算 `codex_enter_actions=0`，並標記
`model_network_requests=NOT_MONITORED`。原 calls、stdout 與畫面證據未變，沒有重跑測試。

PASS 僅限上述版本、環境、字串與兩個重繪序列。其他 Unicode、其他 app、
Windows Terminal 實體 GUI／IME、完整 agent 對話重繪、網路/API 請求監測、
安裝生命週期均 **NOT_RUN** 或 **NOT_MONITORED**。這不是所有字形問題已消失的保證。
本輪沒有改 production code，也沒有合併或重啟共用 daemon；既有 release binary 不需重建。
