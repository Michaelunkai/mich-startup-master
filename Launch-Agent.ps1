[CmdletBinding()]
param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot 'build\MichStartupMaster.exe')
)

$ErrorActionPreference = 'Stop'

if (-not ('MichStartupMasterScriptNativeProcess' -as [type])) {
    Add-Type @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

public sealed class MichStartupMasterScriptProcessSnapshot
{
    public int ProcessId;
    public string ImagePath;
    public string CommandLine;
    public long StartTimeUtcFileTime;
}

public static class MichStartupMasterScriptNativeProcess
{
    private const uint PROCESS_TERMINATE = 0x0001;
    private const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
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
        imagePath = ""; startTime = 0; error = "";
        uint length = 32768; StringBuilder path = new StringBuilder((int)length);
        if (!QueryFullProcessImageName(process, 0, path, ref length)) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
        FILETIME creation, exit, kernel, user;
        if (!GetProcessTimes(process, out creation, out exit, out kernel, out user)) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
        imagePath = path.ToString(); startTime = ((long)creation.High << 32) | creation.Low; return true;
    }

    public static bool TrySnapshot(int processId, out MichStartupMasterScriptProcessSnapshot snapshot, out string error)
    {
        snapshot = null; error = "";
        IntPtr process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, processId);
        if (process == IntPtr.Zero) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
        IntPtr buffer = IntPtr.Zero;
        try {
            string path; long start;
            if (!TryIdentity(process, out path, out start, out error)) return false;
            int required; NtQueryInformationProcess(process, ProcessCommandLineInformation, IntPtr.Zero, 0, out required);
            if (required <= 0 || required > 1048576) { error = "invalid command-line buffer length " + required; return false; }
            buffer = Marshal.AllocHGlobal(required);
            int status = NtQueryInformationProcess(process, ProcessCommandLineInformation, buffer, required, out required);
            if (status < 0) { error = "NtQueryInformationProcess failed with NTSTATUS 0x" + status.ToString("X8"); return false; }
            UNICODE_STRING command = (UNICODE_STRING)Marshal.PtrToStructure(buffer, typeof(UNICODE_STRING));
            snapshot = new MichStartupMasterScriptProcessSnapshot();
            snapshot.ProcessId = processId; snapshot.ImagePath = path; snapshot.StartTimeUtcFileTime = start;
            snapshot.CommandLine = command.Length == 0 ? "" : (Marshal.PtrToStringUni(command.Buffer, command.Length / 2) ?? "");
            return true;
        } finally { if (buffer != IntPtr.Zero) Marshal.FreeHGlobal(buffer); CloseHandle(process); }
    }

    public static bool TerminateExact(int processId, string expectedPath, long expectedStart, out string error)
    {
        error = "";
        IntPtr process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE, false, processId);
        if (process == IntPtr.Zero) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
        try {
            string path; long start;
            if (!TryIdentity(process, out path, out start, out error)) return false;
            if (!String.Equals(Path.GetFullPath(path), Path.GetFullPath(expectedPath), StringComparison.OrdinalIgnoreCase) || start != expectedStart) { error = "process generation no longer matches"; return false; }
            if (!TerminateProcess(process, 1)) { error = new Win32Exception(Marshal.GetLastWin32Error()).Message; return false; }
            return true;
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

function Get-CommandArguments {
    param([string]$CommandLine, [Parameter(Mandatory = $true)][string]$Executable)
    $text = ([string]$CommandLine).Trim()
    if ($text.StartsWith('"')) {
        $end = $text.IndexOf('"', 1)
        if ($end -gt 0 -and (Test-ExactPath -Candidate $text.Substring(1, $end - 1) -Expected $Executable)) { return $text.Substring($end + 1).Trim() }
    }
    if ($text.StartsWith($Executable, [StringComparison]::OrdinalIgnoreCase)) { return $text.Substring($Executable.Length).Trim() }
    throw "Could not safely parse command line for the exact executable: $CommandLine"
}

function Get-ExactAgentSnapshots {
    param([Parameter(Mandatory = $true)][string]$Executable)
    return @(
        foreach ($process in @(Get-Process -Name 'MichStartupMaster' -ErrorAction SilentlyContinue)) {
            $snapshot = Get-NativeSnapshot -Id ([int]$process.Id)
            if ($null -ne $snapshot -and (Test-ExactPath -Candidate $snapshot.ImagePath -Expected $Executable) -and
                [string]::Equals((Get-CommandArguments -CommandLine $snapshot.CommandLine -Executable $Executable), '--agent', [StringComparison]::OrdinalIgnoreCase)) {
                $snapshot
            }
        }
    )
}

if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "MichStartupMaster executable was not found: $ExecutablePath"
}

$resolvedExe = (Resolve-Path -LiteralPath $ExecutablePath -ErrorAction Stop).ProviderPath
$existingAgents = @(Get-ExactAgentSnapshots -Executable $resolvedExe)

if ($existingAgents.Count -gt 1) {
    throw "Refusing to report success while $($existingAgents.Count) exact-path --agent generations are live."
}
if ($existingAgents.Count -eq 1) {
    [pscustomobject][ordered]@{
        Outcome = 'EXACTLY_ONE_AGENT_VERIFIED'
        StartedByThisInvocation = $false
        ProcessId = [int]$existingAgents[0].ProcessId
        ImagePath = [string]$existingAgents[0].ImagePath
        CommandLine = [string]$existingAgents[0].CommandLine
        StartTimeUtcFileTime = [long]$existingAgents[0].StartTimeUtcFileTime
    } | Format-List
    return
}

$workingDirectory = Split-Path -Parent $resolvedExe
$startedProcess = Start-Process -FilePath $resolvedExe -ArgumentList '--agent' -WorkingDirectory $workingDirectory -WindowStyle Hidden -PassThru
try {
    $startedSnapshot = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $agents = @(Get-ExactAgentSnapshots -Executable $resolvedExe)
        if ($agents.Count -eq 1) {
            $candidate = $agents[0]
            if ([int]$candidate.ProcessId -eq [int]$startedProcess.Id) {
                $startedSnapshot = $candidate
                break
            }
        } elseif ($agents.Count -gt 1) {
            throw "Agent launch created or exposed $($agents.Count) exact-path --agent generations."
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($null -eq $startedSnapshot) {
        $finalAgents = @(Get-ExactAgentSnapshots -Executable $resolvedExe)
        throw "Agent launch did not produce exactly one verified surviving generation owned by PID $($startedProcess.Id); exact agents found: $($finalAgents.Count)."
    }
    [pscustomobject][ordered]@{
        Outcome = 'EXACTLY_ONE_AGENT_VERIFIED'
        StartedByThisInvocation = $true
        ProcessId = [int]$startedSnapshot.ProcessId
        ImagePath = [string]$startedSnapshot.ImagePath
        CommandLine = [string]$startedSnapshot.CommandLine
        StartTimeUtcFileTime = [long]$startedSnapshot.StartTimeUtcFileTime
    } | Format-List
} finally {
    $startedProcess.Dispose()
}
