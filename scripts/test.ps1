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
