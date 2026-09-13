; Mich Startup Master — per-user Windows installer (no administrator rights required).
; Build only through scripts\build.ps1 -StageOnly -CompileInstaller. The required
; generated include binds this installer to one hash-validated staged publish.

#ifndef ValidatedStageMetadata
  #error ValidatedStageMetadata is required. Use scripts\build.ps1 -StageOnly -CompileInstaller.
#endif
#include ValidatedStageMetadata
#ifndef ValidatedPublishDir
  #error Validated stage metadata does not define ValidatedPublishDir.
#endif
#ifndef ValidatedStageReceipt
  #error Validated stage metadata does not define ValidatedStageReceipt.
#endif
#ifndef ValidatedInstallerOutputDir
  #error Validated stage metadata does not define ValidatedInstallerOutputDir.
#endif
#ifndef ValidatedStageReceiptSha256
  #error Validated stage metadata does not define ValidatedStageReceiptSha256.
#endif
#ifndef ValidatedPublishManifestSha256
  #error Validated stage metadata does not define ValidatedPublishManifestSha256.
#endif
#ifndef ValidatedExecutableSha256
  #error Validated stage metadata does not define ValidatedExecutableSha256.
#endif
#ifndef ValidatedTransactionId
  #error Validated stage metadata does not define ValidatedTransactionId.
#endif
#ifndef ValidatedPublishManifest
  #error Validated stage metadata does not define ValidatedPublishManifest.
#endif
#ifndef ValidatedPublishManifestFileSha256
  #error Validated stage metadata does not define ValidatedPublishManifestFileSha256.
#endif
#ifndef ValidatedStageMetadataSha256
  #error ValidatedStageMetadataSha256 must be supplied by the build transaction.
#endif
#ifndef ValidatedInstallProvenance
  #error ValidatedInstallProvenance must be supplied by the build transaction.
#endif
#ifndef ValidatedInstallProvenanceSha256
  #error ValidatedInstallProvenanceSha256 must be supplied by the build transaction.
#endif
#ifnexist AddBackslash(ValidatedPublishDir) + "MichStartupMaster.exe"
  #error Validated staged executable is missing.
#endif
#ifnexist ValidatedStageReceipt
  #error Validated stage receipt is missing.
#endif

#define MyAppName "Mich Startup Master"
#define MyAppVersion "2.0.0"
#define MyAppPublisher "Michaelunkai"
#define MyAppExeName "MichStartupMaster.exe"
#define MyAppId "{9E4E8B9C-5E2A-4D8B-9C3E-7F1A2B3C4D5E}"

