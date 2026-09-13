[CmdletBinding()]
param(
    [switch]$StageOnly,
    [switch]$InstallCurrentUser,
    [switch]$CompileInstaller,
    [string]$StageDirectory,
    [string]$RestoreExternalSnapshot,
    [ValidateRange(1, 60)]
    [int]$GracefulStopSeconds = 8
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

if ([string]::IsNullOrWhiteSpace($RestoreExternalSnapshot)) {
    $Root = Split-Path -Path $PSScriptRoot -Parent
} else {
    # The immutable recovery copy lives five levels below the project root:
    # artifacts\runtime-output\deploy-*\external-state-stable-post-stop\external-state.json.
    # Derive the project boundary from the requested canonical manifest so the
    # snapshotted recovery tool does not mistake its transaction directory for
    # the source/deployment root.
    $Root = [IO.Path]::GetFullPath($RestoreExternalSnapshot)
    for ($level = 0; $level -lt 5; $level++) {
        $Root = Split-Path -Path $Root -Parent
        if ([string]::IsNullOrWhiteSpace($Root)) { throw '-RestoreExternalSnapshot is not beneath a canonical project transaction path.' }
    }
}
$ExecutingBuildScriptPath = [IO.Path]::GetFullPath($PSCommandPath)
$DeployDirectory = if ($InstallCurrentUser) { Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster' } else { Join-Path $Root 'build' }
$ArtifactsDirectory = Join-Path $Root 'artifacts'
$RuntimeOutputDirectory = Join-Path $ArtifactsDirectory 'runtime-output'
$GeneratedStageRoot = Join-Path $ArtifactsDirectory 'build-staging'
$ProjectName = 'MichStartupMaster.csproj'
$ExecutableName = 'MichStartupMaster.exe'
$ApplicationDllName = 'MichStartupMaster.dll'
$IconName = 'MichStartupMaster.ico'
$LegacyLauncherName = 'MichStartupMasterAgent.vbs'
$PinnedSdkVersion = '10.0.301'
$TransactionId = (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$DeploymentMutexName = 'Local\MichStartupMaster.ReleaseDeployment'
$ManagedStartupMutationMutexName = 'Local\MichStartupMaster.ManagedStartupMutation'

function Write-BuildState {
    param([Parameter(Mandatory = $true)][string]$Message)
    Write-Host ('[MichStartupMaster] ' + $Message)
}

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    return [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
}

function Test-IsSameOrChildPath {
    param(
        [Parameter(Mandatory = $true)][string]$Candidate,
        [Parameter(Mandatory = $true)][string]$Parent
    )
    $candidatePath = Get-FullPath $Candidate
    $parentPath = Get-FullPath $Parent
    return $candidatePath.Equals($parentPath, [StringComparison]::OrdinalIgnoreCase) -or
        $candidatePath.StartsWith($parentPath + '\', [StringComparison]::OrdinalIgnoreCase)
}

function Get-CanonicalChildPath {
    param(
        [Parameter(Mandatory = $true)][string]$Parent,
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [Parameter(Mandatory = $true)][string]$Context
    )
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [IO.Path]::IsPathRooted($RelativePath)) {
        throw "$Context must be a nonempty relative path."
    }
    $parentPath = Get-FullPath $Parent
    $candidate = Get-FullPath (Join-Path $parentPath $RelativePath)
    if (-not (Test-IsSameOrChildPath -Candidate $candidate -Parent $parentPath) -or
        [string]::Equals($candidate, $parentPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Context escapes its authorized parent: $RelativePath"
    }
    return $candidate
}

function Assert-NoReparsePointTraversal {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$TrustedRoot,
        [Parameter(Mandatory = $true)][string]$Context
    )
    $candidate = Get-FullPath $Path
    $rootPath = Get-FullPath $TrustedRoot
    if (-not (Test-IsSameOrChildPath -Candidate $candidate -Parent $rootPath)) {
        throw "$Context is outside its trusted root."
    }
    $cursor = $candidate
    while ($cursor.Length -ge $rootPath.Length) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "$Context traverses a reparse point: $cursor"
            }
        }
        if ([string]::Equals($cursor, $rootPath, [StringComparison]::OrdinalIgnoreCase)) { break }
        $parent = Split-Path -Path $cursor -Parent
        if ([string]::IsNullOrWhiteSpace($parent) -or [string]::Equals($parent, $cursor, [StringComparison]::OrdinalIgnoreCase)) {
            throw "$Context could not be traced to its trusted root."
        }
        $cursor = Get-FullPath $parent
    }
}

function Assert-NoReparsePointsInTree {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][string]$Context
    )
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { return }
    $rootItem = Get-Item -LiteralPath $Directory -Force -ErrorAction Stop
    if (($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "$Context contains a reparse point: $($rootItem.FullName)" }
    foreach ($item in @(Get-ChildItem -LiteralPath $Directory -Recurse -Force -ErrorAction Stop)) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "$Context contains a reparse point: $($item.FullName)" }
    }
}

function New-EmptyDirectory {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (Test-Path -LiteralPath $Path) {
        $entries = @(Get-ChildItem -LiteralPath $Path -Force -ErrorAction Stop)
        if ($entries.Count -ne 0) {
            throw "Refusing to overwrite non-empty staging directory: $Path"
        }
    } else {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    }
}

function Copy-DirectoryExact {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination
    )
    $sourceFull = Get-FullPath $Source
    $destinationFull = Get-FullPath $Destination
    if (-not [IO.Directory]::Exists($sourceFull)) {
        throw "Source directory was not found: $Source"
    }
    if ([IO.Directory]::Exists($destinationFull) -or [IO.File]::Exists($destinationFull)) {
        throw "Refusing to overwrite existing directory: $Destination"
    }
    $extendedSource = ConvertTo-ExtendedLengthPath $sourceFull
    $extendedDestination = ConvertTo-ExtendedLengthPath $destinationFull
    [void][IO.Directory]::CreateDirectory($extendedDestination)
    $sourcePrefix = $extendedSource.TrimEnd('\') + '\'
    foreach ($directory in [IO.Directory]::EnumerateDirectories($extendedSource, '*', [IO.SearchOption]::AllDirectories)) {
        $relative = $directory.Substring($sourcePrefix.Length)
        [void][IO.Directory]::CreateDirectory((Join-ExtendedPath $extendedDestination $relative))
    }
    foreach ($file in [IO.Directory]::EnumerateFiles($extendedSource, '*', [IO.SearchOption]::AllDirectories)) {
        $relative = $file.Substring($sourcePrefix.Length)
        $destinationFile = Join-ExtendedPath $extendedDestination $relative
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destinationFile))
        [IO.File]::Copy($file, $destinationFile, $false)
    }
}

function ConvertTo-ExtendedLengthPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    $full = [IO.Path]::GetFullPath($Path)
    if ($full.StartsWith('\\?\', [StringComparison]::Ordinal)) { return $full }
    if ($full.StartsWith('\\', [StringComparison]::Ordinal)) { return '\\?\UNC\' + $full.Substring(2) }
    return '\\?\' + $full
}

function Join-ExtendedPath {
    param(
        [Parameter(Mandatory = $true)][string]$Parent,
        [Parameter(Mandatory = $true)][string]$Child
    )
    return $Parent.TrimEnd('\') + '\' + $Child.TrimStart('\')
}

function Get-DirectoryManifest {
    param([Parameter(Mandatory = $true)][string]$Directory)
    $resolvedDirectory = Get-FullPath $Directory
    $enumerationRoot = ConvertTo-ExtendedLengthPath $resolvedDirectory
    $base = $enumerationRoot.TrimEnd('\') + '\'
    $paths = @([IO.Directory]::EnumerateFiles($enumerationRoot, '*', [IO.SearchOption]::AllDirectories) | Sort-Object)
    $rows = New-Object 'Collections.Generic.List[object]'
    $sha256 = [Security.Cryptography.SHA256]::Create()
    $share = [IO.FileShare]([int][IO.FileShare]::ReadWrite -bor [int][IO.FileShare]::Delete)
    try {
        foreach ($path in $paths) {
            $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
            try { $digest = $sha256.ComputeHash($stream) }
            finally { $stream.Dispose() }
            $info = New-Object IO.FileInfo -ArgumentList $path
            [void]$rows.Add([pscustomobject][ordered]@{
                RelativePath = $path.Substring($base.Length).Replace('/', '\')
                Length = [long]$info.Length
                Sha256 = [BitConverter]::ToString($digest).Replace('-', '')
            })
        }
        return $rows.ToArray()
    }
    finally { $sha256.Dispose() }
}

function Assert-ManifestsEqual {
    param(
        [Parameter(Mandatory = $true)][object[]]$Expected,
        [Parameter(Mandatory = $true)][object[]]$Actual,
        [Parameter(Mandatory = $true)][string]$Context
    )
    if ($Expected.Count -ne $Actual.Count) {
        throw "$Context file-count mismatch: expected $($Expected.Count), actual $($Actual.Count)"
    }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        $left = $Expected[$index]
        $right = $Actual[$index]
        if (-not [string]::Equals([string]$left.RelativePath, [string]$right.RelativePath, [StringComparison]::OrdinalIgnoreCase) -or
            [long]$left.Length -ne [long]$right.Length -or
            -not [string]::Equals([string]$left.Sha256, [string]$right.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw "$Context mismatch at index ${index}: expected '$($left.RelativePath)' $($left.Sha256), actual '$($right.RelativePath)' $($right.Sha256)"
        }
    }
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )
    $json = $Value | ConvertTo-Json -Depth 12
    [System.IO.File]::WriteAllText($Path, $json, (New-Object System.Text.UTF8Encoding($false)))
}

function Write-JsonFileAtomic {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )
    $directory = Split-Path -Path $Path -Parent
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }
    $temporary = Join-Path $directory ('.' + [IO.Path]::GetFileName($Path) + '.' + [Guid]::NewGuid().ToString('N') + '.tmp')
    $backup = $Path + '.previous.' + [Guid]::NewGuid().ToString('N')
    $json = $Value | ConvertTo-Json -Depth 12
    $encoding = New-Object Text.UTF8Encoding($false)
    $stream = New-Object IO.FileStream($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = $encoding.GetBytes($json)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally { $stream.Dispose() }
    try {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            [IO.File]::Replace($temporary, $Path, $backup, $true)
            if (Test-Path -LiteralPath $backup -PathType Leaf) { Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue }
        } else {
            [IO.File]::Move($temporary, $Path)
        }
    } finally {
        if (Test-Path -LiteralPath $temporary -PathType Leaf) { Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue }
    }
}

function Write-DeploymentMarker {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Outcome,
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][string]$TransactionDirectory,
        [Parameter(Mandatory = $true)][string]$StageReceipt,
        [string]$DeploymentReceipt,
        [string]$MarkerTransactionId = $TransactionId,
        [string]$SourceManifestSha256,
        [string]$PublishManifestSha256,
        [string]$ExternalSnapshotReceipt,
        [string]$ExternalSnapshotReceiptSha256
    )
    $stageReceiptProvenance = Get-FileProvenance $StageReceipt
    $deploymentReceiptProvenance = if ([string]::IsNullOrWhiteSpace($DeploymentReceipt) -or -not (Test-Path -LiteralPath $DeploymentReceipt -PathType Leaf)) { $null } else { Get-FileProvenance $DeploymentReceipt }
    Write-JsonFileAtomic -Path $Path -Value ([pscustomobject][ordered]@{
        SchemaVersion = 2
        Outcome = $Outcome
        Phase = $Phase
        TransactionId = $MarkerTransactionId
        UpdatedUtc = [DateTime]::UtcNow.ToString('o')
        TransactionDirectory = Get-FullPath $TransactionDirectory
        StageReceipt = $stageReceiptProvenance
        SourceManifestSha256 = $SourceManifestSha256
        PublishManifestSha256 = $PublishManifestSha256
        ExternalSnapshotReceipt = if ([string]::IsNullOrWhiteSpace($ExternalSnapshotReceipt)) { $null } else { Get-FullPath $ExternalSnapshotReceipt }
        ExternalSnapshotReceiptSha256 = if ([string]::IsNullOrWhiteSpace($ExternalSnapshotReceiptSha256)) { $null } else { $ExternalSnapshotReceiptSha256.ToUpperInvariant() }
        DeploymentReceipt = if ($null -eq $deploymentReceiptProvenance) { $null } else { [string]$deploymentReceiptProvenance.Path }
        DeploymentReceiptSha256 = if ($null -eq $deploymentReceiptProvenance) { $null } else { [string]$deploymentReceiptProvenance.Sha256 }
        RecoveryInstruction = 'Inspect the deployment receipt and preserved rollback/external-state artifacts before retrying. Never delete this marker to bypass an incomplete recovery.'
    })
}

function Get-ManifestSha256 {
    param([Parameter(Mandatory = $true)][object[]]$Manifest)
    $canonical = @(
        $Manifest |
            Sort-Object -Property RelativePath |
            ForEach-Object { ([string]$_.RelativePath).ToLowerInvariant() + '|' + [string][long]$_.Length + '|' + ([string]$_.Sha256).ToUpperInvariant() }
    ) -join "`n"
    $bytes = [Text.Encoding]::UTF8.GetBytes($canonical)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '') } finally { $sha.Dispose() }
}

function Get-FileProvenance {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Provenance input was not found: $Path"
    }
    $file = Get-Item -LiteralPath $Path
    return [pscustomobject][ordered]@{
        Path = Get-FullPath $file.FullName
        Length = [long]$file.Length
        Sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
    }
}

function Assert-FileProvenance {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)][string]$Context
    )
    $actual = Get-FileProvenance ([string]$Expected.Path)
    if ([long]$actual.Length -ne [long]$Expected.Length -or
        -not [string]::Equals([string]$actual.Sha256, [string]$Expected.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Context changed after staging: $($Expected.Path)"
    }
}

function Get-StringSha256 {
    param([AllowEmptyString()][string]$Value)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes([string]$Value)))).Replace('-', '')
    } finally { $sha.Dispose() }
}

function Get-SourceManifest {
    param([Parameter(Mandatory = $true)][string]$SourceRoot)
    $files = @()
    foreach ($relativePath in @(
        $ProjectName,
        'global.json',
        'packages.lock.json',
        'dotnet-install.ps1',
        'scripts\build.ps1',
        'installer\MichStartupMaster.iss',
        'Install-LocalProduction.ps1',
        'Install-ProvenLocalTask.ps1',
        'Install-StartupFolderAgent.ps1',
        'Launch-Agent.ps1',
        'Stop-MichStartupMaster.ps1',
        'Test-StartupFolderAgent.ps1',
        'Verify-StartupAgent.ps1'
    )) {
        $path = Join-Path $SourceRoot $relativePath
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Required build input was not found: $path"
        }
        $files += Get-Item -LiteralPath $path
    }
    foreach ($relativeDirectory in @('src', 'assets')) {
        $path = Join-Path $SourceRoot $relativeDirectory
        if (-not (Test-Path -LiteralPath $path -PathType Container)) {
            throw "Required build input directory was not found: $path"
        }
        $files += Get-ChildItem -LiteralPath $path -Recurse -File -Force -ErrorAction Stop
    }
    $base = (Get-FullPath $SourceRoot) + '\'
    return @(
        $files |
            Sort-Object -Property FullName -Unique |
            ForEach-Object {
                [pscustomobject][ordered]@{
                    RelativePath = $_.FullName.Substring($base.Length).Replace('/', '\')
                    Length = [long]$_.Length
                    Sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
                }
            }
    )
}

function Copy-SourceSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$SnapshotRoot
    )
    # The caller allocates a unique attempt path, but the path itself may not
    # exist yet. Create it before copying the first file so a clean build
    # staging root is as reliable as a previously populated one.
    if (-not (Test-Path -LiteralPath $SnapshotRoot -PathType Container)) {
        New-Item -ItemType Directory -Path $SnapshotRoot -Force | Out-Null
    }
    New-EmptyDirectory $SnapshotRoot
    Copy-Item -LiteralPath (Join-Path $SourceRoot $ProjectName) -Destination (Join-Path $SnapshotRoot $ProjectName) -Force
    Copy-Item -LiteralPath (Join-Path $SourceRoot 'global.json') -Destination (Join-Path $SnapshotRoot 'global.json') -Force
    Copy-Item -LiteralPath (Join-Path $SourceRoot 'packages.lock.json') -Destination (Join-Path $SnapshotRoot 'packages.lock.json') -Force
    Copy-Item -LiteralPath (Join-Path $SourceRoot 'dotnet-install.ps1') -Destination (Join-Path $SnapshotRoot 'dotnet-install.ps1') -Force
    foreach ($legacyScript in @('Install-LocalProduction.ps1', 'Install-ProvenLocalTask.ps1', 'Install-StartupFolderAgent.ps1', 'Launch-Agent.ps1', 'Stop-MichStartupMaster.ps1', 'Test-StartupFolderAgent.ps1', 'Verify-StartupAgent.ps1')) {
        Copy-Item -LiteralPath (Join-Path $SourceRoot $legacyScript) -Destination (Join-Path $SnapshotRoot $legacyScript) -Force
    }
    Copy-Item -LiteralPath (Join-Path $SourceRoot 'src') -Destination (Join-Path $SnapshotRoot 'src') -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $SourceRoot 'assets') -Destination (Join-Path $SnapshotRoot 'assets') -Recurse -Force
    New-Item -ItemType Directory -Path (Join-Path $SnapshotRoot 'scripts') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $SourceRoot 'scripts\build.ps1') -Destination (Join-Path $SnapshotRoot 'scripts\build.ps1') -Force
    New-Item -ItemType Directory -Path (Join-Path $SnapshotRoot 'installer') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $SourceRoot 'installer\MichStartupMaster.iss') -Destination (Join-Path $SnapshotRoot 'installer\MichStartupMaster.iss') -Force
}

function New-ConsistentSourceSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$StageRoot,
        [ValidateRange(1, 20)][int]$MaximumAttempts = 10
    )
    $lastError = $null
    for ($attempt = 1; $attempt -le $MaximumAttempts; $attempt++) {
        $attemptDirectory = Join-Path $StageRoot ('source-attempt-' + $attempt)
        $beforeManifest = Get-SourceManifest $SourceRoot
        Copy-SourceSnapshot -SourceRoot $SourceRoot -SnapshotRoot $attemptDirectory
        $snapshotManifest = Get-SourceManifest $attemptDirectory
        $afterManifest = Get-SourceManifest $SourceRoot
        try {
            Assert-ManifestsEqual -Expected $beforeManifest -Actual $snapshotManifest -Context "Source snapshot attempt $attempt"
            Assert-ManifestsEqual -Expected $beforeManifest -Actual $afterManifest -Context "Source stability attempt $attempt"
            return [pscustomobject][ordered]@{
                Directory = $attemptDirectory
                Manifest = $beforeManifest
                Attempts = $attempt
            }
        } catch {
            $lastError = $_
            if ($attempt -lt $MaximumAttempts) {
                Start-Sleep -Milliseconds 200
            }
        }
    }
    throw "Could not capture a consistent source snapshot after $MaximumAttempts attempts. Each attempt was preserved under '$StageRoot'. Last error: $($lastError.Exception.Message)"
}

function Invoke-StagedCliProbe {
    param(
        [Parameter(Mandatory = $true)][string]$ExecutablePath,
        [Parameter(Mandatory = $true)][string]$Arguments,
        [ValidateRange(1000, 300000)][int]$TimeoutMilliseconds = 120000
    )
    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $ExecutablePath
    $startInfo.Arguments = $Arguments
    $startInfo.WorkingDirectory = Split-Path -Path $ExecutablePath -Parent
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw "Could not start staged CLI probe: $Arguments" }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutMilliseconds)) {
            try { $process.Kill() } catch { }
            [void]$process.WaitForExit(5000)
            $timedOutStdout = try { $stdoutTask.GetAwaiter().GetResult() } catch { '' }
            $timedOutStderr = try { $stderrTask.GetAwaiter().GetResult() } catch { '' }
            throw "Staged CLI probe timed out after $TimeoutMilliseconds ms: $Arguments`nstdout: $timedOutStdout`nstderr: $timedOutStderr"
        }
        # The parameterless wait flushes asynchronous redirected-stream handlers after
        # the bounded process wait has already proven termination.
        $process.WaitForExit()
        $stdout = [string]$stdoutTask.GetAwaiter().GetResult()
        $stderr = [string]$stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw "Staged CLI probe failed with exit $($process.ExitCode): $Arguments`nstdout: $stdout`nstderr: $stderr"
        }
        return [pscustomobject][ordered]@{
            Arguments = $Arguments
            ExitCode = [int]$process.ExitCode
            TimeoutMilliseconds = $TimeoutMilliseconds
            Stdout = $stdout.Trim()
            Stderr = $stderr.Trim()
            ExplicitPass = $true
        }
    } finally {
        $process.Dispose()
    }
}

