Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'" |
    Select-Object ProcessId, ParentProcessId, ExecutablePath, CommandLine, CreationDate |
    Format-List
