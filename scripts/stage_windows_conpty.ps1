#requires -Version 7.0
<#
Stage the pinned Microsoft ConPTY x64 runtime beside a freshly built crew.exe.
No installation, PATH update, service restart, or shared daemon mutation occurs.
Use -PackagePath for a previously downloaded, hash-verified offline package.
#>
param(
    [Parameter(Mandatory = $true)][string]$Destination,
    [string]$PackagePath
)

$ErrorActionPreference = 'Stop'
$version = '1.25.260930003'
$source = "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/$version/microsoft.windows.console.conpty.$version.nupkg"
$packageHash = '02B07B349AF66D801159BDF9E440D4A1CE78BB951F37FC8609731665AFDAE7EE'
$runtimeFiles = @(
    @{ Name = 'conpty.dll'; Entry = 'runtimes/win-x64/native/conpty.dll'; Sha256 = 'FEEEF341D891643C62D30B6B07800BC70F0BC148F44CB8C3BEE84AA557AE805A' },
    @{ Name = 'OpenConsole.exe'; Entry = 'build/native/runtimes/x64/OpenConsole.exe'; Sha256 = '3D66B23D0A71BB8EED2B77EDC8B9DF9BF54CE6C8FB74C863A30E760997F80586' }
)
$destinationPath = (Resolve-Path -LiteralPath $Destination).Path
$crewPath = Join-Path $destinationPath 'crew.exe'
if (-not (Test-Path -LiteralPath $crewPath -PathType Leaf)) { throw 'Destination must contain the new crew.exe.' }
$crewBytes = [IO.File]::ReadAllBytes($crewPath)
if ($crewBytes.Length -lt 64 -or $crewBytes[0] -ne 0x4d -or $crewBytes[1] -ne 0x5a) { throw 'crew.exe is not a PE executable.' }
$peOffset = [BitConverter]::ToInt32($crewBytes, 0x3c)
if ($peOffset -lt 0 -or $peOffset -gt $crewBytes.Length - 6 -or
    [BitConverter]::ToUInt32($crewBytes, $peOffset) -ne 0x4550 -or
    [BitConverter]::ToUInt16($crewBytes, $peOffset + 4) -ne 0x8664) {
    throw 'This runtime is for x64 crew.exe only.'
}

$cache = Join-Path $PSScriptRoot "../target/conpty/$version"
New-Item -ItemType Directory -Force -Path $cache | Out-Null
if (-not $PackagePath) {
    $PackagePath = Join-Path $cache 'conpty.nupkg'
    if (-not (Test-Path -LiteralPath $PackagePath)) {
        Invoke-WebRequest -Uri $source -OutFile $PackagePath
    }
}
if ((Get-FileHash -LiteralPath $PackagePath -Algorithm SHA256).Hash -ne $packageHash) {
    throw 'ConPTY package SHA-256 differs from the pinned Microsoft package.'
}

$stage = Join-Path $cache ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
$archive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $PackagePath).Path)
try {
    foreach ($file in $runtimeFiles) {
        $entry = $archive.GetEntry($file.Entry)
        if ($null -eq $entry) { throw "Missing runtime entry: $($file.Entry)" }
        $staged = Join-Path $stage $file.Name
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $staged)
        if ((Get-FileHash -LiteralPath $staged -Algorithm SHA256).Hash -ne $file.Sha256) {
            throw "Runtime hash differs: $($file.Name)"
        }
        $signature = Get-AuthenticodeSignature -LiteralPath $staged
        if ($signature.Status -ne 'Valid' -or $null -eq $signature.SignerCertificate -or
            $signature.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false) -ne 'Microsoft Corporation') {
            throw "Runtime Microsoft signature validation failed: $($file.Name)"
        }
        $target = Join-Path $destinationPath $file.Name
        if ((Test-Path -LiteralPath $target) -and
            (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $file.Sha256) {
            throw "Destination contains a different $($file.Name); use a fresh release directory."
        }
    }
} finally {
    $archive.Dispose()
}
# Validate both files before copying either; matching existing files are left in place.
foreach ($file in $runtimeFiles) {
    $target = Join-Path $destinationPath $file.Name
    if (-not (Test-Path -LiteralPath $target)) {
        Copy-Item -LiteralPath (Join-Path $stage $file.Name) -Destination $target
    }
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../docs/licenses/ConPTY-LICENSE.txt') -Destination $destinationPath
@{
    package = 'Microsoft.Windows.Console.ConPTY'; version = $version; source = $source
    package_sha256 = $packageHash; architecture = 'x64'; publisher = 'Microsoft Corporation'
    files = @($runtimeFiles | ForEach-Object { @{ name = $_.Name; sha256 = $_.Sha256 } })
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $destinationPath 'ConPTY-runtime.json') -Encoding utf8NoBOM
Write-Output "Staged Microsoft ConPTY $version beside $crewPath; existing processes were not restarted."
