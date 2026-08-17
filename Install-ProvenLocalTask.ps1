$ErrorActionPreference = 'Stop'

$taskName = '\MichStartupMaster\MichStartupMasterApp'
$installDirectory = Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster'
$exe = Join-Path $installDirectory 'MichStartupMaster.exe'
$launcher = Join-Path $installDirectory 'MichStartupMasterAgent.vbs'
$wscript = 'C:\Windows\System32\wscript.exe'
$userSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$xmlPath = Join-Path $env:TEMP 'MichStartupMaster-local-task.xml'

foreach ($path in @($exe, $launcher, $wscript)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required local startup path was not found: $path"
    }
}

$escapedWscript = [System.Security.SecurityElement]::Escape($wscript)
$escapedLauncher = [System.Security.SecurityElement]::Escape($launcher)
$escapedDirectory = [System.Security.SecurityElement]::Escape($installDirectory)
$xml = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Mich Startup Master managed startup agent</Description>
    <URI>\MichStartupMaster\MichStartupMasterApp</URI>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>$userSid</UserId>
      <LogonType>InteractiveToken</LogonType>
    </Principal>
  </Principals>
  <Settings>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <StartWhenAvailable>true</StartWhenAvailable>
    <IdleSettings>
      <Duration>PT10M</Duration>
      <WaitTimeout>PT1H</WaitTimeout>
      <StopOnIdleEnd>true</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
  </Settings>
  <Triggers>
    <LogonTrigger />
  </Triggers>
  <Actions Context="Author">
    <Exec>
      <Command>$escapedWscript</Command>
      <Arguments>//B //Nologo &quot;$escapedLauncher&quot;</Arguments>
      <WorkingDirectory>$escapedDirectory</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"@

[System.IO.File]::WriteAllText($xmlPath, $xml, [System.Text.Encoding]::Unicode)
try {
    & schtasks.exe /Create /TN $taskName /XML $xmlPath /F
    if ($LASTEXITCODE -ne 0) {
        throw "schtasks.exe failed with exit code $LASTEXITCODE"
    }
} finally {
    Remove-Item -LiteralPath $xmlPath -Force -ErrorAction SilentlyContinue
}

schtasks.exe /Query /TN $taskName /FO LIST /V
