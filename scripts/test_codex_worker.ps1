$ErrorActionPreference = 'Stop'
$worker = Join-Path $PSScriptRoot 'collab/codex_worker.ps1'
$pwshPath = (Get-Process -Id $PID).Path
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('crew-worker-test-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot
$literalDir = Join-Path $testRoot "space 'quote'"
$null = New-Item -ItemType Directory -Path $literalDir
$inputPath = Join-Path $testRoot 'input.json'
$manifestPath = Join-Path $testRoot 'manifest.json'
$absentPath = Join-Path $testRoot 'absent-thread.json'
$reportPath = Join-Path $literalDir 'report file.md'
$fakePath = Join-Path $testRoot 'fake-codex.ps1'
$capturePath = Join-Path $testRoot 'arguments.json'
$env:FAKE_CODEX_CAPTURE = $capturePath

function Assert-True([bool]$condition, [string]$message) {
    if (-not $condition) { throw "FAIL: $message" }
    Write-Host "PASS: $message"
}

try {
    $manifest = [ordered]@{
        coordinator_thread_id = 'coordinator-1'; worker_name = 'crew-test'; model = 'gpt-6.1-terra'; effort = 'xhigh'
        role = 'test worker'; task_id = 'task-1'; owned_files = @($reportPath)
        crew = [ordered]@{ binary = (Join-Path $literalDir 'preview.exe'); socket = 'isolated.sock'; state = (Join-Path $literalDir 'state.json'); inbox_store = (Join-Path $literalDir 'inbox.json'); config = (Join-Path $literalDir 'config.json'); pane_id = 'p2' }
        sandbox_mode = 'workspace-write'; nonce = 'fresh-nonce'; report_path = $reportPath; working_directory = $literalDir; worker_thread_id = 'thread-1'
    }
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $inputPath -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode Prepare -InputPath $inputPath -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -eq 0 -and ($output | ConvertFrom-Json).ok) 'Prepare validates and creates manifest'
    $before = [System.IO.File]::ReadAllBytes($manifestPath)
    $output = & $pwshPath -NoProfile -File $worker -Mode Prepare -InputPath $inputPath -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -ne 0 -and [Convert]::ToBase64String($before) -ceq [Convert]::ToBase64String([System.IO.File]::ReadAllBytes($manifestPath))) 'Prepare never overwrites existing manifest'
    $invalid = $manifest | ConvertTo-Json -Depth 10 | ConvertFrom-Json
    $invalid.effort = 'low'
    $invalid | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $inputPath -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode Prepare -InputPath $inputPath -ManifestPath (Join-Path $testRoot 'invalid.json')
    Assert-True ($LASTEXITCODE -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $testRoot 'invalid.json'))) 'invalid manifest rejected before write'
    foreach ($badThread in @($false, 0, '', ' ')) {
        $invalid = $manifest | ConvertTo-Json -Depth 10 | ConvertFrom-Json
        $invalid.worker_thread_id = $badThread
        $invalid | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $inputPath -Encoding UTF8
        $output = & $pwshPath -NoProfile -File $worker -Mode Prepare -InputPath $inputPath -ManifestPath (Join-Path $testRoot 'invalid.json')
        Assert-True ($LASTEXITCODE -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $testRoot 'invalid.json'))) "invalid thread rejected: [$badThread]"
    }
    $withoutThread = $manifest | ConvertTo-Json -Depth 10 | ConvertFrom-Json
    $withoutThread.PSObject.Properties.Remove('worker_thread_id')
    $withoutThread | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $inputPath -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode Prepare -InputPath $inputPath -ManifestPath $absentPath
    Assert-True ($LASTEXITCODE -eq 0 -and ($output | ConvertFrom-Json).ok) 'absent expected thread accepted for discovery'
    $outside = $manifest | ConvertTo-Json -Depth 10 | ConvertFrom-Json
    $outside.report_path = Join-Path ($literalDir + '-sibling') 'report.md'
    $outside.owned_files = @($outside.report_path)
    $outside | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $inputPath -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode Prepare -InputPath $inputPath -ManifestPath (Join-Path $testRoot 'invalid.json')
    Assert-True ($LASTEXITCODE -ne 0 -and -not (Test-Path -LiteralPath (Join-Path $testRoot 'invalid.json'))) 'sibling-prefix report path rejected'

    $fake = @'
