$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TaskbarFinder
{
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string windowName);
}
'@

$handle = [TaskbarFinder]::FindWindow('Shell_TrayWnd', $null)
if ($handle -eq [IntPtr]::Zero) {
    throw 'Windows taskbar window was not found.'
}

$taskbar = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
$condition = New-Object System.Windows.Automation.PropertyCondition (
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Button
)

$taskbar.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition) |
    ForEach-Object {
        $bounds = $_.Current.BoundingRectangle
        [pscustomobject]@{
            Name = $_.Current.Name
            AutomationId = $_.Current.AutomationId
            ClassName = $_.Current.ClassName
            Bounds = "$([int]$bounds.X),$([int]$bounds.Y) $([int]$bounds.Width)x$([int]$bounds.Height)"
        }
    } |
    Format-Table -AutoSize
