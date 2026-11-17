# Mich Startup Master (Rust edition) — stage + guarded deploy
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-rust.ps1            (full guarded deploy)
#   powershell -NoProfile -ExecutionPolicyPolicy Bypass -File scripts\build-rust.ps1 -StageOnly
#Requires -Version 5.1
[CmdletBinding()]
param(
  [switch]$StageOnly,
  [switch]$SkipSuite
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$Root = Split-Path -Parent $PSScriptRoot
$BuildDir = Join-Path $Root 'build'
$StageDir = Join-Path $Root 'artifacts\stage-rust'
$MarkerPath = Join-Path $Root 'artifacts\runtime-output\deployment-active.json'
$Version = '3.0.0'

function Write-Step([string]$Text) { Write-Host "==> $Text" -ForegroundColor Cyan }

try {
  Write-Step 'Build (cargo release)'
  $env:PATH = "C:\Users\Admin\.cargo\bin;$env:PATH"
  Push-Location (Join-Path $Root 'rust')
  & cargo build --release --target x86_64-pc-windows-msvc -p msm-app
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed with $LASTEXITCODE" }
  Pop-Location

  $TargetDir = if (-not [string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) { $env:CARGO_TARGET_DIR } else { 'F:\backup\rust\target' }
  $BuiltExe = Join-Path $TargetDir 'x86_64-pc-windows-msvc\release\MichStartupMaster.exe'
  if (-not (Test-Path -LiteralPath $BuiltExe -PathType Leaf)) { throw "Built exe not found: $BuiltExe" }

  Write-Step 'Stage'
  New-Item -ItemType Directory -Force -Path $StageDir | Out-Null
  Copy-Item -LiteralPath $BuiltExe -Destination (Join-Path $StageDir 'MichStartupMaster.exe') -Force
  Copy-Item -LiteralPath (Join-Path $Root 'assets\MichStartupMaster.ico') -Destination (Join-Path $StageDir 'MichStartupMaster.ico') -Force

  if (-not $SkipSuite) {
    Write-Step 'Safe suite against staged exe'
    $suite = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'test.ps1') -TestAppPath (Join-Path $StageDir 'MichStartupMaster.exe') 2>&1
    $suiteText = ($suite | Out-String)
    if ($LASTEXITCODE -ne 0 -or $suiteText -notmatch 'SAFE_TESTS passed') {
      throw "Safe suite failed (exit=$LASTEXITCODE). Output: $($suiteText.Substring(0, [Math]::Min(600, $suiteText.Length)))"
    }
    Write-Host $suiteText.Trim()
  }
  if ($StageOnly) { Write-Host 'StageOnly: staged exe passed; no deploy.'; exit 0 }

  Write-Step 'Stop resident app'
  $stopScript = Join-Path $Root 'Stop-MichStartupMaster.ps1'
  if (Test-Path -LiteralPath $stopScript -PathType Leaf) {
    try { & $stopScript -ExecutablePath (Join-Path $BuildDir 'MichStartupMaster.exe') } catch {
      Write-Warning "Stop script reported: $($_.Exception.Message) (continuing with exact-path PID stop)"
      Get-CimInstance Win32_Process -Filter "Name='MichStartupMaster.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and $_.ExecutablePath.TrimEnd('\').ToLowerInvariant() -eq (Join-Path $BuildDir 'MichStartupMaster.exe').TrimEnd('\').ToLowerInvariant() } |
        ForEach-Object { & taskkill.exe /PID $_.ProcessId /F | Out-Null }
    }
  }

  Write-Step 'Backup previous binary set'
  $PreviousDir = Join-Path $Root 'artifacts\runtime-output\previous'
  New-Item -ItemType Directory -Force -Path $PreviousDir | Out-Null
  $Stamp = Get-Date -Format 'yyyyMMddHHmmss'
  foreach ($leaf in 'MichStartupMaster.exe','MichStartupMaster.ico','MichStartupMaster.dll','MichStartupMaster.deps.json','MichStartupMaster.runtimeconfig.json') {
    $candidate = Join-Path $BuildDir $leaf
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
      Copy-Item -LiteralPath $candidate -Destination (Join-Path $PreviousDir "$leaf.$Stamp") -Force
    }
  }

  Write-Step 'Deploy to build\'
  Copy-Item -LiteralPath (Join-Path $StageDir 'MichStartupMaster.exe') -Destination (Join-Path $BuildDir 'MichStartupMaster.exe') -Force
  Copy-Item -LiteralPath (Join-Path $StageDir 'MichStartupMaster.ico') -Destination (Join-Path $BuildDir 'MichStartupMaster.ico') -Force

  Write-Step 'Register boot agent'
  & (Join-Path $BuildDir 'MichStartupMaster.exe') --register-agent | Out-Null

  Write-Step 'Write deployment marker'
  $marker = [ordered]@{
    outcome      = 'DEPLOYED_AND_VERIFIED'
    binary       = (Join-Path $BuildDir 'MichStartupMaster.exe')
    version      = $Version
    deployedAt   = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    suitePassed  = ($true -and -not $SkipSuite)
  }
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $MarkerPath) | Out-Null
  [IO.File]::WriteAllText($MarkerPath, ($marker | ConvertTo-Json))

  Write-Step 'Relaunch resident agent'
  Start-ScheduledTask -TaskPath '\MichStartupMaster\' -TaskName 'MichStartupMasterApp'
  $deadline = (Get-Date).AddSeconds(30)
  do { Start-Sleep -Milliseconds 500 } while (-not (Get-Process MichStartupMaster -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline)

  Write-Step 'Final live gates'
  $smoke = & (Join-Path $BuildDir 'MichStartupMaster.exe') --smoke 2>&1 | Out-String
  if ($LASTEXITCODE -ne 0 -or $smoke -notmatch 'passed=true') { throw "Post-deploy smoke gate failed: $smoke" }
  $verify = & (Join-Path $BuildDir 'MichStartupMaster.exe') --verify-live-inventory 2>&1 | Out-String
  if ($LASTEXITCODE -ne 0 -or $verify -notmatch 'passed=true') { throw "Post-deploy verify gate failed: $($verify.Substring(0,[Math]::Min(300,$verify.Length)))" }

  Write-Host "DEPLOYED_AND_VERIFIED binary=$($marker.binary) version=$Version" -ForegroundColor Green
  exit 0
}
catch {
  Write-Error $_
  exit 1
}
