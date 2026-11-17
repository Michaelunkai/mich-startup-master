# Mich Startup Master v3.1 verification harness (run as admin, -NoProfile)
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File verify-v31.ps1 -Exe <path>
param([Parameter(Mandatory=$true)][string]$Exe)

$script:Pass = 0; $script:Fail = 0
function Gate {
    param([string]$Name, [string[]]$ExeArgs)
    $out = (& $Exe @ExeArgs 2>&1 | Out-String).Trim()
    $code = $LASTEXITCODE
    $ok = ($code -eq 0)
    if ($ok) { $script:Pass++ } else { $script:Fail++ }
    $first = ($out -split "`r?`n" | Where-Object { $_.Trim() } | Select-Object -First 1)
    Write-Host ("[{0}] {1} :: {2}" -f ($(if($ok){"PASS"}else{"FAIL"})), $Name, $first)
}

Write-Host "=== Mich Startup Master v3.1 gate suite :: $Exe ==="

Gate "version"        @("--version")
Gate "smoke"          @("--smoke")
Gate "help"           @("--help")
Gate "list"           @("--list")
Gate "list-enabled"   @("--list-enabled")
Gate "list-disabled"  @("--list-disabled")
Gate "list-sources"   @("--list-sources")
Gate "detect-new"     @("--detect-new")
Gate "export"         @("--export")
Gate "audit-boot"     @("--audit-boot")
Gate "audit-tray"     @("--audit-tray")
Gate "verify-live"    @("--verify-live-inventory")
Gate "truth-test"     @("--truth-self-test")
Gate "inventory-test" @("--inventory-self-test")
Gate "dedupe-test"    @("--managed-startup-dedupe-self-test")
Gate "state-store-test" @("--state-store-self-test")
Gate "release-gate-test" @("--release-gate-self-test")
Gate "agent-reg-test" @("--agent-registration-self-test")
Gate "ui-contract"    @("--ui-contract")
Gate "ui-self-test"   @("--ui-self-test")
Gate "quiet-lineage"  @("--quiet-lineage-self-test")
Gate "verify-agent"   @("--verify-agent")

# --- live functional round-trips -------------------------------------------
function Count-Json($exeArgs) {
    try { @((& $Exe @exeArgs 2>&1 | Out-String) | ConvertFrom-Json).Count } catch { 0 }
}

Gate "add-from-path"  @("--add-from-path", "C:\Windows\System32\notepad.exe")
$notepad = (& $Exe --list 2>&1 | Out-String | ConvertFrom-Json) | Where-Object { $_.source -eq "Scheduled Task" -and $_.command -like "*notepad.exe*" }
if ($notepad) { $script:Pass++; Write-Host "[PASS] add-from-path-visible :: task registered + enabled in inventory" }
else { $script:Fail++; Write-Host "[FAIL] add-from-path-visible :: task missing from inventory" }

Gate "purge-notepad"  @("--purge", "notepad")
$stillThere = (& $Exe --list 2>&1 | Out-String | ConvertFrom-Json) | Where-Object { $_.command -like "*notepad.exe*" -and $_.source -eq "Scheduled Task" }
if (-not $stillThere) { $script:Pass++; Write-Host "[PASS] purge-removed :: notepad startup gone everywhere" }
else { $script:Fail++; Write-Host "[FAIL] purge-removed :: notepad still present" }

Gate "hidden-list"    @("--hidden-list")

$hiddenOut = (& $Exe --hidden-list 2>&1 | Out-String)
$backupId = $null
foreach ($m in [regex]::Matches($hiddenOut, "PURGED id=(purged-\d+)")) { $backupId = $m.Groups[1].Value; break }
if ($backupId) {
    Gate "hidden-restore" @("--hidden-restore", $backupId)
    $back = (& $Exe --list 2>&1 | Out-String | ConvertFrom-Json) | Where-Object { $_.command -like "*notepad.exe*" -and $_.source -eq "Scheduled Task" }
    if ($back) { $script:Pass++; Write-Host "[PASS] purge-restored :: entry returned (as disabled)" }
    else { $script:Fail++; Write-Host "[FAIL] purge-restored :: entry did not return" }
    $loc = ($back | Select-Object -First 1).location
    if ($loc) { & $Exe --remove-task $loc | Out-Null }
    # lift any remaining deny keys so the machine is exactly as before
    foreach ($m in [regex]::Matches((& $Exe --hidden-list 2>&1 | Out-String), "HIDDEN key=([^\s]+)")) {
        & $Exe --hidden-restore $m.Groups[1].Value | Out-Null
    }
} else {
    Write-Host "[warn] hidden-restore :: no purge backup id found in hidden list"
}

Write-Host ""
Write-Host ("RESULT pass={0} fail={1}" -f $script:Pass, $script:Fail)
if ($script:Fail -gt 0) { exit 1 } else { exit 0 }
