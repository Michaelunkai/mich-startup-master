$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;

public static class TaskbarRenderer
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

    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr handle, IntPtr deviceContext, uint flags);
}
'@

$handle = [TaskbarRenderer]::FindWindow('Shell_TrayWnd', $null)
if ($handle -eq [IntPtr]::Zero) {
    throw 'Windows taskbar window was not found.'
}

$rect = New-Object TaskbarRenderer+RECT
if (-not [TaskbarRenderer]::GetWindowRect($handle, [ref]$rect)) {
    throw 'Windows taskbar bounds could not be read.'
}

$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $dc = $graphics.GetHdc()
    try {
        if (-not [TaskbarRenderer]::PrintWindow($handle, $dc, 2)) {
            throw 'Windows did not render the taskbar window.'
        }
    } finally {
        $graphics.ReleaseHdc($dc)
    }
    $path = 'C:\Temp\MichStartupMaster-shell-taskbar-render.png'
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Output $path
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}
