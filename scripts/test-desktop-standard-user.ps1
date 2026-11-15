# Run desktop tests with an unelevated test binary. The shipped GUI manifest
# is unchanged; a separate target directory prevents mixing the two builds.
$ErrorActionPreference = 'Stop'
$originalManifest = [Environment]::GetEnvironmentVariable('EVOLVE_TEST_NO_MANIFEST', 'Process')
$originalTarget = [Environment]::GetEnvironmentVariable('CARGO_TARGET_DIR', 'Process')
try {
    $env:EVOLVE_TEST_NO_MANIFEST = '1'
    $env:CARGO_TARGET_DIR = Join-Path (Split-Path -Parent $PSScriptRoot) 'target/standard-user-tests'
    $cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
    if (!$cargo) { $cargo = Join-Path $HOME '.cargo/bin/cargo.exe' }
    & $cargo test -p evolve-mod-manager --all-targets
    if ($LASTEXITCODE -ne 0) { throw "Desktop tests failed with exit code $LASTEXITCODE" }
} finally {
    [Environment]::SetEnvironmentVariable('EVOLVE_TEST_NO_MANIFEST', $originalManifest, 'Process')
    [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $originalTarget, 'Process')
}
