# ============================================================================
# remake.ps1 - Mich Startup Master rebuild + run everything
# ----------------------------------------------------------------------------
# Restores everything after Clean.ps1 removed the regenerable artifacts and
# brings the project back to a fully working state by invoking the project's
# OWN canonical guarded build/deploy workflow:
#
#   scripts\build.ps1            (default) stage + validate + deploy the live
#                                app at %LOCALAPPDATA%\Programs\
#                                MichStartupMaster, gracefully stop the old
#                                process, swap files, and relaunch the new
#                                executable. Recreates artifacts\build-staging
#                                and artifacts\runtime-output as it goes.
#   scripts\build.ps1 -StageOnly (-StageOnly) stage + validate only, without
#                                touching the running installation.
#
# After the build exits, this script verifies the app process is running and
# the canonical agent task (MichStartupMasterApp) is in place, then writes a
# receipt. The build's own outcome marker in the log is authoritative
# (DEPLOYED_AND_VERIFIED), because Start-Process + output redirection does not
# reliably expose the child's exit code.
#
#   powershell -ExecutionPolicy Bypass -File remake.ps1          # deploy + run
#   powershell -ExecutionPolicy Bypass -File remake.ps1 -StageOnly  # stage only
# ============================================================================
param([switch]$StageOnly)

$ErrorActionPreference = 'Continue'

$Root        = $PSScriptRoot
$BuildScript = Join-Path $Root 'scripts\build.ps1'
$BuildOut    = Join-Path $Root 'remake-build.log'
$BuildErr    = Join-Path $Root 'remake-build.err.log'
$Receipt     = Join-Path $Root 'remake.receipt'
$swTotal     = [System.Diagnostics.Stopwatch]::StartNew()

Write-Output '============================================================'
Write-Output ' remake.ps1 - Mich Startup Master rebuild + run everything'
Write-Output " Root    : $Root"
Write-Output (" Mode    : {0}" -f $(if ($StageOnly) { 'StageOnly (no deploy)' } else { 'Full guarded deployment' }))
Write-Output '============================================================'

if (-not (Test-Path -LiteralPath $BuildScript -PathType Leaf)) {
    Write-Output "[remake] ERROR: canonical build script not found: $BuildScript"
    exit 1
}

# ---- launch the canonical build/deploy (detached child, output to log) ----
Remove-Item -LiteralPath $BuildOut, $BuildErr -Force -ErrorAction SilentlyContinue

$buildArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $BuildScript)
if ($StageOnly) { $buildArgs += '-StageOnly' }

Write-Output ('[remake] Launching: powershell.exe ' + ($buildArgs -join ' '))
Write-Output "[remake] Log: $BuildOut"

$proc = Start-Process -FilePath 'powershell.exe' `
    -ArgumentList $buildArgs `
    -WorkingDirectory $Root `
    -RedirectStandardOutput $BuildOut `
    -RedirectStandardError $BuildErr `
    -WindowStyle Hidden `
    -PassThru

# ---- progress polling ------------------------------------------------------
$start     = Get-Date
$heartbeat = 0
while (-not $proc.HasExited) {
    Start-Sleep -Seconds 10
    $proc.Refresh()
    $elapsed = (Get-Date) - $start
    if ([int]$elapsed.TotalSeconds -ge ($heartbeat + 30)) {
        $heartbeat = [int]$elapsed.TotalSeconds
        $lastLine = (Get-Content -LiteralPath $BuildOut -Tail 20 -ErrorAction SilentlyContinue |
                     Where-Object { $_.Trim() } | Select-Object -Last 1)
        if ($lastLine) {
            Write-Output ("[remake] {0:mm}:{0:ss} elapsed - {1}" -f $elapsed, ($lastLine.Trim()))
        } else {
            Write-Output ("[remake] {0:mm}:{0:ss} elapsed - working..." -f $elapsed)
        }
    }
}
$proc.WaitForExit()
$elapsed = (Get-Date) - $start

# Exit code is best-effort (can come back empty with redirects).
$exitCode = $null
try { if ($proc.HasExited) { $exitCode = [int]$proc.ExitCode } } catch { }

