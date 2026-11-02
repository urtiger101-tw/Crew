[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Prepare', 'Launch', 'VerifyReceipt')][string]$Mode,
    [Parameter(Mandatory)][string]$ManifestPath,
    [string]$InputPath,
    [string]$CodexPath = 'codex'
)

$ErrorActionPreference = 'Stop'

try {
    if ($Mode -eq 'Prepare' -and [string]::IsNullOrWhiteSpace($InputPath)) {
        throw 'Prepare requires InputPath.'
    }
    $sourcePath = if ($Mode -eq 'Prepare') { $InputPath } else { $ManifestPath }
    $manifest = Get-Content -LiteralPath $sourcePath -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($name in @('coordinator_thread_id', 'worker_name', 'model', 'effort', 'role', 'task_id', 'sandbox_mode', 'nonce', 'report_path', 'working_directory')) {
        if ($manifest.$name -isnot [string] -or [string]::IsNullOrWhiteSpace($manifest.$name) -or $manifest.$name -match '[\r\n\x00]') {
            throw "Invalid manifest field: $name"
        }
    }
    foreach ($name in @('coordinator_thread_id', 'worker_name', 'model', 'task_id', 'nonce')) {
        if ($manifest.$name -cnotmatch '^[A-Za-z0-9._:-]+$') { throw "Invalid manifest token: $name" }
    }
    if ($manifest.effort -cne 'xhigh' -or $manifest.sandbox_mode -cne 'workspace-write') {
        throw 'Only xhigh and workspace-write are supported.'
    }
    if ($null -ne $manifest.worker_thread_id -and ($manifest.worker_thread_id -isnot [string] -or $manifest.worker_thread_id -cnotmatch '^[A-Za-z0-9._:-]+$')) {
        throw 'Invalid worker_thread_id.'
    }
    if ($null -eq $manifest.crew) { throw 'Missing crew object.' }
    foreach ($name in @('binary', 'socket', 'state', 'inbox_store')) {
        if ($manifest.crew.$name -isnot [string] -or [string]::IsNullOrWhiteSpace($manifest.crew.$name) -or $manifest.crew.$name -match '[\r\n\x00]') {
            throw "Invalid crew field: $name"
        }
    }
    foreach ($name in @('config', 'pane_id')) {
        if ($null -ne $manifest.crew.$name -and ($manifest.crew.$name -isnot [string] -or [string]::IsNullOrWhiteSpace($manifest.crew.$name) -or $manifest.crew.$name -match '[\r\n\x00]')) {
            throw "Invalid crew field: $name"
        }
    }
    foreach ($path in @($manifest.report_path, $manifest.working_directory, $manifest.crew.binary, $manifest.crew.state, $manifest.crew.inbox_store) + @($manifest.crew.config | Where-Object { $_ })) {
        if (-not [System.IO.Path]::IsPathFullyQualified($path)) { throw "Path must be absolute: $path" }
    }
    if (-not (Test-Path -LiteralPath $manifest.working_directory -PathType Container)) { throw 'working_directory does not exist.' }
    if ($manifest.owned_files -isnot [array] -or $manifest.owned_files.Count -eq 0) { throw 'owned_files must be a nonempty array.' }
    $reportPath = [System.IO.Path]::GetFullPath($manifest.report_path)
    $workspacePath = [System.IO.Path]::GetFullPath($manifest.working_directory).TrimEnd([char[]]@('\', '/')) + [System.IO.Path]::DirectorySeparatorChar
    if (-not $reportPath.StartsWith($workspacePath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'report_path must be inside working_directory.'
    }
    $ownedPaths = @($manifest.owned_files | ForEach-Object {
        if ($_ -isnot [string] -or [string]::IsNullOrWhiteSpace($_)) { throw 'Invalid owned file.' }
        if ([System.IO.Path]::IsPathFullyQualified($_)) { [System.IO.Path]::GetFullPath($_) }
        else { [System.IO.Path]::GetFullPath((Join-Path $manifest.working_directory $_)) }
    })
    if ($reportPath -notin $ownedPaths) { throw 'report_path must be an owned file.' }

    if ($Mode -eq 'Prepare') {
        $json = $manifest | ConvertTo-Json -Depth 20
        $stream = [System.IO.File]::Open([System.IO.Path]::GetFullPath($ManifestPath), [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
        try {
            $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($json + "`n")
            $stream.Write($bytes, 0, $bytes.Length)
        } finally { $stream.Dispose() }
        [ordered]@{ ok = $true; mode = $Mode; manifest_path = [System.IO.Path]::GetFullPath($ManifestPath) } | ConvertTo-Json -Compress
        exit 0
    }

    $receiptLines = @()
    if (Test-Path -LiteralPath $reportPath -PathType Leaf) {
        $noncePattern = '^RECEIPT nonce=' + [regex]::Escape($manifest.nonce) + '(?:\s|$)'
        $receiptLines = @(Get-Content -LiteralPath $reportPath -Encoding UTF8 | Where-Object { $_ -cmatch $noncePattern })
    }
    if ($Mode -eq 'VerifyReceipt') {
        $expectedThread = if ($manifest.worker_thread_id) { [regex]::Escape($manifest.worker_thread_id) } else { '[A-Za-z0-9._:-]+' }
        $pattern = '^RECEIPT nonce=' + [regex]::Escape($manifest.nonce) + ' worker=' + [regex]::Escape($manifest.worker_name) + ' thread=' + $expectedThread + ' task=' + [regex]::Escape($manifest.task_id) + '$'
        $ok = $receiptLines.Count -eq 1 -and $receiptLines[0] -cmatch $pattern
        [ordered]@{ ok = $ok; mode = $Mode; report_path = $reportPath; matching_nonce_lines = $receiptLines.Count; reason = $(if ($ok) { 'verified' } else { 'missing, duplicate, stale or mismatched receipt' }) } | ConvertTo-Json -Compress
        if ($ok) { exit 0 } else { exit 1 }
    }
    if ($receiptLines.Count -ne 0) { throw 'A receipt for this nonce already exists; use a fresh nonce before launch.' }

    $argsForCodex = @('-a', 'never', '-s', 'workspace-write', '-m', $manifest.model,
        '-C', $manifest.working_directory, '-c', 'model_reasoning_effort="xhigh"',
        '-c', 'shell_environment_policy.inherit="core"')
    $mapping = [ordered]@{
        CREW_SOCKET = 'socket'; CREW_PANE_ID = 'pane_id'; CREW_STATE = 'state'
        CREW_INBOX_STORE = 'inbox_store'; CREW_CONFIG = 'config'; CREW_COOP_BINARY = 'binary'
    }
    foreach ($key in $mapping.Keys) {
        $value = $manifest.crew.($mapping[$key])
        if ($null -ne $value) {
            $tomlString = ConvertTo-Json -InputObject $value -Compress
            $argsForCodex += @('-c', "shell_environment_policy.set.$key=$tomlString")
        }
    }
    $absoluteManifest = [System.IO.Path]::GetFullPath($ManifestPath)
    $prompt = "BOOTSTRAP ONLY. Read the manifest at $absoluteManifest. Append exactly one standalone line to its report_path: RECEIPT nonce=<nonce> worker=<worker_name> thread=<your actual thread id> task=<task_id>, replacing placeholders from the manifest and your actual thread ID. Wait for the coordinator's explicit GO. Do no implementation or other work before GO."
    $argsForCodex += $prompt
    & $CodexPath @argsForCodex
    if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    exit 0
} catch {
    [ordered]@{ ok = $false; mode = $Mode; reason = $_.Exception.Message } | ConvertTo-Json -Compress
    exit 1
}
