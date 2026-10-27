# Stage the five immutable runtime files from the developer-supplied 2025 ZIP.
# The release runner fetches that ZIP from a private GitHub release, NOT this repo.
# Do not publish source code, development builds, symbols or the raw developer ZIP.
param([Parameter(Mandatory=$true)][string]$Archive)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedZip = '5b187f60ede62776f6d48f6605ef02e78e98b0b9d48e8568a6469f906b781119'
$componentHashes = @{
  'evolve_tool.exe' = 'c37e21134d7442c3fa195f9b09d8b951af0ee03b2c7617fe86ba5a7e7cbd4b9e'
  'crypto.dll' = '841105bb6703206cae4d5c73d8d0ced5c50240d9595090a2a09dc2963fd22f78'
  'inject/inject.dll' = '62da468d2a8d98edfc5f9e9e1f6d7f74ca2a9d00712bfa8f163d674ec15ded91'
  'inject/dbghelp.dll' = '937f09c6eba49294f2d5495e7b4065aeae66047d65e7cc92bab92c8a702f74aa'
  'inject/RSAKeyData.bin' = 'cec8d89e39db3f10d5f6cff984f713fe59cf84914f2df1294a27daeb21c42c18'
}

if (!(Test-Path -LiteralPath $Archive -PathType Leaf)) { throw 'Developer archive missing.' }
if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedZip) {
  throw 'Developer tool archive SHA256 mismatch; refusing to package.'
}
Add-Type -AssemblyName System.IO.Compression
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dest = Join-Path $root 'dist/third_party/OfficialStage2'
if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Recurse -Force }
New-Item -Path $dest -ItemType Directory -Force | Out-Null
$zip = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $Archive).Path)
try {
  foreach ($relative in ($componentHashes.Keys | Sort-Object)) {
    $member = $zip.GetEntry("evolve_tool_stage2/$relative")
    if ($null -eq $member -or $member.Length -le 0 -or $member.Length -gt 5MB) {
      throw "Missing or unsafe official tool component: $relative"
    }
    $output = Join-Path $dest ($relative -replace '/', [IO.Path]::DirectorySeparatorChar)
    $parent = Split-Path -Parent $output
    New-Item -Path $parent -ItemType Directory -Force | Out-Null
    $sourceStream = $member.Open()
    $targetStream = [System.IO.File]::Create($output)
    try { $sourceStream.CopyTo($targetStream) }
    finally { $targetStream.Dispose(); $sourceStream.Dispose() }
    $digest = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($digest -ne $componentHashes[$relative]) {
      throw "Official tool component hash mismatch: $relative"
    }
  }
} finally { $zip.Dispose() }
$notice = @'
Evolve Stage 2 developer toolchain, dated 02.05.2025.
Supplied by the game's developers for use with Evolve Stage 2.
The signing executable embeds a private RSA signing key.
Do not treat embedded binaries, hashed binaries, or packaging as key secrecy.
Distributed as separate third-party runtime components of Evolve Mod Manager.
Source/debug projects from the developer ZIP are not distributed.
'@
[System.IO.File]::WriteAllText((Join-Path $dest 'README.txt'), $notice)
Write-Host 'Verified official Stage 2 runtime files staged for both installer and portable releases.'
