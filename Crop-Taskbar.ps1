$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$sourcePath = 'C:\Temp\MichStartupMaster-taskbar-live.png'
$outputPath = 'C:\Temp\MichStartupMaster-taskbar-strip.png'
$source = [System.Drawing.Bitmap]::FromFile($sourcePath)
try {
    $height = [Math]::Min(180, $source.Height)
    $rect = New-Object System.Drawing.Rectangle 0, ($source.Height - $height), $source.Width, $height
    $crop = $source.Clone($rect, $source.PixelFormat)
    try {
        $crop.Save($outputPath, [System.Drawing.Imaging.ImageFormat]::Png)
        Write-Output $outputPath
    } finally {
        $crop.Dispose()
    }
} finally {
    $source.Dispose()
}
