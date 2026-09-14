[CmdletBinding()]
param(
  [switch]$QuietLaunchOnly,
  [switch]$AllowLiveMutation,
  [string]$TestAppPath = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$Root = Split-Path -Path $PSScriptRoot -Parent
$App = if ([string]::IsNullOrWhiteSpace($TestAppPath)) {
  Join-Path $Root 'build\MichStartupMaster.exe'
} else {
  [IO.Path]::GetFullPath($TestAppPath)
}
if (-not (Test-Path -LiteralPath $App -PathType Leaf)) { throw "Missing app: $App" }

$RunId = '{0}-{1}' -f (Get-Date -Format 'yyyyMMddTHHmmssfff'), $PID
$RunDir = Join-Path $Root (Join-Path 'artifacts\runtime-output' ("safe-test-$RunId"))
New-Item -ItemType Directory -Force -Path $RunDir | Out-Null
$script:CommandNumber = 0
$script:HarnessPrimaryError = $null
$script:HarnessCleanupErrors = New-Object 'Collections.Generic.List[string]'
$script:AppIsolationInitialized = $false
$script:ActiveAppStateRoot = ''
$script:RealAppDataEvidenceBefore = $null
$script:PureStateRootAbsentAtStart = $false
$script:IsolationWriteJournal = $null
$script:IsolationJournalStarted = $false

# Capture the caller's process environment before the harness can launch the product. The safe
# probes use a deliberately nonexistent state root; any product write creates it and fails the run.
$script:OriginalKnownStoreEnvironmentEntry = Get-Item Env:MSM_KNOWN_STORE -ErrorAction SilentlyContinue
$script:OriginalKnownStoreEnvironmentExisted = $null -ne $script:OriginalKnownStoreEnvironmentEntry
$script:OriginalKnownStoreEnvironmentValue = if ($script:OriginalKnownStoreEnvironmentExisted) { [string]$script:OriginalKnownStoreEnvironmentEntry.Value } else { '' }
$script:OriginalStateRootEnvironmentEntry = Get-Item Env:MSM_STATE_ROOT -ErrorAction SilentlyContinue
$script:OriginalStateRootEnvironmentExisted = $null -ne $script:OriginalStateRootEnvironmentEntry
$script:OriginalStateRootEnvironmentValue = if ($script:OriginalStateRootEnvironmentExisted) { [string]$script:OriginalStateRootEnvironmentEntry.Value } else { '' }
$script:PureStateRoot = [IO.Path]::GetFullPath((Join-Path $RunDir ("MichStartupMaster-test-pure-$RunId")))
$script:PureKnownStore = [IO.Path]::GetFullPath((Join-Path $script:PureStateRoot 'known-startup-items.tsv'))
$script:RealAppDataRoot = [IO.Path]::GetFullPath((Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)) 'MichStartupMaster'))

