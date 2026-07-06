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
if($count -lt 20){ throw "Startup inventory too small: $count" }
foreach($needle in @('Autorun_current_ahk','AIMemoryBoost','FullScreenSnip','TVStartupCheck')){
  if($text -notlike "*$needle*"){ throw "Expected startup item missing from app list: $needle" }
  "FOUND $needle"
}
foreach($source in @('Windows Service','System Driver','Registry Run','Scheduled Task','Startup Command')){
  if($text -notlike "*`"source`":`"$source`"*"){ throw "Expected startup source missing from app list: $source" }
  "FOUND_SOURCE $source"
}
$items = $text | ConvertFrom-Json
if(@($items | Where-Object { [string]::IsNullOrWhiteSpace($_.appName) }).Count -ne 0){ throw 'Every startup item must have a human-readable appName' }
$cleanupItems = @($items | Where-Object { $_.advice -eq 'Cleanup' })
"CLEANUP_SUGGESTIONS count=$($cleanupItems.Count)"
if(@($cleanupItems | Where-Object { $_.risk -eq 'Critical' }).Count -ne 0){ throw 'High-risk startup items must never be marked as cleanup suggestions' }
$malwarebytesInstalled = (Get-CimInstance Win32_Service | Where-Object { $_.Name -match 'MBAM|Malware' -or $_.DisplayName -match 'Malwarebytes' -or $_.PathName -match 'Malwarebytes' }) -or (Get-CimInstance Win32_SystemDriver | Where-Object { $_.Name -match 'MBAM|Malware|mbam' -or $_.DisplayName -match 'Malwarebytes|MBAM' -or $_.PathName -match 'Malwarebytes' })
if($malwarebytesInstalled){
  foreach($needle in @('Malwarebytes Service','MBAMChameleon','MbamElam')){
    if($text -notlike "*$needle*"){ throw "Expected Malwarebytes startup item missing from app list: $needle" }
    "FOUND_MALWAREBYTES_STARTUP $needle"
  }
}
$systemDriver = @($items | Where-Object { $_.source -eq 'System Driver' -and $_.risk -eq 'Critical' } | Select-Object -First 1)[0]
if(-not $systemDriver){ throw 'At least one boot/system/auto driver must be marked high-risk' }
"FOUND_HIGH_RISK_DRIVER $($systemDriver.name)"
$normalApp = @($items | Where-Object { $_.name -eq 'FullScreenSnip' -and $_.source -eq 'Registry Run' })[0]
if($normalApp -and $normalApp.risk -eq 'Critical'){ throw 'Normal user app startup must not be marked high-risk' }
if($normalApp -and $normalApp.advice -eq 'Cleanup'){ throw 'Normal user app startup must not be marked as suggested cleanup' }

$testService = 'MichStartupMasterTestSvc'
try {
  sc.exe delete $testService | Out-Null
  Start-Sleep -Milliseconds 500
  $bin = '"' + $App + '" --smoke'
  sc.exe create $testService binPath= $bin start= auto DisplayName= $testService | Out-Null
  if($LASTEXITCODE -ne 0){ throw "Could not create test service $testService" }
  $items = & $App --list | ConvertFrom-Json
  $svc = @($items | Where-Object { $_.name -eq $testService -and $_.source -eq 'Windows Service' })[0]
  if(-not $svc -or $svc.enabled -ne $true){ throw 'Disposable auto-start service was not visible as enabled' }
  $p = Start-Process -FilePath $App -ArgumentList @('--set-enabled', $testService, 'false') -Wait -PassThru
  if($p.ExitCode -ne 0){ throw "Disabling disposable service failed: $($p.ExitCode)" }
  $start = (Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$testService").Start
  "SERVICE_TOGGLE disabledStart=$start"
  if($start -ne 4){ throw "Disposable service was not disabled, Start=$start" }
  $p = Start-Process -FilePath $App -ArgumentList @('--set-enabled', $testService, 'true') -Wait -PassThru
  if($p.ExitCode -ne 0){ throw "Enabling disposable service failed: $($p.ExitCode)" }
  $start = (Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$testService").Start
  "SERVICE_TOGGLE restoredStart=$start"
  if($start -ne 2){ throw "Disposable service was not restored to auto-start, Start=$start" }
}
finally {
  sc.exe delete $testService | Out-Null
}
Add-Type -AssemblyName System.Drawing
$icon=[System.Drawing.Icon]::ExtractAssociatedIcon($App)
if($null -eq $icon){ throw 'EXE icon extraction failed' }
"ICON extracted size=$($icon.Width)x$($icon.Height)"
$icon.Dispose()

# UI/contract regression for the Popup column and one-click popup state toggle.
$contractFile = Join-Path $Root 'artifacts\runtime-output\ui-contract.json'
$p = Start-Process -FilePath $App -ArgumentList '--ui-contract' -RedirectStandardOutput $contractFile -Wait -PassThru
"UI_CONTRACT exit=$($p.ExitCode)"
if($p.ExitCode -ne 0){ exit $p.ExitCode }
$contract = Get-Content -LiteralPath $contractFile -Raw | ConvertFrom-Json
$columns = @($contract.columns)
$locationIndex = [array]::IndexOf($columns, 'Location')
$popupIndex = [array]::IndexOf($columns, 'Popup')
"UI_COLUMNS $($columns -join ',')"
if($locationIndex -lt 0 -or $popupIndex -lt 0 -or [math]::Abs($popupIndex - $locationIndex) -ne 1){ throw 'Popup column must be next to Location column' }
if($contract.popupEnabledLabel -ne 'Enabled' -or $contract.popupDisabledLabel -ne 'Disabled'){ throw 'Popup labels must be Enabled/Disabled' }
if(-not $contract.oneClickPopupToggle){ throw 'Popup state must be changeable with one click' }
if(-not $contract.humanReadableNames){ throw 'UI contract must expose human-readable names' }
if(-not $contract.greenCleanupAdvice){ throw 'UI contract must expose green cleanup advice' }
if(-not $contract.contextMenu){ throw 'UI contract must expose right-click context actions' }
if(-not $contract.keyboardShortcuts){ throw 'UI contract must expose keyboard shortcuts' }
if(@($contract.filters) -notcontains 'Suggested cleanup'){ throw 'UI must include Suggested cleanup filter' }
foreach($tool in @('Add startup','Edit startup','Remove startup','Restore startup','Launch now','Open location','Copy command')){
  if(@($contract.tools) -notcontains $tool){ throw "UI tool missing from contract: $tool" }
}

function Test-PopupToggle([string]$StartMode,[string]$ExpectedAfterFirstToggle,[string]$ExpectedAfterSecondToggle){
  $p = Start-Process -FilePath $App -ArgumentList "--add-test-task-$StartMode" -Wait -PassThru
  if($p.ExitCode -ne 0){ exit $p.ExitCode }
  $pattern = if($StartMode -eq 'tray'){ '\MichStartupMaster\HermesSmokeTray-*' } else { '\MichStartupMaster\HermesSmokeNormal-*' }
  $task = (schtasks.exe /Query /FO CSV /V | ConvertFrom-Csv | Where-Object { $_.TaskName -like $pattern } | Select-Object -First 1)
  if(-not $task){ throw "popup toggle seed task not found: $StartMode" }
  $taskName = $task.TaskName
  try {
    $p = Start-Process -FilePath $App -ArgumentList @('--toggle-popup', $taskName) -RedirectStandardOutput (Join-Path $Root "artifacts\runtime-output\toggle-$StartMode-1.txt") -Wait -PassThru
    if($p.ExitCode -ne 0){ throw "first popup toggle failed: $($p.ExitCode)" }
    [xml]$xml = (schtasks.exe /Query /TN $taskName /XML) -join "`n"
    $ns = New-Object System.Xml.XmlNamespaceManager($xml.NameTable); $ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
    $cmd = [string]$xml.SelectSingleNode('//t:Actions/t:Exec/t:Command',$ns).InnerText
    $argsNode = $xml.SelectSingleNode('//t:Actions/t:Exec/t:Arguments',$ns); $argText = if($argsNode){[string]$argsNode.InnerText}else{''}
    $actualAfterFirst = if($argText -like '--tray-run *' -or $argText -eq '--start-in-tray'){ 'Disabled' } else { 'Enabled' }
    "POPUP_TOGGLE $StartMode first=$actualAfterFirst cmd=$cmd args=$argText"
    if($actualAfterFirst -ne $ExpectedAfterFirstToggle){ throw "unexpected popup state after first toggle: $actualAfterFirst" }
    if($ExpectedAfterFirstToggle -eq 'Disabled'){
      schtasks.exe /Change /TN $taskName /TR "`"$App`" --smoke" | Out-Null
      $p = Start-Process -FilePath $App -ArgumentList '--enforce-quiet' -Wait -PassThru
      if($p.ExitCode -ne 0){ throw "quiet enforcement failed: $($p.ExitCode)" }
      [xml]$xml = (schtasks.exe /Query /TN $taskName /XML) -join "`n"
      $ns = New-Object System.Xml.XmlNamespaceManager($xml.NameTable); $ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
      $argsNode = $xml.SelectSingleNode('//t:Actions/t:Exec/t:Arguments',$ns); $argText = if($argsNode){[string]$argsNode.InnerText}else{''}
      if(-not ($argText -like '--tray-run *' -or $argText -eq '--start-in-tray')){ throw "quiet enforcement did not restore wrapper args: $argText" }
      "QUIET_ENFORCE restored args=$argText"
    }

    $p = Start-Process -FilePath $App -ArgumentList @('--toggle-popup', $taskName) -RedirectStandardOutput (Join-Path $Root "artifacts\runtime-output\toggle-$StartMode-2.txt") -Wait -PassThru
    if($p.ExitCode -ne 0){ throw "second popup toggle failed: $($p.ExitCode)" }
    [xml]$xml = (schtasks.exe /Query /TN $taskName /XML) -join "`n"
    $ns = New-Object System.Xml.XmlNamespaceManager($xml.NameTable); $ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
    $argsNode = $xml.SelectSingleNode('//t:Actions/t:Exec/t:Arguments',$ns); $argText = if($argsNode){[string]$argsNode.InnerText}else{''}
    $actualAfterSecond = if($argText -like '--tray-run *' -or $argText -eq '--start-in-tray'){ 'Disabled' } else { 'Enabled' }
    "POPUP_TOGGLE $StartMode second=$actualAfterSecond args=$argText"
    if($actualAfterSecond -ne $ExpectedAfterSecondToggle){ throw "unexpected popup state after second toggle: $actualAfterSecond" }
  }
  finally {
    $shortName = ($taskName -replace '^\\MichStartupMaster\\','')
    Start-Process -FilePath $App -ArgumentList @('--remove-task', $shortName) -Wait | Out-Null
  }
}
Test-PopupToggle 'normal' 'Disabled' 'Enabled'
Test-PopupToggle 'tray' 'Enabled' 'Disabled'

function Test-ModeTask([string]$ModeArg,[string]$ExpectMode){
  $p = Start-Process -FilePath $App -ArgumentList $ModeArg -Wait -PassThru
  "ADD $ExpectMode exit=$($p.ExitCode)"
  if($p.ExitCode -ne 0){ exit $p.ExitCode }
  $pattern = if($ExpectMode -eq 'tray'){ '\MichStartupMaster\HermesSmokeTray-*' } else { '\MichStartupMaster\HermesSmokeNormal-*' }
  $task = (schtasks.exe /Query /FO CSV /V | ConvertFrom-Csv | Where-Object { $_.TaskName -like $pattern } | Select-Object -First 1)
  if(-not $task){ throw "test task not found after add: $ExpectMode" }
  $taskName = $task.TaskName
  [xml]$xml = (schtasks.exe /Query /TN $taskName /XML) -join "`n"
  $ns = New-Object System.Xml.XmlNamespaceManager($xml.NameTable)
  $ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
  $hasLogon = $null -ne $xml.SelectSingleNode('//t:LogonTrigger',$ns)
  $hasDelay = $null -ne $xml.SelectSingleNode('//t:LogonTrigger/t:Delay',$ns)
  $cmd = [string]$xml.SelectSingleNode('//t:Actions/t:Exec/t:Command',$ns).InnerText
  $argsNode = $xml.SelectSingleNode('//t:Actions/t:Exec/t:Arguments',$ns)
  $argText = if($argsNode){[string]$argsNode.InnerText}else{''}
  "TASK $ExpectMode=$taskName hasLogon=$hasLogon hasDelay=$hasDelay commandExists=$(Test-Path -LiteralPath $cmd) cmd=$cmd args=$argText"
  if(-not $hasLogon -or $hasDelay -or -not (Test-Path -LiteralPath $cmd)){ throw "task XML validation failed: $ExpectMode" }
  if($ExpectMode -eq 'tray'){
    if($cmd -ne $App -or -not ($argText -like '--tray-run *' -or $argText -eq '--start-in-tray')){ throw 'tray mode did not use Startup Master quiet launcher' }
  } else {
    if($cmd -ne $App -or $argText -like '--tray-run *' -or $argText -eq '--start-in-tray'){ throw 'normal mode did not run target directly' }
  }
  $shortName = ($taskName -replace '^\\MichStartupMaster\\','')
  $p = Start-Process -FilePath $App -ArgumentList @('--remove-task', $shortName) -Wait -PassThru
  "REMOVE $ExpectMode exit=$($p.ExitCode)"
  if($p.ExitCode -ne 0){ exit $p.ExitCode }
  $old = $ErrorActionPreference; $ErrorActionPreference='Continue'
  schtasks.exe /Query /TN $taskName 2>$null | Out-Null
  $code = $LASTEXITCODE
  $ErrorActionPreference = $old
  if($code -eq 0){ throw "test task still exists after remove: $ExpectMode" }
  "REMOVE $ExpectMode verified missing"
}
Test-ModeTask '--add-test-task-tray' 'tray'
Test-ModeTask '--add-test-task-normal' 'normal'

function Get-TaskActionXml([string]$TaskName){
  [xml]$xml = (schtasks.exe /Query /TN $TaskName /XML) -join "`n"
  $ns = New-Object System.Xml.XmlNamespaceManager($xml.NameTable)
  $ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
  $cmd = [string]$xml.SelectSingleNode('//t:Actions/t:Exec/t:Command',$ns).InnerText
  $argsNode = $xml.SelectSingleNode('//t:Actions/t:Exec/t:Arguments',$ns)
  $argText = if($argsNode){[string]$argsNode.InnerText}else{''}
  [pscustomobject]@{ Command = $cmd; Arguments = $argText; HasLogon = ($null -ne $xml.SelectSingleNode('//t:LogonTrigger',$ns)); HasDelay = ($null -ne $xml.SelectSingleNode('//t:LogonTrigger/t:Delay',$ns)) }
}

function Remove-ProofTask([string]$TaskName){
  if([string]::IsNullOrWhiteSpace($TaskName)){ return }
  $shortName = ($TaskName -replace '^\\MichStartupMaster\\','')
  Start-Process -FilePath $App -ArgumentList @('--remove-task', $shortName) -Wait | Out-Null
}

function Quote-ProcessArg([string]$Value){
  if($null -eq $Value){ $Value = '' }
  '"' + ($Value -replace '"','\"') + '"'
}

function Test-ArbitraryStartupTarget([string]$Name,[string]$Target,[string]$Mode,[scriptblock]$ValidateAction){
  $taskName = "\MichStartupMaster\$Name"
  try {
    Remove-ProofTask $taskName
    $out = Join-Path $Root "artifacts\runtime-output\add-$Name.txt"
    $argLine = @('--add-startup', $Name, $Target, $Mode) | ForEach-Object { Quote-ProcessArg $_ }
    $p = Start-Process -FilePath $App -ArgumentList ($argLine -join ' ') -RedirectStandardOutput $out -Wait -PassThru
    "ADD_STARTUP_PROOF $Name mode=$Mode exit=$($p.ExitCode)"
    if($p.ExitCode -ne 0){ throw "add-startup failed for $Name" }
    $action = Get-TaskActionXml $taskName
    "TASK_ACTION $Name command=$($action.Command) args=$($action.Arguments)"
    if(-not $action.HasLogon -or $action.HasDelay){ throw "startup task trigger invalid for $Name" }
    & $ValidateAction $action
  }
  finally { Remove-ProofTask $taskName }
}

$piper = 'F:\study\AI_ML\AI_and_Machine_Learning\Artificial_Intelligence\Speech\Windows\Dictation\Tray\PiperVoicePaste\PiperVoicePaste.exe'
if(-not (Test-Path -LiteralPath $piper)){ throw "PiperVoicePaste proof target missing: $piper" }
Test-ArbitraryStartupTarget 'PiperVoicePaste-CodexProof-Tray' $piper 'tray' {
  param($action)
  if($action.Command -ne $App){ throw 'Piper tray startup must execute MichStartupMaster tray wrapper' }
  if($action.Arguments -notlike '--tray-run *'){ throw "Piper tray startup did not use encoded tray payload: $($action.Arguments)" }
}
Test-ArbitraryStartupTarget 'PiperVoicePaste-CodexProof-Normal' $piper 'normal' {
  param($action)
  if($action.Command -ne $piper){ throw "Piper normal startup must execute exact PiperVoicePaste.exe, got $($action.Command)" }
}

$proofDir = Join-Path $Root 'artifacts\runtime-output\launcher-targets'
New-Item -ItemType Directory -Force -Path $proofDir | Out-Null
$ps1 = Join-Path $proofDir 'startup proof script.ps1'
$cmd = Join-Path $proofDir 'startup proof command.cmd'
Set-Content -LiteralPath $ps1 -Value '$null = "startup proof"' -Encoding UTF8
Set-Content -LiteralPath $cmd -Value '@echo off' -Encoding ASCII
Test-ArbitraryStartupTarget 'Ps1-Startup-Proof' $ps1 'normal' {
  param($action)
  $combined = "$($action.Command) $($action.Arguments)"
  if($combined -notlike '*\WindowsPowerShell\v1.0\powershell.exe*'){ throw "ps1 startup must preserve a PowerShell execution route, got $combined" }
  if($action.Arguments -notlike '*startup proof script.ps1*'){ throw "ps1 startup arguments missing target: $($action.Arguments)" }
}
Test-ArbitraryStartupTarget 'Cmd-Startup-Proof' $cmd 'normal' {
  param($action)
  if($action.Command -notlike '*\System32\cmd.exe'){ throw "cmd startup must use cmd host, got $($action.Command)" }
  if($action.Arguments -notlike '*startup proof command.cmd*'){ throw "cmd startup arguments missing target: $($action.Arguments)" }
}
