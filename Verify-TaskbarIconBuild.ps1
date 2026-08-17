$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$source = 'C:\MichStartupMaster-GitHubRecovery\src\MichStartupMaster.cs'
$exe = 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe'
$iconPath = 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster-taskbar-v3.ico'
$pinnedShortcutPath = 'C:\Users\micha\AppData\Roaming\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\Mich Startup Master.lnk'

$sourceText = [System.IO.File]::ReadAllText($source)
if ($sourceText -match 'SetCurrentProcessExplicitAppUserModelID|AppUserModelId') {
    throw 'The custom AppUserModelID is still present in the deployed source.'
}

$exeIcon = [System.Drawing.Icon]::ExtractAssociatedIcon($exe)
$taskbarIcon = New-Object System.Drawing.Icon $iconPath
try {
    $shell = New-Object -ComObject WScript.Shell
    $pinnedShortcut = $shell.CreateShortcut($pinnedShortcutPath)
    $expectedIconLocation = "$iconPath,0"
    if ($pinnedShortcut.TargetPath -ne $exe) {
        throw "Pinned target mismatch: $($pinnedShortcut.TargetPath)"
    }
    if ($pinnedShortcut.IconLocation -ne $expectedIconLocation) {
        throw "Pinned icon mismatch: $($pinnedShortcut.IconLocation)"
    }
    $running = @(Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'")
    if ($running.Count -ne 0) {
        throw 'MichStartupMaster is running while offline verification was requested.'
    }

    [pscustomobject]@{
        CustomAppUserModelIdRemoved = $true
        ExecutableIcon = "$($exeIcon.Width)x$($exeIcon.Height)"
        TaskbarIcon = "$($taskbarIcon.Width)x$($taskbarIcon.Height)"
        PinnedTarget = $pinnedShortcut.TargetPath
        PinnedIconLocation = $pinnedShortcut.IconLocation
        RunningInstances = $running.Count
    } | Format-List
} finally {
    if ($null -ne $exeIcon) { $exeIcon.Dispose() }
    if ($null -ne $taskbarIcon) { $taskbarIcon.Dispose() }
}
