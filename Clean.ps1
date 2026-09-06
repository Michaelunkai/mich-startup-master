# ============================================================================
# Clean.ps1 - Mich Startup Master build-artifact cleanup (instant edition)
# ----------------------------------------------------------------------------
# Frees disk space immediately by deleting the regenerable build/deploy
# artifacts that scripts\build.ps1 recreates on its next run:
#   artifacts\build-staging\     historical staged builds (tens of GB)
#   artifacts\runtime-output\    deployment transaction history
#   artifacts\pre-overhaul-*\    pre-overhaul snapshots
#   artifacts\proof\*.png        proof screenshots (gitignored)
# The live installed app (%LOCALAPPDATA%\Programs\MichStartupMaster), the
# user's state (%LOCALAPPDATA%\MichStartupMaster) and every tracked source
# file are NOT touched. Deleting these folders is safe while the app runs.
#
#   powershell -ExecutionPolicy Bypass -File Clean.ps1           # delete
#   powershell -ExecutionPolicy Bypass -File Clean.ps1 -WhatIf   # dry run
# ============================================================================
param([switch]$WhatIf)

$ErrorActionPreference = 'Stop'

$Root    = $PSScriptRoot
$Receipt = Join-Path $Root 'clean.receipt'
$swTotal = [System.Diagnostics.Stopwatch]::StartNew()

Write-Output '============================================================'
Write-Output ' Clean.ps1 - Mich Startup Master build-artifact cleanup'
Write-Output " Root   : $Root"
Write-Output '============================================================'

# ---- collect the regenerable targets -------------------------------------
$artifactRoot = Join-Path $Root 'artifacts'
$targets = New-Object System.Collections.Generic.List[string]
if (Test-Path -LiteralPath (Join-Path $artifactRoot 'build-staging')) {
    $targets.Add((Join-Path $artifactRoot 'build-staging'))
}
if (Test-Path -LiteralPath (Join-Path $artifactRoot 'runtime-output')) {
    $targets.Add((Join-Path $artifactRoot 'runtime-output'))
}
Get-ChildItem -LiteralPath $artifactRoot -Directory -Filter 'pre-overhaul-*' -ErrorAction SilentlyContinue |
    ForEach-Object { $targets.Add($_.FullName) }

# ---- fast size scan (no per-file objects) --------------------------------
$sizeBytes = [long]0
$fileCount = 0
foreach ($t in $targets) {
    try {
        foreach ($f in [System.IO.Directory]::EnumerateFiles($t, '*', [System.IO.SearchOption]::AllDirectories)) {
            try { $sizeBytes += ([System.IO.FileInfo]::new($f)).Length } catch { }
            $fileCount++
        }
    } catch { }
}
$proofPng = @(Get-ChildItem -LiteralPath (Join-Path $artifactRoot 'proof') -Filter '*.png' -File -ErrorAction SilentlyContinue)
$pngBytes = [long]0
foreach ($p in $proofPng) { $pngBytes += $p.Length }

$totalBytes = $sizeBytes + $pngBytes
$totalGB    = $totalBytes / 1GB

Write-Output ("[Clean] Regenerable artifacts found : {0:N2} GB ({1} files)" -f $totalGB, ($fileCount + $proofPng.Count))
Write-Output '[Clean] Targets:'
foreach ($t in $targets) { Write-Output "          $t" }
if ($proofPng.Count -gt 0) { Write-Output ("          proof\*.png ({0:N0} files)" -f $proofPng.Count) }

if ($totalBytes -eq 0) {
    Write-Output '[Clean] Already clean - nothing to delete.'
    Write-Output ('[Clean] Total time: {0:N2} s' -f $swTotal.Elapsed.TotalSeconds)
    exit 0
}

if ($WhatIf) {
    Write-Output '[Clean] -WhatIf set - nothing was deleted.'
    Write-Output ('[Clean] Total time: {0:N2} s' -f $swTotal.Elapsed.TotalSeconds)
    exit 0
}

# ---- delete: rmdir per top-level target (fastest on NTFS), retry locks ----
function Remove-Target {
    param([string]$Path)
    $deleted = $false
    for ($i = 1; $i -le 3 -and -not $deleted; $i++) {
        & $env:ComSpec /c "rmdir /s /q `"$Path`"" 2>$null
        if (-not (Test-Path -LiteralPath $Path)) { $deleted = $true }
        elseif ($i -lt 3) { Start-Sleep -Milliseconds 500 }
    }
    if (-not $deleted) {
        Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue
        $deleted = -not (Test-Path -LiteralPath $Path)
    }
    return $deleted
}

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$leftovers = New-Object System.Collections.Generic.List[string]
foreach ($t in $targets) {
    if (Test-Path -LiteralPath $t) {
        if (-not (Remove-Target $t)) { $leftovers.Add($t) }
    }
}
foreach ($p in $proofPng) {
    Remove-Item -LiteralPath $p.FullName -Force -ErrorAction SilentlyContinue
}
$sw.Stop()

# What actually disappeared (recount only what still exists)
$remainingBytes = [long]0
foreach ($t in $targets) {
    if (Test-Path -LiteralPath $t) {
        try {
            foreach ($f in [System.IO.Directory]::EnumerateFiles($t, '*', [System.IO.SearchOption]::AllDirectories)) {
                try { $remainingBytes += ([System.IO.FileInfo]::new($f)).Length } catch { }
            }
        } catch { }
    }
}
$freedBytes = $totalBytes - $remainingBytes

Write-Output ("[Clean] Deleted {0:N2} GB in {1:N1} s" -f ($freedBytes / 1GB), $sw.Elapsed.TotalSeconds)

$pngLeft = @(Get-ChildItem -LiteralPath (Join-Path $artifactRoot 'proof') -Filter '*.png' -File -ErrorAction SilentlyContinue)
if ($leftovers.Count -gt 0 -or $pngLeft.Count -gt 0) {
    Write-Output '[Clean] NOTE: some files were locked and remain:'
    foreach ($l in $leftovers) { Write-Output "          $l" }
    if ($pngLeft.Count -gt 0) { Write-Output ("          proof\*.png ({0} files)" -f $pngLeft.Count) }
    Write-Output '[Clean] Close whatever holds them, then re-run (or run remake.ps1).'
}

Set-Content -LiteralPath $Receipt -Value @(
    "CLEANED_AT=$(Get-Date -Format o)",
    "PROJECT=$Root",
    "DELETED_BYTES=$freedBytes",
    "DELETED_GB=$([math]::Round($freedBytes / 1GB, 2))",
    "DURATION_SEC=$([math]::Round($sw.Elapsed.TotalSeconds, 2))",
    "TOTAL_SEC=$([math]::Round($swTotal.Elapsed.TotalSeconds, 2))"
) -Encoding UTF8
Write-Output "[Clean] Receipt written: $Receipt"
Write-Output ('[Clean] Total time: {0:N2} s' -f $swTotal.Elapsed.TotalSeconds)
Write-Output '[Clean] Done. Run remake.ps1 to rebuild artifacts and run the project.'
exit 0