$ErrorActionPreference='Stop'
$App='F:\study\Windows\Applications\StartupManager\MichStartupMaster\build\MichStartupMaster.exe'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinApi {
 [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
$p=Get-Process | Where-Object { $_.Path -eq $App } | Select-Object -First 1
if(-not $p){ $p=Start-Process -FilePath $App -PassThru; Start-Sleep -Seconds 2; $p=Get-Process -Id $p.Id }
$p.Refresh()
"PID=$($p.Id) HANDLE=$($p.MainWindowHandle) TITLE=$($p.MainWindowTitle)"
if($p.MainWindowHandle -eq 0){ throw 'main window handle is zero' }
[WinApi]::ShowWindowAsync($p.MainWindowHandle, 5) | Out-Null
[WinApi]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
Start-Sleep -Seconds 1
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$bounds=[System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp=New-Object System.Drawing.Bitmap $bounds.Width,$bounds.Height
$g=[System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bounds.Size)
$out='C:\Temp\hermes_snap\MichStartupMaster_foreground.png'
$bmp.Save($out,[System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"SCREENSHOT $out size=$((Get-Item $out).Length)"
