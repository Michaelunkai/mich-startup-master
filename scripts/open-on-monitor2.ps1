$ErrorActionPreference='Stop'
$Root = Split-Path -Path $PSScriptRoot -Parent
$App = Join-Path $Root 'build\MichStartupMaster.exe'
if(-not (Test-Path -LiteralPath $App)){ throw "Missing app: $App" }
# Only stop old Mich Startup Master instances, then open on the non-primary monitor.
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $App } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinApi {
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
Add-Type -AssemblyName System.Windows.Forms
$target = [System.Windows.Forms.Screen]::AllScreens | Where-Object { -not $_.Primary } | Sort-Object { $_.Bounds.X } | Select-Object -First 1
if(-not $target){ $target = [System.Windows.Forms.Screen]::PrimaryScreen }
$p = Start-Process -FilePath $App -PassThru
Start-Sleep -Seconds 5
$proc = Get-Process -Id $p.Id
if($proc.MainWindowHandle -eq 0){ throw 'Window handle is zero' }
$x=$target.Bounds.X+40; $y=$target.Bounds.Y+40; $w=$target.Bounds.Width-80; $h=$target.Bounds.Height-100
[WinApi]::ShowWindow($proc.MainWindowHandle, 9) | Out-Null
[WinApi]::SetWindowPos($proc.MainWindowHandle, [IntPtr](-1), $x, $y, $w, $h, 0x0040) | Out-Null
[WinApi]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Seconds 1
[WinApi]::SetWindowPos($proc.MainWindowHandle, [IntPtr](-2), $x, $y, $w, $h, 0x0040) | Out-Null
"Opened on monitor bounds=$($target.Bounds.X),$($target.Bounds.Y),$($target.Bounds.Width),$($target.Bounds.Height) pid=$($proc.Id)"
