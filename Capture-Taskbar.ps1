$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $path = 'C:\Temp\MichStartupMaster-taskbar-live.png'
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Output $path
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}
