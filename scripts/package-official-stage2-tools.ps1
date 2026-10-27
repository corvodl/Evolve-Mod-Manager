# Package only the pinned, developer-provided Stage 2 runtime from the
# private source repository. Never publish the original source tree or keys.
param([Parameter(Mandatory=$true)][string]$SourceDirectory)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$componentHashes = @{
  'evolve_tool.exe' = 'c37e21134d7442c3fa195f9b09d8b951af0ee03b2c7617fe86ba5a7e7cbd4b9e'
  'crypto.dll' = '841105bb6703206cae4d5c73d8d0ced5c50240d9595090a2a09dc2963fd22f78'
  'inject/inject.dll' = '62da468d2a8d98edfc5f9e9e1f6d7f74ca2a9d00712bfa8f163d674ec15ded91'
  'inject/dbghelp.dll' = '937f09c6eba49294f2d5495e7b4065aeae66047d65e7cc92bab92c8a702f74aa'
  'inject/RSAKeyData.bin' = 'cec8d89e39db3f10d5f6cff984f713fe59cf84914f2df1294a27daeb21c42c18'
  'LICENSE-BOUNCYCASTLE.html' = '96bf2877756c135014a20942b1060d9850b047ee59f9becd5b4c0e574733d5ed'
}

$source = (Resolve-Path -LiteralPath $SourceDirectory -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath $source -PathType Container)) {
  throw 'Developer tool source directory is missing.'
}
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dest = Join-Path $root 'dist/third_party/OfficialStage2'
# Verify every input before modifying any previous output directory.
foreach ($relative in ($componentHashes.Keys | Sort-Object)) {
  $from = if ($relative -eq 'LICENSE-BOUNCYCASTLE.html') {
    Join-Path $source 'src/evolve_tool/packages/Portable.BouncyCastle-Signed.1.7.0.2/License.html'
  } else {
    Join-Path $source ($relative -replace '/', [IO.Path]::DirectorySeparatorChar)
  }
  if (!(Test-Path -LiteralPath $from -PathType Leaf)) {
    throw "Missing official tool component: $relative"
  }
  $file = Get-Item -LiteralPath $from -Force
  if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) {
    throw "Symlinked official tool component rejected: $relative"
  }
  if ($file.Length -le 0 -or $file.Length -gt 5MB) {
    throw "Unexpected official tool component size: $relative"
  }
  $digest = (Get-FileHash -LiteralPath $from -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($digest -ne $componentHashes[$relative]) {
    throw "Official tool component hash mismatch: $relative"
  }
}
if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Recurse -Force }
New-Item -Path $dest -ItemType Directory -Force | Out-Null
foreach ($relative in ($componentHashes.Keys | Sort-Object)) {
  $from = if ($relative -eq 'LICENSE-BOUNCYCASTLE.html') {
    Join-Path $source 'src/evolve_tool/packages/Portable.BouncyCastle-Signed.1.7.0.2/License.html'
  } else {
    Join-Path $source ($relative -replace '/', [IO.Path]::DirectorySeparatorChar)
  }
  $to = Join-Path $dest ($relative -replace '/', [IO.Path]::DirectorySeparatorChar)
  New-Item -Path (Split-Path -Parent $to) -ItemType Directory -Force | Out-Null
  Copy-Item -LiteralPath $from -Destination $to
  $digest = (Get-FileHash -LiteralPath $to -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($digest -ne $componentHashes[$relative]) {
    throw "Staged official component changed during copy: $relative"
  }
}
$notice = @'
Evolve Stage 2 developer toolchain, dated 02.05.2025.
Supplied by the game's developers for use with Evolve Stage 2.
The signing executable embeds a private RSA signing key.
Hash verification checks file integrity; it does not hide signing material.
This distribution contains only verified runtime components and license.
The developer source tree is not included.
'@
[System.IO.File]::WriteAllText((Join-Path $dest 'README.txt'), $notice)
Write-Host 'Verified official Stage 2 runtime staged for installer and portable releases.'
