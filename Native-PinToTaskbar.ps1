$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NativeTaskbarPin
{
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr ShellExecute(
        IntPtr hwnd,
        string operation,
        string file,
        string parameters,
        string directory,
        int showCommand);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string windowName);
}
'@

$programsDirectory = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs'
$shortcutPath = Join-Path $programsDirectory 'Mich Startup Master.lnk'
$pinnedShortcutPath = Join-Path $env:APPDATA 'Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\Mich Startup Master.lnk'

if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) {
    throw "Start Menu shortcut was not found: $shortcutPath"
}

if (Test-Path -LiteralPath $pinnedShortcutPath -PathType Leaf) {
    Remove-Item -LiteralPath $pinnedShortcutPath -Force
}

$result = [NativeTaskbarPin]::ShellExecute(
    [IntPtr]::Zero,
    'taskbarpin',
    $shortcutPath,
    $null,
    $programsDirectory,
    0
)

$status = $result.ToInt64()
if ($status -le 32) {
    throw "Windows rejected the native taskbar pin command. ShellExecute status: $status"
}

Start-Sleep -Seconds 3
$taskbarHandle = [NativeTaskbarPin]::FindWindow('Shell_TrayWnd', $null)
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
if ($matchingButtons.Count -eq 0) {
    throw 'Windows accepted the pin command but did not create a Mich Startup Master taskbar button.'
}

[pscustomobject]@{
    ShellExecuteStatus = $status
    TaskbarButtons = $matchingButtons -join ' | '
} | Format-List