try {
  $resolvedRunRootForIsolation = [IO.Path]::GetFullPath($RunDir).TrimEnd('\') + '\'
  if (-not $script:PureStateRoot.StartsWith($resolvedRunRootForIsolation, [StringComparison]::OrdinalIgnoreCase) -or
      [IO.Path]::GetFileName($script:PureStateRoot) -notlike 'MichStartupMaster-test-*') {
    throw "Refusing unsafe pure-test state root: $($script:PureStateRoot)"
  }
  if (Test-Path -LiteralPath $script:PureStateRoot) { throw "Pure-test state root already exists: $($script:PureStateRoot)" }
  $script:PureStateRootAbsentAtStart = $true
  [Environment]::SetEnvironmentVariable('MSM_STATE_ROOT', $script:PureStateRoot, [EnvironmentVariableTarget]::Process)
  [Environment]::SetEnvironmentVariable('MSM_KNOWN_STORE', $script:PureKnownStore, [EnvironmentVariableTarget]::Process)
  $script:ActiveAppStateRoot = $script:PureStateRoot
  $script:AppIsolationInitialized = $true

function ConvertTo-NativeArgument([AllowNull()][string]$Value) {
  if ($null -eq $Value -or $Value.Length -eq 0) { return '""' }
  if ($Value -notmatch '[\s"]') { return $Value }

  $builder = New-Object Text.StringBuilder
  [void]$builder.Append('"')
  $slashes = 0
  foreach ($character in $Value.ToCharArray()) {
    if ($character -eq '\') {
      $slashes++
      continue
    }
    if ($character -eq '"') {
      [void]$builder.Append(('\' * (($slashes * 2) + 1)))
      [void]$builder.Append('"')
      $slashes = 0
      continue
    }
    if ($slashes -gt 0) {
      [void]$builder.Append(('\' * $slashes))
      $slashes = 0
    }
    [void]$builder.Append($character)
  }
  if ($slashes -gt 0) { [void]$builder.Append(('\' * ($slashes * 2))) }
  [void]$builder.Append('"')
  return $builder.ToString()
}

function Initialize-TestProcessContainment {
  $expectedImplementationMarker = 'suspended-job-handle-list-direct-journal-v4'
  $jobType = 'MichStartupMaster.TestHarnessV4.TestProcessJob' -as [type]
  $journalType = 'MichStartupMaster.TestHarnessV4.ProtectedPathJournal' -as [type]
  if ($null -ne $jobType -or $null -ne $journalType) {
    $jobMarkerField = if ($null -ne $jobType) { $jobType.GetField('ImplementationMarker') } else { $null }
    $journalMarkerField = if ($null -ne $journalType) { $journalType.GetField('ImplementationMarker') } else { $null }
    $jobMarker = if ($null -ne $jobMarkerField) { [string]$jobMarkerField.GetRawConstantValue() } else { '' }
    $journalMarker = if ($null -ne $journalMarkerField) { [string]$journalMarkerField.GetRawConstantValue() } else { '' }
    if ($null -ne $jobType -and $null -ne $journalType -and
        [string]::Equals($jobMarker, $expectedImplementationMarker, [StringComparison]::Ordinal) -and
        [string]::Equals($journalMarker, $expectedImplementationMarker, [StringComparison]::Ordinal)) { return }
    throw "Refusing stale or partial native test helpers. expected=$expectedImplementationMarker job=$jobMarker journal=$journalMarker"
  }
  Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace MichStartupMaster.TestHarnessV4
{
    public sealed class ContainedProcess
    {
        private IntPtr _job;
        private IntPtr _process;
        private bool _closed;

        internal ContainedProcess(IntPtr job, IntPtr process, int processId, string path, DateTime startTimeUtc)
        {
            _job = job;
            _process = process;
            ProcessId = processId;
            Path = path;
            StartTimeUtc = startTimeUtc;
        }

        public int ProcessId { get; private set; }
        public string Path { get; private set; }
        public DateTime StartTimeUtc { get; private set; }

        public bool WaitForExit(int timeoutMilliseconds)
        {
            if (_closed || _process == IntPtr.Zero) throw new ObjectDisposedException("ContainedProcess");
            uint result = TestProcessJob.WaitForSingleObjectExact(_process, timeoutMilliseconds);
            if (result == TestProcessJob.WaitObject0) return true;
            if (result == TestProcessJob.WaitTimeout) return false;
            throw new Win32Exception(Marshal.GetLastWin32Error(), "WaitForSingleObject(process) failed");
        }

        public int GetExitCode()
        {
            if (_closed || _process == IntPtr.Zero) throw new ObjectDisposedException("ContainedProcess");
            uint exitCode;
            if (!TestProcessJob.GetExitCodeProcessExact(_process, out exitCode))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "GetExitCodeProcess failed");
            if (exitCode == TestProcessJob.StillActive) throw new InvalidOperationException("Contained process is still active");
            return unchecked((int)exitCode);
        }

        public void TerminateAndVerify(int timeoutMilliseconds)
        {
            if (_closed) throw new ObjectDisposedException("ContainedProcess");
            TestProcessJob.TerminateAndWaitForEmpty(_job, 1460, timeoutMilliseconds);
            if (!WaitForExit(timeoutMilliseconds))
                throw new TimeoutException("Exact contained root did not exit after job termination; pid=" + ProcessId + " startUtc=" + StartTimeUtc.ToString("o") + " path=" + Path);
        }

        public void CloseAndVerifyNoSurvivors(int timeoutMilliseconds)
        {
            if (_closed) return;
            Exception drainError = null;
            Exception processCloseError = null;
            Exception jobCloseError = null;
            try
            {
                TestProcessJob.TerminateAndWaitForEmpty(_job, 1460, timeoutMilliseconds);
            }
            catch (Exception exception)
            {
                drainError = exception;
            }
            try
            {
                if (_process != IntPtr.Zero && !TestProcessJob.CloseHandleExact(_process))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "CloseHandle(process) failed");
            }
            catch (Exception exception)
            {
                processCloseError = exception;
            }
            finally
            {
                _process = IntPtr.Zero;
            }
            try
            {
                if (_job != IntPtr.Zero && !TestProcessJob.CloseHandleExact(_job))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "CloseHandle(job) failed");
            }
            catch (Exception exception)
            {
                jobCloseError = exception;
            }
            finally
            {
                _job = IntPtr.Zero;
                _closed = true;
            }
            if (drainError != null || processCloseError != null || jobCloseError != null)
            {
                throw new InvalidOperationException(
                    "Contained generation cleanup failed; pid=" + ProcessId +
                    " startUtc=" + StartTimeUtc.ToString("o") +
                    " path=" + Path +
                    " drain=" + (drainError == null ? "none" : drainError.Message) +
                    " processClose=" + (processCloseError == null ? "none" : processCloseError.Message) +
                    " jobClose=" + (jobCloseError == null ? "none" : jobCloseError.Message));
            }
        }
    }

    public static class TestProcessJob
    {
        public const string ImplementationMarker = "suspended-job-handle-list-direct-journal-v4";
        private const uint JobObjectExtendedLimitInformation = 9;
        private const uint JobObjectLimitKillOnJobClose = 0x00002000;
        private const uint GenericRead = 0x80000000;
        private const uint GenericWrite = 0x40000000;
        private const uint FileShareRead = 0x00000001;
        private const uint FileShareWrite = 0x00000002;
        private const uint FileShareDelete = 0x00000004;
        private const uint CreateAlways = 2;
        private const uint OpenExisting = 3;
        private const uint FileAttributeNormal = 0x00000080;
        private const uint CreateSuspended = 0x00000004;
        private const uint ExtendedStartupInfoPresent = 0x00080000;
        private const uint CreateNoWindow = 0x08000000;
        private const uint ProcThreadAttributeHandleList = 0x00020002;
        private const int StartfUseShowWindow = 0x00000001;
        private const int StartfUseStdHandles = 0x00000100;
        private const short SwHide = 0;
        private static readonly IntPtr InvalidHandleValue = new IntPtr(-1);

        internal const uint WaitObject0 = 0;
        internal const uint WaitTimeout = 258;
        internal const uint StillActive = 259;

        [StructLayout(LayoutKind.Sequential)]
        private struct IoCounters
        {
            public ulong ReadOperationCount;
            public ulong WriteOperationCount;
            public ulong OtherOperationCount;
            public ulong ReadTransferCount;
            public ulong WriteTransferCount;
            public ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct BasicLimitInformation
        {
            public long PerProcessUserTimeLimit;
            public long PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ExtendedLimitInformation
        {
            public BasicLimitInformation BasicLimitInformation;
            public IoCounters IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct BasicAccountingInformation
        {
            public long TotalUserTime;
            public long TotalKernelTime;
            public long ThisPeriodTotalUserTime;
            public long ThisPeriodTotalKernelTime;
            public uint TotalPageFaultCount;
            public uint TotalProcesses;
            public uint ActiveProcesses;
            public uint TotalTerminatedProcesses;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct SecurityAttributes
        {
            public int Length;
            public IntPtr SecurityDescriptor;
            public int InheritHandle;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct StartupInfo
        {
            public int Size;
            public string Reserved;
            public string Desktop;
            public string Title;
            public int X;
            public int Y;
            public int XSize;
            public int YSize;
            public int XCountChars;
            public int YCountChars;
            public int FillAttribute;
            public int Flags;
            public short ShowWindow;
            public short Reserved2Size;
            public IntPtr Reserved2;
            public IntPtr StdInput;
            public IntPtr StdOutput;
            public IntPtr StdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct StartupInfoEx
        {
            public StartupInfo StartupInfo;
            public IntPtr AttributeList;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ProcessInformation
        {
            public IntPtr Process;
            public IntPtr Thread;
            public uint ProcessId;
            public uint ThreadId;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeFileTime
        {
            public uint Low;
            public uint High;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateJobObject(IntPtr securityAttributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetInformationJobObject(IntPtr job, uint informationClass, IntPtr information, uint informationLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateJobObject(IntPtr job, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool QueryInformationJobObject(IntPtr job, uint informationClass, IntPtr information, uint informationLength, out uint returnLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CloseHandle(IntPtr handle);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateFile(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            ref SecurityAttributes securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true, EntryPoint = "CreateProcessW")]
        private static extern bool CreateProcess(
            string applicationName,
            StringBuilder commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            bool inheritHandles,
            uint creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref StartupInfoEx startupInfo,
            out ProcessInformation processInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool InitializeProcThreadAttributeList(
            IntPtr attributeList,
            int attributeCount,
            uint flags,
            ref IntPtr size);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool UpdateProcThreadAttribute(
            IntPtr attributeList,
            uint flags,
            UIntPtr attribute,
            IntPtr value,
            IntPtr size,
            IntPtr previousValue,
            IntPtr returnSize);

        [DllImport("kernel32.dll")]
        private static extern void DeleteProcThreadAttributeList(IntPtr attributeList);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint ResumeThread(IntPtr thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateProcess(IntPtr process, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetProcessTimes(IntPtr process, out NativeFileTime creation, out NativeFileTime exit, out NativeFileTime kernel, out NativeFileTime user);

        private static IntPtr Create()
        {
            IntPtr job = CreateJobObject(IntPtr.Zero, null);
            if (job == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateJobObject failed");
            IntPtr buffer = IntPtr.Zero;
            try
            {
                var information = new ExtendedLimitInformation();
                information.BasicLimitInformation.LimitFlags = JobObjectLimitKillOnJobClose;
                int size = Marshal.SizeOf(typeof(ExtendedLimitInformation));
                buffer = Marshal.AllocHGlobal(size);
                Marshal.StructureToPtr(information, buffer, false);
                if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, buffer, (uint)size))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "SetInformationJobObject failed");
                return job;
            }
            catch
            {
                CloseHandle(job);
                throw;
            }
            finally
            {
                if (buffer != IntPtr.Zero) Marshal.FreeHGlobal(buffer);
            }
        }

        internal static uint WaitForSingleObjectExact(IntPtr handle, int timeoutMilliseconds)
        {
            if (timeoutMilliseconds < 0) throw new ArgumentOutOfRangeException("timeoutMilliseconds");
            return WaitForSingleObject(handle, (uint)timeoutMilliseconds);
        }

        internal static bool GetExitCodeProcessExact(IntPtr process, out uint exitCode)
        {
            return GetExitCodeProcess(process, out exitCode);
        }

        internal static bool CloseHandleExact(IntPtr handle)
        {
            return CloseHandle(handle);
        }

        private static uint GetActiveProcessCount(IntPtr job)
        {
            int size = Marshal.SizeOf(typeof(BasicAccountingInformation));
            IntPtr buffer = Marshal.AllocHGlobal(size);
            try
            {
                uint returned;
                if (!QueryInformationJobObject(job, 1, buffer, (uint)size, out returned))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "QueryInformationJobObject failed");
                var information = (BasicAccountingInformation)Marshal.PtrToStructure(buffer, typeof(BasicAccountingInformation));
                return information.ActiveProcesses;
            }
            finally
            {
                Marshal.FreeHGlobal(buffer);
            }
        }

        private static long[] GetContainedProcessIds(IntPtr job)
        {
            const int capacity = 4096;
            int size = 8 + (capacity * IntPtr.Size);
            IntPtr buffer = Marshal.AllocHGlobal(size);
            try
            {
                uint returned;
                if (!QueryInformationJobObject(job, 3, buffer, (uint)size, out returned))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "QueryInformationJobObject(process ids) failed");
                int count = Marshal.ReadInt32(buffer, 4);
                if (count < 0 || count > capacity) throw new InvalidOperationException("Invalid contained process count: " + count);
                var processIds = new long[count];
                for (int index = 0; index < count; index++)
                {
                    IntPtr value = Marshal.ReadIntPtr(buffer, 8 + (index * IntPtr.Size));
                    processIds[index] = value.ToInt64();
                }
                return processIds;
            }
            finally
            {
                Marshal.FreeHGlobal(buffer);
            }
        }

        internal static void TerminateAndWaitForEmpty(IntPtr job, uint exitCode, int timeoutMilliseconds)
        {
            if (job == IntPtr.Zero) return;
            uint active = GetActiveProcessCount(job);
            if (active == 0) return;
            if (!TerminateJobObject(job, exitCode))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "TerminateJobObject failed");
            var timer = Stopwatch.StartNew();
            while ((active = GetActiveProcessCount(job)) != 0)
            {
                if (timer.ElapsedMilliseconds >= timeoutMilliseconds)
                {
                    long[] processIds = GetContainedProcessIds(job);
                    string pidList = string.Join(",", Array.ConvertAll<long, string>(processIds, delegate(long value) { return value.ToString(); }));
                    throw new TimeoutException("Contained product job still has " + active + " active process(es) after " + timeoutMilliseconds + "ms; pids=" + pidList);
                }
                Thread.Sleep(25);
            }
        }

        private static string QuoteApplication(string path)
        {
            return "\"" + path.Replace("\"", "\\\"") + "\"";
        }

        private static DateTime ReadCreationTimeUtc(IntPtr process)
        {
            NativeFileTime creation;
            NativeFileTime exit;
            NativeFileTime kernel;
            NativeFileTime user;
            if (!GetProcessTimes(process, out creation, out exit, out kernel, out user))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "GetProcessTimes failed");
            long fileTime = ((long)creation.High << 32) | creation.Low;
            return DateTime.FromFileTimeUtc(fileTime);
        }

        private static void CloseNoThrow(IntPtr handle)
        {
            if (handle != IntPtr.Zero && handle != InvalidHandleValue) CloseHandle(handle);
        }

        public static ContainedProcess StartSuspendedAndAssign(
            string applicationPath,
            string arguments,
            string workingDirectory,
            string stdoutPath,
            string stderrPath)
        {
            if (string.IsNullOrWhiteSpace(applicationPath)) throw new ArgumentNullException("applicationPath");
            if (string.IsNullOrWhiteSpace(workingDirectory)) throw new ArgumentNullException("workingDirectory");
            if (string.IsNullOrWhiteSpace(stdoutPath)) throw new ArgumentNullException("stdoutPath");
            if (string.IsNullOrWhiteSpace(stderrPath)) throw new ArgumentNullException("stderrPath");

            string resolvedApplication = Path.GetFullPath(applicationPath);
            string resolvedWorkingDirectory = Path.GetFullPath(workingDirectory);
            string resolvedStdout = Path.GetFullPath(stdoutPath);
            string resolvedStderr = Path.GetFullPath(stderrPath);
            var security = new SecurityAttributes
            {
                Length = Marshal.SizeOf(typeof(SecurityAttributes)),
                SecurityDescriptor = IntPtr.Zero,
                InheritHandle = 1
            };
            IntPtr stdin = IntPtr.Zero;
            IntPtr stdout = IntPtr.Zero;
            IntPtr stderr = IntPtr.Zero;
            IntPtr attributeList = IntPtr.Zero;
            IntPtr attributeListSize = IntPtr.Zero;
            IntPtr inheritedHandleList = IntPtr.Zero;
            IntPtr job = IntPtr.Zero;
            var processInformation = new ProcessInformation();
            bool created = false;
            bool assigned = false;
            bool resumed = false;
            bool attributeListInitialized = false;
            try
            {
                stdin = CreateFile("NUL", GenericRead, FileShareRead | FileShareWrite, ref security, OpenExisting, FileAttributeNormal, IntPtr.Zero);
                if (stdin == InvalidHandleValue) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateFile(NUL) failed");
                stdout = CreateFile(resolvedStdout, GenericWrite, FileShareRead | FileShareWrite | FileShareDelete, ref security, CreateAlways, FileAttributeNormal, IntPtr.Zero);
                if (stdout == InvalidHandleValue) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateFile(stdout) failed");
                stderr = CreateFile(resolvedStderr, GenericWrite, FileShareRead | FileShareWrite | FileShareDelete, ref security, CreateAlways, FileAttributeNormal, IntPtr.Zero);
                if (stderr == InvalidHandleValue) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateFile(stderr) failed");

                job = Create();
                InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref attributeListSize);
                if (attributeListSize == IntPtr.Zero)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "InitializeProcThreadAttributeList(size) failed");
                attributeList = Marshal.AllocHGlobal(attributeListSize);
                if (!InitializeProcThreadAttributeList(attributeList, 1, 0, ref attributeListSize))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "InitializeProcThreadAttributeList failed");
                attributeListInitialized = true;
                inheritedHandleList = Marshal.AllocHGlobal(IntPtr.Size * 3);
                Marshal.WriteIntPtr(inheritedHandleList, 0, stdin);
                Marshal.WriteIntPtr(inheritedHandleList, IntPtr.Size, stdout);
                Marshal.WriteIntPtr(inheritedHandleList, IntPtr.Size * 2, stderr);
                if (!UpdateProcThreadAttribute(
                    attributeList,
                    0,
                    new UIntPtr(ProcThreadAttributeHandleList),
                    inheritedHandleList,
                    new IntPtr(IntPtr.Size * 3),
                    IntPtr.Zero,
                    IntPtr.Zero))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "UpdateProcThreadAttribute(PROC_THREAD_ATTRIBUTE_HANDLE_LIST) failed");

                var startupInfo = new StartupInfoEx
                {
                    StartupInfo = new StartupInfo
                    {
                        Size = Marshal.SizeOf(typeof(StartupInfoEx)),
                        Flags = StartfUseShowWindow | StartfUseStdHandles,
                        ShowWindow = SwHide,
                        StdInput = stdin,
                        StdOutput = stdout,
                        StdError = stderr
                    },
                    AttributeList = attributeList
                };
                string fullCommandLine = QuoteApplication(resolvedApplication);
                if (!string.IsNullOrWhiteSpace(arguments)) fullCommandLine += " " + arguments;
                var mutableCommandLine = new StringBuilder(fullCommandLine);
                if (!CreateProcess(
                    resolvedApplication,
                    mutableCommandLine,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    true,
                    CreateSuspended | CreateNoWindow | ExtendedStartupInfoPresent,
                    IntPtr.Zero,
                    resolvedWorkingDirectory,
                    ref startupInfo,
                    out processInformation))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateProcess(CREATE_SUSPENDED) failed");
                created = true;

                DateTime creationUtc = ReadCreationTimeUtc(processInformation.Process);
                if (!AssignProcessToJobObject(job, processInformation.Process))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "AssignProcessToJobObject(suspended root) failed");
                assigned = true;
                if (ResumeThread(processInformation.Thread) == 0xFFFFFFFF)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "ResumeThread failed");
                resumed = true;
                CloseNoThrow(processInformation.Thread);
                processInformation.Thread = IntPtr.Zero;
                var result = new ContainedProcess(job, processInformation.Process, unchecked((int)processInformation.ProcessId), resolvedApplication, creationUtc);
                job = IntPtr.Zero;
                processInformation.Process = IntPtr.Zero;
                return result;
            }
            catch (Exception primary)
            {
                Exception cleanup = null;
                try
                {
                    if (created && processInformation.Process != IntPtr.Zero)
                    {
                        if (assigned && job != IntPtr.Zero) TerminateAndWaitForEmpty(job, 1460, 5000);
                        else if (!TerminateProcess(processInformation.Process, 1460))
                            throw new Win32Exception(Marshal.GetLastWin32Error(), "TerminateProcess(suspended root) failed");
                        uint wait = WaitForSingleObject(processInformation.Process, 5000);
                        if (wait != WaitObject0)
                            throw new TimeoutException("Suspended root cleanup did not complete; pid=" + processInformation.ProcessId + " wait=" + wait + " resumed=" + resumed);
                    }
                }
                catch (Exception exception)
                {
                    cleanup = exception;
                }
                if (cleanup != null)
                    throw new InvalidOperationException("Contained suspended launch failed and exact root cleanup also failed. launch=" + primary.Message + " cleanup=" + cleanup.Message, primary);
                throw;
            }
            finally
            {
                CloseNoThrow(processInformation.Thread);
                CloseNoThrow(processInformation.Process);
                CloseNoThrow(job);
                CloseNoThrow(stdin);
                CloseNoThrow(stdout);
                CloseNoThrow(stderr);
                if (attributeListInitialized) DeleteProcThreadAttributeList(attributeList);
                if (attributeList != IntPtr.Zero) Marshal.FreeHGlobal(attributeList);
                if (inheritedHandleList != IntPtr.Zero) Marshal.FreeHGlobal(inheritedHandleList);
            }
        }
    }

    public sealed class ProtectedPathJournal : IDisposable
    {
        public const string ImplementationMarker = "suspended-job-handle-list-direct-journal-v4";
        private const uint FileListDirectory = 0x00000001;
        private const uint FileShareRead = 0x00000001;
        private const uint FileShareWrite = 0x00000002;
        private const uint FileShareDelete = 0x00000004;
        private const uint OpenExisting = 3;
        private const uint FileFlagBackupSemantics = 0x02000000;
        private const uint FileFlagOverlapped = 0x40000000;
        private const uint NotifyChangeFileName = 0x00000001;
        private const uint NotifyChangeDirectoryName = 0x00000002;
        private const uint NotifyChangeAttributes = 0x00000004;
        private const uint NotifyChangeSize = 0x00000008;
        private const uint NotifyChangeLastWrite = 0x00000010;
        private const uint NotifyChangeCreation = 0x00000040;
        private const uint NotifyChangeSecurity = 0x00000100;
        private const uint NotifyChangeStreamName = 0x00000200;
        private const uint NotifyChangeStreamSize = 0x00000400;
        private const uint NotifyChangeStreamWrite = 0x00000800;
        private const int ErrorIoPending = 997;
        private const int ErrorOperationAborted = 995;
        private const int ErrorNotFound = 1168;
        private const uint WaitObject0 = 0;
        private const uint WaitTimeout = 258;
        private const uint WaitFailed = 0xFFFFFFFF;
        private const uint Infinite = 0xFFFFFFFF;
        private static readonly IntPtr InvalidHandleValue = new IntPtr(-1);

        private readonly ConcurrentQueue<string> _entries = new ConcurrentQueue<string>();
        private readonly List<NativeWatcher> _watchers = new List<NativeWatcher>();
        private int _stopped;

        [StructLayout(LayoutKind.Sequential)]
        private struct NativeOverlappedData
        {
            public UIntPtr Internal;
            public UIntPtr InternalHigh;
            public uint Offset;
            public uint OffsetHigh;
            public IntPtr EventHandle;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateFile(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            IntPtr securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool ReadDirectoryChangesW(
            IntPtr directory,
            IntPtr buffer,
            uint bufferLength,
            bool watchSubtree,
            uint notifyFilter,
            out uint bytesReturned,
            IntPtr overlapped,
            IntPtr completionRoutine);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetOverlappedResult(IntPtr file, IntPtr overlapped, out uint bytesTransferred, bool wait);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CancelIoEx(IntPtr file, IntPtr overlapped);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateEvent(IntPtr eventAttributes, bool manualReset, bool initialState, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetEvent(IntPtr eventHandle);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool ResetEvent(IntPtr eventHandle);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForMultipleObjects(uint count, IntPtr[] handles, bool waitAll, uint milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CloseHandle(IntPtr handle);

        private static string Normalize(string path)
        {
            return Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        }

        private static bool IsProtected(string candidate, string root)
        {
            string normalized = Normalize(candidate);
            return string.Equals(normalized, root, StringComparison.OrdinalIgnoreCase) ||
                   normalized.StartsWith(root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);
        }

        private static string ActionName(uint action)
        {
            switch (action)
            {
                case 1: return "Added";
                case 2: return "Removed";
                case 3: return "Modified";
                case 4: return "RenamedOldName";
                case 5: return "RenamedNewName";
                default: return "Unknown(" + action + ")";
            }
        }

        private void RecordPath(string label, string eventName, string protectedRoot, string path)
        {
            try
            {
                if (!string.IsNullOrWhiteSpace(path) && IsProtected(path, protectedRoot))
                    _entries.Enqueue(label + "|" + eventName + "|" + Normalize(path));
            }
            catch (Exception exception)
            {
                _entries.Enqueue(label + "|RECORD_ERROR|" + exception.Message);
            }
        }

        private void RecordError(string label, string message)
        {
            _entries.Enqueue(label + "|WATCHER_ERROR_OR_OVERFLOW|" + message);
        }

        public void Add(string watchPath, bool includeSubdirectories, string protectedRoot, string label)
        {
            if (Volatile.Read(ref _stopped) != 0) throw new ObjectDisposedException("ProtectedPathJournal");
            string resolvedWatchPath = Normalize(watchPath);
            string resolvedProtectedRoot = Normalize(protectedRoot);
            if (!Directory.Exists(resolvedWatchPath)) throw new DirectoryNotFoundException("Watcher directory is missing: " + resolvedWatchPath);
            var watcher = new NativeWatcher(this, resolvedWatchPath, includeSubdirectories, resolvedProtectedRoot, label);
            _watchers.Add(watcher);
        }

        public string[] Snapshot()
        {
            return _entries.ToArray();
        }

        public string[] StopAndSnapshot(int timeoutMilliseconds)
        {
            if (Interlocked.Exchange(ref _stopped, 1) == 0)
            {
                foreach (NativeWatcher watcher in _watchers)
                {
                    try { watcher.StopAndDrain(timeoutMilliseconds); }
                    catch (Exception exception) { RecordError("watcher-stop", exception.Message); }
                }
                _watchers.Clear();
            }
            return Snapshot();
        }

        public void Dispose()
        {
            StopAndSnapshot(5000);
        }

        private sealed class NativeWatcher
        {
            private const int BufferSize = 65536;
            private readonly ProtectedPathJournal _owner;
            private readonly string _watchPath;
            private readonly bool _includeSubdirectories;
            private readonly string _protectedRoot;
            private readonly string _label;
            private readonly ManualResetEventSlim _ready = new ManualResetEventSlim(false);
            private IntPtr _directory = InvalidHandleValue;
            private IntPtr _ioEvent = IntPtr.Zero;
            private IntPtr _stopEvent = IntPtr.Zero;
            private IntPtr _buffer = IntPtr.Zero;
            private IntPtr _overlapped = IntPtr.Zero;
            private Thread _thread;
            private Exception _startupError;
            private int _stopRequested;
            private int _drainTimeoutMilliseconds = 5000;
            private int _undrainedIo;
            private int _cleaned;

            internal NativeWatcher(ProtectedPathJournal owner, string watchPath, bool includeSubdirectories, string protectedRoot, string label)
            {
                _owner = owner;
                _watchPath = watchPath;
                _includeSubdirectories = includeSubdirectories;
                _protectedRoot = protectedRoot;
                _label = label;
                try
                {
                    _directory = CreateFile(
                        _watchPath,
                        FileListDirectory,
                        FileShareRead | FileShareWrite | FileShareDelete,
                        IntPtr.Zero,
                        OpenExisting,
                        FileFlagBackupSemantics | FileFlagOverlapped,
                        IntPtr.Zero);
                    if (_directory == InvalidHandleValue)
                        throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateFile(directory watcher) failed: " + _watchPath);
                    _ioEvent = CreateEvent(IntPtr.Zero, true, false, null);
                    if (_ioEvent == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateEvent(io) failed");
                    _stopEvent = CreateEvent(IntPtr.Zero, true, false, null);
                    if (_stopEvent == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateEvent(stop) failed");
                    _buffer = Marshal.AllocHGlobal(BufferSize);
                    _overlapped = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(NativeOverlappedData)));
                    _thread = new Thread(Run);
                    _thread.IsBackground = true;
                    _thread.Name = "MSM path journal " + label;
                    _thread.Start();
                    if (!_ready.Wait(5000))
                        throw new TimeoutException("ReadDirectoryChangesW watcher did not arm within 5000ms: " + label);
                    if (_startupError != null)
                        throw new InvalidOperationException("ReadDirectoryChangesW watcher failed to arm: " + label + " " + _startupError.Message, _startupError);
                }
                catch
                {
                    RequestStop();
                    if (_thread == null || _thread.Join(5000))
                    {
                        if (Volatile.Read(ref _undrainedIo) == 0) Cleanup();
                        else _owner.RecordError(_label, "startup resources retained because pending I/O was not proven drained");
                    }
                    else _owner.RecordError(_label, "startup cleanup timed out; watcher thread remains alive");
                    throw;
                }
            }

            private void PrepareOverlapped()
            {
                if (!ResetEvent(_ioEvent)) throw new Win32Exception(Marshal.GetLastWin32Error(), "ResetEvent(io) failed");
                var value = new NativeOverlappedData { EventHandle = _ioEvent };
                Marshal.StructureToPtr(value, _overlapped, false);
            }

            private void BeginRead()
            {
                PrepareOverlapped();
                uint ignored;
                bool started = ReadDirectoryChangesW(
                    _directory,
                    _buffer,
                    BufferSize,
                    _includeSubdirectories,
                    NotifyChangeFileName | NotifyChangeDirectoryName | NotifyChangeAttributes | NotifyChangeSize |
                        NotifyChangeLastWrite | NotifyChangeCreation | NotifyChangeSecurity | NotifyChangeStreamName |
                        NotifyChangeStreamSize | NotifyChangeStreamWrite,
                    out ignored,
                    _overlapped,
                    IntPtr.Zero);
                if (!started && Marshal.GetLastWin32Error() != ErrorIoPending)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "ReadDirectoryChangesW failed: " + _watchPath);
            }

            private void ConsumeCompletion(bool wait)
            {
                uint bytes;
                if (!GetOverlappedResult(_directory, _overlapped, out bytes, wait))
                {
                    int error = Marshal.GetLastWin32Error();
                    if (error == ErrorOperationAborted && Volatile.Read(ref _stopRequested) != 0) return;
                    throw new Win32Exception(error, "GetOverlappedResult(directory watcher) failed: " + _watchPath);
                }
                if (bytes == 0)
                {
                    _owner.RecordError(_label, "ReadDirectoryChangesW returned zero bytes; notification overflow or enumeration loss");
                    return;
                }
                ParseBuffer(bytes);
            }

            private void CancelAndDrainPending()
            {
                bool canceled = CancelIoEx(_directory, _overlapped);
                int cancelError = canceled ? 0 : Marshal.GetLastWin32Error();
                if (!canceled && cancelError != ErrorNotFound)
                    throw new Win32Exception(cancelError, "CancelIoEx(directory watcher) failed");
                int timeout = Math.Max(1, Volatile.Read(ref _drainTimeoutMilliseconds));
                uint wait = WaitForSingleObject(_ioEvent, (uint)timeout);
                if (wait == WaitTimeout)
                    throw new TimeoutException("Canceled ReadDirectoryChangesW completion did not drain within " + timeout + "ms: " + _label);
                if (wait == WaitFailed)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "WaitForSingleObject(directory completion) failed");
                if (wait != WaitObject0)
                    throw new InvalidOperationException("Unexpected directory completion wait result: " + wait);
                ConsumeCompletion(false);
            }

            private void ParseBuffer(uint bytes)
            {
                int offset = 0;
                int total = checked((int)bytes);
                while (true)
                {
                    if (offset < 0 || total - offset < 12)
                        throw new InvalidDataException("Truncated FILE_NOTIFY_INFORMATION record at offset " + offset + " of " + total);
                    uint nextOffset = unchecked((uint)Marshal.ReadInt32(_buffer, offset));
                    uint action = unchecked((uint)Marshal.ReadInt32(_buffer, offset + 4));
                    uint fileNameBytes = unchecked((uint)Marshal.ReadInt32(_buffer, offset + 8));
                    if ((fileNameBytes & 1) != 0 || fileNameBytes > total - offset - 12)
                        throw new InvalidDataException("Invalid FILE_NOTIFY_INFORMATION name length " + fileNameBytes + " at offset " + offset);
                    string relativePath = Marshal.PtrToStringUni(IntPtr.Add(_buffer, offset + 12), checked((int)fileNameBytes / 2));
                    string fullPath = Path.GetFullPath(Path.Combine(_watchPath, relativePath));
                    _owner.RecordPath(_label, ActionName(action), _protectedRoot, fullPath);
                    if (nextOffset == 0) break;
                    if (nextOffset < 12 || nextOffset > total - offset)
                        throw new InvalidDataException("Invalid FILE_NOTIFY_INFORMATION next offset " + nextOffset + " at offset " + offset);
                    offset = checked(offset + (int)nextOffset);
                }
            }

            private void Run()
            {
                bool armed = false;
                try
                {
                    while (true)
                    {
                        BeginRead();
                        armed = true;
                        _ready.Set();
                        uint wait = WaitForMultipleObjects(2, new[] { _ioEvent, _stopEvent }, false, Infinite);
                        if (wait == WaitFailed)
                            throw new Win32Exception(Marshal.GetLastWin32Error(), "WaitForMultipleObjects(directory watcher) failed");
                        if (wait == WaitObject0)
                        {
                            ConsumeCompletion(false);
                            armed = false;
                            if (Volatile.Read(ref _stopRequested) != 0) break;
                            continue;
                        }
                        if (wait == WaitObject0 + 1)
                        {
                            CancelAndDrainPending();
                            armed = false;
                            break;
                        }
                        throw new InvalidOperationException("Unexpected directory watcher wait result: " + wait);
                    }
                }
                catch (Exception exception)
                {
                    if (!_ready.IsSet) _startupError = exception;
                    _owner.RecordError(_label, exception.Message);
                    if (armed)
                    {
                        try
                        {
                            CancelAndDrainPending();
                            armed = false;
                        }
                        catch (Exception drainException)
                        {
                            Interlocked.Exchange(ref _undrainedIo, 1);
                            _owner.RecordError(_label, "pending I/O drain after watcher failure also failed: " + drainException.Message);
                        }
                    }
                }
                finally
                {
                    _ready.Set();
                }
            }

            private void RequestStop()
            {
                if (Interlocked.Exchange(ref _stopRequested, 1) == 0 && _stopEvent != IntPtr.Zero)
                {
                    if (!SetEvent(_stopEvent)) _owner.RecordError(_label, "SetEvent(stop) failed: " + Marshal.GetLastWin32Error());
                }
            }

            internal void StopAndDrain(int timeoutMilliseconds)
            {
                Volatile.Write(ref _drainTimeoutMilliseconds, Math.Max(1, timeoutMilliseconds));
                RequestStop();
                if (_thread != null && !_thread.Join(timeoutMilliseconds))
                {
                    if (_directory != InvalidHandleValue && _overlapped != IntPtr.Zero) CancelIoEx(_directory, _overlapped);
                    if (!_thread.Join(Math.Min(timeoutMilliseconds, 1000)))
                    {
                        _owner.RecordError(_label, "owned ReadDirectoryChangesW thread did not drain within " + timeoutMilliseconds + "ms");
                        return;
                    }
                }
                if (Volatile.Read(ref _undrainedIo) != 0)
                {
                    _owner.RecordError(_label, "native watcher resources retained because pending I/O was not proven drained");
                    return;
                }
                Cleanup();
            }

            private void Cleanup()
            {
                if (Interlocked.Exchange(ref _cleaned, 1) != 0) return;
                if (_overlapped != IntPtr.Zero) { Marshal.FreeHGlobal(_overlapped); _overlapped = IntPtr.Zero; }
                if (_buffer != IntPtr.Zero) { Marshal.FreeHGlobal(_buffer); _buffer = IntPtr.Zero; }
                if (_stopEvent != IntPtr.Zero) { CloseHandle(_stopEvent); _stopEvent = IntPtr.Zero; }
                if (_ioEvent != IntPtr.Zero) { CloseHandle(_ioEvent); _ioEvent = IntPtr.Zero; }
                if (_directory != InvalidHandleValue) { CloseHandle(_directory); _directory = InvalidHandleValue; }
                _ready.Dispose();
            }
        }
    }
}
'@
  $loadedJobType = 'MichStartupMaster.TestHarnessV4.TestProcessJob' -as [type]
  $loadedJournalType = 'MichStartupMaster.TestHarnessV4.ProtectedPathJournal' -as [type]
  $loadedJobMarkerField = if ($null -ne $loadedJobType) { $loadedJobType.GetField('ImplementationMarker') } else { $null }
  $loadedJournalMarkerField = if ($null -ne $loadedJournalType) { $loadedJournalType.GetField('ImplementationMarker') } else { $null }
  $loadedJobMarker = if ($null -ne $loadedJobMarkerField) { [string]$loadedJobMarkerField.GetRawConstantValue() } else { '' }
  $loadedJournalMarker = if ($null -ne $loadedJournalMarkerField) { [string]$loadedJournalMarkerField.GetRawConstantValue() } else { '' }
  if ($null -eq $loadedJobType -or $null -eq $loadedJournalType -or
      -not [string]::Equals($loadedJobMarker, $expectedImplementationMarker, [StringComparison]::Ordinal) -or
      -not [string]::Equals($loadedJournalMarker, $expectedImplementationMarker, [StringComparison]::Ordinal)) {
    throw "Native containment/journal types did not load at the expected implementation. expected=$expectedImplementationMarker job=$loadedJobMarker journal=$loadedJournalMarker"
  }
}

function Invoke-AppCommand {
  param(
    [Parameter(Mandatory = $true)][string[]]$Arguments,
    [int]$TimeoutSeconds = 120,
    [switch]$AllowFailure
  )

  if (-not $script:AppIsolationInitialized) {
    throw 'Product execution is blocked until the harness establishes an isolated state root.'
  }
  $currentStateRoot = [Environment]::GetEnvironmentVariable('MSM_STATE_ROOT', [EnvironmentVariableTarget]::Process)
  $currentKnownStore = [Environment]::GetEnvironmentVariable('MSM_KNOWN_STORE', [EnvironmentVariableTarget]::Process)
  if ([string]::IsNullOrWhiteSpace($currentStateRoot) -or [string]::IsNullOrWhiteSpace($currentKnownStore)) {
    throw 'Product execution is blocked because the isolated state environment is incomplete.'
  }
  $resolvedCurrentStateRoot = [IO.Path]::GetFullPath($currentStateRoot).TrimEnd('\')
  $resolvedActiveStateRoot = [IO.Path]::GetFullPath($script:ActiveAppStateRoot).TrimEnd('\')
  $resolvedCurrentKnownStore = [IO.Path]::GetFullPath($currentKnownStore)
  if (-not [string]::Equals($resolvedCurrentStateRoot, $resolvedActiveStateRoot, [StringComparison]::OrdinalIgnoreCase) -or
      [IO.Path]::GetFileName($resolvedCurrentStateRoot) -notlike 'MichStartupMaster-test-*' -or
      -not $resolvedCurrentKnownStore.StartsWith($resolvedCurrentStateRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw "Product execution is blocked because its state paths escaped the active test root. root=$resolvedCurrentStateRoot known=$resolvedCurrentKnownStore"
  }

  Initialize-TestProcessContainment
  $script:CommandNumber++
  $stem = '{0:D2}-{1}' -f $script:CommandNumber, (($Arguments[0] -replace '^--', '') -replace '[^A-Za-z0-9_.-]', '-')
  $stdoutPath = Join-Path $RunDir ($stem + '.stdout.txt')
  $stderrPath = Join-Path $RunDir ($stem + '.stderr.txt')
  $argumentLine = (@($Arguments) | ForEach-Object { ConvertTo-NativeArgument ([string]$_) }) -join ' '
  $started = Get-Date
  $containedProcess = $null
  $exitCode = -1
  $createdPid = 0
  $createdPath = $App
  $createdStartUtc = [DateTime]::MinValue
  try {
    $containedProcess = [MichStartupMaster.TestHarnessV4.TestProcessJob]::StartSuspendedAndAssign($App, $argumentLine, $Root, $stdoutPath, $stderrPath)
    $createdPid = $containedProcess.ProcessId
    $createdStartUtc = $containedProcess.StartTimeUtc
    $createdPath = $containedProcess.Path
    $createdIdentity = "pid=$createdPid startUtc=$($createdStartUtc.ToString('o')) path=$createdPath"
    if (-not $containedProcess.WaitForExit($TimeoutSeconds * 1000)) {
      $containedProcess.TerminateAndVerify(5000)
      throw "Command timed out after ${TimeoutSeconds}s: $($Arguments -join ' ') (contained root $createdIdentity and descendants terminated)"
    }
    $exitCode = $containedProcess.GetExitCode()
  }
  finally {
    if ($null -ne $containedProcess) { $containedProcess.CloseAndVerifyNoSurvivors(5000) }
  }
  $stdout = if (Test-Path -LiteralPath $stdoutPath) { Get-Content -LiteralPath $stdoutPath -Raw } else { '' }
  $stderr = if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw } else { '' }
  $durationMs = [int]((Get-Date) - $started).TotalMilliseconds
  if (-not $AllowFailure -and $exitCode -ne 0) {
    throw "Command failed with exit ${exitCode}: $($Arguments -join ' ')`n$stderr`n$stdout"
  }
  [pscustomobject]@{
    Arguments = @($Arguments)
    ExitCode = $exitCode
    Output = [string]$stdout
    Error = [string]$stderr
    DurationMs = $durationMs
    StdoutPath = $stdoutPath
    StderrPath = $stderrPath
  }
}

function Invoke-ConcurrentAppCommands {
  param(
    [Parameter(Mandatory = $true)][object[]]$ArgumentSets,
    [int]$TimeoutSeconds = 120
  )
  if (-not $script:AppIsolationInitialized) { throw 'Concurrent product execution is blocked until the harness establishes an isolated state root.' }
  $currentStateRoot = [Environment]::GetEnvironmentVariable('MSM_STATE_ROOT', [EnvironmentVariableTarget]::Process)
  $currentKnownStore = [Environment]::GetEnvironmentVariable('MSM_KNOWN_STORE', [EnvironmentVariableTarget]::Process)
  $resolvedCurrentStateRoot = [IO.Path]::GetFullPath($currentStateRoot).TrimEnd('\')
  $resolvedActiveStateRoot = [IO.Path]::GetFullPath($script:ActiveAppStateRoot).TrimEnd('\')
  $resolvedCurrentKnownStore = [IO.Path]::GetFullPath($currentKnownStore)
  if ([string]::IsNullOrWhiteSpace($currentStateRoot) -or [string]::IsNullOrWhiteSpace($currentKnownStore) -or
      -not [string]::Equals($resolvedCurrentStateRoot, $resolvedActiveStateRoot, [StringComparison]::OrdinalIgnoreCase) -or
      [IO.Path]::GetFileName($resolvedCurrentStateRoot) -notlike 'MichStartupMaster-test-*' -or
      -not $resolvedCurrentKnownStore.StartsWith($resolvedCurrentStateRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Concurrent product execution is blocked because the isolated state environment is incomplete or escaped.'
  }
  Initialize-TestProcessContainment
  $jobs = New-Object 'Collections.Generic.List[object]'
  $results = New-Object 'Collections.Generic.List[object]'
  try {
    $index = 0
    foreach ($arguments in $ArgumentSets) {
      $script:CommandNumber++
      $stdoutPath = Join-Path $RunDir ('{0:D2}-concurrent-{1}.stdout.txt' -f $script:CommandNumber, $index)
      $stderrPath = Join-Path $RunDir ('{0:D2}-concurrent-{1}.stderr.txt' -f $script:CommandNumber, $index)
      $argumentLine = (@($arguments) | ForEach-Object { ConvertTo-NativeArgument ([string]$_) }) -join ' '
      $job = [MichStartupMaster.TestHarnessV4.TestProcessJob]::StartSuspendedAndAssign($App, $argumentLine, $Root, $stdoutPath, $stderrPath)
      [void]$jobs.Add([pscustomobject]@{ Job = $job; Arguments = @($arguments); Stdout = $stdoutPath; Stderr = $stderrPath })
      $index++
    }
    foreach ($entry in $jobs) {
      if (-not $entry.Job.WaitForExit($TimeoutSeconds * 1000)) { $entry.Job.TerminateAndVerify(5000); throw "Concurrent command timed out: $($entry.Arguments -join ' ')" }
      $exitCode = $entry.Job.GetExitCode()
      $stdout = if (Test-Path -LiteralPath $entry.Stdout) { Get-Content -LiteralPath $entry.Stdout -Raw } else { '' }
      $stderr = if (Test-Path -LiteralPath $entry.Stderr) { Get-Content -LiteralPath $entry.Stderr -Raw } else { '' }
      if ($exitCode -ne 0) { throw "Concurrent command failed with exit $exitCode`: $($entry.Arguments -join ' ')`n$stderr`n$stdout" }
      [void]$results.Add([pscustomobject]@{ Arguments = $entry.Arguments; ExitCode = $exitCode; Output = [string]$stdout; Error = [string]$stderr })
    }
  }
  finally { foreach ($entry in $jobs) { if ($null -ne $entry.Job) { $entry.Job.CloseAndVerifyNoSurvivors(5000) } } }
  $results.ToArray()
}

function Assert-HarnessIsolationOrder {
  $tokens = $null
  $parseErrors = $null
  $ast = [Management.Automation.Language.Parser]::ParseFile($PSCommandPath, [ref]$tokens, [ref]$parseErrors)
  if (@($parseErrors).Count -ne 0) { throw "Harness isolation-order check could not parse its own source: $($parseErrors[0].Message)" }

  $readyAssignments = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.AssignmentStatementAst] -and
      $node.Left.Extent.Text -eq '$script:AppIsolationInitialized' -and
      $node.Right.Extent.Text -eq '$true'
  }, $true) | Sort-Object { $_.Extent.StartOffset })
  $appInvocations = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -and
      [string]::Equals([string]$node.GetCommandName(), 'Invoke-AppCommand', [StringComparison]::OrdinalIgnoreCase)
  }, $true) | Sort-Object { $_.Extent.StartOffset })
  if ($readyAssignments.Count -ne 1 -or $appInvocations.Count -lt 1 -or
      $readyAssignments[0].Extent.StartOffset -ge $appInvocations[0].Extent.StartOffset) {
    throw 'Static isolation regression: an app invocation can precede the single isolation-ready assignment.'
  }

  $invokeDefinition = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-AppCommand'
  }, $true))
  if ($invokeDefinition.Count -ne 1) { throw 'Static isolation regression: Invoke-AppCommand must have one definition.' }
  $concurrentDefinition = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-ConcurrentAppCommands'
  }, $true))
  if ($concurrentDefinition.Count -ne 1) { throw 'Static isolation regression: Invoke-ConcurrentAppCommands must have one definition.' }
  $invokeBodyText = $invokeDefinition[0].Body.Extent.Text
  $guardOffset = $invokeBodyText.IndexOf('AppIsolationInitialized', [StringComparison]::Ordinal)
  $nativeLaunchOffset = $invokeBodyText.IndexOf('StartSuspendedAndAssign', [StringComparison]::Ordinal)
  if ($guardOffset -lt 0 -or $nativeLaunchOffset -lt 0 -or $guardOffset -ge $nativeLaunchOffset -or
      [regex]::Matches($invokeBodyText, 'StartSuspendedAndAssign').Count -ne 1) {
    throw 'Static isolation regression: Invoke-AppCommand must enforce isolation before its single suspended product launch.'
  }
  $concurrentBodyText = $concurrentDefinition[0].Body.Extent.Text
  $concurrentGuardOffset = $concurrentBodyText.IndexOf('AppIsolationInitialized', [StringComparison]::Ordinal)
  $concurrentLaunchOffset = $concurrentBodyText.IndexOf('StartSuspendedAndAssign', [StringComparison]::Ordinal)
  if ($concurrentGuardOffset -lt 0 -or $concurrentLaunchOffset -lt 0 -or $concurrentGuardOffset -ge $concurrentLaunchOffset -or
      [regex]::Matches($concurrentBodyText, 'StartSuspendedAndAssign').Count -ne 1) {
    throw 'Static isolation regression: Invoke-ConcurrentAppCommands must enforce isolation before its suspended product launches.'
  }
  $unguardedProductStarts = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -and
      [string]::Equals([string]$node.GetCommandName(), 'Start-Process', [StringComparison]::OrdinalIgnoreCase) -and
      $node.Extent.Text -match '(?i)(?:^|\s)-FilePath\s+\$App(?:\s|$)'
  }, $true))
  $directAppInvocations = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -and
      $node.CommandElements.Count -gt 0 -and
      $node.CommandElements[0] -is [Management.Automation.Language.VariableExpressionAst] -and
      $node.CommandElements[0].VariablePath.UserPath -eq 'App'
  }, $true))
  if ($unguardedProductStarts.Count -ne 0 -or $directAppInvocations.Count -ne 0) {
    throw 'Static isolation regression: every product launch must pass through the guarded suspended-launch path.'
  }
  $nativeLaunches = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.InvokeMemberExpressionAst] -and
      [string]::Equals([string]$node.Member.Extent.Text, 'StartSuspendedAndAssign', [StringComparison]::Ordinal)
  }, $true))
  $nativeLaunchesOutsideGuards = @($nativeLaunches | Where-Object {
    -not (($_.Extent.StartOffset -ge $invokeDefinition[0].Extent.StartOffset -and $_.Extent.EndOffset -le $invokeDefinition[0].Extent.EndOffset) -or
      ($_.Extent.StartOffset -ge $concurrentDefinition[0].Extent.StartOffset -and $_.Extent.EndOffset -le $concurrentDefinition[0].Extent.EndOffset))
  })
  if ($nativeLaunches.Count -ne 2 -or $nativeLaunchesOutsideGuards.Count -ne 0) {
    throw 'Static isolation regression: native product launches must remain inside the two guarded invocation helpers.'
  }
  $nativeTypeCommands = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -and
      [string]::Equals([string]$node.GetCommandName(), 'Add-Type', [StringComparison]::OrdinalIgnoreCase) -and
      @($node.CommandElements | Where-Object {
        $_ -is [Management.Automation.Language.CommandParameterAst] -and
          [string]::Equals([string]$_.ParameterName, 'TypeDefinition', [StringComparison]::OrdinalIgnoreCase)
      }).Count -eq 1
  }, $true))
  if ($nativeTypeCommands.Count -ne 1) { throw 'Static isolation regression: the native containment helper must have one TypeDefinition.' }
  $nativeSourceLiterals = @($nativeTypeCommands[0].CommandElements | Where-Object {
    $_ -is [Management.Automation.Language.StringConstantExpressionAst]
  } | Sort-Object { $_.Value.Length } -Descending)
  if ($nativeSourceLiterals.Count -lt 1) { throw 'Static isolation regression: native containment source literal is missing.' }
  $nativeSourceText = [string]$nativeSourceLiterals[0].Value
  $createSuspendedOffset = $nativeSourceText.IndexOf('if (!CreateProcess(', [StringComparison]::Ordinal)
  $handleListOffset = $nativeSourceText.IndexOf('if (!UpdateProcThreadAttribute(', [StringComparison]::Ordinal)
  $assignSuspendedOffset = $nativeSourceText.IndexOf('if (!AssignProcessToJobObject(job, processInformation.Process))', [StringComparison]::Ordinal)
  $resumeSuspendedOffset = $nativeSourceText.IndexOf('if (ResumeThread(processInformation.Thread)', [StringComparison]::Ordinal)
  if ($handleListOffset -lt 0 -or $createSuspendedOffset -le $handleListOffset -or
      $nativeSourceText.IndexOf('CreateSuspended | CreateNoWindow | ExtendedStartupInfoPresent', [StringComparison]::Ordinal) -lt 0 -or
      $assignSuspendedOffset -le $createSuspendedOffset -or $resumeSuspendedOffset -le $assignSuspendedOffset) {
    throw 'Static isolation regression: native product creation must use an explicit inherited-handle list, remain suspended through job assignment, and resume only afterward.'
  }
  foreach ($requiredFragment in @(
      'namespace MichStartupMaster.TestHarnessV4',
      'suspended-job-handle-list-direct-journal-v4',
      'ReadDirectoryChangesW(',
      'CancelAndDrainPending()',
      'GetOverlappedResult(',
      'WaitForSingleObject(',
      'NotifyChangeSecurity',
      'NotifyChangeStreamName',
      'NotifyChangeStreamSize',
      'NotifyChangeStreamWrite')) {
    if ($nativeSourceText.IndexOf($requiredFragment, [StringComparison]::Ordinal) -lt 0) {
      throw "Static isolation regression: required containment/evidence fragment is missing: $requiredFragment"
    }
  }
  $legacyWatcherTypeName = 'FileSystem' + 'Watcher'
  if ($nativeSourceText.IndexOf($legacyWatcherTypeName, [StringComparison]::Ordinal) -ge 0) {
    throw 'Static isolation regression: protected-path journaling must use its owned native completion-and-drain path.'
  }
  $manifestDefinitions = @($ast.FindAll({
    param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-AppDataEvidence'
  }, $true))
  if ($manifestDefinitions.Count -ne 1 -or
      $manifestDefinitions[0].Body.Extent.Text.IndexOf('OwnerAndDaclSddl', [StringComparison]::Ordinal) -lt 0 -or
      $manifestDefinitions[0].Body.Extent.Text.IndexOf('AlternateStreams', [StringComparison]::Ordinal) -lt 0) {
    throw 'Static isolation regression: the real AppData manifest must retain owner/DACL and alternate-stream evidence.'
  }
  "PASS static-app-isolation readyOffset=$($readyAssignments[0].Extent.StartOffset) firstInvocationOffset=$($appInvocations[0].Extent.StartOffset) runtimeGate=true singleSuspendedLaunch=true handleAllowList=true assignedBeforeResume=true ownedNativeJournal=true securityEvidence=true"
}

