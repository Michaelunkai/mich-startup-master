$events = Get-WinEvent -FilterHashtable @{
    LogName = 'Application'
    StartTime = (Get-Date).AddMinutes(-10)
} | Where-Object {
    $_.Message -match 'MichStartupMaster'
}

$events | Select-Object -First 5 TimeCreated, ProviderName, Id, LevelDisplayName, Message | Format-List
