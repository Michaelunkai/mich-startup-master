$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MichStartupMasterShellIconNotify
{
    [DllImport("shell32.dll")]
    public static extern void SHChangeNotify(uint eventId, uint flags, IntPtr item1, IntPtr item2);
}
'@

$sourceIcon = 'C:\MichStartupMaster-GitHubRecovery\assets\MichStartupMaster.ico'
$buildDirectory = 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build'
$exe = Join-Path $buildDirectory 'MichStartupMaster.exe'
$runtimeIcon = Join-Path $buildDirectory 'MichStartupMaster.ico'
$taskbarIcon = Join-Path $buildDirectory 'MichStartupMaster-taskbar-v3.ico'
$programsDirectory = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs'
$startMenuShortcut = Join-Path $programsDirectory 'Mich Startup Master.lnk'
$pinnedShortcut = Join-Path $env:APPDATA 'Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\Mich Startup Master.lnk'

foreach ($path in @($sourceIcon, $exe, $startMenuShortcut, $pinnedShortcut)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required path was not found: $path"
    }
}

Copy-Item -LiteralPath $sourceIcon -Destination $runtimeIcon -Force
Copy-Item -LiteralPath $sourceIcon -Destination $taskbarIcon -Force

$shell = New-Object -ComObject WScript.Shell
foreach ($shortcutPath in @($startMenuShortcut, $pinnedShortcut)) {
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $exe
    $shortcut.Arguments = ''
    $shortcut.WorkingDirectory = $buildDirectory
    $shortcut.IconLocation = "$taskbarIcon,0"
    $shortcut.Save()
}

$icon = New-Object System.Drawing.Icon $taskbarIcon
try {
    [MichStartupMasterShellIconNotify]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
    [pscustomobject]@{
        RuntimeIcon = $runtimeIcon
        TaskbarIcon = $taskbarIcon
        IconSize = "$($icon.Width)x$($icon.Height)"
        PinnedShortcut = $pinnedShortcut
        IconLocation = "$taskbarIcon,0"
    } | Format-List
} finally {
    $icon.Dispose()
}
