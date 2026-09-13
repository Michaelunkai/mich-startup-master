[CmdletBinding()]
param(
    [string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    return [IO.Path]::GetFullPath($Path).TrimEnd('\')
}

function Test-PathsEqual {
    param([string]$Left, [string]$Right)
    try {
        return -not [string]::IsNullOrWhiteSpace($Left) -and -not [string]::IsNullOrWhiteSpace($Right) -and
            [string]::Equals((Get-FullPath $Left), (Get-FullPath $Right), [StringComparison]::OrdinalIgnoreCase)
    } catch { return $false }
}

function Get-StringSha256 {
    param([AllowEmptyString()][string]$Value)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes([string]$Value)))).Replace('-', '') }
    finally { $sha.Dispose() }
}

function Write-JsonAtomic {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)]$Value)
    $directory = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
    $temporary = Join-Path $directory ('.' + [IO.Path]::GetFileName($Path) + '.' + [Guid]::NewGuid().ToString('N') + '.tmp')
    $bytes = (New-Object Text.UTF8Encoding($false)).GetBytes(($Value | ConvertTo-Json -Depth 12))
    $stream = New-Object IO.FileStream($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
        $backup = $Path + '.previous.' + [Guid]::NewGuid().ToString('N')
        [IO.File]::Replace($temporary, $Path, $backup, $true)
        Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
    } else { [IO.File]::Move($temporary, $Path) }
}

function Get-DirectoryManifest {
    param([Parameter(Mandatory = $true)][string]$Directory)
    $base = (Get-FullPath $Directory) + '\'
    return @(
        Get-ChildItem -LiteralPath $Directory -Recurse -File -Force -ErrorAction Stop |
            Sort-Object FullName |
            ForEach-Object { [pscustomobject][ordered]@{ RelativePath = $_.FullName.Substring($base.Length).Replace('/', '\'); Length = [long]$_.Length; Sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } }
    )
}

function Assert-ManifestEqual {
    param([object[]]$Expected, [object[]]$Actual, [string]$Context)
    if ($Expected.Count -ne $Actual.Count) { throw "$Context file-count mismatch." }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        if (-not [string]::Equals([string]$Expected[$index].RelativePath, [string]$Actual[$index].RelativePath, [StringComparison]::OrdinalIgnoreCase) -or
            [long]$Expected[$index].Length -ne [long]$Actual[$index].Length -or
            -not [string]::Equals([string]$Expected[$index].Sha256, [string]$Actual[$index].Sha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw "$Context differs at '$($Expected[$index].RelativePath)'."
        }
    }
}

function Copy-DirectorySnapshot {
    param([Parameter(Mandatory = $true)][string]$Source, [Parameter(Mandatory = $true)][string]$Destination)
    if (Test-Path -LiteralPath $Destination) { throw "Snapshot destination already exists: $Destination" }
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    foreach ($entry in @(Get-ChildItem -LiteralPath $Source -Force -ErrorAction Stop)) {
        Copy-Item -LiteralPath $entry.FullName -Destination $Destination -Recurse -Force -ErrorAction Stop
    }
    $manifest = @(Get-DirectoryManifest $Source)
    Assert-ManifestEqual -Expected $manifest -Actual @(Get-DirectoryManifest $Destination) -Context 'Durable directory snapshot'
    return $manifest
}

function Assert-NoReparseTraversal {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$StopDirectory)
    $cursor = Get-FullPath $Path
    $stop = Get-FullPath $StopDirectory
    while ($true) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse-point traversal is not allowed: $cursor" }
        }
        if (Test-PathsEqual $cursor $stop) { break }
        $parent = Split-Path -Parent $cursor
        if ([string]::IsNullOrWhiteSpace($parent) -or (Test-PathsEqual $parent $cursor)) { throw "Path is outside the expected root: $Path" }
        $cursor = $parent
    }
}

function Assert-NoReparsePointsInTree {
    param([Parameter(Mandatory = $true)][string]$Directory, [Parameter(Mandatory = $true)][string]$Context)
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { return }
    $root = Get-Item -LiteralPath $Directory -Force -ErrorAction Stop
    if (($root.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "$Context contains a reparse point: $($root.FullName)" }
    foreach ($item in @(Get-ChildItem -LiteralPath $Directory -Recurse -Force -ErrorAction Stop)) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "$Context contains a reparse point: $($item.FullName)" }
    }
}

function Enter-Mutex {
    param([Parameter(Mandatory = $true)][Threading.Mutex]$Mutex, [Parameter(Mandatory = $true)][string]$Name)
    try {
        if (-not $Mutex.WaitOne([TimeSpan]::FromMinutes(2))) { throw "Timed out waiting for $Name." }
    } catch [Threading.AbandonedMutexException] {
        # An abandoned mutex is acquired by this thread. The durable marker below
        # is authoritative for deciding whether recovery is required.
    }
}

