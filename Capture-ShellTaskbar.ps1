$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;

public static class TaskbarWindow
{
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string windowName);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr handle, out RECT rect);
}
'@

$handle = [TaskbarWindow]::FindWindow('Shell_TrayWnd', $null)
if ($handle -eq [IntPtr]::Zero) {
    throw 'Windows taskbar window was not found.'
}

$rect = New-Object TaskbarWindow+RECT
if (-not [TaskbarWindow]::GetWindowRect($handle, [ref]$rect)) {
    throw 'Windows taskbar bounds could not be read.'
}

$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) {
    throw 'Windows taskbar bounds are invalid.'
}

$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $path = 'C:\Temp\MichStartupMaster-shell-taskbar.png'
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    [pscustomobject]@{
        TaskbarBounds = "$($rect.Left),$($rect.Top) $width x $height"
        Capture = $path
    } | Format-List
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}
