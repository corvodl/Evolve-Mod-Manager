# Download pinned upstream GFX utilities for optional Inno Setup packaging.
# Vendor binaries are not checked into the repo or the portable ZIP.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$target = Join-Path $root 'dist/third_party'
$tempBase = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$temp = Join-Path $tempBase 'evolve-gfx-tool-downloads'
New-Item -ItemType Directory -Path $temp -Force | Out-Null
New-Item -ItemType Directory -Path $target -Force | Out-Null

function Get-VerifiedArchive {
    param([string] $Uri, [string] $ExpectedHash, [string] $Filename)
    $archive = Join-Path $temp $Filename
    Invoke-WebRequest -Uri $Uri -OutFile $archive -MaximumRedirection 5
    $received = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    if ($received -ne $ExpectedHash.ToLowerInvariant()) {
        throw "Upstream GFX tool SHA256 mismatch: $Filename. Packaging refused."
    }
    $bytes = (Get-Item -LiteralPath $archive).Length
    if ($bytes -lt 1MB -or $bytes -gt 55MB) {
        throw "Unexpected upstream GFX tool download size ($bytes bytes): $Filename"
    }
    Write-Host "$Filename verified: $([math]::Round($bytes / 1MB, 1)) MiB compressed"
    return $archive
}

$ffdecZip = Get-VerifiedArchive -Uri 'https://github.com/jindrapetrik/jpexs-decompiler/releases/download/version26.3.0/ffdec_26.3.0.zip' -ExpectedHash '35f4930eb7c380afe66f2117f90b006deac0631473ad7500bb39c78f68645ecd' -Filename 'ffdec_26.3.0.zip'
$ruffleZip = Get-VerifiedArchive -Uri 'https://github.com/ruffle-rs/ruffle/releases/download/v0.7.1/ruffle-0.7.1-windows-x86_64.zip' -ExpectedHash 'a5f6df9881771226e330e08a1fa7666f4caff50ddf9f2f993cdd95293edca627' -Filename 'ruffle-0.7.1-windows-x86_64.zip'
$ruffleWebZip = Get-VerifiedArchive -Uri 'https://github.com/ruffle-rs/ruffle/releases/download/v0.7.1/ruffle-0.7.1-web-selfhosted.zip' -ExpectedHash 'ef12edbee135d581a9cd523837d746da30a07a64dbf1cf472d2e472c656f4b5e' -Filename 'ruffle-0.7.1-web-selfhosted.zip'

$ffdecExtracted = Join-Path $temp 'ffdec'
$ruffleExtracted = Join-Path $temp 'ruffle'
$ruffleWebExtracted = Join-Path $temp 'ruffle-web'
if (Test-Path $ffdecExtracted) { Remove-Item $ffdecExtracted -Recurse -Force }
if (Test-Path $ruffleExtracted) { Remove-Item $ruffleExtracted -Recurse -Force }
if (Test-Path $ruffleWebExtracted) { Remove-Item $ruffleWebExtracted -Recurse -Force }
Expand-Archive -LiteralPath $ffdecZip -DestinationPath $ffdecExtracted
Expand-Archive -LiteralPath $ruffleZip -DestinationPath $ruffleExtracted
Expand-Archive -LiteralPath $ruffleWebZip -DestinationPath $ruffleWebExtracted

# Copy the whole FFDec distribution, including JARs, libraries and license.
$cli = Get-ChildItem $ffdecExtracted -Recurse -File -Filter 'ffdec-cli.exe' | Select-Object -First 1
if (-not $cli) { throw 'FFDec 26.3.0 ZIP has no ffdec-cli.exe; packaging refused.' }
$ffdecSource = $cli.Directory.FullName
foreach ($name in @('ffdec.exe', 'ffdec.jar')) {
    if (-not (Test-Path (Join-Path $ffdecSource $name) -PathType Leaf)) {
        throw "FFDec ZIP missing $name beside ffdec-cli.exe; packaging refused."
    }
}
$ffdecDest = Join-Path $target 'FFDec'
if (Test-Path $ffdecDest) { Remove-Item $ffdecDest -Recurse -Force }
New-Item -ItemType Directory -Path $ffdecDest -Force | Out-Null
Copy-Item -Path (Join-Path $ffdecSource '*') -Destination $ffdecDest -Recurse -Force
$ffdecLicense = Get-ChildItem $ffdecDest -Recurse -File | Where-Object {
    $_.Name -match '^(license|copying)(\.[a-z0-9_-]+)?$'
} | Select-Object -First 1
if (-not $ffdecLicense) { throw 'FFDec license was not found; packaging refused.' }

$ruffleExe = Get-ChildItem $ruffleExtracted -Recurse -File -Filter 'ruffle.exe' | Select-Object -First 1
if (-not $ruffleExe) { throw 'Ruffle ZIP does not contain ruffle.exe.' }
$ruffleDest = Join-Path $target 'Ruffle'
if (Test-Path $ruffleDest) { Remove-Item $ruffleDest -Recurse -Force }
New-Item -ItemType Directory -Path $ruffleDest -Force | Out-Null
Copy-Item -Path (Join-Path $ruffleExe.Directory.FullName '*') -Destination $ruffleDest -Recurse -Force

# Ruffle Web is the only runtime with a live JavaScript ExternalInterface API.
# Bundle it separately from Ruffle Desktop for isolated, local-browser previews.
$ruffleWebJs = Get-ChildItem $ruffleWebExtracted -Recurse -File -Filter 'ruffle.js' | Select-Object -First 1
if (-not $ruffleWebJs) { throw 'Verified Ruffle Web archive contains no ruffle.js.' }
$ruffleWebSource = $ruffleWebJs.Directory.FullName
$ruffleWebWasm = Get-ChildItem $ruffleWebSource -File -Filter '*.wasm' | Select-Object -First 1
if (-not $ruffleWebWasm) { throw 'Verified Ruffle Web archive lacks the WebAssembly runtime.' }
$ruffleWebDest = Join-Path $target 'RuffleWeb'
if (Test-Path $ruffleWebDest) { Remove-Item $ruffleWebDest -Recurse -Force }
New-Item -ItemType Directory -Path $ruffleWebDest -Force | Out-Null
Copy-Item -Path (Join-Path $ruffleWebSource '*') -Destination $ruffleWebDest -Recurse -Force

# Include upstream dependency notices in the redistribution.
Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/ruffle-rs/ruffle/v0.7.1/LICENSE.md' -OutFile (Join-Path $ruffleDest 'LICENSE.md')
Copy-Item -LiteralPath (Join-Path $ruffleDest 'LICENSE.md') -Destination (Join-Path $ruffleWebDest 'LICENSE.md') -Force
if ((Get-Item (Join-Path $ruffleDest 'LICENSE.md')).Length -lt 100) {
    throw 'Ruffle LICENSE.md is missing or unexpectedly small.'
}
$manifest = @"
Pinned GFX tools, installed only when the GFX tools option is selected.
Ruffle Desktop + Web v0.7.1: https://github.com/ruffle-rs/ruffle/releases/tag/v0.7.1
FFDec v26.3.0: https://github.com/jindrapetrik/jpexs-decompiler/releases/tag/version26.3.0

These are independent upstream projects and not part of Evolve Mod Manager.
See LICENSE.md in Ruffle and the FFDec license file before redistributing.
FFDec requires a compatible Java runtime installed separately.
"@
Set-Content -LiteralPath (Join-Path $target 'THIRD_PARTY_TOOLS.txt') -Value $manifest -Encoding utf8
Write-Host "Prepared installer GFX components at $target"
