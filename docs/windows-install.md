# Windows 安裝、升級與卸載

Crew 0.4.0 的 Windows 發行包含兩種檔案：

| 檔案 | 用途 |
| --- | --- |
| `Crew-v0.4.0-x86_64-pc-windows-msvc-setup.exe` | Inno Setup 安裝包，安裝給目前使用者，提供開始功能表捷徑與卸載入口 |
| `Crew-v0.4.0-x86_64-pc-windows-msvc.zip` | Portable ZIP，自行解壓縮與管理；保留原有發布方式 |

從 [Crew Releases](https://github.com/urtiger101-tw/Crew/releases) 下載對應版本。
本安裝包限 **x64 Windows 10 1809 或更新版本，包括 x64 Windows 11**。
32-bit Windows、Windows ARM64 與其他架構不在此安裝包的支援範圍。

## 安裝與啟動

執行 `setup.exe`，選擇使用者可寫入的安裝目錄。預設位置是：

```text
%LOCALAPPDATA%\Programs\Crew
```

安裝程式不要求系統管理員權限，不安裝服務，不自動啟動 Crew，
也不修改使用者或系統的 `PATH`。完成後可從開始功能表的 **Crew → Crew** 啟動，
或在 PowerShell 執行：

```powershell
& "$env:LOCALAPPDATA\Programs\Crew\crew.exe"
& "$env:LOCALAPPDATA\Programs\Crew\crew.exe" --version
& "$env:LOCALAPPDATA\Programs\Crew\tmuxlike.exe" --version
```

`tmuxlike.exe` 是相容命令，與 `crew.exe` 一起安裝。選擇自訂目錄時，請替換上述路徑。

若希望在任意新 terminal 直接輸入 `crew` 或 `tmuxlike`，請自行在 Windows
「編輯帳戶的環境變數」中，將安裝目錄新增到**使用者**的 `Path`，然後開啟新 terminal。
也可以只對目前 PowerShell 工作階段加入路徑：

```powershell
$env:Path = "$env:LOCALAPPDATA\Programs\Crew;$env:Path"
crew --version
```

上面的 PowerShell 指令不會永久更改環境變數。安裝包不會替你加入 PATH，
卸載時也不會更動手動新增的項目；若不再需要，請自行移除那一項。

## 升級與正在執行的工作

執行新版安裝包即可沿用既有安裝目錄與卸載入口。安裝身分固定為
`urtiger101-tw.Crew`，不隨版本改變。

若 Crew 的執行檔仍由 daemon 或其他程式使用，安裝與卸載會停止，請先自行保存工作、
關閉使用中的 Crew，再重試。安裝包不會自動關閉、強制終止或重新啟動你的 daemon。
如要主動停止 Crew server，可以在確認工作已保存後自行執行 `crew server stop`；
這會停止該 server 及其正在執行的程式，不能當成不中斷工作的升級方式。

升級保留使用者設定與狀態，包括 `CREW_CONFIG`、`CREW_STATE` 指向的檔案、
Crew 的 AppData 資料，以及相容模式使用的舊 tmuxlike 設定。安裝包只處理列明的
程式、README、LICENSE、package manifest 與自己的卸載資料，不刪除工作樹或專案。

未註冊為 Crew 安裝、但已有檔案的目的目錄會被拒絕，避免覆寫原有資料。
若原本使用 portable ZIP，請選擇另一個空目錄安裝；設定與狀態仍依 Crew 原本的
設定搜尋規則使用，安裝包不會搬移或刪除 portable 目錄。

安裝包拒絕直接降版。若確實需要回到舊版，先備份設定與狀態，再卸載目前程式，
然後安裝或解壓縮所需版本。程式回退不代表資料格式也已回退。

## 卸載

使用 Windows「已安裝的應用程式」中的 Crew 卸載入口，或執行安裝目錄中的
`unins000.exe`。執行檔仍被使用時，卸載會先拒絕並保留安裝內容。

卸載會移除安裝的程式、文件、manifest、開始功能表捷徑及卸載註冊，
保留設定、狀態、專案及其他非本安裝包建立的檔案。若安裝目錄中有你自己的檔案，
該目錄可能保留下來；這是預期行為。

## Silent 安裝與卸載

以下指令不會自動重新啟動 Windows；`Start-Process -Wait -PassThru` 可以取得真實退出碼。
只有退出碼 `0` 表示成功，要求重新開機或忙碌檔案導致的非零退出碼都應處理後重試。

```powershell
$setup = '.\Crew-v0.4.0-x86_64-pc-windows-msvc-setup.exe'
$result = Start-Process -FilePath $setup -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/RESTARTEXITCODE=3010'
)
$result.ExitCode

$uninstall = "$env:LOCALAPPDATA\Programs\Crew\unins000.exe"
$result = Start-Process -FilePath $uninstall -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
)
$result.ExitCode
```

不要加入 `/CLOSEAPPLICATIONS`、`/FORCECLOSEAPPLICATIONS`、`/RESTARTAPPLICATIONS`
或 `/ALLUSERS`；Crew 安裝包會拒絕這些與安裝契約衝突的選項。

## 校驗與驗收範圍

Crew 執行檔與產生的安裝包目前都**未作 Authenticode 簽章**。Windows 可能顯示未知發行者；
Inno Setup 編譯工具本身的簽章不會轉移到 Crew 安裝包。
發布頁提供 `SHA256SUMS` 與 `release-manifest.json`，涵蓋四份 portable archive、
Windows installer，以及固定的來源 SHA、CI run／attempt 和安裝驗收紀錄。
可用以下命令核對下載檔案的 SHA-256：

```powershell
Get-FileHash .\Crew-v0.4.0-x86_64-pc-windows-msvc-setup.exe -Algorithm SHA256
```

Windows 0.4.0 發行建置使用靜態 MSVC CRT，封裝與安裝 smoke 都檢查實際 PE imports，
拒絕額外的 VC runtime DLL 依賴。安裝包不會下載或安裝 Visual C++ Redistributable。
這項檢查仍會保留正常 Windows 系統 DLL 依賴，不等同完整乾淨 VM 或每一版 Windows 的實測。

CI 與正式 release 的 Windows runner 必須實際編譯 `.iss`，並執行全新安裝、
兩個命令與檔案雜湊驗證、真正的舊版升級、忙碌程式拒絕、降版拒絕，以及卸載測試。
PATH 的值及 registry 型別、設定／狀態與額外檔案也必須保持不變。
任何必要項目缺少、跳過或失敗都不能成為可發布的 installer。

升級 fixture 使用已發布且固定 digest 的 **0.3.0 portable ZIP 中的真實執行檔**，
由目前 `.iss` 建立測試用 0.3.0 installer，再升級到本次程式版本。
這個 fixture **不是過去公開發布的 installer**。0.3.0 的舊程式使用動態 CRT，
升級測試是在已提供該 runtime 的 Windows CI runner 執行。

原生 Windows 執行與互動式安裝畫面、SmartScreen 顯示、其他 Windows 版本及完整乾淨 VM
驗證是不同層級；請以對應 release 的實際 CI 結果為準。

## 維護者：固定的 Inno Setup 工具

使用官方 immutable release 的 **Inno Setup 7.1.0 x64**：

- 下載：<https://github.com/jrsoftware/issrc/releases/download/is-7_1_0/innosetup-7.1.0-x64.exe>
- SHA-256：`0362a383ed217d4c4239b5933866dd96d3eb2102737da92f80f6057a4b40df2f`
- 檔案大小：14,304,168 bytes。
- 下載後先檢查固定 SHA-256，以及 Windows Authenticode `Valid`／`Pyrsys B.V.`，再執行。
- 編譯工具以官方 `/PORTABLE=1 /CURRENTUSER` 模式放在 runner 的暫存目錄，停用捷徑與檔案關聯。

官方 [Inno Setup License](https://jrsoftware.org/files/is/license.txt) 允許商業及非商業用途，
並附保留版權資訊等條件；官方網站另有支持專案的商業授權購買請求。
此建置流程不會購買授權或改變帳戶、費用及全域權限。

參考來源：

- [官方下載與目前版本](https://jrsoftware.org/isdl.php)
- [官方簽章及下載驗證說明](https://jrsoftware.org/isdl-verify.php)
- [PrivilegesRequired](https://jrsoftware.org/ishelp/topic_setup_privilegesrequired.htm)
- [AppId 與升級身分](https://jrsoftware.org/ishelp/topic_setup_appid.htm)
- [CloseApplications 與 silent 行為](https://jrsoftware.org/ishelp/topic_setup_closeapplications.htm)
- [Rust 靜態 CRT](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes)
- [Cargo target flags 與 host proc-macro](https://doc.rust-lang.org/cargo/reference/config.html#buildrustflags)