function New-DurableFileStates {
    param([Parameter(Mandatory = $true)][string[]]$Paths, [Parameter(Mandatory = $true)][string]$SnapshotDirectory)
    New-Item -ItemType Directory -Path $SnapshotDirectory -Force -ErrorAction Stop | Out-Null
    $states = @(); $index = 0
    foreach ($path in $Paths) {
        $index++
        $fullPath = Get-FullPath $path
        $existed = Test-Path -LiteralPath $fullPath -PathType Leaf
        $copy = $null; $sha256 = $null; $length = 0L
        if ($existed) {
            $copy = Join-Path $SnapshotDirectory ('file-{0:D2}.bin' -f $index)
            Copy-Item -LiteralPath $fullPath -Destination $copy -Force -ErrorAction Stop
            $copyItem = Get-Item -LiteralPath $copy -Force -ErrorAction Stop
            $sha256 = (Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash
            $length = [long]$copyItem.Length
        }
        $states += [pscustomobject][ordered]@{
            Path = $fullPath
            Existed = $existed
            Copy = $copy
            Sha256 = $sha256
            Length = $length
            ExpectedPostSha256 = $null
        }
    }
    return @($states)
}

function Assert-DurableFileStates {
    param([object[]]$States)
    foreach ($state in $States) {
        if (-not [bool]$state.Existed) { continue }
        if (-not (Test-Path -LiteralPath ([string]$state.Copy) -PathType Leaf)) { throw "Durable file snapshot is missing: $($state.Copy)" }
        $item = Get-Item -LiteralPath ([string]$state.Copy) -Force -ErrorAction Stop
        if ([long]$item.Length -ne [long]$state.Length -or
            -not [string]::Equals((Get-FileHash -LiteralPath ([string]$state.Copy) -Algorithm SHA256).Hash, [string]$state.Sha256, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Durable file snapshot hash mismatch: $($state.Copy)"
        }
    }
}

function Restore-DurableFileStates {
    param([object[]]$States)
    Assert-DurableFileStates -States $States
    foreach ($state in $States) {
        $path = [string]$state.Path
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $currentHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
            if (-not [string]::IsNullOrWhiteSpace([string]$state.ExpectedPostSha256) -and
                -not [string]::Equals($currentHash, [string]$state.ExpectedPostSha256, [StringComparison]::OrdinalIgnoreCase) -and
                -not ([bool]$state.Existed -and [string]::Equals($currentHash, [string]$state.Sha256, [StringComparison]::OrdinalIgnoreCase))) {
                throw "The rollback target changed after this transaction and was preserved: $path"
            }
        }
        if ([bool]$state.Existed) {
            $parent = Split-Path -Parent $path
            if (-not (Test-Path -LiteralPath $parent -PathType Container)) { New-Item -ItemType Directory -Path $parent -Force -ErrorAction Stop | Out-Null }
            Copy-Item -LiteralPath ([string]$state.Copy) -Destination $path -Force -ErrorAction Stop
            if (-not [string]::Equals((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash, [string]$state.Sha256, [StringComparison]::OrdinalIgnoreCase)) { throw "Restored file differs: $path" }
        } elseif (Test-Path -LiteralPath $path -PathType Leaf) {
            Remove-Item -LiteralPath $path -Force -ErrorAction Stop
        }
    }
}

function Restore-AppDataState {
    param($State, [Parameter(Mandatory = $true)][string]$TransactionDirectory)
    $target = Get-FullPath ([string]$State.Target)
    $allowedTarget = Get-FullPath (Join-Path $env:LOCALAPPDATA 'MichStartupMaster')
    if (-not (Test-PathsEqual $target $allowedTarget)) { throw "Refusing an AppData rollback outside the exact owned path: $target" }
    Assert-NoReparseTraversal -Path $target -StopDirectory (Get-FullPath $env:LOCALAPPDATA)

    if ([bool]$State.Existed) {
        if (-not (Test-Path -LiteralPath ([string]$State.Snapshot) -PathType Container)) { throw 'The durable AppData snapshot is missing.' }
        Assert-NoReparsePointsInTree -Directory ([string]$State.Snapshot) -Context 'Durable AppData snapshot'
        Assert-ManifestEqual -Expected @($State.Manifest) -Actual @(Get-DirectoryManifest ([string]$State.Snapshot)) -Context 'Durable AppData snapshot'
    }

    if (Test-Path -LiteralPath $target -PathType Container) {
        Assert-NoReparsePointsInTree -Directory $target -Context 'Current MichStartupMaster AppData'
        $observed = Join-Path $TransactionDirectory ('rollback-observed-appdata-' + [Guid]::NewGuid().ToString('N'))
        [void](Copy-DirectorySnapshot -Source $target -Destination $observed)
        Remove-Item -LiteralPath $target -Recurse -Force -ErrorAction Stop
    } elseif (Test-Path -LiteralPath $target) {
        throw "The owned AppData target is not a directory and was preserved: $target"
    }

    if ([bool]$State.Existed) {
        [void](Copy-DirectorySnapshot -Source ([string]$State.Snapshot) -Destination $target)
        Assert-ManifestEqual -Expected @($State.Manifest) -Actual @(Get-DirectoryManifest $target) -Context 'Restored AppData'
    }
}

function Test-OwnedAgentLaunch {
    param([string]$TargetPath, [string]$Arguments, [string]$Executable, [string]$Launcher)
    $argumentsText = ([string]$Arguments).Trim()
    if ((Test-PathsEqual $TargetPath $Executable) -and [string]::Equals($argumentsText, '--agent', [StringComparison]::OrdinalIgnoreCase)) { return $true }
    $expandedTarget = [Environment]::ExpandEnvironmentVariables(([string]$TargetPath).Trim())
    $allowedScriptHosts = @(
        (Join-Path $env:SystemRoot 'System32\wscript.exe'),
        (Join-Path $env:SystemRoot 'System32\cscript.exe'),
        (Join-Path $env:SystemRoot 'SysWOW64\wscript.exe'),
        (Join-Path $env:SystemRoot 'SysWOW64\cscript.exe')
    )
    if (@($allowedScriptHosts | Where-Object { Test-PathsEqual $_ $expandedTarget }).Count -ne 1) { return $false }
    foreach ($allowed in @('"' + $Launcher + '"', $Launcher, '//B //NoLogo "' + $Launcher + '"', '//B "' + $Launcher + '"')) {
        if ([string]::Equals($argumentsText, $allowed, [StringComparison]::OrdinalIgnoreCase)) { return $true }
    }
    return $false
}

function Get-ReservedTaskState {
    param([Parameter(Mandatory = $true)][string]$Executable, [Parameter(Mandatory = $true)][string]$Launcher)
    $scheduler = New-Object -ComObject 'Schedule.Service'
    $scheduler.Connect()
    $folder = $null
    try { $folder = $scheduler.GetFolder('\MichStartupMaster') } catch { if ($_.Exception.Message -match 'cannot find|not exist|0x80070002') { return [pscustomobject]@{ Existed = $false } }; throw }
    $task = $null
    try { $task = $folder.GetTask('MichStartupMasterApp') } catch { if ($_.Exception.Message -match 'cannot find|not exist|0x80070002') { return [pscustomobject]@{ Existed = $false } }; throw }
    $actions = $task.Definition.Actions
    if ($actions.Count -ne 1) { throw 'The reserved task has multiple actions and was preserved.' }
    $action = $actions.Item(1)
    if (-not (Test-OwnedAgentLaunch -TargetPath ([string]$action.Path) -Arguments ([string]$action.Arguments) -Executable $Executable -Launcher $Launcher)) {
        throw 'The reserved task is owned by another command and was preserved.'
    }
    return [pscustomobject][ordered]@{
        Existed = $true
        Xml = [string]$task.Xml
        XmlSha256 = Get-StringSha256 ([string]$task.Xml)
        UserId = [string]$task.Definition.Principal.UserId
        LogonType = [int]$task.Definition.Principal.LogonType
        SecurityDescriptor = [string]$task.GetSecurityDescriptor(7)
        SecurityDescriptorSha256 = Get-StringSha256 ([string]$task.GetSecurityDescriptor(7))
    }
}

function Restore-ReservedTaskState {
    param($State, [string]$Executable, [string]$Launcher)
    $scheduler = New-Object -ComObject 'Schedule.Service'; $scheduler.Connect(); $rootFolder = $scheduler.GetFolder('\')
    try { $folder = $scheduler.GetFolder('\MichStartupMaster') } catch { $folder = $rootFolder.CreateFolder('MichStartupMaster', '') }
    if ([bool]$State.Existed) {
        if (-not [string]::Equals((Get-StringSha256 ([string]$State.Xml)), [string]$State.XmlSha256, [StringComparison]::OrdinalIgnoreCase) -or
            -not [string]::Equals((Get-StringSha256 ([string]$State.SecurityDescriptor)), [string]$State.SecurityDescriptorSha256, [StringComparison]::OrdinalIgnoreCase)) { throw 'The durable task XML/security snapshot is hash-invalid.' }
        try {
            $currentTask = $folder.GetTask('MichStartupMasterApp')
            $currentActions = $currentTask.Definition.Actions
            if ($currentActions.Count -ne 1) { throw 'A non-owned task appeared at the reserved path.' }
            $currentAction = $currentActions.Item(1)
            if (-not (Test-OwnedAgentLaunch -TargetPath ([string]$currentAction.Path) -Arguments ([string]$currentAction.Arguments) -Executable $Executable -Launcher $Launcher)) { throw 'A non-owned task appeared at the reserved path.' }
        } catch {
            if ($_.Exception.Message -notmatch 'cannot find|not exist|0x80070002') { throw }
        }
        $task = $folder.RegisterTask('MichStartupMasterApp', [string]$State.Xml, 6, [string]$State.UserId, '', [int]$State.LogonType, '')
        $task.SetSecurityDescriptor([string]$State.SecurityDescriptor, 16)
        if (-not [string]::Equals((Get-StringSha256 ([string]$task.Xml)), [string]$State.XmlSha256, [StringComparison]::OrdinalIgnoreCase) -or
            -not [string]::Equals((Get-StringSha256 ([string]$task.GetSecurityDescriptor(7))), [string]$State.SecurityDescriptorSha256, [StringComparison]::OrdinalIgnoreCase)) { throw 'Restored task XML/security differs.' }
    } else {
        try { $task = $folder.GetTask('MichStartupMasterApp') } catch { return }
        $actions = $task.Definition.Actions
        if ($actions.Count -ne 1) { throw 'A non-owned task appeared at the reserved path.' }
        $action = $actions.Item(1)
        if (-not (Test-OwnedAgentLaunch ([string]$action.Path) ([string]$action.Arguments) $Executable $Launcher)) { throw 'A non-owned task appeared at the reserved path.' }
        $folder.DeleteTask('MichStartupMasterApp', 0)
    }
}

function Get-OwnedShortcutStates {
    param([string]$Executable, [string]$Launcher, [string]$SnapshotDirectory)
    $shell = New-Object -ComObject WScript.Shell
    $rows = @(); $index = 0
    foreach ($path in @(
        (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Startup)) 'Mich Startup Master Agent.lnk'),
        (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::CommonStartup)) 'Mich Startup Master Agent.lnk')
    )) {
        if ([string]::IsNullOrWhiteSpace($path) -or -not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
        $shortcut = $shell.CreateShortcut($path)
        if (-not (Test-OwnedAgentLaunch ([string]$shortcut.TargetPath) ([string]$shortcut.Arguments) $Executable $Launcher)) { continue }
        $index++; $copy = Join-Path $SnapshotDirectory ('startup-shortcut-{0:D2}.lnk' -f $index)
        Copy-Item -LiteralPath $path -Destination $copy -Force -ErrorAction Stop
        $rows += [pscustomobject][ordered]@{ Path = Get-FullPath $path; Copy = $copy; Sha256 = (Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash; TargetPath = [string]$shortcut.TargetPath; Arguments = [string]$shortcut.Arguments }
    }
    return @($rows)
}

function Restore-OwnedShortcutStates {
    param([object[]]$States, [string]$Executable, [string]$Launcher)
    $expected = @{}; foreach ($row in $States) { $expected[(Get-FullPath ([string]$row.Path)).ToLowerInvariant()] = $row }
    $shell = New-Object -ComObject WScript.Shell
    foreach ($path in @(
        (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Startup)) 'Mich Startup Master Agent.lnk'),
        (Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::CommonStartup)) 'Mich Startup Master Agent.lnk')
    )) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
        $shortcut = $shell.CreateShortcut($path)
        if ((Test-OwnedAgentLaunch ([string]$shortcut.TargetPath) ([string]$shortcut.Arguments) $Executable $Launcher) -and -not $expected.ContainsKey((Get-FullPath $path).ToLowerInvariant())) { Remove-Item -LiteralPath $path -Force -ErrorAction Stop }
    }
    foreach ($row in $States) {
        if (-not (Test-Path -LiteralPath ([string]$row.Copy) -PathType Leaf) -or
            -not [string]::Equals((Get-FileHash -LiteralPath ([string]$row.Copy) -Algorithm SHA256).Hash, [string]$row.Sha256, [StringComparison]::OrdinalIgnoreCase)) { throw "Startup shortcut snapshot is missing or hash-invalid: $($row.Copy)" }
        if (Test-Path -LiteralPath ([string]$row.Path) -PathType Leaf) {
            $current = $shell.CreateShortcut([string]$row.Path)
            if (-not (Test-OwnedAgentLaunch -TargetPath ([string]$current.TargetPath) -Arguments ([string]$current.Arguments) -Executable $Executable -Launcher $Launcher)) { throw "A non-owned shortcut appeared at the reserved path and was preserved: $($row.Path)" }
        }
        Copy-Item -LiteralPath ([string]$row.Copy) -Destination ([string]$row.Path) -Force -ErrorAction Stop
        if (-not [string]::Equals((Get-FileHash -LiteralPath ([string]$row.Path) -Algorithm SHA256).Hash, [string]$row.Sha256, [StringComparison]::OrdinalIgnoreCase)) { throw "Restored Startup shortcut differs: $($row.Path)" }
    }
}

