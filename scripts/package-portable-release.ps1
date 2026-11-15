param(
    [Parameter(Mandatory = $true)][string]$SourceDirectory,
    [Parameter(Mandatory = $true)][string]$ArchivePath
)
$ErrorActionPreference = 'Stop'

$source = (Resolve-Path -LiteralPath $SourceDirectory).Path
$members = @(Get-Content -LiteralPath (Join-Path $PSScriptRoot 'portable-release-members.txt'))
$files = @(Get-ChildItem -LiteralPath $source -File -Recurse | ForEach-Object {
    $_.FullName.Substring($source.TrimEnd([IO.Path]::DirectorySeparatorChar).Length + 1).Replace('\', '/')
})
if ($files.Count -ne $members.Count -or @(Compare-Object $members $files -CaseSensitive).Count -ne 0) {
    throw 'Portable staging files do not match the updater inventory.'
}

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::Open($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ArchivePath), [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($member in $members) {
        $path = Join-Path $source $member
        $entry = $archive.CreateEntry($member, [IO.Compression.CompressionLevel]::Optimal)
        $inputStream = [IO.File]::OpenRead($path)
        try {
            $outputStream = $entry.Open()
            try { $inputStream.CopyTo($outputStream) } finally { $outputStream.Dispose() }
        } finally { $inputStream.Dispose() }
    }
} finally { $archive.Dispose() }

& (Join-Path $PSScriptRoot 'verify-portable-release.ps1') -ArchivePath $ArchivePath
