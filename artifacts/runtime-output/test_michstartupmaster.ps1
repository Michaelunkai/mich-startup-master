$ErrorActionPreference='Stop'
$App='F:\study\Windows\Applications\StartupManager\MichStartupMaster\build\MichStartupMaster.exe'
if(-not (Test-Path -LiteralPath $App)){ throw "missing exe $App" }
$pr=Start-Process -FilePath $App -ArgumentList '--smoke' -Wait -PassThru
"SMOKE exit=$($pr.ExitCode)"
if($pr.ExitCode -ne 0){ exit $pr.ExitCode }
$pr=Start-Process -FilePath $App -ArgumentList '--add-test-task' -Wait -PassThru
"ADD exit=$($pr.ExitCode)"
if($pr.ExitCode -ne 0){ exit $pr.ExitCode }
$task = (schtasks.exe /Query /FO CSV /V | ConvertFrom-Csv | Where-Object { $_.TaskName -like '\MichStartupMaster\HermesSmoke-*' } | Select-Object -First 1)
if(-not $task){ throw 'test task not found after add' }
$taskName=$task.TaskName
"TASK=$taskName Schedule=$($task.Schedule) State=$($task.'Scheduled Task State') Action=$($task.'Task To Run')"
[xml]$xml = (schtasks.exe /Query /TN $taskName /XML) -join "`n"
$ns=New-Object System.Xml.XmlNamespaceManager($xml.NameTable); $ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
$hasLogon = $null -ne $xml.SelectSingleNode('//t:LogonTrigger',$ns)
$hasDelay = $null -ne $xml.SelectSingleNode('//t:LogonTrigger/t:Delay',$ns)
"XML hasLogon=$hasLogon hasDelay=$hasDelay"
if(-not $hasLogon -or $hasDelay){ throw 'task logon/no-delay check failed' }
$shortName = ($taskName -replace '^\\MichStartupMaster\\','')
$pr=Start-Process -FilePath $App -ArgumentList @('--remove-task', $shortName) -Wait -PassThru
"REMOVE exit=$($pr.ExitCode)"
if($pr.ExitCode -ne 0){ exit $pr.ExitCode }
$oldEap=$ErrorActionPreference; $ErrorActionPreference='Continue'
$left = schtasks.exe /Query /TN $taskName 2>$null
$code=$LASTEXITCODE; $ErrorActionPreference=$oldEap
if($code -eq 0){ throw 'test task still exists after remove' }
"REMOVE verified missing"
