[CmdletBinding()]
param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot 'build\MichStartupMaster.exe'),
    [int[]]$ProcessId
)

$ErrorActionPreference = 'Stop'

if (-not ('MichStartupMasterScriptNativeProcess' -as [type])) {
    Add-Type @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
public sealed class MichStartupMasterScriptProcessSnapshot { public int ProcessId; public string ImagePath; public string CommandLine; public long StartTimeUtcFileTime; }
public static class MichStartupMasterScriptNativeProcess
{
    private const uint PROCESS_TERMINATE = 0x0001, PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
    private const int ProcessCommandLineInformation = 60;
    [StructLayout(LayoutKind.Sequential)] private struct FILETIME { public uint Low; public uint High; }
    [StructLayout(LayoutKind.Sequential)] private struct UNICODE_STRING { public ushort Length; public ushort MaximumLength; public IntPtr Buffer; }
    [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr OpenProcess(uint access, bool inherit, int processId);
    [DllImport("kernel32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool QueryFullProcessImageName(IntPtr process, uint flags, StringBuilder path, ref uint size);
    [DllImport("kernel32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GetProcessTimes(IntPtr process, out FILETIME creation, out FILETIME exit, out FILETIME kernel, out FILETIME user);
    [DllImport("kernel32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool TerminateProcess(IntPtr process, uint exitCode);
    [DllImport("kernel32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("ntdll.dll")] private static extern int NtQueryInformationProcess(IntPtr process, int infoClass, IntPtr info, int length, out int returned);
    private static bool TryIdentity(IntPtr process, out string imagePath, out long startTime, out string error)
    {
        imagePath = ""; startTime = 0; error = ""; uint length = 32768; StringBuilder path = new StringBuilder((int)length);
        if (!QueryFullProcessImageName(process, 0, path, ref length)) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
        FILETIME creation, exit, kernel, user;
        if (!GetProcessTimes(process, out creation, out exit, out kernel, out user)) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
        imagePath = path.ToString(); startTime = ((long)creation.High << 32) | creation.Low; return true;
    }
    public static bool TrySnapshot(int processId, out MichStartupMasterScriptProcessSnapshot snapshot, out string error)
    {
        snapshot = null; error = ""; IntPtr process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, processId);
        if (process == IntPtr.Zero) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
        IntPtr buffer = IntPtr.Zero;
        try {
            string path; long start; if (!TryIdentity(process, out path, out start, out error)) return false;
            int required; NtQueryInformationProcess(process, ProcessCommandLineInformation, IntPtr.Zero, 0, out required);
            if (required <= 0 || required > 1048576) { error = "invalid command-line buffer length " + required; return false; }
            buffer = Marshal.AllocHGlobal(required); int status = NtQueryInformationProcess(process, ProcessCommandLineInformation, buffer, required, out required);
            if (status < 0) { error = "NtQueryInformationProcess failed with NTSTATUS 0x" + status.ToString("X8"); return false; }
            UNICODE_STRING command = (UNICODE_STRING)Marshal.PtrToStructure(buffer, typeof(UNICODE_STRING));
            snapshot = new MichStartupMasterScriptProcessSnapshot(); snapshot.ProcessId = processId; snapshot.ImagePath = path; snapshot.StartTimeUtcFileTime = start;
            snapshot.CommandLine = command.Length == 0 ? "" : (Marshal.PtrToStringUni(command.Buffer, command.Length / 2) ?? ""); return true;
        } finally { if (buffer != IntPtr.Zero) Marshal.FreeHGlobal(buffer); CloseHandle(process); }
    }
    public static bool TerminateExact(int processId, string expectedPath, long expectedStart, out string error)
    {
        error = ""; IntPtr process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE, false, processId);
        if (process == IntPtr.Zero) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
        try {
            string path; long start; if (!TryIdentity(process, out path, out start, out error)) return false;
            if (!String.Equals(Path.GetFullPath(path), Path.GetFullPath(expectedPath), StringComparison.OrdinalIgnoreCase) || start != expectedStart) { error = "process generation no longer matches"; return false; }
            if (!TerminateProcess(process, 1)) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; } return true;
        } finally { CloseHandle(process); }
    }
}
'@
}

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

function Get-NativeSnapshot {
    param([Parameter(Mandatory = $true)][int]$Id)
    if ($null -eq (Get-Process -Id $Id -ErrorAction SilentlyContinue)) { return $null }
    [MichStartupMasterScriptProcessSnapshot]$snapshot = $null
    $detail = ''
    if (-not [MichStartupMasterScriptNativeProcess]::TrySnapshot($Id, [ref]$snapshot, [ref]$detail)) {
        if ($null -eq (Get-Process -Id $Id -ErrorAction SilentlyContinue)) { return $null }
        throw "Could not inspect PID ${Id} without WMI: $detail"
    }
    return $snapshot
}

function Test-SameGeneration {
    param($Expected, $Actual, [Parameter(Mandatory = $true)][string]$ExpectedPath)
    return $null -ne $Actual -and
        (Test-ExactPath -Candidate $Actual.ImagePath -Expected $ExpectedPath) -and
        [long]$Actual.StartTimeUtcFileTime -eq [long]$Expected.StartTimeUtcFileTime
}

if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "MichStartupMaster executable was not found: $ExecutablePath"
}

$resolvedExe = (Resolve-Path -LiteralPath $ExecutablePath -ErrorAction Stop).ProviderPath
$processIdWasSpecified = $PSBoundParameters.ContainsKey('ProcessId')
if ($processIdWasSpecified) {
    $invalidRequestedIds = @($ProcessId | Where-Object { $_ -le 0 })
    if ($null -eq $ProcessId -or $ProcessId.Count -eq 0 -or $invalidRequestedIds.Count -gt 0) {
        throw 'When -ProcessId is supplied, every requested process ID must be a positive integer.'
    }
    $requestedIds = @($ProcessId | Select-Object -Unique)
} else {
    $requestedIds = @()
}
$exactProcesses = @(
    foreach ($process in @(Get-Process -Name 'MichStartupMaster' -ErrorAction SilentlyContinue)) {
        $snapshot = Get-NativeSnapshot -Id ([int]$process.Id)
        if ($null -ne $snapshot -and (Test-ExactPath -Candidate $snapshot.ImagePath -Expected $resolvedExe)) { $snapshot }
    }
)

if ($requestedIds.Count -gt 0) {
    $exactIds = @($exactProcesses | ForEach-Object { [int]$_.ProcessId })
    $invalidIds = @($requestedIds | Where-Object { $exactIds -notcontains $_ })
    if ($invalidIds.Count -gt 0) {
        throw "Requested process IDs do not belong to the exact executable '$resolvedExe': $($invalidIds -join ', ')"
    }
    $exactProcesses = @($exactProcesses | Where-Object { $requestedIds -contains [int]$_.ProcessId })
}

foreach ($processRow in $exactProcesses) {
    $currentRow = Get-NativeSnapshot -Id ([int]$processRow.ProcessId)
    if (-not (Test-SameGeneration -Expected $processRow -Actual $currentRow -ExpectedPath $resolvedExe)) {
        continue
    }

    $runtimeProcess = Get-Process -Id $currentRow.ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $runtimeProcess) {
        continue
    }
    try {
        $currentRow = Get-NativeSnapshot -Id ([int]$processRow.ProcessId)
        if (-not (Test-SameGeneration -Expected $processRow -Actual $currentRow -ExpectedPath $resolvedExe)) { continue }
        [void]$runtimeProcess.CloseMainWindow()
        $deadline = [DateTime]::UtcNow.AddSeconds(5)
        do {
            Start-Sleep -Milliseconds 100
            $recheck = Get-NativeSnapshot -Id ([int]$processRow.ProcessId)
        } while ((Test-SameGeneration -Expected $processRow -Actual $recheck -ExpectedPath $resolvedExe) -and [DateTime]::UtcNow -lt $deadline)
        if (Test-SameGeneration -Expected $processRow -Actual $recheck -ExpectedPath $resolvedExe) {
            $terminateDetail = ''
            if (-not [MichStartupMasterScriptNativeProcess]::TerminateExact([int]$processRow.ProcessId, $resolvedExe, [long]$processRow.StartTimeUtcFileTime, [ref]$terminateDetail)) {
                $finalCheck = Get-NativeSnapshot -Id ([int]$processRow.ProcessId)
                if (Test-SameGeneration -Expected $processRow -Actual $finalCheck -ExpectedPath $resolvedExe) {
                    throw "Could not terminate exact PID $($processRow.ProcessId): $terminateDetail"
                }
            }
        }
    } finally {
        $runtimeProcess.Dispose()
    }
}

$remaining = @(
    foreach ($process in @(Get-Process -Name 'MichStartupMaster' -ErrorAction SilentlyContinue)) {
        $snapshot = Get-NativeSnapshot -Id ([int]$process.Id)
        if ($null -ne $snapshot -and (Test-ExactPath -Candidate $snapshot.ImagePath -Expected $resolvedExe)) { $snapshot }
    }
)
if ($remaining.Count -ne 0) {
    $remainingSummary = @($remaining | ForEach-Object { 'PID={0} START={1} CMD={2}' -f $_.ProcessId,$_.StartTimeUtcFileTime,$_.CommandLine }) -join ' | '
    throw "Exact-path MichStartupMaster shutdown is incomplete; $($remaining.Count) process generation(s) remain. $remainingSummary"
}
[pscustomobject][ordered]@{
    Outcome = 'ALL_EXACT_PATH_PROCESSES_STOPPED'
    Executable = $resolvedExe
    RequestedProcessIds = @($requestedIds)
    StoppedGenerationCount = @($exactProcesses).Count
    RemainingGenerationCount = 0
} | Format-List
