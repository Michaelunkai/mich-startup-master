$ErrorActionPreference = 'Stop'

$programsDirectory = Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs'
$shortcutPath = Join-Path $programsDirectory 'Mich Startup Master.lnk'
$pinnedDirectory = Join-Path $env:APPDATA 'Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar'
$pinnedShortcutPath = Join-Path $pinnedDirectory 'Mich Startup Master.lnk'
$exe = 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe'
$workingDirectory = Split-Path -Parent $exe
$taskbarIcon = Join-Path $workingDirectory 'MichStartupMaster-taskbar-v3.ico'

if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) {
    throw "Start Menu shortcut was not found: $shortcutPath"
}

$shell = New-Object -ComObject Shell.Application
$folder = $shell.Namespace($programsDirectory)
$item = $folder.ParseName((Split-Path -Leaf $shortcutPath))
$verbs = @($item.Verbs() | ForEach-Object { $_.Name.Replace('&', '').Trim() })
$pinVerb = $item.Verbs() | Where-Object { $_.Name.Replace('&', '').Trim() -match '(?i)pin to taskbar' } | Select-Object -First 1

if ($null -ne $pinVerb) {
    $pinVerb.DoIt()
    Start-Sleep -Seconds 2
    $result = 'native pin command invoked'
} else {
    $result = 'no native pin verb exposed'
}

$wscript = New-Object -ComObject WScript.Shell
$pinnedShortcut = if (Test-Path -LiteralPath $pinnedShortcutPath -PathType Leaf) {
    $wscript.CreateShortcut($pinnedShortcutPath)
} else {
    $null
}
if ($null -ne $pinnedShortcut) {
    $pinnedShortcut.TargetPath = $exe
    $pinnedShortcut.Arguments = ''
    $pinnedShortcut.WorkingDirectory = $workingDirectory
    $pinnedShortcut.IconLocation = "$taskbarIcon,0"
    $pinnedShortcut.Save()
}

[pscustomobject]@{
    Shortcut = $shortcutPath
    PinResult = $result
    NativePinPresent = $null -ne $pinnedShortcut
    PinnedTarget = if ($null -eq $pinnedShortcut) { '' } else { $pinnedShortcut.TargetPath }
    PinnedStartIn = if ($null -eq $pinnedShortcut) { '' } else { $pinnedShortcut.WorkingDirectory }
    PinnedIconLocation = if ($null -eq $pinnedShortcut) { '' } else { $pinnedShortcut.IconLocation }
    AvailableVerbs = $verbs -join ' | '
} | Format-List
