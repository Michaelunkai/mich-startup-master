$ErrorActionPreference = 'Stop'

$processes = Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'"
foreach ($row in $processes) {
    $process = Get-Process -Id $row.ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        continue
    }
    [void]$process.CloseMainWindow()
    if (-not $process.WaitForExit(5000)) {
        Stop-Process -Id $process.Id -Force
    }
}

Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'" |
    Select-Object ProcessId, ExecutablePath, CommandLine |
    Format-List
