$ErrorActionPreference='Continue'
schtasks.exe /Delete /F /TN '\MichStartupMaster\HermesSmoke-20260605020101' 2>$null | Out-Null
exit 0