[Setup]
AppId={{#MyAppId}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\MichStartupMaster
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
DisableWelcomePage=no
; Per-user install: works on any Windows 11 machine without elevation.
PrivilegesRequired=lowest
OutputDir={#ValidatedInstallerOutputDir}
OutputBaseFilename=MichStartupMaster-Setup
SetupIconFile=..\assets\MichStartupMaster.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName}
; Never let Inno terminate/restart name-matched processes without the exact
; path/command-line/start-generation journal enforced by scripts\build.ps1.
CloseApplications=no
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Files]
Source: "{#ValidatedPublishDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#ValidatedStageReceipt}"; DestDir: "{app}\release"; DestName: "stage-receipt.json"; Flags: ignoreversion
Source: "{#ValidatedStageMetadata}"; DestDir: "{app}\release"; DestName: "validated-stage.issinc"; Flags: ignoreversion
Source: "{#ValidatedPublishManifest}"; DestDir: "{app}\release"; DestName: "validated-publish.manifest"; Flags: ignoreversion
Source: "{#ValidatedInstallProvenance}"; DestDir: "{app}\release"; DestName: "install-provenance.json"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\MichStartupMaster.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\MichStartupMaster.ico"; Tasks: desktopicon

[UninstallDelete]
; Inno removes files it installed. Remove the directory only when it is empty so an
; untracked/user-added file under {app} is never recursively destroyed.
Type: dirifempty; Name: "{app}"

[Code]
const
  WaitObject0Value = 0;
  WaitAbandonedValue = $00000080;
  WaitTimeoutValue = $00000102;
  ErrorAlreadyExistsValue = 183;
  ProcessQueryLimitedInformationValue = $00001000;
  BCryptUseSystemPreferredRngValue = $00000002;

type
  TNativeFileTime = record
    LowDateTime: Cardinal;
    HighDateTime: Cardinal;
  end;

function CreateMutexNative(
  SecurityAttributes: LongWord; InitialOwner: Boolean;
  Name: String): THandle;
  external 'CreateMutexW@kernel32.dll stdcall';
function CreateEventNative(
  SecurityAttributes: LongWord; ManualReset, InitialState: Boolean;
  Name: String): THandle;
  external 'CreateEventW@kernel32.dll stdcall';
function WaitForSingleObjectNative(
  Handle: THandle; Milliseconds: Cardinal): Cardinal;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function ReleaseMutexNative(Handle: THandle): Boolean;
  external 'ReleaseMutex@kernel32.dll stdcall';
function CloseHandleNative(Handle: THandle): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';
function GetLastErrorNative(): Cardinal;
  external 'GetLastError@kernel32.dll stdcall';
function GetTickCount64Native(): Int64;
  external 'GetTickCount64@kernel32.dll stdcall';
function SetEnvironmentVariableNative(
  Name, Value: String): Boolean;
  external 'SetEnvironmentVariableW@kernel32.dll stdcall';
function OpenProcessNative(
  DesiredAccess: Cardinal; InheritHandle: Boolean;
  ProcessId: Cardinal): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function GetProcessTimesNative(
  ProcessHandle: THandle;
  var CreationTime, ExitTime, KernelTime, UserTime: TNativeFileTime): Boolean;
  external 'GetProcessTimes@kernel32.dll stdcall';
function BCryptGenRandomNative(
  Algorithm: THandle; var Buffer: Integer;
  BufferLength, Flags: Cardinal): Integer;
  external 'BCryptGenRandom@bcrypt.dll stdcall';

function AcquireNamedMutex(
  const Name: String; const TimeoutMilliseconds: Cardinal;
  var Handle: THandle; var Detail: String): Boolean;
var
  WaitResult: Cardinal;
begin
  Result := False;
  Detail := '';
  Handle := CreateMutexNative(0, False, Name);
  if Handle = 0 then
  begin
    Detail := 'CreateMutex failed: ' +
      SysErrorMessage(GetLastErrorNative());
    Exit;
  end;
  WaitResult := WaitForSingleObjectNative(Handle, TimeoutMilliseconds);
  if (WaitResult = WaitObject0Value) or
     (WaitResult = WaitAbandonedValue) then
    Result := True
  else
  begin
    if WaitResult = WaitTimeoutValue then
      Detail := 'timed out waiting for ' + Name
    else
      Detail := 'WaitForSingleObject failed for ' + Name + ': ' +
        SysErrorMessage(GetLastErrorNative());
    CloseHandleNative(Handle);
    Handle := 0;
  end;
end;

function ReacquireNamedMutex(
  const Name: String; const Handle: THandle;
  const TimeoutMilliseconds: Cardinal; var Detail: String): Boolean;
var
  WaitResult: Cardinal;
begin
  Result := False;
  Detail := '';
  if Handle = 0 then
  begin
    Detail := 'the mutex handle is not open: ' + Name;
    Exit;
  end;
  WaitResult := WaitForSingleObjectNative(Handle, TimeoutMilliseconds);
  if (WaitResult = WaitObject0Value) or
     (WaitResult = WaitAbandonedValue) then
    Result := True
  else if WaitResult = WaitTimeoutValue then
    Detail := 'timed out reacquiring ' + Name
  else
    Detail := 'WaitForSingleObject failed while reacquiring ' + Name +
      ': ' + SysErrorMessage(GetLastErrorNative());
end;

procedure ReleaseNamedMutexOwnership(
  const Name: String; const Handle: THandle);
begin
  if (Handle = 0) or (not ReleaseMutexNative(Handle)) then
    RaiseException('Could not release ' + Name + ': ' +
      SysErrorMessage(GetLastErrorNative()));
end;

procedure ReleaseAndCloseNamedMutex(
  const Name: String; var Handle: THandle; const Held: Boolean);
begin
  if Handle = 0 then
    Exit;
  if Held and (not ReleaseMutexNative(Handle)) then
    Log('Could not release ' + Name + ' during cleanup: ' +
      SysErrorMessage(GetLastErrorNative()));
  if not CloseHandleNative(Handle) then
    Log('Could not close ' + Name + ' during cleanup: ' +
      SysErrorMessage(GetLastErrorNative()));
  Handle := 0;
end;

function NewReleaseChildToken(): String;
var
  Words: TArrayOfInteger;
  Index, ByteValue: Integer;
begin
  SetArrayLength(Words, 8);
  if BCryptGenRandomNative(0, Words[0], 32,
       BCryptUseSystemPreferredRngValue) <> 0 then
    RaiseException(
      'Windows cryptographic random generation failed for the one-shot release-child token.');
  Result := '';
  for Index := 0 to 31 do
  begin
    ByteValue := (Words[Index div 4] shr ((Index mod 4) * 8)) and $FF;
    Result := Result + Format('%.2x', [ByteValue]);
  end;
  if Length(Result) <> 64 then
    RaiseException('The release-child token generator returned an invalid length.');
end;

function CreateReleaseChildProofEvent(
  const Token: String; var Handle: THandle; var Detail: String): Boolean;
var
  LastError: Cardinal;
begin
  Result := False;
  Detail := '';
  Handle := CreateEventNative(0, True, False,
    'Local\MichStartupMaster.ReleaseChild.' + Token);
  LastError := GetLastErrorNative();
  if Handle = 0 then
    Detail := 'CreateEvent failed: ' + SysErrorMessage(LastError)
  else if LastError = ErrorAlreadyExistsValue then
  begin
    Detail := 'the one-shot release-child proof event already exists';
    CloseHandleNative(Handle);
    Handle := 0;
  end
  else
    Result := True;
end;

procedure CloseReleaseChildProofEvent(var Handle: THandle);
begin
  if Handle = 0 then
    Exit;
  if not CloseHandleNative(Handle) then
    Log('Could not close the release-child proof event: ' +
      SysErrorMessage(GetLastErrorNative()));
  Handle := 0;
end;

function RunAuthenticatedReleaseChild(
  const Command, Token: String; const TimeoutMilliseconds: Cardinal;
  var ResultCode: Integer; var Receipt, Detail: String): Boolean;
var
  Shell, Child: Variant;
  PriorDirectory, PriorToken, CommandLine, StdoutText, StderrText: String;
  ProcessId: Integer;
  ProcessHandle: THandle;
  CreationTime, ExitTime, KernelTime, UserTime: TNativeFileTime;
  Deadline: Int64;
  EnvironmentRestored: Boolean;
  EnvironmentRestoreError: Cardinal;
begin
  Result := False;
  ResultCode := -1;
  Receipt := '';
  Detail := '';
  Child := Null;
  ProcessHandle := 0;
  PriorToken := GetEnv('MICH_STARTUP_MASTER_RELEASE_CHILD_TOKEN');
  try
    Shell := CreateOleObject('WScript.Shell');
    PriorDirectory := Shell.CurrentDirectory;
    Shell.CurrentDirectory := ExpandConstant('{app}');
    if not SetEnvironmentVariableNative(
      'MICH_STARTUP_MASTER_RELEASE_CHILD_TOKEN', Token) then
      RaiseException('SetEnvironmentVariable failed: ' +
        SysErrorMessage(GetLastErrorNative()));
    try
      CommandLine := '"' + ExpandConstant('{app}\{#MyAppExeName}') +
        '" ' + Command + ' --release-gate-child';
      Child := Shell.Exec(CommandLine);
    finally
      EnvironmentRestored := SetEnvironmentVariableNative(
        'MICH_STARTUP_MASTER_RELEASE_CHILD_TOKEN', PriorToken);
      if EnvironmentRestored then
        EnvironmentRestoreError := 0
      else
        EnvironmentRestoreError := GetLastErrorNative();
      Shell.CurrentDirectory := PriorDirectory;
      if not EnvironmentRestored then
        RaiseException('Could not clear the release-child token from the installer environment: ' +
          SysErrorMessage(EnvironmentRestoreError));
    end;

    ProcessId := Child.ProcessID;
    ProcessHandle := OpenProcessNative(
      ProcessQueryLimitedInformationValue, False, ProcessId);
    if ProcessHandle = 0 then
      RaiseException('could not open the exact release child PID ' +
        IntToStr(ProcessId) + ': ' + SysErrorMessage(GetLastErrorNative()));
    if not GetProcessTimesNative(ProcessHandle, CreationTime, ExitTime,
      KernelTime, UserTime) then
      RaiseException('could not capture the exact release child generation: ' +
        SysErrorMessage(GetLastErrorNative()));
    CloseHandleNative(ProcessHandle);
    ProcessHandle := 0;

    Deadline := GetTickCount64Native() + TimeoutMilliseconds;
    while (Child.Status = 0) and
          (GetTickCount64Native() < Deadline) do
      Sleep(50);
    if Child.Status = 0 then
    begin
      Child.Terminate();
      Deadline := GetTickCount64Native() + 5000;
      while (Child.Status = 0) and
            (GetTickCount64Native() < Deadline) do
        Sleep(50);
      Detail := Format(
        '%s timed out; exact PID=%d generation=%u:%u termination requested', [Command, ProcessId, CreationTime.HighDateTime,
         CreationTime.LowDateTime]);
      if Child.Status = 0 then
        Detail := Detail + ' but termination was NOT confirmed'
      else
        Detail := Detail + ' and termination was confirmed';
      Exit;
    end;

    ResultCode := Child.ExitCode;
    { WshScriptExec stream handles can remain open in an inherited descendant even
      after the exact child exits. Do not call ReadAll here: the process wait is
      bounded and the receipt remains truthful without an unbounded stream drain. }
    StdoutText := '<not-read: bounded child contract>';
    StderrText := '<not-read: bounded child contract>';
    Receipt := Format(
      'PID=%d; generation=%u:%u; exit=%d; stdout=%s; stderr=%s', [ProcessId, CreationTime.HighDateTime, CreationTime.LowDateTime,
       ResultCode, StdoutText, StderrText]);
    if ResultCode <> 0 then
    begin
      Detail := Command + ' exited with code ' + IntToStr(ResultCode) +
        '. ' + Receipt;
      Exit;
    end;
    Result := True;
  except
    Detail := GetExceptionMessage;
    if not VarIsNull(Child) then
    begin
      try
        if Child.Status = 0 then
          Child.Terminate();
      except
        Detail := Detail + '; exact-child termination also failed: ' +
          GetExceptionMessage;
      end;
    end;
  end;
  if ProcessHandle <> 0 then
    CloseHandleNative(ProcessHandle);
end;

function PathsEqual(const LeftPath, RightPath: String): Boolean;
begin
  Result :=
    (LeftPath <> '') and
    (RightPath <> '') and
    (CompareText(ExpandFileName(LeftPath), ExpandFileName(RightPath)) = 0);
end;

function IsSafeRelativePayloadPath(const RelativePath: String): Boolean;
begin
  Result := False;
  if RelativePath = '' then
    Exit;
  if (RelativePath = '.') or (RelativePath = '..') then
    Exit;
  if RelativePath[1] = '\' then
    Exit;
  if (Pos(':', RelativePath) > 0) or
     (Pos('/', RelativePath) > 0) or
     (Pos('..\', RelativePath) > 0) or
     (Pos('\..', RelativePath) > 0) then
    Exit;
  Result := True;
end;

procedure VerifyInstalledPayload();
var
  ManifestPath, StageReceiptPath, StageMetadataPath, ProvenancePath,
    AppRoot, AppPrefix, Line, Remainder, RelativePath, ExpectedHash,
    InstalledPath: String;
  Lines: TArrayOfString;
  I, FirstSeparator, SecondSeparator: Integer;
begin
  ManifestPath := ExpandConstant('{app}\release\validated-publish.manifest');
  StageReceiptPath := ExpandConstant('{app}\release\stage-receipt.json');
  StageMetadataPath := ExpandConstant('{app}\release\validated-stage.issinc');
  ProvenancePath := ExpandConstant('{app}\release\install-provenance.json');

  if (not FileExists(ManifestPath)) or
     (CompareText(GetSHA256OfFile(ManifestPath),
       '{#ValidatedPublishManifestFileSha256}') <> 0) then
    RaiseException('The installed publish manifest is missing or hash-invalid.');
  if (not FileExists(StageReceiptPath)) or
     (CompareText(GetSHA256OfFile(StageReceiptPath),
       '{#ValidatedStageReceiptSha256}') <> 0) then
    RaiseException('The installed stage receipt is missing or hash-invalid.');
  if (not FileExists(StageMetadataPath)) or
     (CompareText(GetSHA256OfFile(StageMetadataPath),
       '{#ValidatedStageMetadataSha256}') <> 0) then
    RaiseException('The installed stage metadata is missing or hash-invalid.');
  if (not FileExists(ProvenancePath)) or
     (CompareText(GetSHA256OfFile(ProvenancePath),
       '{#ValidatedInstallProvenanceSha256}') <> 0) then
    RaiseException('The installed transaction provenance is missing or hash-invalid.');

  if not LoadStringsFromFile(ManifestPath, Lines) then
    RaiseException('The installed publish manifest could not be read.');
  if GetArrayLength(Lines) = 0 then
    RaiseException('The installed publish manifest is empty.');
  AppRoot := ExpandFileName(ExpandConstant('{app}'));
  AppPrefix := AddBackslash(AppRoot);
  for I := 0 to GetArrayLength(Lines) - 1 do
  begin
    Line := Lines[I];
    FirstSeparator := Pos('|', Line);
    if FirstSeparator <= 1 then
      RaiseException(Format('Malformed publish-manifest row %d.', [I + 1]));
    ExpectedHash := Copy(Line, 1, FirstSeparator - 1);
    Remainder := Copy(Line, FirstSeparator + 1, Length(Line));
    SecondSeparator := Pos('|', Remainder);
    if SecondSeparator <= 1 then
      RaiseException(Format('Malformed publish-manifest row %d.', [I + 1]));
    RelativePath := Copy(Remainder, SecondSeparator + 1, Length(Remainder));
    if not IsSafeRelativePayloadPath(RelativePath) then
      RaiseException('Unsafe path in the validated publish manifest: ' + RelativePath);
    InstalledPath := ExpandFileName(AppPrefix + RelativePath);
    if CompareText(Copy(InstalledPath, 1, Length(AppPrefix)), AppPrefix) <> 0 then
      RaiseException('Publish-manifest path escapes the installation directory: ' + RelativePath);
    if (not FileExists(InstalledPath)) or
       (CompareText(GetSHA256OfFile(InstalledPath), ExpectedHash) <> 0) then
      RaiseException('Installed payload hash mismatch: ' + RelativePath);
  end;
  Log(Format(
    'Validated complete installed payload for transaction {#ValidatedTransactionId}; manifest SHA-256: {#ValidatedPublishManifestSha256}; files: %d', [GetArrayLength(Lines)]));
end;

function IsAllowedSystemScriptHost(const TargetPath: String): Boolean;
begin
  Result :=
    PathsEqual(TargetPath, ExpandConstant('{sys}\wscript.exe')) or
    PathsEqual(TargetPath, ExpandConstant('{sys}\cscript.exe')) or
    PathsEqual(TargetPath, ExpandConstant('{syswow64}\wscript.exe')) or
    PathsEqual(TargetPath, ExpandConstant('{syswow64}\cscript.exe'));
end;

function IsOwnedLauncherArguments(
  const Arguments, ExpectedLauncher: String): Boolean;
var
  TrimmedArguments: String;
begin
  TrimmedArguments := Trim(Arguments);
  Result :=
    (CompareText(TrimmedArguments, '"' + ExpectedLauncher + '"') = 0) or
    (CompareText(TrimmedArguments, ExpectedLauncher) = 0) or
    (CompareText(TrimmedArguments, '//B //NoLogo "' + ExpectedLauncher + '"') = 0) or
    (CompareText(TrimmedArguments, '//B "' + ExpectedLauncher + '"') = 0);
end;

function IsOwnedAgentLaunch(
  const TargetPath, Arguments: String): Boolean;
var
  ExpectedExe, ExpectedLauncher, TrimmedArguments: String;
begin
  ExpectedExe := ExpandConstant('{app}\{#MyAppExeName}');
  ExpectedLauncher := ExpandConstant('{app}\MichStartupMasterAgent.vbs');
  TrimmedArguments := Trim(Arguments);

  Result :=
    (PathsEqual(TargetPath, ExpectedExe) and
      (CompareText(TrimmedArguments, '--agent') = 0)) or
    (IsAllowedSystemScriptHost(TargetPath) and
      IsOwnedLauncherArguments(TrimmedArguments, ExpectedLauncher));
end;

function IsRecognizedPriorAgentLaunch(
  const TargetPath, Arguments: String): Boolean;
begin
  { A same-basename executable elsewhere is not this installation. }
  Result := IsOwnedAgentLaunch(TargetPath, Arguments);
end;

function IsOwnedAgentShortcut(const ShortcutPath: String): Boolean;
var
  Shell, Shortcut: Variant;
begin
  Result := False;
  if not FileExists(ShortcutPath) then
    Exit;

  try
    Shell := CreateOleObject('WScript.Shell');
    Shortcut := Shell.CreateShortcut(ShortcutPath);
    Result := IsOwnedAgentLaunch(Shortcut.TargetPath, Shortcut.Arguments);
  except
    Result := False;
  end;
end;

function TryFindReservedAgentTask(
  var TaskExists: Boolean; var TaskObject, FolderObject: Variant;
  var Detail: String): Boolean;
var
  Scheduler, RootFolder, Folders, CandidateFolder, Tasks, CandidateTask: Variant;
  I, J: Integer;
begin
  Result := False;
  TaskExists := False;
  Detail := '';
  try
    Scheduler := CreateOleObject('Schedule.Service');
    Scheduler.Connect();
    RootFolder := Scheduler.GetFolder('\');
    Folders := RootFolder.GetFolders(0);
    for I := 1 to Folders.Count do
    begin
      CandidateFolder := Folders.Item(I);
      if CompareText(CandidateFolder.Name, 'MichStartupMaster') = 0 then
      begin
        FolderObject := CandidateFolder;
        Tasks := CandidateFolder.GetTasks(1);
        for J := 1 to Tasks.Count do
        begin
          CandidateTask := Tasks.Item(J);
          if CompareText(CandidateTask.Name, 'MichStartupMasterApp') = 0 then
          begin
            TaskObject := CandidateTask;
            TaskExists := True;
          end;
        end;
      end;
    end;
    Result := True;
  except
    Detail := GetExceptionMessage;
  end;
end;

function TaskHasExactOwnedAction(TaskObject: Variant): Boolean;
var
  Actions, Action: Variant;
begin
  Result := False;
  try
    Actions := TaskObject.Definition.Actions;
    if Actions.Count <> 1 then
      Exit;
    Action := Actions.Item(1);
    Result := IsOwnedAgentLaunch(Action.Path, Action.Arguments);
  except
    Result := False;
  end;
end;

function IsCanonicalOwnedAgentTaskObject(TaskObject: Variant): Boolean;
var
  Definition, Actions, Action, Triggers, Trigger, Principal, Settings: Variant;
  TriggerEnabled, StartWhenAvailable, DisallowStartOnBattery,
    StopOnBattery: Boolean;
  TriggerDelay, TriggerUserId, PrincipalUserId: String;
  PrincipalLogonType, PrincipalRunLevel: Integer;
begin
  Result := False;
  try
    if not TaskHasExactOwnedAction(TaskObject) then
      Exit;
    if not TaskObject.Enabled then
      Exit;
    Definition := TaskObject.Definition;
    Actions := Definition.Actions;
    Action := Actions.Item(1);
    if not PathsEqual(Action.WorkingDirectory, ExpandConstant('{app}')) then
      Exit;
    Triggers := Definition.Triggers;
    if Triggers.Count <> 1 then
      Exit;
    Trigger := Triggers.Item(1);
    { UserId exists on ILogonTrigger; accessing it also rejects other trigger kinds. }
    TriggerEnabled := Trigger.Enabled;
    TriggerDelay := Trigger.Delay;
    TriggerUserId := Trigger.UserId;
    if (not TriggerEnabled) or (Trim(TriggerDelay) <> '') or
       (Trim(TriggerUserId) = '') then
      Exit;
    Principal := Definition.Principal;
    PrincipalLogonType := Principal.LogonType;
    PrincipalRunLevel := Principal.RunLevel;
    PrincipalUserId := Principal.UserId;
    if (PrincipalLogonType <> 3) or (PrincipalRunLevel <> 0) or
       (Trim(PrincipalUserId) = '') or
       (CompareText(TriggerUserId, PrincipalUserId) <> 0) then
      Exit;
    Settings := Definition.Settings;
    StartWhenAvailable := Settings.StartWhenAvailable;
    DisallowStartOnBattery := Settings.DisallowStartIfOnBatteries;
    StopOnBattery := Settings.StopIfGoingOnBatteries;
    Result :=
      StartWhenAvailable and
      (not DisallowStartOnBattery) and
      (not StopOnBattery);
  except
    Result := False;
  end;
end;

function ReadReservedAgentTask(
  var TaskExists, Recognized: Boolean;
  var XmlText, UserId, SecurityDescriptor: String;
  var LogonType: Integer; var Detail: String): Boolean;
var
  TaskObject, FolderObject: Variant;
begin
  Result := TryFindReservedAgentTask(
    TaskExists, TaskObject, FolderObject, Detail);
  Recognized := False;
  XmlText := '';
  UserId := '';
  SecurityDescriptor := '';
  LogonType := 0;
  if (not Result) or (not TaskExists) then
    Exit;
  try
    XmlText := TaskObject.Xml;
    UserId := TaskObject.Definition.Principal.UserId;
    LogonType := TaskObject.Definition.Principal.LogonType;
    SecurityDescriptor := TaskObject.GetSecurityDescriptor(7);
    Recognized :=
      TaskHasExactOwnedAction(TaskObject) and
      (LogonType = 3) and (Trim(UserId) <> '') and
      (Trim(SecurityDescriptor) <> '');
  except
    Result := False;
    Detail := GetExceptionMessage;
  end;
end;

function RestorePriorAgentTask(
  const TaskExisted: Boolean;
  const TaskXml, PriorUserId, PriorSecurityDescriptor: String;
  const PriorLogonType: Integer; var Detail: String): Boolean;
var
  TaskExists: Boolean;
  TaskObject, FolderObject, RegisteredTask, Scheduler, RootFolder: Variant;
  FindDetail: String;
begin
  Result := False;
  Detail := '';
  if TaskExisted then
  begin
    if not TryFindReservedAgentTask(
      TaskExists, TaskObject, FolderObject, FindDetail) then
    begin
      Detail := 'could not reopen Task Scheduler for restore: ' + FindDetail;
      Exit;
    end;
    if TaskExists and (not TaskHasExactOwnedAction(TaskObject)) then
    begin
      Detail := 'a non-owned task now occupies the reserved path; it was preserved';
      Exit;
    end;
    try
      Scheduler := CreateOleObject('Schedule.Service');
      Scheduler.Connect();
      RootFolder := Scheduler.GetFolder('\');
      try
        FolderObject := Scheduler.GetFolder('\MichStartupMaster');
      except
        FolderObject := RootFolder.CreateFolder('MichStartupMaster', '');
      end;
    except
      Detail := 'could not reopen or recreate the exact task folder: ' +
        GetExceptionMessage;
      Exit;
    end;
    try
      { RegisterTask accepts the COM/BSTR XML directly. No ANSI temporary file is
        involved, so a UTF-16 XML declaration can never disagree with its bytes. }
      RegisteredTask := FolderObject.RegisterTask(
        'MichStartupMasterApp', TaskXml, 6,
        PriorUserId, '', PriorLogonType, '');
      { TASK_DONT_ADD_PRINCIPAL_ACE = $10; preserve the exact captured DACL. }
      RegisteredTask.SetSecurityDescriptor(PriorSecurityDescriptor, 16);
      if CompareText(RegisteredTask.Xml, TaskXml) <> 0 then
      begin
        Detail := 'Task Scheduler normalized the restored XML differently';
        Exit;
      end;
      if CompareText(
        RegisteredTask.GetSecurityDescriptor(7),
        PriorSecurityDescriptor) <> 0 then
      begin
        Detail := 'restored task security descriptor differs';
        Exit;
      end;
      Result := True;
    except
      Detail := GetExceptionMessage;
    end;
  end
  else
  begin
    if not TryFindReservedAgentTask(
      TaskExists, TaskObject, FolderObject, FindDetail) then
    begin
      Detail := 'could not inspect a newly created task: ' + FindDetail;
      Exit;
    end;
    if not TaskExists then
    begin
      Result := True;
      Exit;
    end;
    if not IsCanonicalOwnedAgentTaskObject(TaskObject) then
    begin
      Detail := 'a non-canonical reserved task appeared; it was preserved';
      Exit;
    end;
    try
      FolderObject.DeleteTask('MichStartupMasterApp', 0);
      if not TryFindReservedAgentTask(
        TaskExists, TaskObject, FolderObject, FindDetail) then
      begin
        Detail := 'could not verify removal of the newly created task: ' + FindDetail;
        Exit;
      end;
      Result := not TaskExists;
      if not Result then
        Detail := 'the newly created task remains after deletion';
    except
      Detail := GetExceptionMessage;
    end;
  end;
end;

function ReadReservedAgentShortcut(
  var ShortcutExists, Recognized: Boolean; var Detail: String): Boolean;
var
  ShortcutPath: String;
  Shell, Shortcut: Variant;
begin
  Result := False;
  ShortcutExists := False;
  Recognized := False;
  Detail := '';
  ShortcutPath := ExpandConstant(
    '{userstartup}\Mich Startup Master Agent.lnk');
  if not FileExists(ShortcutPath) then
  begin
    Result := True;
    Exit;
  end;
  ShortcutExists := True;
  try
    Shell := CreateOleObject('WScript.Shell');
    Shortcut := Shell.CreateShortcut(ShortcutPath);
    Recognized := IsRecognizedPriorAgentLaunch(
      Shortcut.TargetPath, Shortcut.Arguments);
    Result := True;
  except
    Detail := GetExceptionMessage;
  end;
end;

function RestorePriorAgentShortcut(
  const ShortcutExisted: Boolean; const BackupPath, ExpectedHash: String;
  var Detail: String): Boolean;
var
  ShortcutPath: String;
begin
  Result := False;
  Detail := '';
  ShortcutPath := ExpandConstant(
    '{userstartup}\Mich Startup Master Agent.lnk');
  if ShortcutExisted then
  begin
    if FileExists(ShortcutPath) and
       (CompareText(GetSHA256OfFile(ShortcutPath), ExpectedHash) <> 0) and
       (not IsOwnedAgentShortcut(ShortcutPath)) then
    begin
      Detail := 'an unrelated shortcut now occupies the reserved path';
      Exit;
    end;
    if not CopyFile(BackupPath, ShortcutPath, False) then
    begin
      Detail := 'could not restore the prior shortcut bytes';
      Exit;
    end;
    Result := CompareText(
      GetSHA256OfFile(ShortcutPath), ExpectedHash) = 0;
    if not Result then
      Detail := 'restored shortcut hash mismatch';
  end
  else if FileExists(ShortcutPath) then
  begin
    if not IsOwnedAgentShortcut(ShortcutPath) then
    begin
      Detail := 'an unrelated shortcut appeared at the reserved path';
      Exit;
    end;
    Result := DeleteFile(ShortcutPath);
    if not Result then
      Detail := 'could not remove the newly created owned shortcut';
  end
  else
    Result := True;
end;

function DirectoryTreesEqual(const LeftPath, RightPath: String): Boolean;
var
  FindRec: TFindRec;
  LeftEntry, RightEntry: String;
  LeftIsDirectory, RightIsDirectory: Boolean;
begin
  Result := False;
  if DirExists(LeftPath) <> DirExists(RightPath) then
    Exit;
  if not DirExists(LeftPath) then
  begin
    Result := True;
    Exit;
  end;
  if FindFirst(AddBackslash(LeftPath) + '*', FindRec) then
  begin
    try
      repeat
        if (FindRec.Name <> '.') and (FindRec.Name <> '..') then
        begin
          LeftEntry := AddBackslash(LeftPath) + FindRec.Name;
          RightEntry := AddBackslash(RightPath) + FindRec.Name;
          LeftIsDirectory := (FindRec.Attributes and FILE_ATTRIBUTE_DIRECTORY) <> 0;
          if LeftIsDirectory then
          begin
            if (not DirExists(RightEntry)) or
               (not DirectoryTreesEqual(LeftEntry, RightEntry)) then
              Exit;
          end
          else if (not FileExists(RightEntry)) or
                  (CompareText(GetSHA256OfFile(LeftEntry),
                    GetSHA256OfFile(RightEntry)) <> 0) then
            Exit;
        end;
      until not FindNext(FindRec);
    finally
      FindClose(FindRec);
    end;
  end;
  if FindFirst(AddBackslash(RightPath) + '*', FindRec) then
  begin
    try
      repeat
        if (FindRec.Name <> '.') and (FindRec.Name <> '..') then
        begin
          LeftEntry := AddBackslash(LeftPath) + FindRec.Name;
          RightEntry := AddBackslash(RightPath) + FindRec.Name;
          RightIsDirectory :=
            (FindRec.Attributes and FILE_ATTRIBUTE_DIRECTORY) <> 0;
          if RightIsDirectory then
          begin
            if not DirExists(LeftEntry) then
              Exit;
          end
          else if not FileExists(LeftEntry) then
            Exit;
        end;
      until not FindNext(FindRec);
    finally
      FindClose(FindRec);
    end;
  end;
  Result := True;
end;

function CopyDirectorySnapshot(
  const SourcePath, DestinationPath: String; var Detail: String): Boolean;
var
  Fso: Variant;
begin
  Result := False;
  Detail := '';
  try
    if DirExists(DestinationPath) and
       (not DelTree(DestinationPath, True, True, True)) then
    begin
      Detail := 'could not clear the exact snapshot destination';
      Exit;
    end;
    Fso := CreateOleObject('Scripting.FileSystemObject');
    Fso.CopyFolder(SourcePath, DestinationPath, True);
    Result := DirectoryTreesEqual(SourcePath, DestinationPath);
    if not Result then
      Detail := 'the copied directory snapshot differs from its source';
  except
    Detail := GetExceptionMessage;
  end;
end;

function RestoreAppDataSnapshot(
  const AppDataPath, BackupPath: String; const PriorExisted: Boolean;
  var Detail: String): Boolean;
var
  CopyDetail: String;
begin
  Result := False;
  Detail := '';
  if FileExists(AppDataPath) then
  begin
    Detail := 'a non-directory object now occupies the exact AppData path and was preserved';
    Exit;
  end;
  if DirExists(AppDataPath) and
     (not DelTree(AppDataPath, True, True, True)) then
  begin
    Detail := 'could not remove the failed exact AppData tree';
    Exit;
  end;
  if PriorExisted then
  begin
    if not CopyDirectorySnapshot(BackupPath, AppDataPath, CopyDetail) then
    begin
      Detail := 'could not restore the AppData snapshot: ' + CopyDetail;
      Exit;
    end;
    Result := DirectoryTreesEqual(BackupPath, AppDataPath);
    if not Result then
      Detail := 'restored AppData differs from the transaction snapshot';
  end
  else
  begin
    Result := not DirExists(AppDataPath);
    if not Result then
      Detail := 'AppData should be absent after rollback';
  end;
end;

type
  TRegistryRouteSnapshot = record
    View: Integer;
    SubKey: String;
    Name: String;
    Value: String;
  end;

var
  RegistrationRegistryRoutes: array of TRegistryRouteSnapshot;

function RegistrationRegistryRoot(const View: Integer): Integer;
begin
  if View = 64 then
    Result := HKEY_CURRENT_USER_64
  else if View = 32 then
    Result := HKEY_CURRENT_USER_32
  else
    RaiseException(Format('Unsupported registry view: %d', [View]));
end;

function IsOwnedLauncherRegistryCommandForHost(
  const Value, HostPath, LauncherPath: String): Boolean;
begin
  Result :=
    (CompareText(Value, '"' + HostPath + '" "' + LauncherPath + '"') = 0) or
    (CompareText(Value, HostPath + ' "' + LauncherPath + '"') = 0) or
    (CompareText(Value, '"' + HostPath + '" //B //NoLogo "' + LauncherPath + '"') = 0) or
    (CompareText(Value, HostPath + ' //B //NoLogo "' + LauncherPath + '"') = 0) or
    (CompareText(Value, '"' + HostPath + '" //B "' + LauncherPath + '"') = 0) or
    (CompareText(Value, HostPath + ' //B "' + LauncherPath + '"') = 0);
end;

function IsOwnedAgentRegistryCommand(const Command: String): Boolean;
var
  ExePath, LauncherPath, Value: String;
begin
  ExePath := ExpandConstant('{app}\{#MyAppExeName}');
  LauncherPath := ExpandConstant('{app}\MichStartupMasterAgent.vbs');
  Value := Trim(Command);
  Result :=
    (CompareText(Value, '"' + ExePath + '" --agent') = 0) or
    (CompareText(Value, ExePath + ' --agent') = 0) or
    IsOwnedLauncherRegistryCommandForHost(
      Value, ExpandConstant('{sys}\wscript.exe'), LauncherPath) or
    IsOwnedLauncherRegistryCommandForHost(
      Value, ExpandConstant('{sys}\cscript.exe'), LauncherPath) or
    IsOwnedLauncherRegistryCommandForHost(
      Value, ExpandConstant('{syswow64}\wscript.exe'), LauncherPath) or
    IsOwnedLauncherRegistryCommandForHost(
      Value, ExpandConstant('{syswow64}\cscript.exe'), LauncherPath);
end;

function FindRegistrationRegistryRoute(
  const View: Integer; const SubKey, Name: String): Integer;
var
  I: Integer;
begin
  Result := -1;
  for I := 0 to GetArrayLength(RegistrationRegistryRoutes) - 1 do
    if (RegistrationRegistryRoutes[I].View = View) and
       (CompareText(RegistrationRegistryRoutes[I].SubKey, SubKey) = 0) and
       (CompareText(RegistrationRegistryRoutes[I].Name, Name) = 0) then
    begin
      Result := I;
      Exit;
    end;
end;

function CaptureOwnedRegistrationRegistry(var Detail: String): Boolean;
var
  Views: array[0..1] of Integer;
  SubKeys: array[0..1] of String;
  Names: TArrayOfString;
  ViewIndex, KeyIndex, NameIndex, RowIndex: Integer;
  Value: String;
begin
  Result := False;
  Detail := '';
  SetArrayLength(RegistrationRegistryRoutes, 0);
  Views[0] := 64;
  Views[1] := 32;
  SubKeys[0] := 'Software\Microsoft\Windows\CurrentVersion\Run';
  SubKeys[1] := 'Software\Microsoft\Windows\CurrentVersion\RunOnce';
  try
    for ViewIndex := 0 to 1 do
    begin
      for KeyIndex := 0 to 1 do
      begin
        if not RegKeyExists(
          RegistrationRegistryRoot(Views[ViewIndex]), SubKeys[KeyIndex]) then
          Continue;
        if not RegGetValueNames(
          RegistrationRegistryRoot(Views[ViewIndex]),
          SubKeys[KeyIndex], Names) then
        begin
          Detail := 'could not enumerate ' + SubKeys[KeyIndex];
          Exit;
        end;
        for NameIndex := 0 to GetArrayLength(Names) - 1 do
          if RegQueryStringValue(
               RegistrationRegistryRoot(Views[ViewIndex]),
               SubKeys[KeyIndex], Names[NameIndex], Value) and
             IsOwnedAgentRegistryCommand(Value) then
          begin
            RowIndex := GetArrayLength(RegistrationRegistryRoutes);
            SetArrayLength(RegistrationRegistryRoutes, RowIndex + 1);
            RegistrationRegistryRoutes[RowIndex].View := Views[ViewIndex];
            RegistrationRegistryRoutes[RowIndex].SubKey := SubKeys[KeyIndex];
            RegistrationRegistryRoutes[RowIndex].Name := Names[NameIndex];
            RegistrationRegistryRoutes[RowIndex].Value := Value;
          end;
      end;
    end;
    Result := True;
  except
    Detail := GetExceptionMessage;
  end;
end;

function RestoreOwnedRegistrationRegistry(var Detail: String): Boolean;
var
  Views: array[0..1] of Integer;
  SubKeys: array[0..1] of String;
  Names: TArrayOfString;
  ViewIndex, KeyIndex, NameIndex, RowIndex: Integer;
  Value, Errors: String;
begin
  Result := False;
  Detail := '';
  Errors := '';
  Views[0] := 64;
  Views[1] := 32;
  SubKeys[0] := 'Software\Microsoft\Windows\CurrentVersion\Run';
  SubKeys[1] := 'Software\Microsoft\Windows\CurrentVersion\RunOnce';
  try
    for ViewIndex := 0 to 1 do
    begin
      for KeyIndex := 0 to 1 do
        if RegKeyExists(
             RegistrationRegistryRoot(Views[ViewIndex]),
             SubKeys[KeyIndex]) then
        begin
          if not RegGetValueNames(
            RegistrationRegistryRoot(Views[ViewIndex]),
            SubKeys[KeyIndex], Names) then
            Errors := Errors + ' enumerate ' + SubKeys[KeyIndex]
          else
            for NameIndex := 0 to GetArrayLength(Names) - 1 do
              if RegQueryStringValue(
                   RegistrationRegistryRoot(Views[ViewIndex]),
                   SubKeys[KeyIndex], Names[NameIndex], Value) and
                 IsOwnedAgentRegistryCommand(Value) and
                 (FindRegistrationRegistryRoute(
                   Views[ViewIndex], SubKeys[KeyIndex], Names[NameIndex]) < 0) and
                  (not RegDeleteValue(
                    RegistrationRegistryRoot(Views[ViewIndex]),
                    SubKeys[KeyIndex], Names[NameIndex])) then
                Errors := Errors + ' remove ' + Names[NameIndex];
        end;
    end;
    for RowIndex := 0 to GetArrayLength(RegistrationRegistryRoutes) - 1 do
    begin
      if RegQueryStringValue(
           RegistrationRegistryRoot(
             RegistrationRegistryRoutes[RowIndex].View),
           RegistrationRegistryRoutes[RowIndex].SubKey,
           RegistrationRegistryRoutes[RowIndex].Name, Value) and
         (not IsOwnedAgentRegistryCommand(Value)) then
        Errors := Errors + ' preserve non-owned ' +
          RegistrationRegistryRoutes[RowIndex].Name
      else if not RegWriteStringValue(
             RegistrationRegistryRoot(
               RegistrationRegistryRoutes[RowIndex].View),
             RegistrationRegistryRoutes[RowIndex].SubKey,
             RegistrationRegistryRoutes[RowIndex].Name,
             RegistrationRegistryRoutes[RowIndex].Value) then
        Errors := Errors + ' restore ' +
          RegistrationRegistryRoutes[RowIndex].Name;
    end;
    for RowIndex := 0 to GetArrayLength(RegistrationRegistryRoutes) - 1 do
    begin
      if (not RegQueryStringValue(
            RegistrationRegistryRoot(
              RegistrationRegistryRoutes[RowIndex].View),
            RegistrationRegistryRoutes[RowIndex].SubKey,
            RegistrationRegistryRoutes[RowIndex].Name, Value)) or
         (CompareText(Value,
            RegistrationRegistryRoutes[RowIndex].Value) <> 0) then
        Errors := Errors + ' verify ' +
          RegistrationRegistryRoutes[RowIndex].Name;
    end;
    Result := Errors = '';
    if not Result then
      Detail := Trim(Errors);
  except
    Detail := GetExceptionMessage;
  end;
end;

function DeleteCapturedOwnedRegistrationRegistry(
  var Detail: String): Boolean;
var
  RowIndex: Integer;
  Value, Errors: String;
begin
  Result := False;
  Detail := '';
  Errors := '';
  try
    for RowIndex := 0 to GetArrayLength(RegistrationRegistryRoutes) - 1 do
    begin
      if not RegQueryStringValue(
           RegistrationRegistryRoot(
             RegistrationRegistryRoutes[RowIndex].View),
           RegistrationRegistryRoutes[RowIndex].SubKey,
           RegistrationRegistryRoutes[RowIndex].Name, Value) then
        Errors := Errors + ' missing ' +
          RegistrationRegistryRoutes[RowIndex].Name
      else if CompareText(
                Value,
                RegistrationRegistryRoutes[RowIndex].Value) <> 0 then
        Errors := Errors + ' changed ' +
          RegistrationRegistryRoutes[RowIndex].Name
      else if not RegDeleteValue(
             RegistrationRegistryRoot(
               RegistrationRegistryRoutes[RowIndex].View),
             RegistrationRegistryRoutes[RowIndex].SubKey,
             RegistrationRegistryRoutes[RowIndex].Name) then
        Errors := Errors + ' delete ' +
          RegistrationRegistryRoutes[RowIndex].Name;
    end;
    for RowIndex := 0 to GetArrayLength(RegistrationRegistryRoutes) - 1 do
      if RegQueryStringValue(
           RegistrationRegistryRoot(
             RegistrationRegistryRoutes[RowIndex].View),
           RegistrationRegistryRoutes[RowIndex].SubKey,
           RegistrationRegistryRoutes[RowIndex].Name, Value) then
        Errors := Errors + ' verify ' +
          RegistrationRegistryRoutes[RowIndex].Name;
    Result := Errors = '';
    if not Result then
      Detail := Trim(Errors);
  except
    Detail := GetExceptionMessage;
  end;
end;

procedure RegisterAgentOrAbort();
var
  ResultCode, PriorLogonType: Integer;
  TaskExisted, TaskRecognized, ShortcutExisted, ShortcutRecognized,
    TaskRestored, ShortcutRestored, AppDataExisted, AppDataRestored,
    RegistryRestored, DeploymentMutexHeld, MutationMutexHeld,
    ChildSucceeded: Boolean;
  DeploymentMutexHandle, MutationMutexHandle,
    ReleaseChildEventHandle: THandle;
  TaskXml, PriorUserId, PriorTaskSecurity, InspectDetail, TaskRestoreDetail,
    ShortcutRestoreDetail, AppDataRestoreDetail, RegistryRestoreDetail,
    ErrorDetail, ShortcutPath,
    CommonShortcutPath, ShortcutBackupPath, ShortcutHash,
    AppDataPath, AppDataBackupPath, SnapshotDetail, MutexDetail,
    ReleaseChildToken, ChildReceipt: String;
begin
  DeploymentMutexHandle := 0;
  MutationMutexHandle := 0;
  ReleaseChildEventHandle := 0;
  DeploymentMutexHeld := False;
  MutationMutexHeld := False;
  if not AcquireNamedMutex(
       'Local\MichStartupMaster.ReleaseDeployment', 600000,
       DeploymentMutexHandle, MutexDetail) then
    RaiseException('Could not enter the release deployment gate: ' +
      MutexDetail);
  DeploymentMutexHeld := True;
  try
    if not AcquireNamedMutex(
         'Local\MichStartupMaster.ManagedStartupMutation', 120000,
         MutationMutexHandle, MutexDetail) then
      RaiseException('Could not drain the managed startup mutation gate: ' +
        MutexDetail);
    MutationMutexHeld := True;

  Log('Validated transaction: {#ValidatedTransactionId}; publish manifest SHA-256: {#ValidatedPublishManifestSha256}');
  VerifyInstalledPayload();
  if CompareText(GetSHA256OfFile(
       ExpandConstant('{app}\{#MyAppExeName}')),
       '{#ValidatedExecutableSha256}') <> 0 then
    RaiseException('Installed executable does not match the validated staged build.');

  AppDataPath := ExpandConstant('{localappdata}\MichStartupMaster');
  AppDataBackupPath := ExpandConstant('{tmp}\MichStartupMaster-prior-app-data');
  if FileExists(AppDataPath) then
    RaiseException(
      'The exact MichStartupMaster AppData path is a file, not an owned directory. It was preserved and no registration was attempted.');
  AppDataExisted := DirExists(AppDataPath);
  if AppDataExisted and
     (not CopyDirectorySnapshot(
       AppDataPath, AppDataBackupPath, SnapshotDetail)) then
    RaiseException(
      'Could not preserve the exact prior MichStartupMaster AppData tree. No registration was attempted. ' +
      SnapshotDetail);
  SnapshotDetail := '';
  if not CaptureOwnedRegistrationRegistry(SnapshotDetail) then
    RaiseException(
      'Could not preserve owned per-user Run/RunOnce routes. No registration was attempted. ' +
      SnapshotDetail);

  InspectDetail := '';
  if not ReadReservedAgentTask(
    TaskExisted, TaskRecognized, TaskXml, PriorUserId,
    PriorTaskSecurity,
    PriorLogonType, InspectDetail) then
    RaiseException(
      'Could not safely inspect the reserved startup-agent task. No task was changed. ' +
      InspectDetail);
  if TaskExisted and (not TaskRecognized) then
    RaiseException('The reserved startup-agent task is owned by another command. It was preserved and setup was stopped.');

  ShortcutPath := ExpandConstant(
    '{userstartup}\Mich Startup Master Agent.lnk');
  CommonShortcutPath := ExpandConstant(
    '{commonstartup}\Mich Startup Master Agent.lnk');
  if FileExists(CommonShortcutPath) then
    RaiseException(
      'A reserved all-users Startup shortcut exists. This per-user installer preserved it and stopped before registration.');
  InspectDetail := '';
  if not ReadReservedAgentShortcut(
    ShortcutExisted, ShortcutRecognized, InspectDetail) then
    RaiseException(
      'Could not safely inspect the reserved Startup shortcut. No startup route was changed. ' +
      InspectDetail);
  if ShortcutExisted and (not ShortcutRecognized) then
    RaiseException(
      'The reserved Startup shortcut is owned by another command. It was preserved and setup was stopped.');
  ShortcutBackupPath := ExpandConstant(
    '{tmp}\MichStartupMaster-prior-agent-shortcut.lnk');
  ShortcutHash := '';
  if ShortcutExisted then
  begin
    ShortcutHash := GetSHA256OfFile(ShortcutPath);
    if (ShortcutHash = '') or
       (not CopyFile(ShortcutPath, ShortcutBackupPath, True)) then
      RaiseException(
        'Could not preserve the prior Startup shortcut bytes. No startup route was changed.');
  end;

  ReleaseChildToken := NewReleaseChildToken();
  if not CreateReleaseChildProofEvent(
       ReleaseChildToken, ReleaseChildEventHandle, MutexDetail) then
    RaiseException('Could not create the authenticated release-child proof: ' +
      MutexDetail);
  ReleaseNamedMutexOwnership(
    'Local\MichStartupMaster.ManagedStartupMutation',
    MutationMutexHandle);
  MutationMutexHeld := False;
  ChildSucceeded := RunAuthenticatedReleaseChild(
    '--register-agent', ReleaseChildToken, 120000,
    ResultCode, ChildReceipt, ErrorDetail);
  if not ReacquireNamedMutex(
       'Local\MichStartupMaster.ManagedStartupMutation',
       MutationMutexHandle, 120000, MutexDetail) then
    RaiseException(
      'Registration child returned, but the managed startup mutation gate could not be reacquired; rollback was not attempted against an uncoordinated state: ' +
      MutexDetail + '. Child: ' + ErrorDetail);
  MutationMutexHeld := True;
  CloseReleaseChildProofEvent(ReleaseChildEventHandle);
  if ChildSucceeded then
  begin
    Log('Bounded authenticated startup-agent registration and verification passed. ' +
      ChildReceipt);
    Exit;
  end;

  { Every external mutation surface is rolled back independently. }
  TaskRestoreDetail := '';
  TaskRestored := RestorePriorAgentTask(
    TaskExisted, TaskXml, PriorUserId, PriorTaskSecurity,
    PriorLogonType,
    TaskRestoreDetail);
  ShortcutRestoreDetail := '';
  ShortcutRestored := RestorePriorAgentShortcut(
    ShortcutExisted, ShortcutBackupPath, ShortcutHash,
    ShortcutRestoreDetail);
  RegistryRestoreDetail := '';
  RegistryRestored := RestoreOwnedRegistrationRegistry(
    RegistryRestoreDetail);
  AppDataRestoreDetail := '';
  AppDataRestored := RestoreAppDataSnapshot(
    AppDataPath, AppDataBackupPath, AppDataExisted,
    AppDataRestoreDetail);
  if TaskRestored and ShortcutRestored and
     RegistryRestored and AppDataRestored then
    RaiseException(
      ErrorDetail + '. The prior task, reserved Startup shortcut, owned registry routes, and AppData were restored.')
  else
    RaiseException(
      ErrorDetail + '. Rollback was incomplete. Task: ' +
      TaskRestoreDetail + '; shortcut: ' + ShortcutRestoreDetail +
      '; registry: ' + RegistryRestoreDetail +
      '; AppData: ' + AppDataRestoreDetail);
  finally
    CloseReleaseChildProofEvent(ReleaseChildEventHandle);
    ReleaseAndCloseNamedMutex(
      'Local\MichStartupMaster.ManagedStartupMutation',
      MutationMutexHandle, MutationMutexHeld);
    ReleaseAndCloseNamedMutex(
      'Local\MichStartupMaster.ReleaseDeployment',
      DeploymentMutexHandle, DeploymentMutexHeld);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    RegisterAgentOrAbort();
end;

function IsOwnedManualShortcut(const ShortcutPath: String): Boolean;
var
  Shell, Shortcut: Variant;
  Arguments: String;
begin
  Result := False;
  if not FileExists(ShortcutPath) then
    Exit;
  try
    Shell := CreateOleObject('WScript.Shell');
    Shortcut := Shell.CreateShortcut(ShortcutPath);
    Arguments := Shortcut.Arguments;
    Result :=
      PathsEqual(Shortcut.TargetPath,
        ExpandConstant('{app}\{#MyAppExeName}')) and
      (Trim(Arguments) = '');
  except
    Result := False;
  end;
end;

function ReadShortcutOwnership(
  const ShortcutPath: String; const AgentShortcut: Boolean;
  var ShortcutExists, Owned: Boolean; var Detail: String): Boolean;
var
  Shell, Shortcut: Variant;
  Arguments: String;
begin
  Result := False;
  ShortcutExists := FileExists(ShortcutPath);
  Owned := False;
  Detail := '';
  if not ShortcutExists then
  begin
    Result := True;
    Exit;
  end;
  try
    Shell := CreateOleObject('WScript.Shell');
    Shortcut := Shell.CreateShortcut(ShortcutPath);
    Arguments := Shortcut.Arguments;
    if AgentShortcut then
      Owned := IsOwnedAgentLaunch(
        Shortcut.TargetPath, Shortcut.Arguments)
    else
      Owned :=
        PathsEqual(Shortcut.TargetPath,
          ExpandConstant('{app}\{#MyAppExeName}')) and
        (Trim(Arguments) = '');
    Result := True;
  except
    Detail := GetExceptionMessage;
  end;
end;

function RestoreShortcutBytes(
  const ShortcutPath, BackupPath, ExpectedHash: String;
  const AgentShortcut: Boolean; var Detail: String): Boolean;
begin
  Result := False;
  Detail := '';
  if not FileExists(BackupPath) then
  begin
    Detail := 'the transaction backup is missing';
    Exit;
  end;
  if FileExists(ShortcutPath) and
     (CompareText(GetSHA256OfFile(ShortcutPath), ExpectedHash) <> 0) then
  begin
    if (AgentShortcut and (not IsOwnedAgentShortcut(ShortcutPath))) or
       ((not AgentShortcut) and (not IsOwnedManualShortcut(ShortcutPath))) then
    begin
      Detail := 'an unrelated shortcut now occupies the reserved path';
      Exit;
    end;
  end;
  if not CopyFile(BackupPath, ShortcutPath, False) then
  begin
    Detail := 'could not restore the shortcut bytes';
    Exit;
  end;
  Result := CompareText(
    GetSHA256OfFile(ShortcutPath), ExpectedHash) = 0;
  if not Result then
    Detail := 'restored shortcut hash mismatch';
end;

procedure AppendFailure(var Failures: String; const Value: String);
begin
  if Value = '' then
    Exit;
  if Failures <> '' then
    Failures := Failures + ' | ';
  Failures := Failures + Value;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  TaskExists, TaskRecognized, TaskOwned,
    UserShortcutExists, UserShortcutOwned,
    CommonShortcutExists, CommonShortcutOwned,
    MenuShortcutExists, MenuShortcutOwned,
    TaskRestored, UserRestored, CommonRestored, MenuRestored,
    RegistryDeleted, RegistryRestored,
    DeploymentMutexHeld, MutationMutexHeld: Boolean;
  TaskObject, FolderObject: Variant;
  PriorLogonType: Integer;
  DeploymentMutexHandle, MutationMutexHandle: THandle;
  Detail, InspectDetail, CleanupFailures, RollbackFailures,
    TaskXml, PriorUserId, PriorTaskSecurity,
    UserShortcutPath, CommonShortcutPath, MenuShortcutPath,
    UserBackupPath, CommonBackupPath, MenuBackupPath,
    UserHash, CommonHash, MenuHash,
    TaskRestoreDetail, UserRestoreDetail, CommonRestoreDetail,
    MenuRestoreDetail, RegistryDeleteDetail, RegistryRestoreDetail,
    MutexDetail: String;
begin
  if CurUninstallStep <> usUninstall then
    Exit;

  DeploymentMutexHandle := 0;
  MutationMutexHandle := 0;
  DeploymentMutexHeld := False;
  MutationMutexHeld := False;
  if not AcquireNamedMutex(
       'Local\MichStartupMaster.ReleaseDeployment', 600000,
       DeploymentMutexHandle, MutexDetail) then
    RaiseException('Uninstall could not enter the release deployment gate: ' +
      MutexDetail);
  DeploymentMutexHeld := True;
  try
    if not AcquireNamedMutex(
         'Local\MichStartupMaster.ManagedStartupMutation', 120000,
         MutationMutexHandle, MutexDetail) then
      RaiseException('Uninstall could not enter the managed startup mutation gate: ' +
        MutexDetail);
    MutationMutexHeld := True;

  { Preflight every reserved route and capture byte/XML/security snapshots before
    deleting anything. Ownership mismatches are preserved; inspection failures
    stop the uninstall without mutation. }
  Detail := '';
  if not ReadReservedAgentTask(
    TaskExists, TaskRecognized, TaskXml, PriorUserId,
    PriorTaskSecurity, PriorLogonType, Detail) then
    RaiseException(
      'Uninstall stopped because the startup-agent task could not be inspected: ' +
      Detail);
  TaskOwned := False;
  if TaskExists then
  begin
    InspectDetail := '';
    if not TryFindReservedAgentTask(
      TaskExists, TaskObject, FolderObject, InspectDetail) then
      RaiseException(
        'Uninstall stopped because task ownership could not be reopened: ' +
        InspectDetail);
    TaskOwned := TaskHasExactOwnedAction(TaskObject);
    if TaskOwned and
       ((not TaskRecognized) or
        (not IsCanonicalOwnedAgentTaskObject(TaskObject))) then
      RaiseException(
        'Uninstall stopped because the owned reserved task is non-canonical. It was preserved for explicit repair.');
    if not TaskOwned then
      Log('Reserved task ownership mismatch; it will be preserved.');
  end;

  UserShortcutPath :=
    ExpandConstant('{userstartup}\Mich Startup Master Agent.lnk');
  CommonShortcutPath :=
    ExpandConstant('{commonstartup}\Mich Startup Master Agent.lnk');
  MenuShortcutPath :=
    ExpandConstant('{autoprograms}\{#MyAppName}.lnk');
  if not ReadShortcutOwnership(
       UserShortcutPath, True, UserShortcutExists,
       UserShortcutOwned, Detail) then
    RaiseException('Uninstall could not inspect the user Startup shortcut: ' +
      Detail);
  if not ReadShortcutOwnership(
       CommonShortcutPath, True, CommonShortcutExists,
       CommonShortcutOwned, Detail) then
    RaiseException('Uninstall could not inspect the common Startup shortcut: ' +
      Detail);
  if not ReadShortcutOwnership(
       MenuShortcutPath, False, MenuShortcutExists,
       MenuShortcutOwned, Detail) then
    RaiseException('Uninstall could not inspect the Start-menu shortcut: ' +
      Detail);
  if UserShortcutExists and (not UserShortcutOwned) then
    Log('User Startup shortcut ownership mismatch; it will be preserved.');
  if CommonShortcutExists and (not CommonShortcutOwned) then
    Log('Common Startup shortcut ownership mismatch; it will be preserved.');
  if MenuShortcutExists and (not MenuShortcutOwned) then
    Log('Start-menu shortcut ownership mismatch; it will be preserved.');

  UserBackupPath := ExpandConstant(
    '{tmp}\MichStartupMaster-uninstall-user-startup.lnk');
  CommonBackupPath := ExpandConstant(
    '{tmp}\MichStartupMaster-uninstall-common-startup.lnk');
  MenuBackupPath := ExpandConstant(
    '{tmp}\MichStartupMaster-uninstall-start-menu.lnk');
  if UserShortcutOwned then
  begin
    UserHash := GetSHA256OfFile(UserShortcutPath);
    if (UserHash = '') or
       (not CopyFile(UserShortcutPath, UserBackupPath, True)) then
      RaiseException('Uninstall could not snapshot the owned user Startup shortcut. No cleanup was attempted.');
  end;
  if CommonShortcutOwned then
  begin
    CommonHash := GetSHA256OfFile(CommonShortcutPath);
    if (CommonHash = '') or
       (not CopyFile(CommonShortcutPath, CommonBackupPath, True)) then
      RaiseException('Uninstall could not snapshot the owned common Startup shortcut. No cleanup was attempted.');
  end;
  if MenuShortcutOwned then
  begin
    MenuHash := GetSHA256OfFile(MenuShortcutPath);
    if (MenuHash = '') or
       (not CopyFile(MenuShortcutPath, MenuBackupPath, True)) then
      RaiseException('Uninstall could not snapshot the owned Start-menu shortcut. No cleanup was attempted.');
  end;
  Detail := '';
  if not CaptureOwnedRegistrationRegistry(Detail) then
    RaiseException(
      'Uninstall could not snapshot owned per-user Run/RunOnce routes. No cleanup was attempted. ' +
      Detail);

  CleanupFailures := '';
  if TaskOwned then
  begin
    try
      FolderObject.DeleteTask('MichStartupMasterApp', 0);
    except
      AppendFailure(CleanupFailures,
        'delete task: ' + GetExceptionMessage);
    end;
    Detail := '';
    if not TryFindReservedAgentTask(
      TaskExists, TaskObject, FolderObject, Detail) then
      AppendFailure(CleanupFailures,
        'verify task deletion: ' + Detail)
    else if TaskExists then
      AppendFailure(CleanupFailures,
        'verify task deletion: task remains');
  end;
  if UserShortcutOwned then
  begin
    if not DeleteFile(UserShortcutPath) then
      AppendFailure(CleanupFailures,
        'delete user Startup shortcut');
    if FileExists(UserShortcutPath) then
      AppendFailure(CleanupFailures,
        'verify user Startup shortcut deletion');
  end;
  if CommonShortcutOwned then
  begin
    if not DeleteFile(CommonShortcutPath) then
      AppendFailure(CleanupFailures,
        'delete common Startup shortcut');
    if FileExists(CommonShortcutPath) then
      AppendFailure(CleanupFailures,
        'verify common Startup shortcut deletion');
  end;
  if MenuShortcutOwned then
  begin
    if not DeleteFile(MenuShortcutPath) then
      AppendFailure(CleanupFailures,
        'delete Start-menu shortcut');
    if FileExists(MenuShortcutPath) then
      AppendFailure(CleanupFailures,
        'verify Start-menu shortcut deletion');
  end;
  RegistryDeleteDetail := '';
  RegistryDeleted := DeleteCapturedOwnedRegistrationRegistry(
    RegistryDeleteDetail);
  if not RegistryDeleted then
    AppendFailure(CleanupFailures,
      'delete owned Run/RunOnce routes: ' + RegistryDeleteDetail);

  if CleanupFailures <> '' then
  begin
    RollbackFailures := '';
    if TaskOwned then
    begin
      TaskRestored := RestorePriorAgentTask(
        True, TaskXml, PriorUserId, PriorTaskSecurity,
        PriorLogonType, TaskRestoreDetail);
      if not TaskRestored then
        AppendFailure(RollbackFailures,
          'restore task: ' + TaskRestoreDetail);
    end;
    if UserShortcutOwned then
    begin
      UserRestored := RestoreShortcutBytes(
        UserShortcutPath, UserBackupPath, UserHash,
        True, UserRestoreDetail);
      if not UserRestored then
        AppendFailure(RollbackFailures,
          'restore user Startup shortcut: ' + UserRestoreDetail);
    end;
    if CommonShortcutOwned then
    begin
      CommonRestored := RestoreShortcutBytes(
        CommonShortcutPath, CommonBackupPath, CommonHash,
        True, CommonRestoreDetail);
      if not CommonRestored then
        AppendFailure(RollbackFailures,
          'restore common Startup shortcut: ' + CommonRestoreDetail);
    end;
    if MenuShortcutOwned then
    begin
      MenuRestored := RestoreShortcutBytes(
        MenuShortcutPath, MenuBackupPath, MenuHash,
        False, MenuRestoreDetail);
      if not MenuRestored then
        AppendFailure(RollbackFailures,
          'restore Start-menu shortcut: ' + MenuRestoreDetail);
    end;
    RegistryRestoreDetail := '';
    RegistryRestored := RestoreOwnedRegistrationRegistry(
      RegistryRestoreDetail);
    if not RegistryRestored then
      AppendFailure(RollbackFailures,
        'restore owned Run/RunOnce routes: ' + RegistryRestoreDetail);
    if RollbackFailures = '' then
      RaiseException(
        'Uninstall startup cleanup failed and every changed route was restored: ' +
        CleanupFailures)
    else
      RaiseException(
        'Uninstall startup cleanup failed and rollback was incomplete. Cleanup: ' +
        CleanupFailures + '; rollback: ' + RollbackFailures);
  end;

  { Persistent recovery/state data under
    %LOCALAPPDATA%\MichStartupMaster is deliberately preserved. }
  finally
    ReleaseAndCloseNamedMutex(
      'Local\MichStartupMaster.ManagedStartupMutation',
      MutationMutexHandle, MutationMutexHeld);
    ReleaseAndCloseNamedMutex(
      'Local\MichStartupMaster.ReleaseDeployment',
      DeploymentMutexHandle, DeploymentMutexHeld);
  end;
end;
