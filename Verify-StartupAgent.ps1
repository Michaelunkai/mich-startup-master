[CmdletBinding()]
param(
    [string]$ExecutablePath = (Join-Path $env:LOCALAPPDATA 'Programs\MichStartupMaster\MichStartupMaster.exe')
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
if (-not ('MichStartupMasterWindows' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class MichStartupMasterWindows
{
    public delegate bool EnumWindowsProc(IntPtr handle, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr handle);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr handle, out uint processId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr handle, StringBuilder text, int maxCount);
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

if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "MichStartupMaster executable was not found: $ExecutablePath"
}

$expectedExe = (Resolve-Path -LiteralPath $ExecutablePath -ErrorAction Stop).ProviderPath
$verificationOutput = @(& $expectedExe '--verify-agent' 2>&1)
$verificationExitCode = $LASTEXITCODE
$verificationReceipt = ($verificationOutput | ForEach-Object { [string]$_ }) -join [Environment]::NewLine
if ($verificationExitCode -ne 0) {
    throw "Read-only agent verification failed with exit code $verificationExitCode. $verificationReceipt"
}
if ($verificationReceipt -notmatch '(?m)^REGISTER_AGENT_OK\b' -or
    $verificationReceipt -notmatch '\bownedRoutes=1\b' -or
    $verificationReceipt -notmatch '\blegacyRoutes=0\b' -or
    $verificationReceipt -notmatch [regex]::Escape($expectedExe + ' --agent')) {
    throw "The read-only app receipt did not prove one canonical boot task for this exact executable with no legacy routes. $verificationReceipt"
}

$processes = @(
    foreach ($process in @(Get-Process -Name 'MichStartupMaster' -ErrorAction SilentlyContinue)) {
        $snapshot = Get-NativeSnapshot -Id ([int]$process.Id)
        if ($null -ne $snapshot -and (Test-ExactPath -Candidate $snapshot.ImagePath -Expected $expectedExe) -and
            [string]::Equals((Get-CommandArguments -CommandLine $snapshot.CommandLine -Executable $expectedExe), '--agent', [StringComparison]::OrdinalIgnoreCase)) {
            $snapshot
        }
    }
)

if ($processes.Count -ne 1) {
    throw "Expected one local --agent process, found $($processes.Count)."
}

$visibleWindows = New-Object System.Collections.Generic.List[string]
$callback = [MichStartupMasterWindows+EnumWindowsProc]{
    param($handle, $unused)
    [uint32]$processId = 0
    [void][MichStartupMasterWindows]::GetWindowThreadProcessId($handle, [ref]$processId)
    if ($processId -eq [uint32]$processes[0].ProcessId -and [MichStartupMasterWindows]::IsWindowVisible($handle)) {
        $text = New-Object System.Text.StringBuilder 512
        [void][MichStartupMasterWindows]::GetWindowText($handle, $text, $text.Capacity)
        $visibleWindows.Add($text.ToString())
    }
    return $true
}
[void][MichStartupMasterWindows]::EnumWindows($callback, [IntPtr]::Zero)

if ($visibleWindows.Count -ne 0) {
    throw "Agent exposed visible windows: $($visibleWindows -join ' | ')"
}

[pscustomobject]@{
    Outcome = 'BOOT_REGISTRATION_AND_HIDDEN_AGENT_VERIFIED'
    VerifiedScope = 'Canonical boot registration plus exactly one exact-path hidden --agent process'
    AgentProcess = $processes[0].ProcessId
    Executable = $processes[0].ImagePath
    CommandLine = $processes[0].CommandLine
    StartTimeUtcFileTime = $processes[0].StartTimeUtcFileTime
    VisibleWindows = $visibleWindows.Count
    BootRegistrationVerified = $true
    ExactHiddenAgentProcessVerified = $true
    WrapperLineageClaimed = $false
    TrayIconReadinessClaimed = $false
    UnclaimedScope = 'Managed-app tray wrappers and tray-icon click readiness require the dedicated live harness and are intentionally not claimed by this legacy verifier.'
    VerificationReceipt = $verificationReceipt.Trim()
} | Format-List