function Test-OwnedRegistryCommand {
    param([string]$Command, [string]$Executable, [string]$Launcher)
    $expanded = [Environment]::ExpandEnvironmentVariables(([string]$Command).Trim())
    foreach ($prefix in @('"' + $Executable + '"', $Executable)) {
        if ($expanded.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -and [string]::Equals($expanded.Substring($prefix.Length).Trim(), '--agent', [StringComparison]::OrdinalIgnoreCase)) { return $true }
    }
    foreach ($host in @(
        (Join-Path $env:SystemRoot 'System32\wscript.exe'),
        (Join-Path $env:SystemRoot 'System32\cscript.exe'),
        (Join-Path $env:SystemRoot 'SysWOW64\wscript.exe'),
        (Join-Path $env:SystemRoot 'SysWOW64\cscript.exe')
    )) {
        foreach ($prefix in @('"' + $host + '"', $host)) {
            if (-not $expanded.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { continue }
            $rest = $expanded.Substring($prefix.Length).Trim()
            foreach ($allowed in @('"' + $Launcher + '"', $Launcher, '//B //NoLogo "' + $Launcher + '"', '//B "' + $Launcher + '"')) { if ([string]::Equals($rest, $allowed, [StringComparison]::OrdinalIgnoreCase)) { return $true } }
        }
    }
    return $false
}

function Get-OwnedRegistryRows {
    param([string]$Executable, [string]$Launcher)
    $rows = @()
    foreach ($viewName in @('Registry64', 'Registry32')) {
        $view = [Enum]::Parse([Microsoft.Win32.RegistryView], $viewName)
        $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, $view)
        try {
            foreach ($subKey in @('Software\Microsoft\Windows\CurrentVersion\Run', 'Software\Microsoft\Windows\CurrentVersion\RunOnce')) {
                $key = $baseKey.OpenSubKey($subKey, $false)
                if ($null -eq $key) { continue }
                try {
                    foreach ($name in $key.GetValueNames()) {
                        $value = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                        if ($null -ne $value -and (Test-OwnedRegistryCommand -Command ([string]$value) -Executable $Executable -Launcher $Launcher)) {
                            $serialized = [Management.Automation.PSSerializer]::Serialize($value)
                            $rows += [pscustomobject][ordered]@{
                                View = $viewName
                                SubKey = $subKey
                                Name = [string]$name
                                Kind = [string]$key.GetValueKind($name)
                                DataBase64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($serialized))
                                DataSha256 = Get-StringSha256 $serialized
                            }
                        }
                    }
                } finally { $key.Dispose() }
            }
        } finally { $baseKey.Dispose() }
    }
    return @($rows | Sort-Object View,SubKey,Name)
}

