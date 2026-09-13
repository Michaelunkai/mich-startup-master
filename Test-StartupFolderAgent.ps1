[CmdletBinding()]
param(
    [string]$ExecutablePath = (Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster\MichStartupMaster.exe')
)

$ErrorActionPreference = 'Stop'

function Test-ExactPath {
    param(
        [string]$Candidate,
        [string]$Expected
    )

    if ([string]::IsNullOrWhiteSpace($Candidate)) {
        return $false
    }

    try {
        return [string]::Equals(
            [IO.Path]::GetFullPath($Candidate),
            [IO.Path]::GetFullPath($Expected),
            [StringComparison]::OrdinalIgnoreCase)
    } catch {
        return $false
    }
}

if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "MichStartupMaster executable was not found: $ExecutablePath"
}

$resolvedExe = (Resolve-Path -LiteralPath $ExecutablePath -ErrorAction Stop).ProviderPath
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Startup')) 'Mich Startup Master Agent.lnk'
$ownedLegacyShortcut = $false
if (Test-Path -LiteralPath $shortcutPath -PathType Leaf) {
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $null
    try {
        $shortcut = $shell.CreateShortcut($shortcutPath)
        $ownedLegacyShortcut = (Test-ExactPath -Candidate ([string]$shortcut.TargetPath) -Expected $resolvedExe) -and
            [string]::Equals(([string]$shortcut.Arguments).Trim(), '--agent', [StringComparison]::OrdinalIgnoreCase)
    } finally {
        if ($null -ne $shortcut) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
        }
        if ($null -ne $shell) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
        }
    }
}

if ($ownedLegacyShortcut) {
    throw "The app-owned legacy Startup-folder route still exists: $shortcutPath"
}

$verificationOutput = @(& $resolvedExe '--verify-agent' 2>&1)
$verificationExitCode = $LASTEXITCODE
$verificationReceipt = ($verificationOutput | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
if ($verificationExitCode -ne 0) {
    throw "Read-only agent verification failed with exit code $verificationExitCode. $verificationReceipt"
}

[pscustomobject]@{
    StartupRoute = '\MichStartupMaster\MichStartupMasterApp'
    LegacyShortcut = $shortcutPath
    OwnedLegacyShortcutPresent = $false
    VerificationReceipt = $verificationReceipt.Trim()
} | Format-List
