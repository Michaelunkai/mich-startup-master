# Spec 02 — Scan surfaces & ID formats (from C# StartupService)

## Registry view rules
- Machine (HKLM): enumerate BOTH Registry64 + Registry32 views (on 64-bit OS), view label suffix ` [Registry32]`/` [Registry64]` appended to Location; RegistryView field set.
- HKCU / HKU: SINGLE view only (WOW64-shared; enumerating both duplicates physical values).
- `NativeRegistryViewLabel()` used for non-redirected SYSTEM keys (likely "Registry64" — verify in source ~7300).

## ScanAll pipeline (ORDER MATTERS)
1. AddCommonRegistryStartup — Run keys etc. (main variant, richer: capability preflight)
2. AddRegistryStartupExtensions — extended autoruns (winlogon notify, IFEO, KnownDLLs, network providers, winsock, print monitors, media codecs)
3. AddLoadedUserRegistryStartup — other users' hives (HKU\S-1-5-21-*) + service accounts S-1-5-18/19/20
4. AddAllStartupFolders — user/common/all-profiles startup folders
5. AddIsolatedProvider("startup-command-main") → source "Startup Command" (Win32_StartupCommand via own-exe child, 10s timeout, retry 2x)
6. AddLogonTasks — scheduled tasks (COM walk as isolated provider "task-main" 10s?; check main call site)
7. AddAutoServices (Windows Service incl. trigger-start; main variant)
8. AddAutoDrivers (System Driver)
9. AddSystemBootEntries — Winlogon, BootExecute, LSA, AppInit, Active Setup
10. AddExtendedAutorunSources — the extended set incl. IE addons, Explorer shell extensions
11. AddGroupPolicyScripts
12. AddIsolatedProvider("wmi-main") → "WMI Event Consumer" (root\subscription, 10s)
13. AddPackagedStartupTasks — UWP manifests + AppModel State authority
14. ApplyStartupApproved(items, ReadStartupApproved()) — Explorer approval overlay sets Enabled for Run/folder rows
15. RetireDisabledRecordsMatchingLiveRoutes + append disabled store rows (Id `disabled|...`)
16. AddLegacyV2Items (Id `legacy|...`, Source "Legacy v2")
17. HydrateHumanNames — AppName from installed products (InstalledProductPrefix), payload candidates (wscript wrappers → real exe, e.g. OpenSpeedy), script identity `script:` prefix, GameAutoInstall grouping
18. ApplyTruthEvidence — VerifiedState per source (see below)
19. ApplyRuntimeEvidence — running-process evidence
20. Dedupe → sort Enabled(0/1) then Source then Name