function Restore-OwnedRegistryRows {
    param([object[]]$Rows, [string]$Executable, [string]$Launcher)
    $expected = @{}; foreach ($row in $Rows) { $expected[(([string]$row.View)+'|'+([string]$row.SubKey)+'|'+([string]$row.Name)).ToLowerInvariant()] = $row }
    foreach ($current in @(Get-OwnedRegistryRows $Executable $Launcher)) {
        $identity = (([string]$current.View)+'|'+([string]$current.SubKey)+'|'+([string]$current.Name)).ToLowerInvariant(); if ($expected.ContainsKey($identity)) { continue }
        $view = [Enum]::Parse([Microsoft.Win32.RegistryView], [string]$current.View); $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, $view)
        try { $key = $baseKey.OpenSubKey([string]$current.SubKey, $true); if ($null -ne $key) { try { $key.DeleteValue([string]$current.Name, $false) } finally { $key.Dispose() } } } finally { $baseKey.Dispose() }
    }
    foreach ($row in $Rows) {
        $serialized = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String([string]$row.DataBase64)); if (-not [string]::Equals((Get-StringSha256 $serialized), [string]$row.DataSha256, [StringComparison]::OrdinalIgnoreCase)) { throw 'Registry snapshot payload hash mismatch.' }
        $data = [Management.Automation.PSSerializer]::Deserialize($serialized); $view = [Enum]::Parse([Microsoft.Win32.RegistryView], [string]$row.View); $kind = [Enum]::Parse([Microsoft.Win32.RegistryValueKind], [string]$row.Kind); $baseKey = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, $view)
        try {
            $key = $baseKey.CreateSubKey([string]$row.SubKey, $true)
            try {
                $currentData = $key.GetValue([string]$row.Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                if ($null -ne $currentData -and -not (Test-OwnedRegistryCommand -Command ([string]$currentData) -Executable $Executable -Launcher $Launcher)) { throw "A non-owned registry value appeared and was preserved: $($row.SubKey)\$($row.Name)" }
                $key.SetValue([string]$row.Name, $data, $kind)
            } finally { $key.Dispose() }
        } finally { $baseKey.Dispose() }
    }
}

