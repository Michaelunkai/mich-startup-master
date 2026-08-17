$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MichStartupMasterShellRefresh
{
    [DllImport("shell32.dll")]
    public static extern void SHChangeNotify(uint eventId, uint flags, IntPtr item1, IntPtr item2);
}
'@

$exe = 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe'
$workingDirectory = Split-Path -Parent $exe
$programsDirectory = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs'
$shortcutPath = Join-Path $programsDirectory 'Mich Startup Master.lnk'

if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "Canonical executable was not found: $exe"
}

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $exe
$shortcut.Arguments = ''
$shortcut.WorkingDirectory = $workingDirectory
$shortcut.IconLocation = "$exe,0"
$shortcut.Description = 'Mich Startup Master'
$shortcut.Save()
[MichStartupMasterShellRefresh]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)

$saved = $shell.CreateShortcut($shortcutPath)
$icon = [System.Drawing.Icon]::ExtractAssociatedIcon($exe)

[pscustomobject]@{
    Shortcut = $shortcutPath
    Target = $saved.TargetPath
    StartIn = $saved.WorkingDirectory
    IconLocation = $saved.IconLocation
    EmbeddedIconSize = if ($null -eq $icon) { 'missing' } else { "$($icon.Width)x$($icon.Height)" }
} | Format-List
