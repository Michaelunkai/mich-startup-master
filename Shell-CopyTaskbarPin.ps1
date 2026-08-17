$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ShellTaskbarPinFinder
{
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string windowName);
}
'@

$programsDirectory = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs'
$shortcutPath = Join-Path $programsDirectory 'Mich Startup Master.lnk'
$pinnedDirectory = Join-Path $env:APPDATA 'Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar'
$pinnedShortcutPath = Join-Path $pinnedDirectory 'Mich Startup Master.lnk'

if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) {
    throw "Start Menu shortcut was not found: $shortcutPath"
}

if (Test-Path -LiteralPath $pinnedShortcutPath -PathType Leaf) {
    Remove-Item -LiteralPath $pinnedShortcutPath -Force
}

$shell = New-Object -ComObject Shell.Application
$taskbarFolder = $shell.Namespace($pinnedDirectory)
if ($null -eq $taskbarFolder) {
    throw "Windows did not resolve the Taskbar pinned-items namespace: $pinnedDirectory"
}

$taskbarFolder.CopyHere($shortcutPath, 16)
Start-Sleep -Seconds 4

$taskbarHandle = [ShellTaskbarPinFinder]::FindWindow('Shell_TrayWnd', $null)
$taskbar = [System.Windows.Automation.AutomationElement]::FromHandle($taskbarHandle)
$condition = New-Object System.Windows.Automation.PropertyCondition (
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button
)
$matchingButtons = @(
    $taskbar.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition) |
        Where-Object { $_.Current.Name -match 'Mich Startup Master' } |
        ForEach-Object { $_.Current.Name }
)

[pscustomobject]@{
    PinnedFileCreated = Test-Path -LiteralPath $pinnedShortcutPath
    LiveTaskbarButtons = $matchingButtons -join ' | '
} | Format-List
