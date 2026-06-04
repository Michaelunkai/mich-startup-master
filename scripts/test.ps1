$ErrorActionPreference='Stop'
$Root = Split-Path -Path $PSScriptRoot -Parent
$App = Join-Path $Root 'build\MichStartupMaster.exe'
if(-not (Test-Path -LiteralPath $App)){ throw "Missing app: $App" }
$p = Start-Process -FilePath $App -ArgumentList '--smoke' -Wait -PassThru
"SMOKE exit=$($p.ExitCode)"
if($p.ExitCode -ne 0){ exit $p.ExitCode }
$outFile = Join-Path $Root 'artifacts\runtime-output\current-list.json'
New-Item -ItemType Directory -Force -Path (Split-Path -Path $outFile -Parent) | Out-Null
$p = Start-Process -FilePath $App -ArgumentList '--list' -RedirectStandardOutput $outFile -Wait -PassThru
if($p.ExitCode -ne 0){ throw "--list failed $($p.ExitCode)" }
$text = Get-Content -LiteralPath $outFile -Raw
$count = ([regex]::Matches($text,'\"name\"')).Count
"LIST count=$count"
foreach($needle in @('Autorun_current_ahk','AIMemoryBoost','FullScreenSnip','TVStartupCheck')){
  if($text -notlike "*$needle*"){ throw "Expected startup item missing from app list: $needle" }
  "FOUND $needle"
}
$p = Start-Process -FilePath $App -ArgumentList '--add-test-task' -Wait -PassThru
"ADD exit=$($p.ExitCode)"
if($p.ExitCode -ne 0){ exit $p.ExitCode }
$task = (schtasks.exe /Query /FO CSV /V | ConvertFrom-Csv | Where-Object { $_.TaskName -like '\MichStartupMaster\HermesSmoke-*' } | Select-Object -First 1)
if(-not $task){ throw 'test task not found after add' }
$taskName = $task.TaskName
[xml]$xml = (schtasks.exe /Query /TN $taskName /XML) -join "`n"
$ns = New-Object System.Xml.XmlNamespaceManager($xml.NameTable)
$ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
$hasLogon = $null -ne $xml.SelectSingleNode('//t:LogonTrigger',$ns)
$hasDelay = $null -ne $xml.SelectSingleNode('//t:LogonTrigger/t:Delay',$ns)
$cmd = [string]$xml.SelectSingleNode('//t:Actions/t:Exec/t:Command',$ns).InnerText
"TASK=$taskName hasLogon=$hasLogon hasDelay=$hasDelay commandExists=$(Test-Path -LiteralPath $cmd)"
if(-not $hasLogon -or $hasDelay -or -not (Test-Path -LiteralPath $cmd)){ throw 'task XML validation failed' }
$shortName = ($taskName -replace '^\\MichStartupMaster\\','')
$p = Start-Process -FilePath $App -ArgumentList @('--remove-task', $shortName) -Wait -PassThru
"REMOVE exit=$($p.ExitCode)"
if($p.ExitCode -ne 0){ exit $p.ExitCode }
$old = $ErrorActionPreference; $ErrorActionPreference='Continue'
schtasks.exe /Query /TN $taskName 2>$null | Out-Null
$code = $LASTEXITCODE
$ErrorActionPreference = $old
if($code -eq 0){ throw 'test task still exists after remove' }
'REMOVE verified missing'
