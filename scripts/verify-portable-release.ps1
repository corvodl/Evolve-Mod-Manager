param([Parameter(Mandatory = $true)][string]$ArchivePath)
$ErrorActionPreference = 'Stop'

$expected = @(Get-Content -LiteralPath (Join-Path $PSScriptRoot 'portable-release-members.txt'))
if ($expected.Count -ne 16 -or @($expected | Select-Object -Unique).Count -ne $expected.Count) {
    throw 'Portable release inventory is invalid.'
}

Add-Type -AssemblyName System.IO.Compression
$archive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $ArchivePath).Path)
try {
    $actual = @($archive.Entries | ForEach-Object { $_.FullName })
    if (@(Compare-Object $expected $actual -CaseSensitive).Count -ne 0 -or
        $actual.Count -ne $expected.Count) {
        throw 'Portable ZIP does not match the updater inventory.'
    }
    if (@($archive.Entries | Where-Object { $_.Length -le 0 }).Count -ne 0) {
        throw 'Portable ZIP contains an empty release member.'
    }
} finally {
    $archive.Dispose()
}
