# Spec 03 — Mutations (Disable/Enable), agent registration, store record formats

## DisableCore dispatch by Id prefix
- `legacy|` → EnabledStartupService.Remove + MarkV2MigratedChecked(\MichStartupMaster\<name>)
- ApprovalRoot set (overlay) → SetStartupApproval(item,false) + EnabledStartupService.Remove
- `packaged|` → SetPackagedStartupTaskState(item,false) (State=1) + enabled-store Remove
- `winlogon|` → DisableWinlogonComponent: read before-state, BuildWinlogonRemovalPlan (remove exact index), DisabledStoreService.Add("winlogon", ..., EncodeAdvancedDisabledState(record)); WriteRegistryValueStateExact(root,sub,val,before,after); rollback removes store row
- `appinit|` → DisableAppInit: plan toggles LoadAppInit_DLLs→0, guard=AppInit_DLLs bytes must be unchanged
- `advanced|` → DisableAdvancedMultiStringComponent (indexed REG_MULTI_SZ removal; view from item.RegistryView else native)
- `extension|` → DisableExtensionRoute ONLY if ExtensionDisableSurface(kind,root,sub,val,source): media-codec@HKLM Drivers32; url-search-hook@HKCU UrlSearchHooks; ie-toolbar/ie-search-hook (HKLM/HKU [S-1-5-21-…\]SOFTWARE\[WOW6432Node\]Microsoft\Internet Explorer\{Toolbar|URLSearchHooks}); explorer-shell-hook (@…\Explorer\ShellExecuteHooks). Record routeType=extension OriginalIndex=-1 (RequiresExpertConfirmation=!currentUser for these).
- `reg|` → DisableRegistry: read value+kind, DisabledStoreService.Add("reg", Name, scope, Command, Location, metadata = `root.Name\tsubKey\tname\tkind\tsource\tB64(payload)\tviewLabel`), then DeleteValue; rollback removes row. (legacy fallback for old 3-part ids: HKCU/HKLM Run)
- `active|` → DisableActiveSetup: delete StubPath value; metadata `root\tsubKey\tStubPath\tkind\tB64(payload)\tview`
- `folder|` → move file to `%LOCALAPPDATA%\MichStartupMaster\DisabledStartupFolderItems\<name>.<ticks>.disabled`; metadata=original path (Status=source path), Command becomes dest path
- `task|` → `schtasks.exe /Change /TN "<Location>" /Disable` (RunChecked) + ProtectedQuietService.UnprotectTask(location)
- `service|`/`driver|` → DisableServiceOrDriver: read Start (must be 0..3), metadata `name\toriginalStart`, set Start=3 (demand — still manually usable; NOT 4)
- `wmisub|` → archive Status (filter\tconsumer\tclass[\tquery]) + delete binding via WMI
- After all (except overlay/packaged/task quiet handling): EnabledStartupService.Remove(item)

## EnableCore dispatch
- legacy| → EnableLegacyV2 (recreate managed task from state-v2.json)
- overlay → SetStartupApproval(true) + UpsertFromItemChecked + ProtectedDisabledService.Unprotect
- packaged| → State=2 + upsert + unprotect
- `disabled|winlogon|`/`appinit|`/`advanced|` → RestoreAdvancedRegistryValue: decode record from Status (JSON), verify guard unchanged, WriteRegistryValueStateExact(After→Before), remove store row
- `disabled|reg|` → RestoreRegistry: parse meta `HKEY_…\tsubKey\tvalueName\tkind\tsource\tB64(payload)\tview` (legacy fallback kindIndex=1); if value exists live already → just remove store row (vendor re-created it; live wins); else SetValue exact kind; remove row
- `disabled|active|` → refuse if StubPath re-created externally; else write back
- `disabled|folder|` → File.Move(dest→Status original); refuse if original exists
- `disabled|service|`/`driver|` → RestoreServiceOrDriver: current must be 3 or 4 (3=our demand-start, 4=legacy hard disable); set back to stored start; remove row
- `disabled|wmisub|` → re-create __FilterToConsumerBinding (Filter/Consumer refs from meta[0]/meta[1])
- `task|` → `schtasks /Change /TN <loc> /Enable` + upsert + unprotect + re-assert quiet protection if IsQuietLaunch
- Then: EnabledStartupService.UpsertFromItemChecked + ProtectedDisabledService.Unprotect

## Transaction wrappers
- Single: ExecuteStartupMutation = coordinator.Run { snapshot StartupMutationSnapshot.Capture(item, op); mutate; on error snapshot.Restore(item) } (message: "…; every earlier startup route and intent stores was restored. Cause: …")
- Bulk: SetAllRoutesEnabled — filter changed-only; GroupBy SharedUserRegistryPhysicalIdentity (collapse HKCU/HKU shared views, EXCEPT Classes/_Classes); order by BulkMutationSourceKey then component index DESC then Id; EnsureMutationAuthorized for ALL first (fail closed); snapshot before each; rollback in reverse order on failure.
- EnsureMutationAuthorized: ExternalAuthority → throw; !CanDisable → throw MutationReason; RequiresExpertConfirmation && !expertConfirmed → throw.
- Disable()/Enable() only check authorization when (MutationCapability set || RequiresExpertConfirmation || ExternalAuthority set) — plain rows (e.g. Registry Run without capability fields from providers) proceed.

