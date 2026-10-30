# Stage an independent GPLv2 command-line tool with its corresponding source.
# Nothing is fetched unless -Fetch is explicitly supplied.
param(
  [switch]$Fetch,
  [string]$BinaryPath,
  [string]$SourceArchivePath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$revision = '6ae79ce6aeb669b632ce9adee452a6ad99e1f883'
$version = 'v2.0.0'
$expectedBinaryHash = '75feca2813a6517cad9a8e5df28687860a3b0bab9dcbcfeef65e56850d53b0b7'
$sourceName = 'Cryengine-Converter-source-v2.0.0.zip'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$dest = Join-Path $root 'dist/third_party/Cryengine-Converter'
$work = Join-Path $root ('dist/.converter-stage-' + [guid]::NewGuid().ToString('N'))

if ($Fetch -and ($BinaryPath -or $SourceArchivePath)) {
  throw 'Use either -Fetch or both local input paths, not both.'
}
if (!$Fetch -and (!$BinaryPath -or !$SourceArchivePath)) {
  throw 'Supply both -BinaryPath and -SourceArchivePath, or opt in to downloads with -Fetch.'
}
if (Test-Path -LiteralPath $dest) {
  throw "Destination already exists; remove it explicitly before restaging: $dest"
}

function Assert-LocalFile([string]$path) {
  $file = Get-Item -LiteralPath $path -ErrorAction Stop
  if ($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw "Expected a regular file, not a directory or link: $path"
  }
  return $file.FullName
}

try {
  New-Item -ItemType Directory -Force -Path (Join-Path $root 'dist') | Out-Null
  New-Item -ItemType Directory -Path $work -ErrorAction Stop | Out-Null
  if ($Fetch) {
    $BinaryPath = Join-Path $work 'cgf-converter.exe'
    $SourceArchivePath = Join-Path $work $sourceName
    Invoke-WebRequest -Uri "https://github.com/Markemp/Cryengine-Converter/releases/download/$version/cgf-converter.exe" -OutFile $BinaryPath -MaximumRedirection 8
    Invoke-WebRequest -Uri "https://github.com/Markemp/Cryengine-Converter/archive/$revision.zip" -OutFile $SourceArchivePath -MaximumRedirection 8
  }
  $BinaryPath = Assert-LocalFile $BinaryPath
  $SourceArchivePath = Assert-LocalFile $SourceArchivePath
  $actual = (Get-FileHash -LiteralPath $BinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $expectedBinaryHash) {
    throw "Cryengine-Converter binary SHA256 mismatch: $actual"
  }

  Add-Type -AssemblyName System.IO.Compression
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $archive = [IO.Compression.ZipFile]::OpenRead($SourceArchivePath)
  try {
    $prefix = "Cryengine-Converter-$revision/"
    $license = $archive.Entries | Where-Object { $_.FullName -ceq "${prefix}LICENSE" } | Select-Object -First 1
    if (!$license -or $license.Length -lt 100 -or $license.Length -gt 100KB) {
      throw 'Corresponding source archive must contain the GPLv2 LICENSE at the pinned revision.'
    }
    $reader = [IO.StreamReader]::new($license.Open())
    try { $licenseText = $reader.ReadToEnd() } finally { $reader.Dispose() }
    if ($licenseText -notmatch 'GNU GENERAL PUBLIC LICENSE' -or $licenseText -notmatch 'Version 2') {
      throw 'Source archive LICENSE does not identify GNU GPL version 2.'
    }
    if (!($archive.Entries | Where-Object { $_.FullName.StartsWith($prefix, [StringComparison]::Ordinal) -and $_.FullName -match '\.(?:cs|cpp|py|java|c)$' } | Select-Object -First 1)) {
      throw 'Source archive contains no source files under the pinned revision.'
    }
    if ($archive.Entries | Where-Object { !$_.FullName.StartsWith($prefix, [StringComparison]::Ordinal) }) {
      throw 'Unexpected paths in source archive.'
    }
  } finally { $archive.Dispose() }

  $staged = Join-Path $work 'staged'
  New-Item -ItemType Directory -Path $staged | Out-Null
  Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $staged 'cgf-converter.exe')
  Copy-Item -LiteralPath $SourceArchivePath -Destination (Join-Path $staged $sourceName)
  if ((Get-FileHash -LiteralPath (Join-Path $staged 'cgf-converter.exe') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedBinaryHash) {
    throw 'Staged converter binary changed during copy.'
  }
  @"
Cryengine-Converter by Markemp and contributors ($version)
Upstream: https://github.com/Markemp/Cryengine-Converter
Source revision: $revision
License: GNU General Public License version 2 (see LICENSE in $sourceName).
Unmodified upstream cgf-converter.exe SHA256: $expectedBinaryHash
The corresponding upstream source archive is included alongside the executable.

This is a separate, independent command-line tool, not linked into Evolve Mod Manager.
You may copy, modify and redistribute it under the GPLv2; preserve its license and
provide corresponding source with redistributions of the executable.
"@ | Set-Content -LiteralPath (Join-Path $staged 'NOTICE.txt') -Encoding UTF8

  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dest) | Out-Null
  if (Test-Path -LiteralPath $dest) { throw "Destination appeared during staging: $dest" }
  Move-Item -LiteralPath $staged -Destination $dest
  Write-Host "Verified Cryengine-Converter and corresponding source staged at $dest"
} finally {
  if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
}
