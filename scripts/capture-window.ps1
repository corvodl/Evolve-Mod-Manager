param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$Output
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
$interop = @'
using System;
using System.Runtime.InteropServices;
public static class WindowCoordinates {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
}
'@
Add-Type -TypeDefinition $interop
$fullExe = (Resolve-Path -LiteralPath $Executable).Path
New-Item -ItemType Directory -Force -Path (Split-Path $Output -Parent) | Out-Null
$process = Start-Process -FilePath $fullExe -PassThru
try {
    for ($attempt = 0; $attempt -lt 24; $attempt++) {
        Start-Sleep -Milliseconds 750
        $process.Refresh()
        if ($process.HasExited) { throw "Evolve Mod Manager exited before a screenshot was available." }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { break }
    }
    $process.Refresh()
    if ($process.MainWindowHandle -eq [IntPtr]::Zero) {
        throw "No GUI window handle; the Windows runner may not have an interactive desktop."
    }
    Start-Sleep -Seconds 3
    $rect = New-Object WindowCoordinates+RECT
    if (-not [WindowCoordinates]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) {
        throw "Could not read the window bounds."
    }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -lt 400 -or $height -lt 300 -or $width -gt 3500 -or $height -gt 2200) {
        throw "Unexpected application window dimensions: $width x $height"
    }
    $bitmap = [System.Drawing.Bitmap]::new($width, $height)
    try {
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        }
        finally { $graphics.Dispose() }
        # Reject black/flat frames that indicate missing desktop or GPU output.
        $pixels = New-Object 'System.Collections.Generic.HashSet[int]'
        for ($y = 0; $y -lt $height; $y += [Math]::Max(1,[int]($height / 12))) {
            for ($x = 0; $x -lt $width; $x += [Math]::Max(1,[int]($width / 12))) {
                $null = $pixels.Add($bitmap.GetPixel($x, $y).ToArgb())
            }
        }
        if ($pixels.Count -lt 12) { throw "Capture appears blank; refusing to publish misleading screenshot." }
        $bitmap.Save($Output, [System.Drawing.Imaging.ImageFormat]::Png)
        Write-Host "Captured actual application window: $width x $height, sampled colors: $($pixels.Count)"
    }
    finally { $bitmap.Dispose() }
}
finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
}