# The build's OWN outcome line is authoritative.
$outcome = 'UNKNOWN'
if (Test-Path -LiteralPath $BuildOut) {
    $tail = (Get-Content -LiteralPath $BuildOut -Tail 300 -ErrorAction SilentlyContinue) -join "`n"
    if ($tail -match 'Outcome\s*:\s*DEPLOYED_AND_VERIFIED')          { $outcome = 'DEPLOYED_AND_VERIFIED' }
    elseif ($tail -match 'DEPLOYED_AND_VERIFIED')                     { $outcome = 'DEPLOYED_AND_VERIFIED' }
    elseif ($tail -match 'DEPLOYMENT_FAILED')                         { $outcome = 'DEPLOYMENT_FAILED' }
    elseif ($tail -match 'DEPLOYED_WITH_ISSUES|PARTIAL|ROLLBACK')     { $outcome = 'PARTIAL' }
}
$buildOk = ($outcome -eq 'DEPLOYED_AND_VERIFIED')
if (-not $buildOk -and $null -ne $exitCode) { $buildOk = ($exitCode -eq 0) }
if ($outcome -match 'FAILED|PARTIAL') { $buildOk = $false }

Write-Output ("[remake] build.ps1 finished after {0:N0} s (outcome: {1}; exit code: {2})." -f $elapsed.TotalSeconds, $outcome, $(if ($null -eq $exitCode) { 'n/a' } else { $exitCode }))

if (Test-Path -LiteralPath $BuildErr) {
    $errTail = Get-Content -LiteralPath $BuildErr -Tail 15 -ErrorAction SilentlyContinue |
        Where-Object { $_.Trim() }
    if ($errTail) {
        Write-Output '[remake] stderr tail:'
        $errTail | ForEach-Object { Write-Output ("          " + $_.Trim()) }
    }
}

# ---- verify: app process + agent task --------------------------------------
$appProc   = Get-Process -Name 'MichStartupMaster' -ErrorAction SilentlyContinue | Select-Object -First 1
$appPath   = ''
if ($appProc) {
    try { $appPath = $appProc.Path } catch { $appPath = '(access denied)' }
}
$task      = Get-ScheduledTask -TaskName 'MichStartupMasterApp' -ErrorAction SilentlyContinue
$taskState = if ($task) { [string]$task.State } else { 'MISSING' }

$ok   = $false
$note = ''
if ($StageOnly) {
    $ok = $buildOk
    if (-not $ok) { $note = ' (stage/validation reported failure - see remake-build.log)' }
} else {
    if ($buildOk -and $appProc) {
        $ok = $true
    } elseif ($buildOk -and -not $appProc) {
        $note = ' (build deployed, but no MichStartupMaster process is currently running)'
    } else {
        $note = ' (build reported failure - see remake-build.log)'
    }
}

Write-Output '------------------------------------------------------------'
Write-Output "[remake] build outcome     : $outcome"
Write-Output "[remake] app process       : $(if ($appProc) { 'RUNNING (pid ' + $appProc.Id + ')' } else { 'not running' })"
if ($appPath) { Write-Output "[remake] app path          : $appPath" }
Write-Output "[remake] agent task        : MichStartupMasterApp = $taskState"
Write-Output "[remake] verdict           : $(if ($ok) { 'PASS' } else { 'FAIL' })$note"
Write-Output ("[remake] total time        : {0:N0} s" -f $swTotal.Elapsed.TotalSeconds)
Write-Output '------------------------------------------------------------'

Set-Content -LiteralPath $Receipt -Value @(
    "REMADE_AT=$(Get-Date -Format o)",
    "PROJECT=$Root",
    "MODE=$(if ($StageOnly) { 'STAGE_ONLY' } else { 'DEPLOY' })",
    "BUILD_OUTCOME=$outcome",
    "BUILD_EXIT_CODE=$(if ($null -eq $exitCode) { 'n/a' } else { $exitCode })",
    "VERDICT=$(if ($ok) { 'PASS' } else { 'FAIL' })",
    "APP_PROCESS=$(if ($appProc) { 'RUNNING pid ' + $appProc.Id } else { 'NOT_RUNNING' })",
    "APP_PATH=$appPath",
    "AGENT_TASK=MichStartupMasterApp=$taskState",
    "TOTAL_SEC=$([math]::Round($swTotal.Elapsed.TotalSeconds, 0))",
    "BUILD_LOG=$BuildOut"
) -Encoding UTF8
Write-Output "[remake] Receipt: $Receipt"

if ($ok) {
    Write-Output '[remake] DONE - artifacts regenerated and the project is running.'
    exit 0
} else {
    Write-Output '[remake] NOT fully green - review the tail of:'
    Write-Output "        $BuildOut"
    exit 1
}