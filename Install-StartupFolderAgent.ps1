$ErrorActionPreference = 'Stop'

$installDirectory = Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster'
$exe = Join-Path $installDirectory 'MichStartupMaster.exe'
$icon = Join-Path $installDirectory 'MichStartupMaster-taskbar-v3.ico'
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Startup')) 'Mich Startup Master Agent.lnk'

foreach ($path in @($exe, $icon)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required local production file was not found: $path"
    }
}

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $exe
$shortcut.Arguments = '--agent'
$shortcut.WorkingDirectory = $installDirectory
$shortcut.IconLocation = "$icon,0"
$shortcut.Description = 'Mich Startup Master hidden startup agent'
$shortcut.Save()

Disable-ScheduledTask -TaskPath '\MichStartupMaster\' -TaskName 'MichStartupMasterApp' | Out-Null

[pscustomobject]@{
    StartupShortcut = $shortcutPath
    Target = $exe
    Arguments = '--agent'
    IconLocation = "$icon,0"
    ScheduledTaskDisabled = $true
} | Format-List