## Registry surfaces (exact keys, source names)
Run keys (both HKLM views + HKCU + loaded hives):
- `Software\Microsoft\Windows\CurrentVersion\Run` → "Registry Run"
- `...\RunOnce` → "Registry RunOnce"
- `...\RunServices` → "Registry RunServices"
- `...\RunServicesOnce` → (SourceForRegistryPath — check; likely "Registry RunServices")
- `...\Policies\Explorer\Run` → "Policy Run"
- `SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows` values `Load`,`Run` → "Legacy Windows Run"
- `SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnceEx\<child>\<values>` (skip Title/Flags/Depend) → "Registry RunOnceEx", Name = valueName or childName if default
- `SOFTWARE\Microsoft\Windows\CurrentVersion\ShellServiceObjectDelayLoad` + `...\Explorer\SharedTaskScheduler` → "Explorer Startup Extension"
- Winlogon `SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon` values: Machine [Shell,Userinit,Taskman,AppSetup,System], User/HKU [Shell,Userinit,Taskman] → "Winlogon Autostart", split unquoted commas → one row per component, Name `Value #n` if >1
- AppInit: `...Windows` AppInit_DLLs + LoadAppInit_DLLs dword → single row "AppInit DLLs", Enabled = LoadAppInit_DLLs != 0
- Active Setup: `SOFTWARE\Microsoft\Active Setup\Installed Components\<guid>` StubPath → "Active Setup"
- HKCU `Environment` value `UserInitMprLogonScript` → "User Logon Script"
- AppCertDlls: HKLM `SYSTEM\CurrentControlSet\Control\Session Manager\AppCertDlls` all values → "AppCert DLLs"
- Advanced (audit): Session Manager BootExecute/SetupExecute/Execute/S0InitialCommand → "Boot Execute" (Id `advanced|b64(HKLM)|b64(path)|b64(value)|index`); Lsa Authentication/Notification/Security Packages + Lsa\OSConfig\Security Packages → "LSA Startup Package"
- Winlogon Notify subkeys DllName → "Winlogon Notification"
- IFEO Debugger/VerifierDlls + SilentProcessExit MonitorProcess (subkey-scoped) → "Image Hijack"
- Session Manager\KnownDLLs + KnownDLLs32 (exclude DllDirectory) → "Known DLL"
- NetworkProvider\Order ProviderOrder → each `HKLM\SYSTEM\CurrentControlSet\Services\<p>\NetworkProvider` ProviderPath → "Network Provider"
- Winsock catalogs (32+64) via WSAEnumProtocols → "Winsock Provider" (ReadOnly; GetWinsockProviderPath)
- Print Monitors `SYSTEM\CurrentControlSet\Control\Print\Monitors\<m>` Driver → "Print Monitor"
- Media codecs `SOFTWARE\Microsoft\Windows NT\CurrentVersion\Drivers32` all values → "Media Codec"
- Group Policy scripts: HKLM `...Group Policy\Scripts\Startup`, HKLM `...Group Policy\State\Machine\Scripts\Startup`, HKCU `...Scripts\Logon`, HKCU `...State\User\Scripts\Logon` — recursive ≤8 deep, values Script+Parameters → "Group Policy Script"
- WMI: root\subscription __FilterToConsumerBinding → CommandLineEventConsumer/ActiveScriptEventConsumer → "WMI Event Consumer"
- Explorer shell extensions + IE addons: AddExplorerShellExtensions / AddInternetExplorerAddons (details later in source)

## ID formats (byte-exact for dedupe + stores)
- reg: `reg|{scope}|{b64(rootName)}|{b64(subKey)}|{b64(valueName)}[|viewLabel]`
- folder: `folder|{scope}|{fileFullPath}`
- task: `task|{normalizedPath}` (path `\A\B`, forward→back slashes, no trailing)
- winlogon: `winlogon|{scope}|{b64(root)}|{b64(sub)}|{b64(value)}|{index}|{view}`
- active: `active|{scope}|{b64(root)}|{b64(sub)}[|view]`
- appinit: `appinit|{b64(path)}|{view}`
- advanced: `advanced|{b64(root)}|{b64(sub)}|{b64(value)}|{index}` (audit) ; protection key: `advanced|{b64(root)}|{b64(sub)}|{b64(value)}|{index}|{view}`
- service: `service|{b64(name)}` / driver: `driver|{b64(name)}`; trigger-start source "Service (Trigger Start)"
- gpscript: `gpscript|{b64(root)}|{b64(sub)}`
- wmisub: `wmisub|{b64(bindingPath)}`
- packaged: `packaged|{b64(scope)}|{b64(manifestPath)}|{b64(registrationName)}`; registrationName = family!taskId
- disabled rows: `disabled|...` (see stores spec), legacy: `legacy|...`
- error rows: `error|...` (ErrorItem: Status "Read failed", Command=exception message; smoke counts these)

