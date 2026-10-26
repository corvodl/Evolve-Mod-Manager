# Download only the pinned official vgmstream Windows x64 CLI archive.
# Package the decoder and codec DLLs for the optional Audio Tools installer task.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$target = Join-Path $root 'dist/third_party/vgmstream'
$tempBase = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$temp = Join-Path $tempBase 'evolve-vgmstream-r2117'
$archive = Join-Path $temp 'vgmstream-win64.zip'
$expanded = Join-Path $temp 'extracted'
New-Item -ItemType Directory -Path $temp -Force | Out-Null
Invoke-WebRequest -Uri 'https://github.com/vgmstream/vgmstream/releases/download/r2117/vgmstream-win64.zip' -OutFile $archive -MaximumRedirection 5
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
if ($hash -ne '6c4a8a3813864fefed081bbd337dbc0ad93bf88e0b92f5db98d7ab258b22dc6c') {
    throw 'vgmstream release archive SHA256 mismatch; packaging refused.'
}
if ((Get-Item -LiteralPath $archive).Length -gt 20MB) {
    throw 'vgmstream release unexpectedly large.'
}
if (Test-Path $expanded) { Remove-Item -LiteralPath $expanded -Recurse -Force }
Expand-Archive -LiteralPath $archive -DestinationPath $expanded
$cli = Get-ChildItem -LiteralPath $expanded -File -Recurse -Filter 'vgmstream-cli.exe' | Select-Object -First 1
if (-not $cli) { throw 'Pinned vgmstream ZIP did not include vgmstream-cli.exe.' }
if (Test-Path $target) { Remove-Item -LiteralPath $target -Recurse -Force }
New-Item -ItemType Directory -Path $target -Force | Out-Null
Copy-Item -Path (Join-Path $cli.DirectoryName '*') -Destination $target -Recurse -Force
$notice = Join-Path $target 'COPYING'
if (-not (Test-Path -LiteralPath $notice -PathType Leaf)) {
    Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/vgmstream/vgmstream/r2117/COPYING' -OutFile $notice
}
if ((Get-Item -LiteralPath $notice).Length -lt 100) { throw 'vgmstream copyright notice is missing.' }
Write-Host "Verified and staged optional vgmstream r2117 CLI with copyright notice."


# Optional MIT-licensed Ogg Vorbis to FSB5 single-sample remuxer.
$remuxArchive = Join-Path $temp 'oggvorbis2fsb5-win64.zip'
$remuxExpanded = Join-Path $temp 'oggvorbis2fsb5-extracted'
Invoke-WebRequest -Uri 'https://github.com/uyjulian/oggvorbis2fsb5/releases/download/latest/oggvorbis2fsb5-win32-amd64-clang.zip' -OutFile $remuxArchive -MaximumRedirection 5
$remuxHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $remuxArchive).Hash.ToLowerInvariant()
if ($remuxHash -ne '6dfdd707d50daa65849c7a9632fdadc2a2b463c377c1830bdb2b64eceef85866') {
    throw 'Vorbis FSB5 remuxer SHA-256 mismatch; packaging refused.'
}
if ((Get-Item -LiteralPath $remuxArchive).Length -gt 5MB) { throw 'Remuxer release ZIP exceeds size budget.' }
if (Test-Path $remuxExpanded) { Remove-Item -LiteralPath $remuxExpanded -Recurse -Force }
Expand-Archive -LiteralPath $remuxArchive -DestinationPath $remuxExpanded
$remuxExe = Get-ChildItem -LiteralPath $remuxExpanded -File -Recurse -Filter 'oggvorbis2fsb5.exe' | Select-Object -First 1
if (-not $remuxExe) { throw 'Verified remux release lacks oggvorbis2fsb5.exe.' }
$remuxDest = Join-Path $root 'dist/third_party/oggvorbis2fsb5'
if (Test-Path $remuxDest) { Remove-Item -LiteralPath $remuxDest -Recurse -Force }
New-Item -ItemType Directory -Path $remuxDest -Force | Out-Null
Copy-Item -Path (Join-Path $remuxExe.Directory.FullName '*') -Destination $remuxDest -Recurse -Force
$remuxLicense = Join-Path $remuxDest 'LICENSE'
if (-not (Test-Path -LiteralPath $remuxLicense -PathType Leaf)) {
    Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/uyjulian/oggvorbis2fsb5/master/LICENSE' -OutFile $remuxLicense
}
if ((Get-Item -LiteralPath $remuxLicense).Length -lt 100) {
    throw 'Vorbis remuxer MIT license notice is missing.'
}
Write-Host "Verified and staged optional MIT Vorbis-to-FSB5 remuxer."
