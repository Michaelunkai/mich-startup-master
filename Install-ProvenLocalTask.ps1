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

if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "Required local executable was not found: $ExecutablePath"
}

$resolvedExe = (Resolve-Path -LiteralPath $ExecutablePath -ErrorAction Stop).ProviderPath
$registrationReceipt = Invoke-AppCommand -Executable $resolvedExe -Command '--register-agent'
$verificationReceipt = Invoke-AppCommand -Executable $resolvedExe -Command '--verify-agent'

[pscustomobject]@{
    StartupRoute = '\MichStartupMaster\MichStartupMasterApp'
    Executable = $resolvedExe
    Arguments = '--agent'
    RegistrationReceipt = $registrationReceipt
    VerificationReceipt = $verificationReceipt
} | Format-List
