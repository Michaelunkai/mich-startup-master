$ErrorActionPreference='Stop'
$App='F:\study\Windows\Applications\StartupManager\MichStartupMaster\build\MichStartupMaster.exe'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinApi2 {
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$target=[System.Windows.Forms.Screen]::AllScreens | Where-Object { -not $_.Primary } | Sort-Object { $_.Bounds.X } | Select-Object -First 1
if(-not $target){ throw 'no secondary monitor' }
$p=Get-Process | Where-Object { $_.Path -eq $App } | Select-Object -First 1
if(-not $p){ $p=Start-Process -FilePath $App -PassThru; Start-Sleep -Seconds 4; $p=Get-Process -Id $p.Id }
$p.Refresh()
if($p.MainWindowHandle -eq 0){ throw 'Startup Master handle is zero' }
$topmost=[IntPtr](-1); $notop=[IntPtr](-2)
$x=$target.Bounds.X + 40; $y=$target.Bounds.Y + 40; $w=$target.Bounds.Width - 80; $h=$target.Bounds.Height - 100
[WinApi2]::ShowWindow($p.MainWindowHandle, 9) | Out-Null
[WinApi2]::SetWindowPos($p.MainWindowHandle, $topmost, $x, $y, $w, $h, 0x0040) | Out-Null
[WinApi2]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
Start-Sleep -Seconds 2
[WinApi2]::SetWindowPos($p.MainWindowHandle, $notop, $x, $y, $w, $h, 0x0040) | Out-Null
Start-Sleep -Milliseconds 500
$bounds=$target.Bounds
$bmp=New-Object System.Drawing.Bitmap $bounds.Width,$bounds.Height
$g=[System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bounds.Size)
$out='C:\Temp\hermes_snap\MichStartupMaster_monitor2_full_list_top.png'
$bmp.Save($out,[System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"MONITOR2 Bounds=$($bounds.X),$($bounds.Y),$($bounds.Width),$($bounds.Height)"
"GUI_PROCESS pid=$($p.Id) handle=$($p.MainWindowHandle) title=$($p.MainWindowTitle)"
"SCREENSHOT $out size=$((Get-Item $out).Length)"
