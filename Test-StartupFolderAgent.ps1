$ErrorActionPreference = 'Stop'

$shortcutPath = Join-Path ([Environment]::GetFolderPath('Startup')) 'Mich Startup Master Agent.lnk'
Start-Process -FilePath $shortcutPath
Start-Sleep -Seconds 2

Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'" |
    Where-Object { $_.CommandLine -match '--agent' } |
    Select-Object -First 1 |
    ForEach-Object {
        $runtime = Get-Process -Id $_.ProcessId
        if ($runtime.MainWindowHandle -ne 0) {
            throw "Startup agent exposed a main window: $($runtime.MainWindowTitle)"
        }
        $_ | Select-Object ProcessId, ParentProcessId, ExecutablePath, CommandLine, CreationDate
    } |
    Format-List
