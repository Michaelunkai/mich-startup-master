$ErrorActionPreference='Stop'
$App='F:\study\Windows\Applications\StartupManager\MichStartupMaster\build\MichStartupMaster.exe'
# Stop only this app's old instances before relaunching on monitor 2.
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $App } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinApi {
 [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$screens=[System.Windows.Forms.Screen]::AllScreens
$target=$screens | Where-Object { -not $_.Primary } | Sort-Object { $_.Bounds.X } | Select-Object -First 1
if(-not $target){ $target=$screens | Select-Object -First 1 }
$p=Start-Process -FilePath $App -PassThru
Start-Sleep -Seconds 6
$proc=Get-Process -Id $p.Id
if($proc.MainWindowHandle -eq 0){ Start-Sleep -Seconds 2; $proc.Refresh() }
if($proc.MainWindowHandle -eq 0){ throw 'main window handle is zero' }
$x=$target.Bounds.X + 80; $y=$target.Bounds.Y + 80; $w=[Math]::Min(1320,$target.Bounds.Width-160); $h=[Math]::Min(900,$target.Bounds.Height-160)
[WinApi]::ShowWindowAsync($proc.MainWindowHandle, 5) | Out-Null
[WinApi]::SetWindowPos($proc.MainWindowHandle, [IntPtr]::Zero, $x, $y, $w, $h, 0x0040) | Out-Null
[WinApi]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Seconds 1
$bounds=$target.Bounds
$bmp=New-Object System.Drawing.Bitmap $bounds.Width,$bounds.Height
$g=[System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bounds.Size)
$out='C:\Temp\hermes_snap\MichStartupMaster_monitor2_full_list.png'
New-Item -ItemType Directory -Force -Path (Split-Path -Path $out -Parent) | Out-Null
$bmp.Save($out,[System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"MONITOR2 Primary=$($target.Primary) Bounds=$($target.Bounds.X),$($target.Bounds.Y),$($target.Bounds.Width),$($target.Bounds.Height)"
"GUI_PROCESS pid=$($proc.Id) handle=$($proc.MainWindowHandle) title=$($proc.MainWindowTitle)"
"SCREENSHOT $out size=$((Get-Item $out).Length)"
