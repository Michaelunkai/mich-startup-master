$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$exe = 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe'
$preview = 'C:\MichStartupMaster-GitHubRecovery\assets\deployed-exe-icon.png'
$icon = [System.Drawing.Icon]::ExtractAssociatedIcon($exe)
if ($null -eq $icon) {
    throw "Windows could not extract an icon from $exe"
}

try {
    $bitmap = $icon.ToBitmap()
    try {
        $bitmap.Save($preview, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $bitmap.Dispose()
    }
    [pscustomobject]@{
        Executable = $exe
        ExtractedSize = "$($icon.Width)x$($icon.Height)"
        Preview = $preview
    } | Format-List
} finally {
    $icon.Dispose()
}