## Registry exactness
- RegistryValueState {Exists, Kind(String/ExpandString/Binary/DWord/QWord/MultiString/None), Payload, Hash} — hash=SHA256("1|0\nkind\npayload") hex; MULTI_SZ payload = JSON array string (System.Text.Json, e.g. `["a","b"]`); Binary = base64; DWord/QWord invariant int string.
- WriteRegistryValueStateExact: re-read current, must equal expectedBefore (else "changed after scanning; refresh"), write desired (SetValue kind-exact / DeleteValue), re-read verify == desiredAfter.
- ShouldApplyExactRegistryRollback: no-op if current==original; throw if current!=expectedMutation; true only if current==expectedMutation.
- SerializeRegistryValue: MultiString→JSON array; Binary→B64; else invariant string.

## StartupApproved
- 12-byte value: 4-byte state + 8-byte FILETIME. Valid only if len==12, bytes1-3==0, (b0&0xF8)==0. disabled = b0&1. Toggle: only flip bit0, preserve rest.
- Read roots: HKCU + HKLM + loaded hives; kinds Run, Run32, StartupFolder, StartupFolder32, StartupTasks; path `Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\<kind>`; view = native (Registry64).
- ApplyStartupApproved: match by scope + name (Startup Folder: file name vs approval name; Packaged: family!taskId vs taskId after '!'; Startup Command: name + Wow6432Node↔Run32; Registry Run: name + view/Run32). Malformed → error|Startup Approval| row (Raw bytes). Match found → overlay on canonical row (Enabled=!disabled; capability StartupApprovedOverlay if writable; status suffix `; Explorer StartupApproved=<disabled|state 0xXX>`). Ambiguous (multiple, after mirror filtering) → error|Startup Approval Ambiguity|. Orphan enabled → new row Id `startupapproval|b64(scope)|b64(root)|b64(path)|b64(name)` Source "Startup Approval". Orphan disabled → Id `staleapproval|...` Source "Stale Startup Metadata" Enabled=false.
- SetStartupApproval: refuse if value missing/kind changed/bytes differ from scan (ApprovalData) → "refresh before changing it"; write toggled bytes; verify readback.

## Agent registration (boot logon task)
- Task `\MichStartupMaster\MichStartupMasterApp`, action `<exe> --agent`, single logon trigger (current user, no delay), Enabled=true, DisallowStartIfOnBatteries=false, StopIfGoingOnBatteries=false, StartWhenAvailable=true, Principal InteractiveToken LeastPrivilege.
- EnsureAgentRegisteredCore: transactional — verify/create task, retire legacy shortcuts (`Mich Startup Master Agent.lnk` in user+common Startup folders whose target is own exe --agent or system wscript/cscript running MichStartupMasterAgent.vbs containing exe path + --agent), rollback restores task XML + shortcuts.
- VerifyAgentRegistration receipt: `REGISTER_AGENT_OK|FAILED task="<loc>" action="<exe> --agent" <taskDetail flags> ownedRoutes=N legacyRoutes=N` (taskDetail = singleLogonTrigger=… etc; taskExists=false if missing)
- Missing-task HRESULTs: 0x80070002, 0x80070003, 0x8004130F (+inner exceptions) are "missing"; others preserved as errors.

## DisabledStoreService.Add → returns Id `disabled|<type>|...`; Remove(id); LoadDisabledItems() returns StartupItems with Id `disabled|…`, Command/Status carrying metadata (see 01/02/03 formats above).
- reg metadata columns: root\tsubKey\tvalueName\tkind\tsource\tB64(payload)\tview
- service/driver: name\tstart ; folder: originalPath ; wmisub: filter\tconsumer\tclass[\tquery] ; winlogon/appinit/advanced/extension: JSON AdvancedDisabledState (exact System.Text.Json casing, camelCase? — NO: C# uses default JsonSerializer options → PascalCase property names, e.g. {"Version":1,"RouteType":"winlogon",...}; RegistryValueState nested also PascalCase: Exists/Kind/Payload/Hash)

## Managed startup (AddManagedStartup / managed tasks) — read later (9127-10005): ChooseManagedTaskName (sanitize, collision suffix, non-Latin names), FindEquivalentManagedRoute (reuse), BuildDirectAction (wscript //B for .vbs/.js etc, direct for .com/.exe), StripKnownQuietArguments (--minimize-to-tray for Speedy.exe, --start-in-tray for self), quiet payload task command = `<exe> --tray-run <b64(target\nargs)>`, CanUpsertManagedTaskState, EditStartup, DeleteManagedTask, TogglePopupMode, ResolveLaunchTarget, TryDecodeTrayPayload, IsQuietLaunch.

## Misc exact strings
- RunChecked("schtasks.exe", "/Change /TN " + Q(loc) + " /Disable|/Enable") — Q() = quote
- error rows: ErrorItem(source, scope, location, ex) → Id `error|<source>|<location>` (with '\_' etc raw), Status "Read failed", Command = exception message (TaskReadException: `TypeName HRESULT=0x…: message`)