function Get-AlternateStreamSha256 {
  param([Parameter(Mandatory = $true)][string]$LiteralPath)

  $share = [IO.FileShare]([int][IO.FileShare]::ReadWrite -bor [int][IO.FileShare]::Delete)
  $stream = [IO.File]::Open($LiteralPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
  $sha256 = [Security.Cryptography.SHA256]::Create()
  try {
    return [BitConverter]::ToString($sha256.ComputeHash($stream)).Replace('-', '')
  }
  finally {
    $sha256.Dispose()
    $stream.Dispose()
  }
}

function Get-AlternateStreamEvidence {
  param([Parameter(Mandatory = $true)][string]$LiteralPath)

  $records = New-Object 'Collections.Generic.List[object]'
  foreach ($streamInfo in @(Get-Item -LiteralPath $LiteralPath -Stream * -ErrorAction Stop | Sort-Object Stream)) {
    $streamName = [string]$streamInfo.Stream
    if ($streamName -eq ':$DATA' -or $streamName -eq '$DATA') { continue }
    $streamPath = if ($streamName.StartsWith(':', [StringComparison]::Ordinal)) {
      $LiteralPath + $streamName
    } else {
      $LiteralPath + ':' + $streamName
    }
    [void]$records.Add([pscustomobject]@{
      Name = $streamName
      Length = [long]$streamInfo.Length
      Sha256 = Get-AlternateStreamSha256 -LiteralPath $streamPath
    })
  }
  return @($records.ToArray())
}

function Get-AppDataEvidence {
  param([Parameter(Mandatory = $true)][string]$Path)

  $resolvedRoot = [IO.Path]::GetFullPath($Path).TrimEnd('\')
  if (-not (Test-Path -LiteralPath $resolvedRoot)) {
    return ([pscustomobject]@{ Exists = $false; Entries = @() } | ConvertTo-Json -Compress -Depth 6)
  }
  if (-not (Test-Path -LiteralPath $resolvedRoot -PathType Container)) { throw "Real app-data root is not a directory: $resolvedRoot" }

  $items = New-Object 'Collections.Generic.List[object]'
  [void]$items.Add((Get-Item -LiteralPath $resolvedRoot -Force))
  foreach ($child in @(Get-ChildItem -LiteralPath $resolvedRoot -Force -Recurse -ErrorAction Stop)) { [void]$items.Add($child) }
  $records = New-Object 'Collections.Generic.List[object]'
  $securitySections = [Security.AccessControl.AccessControlSections]::Owner -bor [Security.AccessControl.AccessControlSections]::Access
  foreach ($item in @($items.ToArray() | Sort-Object FullName)) {
    $fullName = [IO.Path]::GetFullPath([string]$item.FullName)
    $relative = if ([string]::Equals($fullName.TrimEnd('\'), $resolvedRoot, [StringComparison]::OrdinalIgnoreCase)) {
      '.'
    } else {
      $fullName.Substring($resolvedRoot.Length).TrimStart('\')
    }
    $isDirectory = [bool]$item.PSIsContainer
    $security = Get-Acl -LiteralPath $fullName -ErrorAction Stop
    [void]$records.Add([pscustomobject]@{
      Path = $relative
      Kind = if ($isDirectory) { 'Directory' } else { 'File' }
      Length = if ($isDirectory) { 0L } else { [long]$item.Length }
      Attributes = [int]$item.Attributes
      CreationUtcTicks = [long]$item.CreationTimeUtc.Ticks
      LastWriteUtcTicks = [long]$item.LastWriteTimeUtc.Ticks
      Sha256 = if ($isDirectory) { '' } else { [string](Get-FileHash -Algorithm SHA256 -LiteralPath $fullName).Hash }
      OwnerAndDaclSddl = [string]$security.GetSecurityDescriptorSddlForm($securitySections)
      AlternateStreams = @(Get-AlternateStreamEvidence -LiteralPath $fullName)
    })
  }
  return ([pscustomobject]@{ Exists = $true; Entries = @($records.ToArray()) } | ConvertTo-Json -Compress -Depth 6)
}

function Start-IsolationWriteJournal {
  if ($script:IsolationJournalStarted) { throw 'Isolation write journal was already started.' }
  Initialize-TestProcessContainment
  if (-not ('MichStartupMaster.TestHarnessV4.ProtectedPathJournal' -as [type])) { throw 'Protected-path journal type did not load.' }
  $script:IsolationWriteJournal = New-Object MichStartupMaster.TestHarnessV4.ProtectedPathJournal
  $script:IsolationJournalStarted = $true

  # The pure root must remain nonexistent, so watch its existing parent and filter every event to
  # the exact protected path. Receipt files are siblings and are intentionally outside that path.
  $script:IsolationWriteJournal.Add($RunDir, $false, $script:PureStateRoot, 'pure-state-root-parent')

  $realAppDataParent = Split-Path -Path $script:RealAppDataRoot -Parent
  $script:IsolationWriteJournal.Add($realAppDataParent, $false, $script:RealAppDataRoot, 'real-app-data-parent')
  if (Test-Path -LiteralPath $script:RealAppDataRoot -PathType Container) {
    $script:IsolationWriteJournal.Add($script:RealAppDataRoot, $true, $script:RealAppDataRoot, 'real-app-data-tree')
  }
}

function Get-IsolationWriteJournalEntries {
  if ($null -eq $script:IsolationWriteJournal) { return @() }
  return @($script:IsolationWriteJournal.Snapshot() | Sort-Object -Unique)
}

function Assert-IsolationWriteJournalClean {
  $entries = @(Get-IsolationWriteJournalEntries)
  if ($entries.Count -ne 0) {
    $sample = @($entries | Select-Object -First 10) -join ' | '
    throw "Protected-path write journal recorded $($entries.Count) event(s): $sample"
  }
}

function Stop-IsolationWriteJournal {
  if ($null -eq $script:IsolationWriteJournal) { $script:IsolationJournalStarted = $false; return @() }
  $journal = $script:IsolationWriteJournal
  try {
    return @($journal.StopAndSnapshot(5000) | Sort-Object -Unique)
  }
  finally {
    $journal.Dispose()
    $script:IsolationWriteJournal = $null
    $script:IsolationJournalStarted = $false
  }
}

function Assert-PureIsolationUnchanged {
  Assert-IsolationWriteJournalClean
  if (Test-Path -LiteralPath $script:PureStateRoot) {
    throw "A safe product probe wrote to its isolated state root: $($script:PureStateRoot)"
  }
  $realAppDataEvidenceAfter = Get-AppDataEvidence -Path $script:RealAppDataRoot
  if (-not [string]::Equals([string]$script:RealAppDataEvidenceBefore, [string]$realAppDataEvidenceAfter, [StringComparison]::Ordinal)) {
    throw "A safe product probe coincided with a write under the real app-data root: $($script:RealAppDataRoot)"
  }
  "PASS pure-state-isolation rootStayedAbsent=true realAppDataUnchanged=true root=$($script:PureStateRoot)"
}

function Assert-Match {
  param([string]$Value, [string]$Pattern, [string]$Message)
  if ($Value -notmatch $Pattern) { throw "$Message Receipt: $Value" }
}

function Get-ReceiptValue {
  param([string]$Line, [string]$Name)
  $match = [regex]::Match($Line, '(?:^|\s)' + [regex]::Escape($Name) + '=(?<value>[^\s]+)')
  if (-not $match.Success) { throw "Receipt is missing '$Name': $Line" }
  return $match.Groups['value'].Value
}

function Convert-AuditLine {
  param([string]$Line, [string]$Prefix)
  if ([string]::IsNullOrWhiteSpace($Line) -or -not $Line.StartsWith($Prefix + ' ', [StringComparison]::Ordinal)) {
    throw "Missing $Prefix header: $Line"
  }
  $values = @{}
  foreach ($match in [regex]::Matches($Line, '(?<key>[A-Za-z_]+)=(?<value>[^\s]+)')) {
    $values[$match.Groups['key'].Value] = $match.Groups['value'].Value
  }
  return $values
}

function New-StateStoreSnapshot {
  param([Parameter(Mandatory = $true)][string]$Path)
  $fullPath = [IO.Path]::GetFullPath($Path)
  $exists = Test-Path -LiteralPath $fullPath -PathType Leaf
  $item = if ($exists) { Get-Item -LiteralPath $fullPath } else { $null }
  [pscustomobject]@{
    Path = $fullPath
    Existed = $exists
    Bytes = if ($exists) { [IO.File]::ReadAllBytes($fullPath) } else { [byte[]]@() }
    CreationTimeUtc = if ($exists) { $item.CreationTimeUtc } else { [DateTime]::MinValue }
    LastWriteTimeUtc = if ($exists) { $item.LastWriteTimeUtc } else { [DateTime]::MinValue }
    Attributes = if ($exists) { $item.Attributes } else { [IO.FileAttributes]::Normal }
  }
}

function Restore-StateStoreSnapshot {
  param([Parameter(Mandatory = $true)]$Snapshot)
  $path = [string]$Snapshot.Path
  if (-not $Snapshot.Existed) {
    if (Test-Path -LiteralPath $path -PathType Leaf) { Remove-Item -LiteralPath $path -Force }
    return
  }

  $directory = Split-Path -Path $path -Parent
  if (-not (Test-Path -LiteralPath $directory -PathType Container)) { [void][IO.Directory]::CreateDirectory($directory) }
  $temporary = Join-Path $directory ((Split-Path -Path $path -Leaf) + '.test-restore.' + [Guid]::NewGuid().ToString('N'))
  try {
    [IO.File]::WriteAllBytes($temporary, [byte[]]$Snapshot.Bytes)
    if (Test-Path -LiteralPath $path -PathType Leaf) { [IO.File]::Replace($temporary, $path, $null, $true) }
    else { [IO.File]::Move($temporary, $path) }
    [IO.File]::SetCreationTimeUtc($path, [DateTime]$Snapshot.CreationTimeUtc)
    [IO.File]::SetLastWriteTimeUtc($path, [DateTime]$Snapshot.LastWriteTimeUtc)
    [IO.File]::SetAttributes($path, [IO.FileAttributes]$Snapshot.Attributes)
  }
  finally {
    if (Test-Path -LiteralPath $temporary -PathType Leaf) { Remove-Item -LiteralPath $temporary -Force }
  }
}

function Assert-StateStoreSnapshotRestored {
  param([Parameter(Mandatory = $true)]$Snapshot)
  $exists = Test-Path -LiteralPath $Snapshot.Path -PathType Leaf
  if ($exists -ne [bool]$Snapshot.Existed) { throw "State-store existence was not restored: $($Snapshot.Path)" }
  if (-not $exists) { return }
  $actual = [IO.File]::ReadAllBytes([string]$Snapshot.Path)
  $expectedText = [Convert]::ToBase64String([byte[]]$Snapshot.Bytes)
  $actualText = [Convert]::ToBase64String($actual)
  if (-not [string]::Equals($actualText, $expectedText, [StringComparison]::Ordinal)) { throw "State-store bytes were not restored: $($Snapshot.Path)" }
  $item = Get-Item -LiteralPath $Snapshot.Path
  if ($item.CreationTimeUtc -ne [DateTime]$Snapshot.CreationTimeUtc) { throw "State-store creation time was not restored: $($Snapshot.Path)" }
  if ($item.LastWriteTimeUtc -ne [DateTime]$Snapshot.LastWriteTimeUtc) { throw "State-store write time was not restored: $($Snapshot.Path)" }
  if ($item.Attributes -ne [IO.FileAttributes]$Snapshot.Attributes) { throw "State-store attributes were not restored: $($Snapshot.Path)" }
}

function Copy-RegistryValueData {
  param([AllowNull()]$Value)
  if ($Value -is [byte[]]) { return ,([byte[]]$Value.Clone()) }
  if ($Value -is [string[]]) { return ,([string[]]$Value.Clone()) }
  return $Value
}

function New-HkcuRegistryValueSnapshot {
  param(
    [Parameter(Mandatory = $true)][string]$SubKey,
    [Parameter(Mandatory = $true)][string]$Name
  )
  $key = $null
  try {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($SubKey, $false)
    $exists = $null -ne $key -and @($key.GetValueNames() | Where-Object { [string]::Equals($_, $Name, [StringComparison]::OrdinalIgnoreCase) }).Count -eq 1
    [pscustomobject]@{
      SubKey = $SubKey
      Name = $Name
      SubKeyExisted = $null -ne $key
      Existed = $exists
      Kind = if ($exists) { $key.GetValueKind($Name) } else { [Microsoft.Win32.RegistryValueKind]::None }
      Value = if ($exists) { Copy-RegistryValueData ($key.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)) } else { $null }
    }
  }
  finally { if ($null -ne $key) { $key.Dispose() } }
}

function Test-RegistryValueDataEqual {
  param([AllowNull()]$Left, [AllowNull()]$Right)
  if ($Left -is [byte[]] -and $Right -is [byte[]]) { return [Convert]::ToBase64String([byte[]]$Left) -ceq [Convert]::ToBase64String([byte[]]$Right) }
  if ($Left -is [string[]] -and $Right -is [string[]]) {
    if ($Left.Count -ne $Right.Count) { return $false }
    for ($index = 0; $index -lt $Left.Count; $index++) {
      if (-not [string]::Equals([string]$Left[$index], [string]$Right[$index], [StringComparison]::Ordinal)) { return $false }
    }
    return $true
  }
  return [object]::Equals($Left, $Right)
}

function Restore-HkcuRegistryValueSnapshot {
  param([Parameter(Mandatory = $true)]$Snapshot)
  if (-not [bool]$Snapshot.SubKeyExisted) { throw "Registry subkey was absent at snapshot time; refusing to create it during value restoration: HKCU\$($Snapshot.SubKey)" }
  $key = $null
  try {
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey([string]$Snapshot.SubKey)
    if ($Snapshot.Existed) { $key.SetValue([string]$Snapshot.Name, (Copy-RegistryValueData $Snapshot.Value), [Microsoft.Win32.RegistryValueKind]$Snapshot.Kind) }
    else { $key.DeleteValue([string]$Snapshot.Name, $false) }
  }
  finally { if ($null -ne $key) { $key.Dispose() } }
}

function Assert-HkcuRegistryValueSnapshotRestored {
  param([Parameter(Mandatory = $true)]$Snapshot)
  $actual = New-HkcuRegistryValueSnapshot -SubKey ([string]$Snapshot.SubKey) -Name ([string]$Snapshot.Name)
  if ([bool]$actual.SubKeyExisted -ne [bool]$Snapshot.SubKeyExisted) { throw "Registry subkey existence was not restored: HKCU\$($Snapshot.SubKey)" }
  if ([bool]$actual.Existed -ne [bool]$Snapshot.Existed) { throw "Registry value existence was not restored: HKCU\$($Snapshot.SubKey)\$($Snapshot.Name)" }
  if (-not $Snapshot.Existed) { return }
  if ($actual.Kind -ne $Snapshot.Kind) { throw "Registry value type was not restored: HKCU\$($Snapshot.SubKey)\$($Snapshot.Name)" }
  if (-not (Test-RegistryValueDataEqual $actual.Value $Snapshot.Value)) { throw "Registry value bytes were not restored: HKCU\$($Snapshot.SubKey)\$($Snapshot.Name)" }
}

function Test-IsTaskNotFoundError {
  param([Parameter(Mandatory = $true)]$ErrorRecord)
  $exception = $ErrorRecord.Exception
  while ($null -ne $exception) {
    if ($exception.HResult -eq -2147024894) { return $true }
    $exception = $exception.InnerException
  }
  return $false
}

function Get-ExactScheduledTask {
  param([Parameter(Mandatory = $true)][string]$TaskLocation)
  $lastSlash = $TaskLocation.LastIndexOf('\')
  if ($lastSlash -lt 0 -or $lastSlash -eq ($TaskLocation.Length - 1)) { throw "Invalid exact task location: $TaskLocation" }
  $taskPath = $TaskLocation.Substring(0, $lastSlash + 1)
  $comTaskPath = if ($taskPath.Length -eq 1) { '\' } else { $taskPath.TrimEnd('\') }
  $taskName = $TaskLocation.Substring($lastSlash + 1)
  $service = $null
  $folder = $null
  $task = $null
  try {
    $service = New-Object -ComObject 'Schedule.Service'
    $service.Connect()
    try { $folder = $service.GetFolder($comTaskPath) }
    catch {
      if (Test-IsTaskNotFoundError -ErrorRecord $_) { return $null }
      throw "Task Scheduler folder lookup failed closed for '$taskPath': $($_.Exception.Message)"
    }
    try { $task = $folder.GetTask($taskName) }
    catch {
      if (Test-IsTaskNotFoundError -ErrorRecord $_) { return $null }
      throw "Task Scheduler task lookup failed closed for '$TaskLocation': $($_.Exception.Message)"
    }
    [pscustomobject]@{
      Location = $TaskLocation
      State = if ([bool]$task.Enabled) { 'Enabled' } else { 'Disabled' }
      Xml = [string]$task.Xml
    }
  }
  finally {
    foreach ($comObject in @($task, $folder, $service)) {
      if ($null -ne $comObject -and [Runtime.InteropServices.Marshal]::IsComObject($comObject)) {
        try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($comObject) } catch { }
      }
    }
  }
}

function Get-ScheduledTaskFolderEntries {
  param([Parameter(Mandatory = $true)][string]$TaskPath)
  if (-not $TaskPath.StartsWith('\', [StringComparison]::Ordinal) -or -not $TaskPath.EndsWith('\', [StringComparison]::Ordinal)) { throw "Invalid exact task folder: $TaskPath" }
  $comTaskPath = if ($TaskPath.Length -eq 1) { '\' } else { $TaskPath.TrimEnd('\') }
  $service = $null
  $folder = $null
  $collection = $null
  $entries = New-Object 'Collections.Generic.List[object]'
  try {
    $service = New-Object -ComObject 'Schedule.Service'
    $service.Connect()
    try { $folder = $service.GetFolder($comTaskPath) }
    catch {
      if (Test-IsTaskNotFoundError -ErrorRecord $_) { return @() }
      throw "Task Scheduler folder enumeration failed closed for '$TaskPath': $($_.Exception.Message)"
    }
    $collection = $folder.GetTasks(1)
    for ($index = 1; $index -le $collection.Count; $index++) {
      $task = $null
      try {
        $task = $collection.Item($index)
        [void]$entries.Add([pscustomobject]@{
          Name = [string]$task.Name
          Location = [string]$task.Path
          State = if ([bool]$task.Enabled) { 'Enabled' } else { 'Disabled' }
          Xml = [string]$task.Xml
        })
      }
      finally {
        if ($null -ne $task -and [Runtime.InteropServices.Marshal]::IsComObject($task)) {
          try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($task) } catch { }
        }
      }
    }
    return @($entries.ToArray())
  }
  finally {
    foreach ($comObject in @($collection, $folder, $service)) {
      if ($null -ne $comObject -and [Runtime.InteropServices.Marshal]::IsComObject($comObject)) {
        try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($comObject) } catch { }
      }
    }
  }
}

function Assert-UiContract {
  $result = Invoke-AppCommand @('--ui-contract')
  try { $contract = $result.Output | ConvertFrom-Json } catch { throw "UI contract is not valid JSON: $($result.Output)" }
  if ($contract.layout -ne 'responsive-native-control-center') { throw 'UI must use the responsive native control-center layout.' }
  if (-not $contract.stateModeSeparated) { throw 'Startup state and window/tray mode must be separate.' }
  if (-not $contract.startInTrayPrePaintSuppression) { throw 'Start-in-tray must suppress the first visible frame.' }
  if (-not $contract.refreshIsReadOnly) { throw 'Refresh must be read-only.' }
  if (-not $contract.sortableColumns -or -not $contract.keyboardAccessibleSorting -or $contract.statusInitialSort -ne 'Enabled first' -or -not $contract.repeatedColumnClickReversesSort) { throw 'Every inventory column must sort by mouse or keyboard, with Enabled first on the initial Status click and reversal on repeat.' }
  if (-not $contract.humanReadableNames -or -not $contract.appsAggregatedByInstalledProductOrCanonicalTarget -or -not $contract.appsNeverAggregatedByDisplayName -or -not $contract.allRoutesRemainRouteLevel) { throw 'The UI must expose readable names, one installed-product app row when ownership is proven, canonical-target fallback rows, and every exact underlying route.' }
  if (-not $contract.aggregateNonBulkActionsFailClosed -or -not $contract.aggregateBulkDisableTransactional -or -not $contract.aggregateBulkEnableTransactional -or -not $contract.aggregateManageRoutesOneClick) { throw 'Aggregate rows must provide transactional bulk enable/disable while every ambiguous non-bulk action fails closed.' }
  if (-not $contract.contextualActions -or -not $contract.globalToolsInMenu) { throw 'Context actions and global tools must be separated.' }
  if (-not $contract.keyboardShortcuts -or -not $contract.accessibilityNames -or -not $contract.emptyLoadingErrorStates) { throw 'UI accessibility/state contracts are incomplete.' }
  if ($contract.defaultNewMode -ne 'Window') { throw 'New startup entries must default to Window mode.' }
  if (-not $contract.actualApplicationTrayIconOnly) { throw 'Quiet mode must expose only the actual application identity in the system tray.' }
  if (-not $contract.capabilityAwareActions -or -not $contract.expertBootChangeConfirmation -or -not $contract.elevationAndRebootReasons -or -not $contract.externalAuthorityState -or -not $contract.multiActionTasksDisableEditQuietAndRun) { throw 'Advanced boot-route capability, confirmation, authority, and multi-action UI contracts are incomplete.' }

  $columns = @($contract.columns)
  $statusIndex = [array]::IndexOf($columns, 'Status')
  $modeIndex = [array]::IndexOf($columns, 'Mode')
  if ($statusIndex -lt 0 -or $modeIndex -lt 0 -or $statusIndex -eq $modeIndex) { throw 'Status and Mode must be separate primary columns.' }
  if (@($columns) -notcontains 'Risk') { throw 'The heuristic severity column must be labeled Risk, not startup impact.' }
  foreach ($detail in @('Location', 'Launch command')) {
    if (@($contract.detailFields) -notcontains $detail) { throw "UI detail field missing: $detail" }
  }
  foreach ($mode in @('Window', 'Quiet (tray)', 'Not supported')) {
    if (@($contract.modeLabels) -notcontains $mode) { throw "UI mode missing: $mode" }
  }
  foreach ($filter in @('Apps', 'All routes', 'Needs attention', 'Disabled')) {
    if (@($contract.filters) -notcontains $filter) { throw "UI filter missing: $filter" }
  }
  foreach ($tool in @('Add startup', 'Edit startup', 'Disable at boot', 'Enable at boot', 'Quiet (tray)', 'Window mode', 'Run now', 'Open location', 'Copy command', 'Verify coverage', 'Repair startup rules')) {
    if (@($contract.tools) -notcontains $tool) { throw "UI tool missing: $tool" }
  }
  "PASS ui-contract columns=$($columns.Count) filters=$(@($contract.filters).Count)"

  $selfTest = Invoke-AppCommand @('--ui-self-test')
  Assert-Match $selfTest.Output 'UI_SELF_TEST passed' 'UI self-test did not pass.'
  Assert-Match $selfTest.Output 'scales=100,150,200' 'UI self-test did not cover all required scales.'
  Assert-Match $selfTest.Output 'startInTrayInitialVisible=false' 'Start-in-tray painted an initial frame.'
  foreach ($requiredReceipt in @('capabilityAwareActions=true', 'expertConfirmation=true', 'externalAuthority=true', 'multiActionFailClosed=true')) {
    if ($selfTest.Output -notmatch [regex]::Escape($requiredReceipt)) { throw "UI self-test is missing '$requiredReceipt': $($selfTest.Output)" }
  }
  "PASS ui-self-test $($selfTest.Output.Trim())"
}

function ConvertFrom-JsonArray {
  param(
    [Parameter(Mandatory = $true)][string]$Json,
    [Parameter(Mandatory = $true)][string]$Context
  )

  try { $decoded = $Json | ConvertFrom-Json -ErrorAction Stop }
  catch { throw "$Context is not valid JSON: $($Json.Substring(0, [Math]::Min(500, $Json.Length)))" }
  if ($null -eq $decoded) { return }
  if ($decoded -is [Array]) {
    foreach ($row in $decoded) { $row }
    return
  }
  $decoded
}

function Test-QuietLaunchContract {
  $fixture = Join-Path $RunDir 'quiet-launch-fixture'
  New-Item -ItemType Directory -Force -Path $fixture | Out-Null
  $speedy = Join-Path $fixture 'Speedy.exe'
  $ordinary = Join-Path $fixture 'OrdinaryApp.exe'
  New-Item -ItemType File -Force -Path $speedy | Out-Null
  New-Item -ItemType File -Force -Path $ordinary | Out-Null
  $legacyVbs = Join-Path $fixture 'openspeedy-silent.vbs'
  [IO.File]::WriteAllText($legacyVbs, "Option Explicit`r`nDim exe`r`nexe = `"$speedy`"`r`n", [Text.Encoding]::UTF8)

  function Get-QuietPlan([string]$Target, [string]$Arguments) {
    $payload = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Target + "`n" + $Arguments))
    $result = Invoke-AppCommand @('--quiet-plan', $payload)
    try { return $result.Output | ConvertFrom-Json } catch { throw "Quiet plan is not valid JSON: $($result.Output)" }
  }

  $windowsRoot = if (-not [string]::IsNullOrWhiteSpace($env:WINDIR)) { $env:WINDIR } elseif (-not [string]::IsNullOrWhiteSpace($env:SystemRoot)) { $env:SystemRoot } else { 'C:\Windows' }
  $wscript = Join-Path $windowsRoot 'System32\wscript.exe'
  $legacy = Get-QuietPlan $wscript ('"' + $legacyVbs + '"')
  if ($legacy.strategy -ne 'native-tray' -or $legacy.usesWrapper -ne $false) { throw "OpenSpeedy legacy chain did not migrate to native tray: $($legacy | ConvertTo-Json -Compress)" }
  if (-not [IO.Path]::GetFullPath([string]$legacy.target).Equals([IO.Path]::GetFullPath($speedy), [StringComparison]::OrdinalIgnoreCase)) { throw "OpenSpeedy target was not unwrapped: $($legacy.target)" }
  if ([string]$legacy.arguments -notmatch '(?i)(^|\s)--minimize-to-tray(\s|$)') { throw "OpenSpeedy tray flag is missing: $($legacy.arguments)" }

  $generic = Get-QuietPlan $ordinary ''
  if ($generic.strategy -ne 'managed-proxy' -or $generic.usesWrapper -ne $true) { throw "Generic quiet plan must use boot-only managed proxy suppression: $($generic | ConvertTo-Json -Compress)" }
  'PASS quiet-plan OpenSpeedy=native-tray generic=managed-proxy'

  $policyResult = Invoke-AppCommand @('--quiet-policy-probe')
  try { $policy = $policyResult.Output | ConvertFrom-Json } catch { throw "Quiet policy probe is not valid JSON: $($policyResult.Output)" }
  if ($policy.passed -ne $true -or
      $policy.firstWindowHidden -ne $true -or
      $policy.automaticSameWindowReshowHidden -ne $true -or
      [int]$policy.automaticReshowHideCount -lt 2 -or
      $policy.delayedSecondWindowHidden -ne $true -or
      $policy.veryLateWindowAllowed -ne $true -or
      $policy.bootstrapCompletionCancelsSuppression -ne $true -or
      $policy.nativeTrayReadinessEndsBootstrap -ne $true -or
      $policy.nativeTrayReadinessWaitsForInitialWindow -ne $true -or
      $policy.chatGptTrayHostRecognized -ne $true -or
      $policy.winFormsTrayHostRecognized -ne $true -or
      $policy.winFormsMainWindowNotTrayHost -ne $true -or
      $policy.chatGptFilenameDoesNotProveTray -ne $true -or
      $policy.runtimeNativeTrayHostOverridesStaticMiss -ne $true -or
      $policy.runtimeNativeTrayHostRemovesFallback -ne $true -or
      $policy.fallbackReadinessEndsBootstrap -ne $true -or
      $policy.launcherHandoffWaitsForRuntimeIcon -ne $true -or
      $policy.nativeControllerExitsAfterBootstrap -ne $true -or
      $policy.missingRuntimeTrayHostGetsFallback -ne $true -or
      $policy.explicitActivationPermanentlyCancelsSuppression -ne $true -or
      $policy.unrelatedInputFocusStealDoesNotCancelSuppression -ne $true -or
      [string]$policy.suppressionCancellationInputs -ne 'explicit-proxy-or-bootstrap-complete' -or
      [int]$policy.manualWindowRehideCount -ne 0 -or
      $policy.proxyOpenImmediateActivation -ne $true -or
      $policy.trackedWindowEventHiddenImmediately -ne $true -or
      $policy.untrackedWindowEventIgnored -ne $true -or
      $policy.liveLineageNeverRelaunched -ne $true -or
      $policy.pendingWindowRestoredOnce -ne $true -or
      $policy.rejectedRestoreKeepsPending -ne $true -or
      $policy.pendingCaptureIsBounded -ne $true -or
      $policy.pendingManualOverlapKeepsObserver -ne $true -or
      $policy.nonSingleInstanceLaunchExactlyOnce -ne $true -or
      $policy.fallbackIconExactlyOne -ne $true -or
      [string]$policy.fallbackIconIdentity -ne 'launch-payload-or-launch-handler-only' -or
      $policy.startupMasterIconFallback -ne $false -or
      $policy.nativeTrayCapabilityUsesNoProxy -ne $true -or
      $policy.managedPlanOwnsOneProxy -ne $true -or
      $policy.launcherPayloadRouteRecognized -ne $true -or
      $policy.launcherPayloadRejectsOrdinaryName -ne $true -or
      $policy.launcherPayloadExecutablePathRecognized -ne $true -or
      $policy.launcherPayloadBasenameOnlyRejected -ne $true -or
      $policy.managedNativeTraySignatureRecognized -ne $true -or
      $policy.nativeLaunchSerializedExactlyOnce -ne $true -or
      [int]$policy.serializedLaunchCount -ne 1 -or
      [int]$policy.nativeTrayReadyWindowGraceMs -ne 3000 -or
      [int]$policy.initialObservationMinimumMs -ne 10000 -or
      [int]$policy.initialObservationStabilityMs -ne 3000 -or
      [int]$policy.initialObservationMaximumMs -ne 20000) {
    throw "Quiet window policy does not prove boot-only suppression, post-bootstrap manual activation, exact one-launch behavior, and no re-hide: $($policyResult.Output)"
  }
  'PASS quiet-policy bootOnlySuppression=true trayReadyRelease=true nativeControllerExit=true runtimeFallback=true runtimeNativeWins=true chatGptNative=true veryLateAllowed=true explicitProxyCancel=true focusStealIgnored=true oneLaunch=true oneIcon=true manualRehideCount=0'

  $lineageResult = Invoke-AppCommand @('--quiet-lineage-self-test')
  try { $lineage = $lineageResult.Output | ConvertFrom-Json } catch { throw "Quiet lineage self-test is not valid JSON: $($lineageResult.Output)" }
  if ($lineage.passed -ne $true -or $lineage.bufferedMultiHopPromoted -ne $true -or $lineage.liveDescendantPromoted -ne $true -or $lineage.unrelatedIgnored -ne $true -or $lineage.unqualifiedPidsNotActionable -ne $true -or $lineage.snapshotDescendantsPromoted -ne $true -or $lineage.snapshotUnrelatedIgnored -ne $true -or $lineage.callbackMultiHopPromoted -ne $true -or $lineage.callbackUnrelatedIgnored -ne $true -or $lineage.exactArgumentIdentityMatched -ne $true -or $lineage.sameExecutableDifferentArgumentsIgnored -ne $true -or $lineage.argumentCaseDifferenceIgnored -ne $true -or $lineage.quotedArgumentBoundaryPreserved -ne $true -or $lineage.extraArgumentIgnored -ne $true -or $lineage.launchHashPathCaseInsensitive -ne $true -or $lineage.launchHashArgumentCaseSensitive -ne $true -or $lineage.launchHashQuotedBoundaryPreserved -ne $true -or [int]$lineage.snapshotCalls -ne 1 -or [int]$lineage.externalProcessesStarted -ne 0) {
    throw "Quiet lineage self-test failed: $($lineageResult.Output)"
  }
  $tracked = @($lineage.tracked | ForEach-Object { [int]$_ })
  if (($tracked -join ',') -ne '4100,4101,4102,4103') { throw "Quiet lineage tracked the wrong processes: $($tracked -join ',')" }
  $snapshotTracked = @($lineage.snapshotTracked | ForEach-Object { [int]$_ })
  if (($snapshotTracked -join ',') -ne '5100,5101,5102,5103') { throw "Quiet snapshot traversal tracked the wrong processes: $($snapshotTracked -join ',')" }
  'PASS quiet-lineage tracked=4100,4101,4102,4103 exactArgv=true callbackMultiHop=true snapshotCalls=1 externalProcessesStarted=0'

  $performanceResult = Invoke-AppCommand @('--quiet-performance-probe')
  try { $performance = $performanceResult.Output | ConvertFrom-Json } catch { throw "Quiet performance probe is not valid JSON: $($performanceResult.Output)" }
  if ($performance.passed -ne $true -or [int]$performance.steadyStateSystemSnapshots -ne 0 -or [int]$performance.steadyStateTopWindowEnumerations -ne 0 -or [int]$performance.callbackWholeSystemSnapshots -ne 0 -or $performance.steadyGlobalWindowHookRunning -ne $false -or $performance.steadyGlobalLineageWatcherRunning -ne $false -or [int]$performance.steadyScopedWindowHookCount -lt 1 -or $performance.steadyJobObserverRunning -ne $true -or $performance.watcherStoppedAfterInitial -ne $true -or $performance.windowHookStopped -ne $true -or $performance.timersDisposed -ne $true -or $performance.callbackDrainPreservesEvidence -ne $true -or [int]$performance.postDisposeCallbacksExecuted -ne 0 -or $performance.manualOpenImmediate -ne $true -or [int]$performance.manualWindowRehideCount -ne 0) {
    throw "Quiet steady-state performance/teardown probe failed: $($performanceResult.Output)"
  }
  'PASS quiet-performance steadySnapshots=0 steadyGlobalHooks=0 scopedHooks=true jobObserver=true disposed=true manualOpenImmediate=true'
}

function Test-SafeProductContracts {
  $truth = Invoke-AppCommand @('--truth-self-test')
  Assert-Match $truth.Output '^TRUTH_SELF_TEST checks=12 passed=12 codex=unverified contradictions=drifted missing=unknown' 'Startup truth regression failed.'
  "PASS startup-truth $($truth.Output.Trim())"

  $inventorySelfTest = Invoke-AppCommand @('--inventory-self-test')
  Assert-Match $inventorySelfTest.Output '^INVENTORY_SELF_TEST ' 'Inventory fixture self-test did not produce its receipt.'
  $inventoryChecks = [int](Get-ReceiptValue $inventorySelfTest.Output 'checks')
  $inventoryPassed = [int](Get-ReceiptValue $inventorySelfTest.Output 'passed')
  $surfaceContracts = [int](Get-ReceiptValue $inventorySelfTest.Output 'surface_contracts')
  $expectedSurfaceNames = @(
    'Registry_Run', 'Registry_RunOnce', 'Registry_RunOnceEx', 'Registry_RunServices', 'Policy_Run',
    'Legacy_Windows_Run', 'User_Logon_Script', 'Startup_Folder', 'Startup_Command', 'Scheduled_Task',
    'Windows_Service', 'System_Driver', 'Winlogon_Autostart', 'Winlogon_Notification',
    'Explorer_Startup_Extension', 'Explorer_Shell_Extension', 'Internet_Explorer_Add-on',
    'AppInit_DLLs', 'AppCert_DLLs', 'Active_Setup', 'Boot_Execute', 'LSA_Startup_Package',
    'Image_Hijack', 'Known_DLL', 'Network_Provider', 'Winsock_Provider', 'Print_Monitor',
    'Media_Codec', 'Group_Policy_Script', 'WMI_Event_Consumer', 'Packaged_Startup_Task'
  )
  $surfaceNames = @((Get-ReceiptValue $inventorySelfTest.Output 'surface_names').Split(','))
  $serviceStartModes = Get-ReceiptValue $inventorySelfTest.Output 'service_start_modes'
  if ($inventoryChecks -lt 1 -or $inventoryChecks -ne $inventoryPassed -or $surfaceContracts -ne $expectedSurfaceNames.Count) { throw "Inventory fixture receipt is incomplete: $($inventorySelfTest.Output)" }
  if (($surfaceNames -join ',') -ne ($expectedSurfaceNames -join ',')) { throw "Inventory surface names drifted or lost a provider contract: $($inventorySelfTest.Output)" }
  if ($serviceStartModes -ne '0,1,2,3') { throw "Exact service/driver Start metadata fixtures do not cover 0,1,2,3: $($inventorySelfTest.Output)" }
  "PASS inventory-self-test checks=$inventoryChecks surfaces=$surfaceContracts serviceStartModes=$serviceStartModes"

  $managedDedupeSelfTest = Invoke-AppCommand @('--managed-startup-dedupe-self-test')
  $managedDedupeReceipt = $managedDedupeSelfTest.Output.Trim()
  Assert-Match $managedDedupeReceipt '^MANAGED_DEDUPE_SELF_TEST checks=\d+ passed=\d+ exactOne=true staleAliasRetired=true conflictsFailClosed=true$' 'Managed startup dedupe fixture self-test did not produce a complete receipt.'
  $managedDedupeChecks = [int](Get-ReceiptValue $managedDedupeReceipt 'checks')
  $managedDedupePassed = [int](Get-ReceiptValue $managedDedupeReceipt 'passed')
  if ($managedDedupeChecks -lt 1 -or $managedDedupeChecks -ne $managedDedupePassed) { throw "Managed startup dedupe fixture self-test failed: $managedDedupeReceipt" }
  'PASS managed-startup-dedupe exactOne=true staleAliasRetired=true conflictsFailClosed=true'

  $stateSelfTest = Invoke-AppCommand @('--state-store-self-test')
  Assert-Match $stateSelfTest.Output '^STATE_STORE_SELF_TEST workers=12 atomic=ok recovery=ok' 'State-store concurrency/atomicity self-test failed.'
  'PASS state-store-self-test workers=12 atomic=ok recovery=ok'

  $agentSelfTest = Invoke-AppCommand @('--agent-registration-self-test')
  Assert-Match $agentSelfTest.Output '^AGENT_REGISTRATION_SELF_TEST_OK ' 'Agent registration fixture self-test failed.'
  $requiredAgentFields = @(
    'missingTaskErrorsRecognized',
    'mappedMissingErrorsRecognized',
    'schedulerFailuresPreserved',
    'goodAccepted',
    'omittedDefaultAccepted',
    'delayedRejected',
    'wrongActionRejected',
    'settingsFalseRejected',
    'triggerFalseRejected',
    'batteryStopRejected',
    'badPrincipalRejected',
    'extraTriggerRejected',
    'extraActionRejected',
    'wrongUserRejected',
    'legacyDirectAccepted',
    'legacyWscriptAccepted',
    'legacyCscriptAccepted',
    'unrelatedShortcutRejected',
    'sameNameWrongPathRejected'
  )
  $agentBooleans = @{}
  foreach ($match in [regex]::Matches($agentSelfTest.Output, '(?<name>[A-Za-z][A-Za-z0-9_]*)=(?<value>true|false)(?:\s|$)', [Text.RegularExpressions.RegexOptions]::IgnoreCase)) {
    $agentBooleans[$match.Groups['name'].Value] = $match.Groups['value'].Value.ToLowerInvariant()
  }
  foreach ($field in $requiredAgentFields) {
    if (-not $agentBooleans.ContainsKey($field)) { throw "Agent registration receipt is missing '$field': $($agentSelfTest.Output)" }
  }
  if ($agentBooleans.Count -ne $requiredAgentFields.Count) { throw "Agent registration receipt added an unasserted boolean field: $($agentSelfTest.Output)" }
  $falseAgentFields = @($agentBooleans.GetEnumerator() | Where-Object { $_.Value -ne 'true' } | ForEach-Object { $_.Key } | Sort-Object)
  if ($falseAgentFields.Count -ne 0) {
    throw "Agent registration safety fields failed: $($falseAgentFields -join ', '). Receipt: $($agentSelfTest.Output)"
  }
  "PASS agent-registration-self-test required=$($requiredAgentFields.Count) emitted=$($agentBooleans.Count) all=true"

  $releaseGateSelfTest = Invoke-AppCommand @('--release-gate-self-test')
  Assert-Match $releaseGateSelfTest.Output '^RELEASE_GATE_SELF_TEST_OK ' 'Release deployment/registration gate self-test failed.'
  foreach ($requiredGateField in @('passed=true', 'normalOrder=true', 'unauthorizedRejected=true', 'childBypass=true', 'heldGateRequired=true', 'eventRequired=true', 'oneShot=true', 'tokenLeak=false', 'externalProcessesStarted=0')) {
    if ($releaseGateSelfTest.Output -notmatch [regex]::Escape($requiredGateField)) { throw "Release gate self-test is missing '$requiredGateField': $($releaseGateSelfTest.Output)" }
  }
  'PASS release-gate-self-test continuousOrdering=true authenticatedChild=true oneShot=true tokenLeak=false externalProcessesStarted=0'

  Test-QuietLaunchContract
  if ($QuietLaunchOnly) { return }

  Assert-UiContract

  $smoke = Invoke-AppCommand @('--smoke')
  Assert-Match $smoke.Output '^SMOKE passed=true inventory=(?<count>\d+) invalid=0 duplicate_ids=0' 'Smoke scan failed.'
  $smokeCount = [int]([regex]::Match($smoke.Output, 'inventory=(?<count>\d+)').Groups['count'].Value)
  if ($smokeCount -lt 1) { throw 'Smoke scan returned an empty Windows startup inventory.' }
  "PASS smoke inventory=$smokeCount"

  $listResult = Invoke-AppCommand @('--list')
  $items = @(ConvertFrom-JsonArray -Json $listResult.Output -Context '--list output')
  if ($items.Count -lt 1) { throw '--list returned no startup registrations.' }
  if (@($items | Where-Object { [string]::IsNullOrWhiteSpace([string]$_.id) }).Count -ne 0) { throw 'Every startup row must expose a stable nonempty id.' }
  $duplicateIds = @($items | Group-Object { ([string]$_.id).ToLowerInvariant() } | Where-Object { $_.Count -gt 1 })
  if ($duplicateIds.Count -ne 0) { throw "Startup row ids must be unique: $($duplicateIds[0].Name)" }
  if (@($items | Where-Object { [string]::IsNullOrWhiteSpace([string]$_.appName) }).Count -ne 0) { throw 'Every startup row must have a human-readable application name.' }
  if (@($items | Where-Object { [string]::IsNullOrWhiteSpace([string]$_.source) -or [string]::IsNullOrWhiteSpace([string]$_.location) }).Count -ne 0) { throw 'Every startup row must identify its source and location.' }
  $readFailures = @($items | Where-Object {
    ([string]$_.status -match '(?i)\bread failed\b') -or
    ([string]$_.id).StartsWith('error|', [StringComparison]::OrdinalIgnoreCase)
  })
  if ($readFailures.Count -ne 0) { throw "--list contains an unreadable startup surface: $($readFailures[0] | ConvertTo-Json -Compress)" }
  if (@($items | Where-Object { $_.advice -eq 'Cleanup' -and $_.risk -eq 'Critical' }).Count -ne 0) { throw 'A critical startup route was incorrectly marked as cleanup.' }
  foreach ($source in @('Scheduled Task', 'Windows Service', 'System Driver', 'Boot Execute')) {
    if (@($items | Where-Object { $_.source -eq $source }).Count -lt 1) { throw "Required startup source missing from --list: $source" }
  }
  $physicalDuplicates = @($items | Group-Object { '{0}|{1}|{2}|{3}' -f $_.source, $_.location, $_.name, $_.command } | Where-Object { $_.Count -gt 1 })
  if ($physicalDuplicates.Count -ne 0) { throw "The same physical startup registration was listed more than once: $($physicalDuplicates[0].Name)" }
  $firstRouteIds = @{}
  foreach ($item in $items) {
    $routeKey = (@([string]$item.source, [string]$item.scope, [string]$item.location, [string]$item.name, [string]$item.command) | ForEach-Object { $_.Trim().ToLowerInvariant() }) -join [char]31
    $firstRouteIds[$routeKey] = [string]$item.id
  }
  $secondListResult = Invoke-AppCommand @('--list')
  $secondItems = @(ConvertFrom-JsonArray -Json $secondListResult.Output -Context 'Second --list output')
  if ($secondItems.Count -lt 1) { throw 'Second --list returned no startup registrations.' }
  if (@($secondItems | Where-Object { [string]::IsNullOrWhiteSpace([string]$_.id) }).Count -ne 0) { throw 'Every row in a repeated --list scan must expose a stable nonempty id.' }
  $secondDuplicateIds = @($secondItems | Group-Object { ([string]$_.id).ToLowerInvariant() } | Where-Object { $_.Count -gt 1 })
  if ($secondDuplicateIds.Count -ne 0) { throw "Repeated --list scan produced a duplicate id: $($secondDuplicateIds[0].Name)" }
  $secondReadFailures = @($secondItems | Where-Object { ([string]$_.status -match '(?i)\bread failed\b') -or ([string]$_.id).StartsWith('error|', [StringComparison]::OrdinalIgnoreCase) })
  if ($secondReadFailures.Count -ne 0) { throw "Repeated --list contains an unreadable startup surface: $($secondReadFailures[0] | ConvertTo-Json -Compress)" }
  $stableRouteCount = 0
  foreach ($item in $secondItems) {
    $routeKey = (@([string]$item.source, [string]$item.scope, [string]$item.location, [string]$item.name, [string]$item.command) | ForEach-Object { $_.Trim().ToLowerInvariant() }) -join [char]31
    if (-not $firstRouteIds.ContainsKey($routeKey)) { continue }
    $stableRouteCount++
    if (-not [string]::Equals([string]$item.id, [string]$firstRouteIds[$routeKey], [StringComparison]::OrdinalIgnoreCase)) {
      throw "A stable physical startup route changed id between consecutive read-only scans: route=$routeKey first=$($firstRouteIds[$routeKey]) second=$($item.id)"
    }
  }
  if ($stableRouteCount -lt 1) { throw 'Consecutive --list scans had no stable physical route in common; id stability is unproven.' }
  $registryCommands = @($items | Where-Object { $_.source -like 'Registry Run*' } | ForEach-Object { ([string]$_.command -replace '\s+', ' ').Trim().ToLowerInvariant() })
  $wmiMirrors = @($items | Where-Object { $_.source -eq 'Startup Command' -and $registryCommands -contains (([string]$_.command -replace '\s+', ' ').Trim().ToLowerInvariant()) })
  if ($wmiMirrors.Count -ne 0) { throw "Win32_StartupCommand mirror was listed beside its native registry route: $($wmiMirrors[0].name)" }

  $openSpeedyInventory = @($items | Where-Object {
    $_.source -eq 'Scheduled Task' -and (([string]$_.name + ' ' + [string]$_.appName + ' ' + [string]$_.command) -match '(?i)openspeedy|(?:^|[\\/])speedy\.exe')
  })
  $openSpeedyReceipt = 'OpenSpeedy=absent(optional)'
  if ($openSpeedyInventory.Count -ne 0) {
    $openSpeedyDirect = @($openSpeedyInventory | Where-Object {
      $_.enabled -and [string]$_.command -match '(?i)(?:^|[\\/])(?:Open)?Speedy\.exe(?:"|\s|$)' -and
      [string]$_.command -match '(?i)(^|\s)--minimize-to-tray(\s|$)' -and
      [string]$_.command -notmatch '(?i)--tray-run|wscript\.exe|openspeedy-silent\.vbs'
    })
    if ($openSpeedyDirect.Count -ne 1) { throw "Expected exactly one enabled direct native OpenSpeedy Quiet route when OpenSpeedy is registered, found $($openSpeedyDirect.Count)." }
    $enabledLegacyOpenSpeedy = @($openSpeedyInventory | Where-Object { $_.enabled -and [string]$_.command -match '(?i)--tray-run|wscript\.exe|openspeedy-silent\.vbs' })
    if ($enabledLegacyOpenSpeedy.Count -ne 0) { throw "An enabled legacy/wrapper OpenSpeedy route remains: $($enabledLegacyOpenSpeedy[0].command)" }
    $openSpeedyItem = $openSpeedyDirect[0]
    $openSpeedyTask = [string]$openSpeedyItem.location
    if ([string]::IsNullOrWhiteSpace($openSpeedyTask) -or -not $openSpeedyTask.StartsWith('\MichStartupMaster\', [StringComparison]::OrdinalIgnoreCase)) { throw "OpenSpeedy's managed route has an invalid task identity: $openSpeedyTask" }
    if ($openSpeedyItem.source -ne 'Scheduled Task' -or -not $openSpeedyItem.enabled -or $openSpeedyItem.popup -ne 'Disabled') { throw "OpenSpeedy's inventory row is not an enabled Quiet scheduled task: $($openSpeedyItem | ConvertTo-Json -Compress)" }
    if ([string]$openSpeedyItem.command -notmatch '(?i)(?:^|[\\/])(?:Open)?Speedy\.exe(?:"|\s|$)' -or [string]$openSpeedyItem.command -notmatch '(?i)(^|\s)--minimize-to-tray(\s|$)' -or [string]$openSpeedyItem.command -match '(?i)--tray-run|wscript\.exe|openspeedy-silent\.vbs') {
      throw "OpenSpeedy must use its direct native tray launch, not a fallback/wrapper: $($openSpeedyItem.command)"
    }
    $openSpeedyReceipt = "OpenSpeedyTask=$openSpeedyTask directNative=true enabledLegacy=0"
  }
  "PASS list items=$($items.Count) sources=$(@($items.source | Sort-Object -Unique).Count) uniqueIds=$($items.Count) stableRouteIds=$stableRouteCount physicalDuplicates=0 $openSpeedyReceipt"

  $auditResult = Invoke-AppCommand @('--audit-boot')
  $auditLines = @($auditResult.Output -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
  $bootHeaders = @($auditLines | Where-Object { $_.StartsWith('BOOT_AUDIT ', [StringComparison]::Ordinal) })
  $trayHeaders = @($auditLines | Where-Object { $_.StartsWith('TRAY_AUDIT ', [StringComparison]::Ordinal) })
  if ($bootHeaders.Count -ne 1 -or $trayHeaders.Count -ne 1) { throw "Audit output must contain exactly one BOOT_AUDIT and one TRAY_AUDIT header: $($auditResult.Output)" }
  $bootLine = $bootHeaders[0]
  $trayLine = $trayHeaders[0]
  $boot = Convert-AuditLine $bootLine 'BOOT_AUDIT'
  $tray = Convert-AuditLine $trayLine 'TRAY_AUDIT'
  foreach ($field in @('independent', 'independence_scope', 'sources', 'shown', 'gaps', 'errors', 'surfaces')) {
    if (-not $boot.ContainsKey($field)) { throw "BOOT_AUDIT is missing '$field': $bootLine" }
  }
  if ($boot['independent'] -ne 'true' -or $boot['independence_scope'] -ne 'authoritative-enumerators') { throw "Boot audit is not independently backed by authoritative startup enumerators: $bootLine" }
  $bootSources = [int]$boot['sources']; $bootShown = [int]$boot['shown']; $bootGaps = [int]$boot['gaps']; $bootErrors = [int]$boot['errors']
  if ($bootSources -lt 1 -or $bootShown -lt 1 -or [string]::IsNullOrWhiteSpace([string]$boot['surfaces'])) { throw "Boot audit returned no independently enumerated coverage: $bootLine" }
  if ($bootGaps -ne 0 -or $bootErrors -ne 0) { throw "Boot coverage is incomplete or unreadable: $bootLine" }

  foreach ($field in @('apps', 'running', 'findings', 'uncertain')) {
    if (-not $tray.ContainsKey($field)) { throw "TRAY_AUDIT is missing '$field': $trayLine" }
  }
  $trayApps = [int]$tray['apps']; $trayRunning = [int]$tray['running']; $trayFindings = [int]$tray['findings']; $trayUncertain = [int]$tray['uncertain']
  if ($trayApps -lt 1 -or $trayRunning -ne $trayApps -or $trayFindings -ne 0 -or $trayUncertain -ne 0) { throw "Quiet startup tray coverage is empty or not clean: $trayLine" }
  if ($auditLines.Count -ne 2) { throw "A clean audit receipt must not contain uncounted detail rows: $($auditResult.Output)" }
  "PASS independent-audit sources=$bootSources shown=$bootShown gaps=0 errors=0 trayApps=$trayApps trayRunning=$trayRunning findings=0 uncertain=0"

  Add-Type -AssemblyName System.Drawing
  $icon = [Drawing.Icon]::ExtractAssociatedIcon($App)
  if ($null -eq $icon) { throw 'EXE icon extraction failed.' }
  try { "PASS icon size=$($icon.Width)x$($icon.Height)" } finally { $icon.Dispose() }
}

function Test-LiveMutationSuite {
  # This suite is deliberately opt-in. It creates uniquely named disposable objects and removes
  # only those exact objects; it never kills processes by image name or touches unrelated tasks.
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = New-Object Security.Principal.WindowsPrincipal($identity)
  if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw '-AllowLiveMutation requires an elevated PowerShell session.'
  }

  $suffix = '{0}_{1}' -f $PID, ([Guid]::NewGuid().ToString('N').Substring(0, 8))
  $serviceName = "MSMTestSvc_$suffix"
  $demandServiceName = "MSMTestDemand_$suffix"
  $watcherName = "MSM_WatcherProbe_$suffix"
  $registryRoundTripName = "MSM_RegistryRoundTrip_$suffix"
  $disposableTarget = Join-Path $RunDir ("MSM-live-fixture-$suffix.exe")
  # Keep the live state root short. The product is long-path aware, but Windows PowerShell 5.1's
  # Test-Path/Remove-Item can report a just-moved quarantine file as missing once the deeply nested
  # repository receipt path crosses MAX_PATH, producing a false failure after a successful move.
  $isolatedStateRoot = [IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) ("MichStartupMaster-test-live-$suffix")))
  $watcherStore = Join-Path $isolatedStateRoot 'watcher-known.tsv'
  $taskDisplayName = "MSMTask_$suffix"
  $taskLocation = "\MichStartupMaster\$taskDisplayName"
  $runSubKey = 'Software\Microsoft\Windows\CurrentVersion\Run'
  $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
  $startupFolder = [Environment]::GetFolderPath([Environment+SpecialFolder]::Startup)
  if ([string]::IsNullOrWhiteSpace($startupFolder)) { throw 'Current-user Startup folder could not be resolved before live mutation.' }
  $startupFolderExisted = Test-Path -LiteralPath $startupFolder -PathType Container
  $startupFolderItem = if ($startupFolderExisted) { Get-Item -LiteralPath $startupFolder } else { $null }
  $startupFolderCreationUtc = if ($startupFolderExisted) { $startupFolderItem.CreationTimeUtc } else { [DateTime]::MinValue }
  $startupFolderWriteUtc = if ($startupFolderExisted) { $startupFolderItem.LastWriteTimeUtc } else { [DateTime]::MinValue }
  $startupFolderAttributes = if ($startupFolderExisted) { $startupFolderItem.Attributes } else { [IO.FileAttributes]::Directory }
  $startupFile = Join-Path $startupFolder ("MSM_StartupFolder_$suffix.cmd")
  $startupBytes = [Text.Encoding]::UTF8.GetBytes("@echo off`r`nrem MichStartupMaster exact startup-folder fixture $suffix`r`n")
  $registryRoundTripCommand = '"' + $disposableTarget + '" --smoke --live-fixture ' + $suffix + ' registry'
  $registryRoundTripView = if ([Environment]::Is64BitOperatingSystem) { 'Registry64' } else { 'Registry32' }
  $createdTasks = New-Object 'Collections.Generic.List[string]'
  $createdServices = New-Object 'Collections.Generic.List[string]'
  $cleanupErrors = New-Object 'Collections.Generic.List[string]'
  $suiteError = $null
  $isolatedStateRootCreatedBySuite = $false
  $disposableTargetCreatedBySuite = $false
  $preexistingOwnedTaskLocations = @()
  $knownStoreEnvironmentEntry = Get-Item Env:MSM_KNOWN_STORE -ErrorAction SilentlyContinue
  $knownStoreEnvironmentExisted = $null -ne $knownStoreEnvironmentEntry
  $knownStoreEnvironmentBefore = if ($knownStoreEnvironmentExisted) { [string]$knownStoreEnvironmentEntry.Value } else { '' }
  $stateRootEnvironmentEntry = Get-Item Env:MSM_STATE_ROOT -ErrorAction SilentlyContinue
  $stateRootEnvironmentExisted = $null -ne $stateRootEnvironmentEntry
  $stateRootEnvironmentBefore = if ($stateRootEnvironmentExisted) { [string]$stateRootEnvironmentEntry.Value } else { '' }
  $activeAppStateRootBefore = $script:ActiveAppStateRoot

  $appData = $isolatedStateRoot
  $stateStorePaths = @(
    (Join-Path $appData 'disabled-items.tsv'),
    (Join-Path $appData 'protected-disabled-items.tsv'),
    (Join-Path $appData 'protected-quiet-popup-items.tsv'),
    (Join-Path $appData 'enabled-startup-items.tsv'),
    (Join-Path $appData 'migrated-v2-items.tsv'),
    (Join-Path $appData 'known-startup-items.tsv'),
    $watcherStore
  )
  $snapshotPaths = @($stateStorePaths | ForEach-Object { [IO.Path]::GetFullPath($_); [IO.Path]::GetFullPath($_ + '.bak') } | Sort-Object -Unique)
  $stateStoreSnapshots = @($snapshotPaths | ForEach-Object { New-StateStoreSnapshot -Path $_ })
  $startupFileSnapshot = New-StateStoreSnapshot -Path $startupFile
  $runKeyReadHandle = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($runSubKey, $false)
  if ($null -eq $runKeyReadHandle) { throw "HKCU Registry Run key is absent; refusing a live round-trip that could create it: HKCU\$runSubKey" }
  $runKeyReadHandle.Dispose()
  $registrySnapshots = @(
    (New-HkcuRegistryValueSnapshot -SubKey $runSubKey -Name $watcherName),
    (New-HkcuRegistryValueSnapshot -SubKey $runSubKey -Name $registryRoundTripName)
  )

  try {
    if (Test-Path -LiteralPath $isolatedStateRoot) { throw "Isolated live-test state root already exists: $isolatedStateRoot" }
    if ([string]::Equals($isolatedStateRoot, $script:PureStateRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Live mutation state root must be separate from the pure-probe state root.' }
    [void][IO.Directory]::CreateDirectory($isolatedStateRoot)
    $isolatedStateRootCreatedBySuite = $true
    [Environment]::SetEnvironmentVariable('MSM_STATE_ROOT', $isolatedStateRoot, [EnvironmentVariableTarget]::Process)
    [Environment]::SetEnvironmentVariable('MSM_KNOWN_STORE', $watcherStore, [EnvironmentVariableTarget]::Process)
    $script:ActiveAppStateRoot = $isolatedStateRoot

    $bulkDisableSelfTest = Invoke-AppCommand @('--bulk-disable-self-test')
    $bulkDisableReceipt = $bulkDisableSelfTest.Output.Trim()
    Assert-Match $bulkDisableReceipt '^BULK_DISABLE_SELF_TEST passed=true failedClosed=true registryRestored=true approvalRestored=true successfulBulkDisable=true successfulBulkRestore=true sharedPhysicalCollapsed=true enableFailedClosed=true enableApprovalRestored=true storesRestored=true$' 'Transactional bulk enable/disable self-test did not prove successful multi-route state changes, shared HKCU physical-route collapse, exact restoration, and fail-closed rollback including StartupApproved metadata.'
    'PASS bulk-state transaction=true disableFailedClosed=true enableFailedClosed=true registryRestored=true storesRestored=true'

    foreach ($candidateService in @($serviceName, $demandServiceName)) {
      if ((Test-Path -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$candidateService") -or $null -ne (Get-Service -Name $candidateService -ErrorAction SilentlyContinue)) {
        throw "Disposable service identity already exists: $candidateService"
      }
    }
    $existingRunProperties = Get-ItemProperty -Path $runKey -ErrorAction Stop
    foreach ($candidateValue in @($watcherName, $registryRoundTripName)) {
      if ($null -ne $existingRunProperties.PSObject.Properties[$candidateValue]) { throw "Disposable Registry Run identity already exists: $candidateValue" }
    }
    if ($startupFileSnapshot.Existed) { throw "Disposable Startup-folder identity already exists: $startupFile" }
    if (-not $startupFolderExisted) { [void][IO.Directory]::CreateDirectory($startupFolder) }
    $preexistingOwnedTasks = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($taskDisplayName, [StringComparison]::OrdinalIgnoreCase) })
    $preexistingOwnedTaskLocations = @($preexistingOwnedTasks | ForEach-Object { [string]$_.Location })
    if ($preexistingOwnedTasks.Count -ne 0) { throw "Disposable task identity prefix already exists: $($preexistingOwnedTasks[0].Location)" }
    if (Test-Path -LiteralPath $disposableTarget) { throw "Disposable target already exists: $disposableTarget" }
    $disposableTargetCreatedBySuite = $true
    Copy-Item -LiteralPath $App -Destination $disposableTarget

    $binaryPath = '"' + $disposableTarget + '" --smoke --live-fixture ' + $suffix
    [void]$createdServices.Add($serviceName)
    & sc.exe create $serviceName 'binPath=' $binaryPath 'start=' 'auto' 'DisplayName=' $serviceName | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not create disposable service $serviceName" }
    $createdStart = (Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$serviceName").Start
    if ($createdStart -ne 2) { throw "Disposable service was not created with automatic Start=2: Start=$createdStart" }
    $serviceListResult = Invoke-AppCommand @('--list')
    $serviceRows = @(ConvertFrom-JsonArray -Json $serviceListResult.Output -Context 'Service round-trip --list output')
    $serviceMatches = @($serviceRows | Where-Object { $_.name -eq $serviceName -and $_.source -eq 'Windows Service' -and $_.enabled })
    if ($serviceMatches.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$serviceMatches[0].id)) { throw "Disposable service was not uniquely visible: $serviceName" }
    [void](Invoke-AppCommand @('--set-enabled', [string]$serviceMatches[0].id, 'false'))
    $disabledStart = (Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$serviceName").Start
    if ($disabledStart -ne 4) { throw "Disposable service was not disabled: Start=$disabledStart" }
    $disabledListResult = Invoke-AppCommand @('--list')
    $disabledRows = @(ConvertFrom-JsonArray -Json $disabledListResult.Output -Context 'Disabled service --list output' | Where-Object { $_.name -eq $serviceName -and $_.source -eq 'Windows Service' -and -not $_.enabled })
    if ($disabledRows.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$disabledRows[0].id)) { throw "Disposable service did not expose one reversible disabled row: $serviceName" }
    [void](Invoke-AppCommand @('--set-enabled', [string]$disabledRows[0].id, 'true'))
    $restoredStart = (Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$serviceName").Start
    if ($restoredStart -ne 2) { throw "Disposable service was not restored: Start=$restoredStart" }
    "PASS live-service name=$serviceName disabledStart=4 restoredStart=2"

    [void]$createdServices.Add($demandServiceName)
    & sc.exe create $demandServiceName 'binPath=' $binaryPath 'start=' 'demand' 'DisplayName=' $demandServiceName | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not create disposable demand service $demandServiceName" }
    $demandServiceKeyPath = "HKLM:\SYSTEM\CurrentControlSet\Services\$demandServiceName"
    if ((Get-ItemProperty -LiteralPath $demandServiceKeyPath).Start -ne 3) { throw "Disposable demand service was not created with Start=3: $demandServiceName" }
    $demandServiceListResult = Invoke-AppCommand @('--list')
    $demandServiceRows = @(ConvertFrom-JsonArray -Json $demandServiceListResult.Output -Context 'Demand service --list output' | Where-Object { $_.name -eq $demandServiceName -and $_.source -eq 'Windows Service' })
    if ($demandServiceRows.Count -ne 0) { throw "Demand-start service was incorrectly presented as a boot startup route: $demandServiceName" }
    $demandStartAfterScan = (Get-ItemProperty -LiteralPath $demandServiceKeyPath).Start
    if ($demandStartAfterScan -ne 3) { throw "Read-only inventory altered the demand service's exact Start=3 value: Start=$demandStartAfterScan" }
    "PASS live-service-demand name=$demandServiceName startupRoute=false startUnchanged=3"

    $registryKeyHandle = $null
    try {
      $registryKeyHandle = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($runSubKey)
      $registryKeyHandle.SetValue($registryRoundTripName, $registryRoundTripCommand, [Microsoft.Win32.RegistryValueKind]::String)
    }
    finally { if ($null -ne $registryKeyHandle) { $registryKeyHandle.Dispose() } }
    $registryOriginal = New-HkcuRegistryValueSnapshot -SubKey $runSubKey -Name $registryRoundTripName
    if (-not $registryOriginal.Existed -or $registryOriginal.Kind -ne [Microsoft.Win32.RegistryValueKind]::String -or -not [string]::Equals([string]$registryOriginal.Value, $registryRoundTripCommand, [StringComparison]::Ordinal)) { throw "Disposable Registry Run string value was not created exactly: $registryRoundTripName" }
    $registryListResult = Invoke-AppCommand @('--list')
    $registryRows = @(ConvertFrom-JsonArray -Json $registryListResult.Output -Context 'Registry round-trip --list output' | Where-Object { $_.name -eq $registryRoundTripName -and $_.source -eq 'Registry Run' -and $_.registryView -eq $registryRoundTripView -and $_.enabled })
    if ($registryRows.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$registryRows[0].id)) { throw "Disposable Registry Run value was not uniquely visible: $registryRoundTripName" }
    [void](Invoke-AppCommand @('--set-enabled', [string]$registryRows[0].id, 'false'))
    $registryAfterDisable = New-HkcuRegistryValueSnapshot -SubKey $runSubKey -Name $registryRoundTripName
    if ($registryAfterDisable.Existed) { throw "Registry Run value still existed after disable: $registryRoundTripName" }
    $registryDisabledListResult = Invoke-AppCommand @('--list')
    $registryDisabledRows = @(ConvertFrom-JsonArray -Json $registryDisabledListResult.Output -Context 'Disabled registry --list output' | Where-Object { $_.name -eq $registryRoundTripName -and $_.source -eq 'Registry Run' -and -not $_.enabled })
    if ($registryDisabledRows.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$registryDisabledRows[0].id)) { throw "Disabled Registry Run value was not uniquely reversible: $registryRoundTripName" }
    [void](Invoke-AppCommand @('--set-enabled', [string]$registryDisabledRows[0].id, 'true'))
    Assert-HkcuRegistryValueSnapshotRestored -Snapshot $registryOriginal
    "PASS live-registry-run name=$registryRoundTripName kind=String characters=$($registryRoundTripCommand.Length) exactRestore=true"

    [IO.File]::WriteAllBytes($startupFile, $startupBytes)
    $startupOriginal = New-StateStoreSnapshot -Path $startupFile
    $startupListResult = Invoke-AppCommand @('--list')
    $startupRows = @(ConvertFrom-JsonArray -Json $startupListResult.Output -Context 'Startup-folder round-trip --list output' | Where-Object { $_.source -eq 'Startup Folder' -and [string]::Equals([string]$_.command, $startupFile, [StringComparison]::OrdinalIgnoreCase) -and $_.enabled })
    if ($startupRows.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$startupRows[0].id)) { throw "Disposable Startup-folder file was not uniquely visible: $startupFile" }
    [void](Invoke-AppCommand @('--set-enabled', [string]$startupRows[0].id, 'false'))
    if (Test-Path -LiteralPath $startupFile) { throw "Startup-folder file remained at its original path after disable: $startupFile" }
    $startupDisabledListResult = Invoke-AppCommand @('--list')
    $startupDisabledRows = @(ConvertFrom-JsonArray -Json $startupDisabledListResult.Output -Context 'Disabled startup-folder --list output' | Where-Object { $_.source -eq 'Startup Folder' -and [string]::Equals([string]$_.status, $startupFile, [StringComparison]::OrdinalIgnoreCase) -and -not $_.enabled })
    if ($startupDisabledRows.Count -ne 1 -or [string]::IsNullOrWhiteSpace([string]$startupDisabledRows[0].id)) { throw "Disabled Startup-folder file was not uniquely reversible: $startupFile" }
    $startupQuarantinePath = [string]$startupDisabledRows[0].command
    $startupQuarantineDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $startupQuarantinePath -PathType Leaf) -and [DateTime]::UtcNow -lt $startupQuarantineDeadline) { Start-Sleep -Milliseconds 50 }
    if (-not (Test-Path -LiteralPath $startupQuarantinePath -PathType Leaf)) { throw "Startup-folder quarantine file is missing: $startupQuarantinePath" }
    [void](Invoke-AppCommand @('--set-enabled', [string]$startupDisabledRows[0].id, 'true'))
    $startupRestoreDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $startupFile -PathType Leaf) -and [DateTime]::UtcNow -lt $startupRestoreDeadline) { Start-Sleep -Milliseconds 50 }
    Assert-StateStoreSnapshotRestored -Snapshot $startupOriginal
    if (Test-Path -LiteralPath $startupQuarantinePath) { throw "Startup-folder quarantine copy remained after restore: $startupQuarantinePath" }
    $startupRestoredListResult = Invoke-AppCommand @('--list')
    $startupRestoredRows = @(ConvertFrom-JsonArray -Json $startupRestoredListResult.Output -Context 'Restored startup-folder --list output' | Where-Object { $_.source -eq 'Startup Folder' -and [string]::Equals([string]$_.command, $startupFile, [StringComparison]::OrdinalIgnoreCase) -and $_.enabled })
    if ($startupRestoredRows.Count -ne 1) { throw "Startup-folder restore did not return exactly one original route: $startupFile" }
    "PASS live-startup-folder path=$startupFile bytes=$($startupBytes.Length) exactRestore=true"

    $seed = Invoke-AppCommand @('--detect-new')
    Assert-Match $seed.Output 'count=0' 'Watcher baseline was not isolated.'
    New-ItemProperty -Path $runKey -Name $watcherName -Value '"C:\Windows\System32\cmd.exe" /c exit' -PropertyType String | Out-Null
    $detected = Invoke-AppCommand @('--detect-new')
    $match = [regex]::Match($detected.Output, 'count=(?<count>\d+)')
    if (-not $match.Success -or [int]$match.Groups['count'].Value -lt 1) { throw "Watcher did not report the disposable registry value: $($detected.Output)" }
    "PASS live-watcher value=$watcherName"

    $targetArguments = "--smoke --live-fixture $suffix managed"
    [void]$createdTasks.Add($taskLocation)

    # Prove the complete add decision is serialized, not only the final write. Eight separate
    # processes race with different friendly names but one exact target/argument identity. Every
    # caller must converge on one task; this is the real-world OpenWhispr/OpenWhisprLauncher case.
    $racePrefix = $taskDisplayName + 'Race'
    $raceArguments = "--smoke --live-fixture $suffix concurrent"
    $raceArgumentSets = @(for ($raceIndex = 0; $raceIndex -lt 8; $raceIndex++) { ,@('--add-startup', ($racePrefix + $raceIndex), $disposableTarget, $raceArguments, 'normal') })
    $raceResults = @(Invoke-ConcurrentAppCommands -ArgumentSets $raceArgumentSets)
    $raceTasks = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($racePrefix, [StringComparison]::OrdinalIgnoreCase) })
    foreach ($raceTask in $raceTasks) { if (@($createdTasks | Where-Object { [string]::Equals($_, [string]$raceTask.Location, [StringComparison]::OrdinalIgnoreCase) }).Count -eq 0) { [void]$createdTasks.Add([string]$raceTask.Location) } }
    if ($raceTasks.Count -ne 1) { throw "Concurrent equivalent adds created $($raceTasks.Count) routes instead of exactly one: $($raceTasks | ConvertTo-Json -Compress)" }
    $raceWinner = [string]$raceTasks[0].Location
    foreach ($raceResult in $raceResults) {
      $raceMatch = [regex]::Match($raceResult.Output, 'task=(?<task>\\MichStartupMaster\\[^\s]+)')
      if (-not $raceMatch.Success -or -not [string]::Equals($raceMatch.Groups['task'].Value, $raceWinner, [StringComparison]::OrdinalIgnoreCase)) { throw "Concurrent add did not converge on $raceWinner`: $($raceResult.Output)" }
    }
    [void](Invoke-AppCommand @('--remove-task', $raceWinner))
    [void]$createdTasks.Remove($raceWinner)
    if (@(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($racePrefix, [StringComparison]::OrdinalIgnoreCase) }).Count -ne 0) { throw "Concurrent-add fixture remained after cleanup: $raceWinner" }
    "PASS concurrent-add processes=8 routes=1 converged=true cleanup=true"

    $firstAdd = Invoke-AppCommand @('--add-startup', $taskDisplayName, $disposableTarget, $targetArguments, 'normal')
    $ownedTasksAfterFirstAdd = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($taskDisplayName, [StringComparison]::OrdinalIgnoreCase) })
    foreach ($ownedTask in $ownedTasksAfterFirstAdd) {
      if (@($createdTasks | Where-Object { [string]::Equals($_, [string]$ownedTask.Location, [StringComparison]::OrdinalIgnoreCase) }).Count -eq 0) { [void]$createdTasks.Add([string]$ownedTask.Location) }
    }
    if ($ownedTasksAfterFirstAdd.Count -ne 1 -or -not [string]::Equals([string]$ownedTasksAfterFirstAdd[0].Location, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "First managed add did not create exactly one disposable task identity: $($ownedTasksAfterFirstAdd | ConvertTo-Json -Compress)" }
    $firstTaskMatch = [regex]::Match($firstAdd.Output, 'task=(?<task>\\MichStartupMaster\\[^\s]+)')
    if (-not $firstTaskMatch.Success -or -not [string]::Equals($firstTaskMatch.Groups['task'].Value, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "First managed add did not use the exact disposable task: $($firstAdd.Output)" }
    $secondAdd = Invoke-AppCommand @('--add-startup', $taskDisplayName, $disposableTarget, $targetArguments, 'normal')
    $secondTaskMatch = [regex]::Match($secondAdd.Output, 'task=(?<task>\\MichStartupMaster\\[^\s]+)')
    if (-not $secondTaskMatch.Success -or -not [string]::Equals($secondTaskMatch.Groups['task'].Value, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Identical repeated add created or selected a different task: $($secondAdd.Output)" }
    $managedAfterRepeatResult = Invoke-AppCommand @('--list-managed')
    $managedAfterRepeat = @(ConvertFrom-JsonArray -Json $managedAfterRepeatResult.Output -Context 'Repeated add --list-managed output' | Where-Object { $_.kind -eq 'managed-task' -and [string]::Equals([string]$_.target, $disposableTarget, [StringComparison]::OrdinalIgnoreCase) })
    if ($managedAfterRepeat.Count -ne 1 -or -not [string]::Equals([string]$managedAfterRepeat[0].task, $taskLocation, [StringComparison]::OrdinalIgnoreCase) -or $managedAfterRepeat[0].mode -ne 'normal') { throw "Identical repeated add was not idempotent: $($managedAfterRepeat | ConvertTo-Json -Compress)" }
    $ownedTasksAfterRepeat = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($taskDisplayName, [StringComparison]::OrdinalIgnoreCase) })
    if ($ownedTasksAfterRepeat.Count -ne 1 -or -not [string]::Equals([string]$ownedTasksAfterRepeat[0].Location, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Identical repeated add created a duplicate scheduled task: $($ownedTasksAfterRepeat | ConvertTo-Json -Compress)" }

    $windowTask = Get-ExactScheduledTask -TaskLocation $taskLocation
    if ($null -eq $windowTask -or $windowTask.State -eq 'Disabled') { throw "Window-mode managed task is missing or disabled: $taskLocation" }
    [xml]$windowXml = $windowTask.Xml
    $windowNamespace = New-Object Xml.XmlNamespaceManager($windowXml.NameTable)
    $windowNamespace.AddNamespace('t', 'http://schemas.microsoft.com/windows/2004/02/mit/task')
    if (@($windowXml.SelectNodes('//t:Triggers/t:LogonTrigger', $windowNamespace)).Count -ne 1 -or $null -ne $windowXml.SelectSingleNode('//t:LogonTrigger/t:Delay', $windowNamespace) -or @($windowXml.SelectNodes('//t:Actions/t:Exec', $windowNamespace)).Count -ne 1) { throw "Managed task is not one immediate logon trigger plus one exact action: $taskLocation" }
    $windowCommand = [string]$windowXml.SelectSingleNode('//t:Actions/t:Exec/t:Command', $windowNamespace).InnerText
    $windowArguments = [string]$windowXml.SelectSingleNode('//t:Actions/t:Exec/t:Arguments', $windowNamespace).InnerText
    if (-not [string]::Equals([IO.Path]::GetFullPath($windowCommand), [IO.Path]::GetFullPath($disposableTarget), [StringComparison]::OrdinalIgnoreCase) -or $windowArguments -ne $targetArguments) { throw "Window-mode task did not preserve the exact target and arguments: $taskLocation" }

    # Create the exact failure shape that previously left a second broken-icon route: a disabled
    # Launcher-named managed sibling. It is a disposable clone of the verified primary task, then
    # the real Scheduler reconciliation must retire it and its intent without touching the primary.
    $duplicateTaskName = $taskDisplayName + 'Launcher'
    $duplicateTaskLocation = "\MichStartupMaster\$duplicateTaskName"
    [void]$createdTasks.Add($duplicateTaskLocation)
    $duplicateService = $null
    $duplicateFolder = $null
    $duplicateTask = $null
    try {
      $duplicateService = New-Object -ComObject 'Schedule.Service'
      $duplicateService.Connect()
      $duplicateFolder = $duplicateService.GetFolder('\MichStartupMaster')
      $duplicateXml = ([string]$windowTask.Xml).Replace($taskLocation, $duplicateTaskLocation)
      if ([string]::Equals($duplicateXml, [string]$windowTask.Xml, [StringComparison]::Ordinal)) { throw "Disposable duplicate XML did not contain its exact primary task location: $taskLocation" }
      $duplicateTask = $duplicateFolder.RegisterTask($duplicateTaskName, $duplicateXml, 6, $null, $null, 3, $null)
      $duplicateTask.Enabled = $false
    }
    finally {
      foreach ($comObject in @($duplicateTask, $duplicateFolder, $duplicateService)) {
        if ($null -ne $comObject -and [Runtime.InteropServices.Marshal]::IsComObject($comObject)) {
          try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($comObject) } catch { }
        }
      }
    }
    $duplicateBeforeRepair = Get-ExactScheduledTask -TaskLocation $duplicateTaskLocation
    if ($null -eq $duplicateBeforeRepair -or $duplicateBeforeRepair.State -ne 'Disabled') { throw "Disposable disabled Launcher sibling was not created exactly: $duplicateTaskLocation" }
    $duplicateRepair = Invoke-AppCommand @('--reconcile-managed-startups', $taskDisplayName)
    Assert-Match $duplicateRepair.Output '^MANAGED_DEDUPE scanned=2 retired=1 conflicts=0 ' 'Managed duplicate repair did not report the exact disposable pair.'
    if ($null -ne (Get-ExactScheduledTask -TaskLocation $duplicateTaskLocation)) { throw "Managed duplicate reconciliation did not remove the disabled Launcher sibling: $duplicateTaskLocation" }
    $ownedTasksAfterRepair = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($taskDisplayName, [StringComparison]::OrdinalIgnoreCase) })
    if ($ownedTasksAfterRepair.Count -ne 1 -or -not [string]::Equals([string]$ownedTasksAfterRepair[0].Location, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Managed duplicate reconciliation did not retain only the canonical task: $($ownedTasksAfterRepair | ConvertTo-Json -Compress)" }
    $managedAfterDuplicateRepairResult = Invoke-AppCommand @('--list-managed')
    $managedAfterDuplicateRepair = @(ConvertFrom-JsonArray -Json $managedAfterDuplicateRepairResult.Output -Context 'Managed duplicate repair --list-managed output' | Where-Object { $_.kind -eq 'managed-task' -and [string]::Equals([string]$_.target, $disposableTarget, [StringComparison]::OrdinalIgnoreCase) })
    if ($managedAfterDuplicateRepair.Count -ne 1 -or -not [string]::Equals([string]$managedAfterDuplicateRepair[0].task, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Managed duplicate reconciliation left a second persisted route: $($managedAfterDuplicateRepair | ConvertTo-Json -Compress)" }
    'PASS live-managed-dedupe primary=retained disabledLauncher=retired persistedRoutes=1'

    $quietChange = Invoke-AppCommand @('--add-startup', $taskDisplayName, $disposableTarget, $targetArguments, 'tray')
    $quietTaskMatch = [regex]::Match($quietChange.Output, 'task=(?<task>\\MichStartupMaster\\[^\s]+)')
    if (-not $quietTaskMatch.Success -or -not [string]::Equals($quietTaskMatch.Groups['task'].Value, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Window-to-Quiet created a different task: $($quietChange.Output)" }
    $managedQuietResult = Invoke-AppCommand @('--list-managed')
    $managedQuiet = @(ConvertFrom-JsonArray -Json $managedQuietResult.Output -Context 'Quiet mode --list-managed output' | Where-Object { $_.kind -eq 'managed-task' -and [string]::Equals([string]$_.target, $disposableTarget, [StringComparison]::OrdinalIgnoreCase) })
    if ($managedQuiet.Count -ne 1 -or $managedQuiet[0].mode -ne 'tray' -or -not [string]::Equals([string]$managedQuiet[0].task, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Window-to-Quiet duplicated or lost the managed route: $($managedQuiet | ConvertTo-Json -Compress)" }
    $ownedTasksAfterQuiet = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($taskDisplayName, [StringComparison]::OrdinalIgnoreCase) })
    if ($ownedTasksAfterQuiet.Count -ne 1 -or -not [string]::Equals([string]$ownedTasksAfterQuiet[0].Location, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Window-to-Quiet created a duplicate scheduled task: $($ownedTasksAfterQuiet | ConvertTo-Json -Compress)" }
    $quietTask = Get-ExactScheduledTask -TaskLocation $taskLocation
    [xml]$quietXml = $quietTask.Xml
    $quietNamespace = New-Object Xml.XmlNamespaceManager($quietXml.NameTable)
    $quietNamespace.AddNamespace('t', 'http://schemas.microsoft.com/windows/2004/02/mit/task')
    $quietCommand = [string]$quietXml.SelectSingleNode('//t:Actions/t:Exec/t:Command', $quietNamespace).InnerText
    $quietArguments = [string]$quietXml.SelectSingleNode('//t:Actions/t:Exec/t:Arguments', $quietNamespace).InnerText
    if (-not [string]::Equals([IO.Path]::GetFullPath($quietCommand), [IO.Path]::GetFullPath($App), [StringComparison]::OrdinalIgnoreCase) -or $quietArguments -notmatch '^--tray-run\s+[A-Za-z0-9+/=]+$') { throw "Quiet-mode task did not use the exact one-shot launcher: $taskLocation" }

    # Repeat the stale-sibling repair against the actual Quiet route and deliberately seed the
    # old quiet-intent record. This proves a deleted tray wrapper cannot come back on the next
    # boot from a forgotten state-file row (the source of duplicated/broken tray icons).
    $quietDuplicateService = $null
    $quietDuplicateFolder = $null
    $quietDuplicateTask = $null
    try {
      $quietDuplicateService = New-Object -ComObject 'Schedule.Service'
      $quietDuplicateService.Connect()
      $quietDuplicateFolder = $quietDuplicateService.GetFolder('\MichStartupMaster')
      $quietDuplicateXml = ([string]$quietTask.Xml).Replace($taskLocation, $duplicateTaskLocation)
      if ([string]::Equals($quietDuplicateXml, [string]$quietTask.Xml, [StringComparison]::Ordinal)) { throw "Disposable Quiet duplicate XML did not contain its exact primary task location: $taskLocation" }
      $quietDuplicateTask = $quietDuplicateFolder.RegisterTask($duplicateTaskName, $quietDuplicateXml, 6, $null, $null, 3, $null)
      $quietDuplicateTask.Enabled = $false
    }
    finally {
      foreach ($comObject in @($quietDuplicateTask, $quietDuplicateFolder, $quietDuplicateService)) {
        if ($null -ne $comObject -and [Runtime.InteropServices.Marshal]::IsComObject($comObject)) {
          try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($comObject) } catch { }
        }
      }
    }
    $quietStorePath = Join-Path $isolatedStateRoot 'protected-quiet-popup-items.tsv'
    if (-not (Test-Path -LiteralPath $quietStorePath -PathType Leaf)) { throw "Quiet intent store was not created for the primary route: $quietStorePath" }
    $duplicateQuietRecord = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($duplicateTaskLocation)) + "`t" + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($disposableTarget)) + "`t" + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($targetArguments))
    [IO.File]::AppendAllText($quietStorePath, $duplicateQuietRecord + [Environment]::NewLine, (New-Object Text.UTF8Encoding($false)))
    $quietDuplicateBeforeRepair = Get-ExactScheduledTask -TaskLocation $duplicateTaskLocation
    if ($null -eq $quietDuplicateBeforeRepair -or $quietDuplicateBeforeRepair.State -ne 'Disabled') { throw "Disposable disabled Quiet Launcher sibling was not created exactly: $duplicateTaskLocation" }
    $quietDuplicateRepair = Invoke-AppCommand @('--reconcile-managed-startups', $taskDisplayName)
    Assert-Match $quietDuplicateRepair.Output '^MANAGED_DEDUPE scanned=2 retired=1 conflicts=0 ' 'Quiet managed duplicate repair did not report the exact disposable pair.'
    if ($null -ne (Get-ExactScheduledTask -TaskLocation $duplicateTaskLocation)) { throw "Quiet managed duplicate reconciliation did not remove the disabled Launcher sibling: $duplicateTaskLocation" }
    $quietStoredTasks = @(
      foreach ($line in [IO.File]::ReadAllLines($quietStorePath)) {
        $parts = $line.Split("`t")
        if ($parts.Length -lt 3) { throw "Quiet intent store contained a malformed row after duplicate repair: $line" }
        try { [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($parts[0])) }
        catch { throw "Quiet intent store contained a non-base64 task location after duplicate repair: $line" }
      }
    )
    if (@($quietStoredTasks | Where-Object { [string]::Equals($_, $taskLocation, [StringComparison]::OrdinalIgnoreCase) }).Count -ne 1 -or @($quietStoredTasks | Where-Object { [string]::Equals($_, $duplicateTaskLocation, [StringComparison]::OrdinalIgnoreCase) }).Count -ne 0) { throw "Quiet managed duplicate reconciliation did not compact the persisted route intent: $($quietStoredTasks -join ',')" }
    $managedAfterQuietDuplicateRepairResult = Invoke-AppCommand @('--list-managed')
    $managedAfterQuietDuplicateRepair = @(ConvertFrom-JsonArray -Json $managedAfterQuietDuplicateRepairResult.Output -Context 'Quiet managed duplicate repair --list-managed output' | Where-Object { $_.kind -eq 'managed-task' -and [string]::Equals([string]$_.target, $disposableTarget, [StringComparison]::OrdinalIgnoreCase) })
    if ($managedAfterQuietDuplicateRepair.Count -ne 1 -or $managedAfterQuietDuplicateRepair[0].mode -ne 'tray' -or -not [string]::Equals([string]$managedAfterQuietDuplicateRepair[0].task, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Quiet managed duplicate reconciliation left a second persisted route: $($managedAfterQuietDuplicateRepair | ConvertTo-Json -Compress)" }
    'PASS live-managed-quiet-dedupe primary=retained disabledLauncher=retired quietIntent=compacted trayRoutes=1'

    $windowChange = Invoke-AppCommand @('--add-startup', $taskDisplayName, $disposableTarget, $targetArguments, 'normal')
    $windowTaskMatch = [regex]::Match($windowChange.Output, 'task=(?<task>\\MichStartupMaster\\[^\s]+)')
    if (-not $windowTaskMatch.Success -or -not [string]::Equals($windowTaskMatch.Groups['task'].Value, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Quiet-to-Window created a different task: $($windowChange.Output)" }
    $managedWindowAgainResult = Invoke-AppCommand @('--list-managed')
    $managedWindowAgain = @(ConvertFrom-JsonArray -Json $managedWindowAgainResult.Output -Context 'Window mode --list-managed output' | Where-Object { $_.kind -eq 'managed-task' -and [string]::Equals([string]$_.target, $disposableTarget, [StringComparison]::OrdinalIgnoreCase) })
    if ($managedWindowAgain.Count -ne 1 -or $managedWindowAgain[0].mode -ne 'normal' -or -not [string]::Equals([string]$managedWindowAgain[0].task, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Quiet-to-Window duplicated or lost the managed route: $($managedWindowAgain | ConvertTo-Json -Compress)" }
    $ownedTasksAfterWindow = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($taskDisplayName, [StringComparison]::OrdinalIgnoreCase) })
    if ($ownedTasksAfterWindow.Count -ne 1 -or -not [string]::Equals([string]$ownedTasksAfterWindow[0].Location, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Quiet-to-Window created a duplicate scheduled task: $($ownedTasksAfterWindow | ConvertTo-Json -Compress)" }

    [void](Invoke-AppCommand @('--set-enabled', $taskLocation, 'false'))
    $disabledTask = Get-ExactScheduledTask -TaskLocation $taskLocation
    if ($null -eq $disabledTask -or $disabledTask.State -ne 'Disabled') { throw "Managed task did not disable: $taskLocation" }
    $disabledXmlBeforeModeAttempt = [string]$disabledTask.Xml
    $disabledEditAttempt = Invoke-AppCommand @('--add-startup', $taskDisplayName, $disposableTarget, ($targetArguments + ' disabled-edit-attempt'), 'tray') -AllowFailure
    if ($disabledEditAttempt.ExitCode -eq 0) { throw "Edit/upsert on a disabled task did not fail closed: $($disabledEditAttempt.Output)" }
    $disabledTaskAfterEditAttempt = Get-ExactScheduledTask -TaskLocation $taskLocation
    if ($null -eq $disabledTaskAfterEditAttempt -or $disabledTaskAfterEditAttempt.State -ne 'Disabled' -or -not [string]::Equals([string]$disabledTaskAfterEditAttempt.Xml, $disabledXmlBeforeModeAttempt, [StringComparison]::Ordinal)) { throw "Rejected disabled-task edit altered or enabled the task: $taskLocation" }
    $disabledModeAttempt = Invoke-AppCommand @('--toggle-popup', $taskLocation) -AllowFailure
    if ($disabledModeAttempt.ExitCode -eq 0) { throw "Mode change on a disabled task did not fail closed: $($disabledModeAttempt.Output)" }
    $disabledTaskAfterModeAttempt = Get-ExactScheduledTask -TaskLocation $taskLocation
    if ($null -eq $disabledTaskAfterModeAttempt -or $disabledTaskAfterModeAttempt.State -ne 'Disabled' -or -not [string]::Equals([string]$disabledTaskAfterModeAttempt.Xml, $disabledXmlBeforeModeAttempt, [StringComparison]::Ordinal)) { throw "Rejected disabled-task mode change altered or enabled the task: $taskLocation" }
    $disabledTaskListResult = Invoke-AppCommand @('--list')
    $disabledTaskRows = @(ConvertFrom-JsonArray -Json $disabledTaskListResult.Output -Context 'Disabled managed task --list output' | Where-Object { [string]::Equals([string]$_.location, $taskLocation, [StringComparison]::OrdinalIgnoreCase) })
    if ($disabledTaskRows.Count -ne 1 -or $disabledTaskRows[0].enabled) { throw "Rejected disabled-task mode change created a duplicate or enabled route: $taskLocation" }
    $ownedTasksAfterRejectedEdits = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($taskDisplayName, [StringComparison]::OrdinalIgnoreCase) })
    foreach ($ownedTask in $ownedTasksAfterRejectedEdits) {
      if (@($createdTasks | Where-Object { [string]::Equals($_, [string]$ownedTask.Location, [StringComparison]::OrdinalIgnoreCase) }).Count -eq 0) { [void]$createdTasks.Add([string]$ownedTask.Location) }
    }
    if ($ownedTasksAfterRejectedEdits.Count -ne 1 -or -not [string]::Equals([string]$ownedTasksAfterRejectedEdits[0].Location, $taskLocation, [StringComparison]::OrdinalIgnoreCase)) { throw "Rejected disabled-task edit created a duplicate scheduled task: $($ownedTasksAfterRejectedEdits | ConvertTo-Json -Compress)" }
    [void](Invoke-AppCommand @('--set-enabled', $taskLocation, 'true'))
    $enabledTask = Get-ExactScheduledTask -TaskLocation $taskLocation
    if ($null -eq $enabledTask -or $enabledTask.State -eq 'Disabled') { throw "Managed task did not re-enable after the fail-closed edit check: $taskLocation" }
    "PASS live-managed-task task=$taskLocation repeatedAdd=same WindowToQuiet=same QuietToWindow=same disabledAddEdit=rejected disabledModeChange=rejected noDuplicate=true"
  }
  catch {
    $suiteError = $_
  }
  finally {
    foreach ($registrySnapshot in $registrySnapshots) {
      try {
        Restore-HkcuRegistryValueSnapshot -Snapshot $registrySnapshot
        Assert-HkcuRegistryValueSnapshotRestored -Snapshot $registrySnapshot
      }
      catch { [void]$cleanupErrors.Add("registry=$($registrySnapshot.Name) $($_.Exception.Message)") }
    }

    try {
      $ownedTasksAtCleanup = @(Get-ScheduledTaskFolderEntries -TaskPath '\MichStartupMaster\' | Where-Object { ([string]$_.Name).StartsWith($taskDisplayName, [StringComparison]::OrdinalIgnoreCase) })
      foreach ($ownedTask in $ownedTasksAtCleanup) {
        if (@($preexistingOwnedTaskLocations | Where-Object { [string]::Equals($_, [string]$ownedTask.Location, [StringComparison]::OrdinalIgnoreCase) }).Count -ne 0) { continue }
        if (@($createdTasks | Where-Object { [string]::Equals($_, [string]$ownedTask.Location, [StringComparison]::OrdinalIgnoreCase) }).Count -eq 0) { [void]$createdTasks.Add([string]$ownedTask.Location) }
      }
    }
    catch { [void]$cleanupErrors.Add("task-discovery=$($_.Exception.Message)") }

    foreach ($taskLocation in @($createdTasks)) {
      $shortName = $taskLocation.Substring($taskLocation.LastIndexOf('\') + 1)
      try { [void](Invoke-AppCommand @('--remove-task', $shortName) -AllowFailure) }
      catch { }
      try {
        if ($null -ne (Get-ExactScheduledTask -TaskLocation $taskLocation)) {
          Unregister-ScheduledTask -TaskPath '\MichStartupMaster\' -TaskName $shortName -Confirm:$false -ErrorAction Stop | Out-Null
        }
      }
      catch { [void]$cleanupErrors.Add("task-cleanup=$taskLocation $($_.Exception.Message)") }
      try {
        if ($null -ne (Get-ExactScheduledTask -TaskLocation $taskLocation)) { [void]$cleanupErrors.Add("task-remains=$taskLocation") }
      }
      catch { [void]$cleanupErrors.Add("task-verify=$taskLocation $($_.Exception.Message)") }
    }

    foreach ($candidateService in @($createdServices)) {
      try {
        & sc.exe delete $candidateService | Out-Null
        for ($attempt = 0; $attempt -lt 50; $attempt++) {
          if (-not (Test-Path -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$candidateService") -and $null -eq (Get-Service -Name $candidateService -ErrorAction SilentlyContinue)) { break }
          Start-Sleep -Milliseconds 100
        }
        if ((Test-Path -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$candidateService") -or $null -ne (Get-Service -Name $candidateService -ErrorAction SilentlyContinue)) {
          [void]$cleanupErrors.Add("service-remains=$candidateService")
        }
      }
      catch { [void]$cleanupErrors.Add("service-cleanup=$candidateService $($_.Exception.Message)") }
    }

    try {
      Restore-StateStoreSnapshot -Snapshot $startupFileSnapshot
      Assert-StateStoreSnapshotRestored -Snapshot $startupFileSnapshot
      if ($startupFolderExisted) {
        $startupMetadataError = $null
        for ($attempt = 0; $attempt -lt 50; $attempt++) {
          try {
            $currentStartupFolder = Get-Item -LiteralPath $startupFolder
            if ($currentStartupFolder.CreationTimeUtc -ne $startupFolderCreationUtc) {
              [IO.Directory]::SetCreationTimeUtc($startupFolder, $startupFolderCreationUtc)
            }
            if ($currentStartupFolder.Attributes -ne $startupFolderAttributes) {
              [IO.File]::SetAttributes($startupFolder, $startupFolderAttributes)
            }
            $startupMetadataError = $null
            break
          }
          catch {
            $startupMetadataError = $_
            Start-Sleep -Milliseconds 100
          }
        }
        if ($null -ne $startupMetadataError) { throw $startupMetadataError }
        $restoredStartupFolder = Get-Item -LiteralPath $startupFolder
        if ($restoredStartupFolder.CreationTimeUtc -ne $startupFolderCreationUtc -or $restoredStartupFolder.Attributes -ne $startupFolderAttributes) {
          throw "Startup-folder directory metadata was not restored exactly: $startupFolder"
        }
      } elseif (Test-Path -LiteralPath $startupFolder -PathType Container) {
        if ([IO.Directory]::GetFileSystemEntries($startupFolder).Length -ne 0) { throw "Startup folder was absent before the test but is now nonempty; refusing broad cleanup: $startupFolder" }
        [IO.Directory]::Delete($startupFolder, $false)
      }
      if ((Test-Path -LiteralPath $startupFolder -PathType Container) -ne $startupFolderExisted) { throw "Startup-folder directory existence was not restored: $startupFolder" }
    }
    catch { [void]$cleanupErrors.Add("startup-folder=$startupFile $($_.Exception.Message)") }

    try {
      if ($knownStoreEnvironmentExisted) { [Environment]::SetEnvironmentVariable('MSM_KNOWN_STORE', $knownStoreEnvironmentBefore, [EnvironmentVariableTarget]::Process) }
      else { [Environment]::SetEnvironmentVariable('MSM_KNOWN_STORE', $null, [EnvironmentVariableTarget]::Process) }
      $restoredEnvironmentEntry = Get-Item Env:MSM_KNOWN_STORE -ErrorAction SilentlyContinue
      if ($knownStoreEnvironmentExisted) {
        if ($null -eq $restoredEnvironmentEntry -or -not [string]::Equals([string]$restoredEnvironmentEntry.Value, $knownStoreEnvironmentBefore, [StringComparison]::Ordinal)) { [void]$cleanupErrors.Add('environment-not-restored=MSM_KNOWN_STORE') }
      } elseif ($null -ne $restoredEnvironmentEntry) { [void]$cleanupErrors.Add('environment-not-removed=MSM_KNOWN_STORE') }
    }
    catch { [void]$cleanupErrors.Add("environment-restore=MSM_KNOWN_STORE $($_.Exception.Message)") }

    try {
      if ($stateRootEnvironmentExisted) { [Environment]::SetEnvironmentVariable('MSM_STATE_ROOT', $stateRootEnvironmentBefore, [EnvironmentVariableTarget]::Process) }
      else { [Environment]::SetEnvironmentVariable('MSM_STATE_ROOT', $null, [EnvironmentVariableTarget]::Process) }
      $restoredStateRootEntry = Get-Item Env:MSM_STATE_ROOT -ErrorAction SilentlyContinue
      if ($stateRootEnvironmentExisted) {
        if ($null -eq $restoredStateRootEntry -or -not [string]::Equals([string]$restoredStateRootEntry.Value, $stateRootEnvironmentBefore, [StringComparison]::Ordinal)) { [void]$cleanupErrors.Add('environment-not-restored=MSM_STATE_ROOT') }
      } elseif ($null -ne $restoredStateRootEntry) { [void]$cleanupErrors.Add('environment-not-removed=MSM_STATE_ROOT') }
    }
    catch { [void]$cleanupErrors.Add("environment-restore=MSM_STATE_ROOT $($_.Exception.Message)") }
    $script:ActiveAppStateRoot = $activeAppStateRootBefore
    if (-not [string]::Equals([string]$script:ActiveAppStateRoot, [string]$activeAppStateRootBefore, [StringComparison]::OrdinalIgnoreCase)) { [void]$cleanupErrors.Add('active-state-root-not-restored') }

    foreach ($snapshot in $stateStoreSnapshots) {
      try {
        try { Assert-StateStoreSnapshotRestored -Snapshot $snapshot }
        catch {
          Restore-StateStoreSnapshot -Snapshot $snapshot
          Assert-StateStoreSnapshotRestored -Snapshot $snapshot
        }
      }
      catch { [void]$cleanupErrors.Add("state-store=$($snapshot.Path) $($_.Exception.Message)") }
    }

    try {
      $resolvedStateRoot = [IO.Path]::GetFullPath($isolatedStateRoot).TrimEnd('\')
      $resolvedTempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
      if (-not $resolvedStateRoot.StartsWith($resolvedTempRoot, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolvedStateRoot) -notlike 'MichStartupMaster-test-live-*') {
        throw "Refusing to clean an unvalidated isolated state root: $resolvedStateRoot"
      }
      if ($isolatedStateRootCreatedBySuite) {
        $stateRootDeleteError = $null
        for ($attempt = 0; $attempt -lt 50; $attempt++) {
          try {
            if (Test-Path -LiteralPath $resolvedStateRoot -PathType Container) { [IO.Directory]::Delete($resolvedStateRoot, $true) }
            $stateRootDeleteError = $null
            break
          }
          catch {
            $stateRootDeleteError = $_
            Start-Sleep -Milliseconds 100
          }
        }
        if ($null -ne $stateRootDeleteError) { throw $stateRootDeleteError }
        if (Test-Path -LiteralPath $resolvedStateRoot) { [void]$cleanupErrors.Add("isolated-state-root-remains=$resolvedStateRoot") }
      }
    }
    catch { [void]$cleanupErrors.Add("isolated-state-root=$isolatedStateRoot $($_.Exception.Message)") }

    try {
      if ($disposableTargetCreatedBySuite) {
        if (Test-Path -LiteralPath $disposableTarget -PathType Leaf) { Remove-Item -LiteralPath $disposableTarget -Force }
        if (Test-Path -LiteralPath $disposableTarget) { [void]$cleanupErrors.Add("fixture-remains=$disposableTarget") }
      }
    }
    catch { [void]$cleanupErrors.Add("fixture-cleanup=$disposableTarget $($_.Exception.Message)") }
  }

  if ($null -ne $suiteError -or $cleanupErrors.Count -ne 0) {
    $primary = if ($null -eq $suiteError) { 'none' } else { $suiteError.Exception.Message + ' at ' + $suiteError.ScriptStackTrace }
    $cleanup = if ($cleanupErrors.Count -eq 0) { 'none' } else { $cleanupErrors -join ' | ' }
    throw "Live mutation suite failed. primary=$primary cleanup=$cleanup"
  }
  foreach ($snapshot in $stateStoreSnapshots) { Assert-StateStoreSnapshotRestored -Snapshot $snapshot }
  foreach ($registrySnapshot in $registrySnapshots) { Assert-HkcuRegistryValueSnapshotRestored -Snapshot $registrySnapshot }
  Assert-StateStoreSnapshotRestored -Snapshot $startupFileSnapshot
  if (Test-Path -LiteralPath $isolatedStateRoot) { throw "Isolated state root remained after verified cleanup: $isolatedStateRoot" }
  "PASS live-cleanup tasks=$($createdTasks.Count) services=$($createdServices.Count) registryValues=$($registrySnapshots.Count) startupFolder=restored stores=$($stateStoreSnapshots.Count) isolatedStateRoot=removed environments=restored fixture=removed"
}

  Assert-HarnessIsolationOrder
  $script:RealAppDataEvidenceBefore = Get-AppDataEvidence -Path $script:RealAppDataRoot
  Start-IsolationWriteJournal
  Test-SafeProductContracts
  Assert-PureIsolationUnchanged

  if ($QuietLaunchOnly) {
    "SAFE_TESTS passed scope=quiet-only app=$App receipts=$RunDir"
  } elseif ($AllowLiveMutation) {
    Test-LiveMutationSuite
    "ALL_TESTS passed safe=true liveMutation=true app=$App receipts=$RunDir"
  } else {
    "SAFE_TESTS passed liveMutation=false app=$App receipts=$RunDir"
    'LIVE_MUTATION_TESTS skipped; rerun from an elevated shell with -AllowLiveMutation only when disposable Windows changes are intended.'
  }
}
catch {
  $script:HarnessPrimaryError = $_
}
finally {
  $script:AppIsolationInitialized = $false

  try {
    $protectedWriteEvents = @(Stop-IsolationWriteJournal)
    if ($protectedWriteEvents.Count -ne 0) {
      [void]$script:HarnessCleanupErrors.Add("protected-write-events=$($protectedWriteEvents.Count) $(@($protectedWriteEvents | Select-Object -First 10) -join ' | ')")
    }
  }
  catch { [void]$script:HarnessCleanupErrors.Add("write-journal-cleanup=$($_.Exception.Message)") }

  if ($null -ne $script:RealAppDataEvidenceBefore) {
    try {
      $finalRealAppDataEvidence = Get-AppDataEvidence -Path $script:RealAppDataRoot
      if (-not [string]::Equals([string]$script:RealAppDataEvidenceBefore, [string]$finalRealAppDataEvidence, [StringComparison]::Ordinal)) {
        [void]$script:HarnessCleanupErrors.Add("real-app-data-changed=$($script:RealAppDataRoot)")
      }
    }
    catch { [void]$script:HarnessCleanupErrors.Add("real-app-data-verification=$($_.Exception.Message)") }
  }

  try {
    if ($script:PureStateRootAbsentAtStart -and (Test-Path -LiteralPath $script:PureStateRoot)) {
      [void]$script:HarnessCleanupErrors.Add("pure-state-root-was-created=$($script:PureStateRoot)")
      $resolvedPureRootForCleanup = [IO.Path]::GetFullPath($script:PureStateRoot).TrimEnd('\')
      $resolvedRunRootForCleanup = [IO.Path]::GetFullPath($RunDir).TrimEnd('\') + '\'
      if (-not $resolvedPureRootForCleanup.StartsWith($resolvedRunRootForCleanup, [StringComparison]::OrdinalIgnoreCase) -or
          [IO.Path]::GetFileName($resolvedPureRootForCleanup) -notlike 'MichStartupMaster-test-*') {
        throw "Refusing to clean an unvalidated pure-test state root: $resolvedPureRootForCleanup"
      }
      if (Test-Path -LiteralPath $resolvedPureRootForCleanup -PathType Container) { [IO.Directory]::Delete($resolvedPureRootForCleanup, $true) }
      elseif (Test-Path -LiteralPath $resolvedPureRootForCleanup -PathType Leaf) { [IO.File]::Delete($resolvedPureRootForCleanup) }
      if (Test-Path -LiteralPath $resolvedPureRootForCleanup) { throw "Pure-test state root remains after exact cleanup: $resolvedPureRootForCleanup" }
    }
  }
  catch { [void]$script:HarnessCleanupErrors.Add("pure-state-root-cleanup=$($_.Exception.Message)") }

  try {
    if ($script:OriginalKnownStoreEnvironmentExisted) { [Environment]::SetEnvironmentVariable('MSM_KNOWN_STORE', $script:OriginalKnownStoreEnvironmentValue, [EnvironmentVariableTarget]::Process) }
    else {
      [Environment]::SetEnvironmentVariable('MSM_KNOWN_STORE', $null, [EnvironmentVariableTarget]::Process)
      # Windows PowerShell can retain an empty Env: provider entry after the .NET
      # process-environment API deletes the variable. Remove that exact provider
      # entry too so the caller's originally absent environment is restored.
      Remove-Item -LiteralPath 'Env:MSM_KNOWN_STORE' -ErrorAction SilentlyContinue
    }
    $restoredTopKnownStore = Get-Item Env:MSM_KNOWN_STORE -ErrorAction SilentlyContinue
    if ($script:OriginalKnownStoreEnvironmentExisted) {
      if ($null -eq $restoredTopKnownStore -or -not [string]::Equals([string]$restoredTopKnownStore.Value, $script:OriginalKnownStoreEnvironmentValue, [StringComparison]::Ordinal)) { throw 'MSM_KNOWN_STORE was not restored exactly.' }
    } elseif ($null -ne $restoredTopKnownStore) { throw 'MSM_KNOWN_STORE was not removed after the harness.' }
  }
  catch { [void]$script:HarnessCleanupErrors.Add("top-level-environment-restore=MSM_KNOWN_STORE $($_.Exception.Message)") }

  try {
    if ($script:OriginalStateRootEnvironmentExisted) { [Environment]::SetEnvironmentVariable('MSM_STATE_ROOT', $script:OriginalStateRootEnvironmentValue, [EnvironmentVariableTarget]::Process) }
    else {
      [Environment]::SetEnvironmentVariable('MSM_STATE_ROOT', $null, [EnvironmentVariableTarget]::Process)
      Remove-Item -LiteralPath 'Env:MSM_STATE_ROOT' -ErrorAction SilentlyContinue
    }
    $restoredTopStateRoot = Get-Item Env:MSM_STATE_ROOT -ErrorAction SilentlyContinue
    if ($script:OriginalStateRootEnvironmentExisted) {
      if ($null -eq $restoredTopStateRoot -or -not [string]::Equals([string]$restoredTopStateRoot.Value, $script:OriginalStateRootEnvironmentValue, [StringComparison]::Ordinal)) { throw 'MSM_STATE_ROOT was not restored exactly.' }
    } elseif ($null -ne $restoredTopStateRoot) { throw 'MSM_STATE_ROOT was not removed after the harness.' }
  }
  catch { [void]$script:HarnessCleanupErrors.Add("top-level-environment-restore=MSM_STATE_ROOT $($_.Exception.Message)") }
  $script:ActiveAppStateRoot = ''
}

if ($null -ne $script:HarnessPrimaryError -or $script:HarnessCleanupErrors.Count -ne 0) {
  $primaryFailure = if ($null -eq $script:HarnessPrimaryError) { 'none' } else { $script:HarnessPrimaryError.Exception.Message }
  $cleanupFailure = if ($script:HarnessCleanupErrors.Count -eq 0) { 'none' } else { $script:HarnessCleanupErrors -join ' | ' }
  throw "Harness failed. primary=$primaryFailure cleanup=$cleanupFailure"
}
