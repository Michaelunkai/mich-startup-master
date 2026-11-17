# Capture the Mich Startup Master window to PNG (DPI-aware PrintWindow).
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File shot.ps1 <out.png> [waitMs]
param([Parameter(Mandatory=$true)][string]$Out, [int]$WaitMs = 2500)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WinCap {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lp);
    [DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr hWnd, [MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[WinCap]::SetProcessDPIAware() | Out-Null
Start-Sleep -Milliseconds $WaitMs
$found = [IntPtr]::Zero
$callback = [WinCap+EnumWindowsProc]{ param($h, $l)
    $sb = New-Object System.Text.StringBuilder 256
    [WinCap]::GetWindowTextW($h, $sb, 256) | Out-Null
    if ($sb.ToString().Contains("Mich Startup Master") -and [WinCap]::IsWindowVisible($h)) {
        $script:found = $h; return $false
    }
    return $true
}
[WinCap]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
if ($found -eq [IntPtr]::Zero) { Write-Host "SHOT window-not-found"; exit 3 }
$rect = New-Object WinCap+RECT
[WinCap]::GetWindowRect($found, [ref]$rect) | Out-Null
$w = $rect.R - $rect.L; $h = $rect.B - $rect.T
Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[WinCap]::PrintWindow($found, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc); $g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Host "SHOT saved=$Out ${w}x${h}"