function Assert-StagedBehavior {
    param([Parameter(Mandatory = $true)][string]$Directory)
    $executable = Join-Path $Directory $ExecutableName
    $probes = @()

    $version = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--version' -TimeoutMilliseconds 30000
    if ($version.Stdout -notmatch '^MichStartupMaster 2\.0\.0$') { throw "Unexpected staged version receipt: $($version.Stdout)" }
    $probes += $version

    $state = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--state-store-self-test'
    if ($state.Stdout -notmatch '^STATE_STORE_SELF_TEST workers=12 atomic=ok recovery=ok$') { throw "State-store behavior gate failed: $($state.Stdout)" }
    $probes += $state

    $truth = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--truth-self-test'
    if ($truth.Stdout.Trim() -ne 'TRUTH_SELF_TEST checks=11 passed=11 codex=unverified contradictions=drifted missing=unknown') { throw "Startup truth gate failed: $($truth.Stdout)" }
    $probes += $truth

    $inventory = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--inventory-self-test'
    $expectedSurfaceNames = 'Registry_Run,Registry_RunOnce,Registry_RunOnceEx,Registry_RunServices,Policy_Run,Legacy_Windows_Run,User_Logon_Script,Startup_Folder,Scheduled_Task,Windows_Service,System_Driver,Winlogon_Autostart,Winlogon_Notification,Explorer_Startup_Extension,Explorer_Shell_Extension,Internet_Explorer_Add-on,AppInit_DLLs,AppCert_DLLs,Active_Setup,Boot_Execute,LSA_Startup_Package,Image_Hijack,Known_DLL,Network_Provider,Winsock_Provider,Print_Monitor,Media_Codec,Group_Policy_Script,WMI_Event_Consumer,Packaged_Startup_Task'
    $inventoryMatch = [regex]::Match($inventory.Stdout, '^INVENTORY_SELF_TEST kind=fixtures checks=(\d+) passed=(\d+) surface_contracts=30 surface_names=([^\s]+) service_start_modes=0,1,2,3$')
    if (-not $inventoryMatch.Success) {
        throw "Inventory behavior gate receipt is malformed: $($inventory.Stdout)"
    }
    if ([int]$inventoryMatch.Groups[1].Value -lt 1 -or
        [int]$inventoryMatch.Groups[1].Value -ne [int]$inventoryMatch.Groups[2].Value -or
        -not [string]::Equals($inventoryMatch.Groups[3].Value, $expectedSurfaceNames, [StringComparison]::Ordinal)) {
        throw "Inventory behavior gate fields failed: $($inventory.Stdout)"
    }
    $probes += $inventory

    $managedDedupe = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--managed-startup-dedupe-self-test'
    $managedDedupeMatch = [regex]::Match($managedDedupe.Stdout, '^MANAGED_DEDUPE_SELF_TEST checks=(\d+) passed=(\d+) exactOne=true staleAliasRetired=true conflictsFailClosed=true$')
    if (-not $managedDedupeMatch.Success -or [int]$managedDedupeMatch.Groups[1].Value -lt 1 -or [int]$managedDedupeMatch.Groups[1].Value -ne [int]$managedDedupeMatch.Groups[2].Value) {
        throw "Managed-startup dedupe behavior gate failed: $($managedDedupe.Stdout)"
    }
    $probes += $managedDedupe

    $bulkDisable = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--bulk-disable-self-test'
    if ($bulkDisable.Stdout.Trim() -ne 'BULK_DISABLE_SELF_TEST passed=true failedClosed=true registryRestored=true storesRestored=true') {
        throw "Transactional bulk-disable behavior gate failed: $($bulkDisable.Stdout)"
    }
    $probes += $bulkDisable

    $agent = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--agent-registration-self-test'
    $missingAgentFields = @(
        @(
            'goodAccepted=true', 'omittedDefaultAccepted=true', 'delayedRejected=true',
            'wrongActionRejected=true', 'settingsFalseRejected=true',
            'triggerFalseRejected=true', 'batteryStopRejected=true',
            'badPrincipalRejected=true', 'extraTriggerRejected=true',
            'extraActionRejected=true', 'wrongUserRejected=true',
            'legacyDirectAccepted=true', 'legacyWscriptAccepted=true',
            'legacyCscriptAccepted=true', 'unrelatedShortcutRejected=true',
            'sameNameWrongPathRejected=true'
        ) | Where-Object { $agent.Stdout -notmatch ('(?:^|\s)' + [regex]::Escape($_) + '(?:\s|$)') }
    )
    if ($agent.Stdout -notmatch '^AGENT_REGISTRATION_SELF_TEST_OK ' -or $missingAgentFields.Count -ne 0) {
        throw "Agent-registration behavior gate failed: $($agent.Stdout)"
    }
    $probes += $agent

    $releaseGate = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--release-gate-self-test'
    $missingReleaseGateFields = @(
        @('passed=true', 'normalOrder=true', 'unauthorizedRejected=true', 'childBypass=true',
          'heldGateRequired=true', 'eventRequired=true', 'oneShot=true', 'tokenLeak=false',
          'externalProcessesStarted=0') | Where-Object { $releaseGate.Stdout -notmatch ('(?:^|\s)' + [regex]::Escape($_) + '(?:\s|$)') }
    )
    if ($releaseGate.Stdout -notmatch '^RELEASE_GATE_SELF_TEST_OK ' -or $missingReleaseGateFields.Count -ne 0) {
        throw "Release-gate behavior gate failed: $($releaseGate.Stdout)"
    }
    $probes += $releaseGate

    $policy = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--quiet-policy-probe'
    try { $policyJson = $policy.Stdout | ConvertFrom-Json -ErrorAction Stop } catch { throw "Quiet-policy behavior gate is not JSON: $($policy.Stdout)" }
    if ($policyJson.passed -ne $true -or $policyJson.firstWindowHidden -ne $true -or
        $policyJson.automaticSameWindowReshowHidden -ne $true -or [int]$policyJson.automaticReshowHideCount -lt 2 -or
        $policyJson.delayedSecondWindowHidden -ne $true -or $policyJson.veryLateWindowAllowed -ne $true -or
        $policyJson.bootstrapCompletionCancelsSuppression -ne $true -or
        $policyJson.nativeTrayReadinessEndsBootstrap -ne $true -or
        $policyJson.nativeTrayReadinessWaitsForInitialWindow -ne $true -or
        $policyJson.chatGptTrayHostRecognized -ne $true -or
        $policyJson.winFormsTrayHostRecognized -ne $true -or
        $policyJson.winFormsMainWindowNotTrayHost -ne $true -or
        $policyJson.chatGptFilenameDoesNotProveTray -ne $true -or
        $policyJson.runtimeNativeTrayHostOverridesStaticMiss -ne $true -or
        $policyJson.runtimeNativeTrayHostRemovesFallback -ne $true -or
        $policyJson.fallbackReadinessEndsBootstrap -ne $true -or
        $policyJson.launcherHandoffWaitsForRuntimeIcon -ne $true -or
        $policyJson.nativeControllerExitsAfterBootstrap -ne $true -or
        $policyJson.missingRuntimeTrayHostGetsFallback -ne $true -or
        $policyJson.explicitActivationPermanentlyCancelsSuppression -ne $true -or
        $policyJson.unrelatedInputFocusStealDoesNotCancelSuppression -ne $true -or
        [string]$policyJson.suppressionCancellationInputs -ne 'explicit-proxy-or-bootstrap-complete' -or
        [int]$policyJson.manualWindowRehideCount -ne 0 -or $policyJson.proxyOpenImmediateActivation -ne $true -or
        $policyJson.trackedWindowEventHiddenImmediately -ne $true -or $policyJson.untrackedWindowEventIgnored -ne $true -or
        $policyJson.liveLineageNeverRelaunched -ne $true -or $policyJson.pendingWindowRestoredOnce -ne $true -or
        $policyJson.rejectedRestoreKeepsPending -ne $true -or $policyJson.pendingCaptureIsBounded -ne $true -or
        $policyJson.pendingManualOverlapKeepsObserver -ne $true -or $policyJson.nonSingleInstanceLaunchExactlyOnce -ne $true -or
        $policyJson.fallbackIconExactlyOne -ne $true -or
        [string]$policyJson.fallbackIconIdentity -ne 'launch-payload-or-launch-handler-only' -or
        $policyJson.startupMasterIconFallback -ne $false -or
        $policyJson.nativeTrayCapabilityUsesNoProxy -ne $true -or
        $policyJson.managedPlanOwnsOneProxy -ne $true -or
        $policyJson.launcherPayloadRouteRecognized -ne $true -or
        $policyJson.launcherPayloadRejectsOrdinaryName -ne $true -or
        $policyJson.launcherPayloadExecutablePathRecognized -ne $true -or
        $policyJson.launcherPayloadBasenameOnlyRejected -ne $true -or
        $policyJson.managedNativeTraySignatureRecognized -ne $true -or
        $policyJson.nativeLaunchSerializedExactlyOnce -ne $true -or
        [int]$policyJson.serializedLaunchCount -ne 1 -or [int]$policyJson.nativeTrayReadyWindowGraceMs -ne 3000 -or
        [int]$policyJson.initialObservationMinimumMs -ne 10000 -or
        [int]$policyJson.initialObservationStabilityMs -ne 3000 -or [int]$policyJson.initialObservationMaximumMs -ne 20000) {
        throw "Quiet-policy behavior gate fields failed: $($policy.Stdout)"
    }
    $probes += $policy

    $lineage = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--quiet-lineage-self-test'
    try { $lineageJson = $lineage.Stdout | ConvertFrom-Json -ErrorAction Stop } catch { throw "Quiet-lineage behavior gate is not JSON: $($lineage.Stdout)" }
    if ($lineageJson.passed -ne $true -or $lineageJson.bufferedMultiHopPromoted -ne $true -or
        $lineageJson.liveDescendantPromoted -ne $true -or $lineageJson.unrelatedIgnored -ne $true -or
        $lineageJson.unqualifiedPidsNotActionable -ne $true -or $lineageJson.callbackMultiHopPromoted -ne $true -or
        $lineageJson.callbackUnrelatedIgnored -ne $true -or $lineageJson.exactArgumentIdentityMatched -ne $true -or
        $lineageJson.sameExecutableDifferentArgumentsIgnored -ne $true -or $lineageJson.argumentCaseDifferenceIgnored -ne $true -or
        $lineageJson.quotedArgumentBoundaryPreserved -ne $true -or $lineageJson.extraArgumentIgnored -ne $true -or
        $lineageJson.launchHashPathCaseInsensitive -ne $true -or $lineageJson.launchHashArgumentCaseSensitive -ne $true -or
        $lineageJson.launchHashQuotedBoundaryPreserved -ne $true -or [int]$lineageJson.snapshotCalls -ne 1 -or
        [int]$lineageJson.externalProcessesStarted -ne 0 -or
        ((@($lineageJson.tracked | ForEach-Object { [int]$_ }) -join ',') -ne '4100,4101,4102,4103')) {
        throw "Quiet-lineage behavior gate fields failed: $($lineage.Stdout)"
    }
    $probes += $lineage

    $performance = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--quiet-performance-probe'
    try { $performanceJson = $performance.Stdout | ConvertFrom-Json -ErrorAction Stop } catch { throw "Quiet-performance behavior gate is not JSON: $($performance.Stdout)" }
    if ($performanceJson.passed -ne $true -or [int]$performanceJson.steadyStateSystemSnapshots -ne 0 -or
        [int]$performanceJson.steadyStateTopWindowEnumerations -ne 0 -or [int]$performanceJson.callbackWholeSystemSnapshots -ne 0 -or
        $performanceJson.steadyGlobalWindowHookRunning -ne $false -or $performanceJson.steadyGlobalLineageWatcherRunning -ne $false -or
        [int]$performanceJson.steadyScopedWindowHookCount -lt 1 -or $performanceJson.steadyJobObserverRunning -ne $true -or
        $performanceJson.watcherStoppedAfterInitial -ne $true -or $performanceJson.windowHookStopped -ne $true -or
        $performanceJson.timersDisposed -ne $true -or $performanceJson.callbackDrainPreservesEvidence -ne $true -or
        [int]$performanceJson.postDisposeCallbacksExecuted -ne 0 -or $performanceJson.manualOpenImmediate -ne $true -or
        [int]$performanceJson.manualWindowRehideCount -ne 0) {
        throw "Quiet-performance behavior gate fields failed: $($performance.Stdout)"
    }
    $probes += $performance

    $uiContract = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--ui-contract'
    try { $uiJson = $uiContract.Stdout | ConvertFrom-Json -ErrorAction Stop } catch { throw "UI-contract behavior gate is not JSON: $($uiContract.Stdout)" }
    if ($uiJson.layout -ne 'responsive-native-control-center' -or $uiJson.defaultFilter -ne 'All routes' -or
        $uiJson.defaultViewIsCompleteRouteInventory -ne $true -or $uiJson.stateModeSeparated -ne $true -or
        $uiJson.startInTrayPrePaintSuppression -ne $true -or $uiJson.refreshIsReadOnly -ne $true -or
        $uiJson.defaultNewMode -ne 'Window' -or $uiJson.appsAggregatedByCanonicalTarget -ne $true -or
        $uiJson.appsNeverAggregatedByDisplayName -ne $true -or $uiJson.allRoutesRemainRouteLevel -ne $true -or
        $uiJson.aggregateNonBulkActionsFailClosed -ne $true -or $uiJson.aggregateBulkDisableTransactional -ne $true -or $uiJson.aggregateManageRoutesOneClick -ne $true -or
        $uiJson.actualApplicationTrayIconOnly -ne $true -or
        $uiJson.capabilityAwareActions -ne $true -or $uiJson.expertBootChangeConfirmation -ne $true -or
        $uiJson.elevationAndRebootReasons -ne $true -or $uiJson.externalAuthorityState -ne $true -or
        $uiJson.multiActionTasksDisableEditQuietAndRun -ne $true -or
        @($uiJson.columns) -notcontains 'Status' -or @($uiJson.columns) -notcontains 'Mode') {
        throw "UI-contract behavior gate fields failed: $($uiContract.Stdout)"
    }
    $probes += $uiContract

    $uiSelfTest = Invoke-StagedCliProbe -ExecutablePath $executable -Arguments '--ui-self-test'
    if ($uiSelfTest.Stdout -notmatch '^UI_SELF_TEST passed ' -or
        $uiSelfTest.Stdout -notmatch '(?:^|\s)scales=100,150,200(?:\s|$)' -or
        $uiSelfTest.Stdout -notmatch '(?:^|\s)startInTrayInitialVisible=false(?:\s|$)' -or
        $uiSelfTest.Stdout -notmatch '(?:^|\s)stateModeSeparated=true(?:\s|$)' -or
        $uiSelfTest.Stdout -notmatch '(?:^|\s)capabilityAwareActions=true(?:\s|$)' -or
        $uiSelfTest.Stdout -notmatch '(?:^|\s)expertConfirmation=true(?:\s|$)' -or
        $uiSelfTest.Stdout -notmatch '(?:^|\s)externalAuthority=true(?:\s|$)' -or
        $uiSelfTest.Stdout -notmatch '(?:^|\s)multiActionFailClosed=true(?:\s|$)') {
        throw "UI self-test behavior gate failed: $($uiSelfTest.Stdout)"
    }
    $probes += $uiSelfTest
    return @($probes)
}

function Assert-BuildLayout {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [switch]$RunBehaviorGate
    )
    $requiredFiles = @($ExecutableName, $ApplicationDllName, $IconName)
    foreach ($requiredFile in $requiredFiles) {
        $requiredPath = Join-Path $Directory $requiredFile
        if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
            throw "Staged build is missing required file: $requiredPath"
        }
        if ((Get-Item -LiteralPath $requiredPath).Length -le 0) {
            throw "Staged build contains an empty required file: $requiredPath"
        }
    }

    [void][Reflection.AssemblyName]::GetAssemblyName((Join-Path $Directory $ApplicationDllName))
    Add-Type -AssemblyName System.Drawing
    $executableIcon = [System.Drawing.Icon]::ExtractAssociatedIcon((Join-Path $Directory $ExecutableName))
    if ($null -eq $executableIcon) {
        throw "Executable icon extraction failed: $(Join-Path $Directory $ExecutableName)"
    }
    try {
        $embeddedIconSize = "$($executableIcon.Width)x$($executableIcon.Height)"
    } finally {
        $executableIcon.Dispose()
    }

    $bundledIcon = New-Object System.Drawing.Icon (Join-Path $Directory $IconName)
    try {
        $bundledIconSize = "$($bundledIcon.Width)x$($bundledIcon.Height)"
    } finally {
        $bundledIcon.Dispose()
    }

    $behaviorGate = if ($RunBehaviorGate) { @(Assert-StagedBehavior -Directory $Directory) } else { @() }

    return [pscustomobject][ordered]@{
        ExecutableSha256 = (Get-FileHash -LiteralPath (Join-Path $Directory $ExecutableName) -Algorithm SHA256).Hash
        BundledIconSha256 = (Get-FileHash -LiteralPath (Join-Path $Directory $IconName) -Algorithm SHA256).Hash
        EmbeddedIconSize = $embeddedIconSize
        BundledIconSize = $bundledIconSize
        BehaviorGate = @($behaviorGate)
    }
}

if (-not ('MichStartupMasterBuildNativeProcess' -as [type])) {
    Add-Type @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

public sealed class MichStartupMasterBuildProcessSnapshot
{
    public int ProcessId;
    public int ParentProcessId;
    public string ImagePath;
    public string CommandLine;
    public long StartTimeUtcFileTime;
}

public static class MichStartupMasterBuildNativeProcess
{
    private const uint PROCESS_TERMINATE = 0x0001;
    private const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
    private const int ProcessCommandLineInformation = 60;

    [StructLayout(LayoutKind.Sequential)]
    private struct FILETIME { public uint Low; public uint High; }

    [StructLayout(LayoutKind.Sequential)]
    private struct UNICODE_STRING
    {
        public ushort Length;
        public ushort MaximumLength;
        public IntPtr Buffer;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct PROCESS_BASIC_INFORMATION
    {
        public IntPtr Reserved1;
        public IntPtr PebBaseAddress;
        public IntPtr Reserved2_0;
        public IntPtr Reserved2_1;
        public IntPtr UniqueProcessId;
        public IntPtr InheritedFromUniqueProcessId;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr OpenProcess(uint access, bool inheritHandle, int processId);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool QueryFullProcessImageName(
        IntPtr process, uint flags, StringBuilder imagePath, ref uint size);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetProcessTimes(
        IntPtr process, out FILETIME creation, out FILETIME exit,
        out FILETIME kernel, out FILETIME user);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool TerminateProcess(IntPtr process, uint exitCode);

    [DllImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr handle);

    [DllImport("ntdll.dll")]
    private static extern int NtQueryInformationProcess(
        IntPtr process, int informationClass, IntPtr information,
        int informationLength, out int returnLength);

    [DllImport("ntdll.dll", EntryPoint = "NtQueryInformationProcess")]
    private static extern int NtQueryBasicInformationProcess(
        IntPtr process, int informationClass,
        out PROCESS_BASIC_INFORMATION information,
        int informationLength, out int returnLength);

    public static bool TrySnapshot(
        int processId, out MichStartupMasterBuildProcessSnapshot snapshot,
        out string error)
    {
        snapshot = null;
        error = "";
        IntPtr process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, processId);
        if (process == IntPtr.Zero)
        {
            error = "OpenProcess failed: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
            return false;
        }

        IntPtr buffer = IntPtr.Zero;
        try
        {
            uint pathLength = 32768;
            StringBuilder path = new StringBuilder((int)pathLength);
            if (!QueryFullProcessImageName(process, 0, path, ref pathLength))
            {
                error = "QueryFullProcessImageName failed: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                return false;
            }

            FILETIME creation, exit, kernel, user;
            if (!GetProcessTimes(process, out creation, out exit, out kernel, out user))
            {
                error = "GetProcessTimes failed: " + new Win32Exception(Marshal.GetLastWin32Error()).Message;
                return false;
            }
            long startFileTime = ((long)creation.High << 32) | creation.Low;

            PROCESS_BASIC_INFORMATION basic;
            int basicReturned;
            int basicStatus = NtQueryBasicInformationProcess(
                process, 0, out basic,
                Marshal.SizeOf(typeof(PROCESS_BASIC_INFORMATION)), out basicReturned);
            if (basicStatus < 0)
            {
                error = "NtQueryInformationProcess(0) failed with NTSTATUS 0x" + basicStatus.ToString("X8");
                return false;
            }

            int required;
            NtQueryInformationProcess(process, ProcessCommandLineInformation, IntPtr.Zero, 0, out required);
            if (required <= 0 || required > (1024 * 1024))
            {
                error = "NtQueryInformationProcess(60) returned an invalid required length: " + required;
                return false;
            }
            buffer = Marshal.AllocHGlobal(required);
            int status = NtQueryInformationProcess(
                process, ProcessCommandLineInformation, buffer, required, out required);
            if (status < 0)
            {
                error = "NtQueryInformationProcess(60) failed with NTSTATUS 0x" + status.ToString("X8");
                return false;
            }
            UNICODE_STRING command = (UNICODE_STRING)Marshal.PtrToStructure(buffer, typeof(UNICODE_STRING));
            string commandLine = command.Length == 0
                ? ""
                : Marshal.PtrToStringUni(command.Buffer, command.Length / 2);

            snapshot = new MichStartupMasterBuildProcessSnapshot();
            snapshot.ProcessId = processId;
            snapshot.ParentProcessId = unchecked((int)basic.InheritedFromUniqueProcessId.ToInt64());
            snapshot.ImagePath = path.ToString();
            snapshot.CommandLine = commandLine ?? "";
            snapshot.StartTimeUtcFileTime = startFileTime;
            return true;
        }
        finally
        {
            if (buffer != IntPtr.Zero) Marshal.FreeHGlobal(buffer);
            CloseHandle(process);
        }
    }

    public static bool TerminateExact(
        int processId, string expectedPath, long expectedStartTime,
        out string error)
    {
        error = "";
        IntPtr process = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
            false, processId);
        if (process == IntPtr.Zero)
        {
            error = "OpenProcess for termination failed: " +
                new Win32Exception(Marshal.GetLastWin32Error()).Message;
            return false;
        }
        try
        {
            uint pathLength = 32768;
            StringBuilder path = new StringBuilder((int)pathLength);
            if (!QueryFullProcessImageName(process, 0, path, ref pathLength))
            {
                error = "QueryFullProcessImageName before termination failed: " +
                    new Win32Exception(Marshal.GetLastWin32Error()).Message;
                return false;
            }
            FILETIME creation, exit, kernel, user;
            if (!GetProcessTimes(process, out creation, out exit, out kernel, out user))
            {
                error = "GetProcessTimes before termination failed: " +
                    new Win32Exception(Marshal.GetLastWin32Error()).Message;
                return false;
            }
            long startTime = ((long)creation.High << 32) | creation.Low;
            if (!String.Equals(
                    Path.GetFullPath(path.ToString()),
                    Path.GetFullPath(expectedPath),
                    StringComparison.OrdinalIgnoreCase) ||
                startTime != expectedStartTime)
            {
                error = "process path/start-time generation no longer matches";
                return false;
            }
            if (!TerminateProcess(process, 1))
            {
                error = "TerminateProcess failed: " +
                    new Win32Exception(Marshal.GetLastWin32Error()).Message;
                return false;
            }
            return true;
        }
        finally { CloseHandle(process); }
    }
}
'@
}

function Get-NativeProcessSnapshot {
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [switch]$AllowExitedRace
    )
    if ($null -eq (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) { return $null }
    [MichStartupMasterBuildProcessSnapshot]$snapshot = $null
    $nativeError = ''
    if (-not [MichStartupMasterBuildNativeProcess]::TrySnapshot($ProcessId, [ref]$snapshot, [ref]$nativeError)) {
        if ($null -eq (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) { return $null }
        if ($AllowExitedRace) {
            # CloseMainWindow can leave a terminating process briefly discoverable while
            # QueryFullProcessImageName already rejects its torn-down native object. Wait
            # only for that exact PID to disappear; a still-live unqueryable generation
            # remains a hard fail instead of being mistaken for successful termination.
            $exitRaceDeadline = [DateTime]::UtcNow.AddSeconds(10)
            do {
                $exitingProcess = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
                if ($null -eq $exitingProcess) { return $null }
                try { if ($exitingProcess.HasExited) { return $null } }
                catch { }
                finally { if ($null -ne $exitingProcess) { $exitingProcess.Dispose() } }
                Start-Sleep -Milliseconds 50
            } while ([DateTime]::UtcNow -lt $exitRaceDeadline)
        }
        throw "Could not capture bounded native identity for PID ${ProcessId}: $nativeError"
    }
    return $snapshot
}

function Get-CommandArguments {
    param(
        [string]$CommandLine,
        [Parameter(Mandatory = $true)][string]$ExecutablePath
    )
    if ([string]::IsNullOrWhiteSpace($CommandLine)) {
        return ''
    }
    $trimmed = $CommandLine.Trim()
    if ($trimmed.StartsWith('"')) {
        $closingQuote = $trimmed.IndexOf('"', 1)
        if ($closingQuote -gt 0) {
            $commandExecutable = $trimmed.Substring(1, $closingQuote - 1)
            if ([string]::Equals((Get-FullPath $commandExecutable), (Get-FullPath $ExecutablePath), [StringComparison]::OrdinalIgnoreCase)) {
                return $trimmed.Substring($closingQuote + 1).Trim()
            }
        }
    }
    if ($trimmed.StartsWith($ExecutablePath, [StringComparison]::OrdinalIgnoreCase)) {
        return $trimmed.Substring($ExecutablePath.Length).Trim()
    }
    throw "Could not safely parse process command line: $CommandLine"
}

function Get-LiveProcessState {
    param([Parameter(Mandatory = $true)][string]$ExecutablePath)
    $normalizedExecutable = Get-FullPath $ExecutablePath
    $rows = @()
    foreach ($process in @(Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($ExecutableName)) -ErrorAction SilentlyContinue)) {
        $snapshot = Get-NativeProcessSnapshot -ProcessId ([int]$process.Id)
        if ($null -eq $snapshot -or -not [string]::Equals((Get-FullPath $snapshot.ImagePath), $normalizedExecutable, [StringComparison]::OrdinalIgnoreCase)) { continue }
        $launchArguments = Get-CommandArguments -CommandLine $snapshot.CommandLine -ExecutablePath $ExecutablePath
        $rows += [pscustomobject][ordered]@{
            ProcessId = [int]$snapshot.ProcessId
            ParentProcessId = [int]$snapshot.ParentProcessId
            ExecutablePath = $normalizedExecutable
            CommandLine = [string]$snapshot.CommandLine
            Arguments = $launchArguments
            Mode = if ([string]::IsNullOrWhiteSpace($launchArguments)) { 'interactive' } elseif ($launchArguments -match '^--agent(?:\s|$)') { 'agent' } elseif ($launchArguments -match '^--tray-run(?:\s|$)') { 'tray-wrapper' } else { 'other' }
            StartTimeUtcFileTime = [long]$snapshot.StartTimeUtcFileTime
        }
    }
    return @($rows | Sort-Object -Property ProcessId)
}

