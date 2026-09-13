[CmdletBinding()]
param(
    [string]$ExecutablePath = (Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster\MichStartupMaster.exe')
)

$ErrorActionPreference = 'Stop'

function Invoke-AppCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Executable,

        [Parameter(Mandatory = $true)]
        [string]$Command
    )

    $commandOutput = @(& $Executable $Command 2>&1)
    $commandExitCode = $LASTEXITCODE
    $receipt = ($commandOutput | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
    if ($commandExitCode -ne 0) {
        throw "MichStartupMaster $Command failed with exit code $commandExitCode. $receipt"
    }
    return $receipt.Trim()
}

function Get-AgentShortcutIdentity {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ShortcutPath
    )

    if (-not (Test-Path -LiteralPath $ShortcutPath -PathType Leaf)) {
        return $null
    }

    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $null
    try {
        $shortcut = $shell.CreateShortcut($ShortcutPath)
        return [pscustomobject]@{
            TargetPath = [string]$shortcut.TargetPath
            Arguments = ([string]$shortcut.Arguments).Trim()
        }
    } finally {
        if ($null -ne $shortcut) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
        }
        if ($null -ne $shell) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
        }
    }
}

function Test-OwnedAgentShortcut {
    param(
        $Identity,
        [Parameter(Mandatory = $true)]
        [string]$ExpectedExe
    )

    if ($null -eq $Identity -or [string]::IsNullOrWhiteSpace($Identity.TargetPath)) {
        return $false
    }

    try {
        $target = [IO.Path]::GetFullPath($Identity.TargetPath)
        $expected = [IO.Path]::GetFullPath($ExpectedExe)
        return [string]::Equals($target, $expected, [StringComparison]::OrdinalIgnoreCase) -and
            [string]::Equals($Identity.Arguments, '--agent', [StringComparison]::OrdinalIgnoreCase)
    } catch {
        return $false
    }
}

if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "Required local executable was not found: $ExecutablePath"
}

$resolvedExe = (Resolve-Path -LiteralPath $ExecutablePath -ErrorAction Stop).ProviderPath
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Startup')) 'Mich Startup Master Agent.lnk'
$identityBefore = Get-AgentShortcutIdentity -ShortcutPath $shortcutPath
$ownedBefore = Test-OwnedAgentShortcut -Identity $identityBefore -ExpectedExe $resolvedExe

# Registration is authoritative: it creates the one app-owned task and retires
# its exact legacy shortcut to a recoverable app-data backup. A same-named but
# differently targeted shortcut is never modified by this helper.
$registrationReceipt = Invoke-AppCommand -Executable $resolvedExe -Command '--register-agent'

$identityAfter = Get-AgentShortcutIdentity -ShortcutPath $shortcutPath
$ownedAfter = Test-OwnedAgentShortcut -Identity $identityAfter -ExpectedExe $resolvedExe
if ($ownedAfter) {
    throw "Canonical registration left its legacy startup shortcut in place: $shortcutPath"
}

$verificationReceipt = Invoke-AppCommand -Executable $resolvedExe -Command '--verify-agent'
[pscustomobject]@{
    StartupRoute = '\MichStartupMaster\MichStartupMasterApp'
    LegacyShortcut = $shortcutPath
    OwnedShortcutFound = $ownedBefore
    OwnedShortcutRetired = ($ownedBefore -and -not $ownedAfter)
    UnrelatedShortcutPreserved = ($null -ne $identityAfter -and -not $ownedAfter)
    RegistrationReceipt = $registrationReceipt
    VerificationReceipt = $verificationReceipt
} | Format-List
