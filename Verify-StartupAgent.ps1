$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class MichStartupMasterWindows
{
    public delegate bool EnumWindowsProc(IntPtr handle, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr handle);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr handle, out uint processId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr handle, StringBuilder text, int maxCount);
}
'@

$expectedExe = 'C:\Users\micha\AppData\Local\Programs\MichStartupMaster\MichStartupMaster.exe'
$expectedLauncher = 'C:\Users\micha\AppData\Local\Programs\MichStartupMaster\MichStartupMasterAgent.vbs'
$taskName = '\MichStartupMaster\MichStartupMasterApp'
$processes = @(
    Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'" |
        Where-Object {
            $_.ExecutablePath -eq $expectedExe -and
            $_.CommandLine -match '--agent'
        }
)

if ($processes.Count -ne 1) {
    throw "Expected one local --agent process, found $($processes.Count)."
}

$visibleWindows = New-Object System.Collections.Generic.List[string]
$callback = [MichStartupMasterWindows+EnumWindowsProc]{
    param($handle, $unused)
    [uint32]$processId = 0
    [void][MichStartupMasterWindows]::GetWindowThreadProcessId($handle, [ref]$processId)
    if ($processId -eq [uint32]$processes[0].ProcessId -and [MichStartupMasterWindows]::IsWindowVisible($handle)) {
        $text = New-Object System.Text.StringBuilder 512
        [void][MichStartupMasterWindows]::GetWindowText($handle, $text, $text.Capacity)
        $visibleWindows.Add($text.ToString())
    }
    return $true
}
[void][MichStartupMasterWindows]::EnumWindows($callback, [IntPtr]::Zero)

$task = schtasks.exe /Query /TN $taskName /FO LIST /V | Out-String
if ($task -notmatch 'Scheduled Task State:\s+Enabled') {
    throw 'Scheduled task is not enabled.'
}
if ($task -notmatch [regex]::Escape($expectedLauncher)) {
    throw 'Scheduled task does not point to the hidden local launcher.'
}
if ($visibleWindows.Count -ne 0) {
    throw "Agent exposed visible windows: $($visibleWindows -join ' | ')"
}

[pscustomobject]@{
    AgentProcess = $processes[0].ProcessId
    Executable = $processes[0].ExecutablePath
    CommandLine = $processes[0].CommandLine
    VisibleWindows = $visibleWindows.Count
    ScheduledTaskEnabled = $true
} | Format-List