## Scheduled tasks detail
- COM: Schedule.Service → GetFolder("\") recursive; GetTasks(1)=TASK_ENUM_HIDDEN; budgets: 2048 folders / 16384 tasks / depth 64.
- Trigger types: startup = 8(Boot)/9(Logon)/11(SessionState); names Event/Time/Daily/Weekly/Monthly/MonthlyDOW/Idle/Registration/Boot/Logon/SessionState/TypeN. Unknown type (read failed) counts as startup-candidate.
- File audit: C:\Windows\System32\Tasks recursive (skip reparse points), XML parse (DTD prohibited, 16MiB cap), XML trigger names map: BootTrigger=8 LogonTrigger=9 EventTrigger=0 TimeTrigger=1 IdleTrigger=6 RegistrationTrigger=7 SessionStateChangeTrigger=11 else -1. Task Enabled from Task\Settings\Enabled (default true). StateLabel "Definition".
- Row: every task inventoried (not only boot). Source = "Scheduled Task" if startup trigger else "Scheduled Task (Any Trigger)". Enabled = TaskEnabled (task-level flag ONLY). Status string: `{Enabled|Disabled} {kinds}[ startup task| scheduled task (non-boot trigger)| scheduled task (no trigger; starts only when another program or service runs it)][ (all triggers disabled; runs only when started on demand)][ (boot/logon trigger disabled; still runs on {liveKinds})]; state={StateLabel}; taskEnabled={b}; modeEditable={b}; triggers={name}(index=i;enabled=b[;id=..];delay=..[;userId=..]) || ...; actions=Exec(index=;path=;arguments=;workingDirectory=)|ComHandler(index=;classId=;data=)...`
- ExternalAuthority = "Windows task library" ONLY for non-startup tasks under \Microsoft\.
- Command = actions joined " || ": Exec → `WinArg(normalizedPath) + " " + args` trimmed; ComHandler → `COM handler {classId}`; else `Task action {kind|type N}`.
- NormalizeScheduledTaskExecutable strips outer quote pairs. TaskExecutablePathsEqual: expand env, /→\, GetFullPath, case-insens, trim trailing \.
- FriendlyScheduledTaskName(taskPath) — friendly name (read later).
- Truth readback: schedule COM GetTask(loc), definition.Actions.Count==1 && Type==0 else "Drifted"/multi-action → Unknown path; compare exact execute+args vs scanned; probe files only on safe system volumes; managed intent compare via QuietLaunchPlanner.NormalizePersistent; Enabled = task.Enabled && any-enabled-trigger.

## Services / drivers
- OpenSCManager(ENUMERATE_SERVICE|...), EnumServicesStatusEx level 0, SERVICE_STATE_ALL; Win32=0x30 services, 0x0B drivers.
- StartType ≤2 (boot/system/auto) → row Enabled=true, CanDisable=IsElevated(). Status `SCM start={n} state={n}`.
- StartType 3 (manual) + TriggerInfo key exists + not driver → "Service (Trigger Start)" row.
- StartType 4 disabled → skip.
- Command = BinaryPathName. Location = `HKLM\SYSTEM\CurrentControlSet\Services\{name}`.

## Packaged (UWP) startup tasks
- Repos: HKCU + HKLM(+HKU per-user) `Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages` (HKLM: `SOFTWARE\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages`); PackageRootFolder/Path → AppxManifest.xml (open directly, no existence probe).
- Manifest: Extension Category endswith .startupTask; StartupTask TaskId/DisplayName/Enabled; command = Executable (relative→manifest dir) else `Package activation {family!taskId}[ (entryPoint)]`.
- State authority: `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData\{family}\{taskId}` value `State` (DWORD): 0/1 disabled, 2 enabled, 3/4 policy → CanDisable=false + ExternalAuthority "Windows packaged startup policy". Toggle writes State 2/1 (same as Settings/TaskManager).
- humanName resolution: taskDisplay (if not ms-resource:) → manifest DisplayName → repo DisplayName → packageName → taskId.

## StartupApproved (Explorer overlay)
- ReadStartupApproved + ApplyStartupApproved — key: `Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run[Once]` (+ StartupFolder) HKCU/HKLM; binary value: first byte 02/06 = enabled, 03 = disabled (verify exact bytes in source); maps to Run value name / folder file name; sets Enabled on canonical row; overlay fields ApprovalRoot/Path/Name/Data/RegistryView kept on the canonical row; separate "Startup Approval"/"Stale Startup Metadata" rows for orphan approvals.

## Truth evidence (ApplyTruthEvidence)
- All: VerifiedState="Unknown" default; PresentationState "Needs verification"/"Unsupported" (PopupLabel N/A).
- error| → provider failed; disabled| → "Disabled" verified; legacy| → Unknown retained.
- "Startup Command" → Unknown (projection); "Startup Approval"/"Stale Startup Metadata" → Unknown metadata.
- Packaged → Enabled/Disabled per State authority.
- task| → full readback verify (above) → Enabled/Disabled/Drifted/Unknown.
- IsDirectConfiguredStartupSource (Registry Run/RunOnce/RunOnceEx/RunServices/Policy Run/Legacy Windows Run/User Logon Script/Startup Folder/Windows Service/System Driver/Winlogon Autostart/Boot Execute/LSA Startup Package/Group Policy Script) → Enabled/Disabled verified fresh read.
- Winlogon Notification → legacy-ignored note; AppInit → conditional note; other extensions → conditional note.

## Dedupe (Dedupe())
- Win32_StartupCommand mirror collapses into native Registry Run / Startup Folder row when same physical registration (PATH-resolved, same command incl. case-sensitive args); mirror without native twin stays visible; different scope doesn't collapse. (InventorySelfTest documents exact cases.)
- NormBootKey: lowercase, trim, strip quotes, remove ALL whitespace.

## Audit (AuditBootCoverage)
- BOOT_AUDIT line: `BOOT_AUDIT independent=true independence_scope=authoritative-enumerators sources={raw} shown={shown} gaps={gaps} errors={errors} surfaces={Source_Group:count csv, spaces→_}`
- Independent walks: registry (audit variants), folders, task-audit (file XML walk 30s), service-audit, driver-audit (native SCM), advanced, extended, GP scripts, wmi-audit, packaged.
- Gap lines: `{Source} | {Name|?} | {Command}`; AUDIT_ERROR prefix for provider errors. RunOnce volatile re-check allowed. Gap coverage match: same Id, or (no stable Id && name+location match && command match len>8), Startup Command mirror rules, tray-payload decode compare.

## Provider worker protocol
- Parent spawns own exe: `--provider-worker <kind>`; stdout: `MSM_PROVIDER_V1` header + `R\t{b64 field}\t...` 23 fields (16 legacy) = Id,Name,AppName,Source,Scope,Command,Location,Enabled,CanDisable,IsManaged,Status,ApprovalRoot,ApprovalPath,ApprovalName,RegistryView,ApprovalRegistryView,MutationCapability,MutationReason,RequiresExpertConfirmation,RequiresElevation,RequiresReboot,ExternalAuthority. Retry once at 2x timeout; zero rows for task/service/driver = error row "Provider returned zero rows; coverage is not assumed".

## ScanAllWithin
- Budget wrapper: thread + join + stall counter (≥2 stalled scans → TimeoutException without starting new).

## Application identity (HydrateHumanNames) — partial
- ApplicationPayloadCandidates: wscript wrappers → parse script text for exact exe paths (fallback lists, e.g. OpenSpeedy F:\study\... F:\backup\...); executable arg to script host → that exe.
- TryApplyScriptApplicationIdentity: GameAutoInstall scripts → AppName "Game Auto Install" identity grouping; arbitrary scripts → identity `script:...`, AppName from task name.
- SetPortableApplicationIdentity + CorrelateFallbackApplicationIdentities: fallback wrapper and direct route → same ApplicationIdentity.
- Installed products (InstalledProductPrefix ~5824): AppName from installed-product path registration (uninstall keys) — ApplicationIdentity/ApplicationRoot/ApplicationIdentityReason.
- FriendlyNameForPayload: file-description/name of exe.