function Get-AllManagerProcessState {
    param([Parameter(Mandatory = $true)][string[]]$ExecutablePaths)
    $rows = @()
    foreach ($executablePath in @($ExecutablePaths | ForEach-Object { Get-FullPath ([string]$_) } | Sort-Object -Unique)) {
        if (Test-Path -LiteralPath $executablePath -PathType Leaf) {
            $rows += @(Get-LiveProcessState -ExecutablePath $executablePath)
        }
    }
    return @($rows | Sort-Object ExecutablePath, ProcessId)
}

function Get-RecordedProcessCurrentState {
    param(
        [Parameter(Mandatory = $true)]$State,
        [switch]$GenerationChangeMeansExited
    )
    $current = Get-NativeProcessSnapshot -ProcessId ([int]$State.ProcessId) -AllowExitedRace:$GenerationChangeMeansExited
    if ($null -eq $current) { return $null }
    $pathMatches = [string]::Equals((Get-FullPath $current.ImagePath), (Get-FullPath ([string]$State.ExecutablePath)), [StringComparison]::OrdinalIgnoreCase)
    $generationMatches = [long]$current.StartTimeUtcFileTime -eq [long]$State.StartTimeUtcFileTime
    if (-not $pathMatches -or -not $generationMatches) {
        if ($GenerationChangeMeansExited) { return $null }
        throw "PID $($State.ProcessId) no longer has the recorded exact-path/start-time identity; refusing to signal or stop it."
    }
    return $current
}

function Stop-RecordedProcesses {
    param(
        [Parameter(Mandatory = $true)][object[]]$ProcessState,
        [Parameter(Mandatory = $true)][int]$WaitSeconds
    )
    $ordered = @($ProcessState | Sort-Object @{ Expression = { if ($_.Mode -eq 'agent') { 0 } else { 1 } } }, ProcessId)
    foreach ($state in $ordered) {
        $current = Get-RecordedProcessCurrentState -State $state -GenerationChangeMeansExited
        if ($null -eq $current) { continue }
        $process = Get-Process -Id $state.ProcessId -ErrorAction SilentlyContinue
        if ($null -eq $process) { continue }
        try {
            # Materialize and retain the process handle before the final identity
            # validation. Subsequent stop/wait/kill operations therefore remain bound
            # to this exact generation even if its PID later becomes reusable.
            [void]$process.Handle
            $current = Get-RecordedProcessCurrentState -State $state -GenerationChangeMeansExited
            if ($null -eq $current) { continue }
            [void]$process.CloseMainWindow()
            $exitedGracefully = $false
            # A hidden agent can begin orderly teardown while CloseMainWindow reports
            # false because it has no conventional visible main window. The exact
            # Process object still gives us a bounded, generation-safe exit signal.
            try { $exitedGracefully = $process.WaitForExit([Math]::Max(0, $WaitSeconds) * 1000) }
            catch {
                try { $exitedGracefully = $process.HasExited }
                catch { }
            }
            if ($exitedGracefully) { continue }
            try {
                $process.Kill()
                if (-not $process.WaitForExit(5000)) {
                    throw "Exact retained process handle did not terminate within 5000 ms."
                }
            } catch {
                $confirmedExited = $false
                try { $confirmedExited = $process.HasExited }
                catch { }
                if (-not $confirmedExited) {
                    throw "Could not terminate exact retained generation PID $($state.ProcessId): $($_.Exception.Message)"
                }
            }
        } finally {
            $process.Dispose()
        }
    }
}

function Get-ProcessSignature {
    param([Parameter(Mandatory = $true)]$State)
    $argumentBytes = [Text.Encoding]::UTF8.GetBytes([string]$State.Arguments)
    return (Get-FullPath ([string]$State.ExecutablePath)).ToLowerInvariant() + '|' +
        ([string]$State.Mode).ToLowerInvariant() + '|' + [Convert]::ToBase64String($argumentBytes)
}

function Get-ProcessGenerationKey {
    param([Parameter(Mandatory = $true)]$State)
    return (Get-FullPath ([string]$State.ExecutablePath)).ToLowerInvariant() + '|' +
        [string][int]$State.ProcessId + '|' + [string][long]$State.StartTimeUtcFileTime
}

function Add-TransactionDescendantProcesses {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Baseline,
        [Parameter(Mandatory = $true)][string[]]$ExecutablePaths,
        [Parameter(Mandatory = $true)][Collections.Generic.List[object]]$StartedProcessJournal,
        [int]$WaitMilliseconds = 2000
    )
    $baselineKeys = @{}; foreach ($row in $Baseline) { $baselineKeys[(Get-ProcessGenerationKey $row)] = $true }
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMilliseconds)
    $lastJournalCount = -1
    do {
        $knownByProcessId = @{}
        foreach ($row in $StartedProcessJournal.ToArray()) { $knownByProcessId[[int]$row.ProcessId] = $row }
        foreach ($row in @(Get-AllManagerProcessState -ExecutablePaths $ExecutablePaths)) {
            $key = Get-ProcessGenerationKey $row
            if ($baselineKeys.ContainsKey($key)) { continue }
            $alreadyRecorded = $false
            foreach ($journalRow in $StartedProcessJournal.ToArray()) {
                if ([string]::Equals((Get-ProcessGenerationKey $journalRow), $key, [StringComparison]::OrdinalIgnoreCase)) { $alreadyRecorded = $true; break }
            }
            if ($alreadyRecorded) { continue }
            if ($knownByProcessId.ContainsKey([int]$row.ParentProcessId)) {
                $parent = $knownByProcessId[[int]$row.ParentProcessId]
                if ([long]$row.StartTimeUtcFileTime -ge [long]$parent.StartTimeUtcFileTime) {
                    $StartedProcessJournal.Add($row)
                    $knownByProcessId[[int]$row.ProcessId] = $row
                }
            }
        }
        if ($StartedProcessJournal.Count -eq $lastJournalCount) { Start-Sleep -Milliseconds 100 }
        $lastJournalCount = $StartedProcessJournal.Count
    } while ([DateTime]::UtcNow -lt $deadline)
}