@($args) | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $env:FAKE_CODEX_CAPTURE -Encoding UTF8
'@
    Set-Content -LiteralPath $fakePath -Value $fake -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode Launch -ManifestPath $manifestPath -CodexPath $fakePath
    Assert-True ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $capturePath)) 'fake Codex launch succeeds without a model'
    $captured = @(Get-Content -LiteralPath $capturePath -Raw -Encoding UTF8 | ConvertFrom-Json)
    Assert-True (($captured -contains '-a') -and ($captured -contains 'never') -and ($captured -contains 'workspace-write') -and ($captured -contains 'gpt-6.1-terra')) 'launch selects approval, sandbox and model'
    Assert-True (($captured -ccontains $literalDir) -and ($captured[-1].Contains($manifestPath))) 'arguments preserve literal quoted paths'
    $settings = @($captured | Where-Object { $_ -clike 'shell_environment_policy.set.*' })
    Assert-True ($settings.Count -eq 6 -and ($captured -ccontains 'shell_environment_policy.inherit="core"') -and ($captured -ccontains 'model_reasoning_effort="xhigh"')) 'exactly six Crew settings and core inheritance are passed'
    $expectedState = 'shell_environment_policy.set.CREW_STATE=' + (ConvertTo-Json -InputObject $manifest.crew.state -Compress)
    Assert-True ($settings -ccontains $expectedState) 'Crew path is passed as a literal TOML string'
    Assert-True (($captured[-1] -match 'BOOTSTRAP ONLY') -and ($captured[-1] -match 'Wait for the coordinator') -and ($captured[-1] -match 'Do no implementation')) 'initial prompt requires receipt then GO'

    $output = & $pwshPath -NoProfile -File $worker -Mode VerifyReceipt -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -ne 0 -and -not ($output | ConvertFrom-Json).ok) 'missing receipt rejected'
    Set-Content -LiteralPath $reportPath -Value 'RECEIPT nonce=old-nonce worker=crew-test thread=thread-1 task=task-1' -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode VerifyReceipt -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -ne 0) 'stale nonce rejected'
    Set-Content -LiteralPath $reportPath -Value 'RECEIPT nonce=fresh-nonce worker=crew-test thread=wrong task=task-1' -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode VerifyReceipt -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -ne 0) 'wrong thread rejected'
    Set-Content -LiteralPath $reportPath -Value 'RECEIPT nonce=fresh-nonce worker=other thread=thread-1 task=task-1' -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode VerifyReceipt -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -ne 0) 'wrong worker rejected'
    Set-Content -LiteralPath $reportPath -Value 'RECEIPT nonce=fresh-nonce worker=crew-test thread=thread-1 task=other' -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode VerifyReceipt -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -ne 0) 'wrong task rejected'
    Set-Content -LiteralPath $reportPath -Value 'RECEIPT nonce=fresh-nonce worker=crew-test thread=discovered-thread task=task-1' -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode VerifyReceipt -ManifestPath $absentPath
    Assert-True ($LASTEXITCODE -eq 0 -and ($output | ConvertFrom-Json).ok) 'absent expected thread accepts discovered thread token'
    $receipt = 'RECEIPT nonce=fresh-nonce worker=crew-test thread=thread-1 task=task-1'
    Set-Content -LiteralPath $reportPath -Value $receipt -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode VerifyReceipt -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -eq 0 -and ($output | ConvertFrom-Json).ok) 'exact receipt verified'
    $output = & $pwshPath -NoProfile -File $worker -Mode Launch -ManifestPath $manifestPath -CodexPath $fakePath
    Assert-True ($LASTEXITCODE -ne 0) 'launch rejects reused nonce receipt'
    Add-Content -LiteralPath $reportPath -Value $receipt -Encoding UTF8
    $output = & $pwshPath -NoProfile -File $worker -Mode VerifyReceipt -ManifestPath $manifestPath
    Assert-True ($LASTEXITCODE -ne 0) 'duplicate receipt rejected'
} finally {
    Remove-Item Env:FAKE_CODEX_CAPTURE -ErrorAction SilentlyContinue
    foreach ($path in @($inputPath, $manifestPath, $absentPath, $reportPath, $fakePath, $capturePath)) {
        if (Test-Path -LiteralPath $path -PathType Leaf) { Remove-Item -LiteralPath $path }
    }
    Remove-Item -LiteralPath $literalDir
    Remove-Item -LiteralPath $testRoot
}