function Invoke-AppCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Executable,

        [Parameter(Mandatory = $true)]
        [ValidateSet('--register-agent', '--verify-agent')]
        [string]$Command,

        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9A-Fa-f]{64}$')]
        [string]$ReleaseChildToken,

        [ValidateRange(1000, 300000)]
        [int]$TimeoutMilliseconds = 120000
    )

    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Executable
    $startInfo.Arguments = $Command + ' --release-gate-child'
    $startInfo.WorkingDirectory = Split-Path -Path $Executable -Parent
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.EnvironmentVariables['MICH_STARTUP_MASTER_RELEASE_CHILD_TOKEN'] = $ReleaseChildToken
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    $processStarted = $false
    $commandCompleted = $false
    try {
        if (-not $process.Start()) { throw "Could not start MichStartupMaster $Command." }
        $processStarted = $true
        $processId = [int]$process.Id
        $startTimeUtcFileTime = [long]$process.StartTime.ToUniversalTime().ToFileTimeUtc()
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutMilliseconds)) {
            $killError = $null
            try { $process.Kill() } catch { $killError = $_.Exception.Message }
            $terminationConfirmed = $false
            try { $terminationConfirmed = $process.WaitForExit(5000) } catch { if ($null -eq $killError) { $killError = $_.Exception.Message } }
            $terminationState = if ($terminationConfirmed) { 'confirmed' } else { 'NOT confirmed' }
            $killDetail = if ([string]::IsNullOrWhiteSpace($killError)) { '' } else { " Kill error: $killError" }
            throw "MichStartupMaster $Command timed out after $TimeoutMilliseconds ms; exact child PID=$processId startUtcFileTime=$startTimeUtcFileTime termination=$terminationState.$killDetail Redirected streams were deliberately not awaited after the timeout."
        }
        # A descendant could inherit a redirected handle even after this exact child
        # exits. Bound the reader drain as well as the process wait so no failure path
        # can block forever.
        $readerTasks = [Threading.Tasks.Task[]]@($stdoutTask, $stderrTask)
        if (-not [Threading.Tasks.Task]::WaitAll($readerTasks, 5000)) {
            throw "MichStartupMaster $Command exited, but redirected output did not close within 5000 ms; PID=$processId startUtcFileTime=$startTimeUtcFileTime."
        }
        $stdout = [string]$stdoutTask.GetAwaiter().GetResult()
        $stderr = [string]$stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw "MichStartupMaster $Command failed with exit $($process.ExitCode); PID=$processId startUtcFileTime=$startTimeUtcFileTime. stdout: $stdout stderr: $stderr"
        }
        $result = [pscustomobject][ordered]@{
            Command = $Command
            ProcessId = $processId
            StartTimeUtcFileTime = $startTimeUtcFileTime
            ExitCode = [int]$process.ExitCode
            TimeoutMilliseconds = $TimeoutMilliseconds
            Stdout = $stdout.Trim()
            Stderr = $stderr.Trim()
            ReleaseGateChildRequested = $true
            ReleaseChildProofEventNameSha256 = Get-StringSha256 ('Local\MichStartupMaster.ReleaseChild.' + $ReleaseChildToken)
        }
        $commandCompleted = $true
        return $result
    } finally {
        if ($processStarted -and -not $commandCompleted) {
            try {
                if (-not $process.HasExited) {
                    $process.Kill()
                    [void]$process.WaitForExit(5000)
                }
            } catch { }
        }
        $process.Dispose()
    }
}

function New-ReleaseChildToken {
    $bytes = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return ([BitConverter]::ToString($bytes)).Replace('-', '')
}

function New-ReleaseChildProofEvent {
    param(
        [Parameter(Mandatory = $true)]
        [ValidatePattern('^[0-9A-Fa-f]{64}$')]
        [string]$Token
    )

    $eventName = 'Local\MichStartupMaster.ReleaseChild.' + $Token
    $createdNew = $false
    $eventHandle = New-Object Threading.EventWaitHandle($false, [Threading.EventResetMode]::ManualReset, $eventName, ([ref]$createdNew))
    if (-not $createdNew) {
        $eventHandle.Dispose()
        throw 'The one-shot authenticated release-child proof event already exists.'
    }
    return $eventHandle
}

$exe = Join-Path $InstallDirectory 'MichStartupMaster.exe'
$sourceIcon = Join-Path $PSScriptRoot 'assets\MichStartupMaster.ico'
foreach ($requiredPath in @($exe, $sourceIcon)) {
    if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) { throw "Required path was not found: $requiredPath" }
}

