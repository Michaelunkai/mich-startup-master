$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$installDirectory = Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster'
$exe = Join-Path $installDirectory 'MichStartupMaster.exe'
$sourceIcon = Join-Path $PSScriptRoot 'assets\MichStartupMaster.ico'
$runtimeIcon = Join-Path $installDirectory 'MichStartupMaster.ico'
$taskbarIcon = Join-Path $installDirectory 'MichStartupMaster-taskbar-v3.ico'
$programsDirectory = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs'
$shortcutPath = Join-Path $programsDirectory 'Mich Startup Master.lnk'

foreach ($path in @($exe, $sourceIcon)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required path was not found: $path"
    }
}

Copy-Item -LiteralPath $sourceIcon -Destination $runtimeIcon -Force
Copy-Item -LiteralPath $sourceIcon -Destination $taskbarIcon -Force

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $exe
$shortcut.Arguments = ''
$shortcut.WorkingDirectory = $installDirectory
$shortcut.IconLocation = "$taskbarIcon,0"
$shortcut.Description = 'Mich Startup Master'
$shortcut.Save()

$taskPath = '\MichStartupMaster\'
$taskName = 'MichStartupMasterApp'
$action = New-ScheduledTaskAction -Execute $exe -Argument '--agent' -WorkingDirectory $installDirectory
$trigger = New-ScheduledTaskTrigger -AtLogOn
$principal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 0)
Register-ScheduledTask -TaskPath $taskPath -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description 'Mich Startup Master managed startup agent' -Force | Out-Null
Disable-ScheduledTask -TaskPath $taskPath -TaskName $taskName | Out-Null

$icon = New-Object System.Drawing.Icon $taskbarIcon
try {
    [pscustomobject]@{
        InstallDirectory = $installDirectory
        Executable = $exe
        StartMenuShortcut = $shortcutPath
        IconLocation = "$taskbarIcon,0"
        IconSize = "$($icon.Width)x$($icon.Height)"
        Task = "$taskPath$taskName"
        TaskEnabled = $false
    } | Format-List
} finally {
    $icon.Dispose()
}
