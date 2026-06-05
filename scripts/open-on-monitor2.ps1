$ErrorActionPreference='Stop'
$Root = Split-Path -Path $PSScriptRoot -Parent
$App = Join-Path $Root 'build\MichStartupMaster.exe'
if(-not (Test-Path -LiteralPath $App)){ throw "Missing app: $App" }
# Only stop old Mich Startup Master instances, then open on the non-primary monitor.
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $App } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 500
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
Start-Process -FilePath $App | Out-Null
$proc=$null
$deadline=(Get-Date).AddSeconds(15)
do {
  Start-Sleep -Milliseconds 500
  $proc = Get-Process | Where-Object { $_.Path -eq $App -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
} while(-not $proc -and (Get-Date) -lt $deadline)
if(-not $proc){ throw 'Could not find visible Mich Startup Master window' }
$x=$target.Bounds.X+40; $y=$target.Bounds.Y+40; $w=$target.Bounds.Width-80; $h=$target.Bounds.Height-100
[WinApi]::ShowWindow($proc.MainWindowHandle, 9) | Out-Null
[WinApi]::SetWindowPos($proc.MainWindowHandle, [IntPtr](-1), $x, $y, $w, $h, 0x0040) | Out-Null
[WinApi]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Seconds 2
[WinApi]::SetWindowPos($proc.MainWindowHandle, [IntPtr](-2), $x, $y, $w, $h, 0x0040) | Out-Null
"Opened on monitor bounds=$($target.Bounds.X),$($target.Bounds.Y),$($target.Bounds.Width),$($target.Bounds.Height) pid=$($proc.Id)"