$projectRoot = Get-FullPath $PSScriptRoot
$resolvedExe = (Resolve-Path -LiteralPath $exe -ErrorAction Stop).ProviderPath
$resolvedInstallDirectory = Get-FullPath (Split-Path -Parent $resolvedExe)
$resolvedSourceIcon = (Resolve-Path -LiteralPath $sourceIcon -ErrorAction Stop).ProviderPath
$runtimeIcon = Join-Path $resolvedInstallDirectory 'MichStartupMaster.ico'
$taskbarIcon = Join-Path $resolvedInstallDirectory 'MichStartupMaster-taskbar-v3.ico'
$launcher = Join-Path $resolvedInstallDirectory 'MichStartupMasterAgent.vbs'
$programsDirectory = Get-FullPath (Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs')
$shortcutPath = Join-Path $programsDirectory 'Mich Startup Master.lnk'
$appDataPath = Get-FullPath (Join-Path $env:LOCALAPPDATA 'MichStartupMaster')
$artifactRoot = Join-Path $projectRoot 'artifacts\runtime-output'
$transactionId = 'local-install-{0}-{1}' -f ([DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')), ([Guid]::NewGuid().ToString('N'))
$transactionDirectory = Join-Path $artifactRoot $transactionId
$externalStatePath = Join-Path $transactionDirectory 'external-state.json'
$markerPath = Join-Path $transactionDirectory 'transaction-marker.json'
$receiptPath = Join-Path $transactionDirectory 'transaction-receipt.json'

if (-not (Test-Path -LiteralPath $artifactRoot -PathType Container)) { New-Item -ItemType Directory -Path $artifactRoot -Force -ErrorAction Stop | Out-Null }
Assert-NoReparseTraversal -Path $artifactRoot -StopDirectory $projectRoot
New-Item -ItemType Directory -Path $transactionDirectory -ErrorAction Stop | Out-Null
Assert-NoReparseTraversal -Path $transactionDirectory -StopDirectory $projectRoot
Assert-NoReparseTraversal -Path $appDataPath -StopDirectory (Get-FullPath $env:LOCALAPPDATA)

$deploymentMutex = New-Object Threading.Mutex($false, 'Local\MichStartupMaster.ReleaseDeployment')
$mutationMutex = New-Object Threading.Mutex($false, 'Local\MichStartupMaster.ManagedStartupMutation')
$deploymentMutexHeld = $false
$mutationMutexHeld = $false
$mutationStarted = $false
$externalStateHash = $null
$fileStates = @()
$taskState = $null
$startupShortcutStates = @()
$registryRows = @()
$appDataState = $null
$registrationReceipt = $null
$verificationReceipt = $null
$iconSize = $null
$transactionSucceeded = $false
$releaseChildEvent = $null
$releaseChildToken = $null

try {
    Enter-Mutex -Mutex $deploymentMutex -Name 'the release deployment mutex'
    $deploymentMutexHeld = $true
    Enter-Mutex -Mutex $mutationMutex -Name 'the app-wide external-state mutation mutex'
    $mutationMutexHeld = $true

    if (Test-Path -LiteralPath $shortcutPath -PathType Leaf) {
        $inspectShell = New-Object -ComObject WScript.Shell
        $inspectShortcut = $null
        try {
            $inspectShortcut = $inspectShell.CreateShortcut($shortcutPath)
            $existingTarget = [string]$inspectShortcut.TargetPath
            $existingArguments = ([string]$inspectShortcut.Arguments).Trim()
            if (-not (Test-PathsEqual $existingTarget $resolvedExe) -or -not [string]::IsNullOrWhiteSpace($existingArguments)) { throw "The Start-menu shortcut path is occupied by another command and was preserved: $shortcutPath" }
        } finally {
            if ($null -ne $inspectShortcut) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($inspectShortcut) }
            if ($null -ne $inspectShell) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($inspectShell) }
        }
    }

    $preparedShortcut = Join-Path $transactionDirectory 'prepared-start-menu-shortcut.lnk'
    $prepareShell = New-Object -ComObject WScript.Shell
    $prepareLink = $null
    try {
        $prepareLink = $prepareShell.CreateShortcut($preparedShortcut)
        $prepareLink.TargetPath = $resolvedExe
        $prepareLink.Arguments = ''
        $prepareLink.WorkingDirectory = $resolvedInstallDirectory
        $prepareLink.IconLocation = "$taskbarIcon,0"
        $prepareLink.Description = 'Mich Startup Master'
        $prepareLink.Save()
    } finally {
        if ($null -ne $prepareLink) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($prepareLink) }
        if ($null -ne $prepareShell) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($prepareShell) }
    }

    $fileStates = @(New-DurableFileStates -Paths @($runtimeIcon, $taskbarIcon, $shortcutPath) -SnapshotDirectory (Join-Path $transactionDirectory 'file-snapshots'))
    $sourceIconHash = (Get-FileHash -LiteralPath $resolvedSourceIcon -Algorithm SHA256).Hash
    $fileStates[0].ExpectedPostSha256 = $sourceIconHash
    $fileStates[1].ExpectedPostSha256 = $sourceIconHash
    $fileStates[2].ExpectedPostSha256 = (Get-FileHash -LiteralPath $preparedShortcut -Algorithm SHA256).Hash

    $appDataState = [pscustomobject][ordered]@{ Target = $appDataPath; Existed = $false; Snapshot = $null; Manifest = @() }
    if (Test-Path -LiteralPath $appDataPath -PathType Container) {
        Assert-NoReparsePointsInTree -Directory $appDataPath -Context 'MichStartupMaster AppData snapshot source'
        $appDataState.Existed = $true
        $appDataState.Snapshot = Join-Path $transactionDirectory 'appdata-snapshot'
        $appDataState.Manifest = @(Copy-DirectorySnapshot -Source $appDataPath -Destination ([string]$appDataState.Snapshot))
    } elseif (Test-Path -LiteralPath $appDataPath) {
        throw "The exact owned AppData path is not a directory and was preserved: $appDataPath"
    }

    $taskState = Get-ReservedTaskState -Executable $resolvedExe -Launcher $launcher
    $startupShortcutStates = @(Get-OwnedShortcutStates -Executable $resolvedExe -Launcher $launcher -SnapshotDirectory $transactionDirectory)
    $registryRows = @(Get-OwnedRegistryRows -Executable $resolvedExe -Launcher $launcher)
    Assert-DurableFileStates -States $fileStates

    $externalState = [pscustomobject][ordered]@{
        SchemaVersion = 1
        TransactionId = $transactionId
        TransactionDirectory = $transactionDirectory
        CreatedUtc = [DateTime]::UtcNow.ToString('o')
        ProjectRoot = $projectRoot
        ScriptPath = $MyInvocation.MyCommand.Path
        ScriptSha256 = (Get-FileHash -LiteralPath $MyInvocation.MyCommand.Path -Algorithm SHA256).Hash
        Executable = $resolvedExe
        ExecutableSha256 = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash
        SourceIcon = $resolvedSourceIcon
        SourceIconSha256 = $sourceIconHash
        FileStates = @($fileStates)
        AppData = $appDataState
        Task = $taskState
        StartupShortcuts = @($startupShortcutStates)
        RegistryRows = @($registryRows)
    }
    Write-JsonAtomic -Path $externalStatePath -Value $externalState
    $externalStateHash = (Get-FileHash -LiteralPath $externalStatePath -Algorithm SHA256).Hash
    Write-JsonAtomic -Path $markerPath -Value ([pscustomobject][ordered]@{
        SchemaVersion = 1; TransactionId = $transactionId; State = 'Prepared'; UpdatedUtc = [DateTime]::UtcNow.ToString('o')
        ExternalStatePath = $externalStatePath; ExternalStateSha256 = $externalStateHash
        DeploymentMutex = 'Local\MichStartupMaster.ReleaseDeployment'; MutationMutex = 'Local\MichStartupMaster.ManagedStartupMutation'
        ReceiptPath = $null; ReceiptSha256 = $null
    })
    $mutationStarted = $true

    Copy-Item -LiteralPath $resolvedSourceIcon -Destination $runtimeIcon -Force -ErrorAction Stop
    Copy-Item -LiteralPath $resolvedSourceIcon -Destination $taskbarIcon -Force -ErrorAction Stop
    Copy-Item -LiteralPath $preparedShortcut -Destination $shortcutPath -Force -ErrorAction Stop
    foreach ($state in $fileStates) {
        if (-not [string]::Equals((Get-FileHash -LiteralPath ([string]$state.Path) -Algorithm SHA256).Hash, [string]$state.ExpectedPostSha256, [StringComparison]::OrdinalIgnoreCase)) { throw "Installed file differs from its prepared transaction payload: $($state.Path)" }
    }

    $icon = New-Object System.Drawing.Icon $taskbarIcon
    try { $iconSize = "$($icon.Width)x$($icon.Height)" } finally { $icon.Dispose() }

    # Hold the release deployment gate continuously while an authenticated child
    # bypasses only that gate and acquires the shared mutation mutex itself. Normal
    # app actors queue at the release gate, so the parent-to-child handoff cannot
    # expose the durable snapshot to an unrelated mutation.
    $releaseChildToken = New-ReleaseChildToken
    $releaseChildEvent = New-ReleaseChildProofEvent -Token $releaseChildToken
    try {
        $mutationMutex.ReleaseMutex(); $mutationMutexHeld = $false
        $registrationReceipt = Invoke-AppCommand -Executable $resolvedExe -Command '--register-agent' -ReleaseChildToken $releaseChildToken
    } finally {
        if (-not $mutationMutexHeld) {
            Enter-Mutex -Mutex $mutationMutex -Name 'the app-wide external-state mutation mutex'
            $mutationMutexHeld = $true
        }
        if ($null -ne $releaseChildEvent) { $releaseChildEvent.Dispose(); $releaseChildEvent = $null }
        $releaseChildToken = $null
    }

    # Verification gets a fresh one-shot proof event and token. The deployment gate
    # remains held continuously across both independently authenticated children.
    $releaseChildToken = New-ReleaseChildToken
    $releaseChildEvent = New-ReleaseChildProofEvent -Token $releaseChildToken
    try {
        $mutationMutex.ReleaseMutex(); $mutationMutexHeld = $false
        $verificationReceipt = Invoke-AppCommand -Executable $resolvedExe -Command '--verify-agent' -ReleaseChildToken $releaseChildToken
    } finally {
        if (-not $mutationMutexHeld) {
            Enter-Mutex -Mutex $mutationMutex -Name 'the app-wide external-state mutation mutex'
            $mutationMutexHeld = $true
        }
        if ($null -ne $releaseChildEvent) { $releaseChildEvent.Dispose(); $releaseChildEvent = $null }
        $releaseChildToken = $null
    }

    if (-not [string]::Equals((Get-FileHash -LiteralPath $externalStatePath -Algorithm SHA256).Hash, $externalStateHash, [StringComparison]::OrdinalIgnoreCase)) { throw 'The immutable external-state snapshot changed before commit.' }
    $receipt = [pscustomobject][ordered]@{
        SchemaVersion = 1; TransactionId = $transactionId; Result = 'Committed'; CompletedUtc = [DateTime]::UtcNow.ToString('o')
        ExternalStatePath = $externalStatePath; ExternalStateSha256 = $externalStateHash
        RegistrationReceipt = $registrationReceipt; VerificationReceipt = $verificationReceipt; RollbackPhases = @()
    }
    Write-JsonAtomic -Path $receiptPath -Value $receipt
    $receiptHash = (Get-FileHash -LiteralPath $receiptPath -Algorithm SHA256).Hash
    Write-JsonAtomic -Path $markerPath -Value ([pscustomobject][ordered]@{
        SchemaVersion = 1; TransactionId = $transactionId; State = 'Committed'; UpdatedUtc = [DateTime]::UtcNow.ToString('o')
        ExternalStatePath = $externalStatePath; ExternalStateSha256 = $externalStateHash
        DeploymentMutex = 'Local\MichStartupMaster.ReleaseDeployment'; MutationMutex = 'Local\MichStartupMaster.ManagedStartupMutation'
        ReceiptPath = $receiptPath; ReceiptSha256 = $receiptHash
    })
    $transactionSucceeded = $true
} catch {
    $installError = $_
    $rollbackResults = @()
    if ($mutationStarted) {
        if (-not $mutationMutexHeld) {
            try { Enter-Mutex -Mutex $mutationMutex -Name 'the app-wide external-state mutation mutex'; $mutationMutexHeld = $true }
            catch { $rollbackResults += [pscustomobject]@{ Phase = 'AcquireMutationMutex'; Success = $false; Error = $_.Exception.Message } }
        }
        $snapshotAuthorized = $false
        try {
            if (-not (Test-Path -LiteralPath $externalStatePath -PathType Leaf)) { throw 'The durable external-state snapshot is missing.' }
            if (-not [string]::Equals((Get-FileHash -LiteralPath $externalStatePath -Algorithm SHA256).Hash, $externalStateHash, [StringComparison]::OrdinalIgnoreCase)) { throw 'The durable external-state snapshot hash does not match its prepared marker.' }
            $snapshotAuthorized = $true
        } catch { $rollbackResults += [pscustomobject]@{ Phase = 'AuthorizeSnapshot'; Success = $false; Error = $_.Exception.Message } }

        if ($snapshotAuthorized -and $mutationMutexHeld) {
            foreach ($phase in @(
                [pscustomobject]@{ Name = 'Files'; Action = { Restore-DurableFileStates -States $fileStates } },
                [pscustomobject]@{ Name = 'Task'; Action = { Restore-ReservedTaskState -State $taskState -Executable $resolvedExe -Launcher $launcher } },
                [pscustomobject]@{ Name = 'StartupShortcuts'; Action = { Restore-OwnedShortcutStates -States $startupShortcutStates -Executable $resolvedExe -Launcher $launcher } },
                [pscustomobject]@{ Name = 'Registry'; Action = { Restore-OwnedRegistryRows -Rows $registryRows -Executable $resolvedExe -Launcher $launcher } },
                [pscustomobject]@{ Name = 'AppData'; Action = { Restore-AppDataState -State $appDataState -TransactionDirectory $transactionDirectory } }
            )) {
                try { & $phase.Action; $rollbackResults += [pscustomobject]@{ Phase = $phase.Name; Success = $true; Error = $null } }
                catch { $rollbackResults += [pscustomobject]@{ Phase = $phase.Name; Success = $false; Error = $_.Exception.Message } }
            }
        }
    }

    $failedPhases = @($rollbackResults | Where-Object { -not $_.Success })
    $resultName = if ($mutationStarted -and $failedPhases.Count -eq 0) { 'RolledBack' } elseif ($mutationStarted) { 'RollbackIncomplete' } else { 'FailedBeforeMutation' }
    try {
        $failureReceipt = [pscustomobject][ordered]@{
            SchemaVersion = 1; TransactionId = $transactionId; Result = $resultName; CompletedUtc = [DateTime]::UtcNow.ToString('o')
            ExternalStatePath = $externalStatePath; ExternalStateSha256 = $externalStateHash
            Error = $installError.Exception.Message; RegistrationReceipt = $registrationReceipt; VerificationReceipt = $verificationReceipt
            RollbackPhases = @($rollbackResults)
        }
        Write-JsonAtomic -Path $receiptPath -Value $failureReceipt
        $failureReceiptHash = (Get-FileHash -LiteralPath $receiptPath -Algorithm SHA256).Hash
        Write-JsonAtomic -Path $markerPath -Value ([pscustomobject][ordered]@{
            SchemaVersion = 1; TransactionId = $transactionId; State = $resultName; UpdatedUtc = [DateTime]::UtcNow.ToString('o')
            ExternalStatePath = $externalStatePath; ExternalStateSha256 = $externalStateHash
            DeploymentMutex = 'Local\MichStartupMaster.ReleaseDeployment'; MutationMutex = 'Local\MichStartupMaster.ManagedStartupMutation'
            ReceiptPath = $receiptPath; ReceiptSha256 = $failureReceiptHash
        })
    } catch {
        $failedPhases += [pscustomobject]@{ Phase = 'WriteFailureReceipt'; Success = $false; Error = $_.Exception.Message }
    }
    if ($failedPhases.Count -gt 0) { throw "Local installation failed: $($installError.Exception.Message). Durable rollback was incomplete: $((@($failedPhases | ForEach-Object { $_.Phase + ': ' + $_.Error })) -join ' | '). Transaction: $transactionDirectory" }
    if (-not $mutationStarted) { throw "Local installation failed before any live path was changed: $($installError.Exception.Message). Transaction: $transactionDirectory" }
    throw "Local installation failed and the durable external-state snapshot was restored: $($installError.Exception.Message). Transaction: $transactionDirectory"
} finally {
    if ($null -ne $releaseChildEvent) { $releaseChildEvent.Dispose() }
    if ($mutationMutexHeld) { try { $mutationMutex.ReleaseMutex() } catch { } }
    if ($deploymentMutexHeld) { try { $deploymentMutex.ReleaseMutex() } catch { } }
    if ($null -ne $mutationMutex) { $mutationMutex.Dispose() }
    if ($null -ne $deploymentMutex) { $deploymentMutex.Dispose() }
}

if (-not $transactionSucceeded) { throw 'The local installation transaction did not commit.' }
[pscustomobject]@{
    InstallDirectory = $resolvedInstallDirectory
    Executable = $resolvedExe
    StartMenuShortcut = $shortcutPath
    IconLocation = "$taskbarIcon,0"
    IconSize = $iconSize
    StartupRoute = '\MichStartupMaster\MichStartupMasterApp'
    RegistrationReceipt = $registrationReceipt
    VerificationReceipt = $verificationReceipt
    TransactionDirectory = $transactionDirectory
    TransactionMarker = $markerPath
    TransactionReceipt = $receiptPath
} | Format-List