function Assert-ExactProcessMultiplicity {
    param(
        [Parameter(Mandatory = $true)][object[]]$ProcessState,
        [Parameter(Mandatory = $true)][string[]]$ExecutablePaths,
        [int]$WaitSeconds = 10
    )
    $expected = @{}
    foreach ($state in $ProcessState) {
        $signature = Get-ProcessSignature $state
        if (-not $expected.ContainsKey($signature)) { $expected[$signature] = 0 }
        $expected[$signature]++
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($WaitSeconds)
    do {
        $actualRows = @(Get-AllManagerProcessState -ExecutablePaths $ExecutablePaths)
        $actual = @{}
        foreach ($state in $actualRows) {
            $signature = Get-ProcessSignature $state
            if (-not $actual.ContainsKey($signature)) { $actual[$signature] = 0 }
            $actual[$signature]++
        }
        $mismatches = @()
        foreach ($signature in @((@($expected.Keys) + @($actual.Keys)) | Sort-Object -Unique)) {
            $expectedCount = if ($expected.ContainsKey($signature)) { [int]$expected[$signature] } else { 0 }
            $actualCount = if ($actual.ContainsKey($signature)) { [int]$actual[$signature] } else { 0 }
            if ($expectedCount -ne $actualCount) {
                $mismatches += "$signature expected=$expectedCount actual=$actualCount"
            }
        }
        if ($mismatches.Count -eq 0) { return @($actualRows) }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Exact process multiplicity restoration failed: $($mismatches -join '; ')"
}

function Restart-RecordedProcesses {
    param(
        [Parameter(Mandatory = $true)][object[]]$ProcessState,
        [Parameter(Mandatory = $true)][string[]]$ExecutablePaths,
        [Collections.Generic.List[object]]$StartedProcessJournal
    )
    $results = @()
    $baseline = @(Get-AllManagerProcessState -ExecutablePaths $ExecutablePaths)
    $groups = @(
        $ProcessState |
            Group-Object -Property { Get-ProcessSignature $_ } |
            Sort-Object @{ Expression = {
                $mode = [string]$_.Group[0].Mode
                if ($mode -eq 'agent') { 0 } elseif ($mode -eq 'interactive') { 1 } elseif ($mode -eq 'tray-wrapper') { 2 } else { 3 }
            } }, Name
    )
    foreach ($group in $groups) {
        $representative = $group.Group[0]
        $executablePath = Get-FullPath ([string]$representative.ExecutablePath)
        $liveRows = @(Get-LiveProcessState -ExecutablePath $executablePath)
        $surviving = @($liveRows | Where-Object { (Get-ProcessSignature $_) -eq $group.Name }).Count
        $missing = [Math]::Max(0, [int]$group.Count - $surviving)
        $startedIds = @()
        $startedProcesses = @()
        for ($index = 0; $index -lt $missing; $index++) {
            # Bypass ShellExecute, which appends whitespace to the recorded command line.
            $startInfo = New-Object Diagnostics.ProcessStartInfo
            $startInfo.FileName = $executablePath
            $startInfo.WorkingDirectory = Split-Path -Path $executablePath -Parent
            $startInfo.Arguments = [string]$representative.Arguments
            $startInfo.UseShellExecute = $false
            $startInfo.CreateNoWindow = -not [string]::IsNullOrWhiteSpace([string]$representative.Arguments)
            $started = [Diagnostics.Process]::Start($startInfo)
            try {
                $startedIds += [int]$started.Id
                $startedSnapshot = $null
                $snapshotDeadline = [DateTime]::UtcNow.AddSeconds(2)
                do {
                    $startedSnapshot = Get-NativeProcessSnapshot -ProcessId ([int]$started.Id)
                    if ($null -ne $startedSnapshot) { break }
                    Start-Sleep -Milliseconds 50
                } while ([DateTime]::UtcNow -lt $snapshotDeadline)
                if ($null -eq $startedSnapshot) {
                    throw "Could not capture the exact path/start-time generation for started PID $($started.Id); rollback ownership cannot be proven."
                }
                if (-not (Test-PathsEqual -LeftPath ([string]$startedSnapshot.ImagePath) -RightPath $executablePath)) {
                    throw "Started PID $($started.Id) does not use the exact deployment executable."
                }
                $startedProcessState = [pscustomobject][ordered]@{
                    ProcessId = [int]$startedSnapshot.ProcessId
                    ParentProcessId = [int]$startedSnapshot.ParentProcessId
                    ExecutablePath = Get-FullPath ([string]$startedSnapshot.ImagePath)
                    CommandLine = [string]$startedSnapshot.CommandLine
                    Arguments = Get-CommandArguments -CommandLine ([string]$startedSnapshot.CommandLine) -ExecutablePath $executablePath
                    Mode = [string]$representative.Mode
                    StartTimeUtcFileTime = [long]$startedSnapshot.StartTimeUtcFileTime
                }
                $startedProcesses += $startedProcessState
                if ($null -ne $StartedProcessJournal) { $StartedProcessJournal.Add($startedProcessState) }
            } finally {
                $started.Dispose()
            }
        }
        $results += [pscustomobject][ordered]@{
            Mode = [string]$representative.Mode
            Arguments = [string]$representative.Arguments
            DesiredCount = [int]$group.Count
            SurvivingBeforeRestart = $surviving
            StartedCount = $missing
            StartedProcessIds = @($startedIds)
            StartedProcesses = @($startedProcesses)
        }
        # Give the agent a chance to reconstruct wrapper children before the next
        # signature is considered. The next group re-reads live state and therefore
        # never duplicates a wrapper that a surviving/restarted agent already created.
        if ($missing -gt 0 -and $null -ne $StartedProcessJournal) {
            Add-TransactionDescendantProcesses -Baseline $baseline -ExecutablePaths $ExecutablePaths -StartedProcessJournal $StartedProcessJournal -WaitMilliseconds 600
        }
    }
    if ($null -ne $StartedProcessJournal) {
        Add-TransactionDescendantProcesses -Baseline $baseline -ExecutablePaths $ExecutablePaths -StartedProcessJournal $StartedProcessJournal -WaitMilliseconds 1200
    }
    [void](Assert-ExactProcessMultiplicity -ProcessState $ProcessState -ExecutablePaths $ExecutablePaths -WaitSeconds 10)
    return @($results)
}

function Test-PathsEqual {
    param([string]$LeftPath, [string]$RightPath)
    try {
        return -not [string]::IsNullOrWhiteSpace($LeftPath) -and
            -not [string]::IsNullOrWhiteSpace($RightPath) -and
            [string]::Equals((Get-FullPath $LeftPath), (Get-FullPath $RightPath), [StringComparison]::OrdinalIgnoreCase)
    } catch { return $false }
}

function Get-OwnedLaunchPaths {
    param([Parameter(Mandatory = $true)][string]$DeploymentDirectory)
    $installedDirectory = Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster'
    return [pscustomobject][ordered]@{
        Executables = @(
            (Join-Path $DeploymentDirectory $ExecutableName),
            (Join-Path (Join-Path $Root 'build') $ExecutableName),
            (Join-Path $installedDirectory $ExecutableName)
        ) | Select-Object -Unique
        Launchers = @(
            (Join-Path $DeploymentDirectory $LegacyLauncherName),
            (Join-Path (Join-Path $Root 'build') $LegacyLauncherName),
            (Join-Path $installedDirectory $LegacyLauncherName)
        ) | Select-Object -Unique
    }
}

function Test-OwnedAgentLaunch {
    param(
        [string]$TargetPath,
        [string]$Arguments,
        [Parameter(Mandatory = $true)][string[]]$AllowedExecutables,
        [Parameter(Mandatory = $true)][string[]]$AllowedLaunchers
    )
    $argumentText = ([string]$Arguments).Trim()
    foreach ($executable in $AllowedExecutables) {
        if ((Test-PathsEqual $TargetPath $executable) -and [string]::Equals($argumentText, '--agent', [StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }
    $expandedTarget = [Environment]::ExpandEnvironmentVariables(([string]$TargetPath).Trim())
    $allowedScriptHosts = @(
        (Join-Path $env:SystemRoot 'System32\wscript.exe'),
        (Join-Path $env:SystemRoot 'System32\cscript.exe'),
        (Join-Path $env:SystemRoot 'SysWOW64\wscript.exe'),
        (Join-Path $env:SystemRoot 'SysWOW64\cscript.exe')
    )
    if (@($allowedScriptHosts | Where-Object { Test-PathsEqual -LeftPath $_ -RightPath $expandedTarget }).Count -ne 1) { return $false }
    foreach ($launcher in $AllowedLaunchers) {
        foreach ($exactArguments in @('"' + $launcher + '"', $launcher, '//B //NoLogo "' + $launcher + '"', '//B "' + $launcher + '"')) {
            if ([string]::Equals($argumentText, $exactArguments, [StringComparison]::OrdinalIgnoreCase)) { return $true }
        }
    }
    return $false
}

function Test-OwnedAppTaskXml {
    param(
        [string]$Xml,
        [Parameter(Mandatory = $true)][string[]]$AllowedExecutables,
        [Parameter(Mandatory = $true)][string[]]$AllowedLaunchers
    )
    try {
        [xml]$document = $Xml
        $execNodes = @($document.SelectNodes("//*[local-name()='Actions']/*[local-name()='Exec']"))
        if ($execNodes.Count -ne 1) { return $false }
        $commandNode = $execNodes[0].SelectSingleNode("./*[local-name()='Command']")
        $argumentsNode = $execNodes[0].SelectSingleNode("./*[local-name()='Arguments']")
        $command = if ($null -eq $commandNode) { '' } else { [string]$commandNode.InnerText }
        $arguments = if ($null -eq $argumentsNode) { '' } else { ([string]$argumentsNode.InnerText).Trim() }
        if (Test-OwnedAgentLaunch -TargetPath $command -Arguments $arguments -AllowedExecutables $AllowedExecutables -AllowedLaunchers $AllowedLaunchers) { return $true }
        foreach ($executable in $AllowedExecutables) {
            if ((Test-PathsEqual $command $executable) -and
                ($arguments -match '^--start-in-tray$' -or $arguments -match '^--tray-run\s+[A-Za-z0-9+/=]+$')) {
                return $true
            }
        }
    } catch { }
    return $false
}

function Get-ManagedTaskNamespaceRows {
    $rows = @()
    try {
        $scheduler = New-Object -ComObject 'Schedule.Service'
        $scheduler.Connect()
        $stack = New-Object Collections.Stack
        $stack.Push($scheduler.GetFolder('\MichStartupMaster'))
        while ($stack.Count -gt 0) {
            $folder = $stack.Pop()
            foreach ($task in @($folder.GetTasks(1))) {
                $rows += [pscustomobject][ordered]@{
                    TaskPath = [string]$task.Path
                    Xml = [string]$task.Xml
                    SecurityDescriptor = [string]$task.GetSecurityDescriptor(7)
                }
            }
            foreach ($child in @($folder.GetFolders(0))) { $stack.Push($child) }
        }
    } catch {
        # A missing namespace is a valid empty pre-deploy state. Other COM failures
        # must be distinguishable from absence so a deployment never proceeds with
        # an incomplete rollback artifact.
        if ($_.Exception.Message -notmatch 'cannot find|not exist|0x80070002') { throw }
    }
    return @($rows | Sort-Object -Property TaskPath)
}

function Get-OwnedManagedTaskRows {
    param(
        [Parameter(Mandatory = $true)][string[]]$AllowedExecutables,
        [Parameter(Mandatory = $true)][string[]]$AllowedLaunchers
    )
    return @(
        Get-ManagedTaskNamespaceRows |
            Where-Object { Test-OwnedAppTaskXml -Xml ([string]$_.Xml) -AllowedExecutables $AllowedExecutables -AllowedLaunchers $AllowedLaunchers }
    )
}

function Set-ManagedTaskSecurityDescriptor {
    param(
        [Parameter(Mandatory = $true)][string]$TaskPath,
        [Parameter(Mandatory = $true)][string]$SecurityDescriptor
    )
    $separator = $TaskPath.LastIndexOf('\')
    if ($separator -lt 0 -or $separator -eq ($TaskPath.Length - 1)) { throw "Invalid scheduled-task path: $TaskPath" }
    $folderPath = if ($separator -eq 0) { '\' } else { $TaskPath.Substring(0, $separator) }
    $taskName = $TaskPath.Substring($separator + 1)
    $scheduler = New-Object -ComObject 'Schedule.Service'
    $scheduler.Connect()
    $folder = $scheduler.GetFolder($folderPath)
    $task = $folder.GetTask($taskName)
    # TASK_DONT_ADD_PRINCIPAL_ACE (0x10) prevents the scheduler from silently
    # widening the exact captured DACL during restoration.
    $task.SetSecurityDescriptor($SecurityDescriptor, 16)
}

function Test-SecurityDescriptorSemanticallyEqual {
    param(
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Actual
    )
    try {
        $left = New-Object Security.AccessControl.RawSecurityDescriptor($Expected)
        $right = New-Object Security.AccessControl.RawSecurityDescriptor($Actual)
        if (-not [string]::Equals([string]$left.Owner, [string]$right.Owner, [StringComparison]::OrdinalIgnoreCase) -or
            -not [string]::Equals([string]$left.Group, [string]$right.Group, [StringComparison]::OrdinalIgnoreCase)) {
            return $false
        }
        foreach ($aclName in @('DiscretionaryAcl', 'SystemAcl')) {
            $leftAcl = $left.$aclName
            $rightAcl = $right.$aclName
            if (($null -eq $leftAcl) -ne ($null -eq $rightAcl)) { return $false }
            if ($null -eq $leftAcl) { continue }
            if ($leftAcl.Count -ne $rightAcl.Count) { return $false }
            $leftAces = @(
                foreach ($ace in $leftAcl) {
                    $bytes = New-Object byte[] $ace.BinaryLength
                    $ace.GetBinaryForm($bytes, 0)
                    [Convert]::ToBase64String($bytes)
                }
            ) | Sort-Object
            $rightAces = @(
                foreach ($ace in $rightAcl) {
                    $bytes = New-Object byte[] $ace.BinaryLength
                    $ace.GetBinaryForm($bytes, 0)
                    [Convert]::ToBase64String($bytes)
                }
            ) | Sort-Object
            if (@(Compare-Object -ReferenceObject $leftAces -DifferenceObject $rightAces -SyncWindow 0).Count -ne 0) { return $false }
        }
        return $true
    } catch {
        return $false
    }
}

function Get-OwnedStartupShortcutRows {
    param(
        [Parameter(Mandatory = $true)][string[]]$AllowedExecutables,
        [Parameter(Mandatory = $true)][string[]]$AllowedLaunchers
    )
    $rows = @()
    $shell = New-Object -ComObject WScript.Shell
    $reservedPaths = @(
        (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Startup)) 'Mich Startup Master Agent.lnk'),
        (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::CommonStartup)) 'Mich Startup Master Agent.lnk')
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Select-Object -Unique
    foreach ($path in $reservedPaths) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
        try {
            $shortcut = $shell.CreateShortcut($path)
            if (Test-OwnedAgentLaunch -TargetPath ([string]$shortcut.TargetPath) -Arguments ([string]$shortcut.Arguments) -AllowedExecutables $AllowedExecutables -AllowedLaunchers $AllowedLaunchers) {
                $rows += [pscustomobject][ordered]@{
                    OriginalPath = Get-FullPath $path
                    TargetPath = [string]$shortcut.TargetPath
                    Arguments = [string]$shortcut.Arguments
                }
            }
        } catch { throw "Could not inspect Startup shortcut '$path': $($_.Exception.Message)" }
    }
    return @($rows | Sort-Object -Property OriginalPath)
}

function Test-OwnedStartupShortcutFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$AllowedExecutables,
        [Parameter(Mandatory = $true)][string[]]$AllowedLaunchers
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    $shell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shell.CreateShortcut($Path)
        return Test-OwnedAgentLaunch -TargetPath ([string]$shortcut.TargetPath) -Arguments ([string]$shortcut.Arguments) -AllowedExecutables $AllowedExecutables -AllowedLaunchers $AllowedLaunchers
    } catch {
        throw "Could not inspect reserved Startup shortcut '$Path': $($_.Exception.Message)"
    }
}

function Test-OwnedRegistryCommand {
    param(
        [string]$Command,
        [Parameter(Mandatory = $true)][string[]]$AllowedExecutables,
        [Parameter(Mandatory = $true)][string[]]$AllowedLaunchers
    )
    $expanded = [Environment]::ExpandEnvironmentVariables(([string]$Command).Trim())
    foreach ($executable in $AllowedExecutables) {
        foreach ($prefix in @('"' + $executable + '"', $executable)) {
            if ($expanded.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
                $remaining = $expanded.Substring($prefix.Length).Trim()
                if ([string]::Equals($remaining, '--agent', [StringComparison]::OrdinalIgnoreCase)) { return $true }
            }
        }
    }
    foreach ($hostName in @('wscript.exe', 'cscript.exe')) {
        $hostPaths = @((Join-Path $env:SystemRoot ('System32\' + $hostName)), (Join-Path $env:SystemRoot ('SysWOW64\' + $hostName)))
        foreach ($hostPath in $hostPaths) {
            foreach ($prefix in @('"' + $hostPath + '"', $hostPath)) {
                if (-not $expanded.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { continue }
                $remaining = $expanded.Substring($prefix.Length).Trim()
                foreach ($launcher in $AllowedLaunchers) {
                    foreach ($exactArguments in @('"' + $launcher + '"', $launcher, '//B //NoLogo "' + $launcher + '"', '//B "' + $launcher + '"')) {
                        if ([string]::Equals($remaining, $exactArguments, [StringComparison]::OrdinalIgnoreCase)) { return $true }
                    }
                }
            }
        }
    }
    return $false
}

function Get-OwnedRegistryLaunchRows {
    param(
        [Parameter(Mandatory = $true)][string[]]$AllowedExecutables,
        [Parameter(Mandatory = $true)][string[]]$AllowedLaunchers
    )
    $rows = @()
    $subKeys = @('Software\Microsoft\Windows\CurrentVersion\Run', 'Software\Microsoft\Windows\CurrentVersion\RunOnce')
    foreach ($hiveName in @('CurrentUser', 'LocalMachine')) {
        $hive = [Enum]::Parse([Microsoft.Win32.RegistryHive], $hiveName)
        foreach ($viewName in @('Registry64', 'Registry32')) {
            $view = [Enum]::Parse([Microsoft.Win32.RegistryView], $viewName)
            $baseKey = $null
            try {
                $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey($hive, $view)
                foreach ($subKeyPath in $subKeys) {
                    $key = $baseKey.OpenSubKey($subKeyPath, $false)
                    if ($null -eq $key) { continue }
                    try {
                        foreach ($valueName in $key.GetValueNames()) {
                            $data = $key.GetValue($valueName, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                            if ($null -eq $data -or -not (Test-OwnedRegistryCommand -Command ([string]$data) -AllowedExecutables $AllowedExecutables -AllowedLaunchers $AllowedLaunchers)) { continue }
                            $serialized = [Management.Automation.PSSerializer]::Serialize($data)
                            $rows += [pscustomobject][ordered]@{
                                Hive = $hiveName
                                View = $viewName
                                SubKey = $subKeyPath
                                Name = [string]$valueName
                                Kind = [string]$key.GetValueKind($valueName)
                                DataBase64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($serialized))
                                DataSha256 = Get-StringSha256 $serialized
                            }
                        }
                    } finally { $key.Dispose() }
                }
            } finally { if ($null -ne $baseKey) { $baseKey.Dispose() } }
        }
    }
    return @($rows | Sort-Object Hive, View, SubKey, Name -Unique)
}

function Get-RegistryRowIdentity {
    param([Parameter(Mandatory = $true)]$Row)
    return ([string]$Row.Hive).ToLowerInvariant() + '|' + ([string]$Row.View).ToLowerInvariant() + '|' + ([string]$Row.SubKey).ToLowerInvariant() + '|' + ([string]$Row.Name).ToLowerInvariant()
}

function New-ExternalStateSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$SnapshotDirectory,
        [Parameter(Mandatory = $true)][string]$DeploymentDirectory,
        [Parameter(Mandatory = $true)][string]$ProjectRoot,
        [Parameter(Mandatory = $true)][string]$TransactionDirectory,
        [Parameter(Mandatory = $true)][string]$SnapshotKind,
        [Parameter(Mandatory = $true)][string]$StageReceiptPath,
        [Parameter(Mandatory = $true)][string]$RestoreToolSource,
        [Parameter(Mandatory = $true)][string]$SourceManifestSha256,
        [Parameter(Mandatory = $true)][string]$PublishManifestSha256
    )
    if ($SnapshotKind -notin @('INITIAL_RECOVERY', 'STABLE_POST_STOP')) { throw "Unsupported external snapshot kind: $SnapshotKind" }
    New-EmptyDirectory $SnapshotDirectory
    $restoreToolRelativePath = 'restore-build.ps1'
    $restoreToolPath = Join-Path $SnapshotDirectory $restoreToolRelativePath
    Copy-Item -LiteralPath $RestoreToolSource -Destination $restoreToolPath -Force -ErrorAction Stop
    $boundStageReceiptRelativePath = 'bound-stage-receipt.json'
    $boundStageReceiptPath = Join-Path $SnapshotDirectory $boundStageReceiptRelativePath
    Copy-Item -LiteralPath $StageReceiptPath -Destination $boundStageReceiptPath -Force -ErrorAction Stop
    $paths = Get-OwnedLaunchPaths -DeploymentDirectory $DeploymentDirectory
    $appDataPath = Join-Path $env:LOCALAPPDATA 'MichStartupMaster'
    $appDataCopy = Join-Path $SnapshotDirectory 'app-data'
    $appDataExisted = Test-Path -LiteralPath $appDataPath -PathType Container
    if ($appDataExisted) {
        Assert-NoReparsePointTraversal -Path $appDataPath -TrustedRoot (Get-FullPath $env:LOCALAPPDATA) -Context 'MichStartupMaster AppData snapshot source'
        Assert-NoReparsePointsInTree -Directory $appDataPath -Context 'MichStartupMaster AppData snapshot source'
        Copy-DirectoryExact -Source $appDataPath -Destination $appDataCopy
    }
    else { New-Item -ItemType Directory -Path $appDataCopy -Force | Out-Null }
    $appDataManifest = Get-DirectoryManifest $appDataCopy

    $taskDirectory = Join-Path $SnapshotDirectory 'scheduled-tasks'
    New-Item -ItemType Directory -Path $taskDirectory -Force | Out-Null
    $tasks = @()
    $taskIndex = 0
    foreach ($task in @(Get-OwnedManagedTaskRows -AllowedExecutables $paths.Executables -AllowedLaunchers $paths.Launchers)) {
        $taskIndex++
        $relativeXml = 'scheduled-tasks\task-{0:D4}.xml' -f $taskIndex
        $xmlPath = Join-Path $SnapshotDirectory $relativeXml
        # Task Scheduler emits an XML declaration that identifies UTF-16. Preserve
        # that contract with a UTF-16LE BOM so schtasks /XML can consume the file
        # during manual recovery without an encoding/declaration mismatch.
        [IO.File]::WriteAllText($xmlPath, [string]$task.Xml, [Text.Encoding]::Unicode)
        $tasks += [pscustomobject][ordered]@{
            TaskPath = [string]$task.TaskPath
            XmlRelativePath = $relativeXml
            XmlSha256 = (Get-FileHash -LiteralPath $xmlPath -Algorithm SHA256).Hash
            XmlContentSha256 = Get-StringSha256 ([string]$task.Xml)
            SecurityDescriptor = [string]$task.SecurityDescriptor
            SecurityDescriptorSha256 = Get-StringSha256 ([string]$task.SecurityDescriptor)
        }
    }

    $shortcutDirectory = Join-Path $SnapshotDirectory 'startup-shortcuts'
    New-Item -ItemType Directory -Path $shortcutDirectory -Force | Out-Null
    $shortcuts = @()
    $shortcutIndex = 0
    foreach ($shortcut in @(Get-OwnedStartupShortcutRows -AllowedExecutables $paths.Executables -AllowedLaunchers $paths.Launchers)) {
        $shortcutIndex++
        $relativeCopy = 'startup-shortcuts\shortcut-{0:D4}.lnk' -f $shortcutIndex
        Copy-Item -LiteralPath $shortcut.OriginalPath -Destination (Join-Path $SnapshotDirectory $relativeCopy) -Force -ErrorAction Stop
        $shortcuts += [pscustomobject][ordered]@{
            OriginalPath = [string]$shortcut.OriginalPath
            CopyRelativePath = $relativeCopy
            Sha256 = (Get-FileHash -LiteralPath (Join-Path $SnapshotDirectory $relativeCopy) -Algorithm SHA256).Hash
            TargetPath = [string]$shortcut.TargetPath
            Arguments = [string]$shortcut.Arguments
        }
    }

    $manifestPath = Join-Path $SnapshotDirectory 'external-state.json'
    $restoreCommand = '"' + (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') + '" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "' + $restoreToolPath + '" -RestoreExternalSnapshot "' + $manifestPath + '"'
    [IO.File]::WriteAllLines((Join-Path $SnapshotDirectory 'RESTORE.txt'), @(
        'Automatic deployment rollback validates this transaction-bound snapshot before restoring it.',
        'Manual restoration is accepted only while the durable project marker still hash-binds this exact stable snapshot.',
        'Close every exact-path MichStartupMaster process, then run exactly:',
        $restoreCommand,
        'The restore preserves failed/current state under post-failure-* before replacing only validated MichStartupMaster-owned targets.'
    ), (New-Object Text.UTF8Encoding($false)))
    $registryRows = @(Get-OwnedRegistryLaunchRows -AllowedExecutables $paths.Executables -AllowedLaunchers $paths.Launchers)
    $payloadManifest = Get-DirectoryManifest $SnapshotDirectory
    $snapshot = [pscustomobject][ordered]@{
        SchemaVersion = 3
        Outcome = 'PRE_DEPLOY_EXTERNAL_STATE_CAPTURED'
        CapturedUtc = [DateTime]::UtcNow.ToString('o')
        TransactionId = $TransactionId
        ProjectRoot = Get-FullPath $ProjectRoot
        TransactionDirectory = Get-FullPath $TransactionDirectory
        SnapshotKind = $SnapshotKind
        SnapshotDirectory = Get-FullPath $SnapshotDirectory
        AppDataPath = Get-FullPath $appDataPath
        AppDataExisted = [bool]$appDataExisted
        AppDataCopyRelativePath = 'app-data'
        AppDataManifest = $appDataManifest
        AllowedExecutables = @($paths.Executables)
        AllowedLaunchers = @($paths.Launchers)
        ScheduledTasks = @($tasks)
        StartupShortcuts = @($shortcuts)
        StartupRegistryValues = @($registryRows)
        PayloadManifest = @($payloadManifest)
        PayloadManifestSha256 = Get-ManifestSha256 $payloadManifest
        RestoreToolRelativePath = $restoreToolRelativePath
        RestoreToolSha256 = (Get-FileHash -LiteralPath $restoreToolPath -Algorithm SHA256).Hash
        BoundStageReceiptRelativePath = $boundStageReceiptRelativePath
        BoundStageReceiptSha256 = (Get-FileHash -LiteralPath $boundStageReceiptPath -Algorithm SHA256).Hash
        SourceManifestSha256 = $SourceManifestSha256.ToUpperInvariant()
        PublishManifestSha256 = $PublishManifestSha256.ToUpperInvariant()
        AutomaticRestoreFunction = 'Restore-ExternalStateSnapshot'
    }
    Write-JsonFileAtomic -Path $manifestPath -Value $snapshot
    $manifestProvenance = Get-FileProvenance $manifestPath
    $bindingReceiptPath = Join-Path $SnapshotDirectory 'external-state.receipt.json'
    Write-JsonFileAtomic -Path $bindingReceiptPath -Value ([pscustomobject][ordered]@{
        SchemaVersion = 1
        Outcome = 'EXTERNAL_STATE_SNAPSHOT_HASH_BOUND'
        CreatedUtc = [DateTime]::UtcNow.ToString('o')
        TransactionId = $TransactionId
        ProjectRoot = Get-FullPath $ProjectRoot
        TransactionDirectory = Get-FullPath $TransactionDirectory
        SnapshotDirectory = Get-FullPath $SnapshotDirectory
        SnapshotKind = $SnapshotKind
        Manifest = $manifestProvenance
        PayloadManifestSha256 = [string]$snapshot.PayloadManifestSha256
        RestoreToolSha256 = [string]$snapshot.RestoreToolSha256
        BoundStageReceiptSha256 = [string]$snapshot.BoundStageReceiptSha256
        SourceManifestSha256 = [string]$snapshot.SourceManifestSha256
        PublishManifestSha256 = [string]$snapshot.PublishManifestSha256
    })
    $bindingReceiptProvenance = Get-FileProvenance $bindingReceiptPath
    $snapshot | Add-Member -NotePropertyName BindingReceiptPath -NotePropertyValue $bindingReceiptPath
    $snapshot | Add-Member -NotePropertyName BindingReceiptSha256 -NotePropertyValue $bindingReceiptProvenance.Sha256
    return $snapshot
}

function Assert-ExternalStateMatchesSnapshot {
    param([Parameter(Mandatory = $true)]$Snapshot)
    $appDataPath = Get-FullPath ([string]$Snapshot.AppDataPath)
    $appDataExists = Test-Path -LiteralPath $appDataPath -PathType Container
    if ($appDataExists -ne [bool]$Snapshot.AppDataExisted) {
        throw 'AppData existence changed while the stable external-state snapshot was captured.'
    }
    if ($appDataExists) {
        Assert-ManifestsEqual -Expected @($Snapshot.AppDataManifest) -Actual @(Get-DirectoryManifest $appDataPath) -Context 'Stable post-stop AppData'
    }

    $expectedTasks = @{}; foreach ($row in @($Snapshot.ScheduledTasks)) { $expectedTasks[[string]$row.TaskPath] = $row }
    $actualTasks = @{}; foreach ($row in @(Get-OwnedManagedTaskRows -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) { $actualTasks[[string]$row.TaskPath] = $row }
    if ($expectedTasks.Count -ne $actualTasks.Count) { throw 'Managed task namespace changed while the stable snapshot was captured.' }
    foreach ($path in @($expectedTasks.Keys)) {
        if (-not $actualTasks.ContainsKey($path)) { throw "Managed task disappeared during stable capture: $path" }
        if (-not [string]::Equals((Get-StringSha256 ([string]$actualTasks[$path].Xml)), [string]$expectedTasks[$path].XmlContentSha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Managed task XML changed during stable capture: $path"
        }
        if (-not [string]::Equals((Get-StringSha256 ([string]$actualTasks[$path].SecurityDescriptor)), [string]$expectedTasks[$path].SecurityDescriptorSha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Managed task security changed during stable capture: $path"
        }
    }

    $expectedShortcuts = @{}; foreach ($row in @($Snapshot.StartupShortcuts)) { $expectedShortcuts[[string]$row.OriginalPath] = $row }
    $actualShortcuts = @(Get-OwnedStartupShortcutRows -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))
    if ($expectedShortcuts.Count -ne $actualShortcuts.Count) { throw 'Owned Startup shortcuts changed while the stable snapshot was captured.' }
    foreach ($row in $actualShortcuts) {
        if (-not $expectedShortcuts.ContainsKey([string]$row.OriginalPath)) { throw "Unexpected owned Startup shortcut appeared during stable capture: $($row.OriginalPath)" }
        if (-not [string]::Equals((Get-FileHash -LiteralPath ([string]$row.OriginalPath) -Algorithm SHA256).Hash, [string]$expectedShortcuts[[string]$row.OriginalPath].Sha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Owned Startup shortcut changed during stable capture: $($row.OriginalPath)"
        }
    }

    $expectedRegistry = @{}; foreach ($row in @($Snapshot.StartupRegistryValues)) { $expectedRegistry[(Get-RegistryRowIdentity $row)] = $row }
    $actualRegistry = @{}; foreach ($row in @(Get-OwnedRegistryLaunchRows -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) { $actualRegistry[(Get-RegistryRowIdentity $row)] = $row }
    if ($expectedRegistry.Count -ne $actualRegistry.Count) { throw 'Owned Startup registry values changed while the stable snapshot was captured.' }
    foreach ($identity in @($expectedRegistry.Keys)) {
        if (-not $actualRegistry.ContainsKey($identity)) { throw "Owned Startup registry value disappeared during stable capture: $identity" }
        if ([string]$expectedRegistry[$identity].Kind -ne [string]$actualRegistry[$identity].Kind -or
            [string]$expectedRegistry[$identity].DataSha256 -ne [string]$actualRegistry[$identity].DataSha256) {
            throw "Owned Startup registry value changed during stable capture: $identity"
        }
    }
}

function Assert-ExactPathSet {
    param(
        [Parameter(Mandatory = $true)][object[]]$Actual,
        [Parameter(Mandatory = $true)][object[]]$Expected,
        [Parameter(Mandatory = $true)][string]$Context
    )
    $actualPaths = @($Actual | ForEach-Object { (Get-FullPath ([string]$_)).ToLowerInvariant() } | Sort-Object -Unique)
    $expectedPaths = @($Expected | ForEach-Object { (Get-FullPath ([string]$_)).ToLowerInvariant() } | Sort-Object -Unique)
    if ($actualPaths.Count -ne $expectedPaths.Count) { throw "$Context path-count mismatch." }
    for ($index = 0; $index -lt $expectedPaths.Count; $index++) {
        if (-not [string]::Equals($actualPaths[$index], $expectedPaths[$index], [StringComparison]::OrdinalIgnoreCase)) {
            throw "$Context contains an unauthorized path: $($actualPaths[$index])"
        }
    }
}

function Assert-ExternalSnapshotTargets {
    param(
        [Parameter(Mandatory = $true)]$Snapshot,
        [Parameter(Mandatory = $true)][string]$ManifestPath
    )
    if ([int]$Snapshot.SchemaVersion -ne 3 -or [string]$Snapshot.Outcome -ne 'PRE_DEPLOY_EXTERNAL_STATE_CAPTURED') {
        throw 'External-state snapshot schema/outcome is not supported.'
    }
    $projectRoot = Get-FullPath ([string]$Snapshot.ProjectRoot)
    if (-not [string]::Equals($projectRoot, (Get-FullPath $Root), [StringComparison]::OrdinalIgnoreCase)) {
        throw 'External-state snapshot project root does not match this recovery tool invocation.'
    }
    $transactionId = [string]$Snapshot.TransactionId
    if ($transactionId -notmatch '^\d{8}T\d{9}Z-[0-9a-fA-F]{8}$') { throw 'External-state snapshot transaction ID is malformed.' }
    $runtimeRoot = Get-FullPath (Join-Path $projectRoot 'artifacts\runtime-output')
    $expectedTransactionDirectory = Get-FullPath (Join-Path $runtimeRoot ('deploy-' + $transactionId))
    $transactionDirectory = Get-FullPath ([string]$Snapshot.TransactionDirectory)
    if (-not [string]::Equals($transactionDirectory, $expectedTransactionDirectory, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'External-state snapshot transaction directory is not canonical for this project.'
    }
    if ([string]$Snapshot.SnapshotKind -ne 'STABLE_POST_STOP') {
        throw 'Only the authoritative stable post-stop snapshot may be restored.'
    }
    $snapshotDirectory = Get-FullPath ([string]$Snapshot.SnapshotDirectory)
    $expectedSnapshotDirectory = Get-FullPath (Join-Path $transactionDirectory 'external-state-stable-post-stop')
    if (-not [string]::Equals($snapshotDirectory, $expectedSnapshotDirectory, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'External-state snapshot directory is not the canonical stable transaction directory.'
    }
    $expectedManifestPath = Get-FullPath (Join-Path $snapshotDirectory 'external-state.json')
    if (-not [string]::Equals((Get-FullPath $ManifestPath), $expectedManifestPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'External-state manifest path does not match the canonical stable snapshot.'
    }
    foreach ($pathToCheck in @($runtimeRoot, $transactionDirectory, $snapshotDirectory, $ManifestPath)) {
        Assert-NoReparsePointTraversal -Path $pathToCheck -TrustedRoot $projectRoot -Context 'External-state recovery path'
    }
    Assert-NoReparsePointsInTree -Directory $snapshotDirectory -Context 'External-state recovery payload'

    $expectedAppDataPath = Get-FullPath (Join-Path $env:LOCALAPPDATA 'MichStartupMaster')
    if (-not [string]::Equals((Get-FullPath ([string]$Snapshot.AppDataPath)), $expectedAppDataPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'External-state snapshot AppData target is not the exact MichStartupMaster directory.'
    }
    Assert-NoReparsePointTraversal -Path $expectedAppDataPath -TrustedRoot (Get-FullPath $env:LOCALAPPDATA) -Context 'External-state AppData target'
    Assert-NoReparsePointsInTree -Directory $expectedAppDataPath -Context 'External-state AppData target'
    if ([string]$Snapshot.AppDataCopyRelativePath -ne 'app-data') { throw 'External-state AppData payload path is not canonical.' }

    $expectedLaunchPaths = Get-OwnedLaunchPaths -DeploymentDirectory (Join-Path $projectRoot 'build')
    Assert-ExactPathSet -Actual @($Snapshot.AllowedExecutables) -Expected @($expectedLaunchPaths.Executables) -Context 'Allowed executable set'
    Assert-ExactPathSet -Actual @($Snapshot.AllowedLaunchers) -Expected @($expectedLaunchPaths.Launchers) -Context 'Allowed launcher set'

    $payloadRows = @($Snapshot.PayloadManifest)
    if ($payloadRows.Count -eq 0) { throw 'External-state rollback payload manifest is empty.' }
    if (-not [string]::Equals((Get-ManifestSha256 $payloadRows), [string]$Snapshot.PayloadManifestSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'External-state rollback payload manifest hash is invalid.'
    }
    $payloadIdentities = @{}
    foreach ($row in $payloadRows) {
        $relativePath = [string]$row.RelativePath
        $resolvedPayload = Get-CanonicalChildPath -Parent $snapshotDirectory -RelativePath $relativePath -Context 'Rollback payload path'
        $identity = $relativePath.Replace('/', '\').ToLowerInvariant()
        if ($payloadIdentities.ContainsKey($identity)) { throw "Duplicate rollback payload path: $relativePath" }
        $payloadIdentities[$identity] = $true
        if (-not (Test-Path -LiteralPath $resolvedPayload -PathType Leaf)) { throw "Rollback payload is missing: $resolvedPayload" }
        $file = Get-Item -LiteralPath $resolvedPayload
        if ([long]$file.Length -ne [long]$row.Length -or
            -not [string]::Equals((Get-FileHash -LiteralPath $resolvedPayload -Algorithm SHA256).Hash, [string]$row.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Rollback payload provenance mismatch: $relativePath"
        }
    }

    if ([string]$Snapshot.RestoreToolRelativePath -ne 'restore-build.ps1') { throw 'Restore-tool payload path is not canonical.' }
    $restoreToolPath = Get-CanonicalChildPath -Parent $snapshotDirectory -RelativePath ([string]$Snapshot.RestoreToolRelativePath) -Context 'Restore tool'
    if (-not [string]::Equals((Get-FileHash -LiteralPath $restoreToolPath -Algorithm SHA256).Hash, [string]$Snapshot.RestoreToolSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Restore-tool hash does not match the external-state manifest.'
    }
    $authorizedRecoveryToolPaths = @($restoreToolPath, (Join-Path $projectRoot 'scripts\build.ps1'))
    if (@($authorizedRecoveryToolPaths | Where-Object { Test-PathsEqual -LeftPath $_ -RightPath $ExecutingBuildScriptPath }).Count -ne 1 -or
        -not [string]::Equals((Get-FileHash -LiteralPath $ExecutingBuildScriptPath -Algorithm SHA256).Hash, [string]$Snapshot.RestoreToolSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The executing recovery tool is not the immutable hash-bound build script authorized by this snapshot.'
    }

    if ([string]$Snapshot.BoundStageReceiptRelativePath -ne 'bound-stage-receipt.json') { throw 'Bound stage-receipt payload path is not canonical.' }
    $boundStageReceiptPath = Get-CanonicalChildPath -Parent $snapshotDirectory -RelativePath ([string]$Snapshot.BoundStageReceiptRelativePath) -Context 'Bound stage receipt'
    if (-not [string]::Equals((Get-FileHash -LiteralPath $boundStageReceiptPath -Algorithm SHA256).Hash, [string]$Snapshot.BoundStageReceiptSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Bound stage-receipt hash does not match the external-state manifest.'
    }
    $boundStageReceipt = Get-Content -LiteralPath $boundStageReceiptPath -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
    if ([string]$boundStageReceipt.TransactionId -ne $transactionId -or
        -not (Test-PathsEqual -LeftPath ([string]$boundStageReceipt.SourceRoot) -RightPath $projectRoot) -or
        -not [string]::Equals((Get-ManifestSha256 @($boundStageReceipt.SourceManifest)), [string]$Snapshot.SourceManifestSha256, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals((Get-ManifestSha256 @($boundStageReceipt.PublishManifest)), [string]$Snapshot.PublishManifestSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Bound stage receipt does not match the transaction/source/publish hashes in the external-state manifest.'
    }

    $taskIdentities = @{}
    foreach ($task in @($Snapshot.ScheduledTasks)) {
        $taskPath = [string]$task.TaskPath
        if ($taskPath -notmatch '^\\MichStartupMaster(?:\\[^\\]+)+$' -or $taskPath.Contains('..')) { throw "Unauthorized scheduled-task target: $taskPath" }
        if ($taskIdentities.ContainsKey($taskPath.ToLowerInvariant())) { throw "Duplicate scheduled-task target: $taskPath" }
        $taskIdentities[$taskPath.ToLowerInvariant()] = $true
        $xmlRelativePath = [string]$task.XmlRelativePath
        if (-not $xmlRelativePath.StartsWith('scheduled-tasks\', [StringComparison]::OrdinalIgnoreCase)) { throw "Unauthorized task XML payload path: $xmlRelativePath" }
        $xmlPath = Get-CanonicalChildPath -Parent $snapshotDirectory -RelativePath $xmlRelativePath -Context 'Scheduled-task XML payload'
        if (-not [string]::Equals((Get-FileHash -LiteralPath $xmlPath -Algorithm SHA256).Hash, [string]$task.XmlSha256, [StringComparison]::OrdinalIgnoreCase)) { throw "Task XML payload hash mismatch: $taskPath" }
        $taskXml = [IO.File]::ReadAllText($xmlPath)
        if (-not [string]::Equals((Get-StringSha256 $taskXml), [string]$task.XmlContentSha256, [StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-OwnedAppTaskXml -Xml $taskXml -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) {
            throw "Task XML payload is not an exact owned manager launch: $taskPath"
        }
        if ([string]::IsNullOrWhiteSpace([string]$task.SecurityDescriptor) -or
            -not [string]::Equals((Get-StringSha256 ([string]$task.SecurityDescriptor)), [string]$task.SecurityDescriptorSha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Task security payload is invalid: $taskPath"
        }
    }

    $reservedShortcutPaths = @(
        (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Startup)) 'Mich Startup Master Agent.lnk'),
        (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::CommonStartup)) 'Mich Startup Master Agent.lnk')
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) }
    $shortcutIdentities = @{}
    foreach ($shortcut in @($Snapshot.StartupShortcuts)) {
        $shortcutPath = Get-FullPath ([string]$shortcut.OriginalPath)
        if (-not @($reservedShortcutPaths | Where-Object { Test-PathsEqual -LeftPath $_ -RightPath $shortcutPath }).Count) { throw "Unauthorized Startup shortcut target: $shortcutPath" }
        if ($shortcutIdentities.ContainsKey($shortcutPath.ToLowerInvariant())) { throw "Duplicate Startup shortcut target: $shortcutPath" }
        $shortcutIdentities[$shortcutPath.ToLowerInvariant()] = $true
        if (-not (Test-OwnedAgentLaunch -TargetPath ([string]$shortcut.TargetPath) -Arguments ([string]$shortcut.Arguments) -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) {
            throw "Startup shortcut ownership payload is invalid: $shortcutPath"
        }
        $copyRelativePath = [string]$shortcut.CopyRelativePath
        if (-not $copyRelativePath.StartsWith('startup-shortcuts\', [StringComparison]::OrdinalIgnoreCase)) { throw "Unauthorized shortcut payload path: $copyRelativePath" }
        $copyPath = Get-CanonicalChildPath -Parent $snapshotDirectory -RelativePath $copyRelativePath -Context 'Startup shortcut payload'
        if (-not [string]::Equals((Get-FileHash -LiteralPath $copyPath -Algorithm SHA256).Hash, [string]$shortcut.Sha256, [StringComparison]::OrdinalIgnoreCase)) { throw "Startup shortcut payload hash mismatch: $shortcutPath" }
    }

    $registryIdentities = @{}
    foreach ($row in @($Snapshot.StartupRegistryValues)) {
        if ([string]$row.Hive -notin @('CurrentUser', 'LocalMachine') -or
            [string]$row.View -notin @('Registry64', 'Registry32') -or
            [string]$row.SubKey -notin @('Software\Microsoft\Windows\CurrentVersion\Run', 'Software\Microsoft\Windows\CurrentVersion\RunOnce') -or
            [string]::IsNullOrWhiteSpace([string]$row.Name) -or ([string]$row.Name).Contains('\')) {
            throw "Unauthorized Startup registry target: $(Get-RegistryRowIdentity $row)"
        }
        $identity = Get-RegistryRowIdentity $row
        if ($registryIdentities.ContainsKey($identity)) { throw "Duplicate Startup registry target: $identity" }
        $registryIdentities[$identity] = $true
        $serialized = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String([string]$row.DataBase64))
        if (-not [string]::Equals((Get-StringSha256 $serialized), [string]$row.DataSha256, [StringComparison]::OrdinalIgnoreCase)) { throw "Startup registry payload hash mismatch: $identity" }
        $data = [Management.Automation.PSSerializer]::Deserialize($serialized)
        if (-not (Test-OwnedRegistryCommand -Command ([string]$data) -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) {
            throw "Startup registry payload is not owned by an allowed manager executable: $identity"
        }
    }
    return [pscustomobject][ordered]@{
        ProjectRoot = $projectRoot
        RuntimeRoot = $runtimeRoot
        TransactionDirectory = $transactionDirectory
        SnapshotDirectory = $snapshotDirectory
        ManifestPath = $expectedManifestPath
        BoundStageReceiptPath = $boundStageReceiptPath
    }
}

function Get-AuthorizedExternalSnapshot {
    param([Parameter(Mandatory = $true)][string]$ManifestPath)
    $manifestFullPath = Get-FullPath $ManifestPath
    if (-not (Test-Path -LiteralPath $manifestFullPath -PathType Leaf)) { throw "External-state snapshot was not found: $manifestFullPath" }
    $snapshot = Get-Content -LiteralPath $manifestFullPath -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
    $targetAuthorization = Assert-ExternalSnapshotTargets -Snapshot $snapshot -ManifestPath $manifestFullPath
    $bindingReceiptPath = Get-FullPath (Join-Path $targetAuthorization.SnapshotDirectory 'external-state.receipt.json')
    Assert-NoReparsePointTraversal -Path $bindingReceiptPath -TrustedRoot $targetAuthorization.ProjectRoot -Context 'External-state binding receipt'
    if (-not (Test-Path -LiteralPath $bindingReceiptPath -PathType Leaf)) { throw 'External-state binding receipt is missing.' }
    $bindingReceiptProvenance = Get-FileProvenance $bindingReceiptPath
    $bindingReceipt = Get-Content -LiteralPath $bindingReceiptPath -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
    if ([int]$bindingReceipt.SchemaVersion -ne 1 -or [string]$bindingReceipt.Outcome -ne 'EXTERNAL_STATE_SNAPSHOT_HASH_BOUND' -or
        [string]$bindingReceipt.TransactionId -ne [string]$snapshot.TransactionId -or
        -not (Test-PathsEqual -LeftPath ([string]$bindingReceipt.ProjectRoot) -RightPath $targetAuthorization.ProjectRoot) -or
        -not (Test-PathsEqual -LeftPath ([string]$bindingReceipt.TransactionDirectory) -RightPath $targetAuthorization.TransactionDirectory) -or
        -not (Test-PathsEqual -LeftPath ([string]$bindingReceipt.SnapshotDirectory) -RightPath $targetAuthorization.SnapshotDirectory) -or
        [string]$bindingReceipt.SnapshotKind -ne 'STABLE_POST_STOP') {
        throw 'External-state binding receipt identity is invalid.'
    }
    $manifestProvenance = Get-FileProvenance $manifestFullPath
    if (-not (Test-PathsEqual -LeftPath ([string]$bindingReceipt.Manifest.Path) -RightPath $manifestFullPath) -or
        [long]$bindingReceipt.Manifest.Length -ne [long]$manifestProvenance.Length -or
        -not [string]::Equals([string]$bindingReceipt.Manifest.Sha256, [string]$manifestProvenance.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'External-state manifest is not hash-bound by its immutable binding receipt.'
    }
    foreach ($propertyName in @('PayloadManifestSha256', 'RestoreToolSha256', 'BoundStageReceiptSha256', 'SourceManifestSha256', 'PublishManifestSha256')) {
        if (-not [string]::Equals([string]$bindingReceipt.$propertyName, [string]$snapshot.$propertyName, [StringComparison]::OrdinalIgnoreCase)) {
            throw "External-state binding receipt does not match manifest field $propertyName."
        }
    }

    $markerPath = Get-FullPath (Join-Path $targetAuthorization.RuntimeRoot 'deployment-active.json')
    Assert-NoReparsePointTraversal -Path $markerPath -TrustedRoot $targetAuthorization.ProjectRoot -Context 'Durable deployment marker'
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) { throw 'The durable deployment marker required to authorize restoration is missing.' }
    $marker = Get-Content -LiteralPath $markerPath -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
    if ([int]$marker.SchemaVersion -ne 2 -or
        [string]$marker.TransactionId -ne [string]$snapshot.TransactionId -or
        -not (Test-PathsEqual -LeftPath ([string]$marker.TransactionDirectory) -RightPath $targetAuthorization.TransactionDirectory) -or
        -not (Test-PathsEqual -LeftPath ([string]$marker.ExternalSnapshotReceipt) -RightPath $bindingReceiptPath) -or
        -not [string]::Equals([string]$marker.ExternalSnapshotReceiptSha256, [string]$bindingReceiptProvenance.Sha256, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([string]$marker.StageReceipt.Sha256, [string]$snapshot.BoundStageReceiptSha256, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([string]$marker.SourceManifestSha256, [string]$snapshot.SourceManifestSha256, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([string]$marker.PublishManifestSha256, [string]$snapshot.PublishManifestSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The durable deployment marker does not authorize this exact external-state receipt.'
    }
    $markerState = ([string]$marker.Outcome) + '|' + ([string]$marker.Phase)
    $restoreAuthorizedMarkerStates = @(
        'DEPLOYMENT_IN_PROGRESS|STABLE_POST_STOP_STATE_CAPTURED',
        'DEPLOYMENT_IN_PROGRESS|CANDIDATE_LAYOUT_MOVED',
        'EXTERNAL_RESTORE_IN_PROGRESS|AUTHORIZED_SNAPSHOT_RESTORE',
        'DEPLOYMENT_FAILED|ROLLBACK_INCOMPLETE',
        'EXTERNAL_STATE_RESTORE_INCOMPLETE|ROLLBACK_INCOMPLETE'
    )
    if ($restoreAuthorizedMarkerStates -notcontains $markerState) {
        throw "The durable deployment marker state does not authorize external-state restoration: $markerState"
    }
    if (-not [string]::IsNullOrWhiteSpace([string]$marker.DeploymentReceipt)) {
        $deploymentReceiptPath = Get-FullPath ([string]$marker.DeploymentReceipt)
        if (-not (Test-IsSameOrChildPath -Candidate $deploymentReceiptPath -Parent $targetAuthorization.TransactionDirectory) -or
            -not (Test-Path -LiteralPath $deploymentReceiptPath -PathType Leaf) -or
            -not [string]::Equals((Get-FileHash -LiteralPath $deploymentReceiptPath -Algorithm SHA256).Hash, [string]$marker.DeploymentReceiptSha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'The marker-bound deployment receipt is missing, outside the transaction, or hash-invalid.'
        }
    }
    return [pscustomobject][ordered]@{
        Snapshot = $snapshot
        TargetAuthorization = $targetAuthorization
        BindingReceipt = $bindingReceipt
        BindingReceiptPath = $bindingReceiptPath
        BindingReceiptSha256 = $bindingReceiptProvenance.Sha256
        Marker = $marker
        MarkerPath = $markerPath
    }
}

function Restore-ExternalStateSnapshot {
    param([Parameter(Mandatory = $true)]$Snapshot)
    $manifestPath = Join-Path (Get-FullPath ([string]$Snapshot.SnapshotDirectory)) 'external-state.json'
    $authorization = Get-AuthorizedExternalSnapshot -ManifestPath $manifestPath
    $Snapshot = $authorization.Snapshot
    $snapshotDirectory = $authorization.TargetAuthorization.SnapshotDirectory
    $payloadActual = @(
        foreach ($row in @($Snapshot.PayloadManifest)) {
            $path = Join-Path $snapshotDirectory ([string]$row.RelativePath)
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Rollback payload is missing: $path" }
            $file = Get-Item -LiteralPath $path
            [pscustomobject][ordered]@{ RelativePath = [string]$row.RelativePath; Length = [long]$file.Length; Sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
        }
    )
    Assert-ManifestsEqual -Expected @($Snapshot.PayloadManifest) -Actual $payloadActual -Context 'External rollback payload'

    $failureSuffix = (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssfffZ')
    $observedRoot = Join-Path $snapshotDirectory ('post-failure-' + $failureSuffix)
    New-Item -ItemType Directory -Path $observedRoot -Force | Out-Null
    $phaseResults = New-Object Collections.Generic.List[object]
    $restoreErrors = New-Object Collections.Generic.List[string]
    $schtasks = Join-Path $env:SystemRoot 'System32\schtasks.exe'

    # Scheduled tasks are one independently recoverable phase. Every item is
    # attempted even if a sibling item fails, and the phase is verified from a
    # fresh Task Scheduler COM enumeration before it may be called restored.
    $taskErrors = New-Object Collections.Generic.List[string]
    $currentTasks = @()
    $taskInspectionPassed = $false
    try {
        $currentTasks = @(Get-ManagedTaskNamespaceRows)
        $currentTaskDirectory = Join-Path $observedRoot 'scheduled-tasks'
        New-Item -ItemType Directory -Path $currentTaskDirectory -Force | Out-Null
        $currentIndex = 0
        foreach ($task in $currentTasks) {
            $currentIndex++
            [IO.File]::WriteAllText((Join-Path $currentTaskDirectory ('task-{0:D4}.xml' -f $currentIndex)), [string]$task.Xml, [Text.Encoding]::Unicode)
        }
        $taskInspectionPassed = $true
    } catch { $taskErrors.Add('capture current tasks: ' + $_.Exception.Message) }
    $capturedTaskPaths = @{}; foreach ($task in @($Snapshot.ScheduledTasks)) { $capturedTaskPaths[[string]$task.TaskPath] = $task }
    if ($taskInspectionPassed) {
        $currentTaskByPath = @{}; foreach ($task in $currentTasks) { $currentTaskByPath[[string]$task.TaskPath] = $task }
        foreach ($task in $currentTasks) {
            if ($capturedTaskPaths.ContainsKey([string]$task.TaskPath) -or
                -not (Test-OwnedAppTaskXml -Xml ([string]$task.Xml) -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) { continue }
            try {
                $deleteOutput = @(& $schtasks /Delete /TN ([string]$task.TaskPath) /F 2>&1)
                if ($LASTEXITCODE -ne 0) { throw ($deleteOutput -join [Environment]::NewLine) }
            } catch { $taskErrors.Add("remove candidate task '$($task.TaskPath)': $($_.Exception.Message)") }
        }
        foreach ($task in @($Snapshot.ScheduledTasks)) {
            try {
                if ($currentTaskByPath.ContainsKey([string]$task.TaskPath) -and
                    -not (Test-OwnedAppTaskXml -Xml ([string]$currentTaskByPath[[string]$task.TaskPath].Xml) -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) {
                    throw 'a non-owned task now occupies the exact captured path and was preserved'
                }
                $xmlPath = Join-Path $snapshotDirectory ([string]$task.XmlRelativePath)
                $createOutput = @(& $schtasks /Create /TN ([string]$task.TaskPath) /XML $xmlPath /F 2>&1)
                if ($LASTEXITCODE -ne 0) { throw ($createOutput -join [Environment]::NewLine) }
                if ($null -eq $task.PSObject.Properties['SecurityDescriptor'] -or [string]::IsNullOrWhiteSpace([string]$task.SecurityDescriptor)) {
                    throw 'captured task security descriptor is missing'
                }
                Set-ManagedTaskSecurityDescriptor -TaskPath ([string]$task.TaskPath) -SecurityDescriptor ([string]$task.SecurityDescriptor)
            } catch { $taskErrors.Add("restore task '$($task.TaskPath)': $($_.Exception.Message)") }
        }
    }
    try {
        $verifiedTasks = @{}; foreach ($task in @(Get-ManagedTaskNamespaceRows)) { $verifiedTasks[[string]$task.TaskPath] = $task }
        foreach ($task in @($Snapshot.ScheduledTasks)) {
            if (-not $verifiedTasks.ContainsKey([string]$task.TaskPath)) { throw "restored task is missing: $($task.TaskPath)" }
            $expectedContentHash = if ($null -ne $task.PSObject.Properties['XmlContentSha256']) {
                [string]$task.XmlContentSha256
            } else {
                $capturedXmlPath = Join-Path $snapshotDirectory ([string]$task.XmlRelativePath)
                Get-StringSha256 ([IO.File]::ReadAllText($capturedXmlPath))
            }
            $actualContentHash = Get-StringSha256 ([string]$verifiedTasks[[string]$task.TaskPath].Xml)
            if (-not [string]::Equals($expectedContentHash, $actualContentHash, [StringComparison]::OrdinalIgnoreCase)) { throw "restored task XML differs: $($task.TaskPath)" }
            if ($null -ne $task.PSObject.Properties['SecurityDescriptorSha256'] -and
                -not (Test-SecurityDescriptorSemanticallyEqual -Expected ([string]$task.SecurityDescriptor) -Actual ([string]$verifiedTasks[[string]$task.TaskPath].SecurityDescriptor))) {
                throw "restored task security differs: $($task.TaskPath)"
            }
        }
        foreach ($task in @($verifiedTasks.Values)) {
            if (-not $capturedTaskPaths.ContainsKey([string]$task.TaskPath) -and
                (Test-OwnedAppTaskXml -Xml ([string]$task.Xml) -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) {
                throw "candidate-created owned task remains: $($task.TaskPath)"
            }
        }
    } catch { $taskErrors.Add('verify scheduled tasks: ' + $_.Exception.Message) }
    $taskPassed = $taskErrors.Count -eq 0
    $phaseResults.Add([pscustomobject][ordered]@{ Phase = 'ScheduledTasks'; Passed = $taskPassed; Errors = $taskErrors.ToArray() })
    if (-not $taskPassed) { $restoreErrors.Add('ScheduledTasks: ' + ($taskErrors.ToArray() -join ' | ')) }

    # Reserved/owned Startup shortcuts are restored byte-for-byte independently.
    $shortcutErrors = New-Object Collections.Generic.List[string]
    $capturedShortcutPaths = @{}; foreach ($shortcut in @($Snapshot.StartupShortcuts)) { $capturedShortcutPaths[[string]$shortcut.OriginalPath] = $shortcut }
    try {
        $currentShortcuts = @(Get-OwnedStartupShortcutRows -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))
        $observedShortcutDirectory = Join-Path $observedRoot 'startup-shortcuts'
        New-Item -ItemType Directory -Path $observedShortcutDirectory -Force | Out-Null
        $shortcutIndex = 0
        foreach ($shortcut in $currentShortcuts) {
            $shortcutIndex++
            Copy-Item -LiteralPath $shortcut.OriginalPath -Destination (Join-Path $observedShortcutDirectory ('shortcut-{0:D4}.lnk' -f $shortcutIndex)) -Force -ErrorAction Stop
            if (-not $capturedShortcutPaths.ContainsKey([string]$shortcut.OriginalPath)) { Remove-Item -LiteralPath $shortcut.OriginalPath -Force -ErrorAction Stop }
        }
    } catch { $shortcutErrors.Add('preserve/remove current shortcuts: ' + $_.Exception.Message) }
    foreach ($shortcut in @($Snapshot.StartupShortcuts)) {
        try {
            $destination = [string]$shortcut.OriginalPath
            if ((Test-Path -LiteralPath $destination -PathType Leaf) -and
                -not (Test-OwnedStartupShortcutFile -Path $destination -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) {
                throw 'a non-owned shortcut now occupies the exact captured path and was preserved'
            }
            $destinationDirectory = Split-Path -Path $destination -Parent
            if (-not (Test-Path -LiteralPath $destinationDirectory -PathType Container)) { New-Item -ItemType Directory -Path $destinationDirectory -Force | Out-Null }
            Copy-Item -LiteralPath (Join-Path $snapshotDirectory ([string]$shortcut.CopyRelativePath)) -Destination $destination -Force -ErrorAction Stop
            $restoredHash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
            if (-not [string]::Equals($restoredHash, [string]$shortcut.Sha256, [StringComparison]::OrdinalIgnoreCase)) { throw 'hash mismatch' }
        } catch { $shortcutErrors.Add("restore shortcut '$($shortcut.OriginalPath)': $($_.Exception.Message)") }
    }
    try {
        $verifiedShortcuts = @(Get-OwnedStartupShortcutRows -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))
        foreach ($shortcut in $verifiedShortcuts) {
            if (-not $capturedShortcutPaths.ContainsKey([string]$shortcut.OriginalPath)) { throw "unexpected owned shortcut remains: $($shortcut.OriginalPath)" }
        }
        foreach ($shortcut in @($Snapshot.StartupShortcuts)) {
            if (-not (Test-Path -LiteralPath ([string]$shortcut.OriginalPath) -PathType Leaf)) { throw "restored shortcut is missing: $($shortcut.OriginalPath)" }
            if (-not [string]::Equals((Get-FileHash -LiteralPath ([string]$shortcut.OriginalPath) -Algorithm SHA256).Hash, [string]$shortcut.Sha256, [StringComparison]::OrdinalIgnoreCase)) { throw "restored shortcut differs: $($shortcut.OriginalPath)" }
        }
    } catch { $shortcutErrors.Add('verify Startup shortcuts: ' + $_.Exception.Message) }
    $shortcutPassed = $shortcutErrors.Count -eq 0
    $phaseResults.Add([pscustomobject][ordered]@{ Phase = 'StartupShortcuts'; Passed = $shortcutPassed; Errors = $shortcutErrors.ToArray() })
    if (-not $shortcutPassed) { $restoreErrors.Add('StartupShortcuts: ' + ($shortcutErrors.ToArray() -join ' | ')) }

    # Startup registry values are serialized with their exact type/data and restored
    # without preventing the later AppData phase when one value is inaccessible.
    $registryErrors = New-Object Collections.Generic.List[string]
    $capturedRegistry = @{}; foreach ($row in @($Snapshot.StartupRegistryValues)) { $capturedRegistry[(Get-RegistryRowIdentity $row)] = $row }
    try {
        $currentRegistry = @(Get-OwnedRegistryLaunchRows -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))
        Write-JsonFile -Path (Join-Path $observedRoot 'startup-registry.json') -Value $currentRegistry
        foreach ($row in $currentRegistry) {
            if ($capturedRegistry.ContainsKey((Get-RegistryRowIdentity $row))) { continue }
            try {
                $hive = [Enum]::Parse([Microsoft.Win32.RegistryHive], [string]$row.Hive)
                $view = [Enum]::Parse([Microsoft.Win32.RegistryView], [string]$row.View)
                $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey($hive, $view)
                try {
                    $key = $baseKey.OpenSubKey([string]$row.SubKey, $true)
                    if ($null -ne $key) { try { $key.DeleteValue([string]$row.Name, $false) } finally { $key.Dispose() } }
                } finally { $baseKey.Dispose() }
            } catch { $registryErrors.Add("remove registry value '$(Get-RegistryRowIdentity $row)': $($_.Exception.Message)") }
        }
    } catch { $registryErrors.Add('capture current registry values: ' + $_.Exception.Message) }
    foreach ($row in @($Snapshot.StartupRegistryValues)) {
        try {
            $serialized = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String([string]$row.DataBase64))
            if (-not [string]::Equals((Get-StringSha256 $serialized), [string]$row.DataSha256, [StringComparison]::OrdinalIgnoreCase)) { throw 'payload hash mismatch' }
            $data = [Management.Automation.PSSerializer]::Deserialize($serialized)
            $hive = [Enum]::Parse([Microsoft.Win32.RegistryHive], [string]$row.Hive)
            $view = [Enum]::Parse([Microsoft.Win32.RegistryView], [string]$row.View)
            $kind = [Enum]::Parse([Microsoft.Win32.RegistryValueKind], [string]$row.Kind)
            $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey($hive, $view)
            try {
                $key = $baseKey.CreateSubKey([string]$row.SubKey, $true)
                try {
                    $currentData = $key.GetValue([string]$row.Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                    if ($null -ne $currentData -and
                        -not (Test-OwnedRegistryCommand -Command ([string]$currentData) -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) {
                        throw 'a non-owned value now occupies the exact captured registry identity and was preserved'
                    }
                    $key.SetValue([string]$row.Name, $data, $kind)
                } finally { $key.Dispose() }
            } finally { $baseKey.Dispose() }
        } catch { $registryErrors.Add("restore registry value '$(Get-RegistryRowIdentity $row)': $($_.Exception.Message)") }
    }
    try {
        $verifiedRegistry = @{}; foreach ($row in @(Get-OwnedRegistryLaunchRows -AllowedExecutables @($Snapshot.AllowedExecutables) -AllowedLaunchers @($Snapshot.AllowedLaunchers))) { $verifiedRegistry[(Get-RegistryRowIdentity $row)] = $row }
        foreach ($identity in @($capturedRegistry.Keys)) {
            if (-not $verifiedRegistry.ContainsKey($identity)) { throw "restored registry value is missing: $identity" }
            $expectedRow = $capturedRegistry[$identity]; $actualRow = $verifiedRegistry[$identity]
            if ([string]$expectedRow.Kind -ne [string]$actualRow.Kind -or [string]$expectedRow.DataSha256 -ne [string]$actualRow.DataSha256) { throw "restored registry value differs: $identity" }
        }
        foreach ($identity in @($verifiedRegistry.Keys)) { if (-not $capturedRegistry.ContainsKey($identity)) { throw "unexpected owned registry value remains: $identity" } }
    } catch { $registryErrors.Add('verify startup registry: ' + $_.Exception.Message) }
    $registryPassed = $registryErrors.Count -eq 0
    $phaseResults.Add([pscustomobject][ordered]@{ Phase = 'StartupRegistry'; Passed = $registryPassed; Errors = $registryErrors.ToArray() })
    if (-not $registryPassed) { $restoreErrors.Add('StartupRegistry: ' + ($registryErrors.ToArray() -join ' | ')) }

    # AppData is deliberately last and independent so task/shortcut/registry
    # failures cannot suppress recovery of the user's durable intent stores.
    $appDataErrors = New-Object Collections.Generic.List[string]
    $appDataPath = Get-FullPath ([string]$Snapshot.AppDataPath)
    try {
        if (Test-Path -LiteralPath $appDataPath -PathType Container) {
            Move-Item -LiteralPath $appDataPath -Destination (Join-Path $observedRoot 'app-data') -ErrorAction Stop
        }
        if ([bool]$Snapshot.AppDataExisted) {
            Copy-DirectoryExact -Source (Join-Path $snapshotDirectory ([string]$Snapshot.AppDataCopyRelativePath)) -Destination $appDataPath
            Assert-ManifestsEqual -Expected @($Snapshot.AppDataManifest) -Actual @(Get-DirectoryManifest $appDataPath) -Context 'Restored MichStartupMaster AppData'
        } elseif (Test-Path -LiteralPath $appDataPath) {
            throw "AppData should be absent after restoration but exists: $appDataPath"
        }
    } catch { $appDataErrors.Add($_.Exception.Message) }
    $appDataPassed = $appDataErrors.Count -eq 0
    $phaseResults.Add([pscustomobject][ordered]@{ Phase = 'AppData'; Passed = $appDataPassed; Errors = $appDataErrors.ToArray() })
    if (-not $appDataPassed) { $restoreErrors.Add('AppData: ' + ($appDataErrors.ToArray() -join ' | ')) }

    $result = [pscustomobject][ordered]@{
        RestoredUtc = [DateTime]::UtcNow.ToString('o')
        SnapshotDirectory = $snapshotDirectory
        PreservedFailedState = $observedRoot
        Verified = $restoreErrors.Count -eq 0
        Phases = $phaseResults.ToArray()
        Errors = $restoreErrors.ToArray()
        ScheduledTaskCount = @($Snapshot.ScheduledTasks).Count
        StartupShortcutCount = @($Snapshot.StartupShortcuts).Count
        StartupRegistryValueCount = @($Snapshot.StartupRegistryValues).Count
        AppDataFileCount = @($Snapshot.AppDataManifest).Count
    }
    Write-JsonFile -Path (Join-Path $observedRoot 'restore-result.json') -Value $result
    if ($restoreErrors.Count -gt 0) { throw ('External-state restoration was incomplete: ' + ($restoreErrors.ToArray() -join ' || ')) }
    return $result
}

function Test-DotnetSdk {
    param([Parameter(Mandatory = $true)][string]$DotnetPath)
    if (-not (Test-Path -LiteralPath $DotnetPath -PathType Leaf)) {
        return $false
    }
    $savedDotnetRoot = $env:DOTNET_ROOT
    $savedMultilevelLookup = $env:DOTNET_MULTILEVEL_LOOKUP
    $savedTelemetryOptOut = $env:DOTNET_CLI_TELEMETRY_OPTOUT
    $savedFirstTimeExperience = $env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE
    try {
        $env:DOTNET_ROOT = Split-Path -Path $DotnetPath -Parent
        $env:DOTNET_MULTILEVEL_LOOKUP = '0'
        $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
        $env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'
        $probeOutput = @(& $DotnetPath --version 2>&1)
        $probeExitCode = $LASTEXITCODE
        return $probeExitCode -eq 0 -and
            $probeOutput.Count -gt 0 -and
            [string]::Equals(([string]$probeOutput[0]).Trim(), $PinnedSdkVersion, [StringComparison]::Ordinal)
    } catch {
        return $false
    } finally {
        $env:DOTNET_ROOT = $savedDotnetRoot
        $env:DOTNET_MULTILEVEL_LOOKUP = $savedMultilevelLookup
        $env:DOTNET_CLI_TELEMETRY_OPTOUT = $savedTelemetryOptOut
        $env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = $savedFirstTimeExperience
    }
}

function Resolve-WorkingDotnetSdk {
    param(
        [Parameter(Mandatory = $true)][string[]]$Candidates,
        [Parameter(Mandatory = $true)][string]$StageRoot
    )
    foreach ($candidate in $Candidates) {
        if (-not [string]::IsNullOrWhiteSpace($candidate) -and (Test-DotnetSdk $candidate)) {
            return [pscustomobject][ordered]@{
                Path = Get-FullPath $candidate
                Bootstrapped = $false
                CachePath = $null
            }
        }
    }

    $installer = Join-Path $Root 'dotnet-install.ps1'
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
        throw 'No healthy .NET SDK was found and the isolated SDK bootstrap script is missing.'
    }

    $cacheDirectory = Join-Path $Root '.dotnet'
    $cachedDotnet = Join-Path $cacheDirectory 'dotnet.exe'
    $bootstrapDirectory = Join-Path $StageRoot 'dotnet-bootstrap'
    $bootstrapMutex = New-Object System.Threading.Mutex($false, 'Local\MichStartupMaster.DotnetSdkBootstrap')
    $mutexHeld = $false
    try {
        Write-BuildState 'No healthy pinned SDK candidate was found; waiting for the isolated SDK bootstrap gate.'
        try {
            $mutexHeld = $bootstrapMutex.WaitOne([TimeSpan]::FromMinutes(10))
        } catch [System.Threading.AbandonedMutexException] {
            $mutexHeld = $true
        }
        if (-not $mutexHeld) {
            throw 'Timed out waiting for the isolated .NET SDK bootstrap lock.'
        }

        # Another parallel stage may have populated the cache while this process waited.
        if (Test-DotnetSdk $cachedDotnet) {
            return [pscustomobject][ordered]@{
                Path = Get-FullPath $cachedDotnet
                Bootstrapped = $false
                CachePath = Get-FullPath $cacheDirectory
            }
        }

        if (Test-Path -LiteralPath $bootstrapDirectory) {
            throw "Refusing to overwrite an existing SDK bootstrap directory: $bootstrapDirectory"
        }
        New-Item -ItemType Directory -Path $bootstrapDirectory -Force | Out-Null
        $windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
        # Capture the installer's success-stream output so this function returns only the
        # structured SDK-resolution object.  Letting progress text escape here turns the
        # caller's result into an Object[] and makes `$dotnetResolution.Path` fail on the
        # first clean-machine build.
        $bootstrapOutput = @(
            & $windowsPowerShell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $installer -Version $PinnedSdkVersion -Architecture 'x64' -InstallDir $bootstrapDirectory -NoPath 2>&1
        )
        $bootstrapExitCode = $LASTEXITCODE
        if ($bootstrapExitCode -ne 0) {
            $bootstrapDetail = ($bootstrapOutput | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
            throw "Isolated .NET SDK bootstrap failed with exit $bootstrapExitCode. Partial files were preserved at: $bootstrapDirectory`n$bootstrapDetail"
        }
        $bootstrappedDotnet = Join-Path $bootstrapDirectory 'dotnet.exe'
        if (-not (Test-DotnetSdk $bootstrappedDotnet)) {
            throw "The isolated .NET SDK bootstrap completed but failed its runtime probe: $bootstrappedDotnet"
        }

        if (-not (Test-Path -LiteralPath $cacheDirectory)) {
            Move-Item -LiteralPath $bootstrapDirectory -Destination $cacheDirectory -ErrorAction Stop
            $selectedDotnet = $cachedDotnet
            $selectedCache = $cacheDirectory
        } else {
            # Preserve an unhealthy pre-existing cache rather than overwriting it.
            $selectedDotnet = $bootstrappedDotnet
            $selectedCache = $bootstrapDirectory
        }
        return [pscustomobject][ordered]@{
            Path = Get-FullPath $selectedDotnet
            Bootstrapped = $true
            CachePath = Get-FullPath $selectedCache
        }
    } finally {
        if ($mutexHeld) {
            $bootstrapMutex.ReleaseMutex()
        }
        $bootstrapMutex.Dispose()
    }
}

function Get-SdkProvenance {
    param([Parameter(Mandatory = $true)][string]$DotnetPath)
    $sdkRoot = Split-Path -Path $DotnetPath -Parent
    $sdkDirectory = Join-Path $sdkRoot ('sdk\' + $PinnedSdkVersion)
    if (-not (Test-Path -LiteralPath $sdkDirectory -PathType Container)) {
        throw "Pinned SDK directory was not found: $sdkDirectory"
    }
    $versionOutput = @(& $DotnetPath --version 2>&1)
    if ($LASTEXITCODE -ne 0 -or $versionOutput.Count -eq 0 -or ([string]$versionOutput[0]).Trim() -ne $PinnedSdkVersion) {
        throw "Resolved SDK is not the pinned version ${PinnedSdkVersion}: $DotnetPath"
    }
    $sdkManifest = Get-DirectoryManifest $sdkDirectory
    return [pscustomobject][ordered]@{
        Version = $PinnedSdkVersion
        Dotnet = Get-FileProvenance $DotnetPath
        SdkDirectory = Get-FullPath $sdkDirectory
        SdkFileCount = $sdkManifest.Count
        SdkTotalBytes = [long](($sdkManifest | Measure-Object -Property Length -Sum).Sum)
        SdkManifestSha256 = Get-ManifestSha256 $sdkManifest
    }
}

if (-not [string]::IsNullOrWhiteSpace($RestoreExternalSnapshot)) {
    if ($StageOnly -or $CompileInstaller -or -not [string]::IsNullOrWhiteSpace($StageDirectory)) {
        throw '-RestoreExternalSnapshot cannot be combined with staging, installer, or deployment options.'
    }
    $restoreManifestPath = Get-FullPath $RestoreExternalSnapshot
    $authorization = Get-AuthorizedExternalSnapshot -ManifestPath $restoreManifestPath
    $restoreDeploymentMutex = New-Object Threading.Mutex($false, $DeploymentMutexName)
    $restoreDeploymentMutexHeld = $false
    $restoreMutationMutex = $null
    $restoreMutationMutexHeld = $false
    try {
        try { $restoreDeploymentMutexHeld = $restoreDeploymentMutex.WaitOne([TimeSpan]::FromMinutes(10)) }
        catch [Threading.AbandonedMutexException] { $restoreDeploymentMutexHeld = $true }
        if (-not $restoreDeploymentMutexHeld) { throw 'Timed out waiting for the MichStartupMaster deployment lock before external-state restoration.' }
        # Revalidate every byte/path binding after taking the deployment lock so a
        # caller cannot swap a validated receipt or durable marker before mutation.
        $authorization = Get-AuthorizedExternalSnapshot -ManifestPath $restoreManifestPath
        $snapshot = $authorization.Snapshot
    $restoreMutationMutex = New-Object Threading.Mutex($false, $ManagedStartupMutationMutexName)
        try { $restoreMutationMutexHeld = $restoreMutationMutex.WaitOne([TimeSpan]::FromMinutes(2)) }
        catch [Threading.AbandonedMutexException] { $restoreMutationMutexHeld = $true }
        if (-not $restoreMutationMutexHeld) { throw 'Timed out waiting for the MichStartupMaster external-state mutation lock.' }
        $liveManagerProcesses = @(Get-AllManagerProcessState -ExecutablePaths @($snapshot.AllowedExecutables))
        if ($liveManagerProcesses.Count -ne 0) {
            throw "Close all exact workspace/installed MichStartupMaster generations before restoration; found $($liveManagerProcesses.Count)."
        }
        $boundStageReceipt = $authorization.TargetAuthorization.BoundStageReceiptPath
        Write-DeploymentMarker -Path $authorization.MarkerPath -Outcome 'EXTERNAL_RESTORE_IN_PROGRESS' -Phase 'AUTHORIZED_SNAPSHOT_RESTORE' `
            -TransactionDirectory $authorization.TargetAuthorization.TransactionDirectory -StageReceipt $boundStageReceipt `
            -MarkerTransactionId ([string]$snapshot.TransactionId) -SourceManifestSha256 ([string]$snapshot.SourceManifestSha256) `
            -PublishManifestSha256 ([string]$snapshot.PublishManifestSha256) -ExternalSnapshotReceipt $authorization.BindingReceiptPath `
            -ExternalSnapshotReceiptSha256 $authorization.BindingReceiptSha256
        try {
            $restoreResult = Restore-ExternalStateSnapshot -Snapshot $snapshot
            $manualReceiptPath = Join-Path $authorization.TargetAuthorization.TransactionDirectory 'manual-external-restore-receipt.json'
            Write-JsonFileAtomic -Path $manualReceiptPath -Value ([pscustomobject][ordered]@{
                SchemaVersion = 1
                Outcome = 'EXTERNAL_STATE_RESTORED_AND_VERIFIED'
                TransactionId = [string]$snapshot.TransactionId
                CompletedUtc = [DateTime]::UtcNow.ToString('o')
                ExternalSnapshotReceipt = Get-FileProvenance $authorization.BindingReceiptPath
                Result = $restoreResult
            })
            Write-DeploymentMarker -Path $authorization.MarkerPath -Outcome 'EXTERNAL_STATE_RESTORED_AND_VERIFIED' -Phase 'MANUAL_RESTORE_VERIFIED' `
                -TransactionDirectory $authorization.TargetAuthorization.TransactionDirectory -StageReceipt $boundStageReceipt -DeploymentReceipt $manualReceiptPath `
                -MarkerTransactionId ([string]$snapshot.TransactionId) -SourceManifestSha256 ([string]$snapshot.SourceManifestSha256) `
                -PublishManifestSha256 ([string]$snapshot.PublishManifestSha256) -ExternalSnapshotReceipt $authorization.BindingReceiptPath `
                -ExternalSnapshotReceiptSha256 $authorization.BindingReceiptSha256
            $restoreResult | Format-List
        } catch {
            $restoreFailure = $_
            $failureReceiptPath = Join-Path $authorization.TargetAuthorization.TransactionDirectory 'manual-external-restore-failure.json'
            try {
                Write-JsonFileAtomic -Path $failureReceiptPath -Value ([pscustomobject][ordered]@{
                    SchemaVersion = 1
                    Outcome = 'EXTERNAL_STATE_RESTORE_INCOMPLETE'
                    TransactionId = [string]$snapshot.TransactionId
                    FailedUtc = [DateTime]::UtcNow.ToString('o')
                    Error = $restoreFailure.Exception.ToString()
                    ExternalSnapshotReceipt = Get-FileProvenance $authorization.BindingReceiptPath
                })
                Write-DeploymentMarker -Path $authorization.MarkerPath -Outcome 'EXTERNAL_STATE_RESTORE_INCOMPLETE' -Phase 'ROLLBACK_INCOMPLETE' `
                    -TransactionDirectory $authorization.TargetAuthorization.TransactionDirectory -StageReceipt $boundStageReceipt -DeploymentReceipt $failureReceiptPath `
                    -MarkerTransactionId ([string]$snapshot.TransactionId) -SourceManifestSha256 ([string]$snapshot.SourceManifestSha256) `
                    -PublishManifestSha256 ([string]$snapshot.PublishManifestSha256) -ExternalSnapshotReceipt $authorization.BindingReceiptPath `
                    -ExternalSnapshotReceiptSha256 $authorization.BindingReceiptSha256
            } catch { throw "External-state restoration failed and its durable failure marker could not be written: $($restoreFailure.Exception.Message) || $($_.Exception.Message)" }
            throw $restoreFailure
        }
    } finally {
        if ($restoreMutationMutexHeld) { $restoreMutationMutex.ReleaseMutex() }
        if ($null -ne $restoreMutationMutex) { $restoreMutationMutex.Dispose() }
        if ($restoreDeploymentMutexHeld) { $restoreDeploymentMutex.ReleaseMutex() }
        $restoreDeploymentMutex.Dispose()
    }
    return
}

if ([string]::IsNullOrWhiteSpace($StageDirectory)) {
    $StageDirectory = Join-Path $GeneratedStageRoot $TransactionId
} elseif (-not [System.IO.Path]::IsPathRooted($StageDirectory)) {
    $StageDirectory = Join-Path $Root $StageDirectory
}
$StageDirectory = Get-FullPath $StageDirectory

if ((Test-IsSameOrChildPath -Candidate $StageDirectory -Parent $DeployDirectory) -or
    (Test-IsSameOrChildPath -Candidate $DeployDirectory -Parent $StageDirectory)) {
    throw "Staging and deployment directories must be separate. Stage='$StageDirectory'; Deploy='$DeployDirectory'"
}
$protectedLiveDirectories = @(
    (Join-Path $env:LOCALAPPDATA 'MichStartupMaster'),
    (Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster'),
    [Environment]::GetFolderPath([Environment+SpecialFolder]::Startup),
    [Environment]::GetFolderPath([Environment+SpecialFolder]::CommonStartup)
) | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Select-Object -Unique
foreach ($protectedLiveDirectory in $protectedLiveDirectories) {
    if ((Test-IsSameOrChildPath -Candidate $StageDirectory -Parent $protectedLiveDirectory) -or
        (Test-IsSameOrChildPath -Candidate $protectedLiveDirectory -Parent $StageDirectory)) {
        throw "Staging is forbidden in or above a live application/startup location. Stage='$StageDirectory'; Protected='$protectedLiveDirectory'"
    }
}

New-EmptyDirectory $StageDirectory
Write-BuildState ("Stage transaction {0}: capturing a consistent source snapshot." -f $TransactionId)
$PublishDirectory = Join-Path $StageDirectory 'publish'
$StageReceiptPath = Join-Path $StageDirectory 'stage-receipt.json'
$DependencyLockPath = $null
$InstallerMetadataPath = Join-Path $StageDirectory 'validated-stage.issinc'
$ValidatedPublishManifestPath = Join-Path $StageDirectory 'validated-publish.manifest'
$InstallProvenancePath = Join-Path $StageDirectory 'install-provenance.json'
$InstallerOutputDirectory = Join-Path $StageDirectory 'installer'

$sourceSnapshot = New-ConsistentSourceSnapshot -SourceRoot $Root -StageRoot $StageDirectory
Write-BuildState ("Source snapshot captured on attempt {0}; validating the pinned .NET SDK." -f $sourceSnapshot.Attempts)
$SnapshotDirectory = $sourceSnapshot.Directory
$sourceManifest = $sourceSnapshot.Manifest
$DependencyLockPath = Join-Path $SnapshotDirectory 'packages.lock.json'
$earlyBuildScriptProvenance = Get-FileProvenance (Join-Path $Root 'scripts\build.ps1')
$earlyBootstrapScriptProvenance = Get-FileProvenance (Join-Path $Root 'dotnet-install.ps1')
$earlyInstallerScriptProvenance = Get-FileProvenance (Join-Path $Root 'installer\MichStartupMaster.iss')

$dotnetCommand = Get-Command dotnet -ErrorAction SilentlyContinue
$dotnetFromPath = if ($null -eq $dotnetCommand) { $null } else { $dotnetCommand.Source }
# An explicitly supplied SDK is allowed only as a first candidate, never trusted blindly:
# Resolve-WorkingDotnetSdk still requires the exact pinned version and records its complete
# provenance. This keeps a temporarily unhealthy workspace cache from blocking a safe release
# while preserving the same reproducibility gate.
$dotnetOverride = $env:MICH_STARTUP_MASTER_DOTNET
$dotnetCandidates = @(
    $dotnetOverride,
    (Join-Path $Root '.dotnet\dotnet.exe'),
    'C:\Users\micha\.codex\tools\dotnet-sdk-10.0.301\dotnet.exe',
    'C:\DotnetRepair\dotnet.exe',
    'C:\Program Files\dotnet\dotnet.exe',
    $dotnetFromPath
)
$dotnetCandidates = @($dotnetCandidates | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) })
Assert-FileProvenance -Expected $earlyBootstrapScriptProvenance -Context 'Pre-SDK-resolution bootstrap script'
$dotnetResolution = Resolve-WorkingDotnetSdk -Candidates $dotnetCandidates -StageRoot $StageDirectory
Assert-FileProvenance -Expected $earlyBootstrapScriptProvenance -Context 'Post-SDK-resolution bootstrap script'
$dotnet = $dotnetResolution.Path
Write-BuildState ("Pinned SDK selected at '{0}'. Capturing its reproducibility manifest." -f $dotnet)
$sdkProvenance = Get-SdkProvenance $dotnet
Write-BuildState 'SDK provenance captured; restoring the locked dependency graph.'
$buildInputs = [pscustomobject][ordered]@{
    BuildScript = $earlyBuildScriptProvenance
    BootstrapScript = $earlyBootstrapScriptProvenance
    Project = Get-FileProvenance (Join-Path $Root $ProjectName)
    SnapshotProject = Get-FileProvenance (Join-Path $SnapshotDirectory $ProjectName)
    SdkSelection = Get-FileProvenance (Join-Path $Root 'global.json')
    DependencyLock = Get-FileProvenance (Join-Path $Root 'packages.lock.json')
    InstallerScript = $earlyInstallerScriptProvenance
    SnapshotInstallerScript = Get-FileProvenance (Join-Path $SnapshotDirectory 'installer\MichStartupMaster.iss')
    SnapshotBuildScript = Get-FileProvenance (Join-Path $SnapshotDirectory 'scripts\build.ps1')
    LegacyScripts = @(
        foreach ($legacyScript in @('Install-LocalProduction.ps1', 'Install-ProvenLocalTask.ps1', 'Install-StartupFolderAgent.ps1', 'Launch-Agent.ps1', 'Stop-MichStartupMaster.ps1', 'Test-StartupFolderAgent.ps1', 'Verify-StartupAgent.ps1')) {
            [pscustomobject][ordered]@{
                Root = Get-FileProvenance (Join-Path $Root $legacyScript)
                Snapshot = Get-FileProvenance (Join-Path $SnapshotDirectory $legacyScript)
            }
        }
    )
}

$priorDotnetRoot = $env:DOTNET_ROOT
$priorMultilevelLookup = $env:DOTNET_MULTILEVEL_LOOKUP
$priorTelemetryOptOut = $env:DOTNET_CLI_TELEMETRY_OPTOUT
$priorFirstTimeExperience = $env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE
try {
    $env:DOTNET_ROOT = Split-Path -Path $dotnet -Parent
    $env:DOTNET_MULTILEVEL_LOOKUP = '0'
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'
    & $dotnet restore (Join-Path $SnapshotDirectory $ProjectName) -r win-x64 --locked-mode --disable-parallel --nologo --tl:off
    if ($LASTEXITCODE -ne 0) {
        throw "dotnet restore failed with exit $LASTEXITCODE"
    }
    Assert-ManifestsEqual -Expected $sourceManifest -Actual @(Get-SourceManifest $SnapshotDirectory) -Context 'Snapshotted source after dependency restore'
    Assert-ManifestsEqual -Expected $sourceManifest -Actual @(Get-SourceManifest $Root) -Context 'Root source after dependency restore'
    Write-BuildState 'Locked restore completed; publishing the self-contained Windows candidate.'
    & $dotnet publish (Join-Path $SnapshotDirectory $ProjectName) -c Release -r win-x64 --self-contained true -o $PublishDirectory --no-restore --nologo --disable-build-servers --tl:off -m:1 -p:UseSharedCompilation=false
    if ($LASTEXITCODE -ne 0) {
        throw "dotnet publish failed with exit $LASTEXITCODE"
    }
    Assert-ManifestsEqual -Expected $sourceManifest -Actual @(Get-SourceManifest $SnapshotDirectory) -Context 'Snapshotted source after publish'
    Assert-ManifestsEqual -Expected $sourceManifest -Actual @(Get-SourceManifest $Root) -Context 'Root source after publish'
} finally {
    $env:DOTNET_ROOT = $priorDotnetRoot
    $env:DOTNET_MULTILEVEL_LOOKUP = $priorMultilevelLookup
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = $priorTelemetryOptOut
    $env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = $priorFirstTimeExperience
}

if (-not (Test-Path -LiteralPath $DependencyLockPath -PathType Leaf)) { throw "Locked dependency graph was not generated: $DependencyLockPath" }
$assetsPath = Join-Path $SnapshotDirectory 'obj\project.assets.json'
if (-not (Test-Path -LiteralPath $assetsPath -PathType Leaf)) { throw "Resolved dependency assets were not generated: $assetsPath" }
$nugetSources = @(& $dotnet nuget list source --format Detailed 2>&1 | ForEach-Object { [string]$_ })
if ($LASTEXITCODE -ne 0) { throw "Could not record NuGet source provenance (exit $LASTEXITCODE)." }

Copy-Item -LiteralPath (Join-Path $SnapshotDirectory 'assets\MichStartupMaster.ico') -Destination (Join-Path $PublishDirectory $IconName) -Force

Write-BuildState 'Publish completed; running staged layout and behavior gates.'
$stageValidation = Assert-BuildLayout -Directory $PublishDirectory -RunBehaviorGate
$publishManifest = Get-DirectoryManifest $PublishDirectory
$publishManifestSha256 = Get-ManifestSha256 $publishManifest
$publishManifestLines = @(
    foreach ($row in @($publishManifest | Sort-Object RelativePath)) {
        $relativePath = ([string]$row.RelativePath).Replace('/', '\')
        if ($relativePath.Contains('|') -or $relativePath.Contains("`r") -or $relativePath.Contains("`n")) { throw "Publish path cannot be represented safely in the installer manifest: $relativePath" }
        ([string]$row.Sha256).ToUpperInvariant() + '|' + [string][long]$row.Length + '|' + $relativePath
    }
)
[IO.File]::WriteAllLines($ValidatedPublishManifestPath, $publishManifestLines, (New-Object Text.UTF8Encoding($true)))
$publishManifestFileProvenance = Get-FileProvenance $ValidatedPublishManifestPath
$stageReceipt = [pscustomobject][ordered]@{
    SchemaVersion = 2
    Outcome = 'STAGED_AND_VALIDATED'
    StageOnly = [bool]$StageOnly
    CompileInstaller = [bool]$CompileInstaller
    TransactionId = $TransactionId
    CreatedUtc = [DateTime]::UtcNow.ToString('o')
    SourceRoot = Get-FullPath $Root
    SourceManifest = $sourceManifest
    SourceManifestSha256 = Get-ManifestSha256 $sourceManifest
    SnapshotDirectory = $SnapshotDirectory
    SnapshotAttempts = $sourceSnapshot.Attempts
    BuildInputs = $buildInputs
    PublishDirectory = $PublishDirectory
    PublishSdk = $sdkProvenance
    PublishSdkBootstrapped = $dotnetResolution.Bootstrapped
    PublishSdkCache = $dotnetResolution.CachePath
    RestoreCommand = @('restore', $ProjectName, '-r', 'win-x64', '--locked-mode', '--disable-parallel', '--nologo', '--tl:off')
    PublishCommand = @('publish', $ProjectName, '-c', 'Release', '-r', 'win-x64', '--self-contained', 'true', '--no-restore', '--nologo', '--disable-build-servers', '--tl:off', '-m:1', '-p:UseSharedCompilation=false')
    DependencyLock = Get-FileProvenance $DependencyLockPath
    DependencyAssets = Get-FileProvenance $assetsPath
    NuGetSources = @($nugetSources)
    PublishManifest = $publishManifest
    PublishManifestSha256 = $publishManifestSha256
    PublishManifestFile = $publishManifestFileProvenance
    Validation = $stageValidation
    InstallerMetadata = $InstallerMetadataPath
    InstallerOutputDirectory = $InstallerOutputDirectory
}
Write-JsonFile -Path $StageReceiptPath -Value $stageReceipt

$receiptProvenance = Get-FileProvenance $StageReceiptPath
foreach ($value in @($PublishDirectory, $StageReceiptPath, $ValidatedPublishManifestPath, $InstallerOutputDirectory, $TransactionId, $receiptProvenance.Sha256, $publishManifestSha256, $publishManifestFileProvenance.Sha256, $stageValidation.ExecutableSha256)) {
    if (([string]$value).Contains('"')) { throw 'A validated installer metadata value contains an unsupported quote character.' }
}
$installerMetadataLines = [string[]]@(
    ('#define ValidatedPublishDir "' + $PublishDirectory + '"'),
    ('#define ValidatedStageReceipt "' + $StageReceiptPath + '"'),
    ('#define ValidatedInstallerOutputDir "' + $InstallerOutputDirectory + '"'),
    ('#define ValidatedTransactionId "' + $TransactionId + '"'),
    ('#define ValidatedStageReceiptSha256 "' + $receiptProvenance.Sha256 + '"'),
    ('#define ValidatedPublishManifestSha256 "' + $publishManifestSha256 + '"'),
    ('#define ValidatedPublishManifest "' + $ValidatedPublishManifestPath + '"'),
    ('#define ValidatedPublishManifestFileSha256 "' + $publishManifestFileProvenance.Sha256 + '"'),
    ('#define ValidatedExecutableSha256 "' + $stageValidation.ExecutableSha256 + '"')
)
[IO.File]::WriteAllLines($InstallerMetadataPath, $installerMetadataLines, (New-Object Text.UTF8Encoding($false)))
$installerMetadataProvenance = Get-FileProvenance $InstallerMetadataPath
Write-JsonFileAtomic -Path $InstallProvenancePath -Value ([pscustomobject][ordered]@{
    SchemaVersion = 1
    Outcome = 'INSTALL_PROVENANCE_HASH_BOUND'
    TransactionId = $TransactionId
    CreatedUtc = [DateTime]::UtcNow.ToString('o')
    StageReceipt = $receiptProvenance
    StageMetadata = $installerMetadataProvenance
    PublishManifestFile = $publishManifestFileProvenance
    PublishManifestSha256 = $publishManifestSha256
    ExecutableSha256 = $stageValidation.ExecutableSha256
    SourceManifestSha256 = Get-ManifestSha256 $sourceManifest
})
$installProvenance = Get-FileProvenance $InstallProvenancePath

$installerReceiptPath = $null
if ($CompileInstaller) {
    Write-BuildState 'Candidate behavior gates passed; compiling the hash-bound installer from the immutable snapshot.'
    New-Item -ItemType Directory -Path $InstallerOutputDirectory -Force | Out-Null
    $isccCandidates = @('C:\Program Files (x86)\Inno Setup 6\ISCC.exe', 'C:\Program Files\Inno Setup 6\ISCC.exe')
    $iscc = $isccCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($iscc)) { throw 'Inno Setup 6 compiler was not found.' }
    $preInstallerManifest = Get-DirectoryManifest $PublishDirectory
    Assert-ManifestsEqual -Expected $publishManifest -Actual $preInstallerManifest -Context 'Pre-installer validated publish'
    $snapshotInstallerScript = Join-Path $SnapshotDirectory 'installer\MichStartupMaster.iss'
    Assert-FileProvenance -Expected $buildInputs.SnapshotInstallerScript -Context 'Immutable snapshotted installer script'
    $compilerOutput = @(& $iscc `
        ('/DValidatedStageMetadata=' + $InstallerMetadataPath) `
        ('/DValidatedStageMetadataSha256=' + $installerMetadataProvenance.Sha256) `
        ('/DValidatedInstallProvenance=' + $InstallProvenancePath) `
        ('/DValidatedInstallProvenanceSha256=' + $installProvenance.Sha256) `
        $snapshotInstallerScript 2>&1 | ForEach-Object { [string]$_ })
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed with exit $LASTEXITCODE.`n$($compilerOutput -join [Environment]::NewLine)" }
    Assert-FileProvenance -Expected $buildInputs.SnapshotInstallerScript -Context 'Post-compile immutable snapshotted installer script'
    $postInstallerManifest = Get-DirectoryManifest $PublishDirectory
    Assert-ManifestsEqual -Expected $publishManifest -Actual $postInstallerManifest -Context 'Post-installer validated publish'
    $installerFiles = @(Get-ChildItem -LiteralPath $InstallerOutputDirectory -File -Force -ErrorAction Stop)
    if ($installerFiles.Count -ne 1) { throw "Expected exactly one compiled installer in '$InstallerOutputDirectory', found $($installerFiles.Count)." }
    $installerReceiptPath = Join-Path $StageDirectory 'installer-receipt.json'
    Write-JsonFile -Path $installerReceiptPath -Value ([pscustomobject][ordered]@{
        SchemaVersion = 1
        Outcome = 'INSTALLER_COMPILED_FROM_VALIDATED_STAGE'
        TransactionId = $TransactionId
        CompiledUtc = [DateTime]::UtcNow.ToString('o')
        Compiler = Get-FileProvenance $iscc
        CompilerOutput = @($compilerOutput)
        StageReceipt = $receiptProvenance
        StageMetadata = $installerMetadataProvenance
        InstallProvenance = $installProvenance
        PublishManifestFile = $publishManifestFileProvenance
        InstallerSource = Get-FileProvenance $snapshotInstallerScript
        PublishManifestSha256 = $publishManifestSha256
        Installer = Get-FileProvenance $installerFiles[0].FullName
    })
}

Assert-FileProvenance -Expected $receiptProvenance -Context 'Final immutable stage receipt'
Assert-FileProvenance -Expected $installerMetadataProvenance -Context 'Final immutable installer metadata'
Assert-FileProvenance -Expected $installProvenance -Context 'Final immutable install provenance'
Assert-FileProvenance -Expected $publishManifestFileProvenance -Context 'Final immutable publish manifest file'
Assert-FileProvenance -Expected $buildInputs.BuildScript -Context 'Build script during staging'
Assert-FileProvenance -Expected $buildInputs.BootstrapScript -Context 'SDK bootstrap script during staging'
Assert-FileProvenance -Expected $buildInputs.InstallerScript -Context 'Root installer script during staging'
Assert-FileProvenance -Expected $buildInputs.SnapshotInstallerScript -Context 'Snapshotted installer script during staging'
Assert-FileProvenance -Expected $buildInputs.SnapshotBuildScript -Context 'Snapshotted build script during staging'
foreach ($legacyPair in @($buildInputs.LegacyScripts)) {
    Assert-FileProvenance -Expected $legacyPair.Root -Context 'Root legacy lifecycle script during staging'
    Assert-FileProvenance -Expected $legacyPair.Snapshot -Context 'Snapshotted legacy lifecycle script during staging'
}

if ($StageOnly) {
    Write-BuildState ("Validated StageOnly candidate completed at '{0}'; live deployment was not touched." -f $StageDirectory)
    [pscustomobject][ordered]@{
        Outcome = 'STAGED_AND_VALIDATED'
        LiveDeploymentTouched = $false
        LiveProcessesStopped = $false
        StageDirectory = $StageDirectory
        PublishDirectory = $PublishDirectory
        Receipt = $StageReceiptPath
        InstallerMetadata = $InstallerMetadataPath
        InstallProvenance = $InstallProvenancePath
        ValidatedPublishManifest = $ValidatedPublishManifestPath
        InstallerReceipt = $installerReceiptPath
        ExecutableSha256 = $stageValidation.ExecutableSha256
        EmbeddedIconSize = $stageValidation.EmbeddedIconSize
        FileCount = $publishManifest.Count
    } | Format-List
    return
}

# Refuse to deploy a snapshot when another editor changed an input while publish/validation ran.
$currentSourceManifest = Get-SourceManifest $Root
Assert-ManifestsEqual -Expected $sourceManifest -Actual $currentSourceManifest -Context 'Pre-deploy source freshness'
Assert-FileProvenance -Expected $buildInputs.BuildScript -Context 'Build script'
Assert-FileProvenance -Expected $buildInputs.BootstrapScript -Context 'SDK bootstrap script'
Assert-FileProvenance -Expected $buildInputs.InstallerScript -Context 'Installer script'

$deploymentMutex = New-Object Threading.Mutex($false, $DeploymentMutexName)
$deploymentMutexHeld = $false
$externalMutationMutex = $null
$externalMutationMutexHeld = $false
try {
    try {
        $deploymentMutexHeld = $deploymentMutex.WaitOne([TimeSpan]::FromMinutes(10))
    } catch [Threading.AbandonedMutexException] {
        $deploymentMutexHeld = $true
    }
    if (-not $deploymentMutexHeld) { throw 'Timed out waiting for the MichStartupMaster deployment lock.' }
    if (-not (Test-Path -LiteralPath $RuntimeOutputDirectory -PathType Container)) {
        New-Item -ItemType Directory -Path $RuntimeOutputDirectory -Force | Out-Null
    }
    $ActiveDeploymentMarkerPath = Join-Path $RuntimeOutputDirectory 'deployment-active.json'
    if (Test-Path -LiteralPath $ActiveDeploymentMarkerPath -PathType Leaf) {
        try { $priorMarker = Get-Content -LiteralPath $ActiveDeploymentMarkerPath -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop }
        catch { throw "The durable deployment marker is unreadable. Preserve and inspect it before retrying: $ActiveDeploymentMarkerPath. $($_.Exception.Message)" }
        $priorMarkerIsTerminal =
            ([string]$priorMarker.Outcome -eq 'DEPLOYMENT_COMPLETED' -and [string]$priorMarker.Phase -eq 'DEPLOYED_AND_VERIFIED') -or
            ([string]$priorMarker.Outcome -eq 'DEPLOYMENT_FAILED' -and [string]$priorMarker.Phase -eq 'ROLLBACK_VERIFIED') -or
            ([string]$priorMarker.Outcome -eq 'EXTERNAL_STATE_RESTORED_AND_VERIFIED' -and [string]$priorMarker.Phase -eq 'MANUAL_RESTORE_VERIFIED')
        if (-not $priorMarkerIsTerminal) {
            throw "A prior deployment has an unfinished durable marker. Recover it before retrying: $ActiveDeploymentMarkerPath (transaction: $($priorMarker.TransactionDirectory))"
        }
    }

$TransactionDirectory = Join-Path $RuntimeOutputDirectory ('deploy-' + $TransactionId)
$RollbackCopyDirectory = Join-Path $TransactionDirectory 'rollback-copy'
$CandidateDirectory = Join-Path $TransactionDirectory 'candidate'
$MovedOriginalDirectory = Join-Path $TransactionDirectory 'original-build-moved'
$FailedCandidateDirectory = Join-Path $TransactionDirectory 'failed-candidate'
if ($InstallCurrentUser) {
    # Directory swaps must stay on the installed volume. Recovery receipts and
    # independent backup copies remain in the canonical project transaction.
    $installParent = Get-FullPath (Join-Path $env:LOCALAPPDATA 'Programs')
    $installSwapRoot = Join-Path $installParent ('MichStartupMaster-deploy-' + $TransactionId)
    Assert-NoReparsePointTraversal -Path $DeployDirectory -TrustedRoot (Get-FullPath $env:LOCALAPPDATA) -Context 'Installed deployment target'
    Assert-NoReparsePointsInTree -Directory $DeployDirectory -Context 'Installed deployment tree'
    New-EmptyDirectory $installSwapRoot
    $CandidateDirectory = Get-CanonicalChildPath -Parent $installSwapRoot -RelativePath 'candidate' -Context 'Installed candidate'
    $MovedOriginalDirectory = Get-CanonicalChildPath -Parent $installSwapRoot -RelativePath 'original-build-moved' -Context 'Installed rollback original'
    $FailedCandidateDirectory = Get-CanonicalChildPath -Parent $installSwapRoot -RelativePath 'failed-candidate' -Context 'Installed failed candidate'
}

$ProcessStatePath = Join-Path $TransactionDirectory 'process-state.json'
$TransactionReceiptPath = Join-Path $TransactionDirectory 'deploy-receipt.json'
$DeploymentCommitPath = Join-Path $TransactionDirectory 'deployment-commit.json'
$InitialExternalStateDirectory = Join-Path $TransactionDirectory 'external-state-initial-recovery-copy'
$StableExternalStateDirectory = Join-Path $TransactionDirectory 'external-state-stable-post-stop'
New-Item -ItemType Directory -Path $TransactionDirectory -Force | Out-Null

Copy-DirectoryExact -Source $PublishDirectory -Destination $CandidateDirectory
$candidateManifest = Get-DirectoryManifest $CandidateDirectory
Assert-ManifestsEqual -Expected $publishManifest -Actual $candidateManifest -Context 'Deployment candidate'
[void](Assert-BuildLayout $CandidateDirectory)

$rollbackManifest = @()
if (Test-Path -LiteralPath $DeployDirectory -PathType Container) {
    Copy-DirectoryExact -Source $DeployDirectory -Destination $RollbackCopyDirectory
    $originalManifest = @(Get-DirectoryManifest $DeployDirectory)
    $rollbackManifest = Get-DirectoryManifest $RollbackCopyDirectory
    Assert-ManifestsEqual -Expected $originalManifest -Actual $rollbackManifest -Context 'Pre-deploy rollback copy'
} else {
    New-Item -ItemType Directory -Path $RollbackCopyDirectory -Force | Out-Null
    $originalManifest = @()
}

# Acquire the same app-wide gate honored by every state/registration mutation.
# It remains held through the stable snapshot and layout commit, or through all
# external rollback phases, so no workspace/installed generation can race the
# authoritative state boundary.
    $externalMutationMutex = New-Object Threading.Mutex($false, $ManagedStartupMutationMutexName)
try { $externalMutationMutexHeld = $externalMutationMutex.WaitOne([TimeSpan]::FromMinutes(2)) }
catch [Threading.AbandonedMutexException] { $externalMutationMutexHeld = $true }
if (-not $externalMutationMutexHeld) { throw 'Timed out waiting for the MichStartupMaster external-state mutation lock.' }

# Capture exact native process generations first, then an initial recovery copy.
# The initial copy is deliberately not called authoritative while an agent is
# live; it exists only as a last-resort artifact if the post-stop snapshot fails.
$deployExecutable = Join-Path $DeployDirectory $ExecutableName
$managedProcessPaths = @((Get-OwnedLaunchPaths -DeploymentDirectory $DeployDirectory).Executables | ForEach-Object { Get-FullPath ([string]$_) } | Sort-Object -Unique)
$processState = @(Get-AllManagerProcessState -ExecutablePaths $managedProcessPaths)
Write-JsonFileAtomic -Path $ProcessStatePath -Value ([pscustomobject][ordered]@{
    CapturedUtc = [DateTime]::UtcNow.ToString('o')
    ExecutablePaths = $managedProcessPaths
    Processes = $processState
})
Assert-FileProvenance -Expected $buildInputs.SnapshotBuildScript -Context 'Initial recovery-tool source'
$initialExternalState = New-ExternalStateSnapshot -SnapshotDirectory $InitialExternalStateDirectory -DeploymentDirectory $DeployDirectory `
    -ProjectRoot $Root -TransactionDirectory $TransactionDirectory -SnapshotKind 'INITIAL_RECOVERY' -StageReceiptPath $StageReceiptPath `
    -RestoreToolSource ([string]$buildInputs.SnapshotBuildScript.Path) -SourceManifestSha256 (Get-ManifestSha256 $sourceManifest) -PublishManifestSha256 $publishManifestSha256
if (-not [string]::Equals([string]$initialExternalState.RestoreToolSha256, [string]$buildInputs.SnapshotBuildScript.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Initial recovery copy contains a different restore-tool source hash.'
}

$processesCoordinated = $false
$originalMoved = $false
$candidateMoved = $false
$externalMutationPossible = $false
$externalState = $null
$externalRestoreResult = $null
$restartResults = @()
$candidateStartedProcessJournal = New-Object Collections.Generic.List[object]
Write-DeploymentMarker -Path $ActiveDeploymentMarkerPath -Outcome 'DEPLOYMENT_IN_PROGRESS' -Phase 'READY_TO_COORDINATE_RECORDED_PROCESSES' `
    -TransactionDirectory $TransactionDirectory -StageReceipt $StageReceiptPath -SourceManifestSha256 (Get-ManifestSha256 $sourceManifest) -PublishManifestSha256 $publishManifestSha256
try {
    if ($processState.Count -gt 0) {
        # From this point onward a partial failure may already have stopped one or more
        # recorded modes, so the catch path must always attempt their restoration.
        $processesCoordinated = $true
        Stop-RecordedProcesses -ProcessState $processState -WaitSeconds $GracefulStopSeconds
        $remaining = @(Get-AllManagerProcessState -ExecutablePaths $managedProcessPaths)
        if ($remaining.Count -ne 0) {
            throw "Refusing deployment because $($remaining.Count) unrecorded or surviving workspace/installed manager process generation(s) remain."
        }
    }

    # Shutdown can flush durable intent. Only this post-stop, hash-verified copy is
    # authoritative for automatic rollback. If it cannot be captured, the catch
    # path restarts the old modes while the deployment directory is still intact.
    Assert-FileProvenance -Expected $buildInputs.SnapshotBuildScript -Context 'Stable recovery-tool source'
    $externalState = New-ExternalStateSnapshot -SnapshotDirectory $StableExternalStateDirectory -DeploymentDirectory $DeployDirectory `
        -ProjectRoot $Root -TransactionDirectory $TransactionDirectory -SnapshotKind 'STABLE_POST_STOP' -StageReceiptPath $StageReceiptPath `
        -RestoreToolSource ([string]$buildInputs.SnapshotBuildScript.Path) -SourceManifestSha256 (Get-ManifestSha256 $sourceManifest) -PublishManifestSha256 $publishManifestSha256
    if (-not [string]::Equals([string]$externalState.RestoreToolSha256, [string]$buildInputs.SnapshotBuildScript.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Stable recovery copy contains a different restore-tool source hash.'
    }
    Assert-ExternalStateMatchesSnapshot -Snapshot $externalState
    Write-DeploymentMarker -Path $ActiveDeploymentMarkerPath -Outcome 'DEPLOYMENT_IN_PROGRESS' -Phase 'STABLE_POST_STOP_STATE_CAPTURED' `
        -TransactionDirectory $TransactionDirectory -StageReceipt $StageReceiptPath -SourceManifestSha256 (Get-ManifestSha256 $sourceManifest) `
        -PublishManifestSha256 $publishManifestSha256 -ExternalSnapshotReceipt ([string]$externalState.BindingReceiptPath) `
        -ExternalSnapshotReceiptSha256 ([string]$externalState.BindingReceiptSha256)
    $externalMutationPossible = $true

    if (Test-Path -LiteralPath $DeployDirectory -PathType Container) {
        Move-Item -LiteralPath $DeployDirectory -Destination $MovedOriginalDirectory -ErrorAction Stop
        $originalMoved = $true
    }
    Move-Item -LiteralPath $CandidateDirectory -Destination $DeployDirectory -ErrorAction Stop
    $candidateMoved = $true
    Write-DeploymentMarker -Path $ActiveDeploymentMarkerPath -Outcome 'DEPLOYMENT_IN_PROGRESS' -Phase 'CANDIDATE_LAYOUT_MOVED' `
        -TransactionDirectory $TransactionDirectory -StageReceipt $StageReceiptPath -SourceManifestSha256 (Get-ManifestSha256 $sourceManifest) `
        -PublishManifestSha256 $publishManifestSha256 -ExternalSnapshotReceipt ([string]$externalState.BindingReceiptPath) `
        -ExternalSnapshotReceiptSha256 ([string]$externalState.BindingReceiptSha256)

    $deployedManifest = Get-DirectoryManifest $DeployDirectory
    Assert-ManifestsEqual -Expected $publishManifest -Actual $deployedManifest -Context 'Deployed build'
    $deployedValidation = Assert-BuildLayout $DeployDirectory
    if (-not [string]::Equals($deployedValidation.ExecutableSha256, $stageValidation.ExecutableSha256, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals($deployedValidation.BundledIconSha256, $stageValidation.BundledIconSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Deployed executable/icon hashes do not match the validated stage.'
    }

    # Commit the fully hash-verified layout before any restarted agent is allowed to
    # reconcile AppData/tasks/startup registrations. A later verification failure
    # still rolls back both the build and the captured external state.
    Write-JsonFileAtomic -Path $DeploymentCommitPath -Value ([pscustomobject][ordered]@{
        SchemaVersion = 1
        Outcome = 'DEPLOYMENT_LAYOUT_COMMITTED_BEFORE_RESTART'
        TransactionId = $TransactionId
        CommittedUtc = [DateTime]::UtcNow.ToString('o')
        StageReceiptSha256 = (Get-FileHash -LiteralPath $StageReceiptPath -Algorithm SHA256).Hash
        PublishManifestSha256 = $publishManifestSha256
        DeployedManifestSha256 = Get-ManifestSha256 $deployedManifest
        InitialExternalStateSnapshot = Join-Path $InitialExternalStateDirectory 'external-state.json'
        StableExternalStateSnapshot = Join-Path $StableExternalStateDirectory 'external-state.json'
        ExternalSnapshotReceipt = Get-FileProvenance ([string]$externalState.BindingReceiptPath)
    })

    # Layout and rollback material are now committed. Release the app-wide gate
    # before starting agents, which may acquire it while reconciling durable state.
    if ($externalMutationMutexHeld) {
        $externalMutationMutex.ReleaseMutex()
        $externalMutationMutexHeld = $false
    }
    if ($processState.Count -gt 0) {
        $restartResults = @(Restart-RecordedProcesses -ProcessState $processState -ExecutablePaths $managedProcessPaths -StartedProcessJournal $candidateStartedProcessJournal)
    }

    $successReceipt = [pscustomobject][ordered]@{
        SchemaVersion = 1
        Outcome = 'DEPLOYED_AND_VERIFIED'
        TransactionId = $TransactionId
        CompletedUtc = [DateTime]::UtcNow.ToString('o')
        StageReceipt = Get-FileProvenance $StageReceiptPath
        SourceManifestSha256 = Get-ManifestSha256 $sourceManifest
        PublishManifestSha256 = $publishManifestSha256
        DeployDirectory = $DeployDirectory
        RollbackCopy = $RollbackCopyDirectory
        ExactMovedOriginal = if ($originalMoved) { $MovedOriginalDirectory } else { $null }
        PreviousProcessState = $ProcessStatePath
        InitialExternalStateSnapshot = Join-Path $InitialExternalStateDirectory 'external-state.json'
        ExternalStateSnapshot = Join-Path $StableExternalStateDirectory 'external-state.json'
        ExternalSnapshotReceipt = Get-FileProvenance ([string]$externalState.BindingReceiptPath)
        DeploymentCommit = $DeploymentCommitPath
        PreviousManifest = $originalManifest
        DeployedManifest = $deployedManifest
        Validation = $deployedValidation
        RestartResults = $restartResults
        DeployedManifestSha256 = Get-ManifestSha256 $deployedManifest
        FinalProcessState = if ($processState.Count -gt 0) { @(Get-AllManagerProcessState -ExecutablePaths $managedProcessPaths) } else { @() }
    }
    Write-JsonFileAtomic -Path $TransactionReceiptPath -Value $successReceipt
    Write-DeploymentMarker -Path $ActiveDeploymentMarkerPath -Outcome 'DEPLOYMENT_COMPLETED' -Phase 'DEPLOYED_AND_VERIFIED' `
        -TransactionDirectory $TransactionDirectory -StageReceipt $StageReceiptPath -DeploymentReceipt $TransactionReceiptPath `
        -SourceManifestSha256 (Get-ManifestSha256 $sourceManifest) -PublishManifestSha256 $publishManifestSha256 `
        -ExternalSnapshotReceipt ([string]$externalState.BindingReceiptPath) -ExternalSnapshotReceiptSha256 ([string]$externalState.BindingReceiptSha256)

    [pscustomobject][ordered]@{
        Outcome = 'DEPLOYED_AND_VERIFIED'
        DeployDirectory = $DeployDirectory
        ExecutableSha256 = $deployedValidation.ExecutableSha256
        EmbeddedIconSize = $deployedValidation.EmbeddedIconSize
        RollbackCopy = $RollbackCopyDirectory
        ExactMovedOriginal = if ($originalMoved) { $MovedOriginalDirectory } else { '(new installation)' }
        PreviousModeRestartAttempts = $restartResults.Count
        Receipt = $TransactionReceiptPath
        ExternalStateSnapshot = Join-Path $StableExternalStateDirectory 'external-state.json'
    } | Format-List
} catch {
    $deploymentError = $_
    $rollbackPhases = New-Object Collections.Generic.List[object]
    $rollbackErrors = New-Object Collections.Generic.List[string]
    $restoreVerified = $false
    $mutationGatePassed = $externalMutationMutexHeld
    if (-not $externalMutationMutexHeld) {
        try {
            try { $externalMutationMutexHeld = $externalMutationMutex.WaitOne([TimeSpan]::FromMinutes(2)) }
            catch [Threading.AbandonedMutexException] { $externalMutationMutexHeld = $true }
            if (-not $externalMutationMutexHeld) { throw 'Timed out reacquiring the external-state mutation lock for rollback.' }
            $mutationGatePassed = $true
        } catch {
            $mutationGatePassed = $false
            $rollbackErrors.Add('ExternalMutationGate: ' + $_.Exception.Message)
        }
    }
    $rollbackPhases.Add([pscustomobject][ordered]@{ Phase = 'AcquireExternalMutationGate'; Passed = $mutationGatePassed; Required = $externalMutationPossible; Errors = if ($mutationGatePassed) { @() } else { @('The shared external-state mutation gate was unavailable.') } })

    # Phase 1: stop only candidate generations started by this transaction. A
    # user-opened candidate is never adopted into the rollback stop set.
    $phaseErrors = New-Object Collections.Generic.List[string]
    try {
        if ($candidateStartedProcessJournal.Count -gt 0) {
            Stop-RecordedProcesses -ProcessState $candidateStartedProcessJournal.ToArray() -WaitSeconds $GracefulStopSeconds
        }
        $unownedCandidateProcesses = @(Get-AllManagerProcessState -ExecutablePaths $managedProcessPaths)
        if ($unownedCandidateProcesses.Count -gt 0) {
            throw "Refusing to stop $($unownedCandidateProcesses.Count) workspace/installed process generation(s) not journaled as transaction-started."
        }
    } catch { $phaseErrors.Add($_.Exception.Message) }
    $phasePassed = $phaseErrors.Count -eq 0
    $rollbackPhases.Add([pscustomobject][ordered]@{ Phase = 'StopTransactionStartedCandidateProcesses'; Passed = $phasePassed; Errors = $phaseErrors.ToArray() })
    if (-not $phasePassed) { $rollbackErrors.Add('CandidateProcesses: ' + ($phaseErrors.ToArray() -join ' | ')) }

    # Phase 2: recover and hash-verify the prior build independently.
    $phaseErrors = New-Object Collections.Generic.List[string]
    try {
        if ($candidateMoved -and (Test-Path -LiteralPath $DeployDirectory -PathType Container)) {
            $candidateProcessesStillLive = if (Test-Path -LiteralPath (Join-Path $DeployDirectory $ExecutableName) -PathType Leaf) { @(Get-LiveProcessState -ExecutablePath (Join-Path $DeployDirectory $ExecutableName)) } else { @() }
            if ($candidateProcessesStillLive.Count -gt 0) { throw 'Candidate build still has a live process; its directory was preserved in place.' }
            Move-Item -LiteralPath $DeployDirectory -Destination $FailedCandidateDirectory -ErrorAction Stop
        }
        if ($originalMoved -and (Test-Path -LiteralPath $MovedOriginalDirectory -PathType Container)) {
            if (Test-Path -LiteralPath $DeployDirectory) { throw "Deployment path is occupied and the moved original cannot be restored: $DeployDirectory" }
            Move-Item -LiteralPath $MovedOriginalDirectory -Destination $DeployDirectory -ErrorAction Stop
        } elseif ($originalManifest.Count -gt 0 -and -not (Test-Path -LiteralPath $DeployDirectory -PathType Container)) {
            Copy-DirectoryExact -Source $RollbackCopyDirectory -Destination $DeployDirectory
        }
        if ($originalManifest.Count -gt 0) {
            if (-not (Test-Path -LiteralPath $DeployDirectory -PathType Container)) { throw 'The prior deployment directory is missing.' }
            Assert-ManifestsEqual -Expected $originalManifest -Actual @(Get-DirectoryManifest $DeployDirectory) -Context 'Automatic rollback restoration'
        } elseif (Test-Path -LiteralPath $DeployDirectory) {
            throw 'A deployment path remains even though the pre-deploy state had no build.'
        }
    } catch { $phaseErrors.Add($_.Exception.Message) }
    $buildRestorePassed = $phaseErrors.Count -eq 0
    $rollbackPhases.Add([pscustomobject][ordered]@{ Phase = 'RestorePriorBuild'; Passed = $buildRestorePassed; Errors = $phaseErrors.ToArray() })
    if (-not $buildRestorePassed) { $rollbackErrors.Add('PriorBuild: ' + ($phaseErrors.ToArray() -join ' | ')) }

    # Phase 3: recover every captured external-state surface even if build recovery
    # failed. Restore-ExternalStateSnapshot itself runs tasks, shortcuts, registry,
    # and AppData as separately verified subphases.
    $phaseErrors = New-Object Collections.Generic.List[string]
    try {
        if ($externalMutationPossible) {
            if (-not $externalMutationMutexHeld) { throw 'External state was not restored because the shared mutation gate is not held.' }
            if ($null -eq $externalState) { throw 'The authoritative post-stop external-state snapshot is unavailable.' }
            $unsafeLiveProcesses = @(Get-AllManagerProcessState -ExecutablePaths $managedProcessPaths)
            if ($unsafeLiveProcesses.Count -ne 0) { throw 'External state was not restored because an unowned exact manager process generation is live.' }
            $externalRestoreResult = Restore-ExternalStateSnapshot -Snapshot $externalState
            if ($externalRestoreResult.Verified -ne $true) { throw 'External-state restoration did not return a verified receipt.' }
        }
    } catch { $phaseErrors.Add($_.Exception.Message) }
    $externalRestorePassed = $phaseErrors.Count -eq 0
    $rollbackPhases.Add([pscustomobject][ordered]@{ Phase = 'RestoreExternalState'; Passed = $externalRestorePassed; Required = $externalMutationPossible; Errors = $phaseErrors.ToArray(); Result = $externalRestoreResult })
    if (-not $externalRestorePassed) { $rollbackErrors.Add('ExternalState: ' + ($phaseErrors.ToArray() -join ' | ')) }

    # The external state is now committed/restored. Release the shared mutation
    # gate before old agents are restarted so their own state reconciliation cannot
    # deadlock behind this deployment thread.
    if ($externalMutationMutexHeld) {
        $externalMutationMutex.ReleaseMutex()
        $externalMutationMutexHeld = $false
    }

    # Phase 4: restart only the exact prior command-line multiplicities. It runs
    # even after external-state trouble, but never against an unverified build.
    $phaseErrors = New-Object Collections.Generic.List[string]
    try {
        if ($processesCoordinated -and $processState.Count -gt 0) {
            if (-not $buildRestorePassed) { throw 'Prior processes were not restarted because the prior build was not verified.' }
            $priorExecutable = Join-Path $DeployDirectory $ExecutableName
            if (-not (Test-Path -LiteralPath $priorExecutable -PathType Leaf)) { throw "Prior executable is missing: $priorExecutable" }
            $restartResults = @(Restart-RecordedProcesses -ProcessState $processState -ExecutablePaths $managedProcessPaths)
            [void](Assert-ExactProcessMultiplicity -ProcessState $processState -ExecutablePaths $managedProcessPaths -WaitSeconds 10)
        }
    } catch { $phaseErrors.Add($_.Exception.Message) }
    $processRestorePassed = $phaseErrors.Count -eq 0
    $rollbackPhases.Add([pscustomobject][ordered]@{ Phase = 'RestorePriorProcessModes'; Passed = $processRestorePassed; Required = ($processesCoordinated -and $processState.Count -gt 0); Errors = $phaseErrors.ToArray(); Results = @($restartResults) })
    if (-not $processRestorePassed) { $rollbackErrors.Add('PriorProcesses: ' + ($phaseErrors.ToArray() -join ' | ')) }

    $restoreVerified = $buildRestorePassed -and $externalRestorePassed -and $processRestorePassed -and ((-not $externalMutationPossible) -or $mutationGatePassed)
    $failureReceipt = [pscustomobject][ordered]@{
        SchemaVersion = 2
        Outcome = 'DEPLOYMENT_FAILED'
        TransactionId = $TransactionId
        FailedUtc = [DateTime]::UtcNow.ToString('o')
        StageReceipt = Get-FileProvenance $StageReceiptPath
        SourceManifestSha256 = Get-ManifestSha256 $sourceManifest
        PublishManifestSha256 = $publishManifestSha256
        DeploymentError = $deploymentError.Exception.ToString()
        RestoreVerified = $restoreVerified
        RollbackErrors = $rollbackErrors.ToArray()
        RollbackPhases = $rollbackPhases.ToArray()
        RollbackCopy = $RollbackCopyDirectory
        ExactMovedOriginal = if (Test-Path -LiteralPath $MovedOriginalDirectory) { $MovedOriginalDirectory } else { $null }
        FailedCandidate = if (Test-Path -LiteralPath $FailedCandidateDirectory) { $FailedCandidateDirectory } else { $null }
        PreviousProcessState = $ProcessStatePath
        InitialExternalStateSnapshot = Join-Path $InitialExternalStateDirectory 'external-state.json'
        ExternalStateSnapshot = Join-Path $StableExternalStateDirectory 'external-state.json'
        ExternalSnapshotReceipt = if ($null -eq $externalState) { $null } else { Get-FileProvenance ([string]$externalState.BindingReceiptPath) }
        ExternalMutationPossible = $externalMutationPossible
        ExternalRestoreResult = $externalRestoreResult
        DeploymentCommit = if (Test-Path -LiteralPath $DeploymentCommitPath) { $DeploymentCommitPath } else { $null }
        RestartResults = $restartResults
    }
    Write-JsonFileAtomic -Path $TransactionReceiptPath -Value $failureReceipt
    try {
        $rollbackMarkerPhase = if ($restoreVerified) { 'ROLLBACK_VERIFIED' } else { 'ROLLBACK_INCOMPLETE' }
        $rollbackExternalReceipt = if ($null -eq $externalState) { $null } else { [string]$externalState.BindingReceiptPath }
        $rollbackExternalReceiptSha256 = if ($null -eq $externalState) { $null } else { [string]$externalState.BindingReceiptSha256 }
        Write-DeploymentMarker -Path $ActiveDeploymentMarkerPath -Outcome 'DEPLOYMENT_FAILED' -Phase $rollbackMarkerPhase `
            -TransactionDirectory $TransactionDirectory -StageReceipt $StageReceiptPath -DeploymentReceipt $TransactionReceiptPath `
            -SourceManifestSha256 (Get-ManifestSha256 $sourceManifest) -PublishManifestSha256 $publishManifestSha256 `
            -ExternalSnapshotReceipt $rollbackExternalReceipt `
            -ExternalSnapshotReceiptSha256 $rollbackExternalReceiptSha256
    } catch { $rollbackErrors.Add('DurableMarker: ' + $_.Exception.Message) }

    if ($rollbackErrors.Count -gt 0) {
        throw "Deployment failed: $($deploymentError.Exception.Message). Automatic recovery was incomplete: $($rollbackErrors.ToArray() -join ' || '). Rollback copy: $RollbackCopyDirectory. Receipt: $TransactionReceiptPath"
    }
    throw "Deployment failed and every required prior-state phase was restored and verified: $($deploymentError.Exception.Message). Receipt: $TransactionReceiptPath"
}
} finally {
    if ($externalMutationMutexHeld) {
        $externalMutationMutex.ReleaseMutex()
    }
    if ($null -ne $externalMutationMutex) { $externalMutationMutex.Dispose() }
    if ($deploymentMutexHeld) {
        $deploymentMutex.ReleaseMutex()
    }
    $deploymentMutex.Dispose()
}
