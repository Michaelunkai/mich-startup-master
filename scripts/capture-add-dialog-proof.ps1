$ErrorActionPreference='Stop'
$Root=Split-Path -Path $PSScriptRoot -Parent
$App=Join-Path $Root 'build\MichStartupMaster.exe'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class DialogWinApi {
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
 [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$target=[System.Windows.Forms.Screen]::AllScreens | Where-Object { -not $_.Primary } | Sort-Object { $_.Bounds.X } | Select-Object -First 1
if(-not $target){ $target=[System.Windows.Forms.Screen]::PrimaryScreen }
$p=Start-Process -FilePath $App -ArgumentList '--show-add-dialog' -PassThru
Start-Sleep -Seconds 3
$proc=Get-Process -Id $p.Id
if($proc.MainWindowHandle -eq 0){ throw 'add dialog handle is zero' }
$x=$target.Bounds.X + 220; $y=$target.Bounds.Y + 160
[DialogWinApi]::ShowWindow($proc.MainWindowHandle, 5) | Out-Null
[DialogWinApi]::SetWindowPos($proc.MainWindowHandle, [IntPtr](-1), $x, $y, 720, 520, 0x0040) | Out-Null
[DialogWinApi]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Seconds 1
$bounds=$target.Bounds
$bmp=New-Object Drawing.Bitmap $bounds.Width,$bounds.Height
$g=[Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bounds.Size)
$out=Join-Path $Root 'artifacts\proof\monitor2-add-dialog.png'
New-Item -ItemType Directory -Force -Path (Split-Path -Path $out -Parent) | Out-Null
$bmp.Save($out,[Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
"ADD_DIALOG_SCREENSHOT=$out SIZE=$((Get-Item $out).Length) TITLE=$($proc.MainWindowTitle)"
