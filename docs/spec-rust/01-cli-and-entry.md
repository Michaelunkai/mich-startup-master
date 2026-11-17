# Spec 01 — CLI, entry, state-store plumbing (from C# 2.10.1 source)

## State root
- `%LOCALAPPDATA%\MichStartupMaster` (default). Override env `MSM_STATE_ROOT`:
  must be absolute, leaf must contain `MichStartupMaster-test-` or `msm-test-`, not a root/user-profile/localappdata itself. Throws otherwise.
- Files: `disabled-items.tsv`, `protected-disabled-items.tsv`, `protected-quiet-popup-items.tsv`,
  `enabled-startup-items.tsv`, dir `DisabledStartupFolderItems/`, `crash.log` (1MB rotate to .previous), `known-startup-items.tsv` (first-seen), plus `.bak` recovery pattern per file.
- ManagedTaskRoot: `\MichStartupMaster\`

## Entry semantics
- No args → GUI MainForm (mutex `Local\MichStartupMaster.MainInstance`; if exists → find window class prefix `WindowsForms10.Window`, title EXACT `Mich Startup Master — Startup Control` (em dash U+2014), PostMessage 0x8042, retry 25ms up to 2s)
- `--start-in-tray` / `--agent` → EnsureAgentRegistered + BootReconciliation.RunOnce (background) + MainForm(startInTray:true)
- Unhandled exceptions → crash.log + quiet exit 1 (never WerFault dialog)
- HighDpi PerMonitorV2. Single-instance mutex is `Local\`.

## CLI commands (exact behavior; exit codes matter to scripts)
- `--version` → prints `MichStartupMaster 2.10.0` (hardcoded in C#!) exit 0
- `--list [--list-enabled|--list-disabled]` → JSON array of items (dedupe by Id, case-insensitive, first wins)
- `--list-sources` → `SOURCES count=N` + `  <source>` lines
- `--search <q>` → JSON filtered (Name, AppName, Command, Location, Source, Status, VerifiedState, Id, ApplicationIdentity)
- `--export` → TSV via BuildStartupExportTsv
- `--get-item <key>` → ID:/Name:/Source:/Scope:/Enabled:/Location:/Command:/Status:/Risk:/CanDisable:/IsManaged:/ApplicationIdentity:/MutationCapability:/MutationReason:/RequiresExpertConfirmation:/RequiresElevation:/RequiresReboot:/ExternalAuthority: + `---` separator; key matches Id|Location first, then Name (TrimStart '\'); not found=4, ambiguous=5
- `--set-enabled <key> <true|false>` → disable/enable + rescan verify → `SET_ENABLED <name> requested=<b> verified=<b> matches=<n>`; verified false → exit 3
- `--smoke` → line `SMOKE passed=<b> inventory=<n> invalid=<n> duplicate_ids=<n> provider_errors=<n> type_init_failures=<n> user=<u> appdata=<p>`; pass = count>0 && invalid=0 && dup=0 && providerErr=0 (Id starts `error|` or Status=="Read failed")
- `--audit-boot` → prints AuditBootCoverage() + AuditTrayCoverage() lines; boot pass regex `independent=true`, `gaps=0`, `errors=0`; tray line `^TRAY_AUDIT apps=(\d+) running=(\d+) findings=(\d+) uncertain=(\d+)` pass = apps>0 && apps==running && findings==0 && uncertain==0. Exit 0 pass / 4 fail.
- `--verify-live-inventory` → receipt line from MainForm.VerifyLiveInventory, `passed=true` + exit 0 else 4
- `--add-startup <name> <path> [args] [normal|tray]` → AddManagedStartup → `ADD_STARTUP task=<loc> mode=<m> exists=<b>`; exit 0/2
- `--remove-task <name>` → `REMOVE_TASK <name> exists=<b>` exit 0/3
- `--add-test-task[-tray|-normal]`, `--edit-startup`, `--run-now`, `--open-location`, `--open-folders`
- `--toggle-popup <key>` → quiet-mode toggle; refused if disabled (exit 6)
- `--repair-rules` → `EnforceProtected | EnforceProtected | EnforceEnabled(true)` joined by ` | `
- `--protect-disabled`, `--enforce-disabled|--enforce-quiet|--enforce-enabled`
- `--detect-new` → `DETECT_NEW count=N [first=Name]`
- `--reconcile-managed-startups [prefix]` → receipt with `conflicts=N`; exit 3 if conflicts>0
- Self-tests: `--truth-self-test`, `--inventory-self-test`, `--state-store-self-test` (prints `STATE_STORE_SELF_TEST workers=12 atomic=ok recovery=ok`), `--managed-startup-dedupe-self-test`, `--bulk-disable-self-test`, `--agent-registration-self-test`, `--release-gate-self-test` (prints `RELEASE_GATE_SELF_TEST_OK ...`), `--ui-contract`, `--ui-self-test`, `--quiet-policy-probe`, `--quiet-lineage-self-test`, `--quiet-performance-probe`, `--quiet-plan <base64>`
- Internal: `--shell-open <b64>`, `--state-store-worker <b64path> <token>` (appends token if missing), `--provider-worker`, `--tray-run`
- Release-gate child: flag `--release-gate-child` only with `--register-agent|--verify-agent` + env `MICH_STARTUP_MASTER_RELEASE_CHILD_TOKEN` (64 hex); parent holds deployment mutex, child consumes one-shot event `Local\MichStartupMaster.ReleaseChild.<token>` via claim mutex.

## StateStoreFile semantics (Rust must match)
- Cross-process mutex `Local\MichStartupMaster.State.<sha256(lower(fullpath))[0..32 hex]>`, 30s timeout, AbandonedMutex = acquired
- Recovery: no file + .bak → move .bak→file; both → delete .bak
- Atomic write: tmp file `<name>.tmp.<pid>.<guid-n>` in same dir → File.Replace(path, backup=.bak, ignoreMetadata) → delete .bak; UTF-8 **no BOM**, `\r\n`? (C# WriteLine = \r\n on Windows), flush(true) = FILE_WRITE_THROUGH
- UpdateLines = read-modify-write under lock (transactional); DeleteIfEmpty refuses nonempty
- Generation counter bumped on every write (in-process staleness detection)

## StartupItem fields (the row model)
Id, Name, Source, Scope, Command, Location, AppName, ApplicationIdentity, ApplicationRoot, ApplicationIdentityReason, ApplicationTargets[], RuntimeEvidence, Enabled, VerifiedState (default "Unknown"), EvidenceReason, PresentationState ("Needs verification"), PresentationReason, CanDisable, IsManaged, Status, ApprovalRoot/Path/Name/Data/RegistryView (StartupApproved overlay), RegistryView, MutationCapability, MutationReason, RequiresExpertConfirmation, RequiresElevation, RequiresReboot, ExternalAuthority.
- StateText(): preview| → Enabled/Disabled else VerifiedState
- PopupLabel(): N/A for ~26 system sources (Windows Service, System Driver, Winlogon Autostart/Notification, AppInit/AppCert DLLs, LSA Startup Package, Active Setup, Boot Execute, Image Hijack, Known DLL, Network/Winsock Provider, Print Monitor/Provider, Security Provider, Terminal Server Startup, Icon Service Library, URL Search Hook, Service Dependency, Media Codec, WMI Event Consumer, Group Policy Script, Explorer Startup/Shell Extension, Internet Explorer Add-on, Startup Approval, Stale Startup Metadata); disabled→"Disabled"; quiet-launch→"Disabled" else "Enabled"
- RiskLevel(): Critical (system driver, winlogon autostart, appinit, boot execute, LSA; security/core service via needle list defender/windefend/security/firewall/antivirus/anti-malware/antimalware/malwarebytes/mbam/vpn/credential/crypt/event log/rpc/plugplay/winmgmt; boot task under \Microsoft\Windows\), Review (temp/appdata\local\temp/powershell/cmd.exe/wscript/cscript), System (Scope==Machine), Normal
- AdviceLevel: Cleanup if regex updater|update task|update check|updatecurrentuser|updatecurrentmachine|--wake|/wake|tray icon|tray_icon|trayicon|helper tray|notification icon|telemetry|crash reporter|crashreporter|usage reporter|feedback uploader|installer dialog watchdog|auto.?install guardian|visible downloads live sync|popup rescue (only when enabled && not Critical)

## Mutation coordinator (locking model)
- Two global mutexes: `Local\MichStartupMaster.ReleaseDeployment` then `Local\MichStartupMaster.ManagedStartupMutation` (handoff: release deployment only after mutation owned). 30s timeout. Reentrant per-thread.
- NativeProcessCatalog: Toolhelp32 + OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION) + QueryFullProcessImageName(native) + NtQueryInformationProcess(ProcessCommandLineInformation=60, ProcessBasicInformation=0), device map via QueryDosDevice cached 5s, budget 1500ms, stall counter abandons readers (never blocks scan on wedged volumes).

## Concurrency invariants to preserve in Rust
- Store writes: named-mutex + atomic replace + .bak recovery + generation bump.
- Mutation: two-level mutex, boot reconciliation reentrancy.
- Process catalog bounded, never blocks UI thread.
