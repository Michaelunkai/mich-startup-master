$ErrorActionPreference = 'Stop'

$legacyExe = 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe'
if (-not (Test-Path -LiteralPath $legacyExe -PathType Leaf)) {
    throw "Legacy executable was not found: $legacyExe"
}

& $legacyExe --version
if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {
    throw "Legacy version command failed with exit code $LASTEXITCODE"
}

Start-Process -FilePath $legacyExe -WorkingDirectory (Split-Path -Parent $legacyExe)
Start-Sleep -Seconds 2
$process = Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'" |
    Where-Object { $_.ExecutablePath -eq $legacyExe } |
    Select-Object -First 1
if ($null -eq $process) {
    throw 'Legacy executable did not start.'
}

[pscustomobject]@{
    VersionCommand = 'passed'
    ProcessId = $process.ProcessId
    ExecutablePath = $process.ExecutablePath
    CommandLine = $process.CommandLine
} | Format-List
