# Mich Startup Master 2.7

Mich Startup Master is a native Windows 11 control center for seeing and managing what starts with Windows. Version 2.3 separates two questions that startup tools often blur together:

- **State:** should this registration run at boot/sign-in?
- **Mode:** should the app open a normal window, or start quietly and stay available in the system tray?

The default view is **All routes**: every discovered Windows startup registration is visible immediately, exactly once per registration. Opening the dashboard from its tray icon or using **Clear filters** also resets to that complete route-level view and clears stale search/drill-down state. **Apps** is an optional summary view; it groups exact routes by a proven installed-product path owner or the real executable behind a launcher, includes app-owned automatic services such as Logitech G HUB's updater, and can enable or disable every matching route for one app in a single rollback-safe transaction. Display-name similarity is never treated as ownership evidence.

## What changed in 2.7

- **Packaged task state is now read from the real OS authority.** Explorer's `StartupApproved\StartupTasks` key often does not exist at all, and 2.6 then fell back to the manifest default, marked the row `ExternalAuthority`, and left it permanently non-toggleable — WhatsApp-class apps appeared with the wrong enabled state and could not be enabled or disabled from the app. The scan now reads `HKCU\...\AppModel\SystemAppData\<family>\<task>\State` (the exact value Windows Settings and Task Manager write): State 2 shows as enabled, 0/1 as disabled, and the row is always user-toggleable. Only policy-controlled states (3/4) stay blocked, with the reason shown. Toggling writes that same State value and verifies it after writing.
- **Packaged surfaces are watched in real time.** The change monitor now watches `AppModel\SystemAppData` and `AppModel\Repository\Packages` under HKCU, so a packaged app that enables or adds its startup task (or a package that installs one) refreshes the view immediately instead of waiting for the next poll.

## What changed in 2.6

- **Packaged startup tasks can no longer be missed.** Windows denies existence probes (attribute queries) on files under `C:\Program Files\WindowsApps`, so `File.Exists` reported false for every installed package manifest and the scan silently skipped every packaged StartupTask — WhatsApp, OpenAI Codex, Windows Terminal, the Xbox app and any other Store or MSIX app that registers `windows.startupTask`. The scan now hands every registered package to a direct-open reader: a manifest that exists is always read, a genuinely absent manifest is skipped without noise, and any other read failure still surfaces as an error row so a gap is never silent.
- **Newest-added-first everywhere.** Every completed scan records the first moment each startup identity was observed into a persistent first-seen store (`startup-first-seen.tsv`). All views — Apps, Enabled, Disabled, and search results — lead with the most recently added item (WhatsApp, Codex, ChatGPT-class additions appear at the top the moment they are installed), with the previous name ordering as a stable tiebreaker. Recorded timestamps are never rewritten, and an explicit column sort still takes precedence.

- **Tray audit recognizes native NotifyIcon hosts.** Process Lasso's tray icon window (`*_Notification_Class`) was invisible to the quiet-coverage audit, falsely reporting a running tray app as having no tray host. The audit now accepts native `*_Notification_Class` tray hosts, so the boot gate reports the truth instead of a false gap.

## What changed in 2.5

- **Change-triggered live refresh.** The resident process now watches the registry surfaces it scans (HKCU/HKLM Run, RunOnce, RunServices, policy Run, Winlogon, Environment, and StartupApproved metadata in both registry views), both Startup folders, and the Task Scheduler catalog with native change notifications. An external edit raises one coalesced re-scan within about a second, so the dashboard reflects a startup change made by any program the moment it happens instead of waiting for the 15-second background poll, which remains as a safety net for surfaces without OS notifications (services, drivers, WMI consumers). A change that lands while a scan is already running is never dropped: it is replayed as soon as the in-flight scan completes.
- **A disabled route that exists live again is live.** When a physical registration that was previously disabled through the app reappears (a vendor installer or updater re-created it), the scan retires the stale disabled record instead of showing the same value twice — once enabled and once disabled. The live registration is always what the dashboard shows, so nothing that will actually run at boot can ever display as disabled.
- **Disable/Enable at boot works directly on every app row.** A Windows `Win32_StartupCommand` entry that mirrors an authoritative route in the same application group (for example Ollama: one `Win32_StartupCommand` projection plus the real Startup-folder shortcut) is a mirror with no independent existence: it no longer blocks the app-level state action, is never itself written to, and follows its authoritative twin automatically. The button counts and mutates exactly the authoritative routes. A mirror whose authoritative registration is not enumerable still fails closed, and routes owned by an external authority always block with their reason.
- **`--verify-live-inventory` matches the shipped dashboard.** The release gate now proves the contract the app actually ships: every application-class route renders exactly once in the default **Apps** view (as its own row or inside its proven application group), every enabled route renders exactly once on the **Enabled** view, every not-enabled route renders exactly once on the **Disabled** view, and every route is findable by searching its own displayed name on a view where it lives.

## What changed in 2.2

- Responsive, high-DPI WinForms UI with a compact header, search, four focused filters, contextual actions, keyboard access, accessible names, and explicit loading/empty/error states.
- Separate **Enabled/Disabled** and **Window/Quiet (tray)** columns and controls.
- Click any inventory column header to sort by its displayed value; click it again to reverse. The first **Status** click places all enabled entries first. Keyboard and screen-reader users can use **Sort by** (`Alt+S`) and the adjacent direction button for the same six sort choices and an announced result.
- **Refresh is read-only.** Repair and protection are explicit Tools actions.
- Exact repeated adds reuse one canonical managed route. The complete inspect/choose/write operation is protected by one cross-process transaction, so simultaneous adds also converge on one route. The boot agent transactionally retires exact duplicates and disabled `Launcher`/`Launch` aliases, including their saved quiet/enabled intent, so they cannot reappear after reboot.
- The **Apps** tab exposes **Disable all** and **Enable all** for an app with multiple routes. Every route is authorized before the first write; if any route fails, all earlier routes and intent stores are restored.
- The tray process preloads the complete inventory before its window is opened. During later read-only refreshes, the last verified snapshot remains searchable and the four view tabs remain usable; all state-changing actions stay locked until the replacement scan finishes.
- Script-backed startup applications retain the identity of their script or owning Codex project instead of being mislabeled as PowerShell, `cmd.exe`, AutoHotkey, or a temporary child executable. All routes below `%LOCALAPPDATA%\Codex\GameAutoInstall` therefore appear together as **Game Auto Install**, while the direct qBittorrent client remains a separate **qBittorrent** app.
- Script-hosted startup routes (`wscript`, `cscript`, PowerShell, cmd, and Python) resolve bounded, literal executable payloads instead of being grouped under the generic host. Multi-path fallback wrappers use exact live-process correlation to select the active executable while retaining one physical registration and one mutation.
- A disabled registration whose exact payload is still running is shown as **Disabled · running now**, keeping current runtime observation separate from next-boot configuration.
- StartupApproved metadata is overlaid on its real registry/Startup-folder/Windows startup-command row instead of appearing as a contradictory duplicate. An enabled approval is never hidden even when Windows withholds its launch command.
- An independent boot audit checks the displayed inventory against separate Windows enumerators.
- State stores use cross-process locking and atomic replacement.
- One canonical logon task starts the hidden agent. Legacy app-owned Startup-folder launchers are retired, not retained as a second route.
- Quiet startup no longer contains the old 60-second re-hide guard.

## Startup coverage

The inventory covers the Windows startup surfaces used by this project, including:

- Registry Run, RunOnce, RunOnceEx, RunServices, policy, loaded-user, and 32/64-bit views
- Windows `Win32_StartupCommand` fallback entries, read in a timeout-isolated worker and deduplicated against native routes, including exact PATH-resolved commands such as `wscript.exe` (this covers launchers such as Logitech G HUB that Windows reports even when its Run value is not enumerable)
- Per-user and common Startup folders
- Boot and logon scheduled tasks
- Automatic services and boot/system/automatic drivers
- Winlogon values, Active Setup, AppInit DLLs, and AppCert DLLs
- Winlogon notification DLLs, image-launch interceptors, Known DLLs, network providers, 32/64-bit Winsock catalogs, print monitors, and media codecs
- BootExecute/Session Manager and LSA startup packages
- Group Policy startup/logon scripts
- Executable WMI event consumers
- Explorer startup extensions, shell hooks/icon overlays, and Internet Explorer add-ons (including Browser Helper Objects, toolbars, search hooks, and extensions)
- Packaged application StartupTasks

`--audit-boot` performs a second enumeration and reports `gaps` and provider `errors`. Quiet coverage separately reports expected apps, running apps, findings, and uncertain tray detection. A clean result requires independent enumeration, `gaps=0`, `errors=0`, `apps=running`, `findings=0`, and `uncertain=0`.

`--verify-live-inventory` is the release check for the visible dashboard: it scans Windows, renders the default **Apps** view, and fails if any discovered route is not rendered exactly once (as its own row or inside its proven application group), is duplicated across rows, is contradicted by an enabled/disabled or scope conflict, cannot be found by searching its own displayed name or human app name, is missing from the Enabled view, is contradicted by the independent boot audit, or a running launcher payload does not correlate to the application identity shown in **Apps**. A route whose physical registration was previously disabled through the app but exists live again is treated as live: the stale disabled record retires instead of showing one value twice. The live gate additionally proves Logitech/LGHUB visibility, Game Auto Install project ownership, direct qBittorrent ownership, and rejects any script application that is still grouped under its interpreter or child payload.

Inventory never synchronously probes file metadata on a removable, network, or other non-system volume. Those routes remain visible with their full configured command; task availability is reported as **Unknown** until the storage is responsive instead of freezing the dashboard or declaring the app missing.

System extension and catalog routes are displayed as read-only until their format has a dedicated reversible mutation handler. This prevents a dangerous "disable" action from corrupting a boot-critical registration while still making its configured module/path visible.

## Quiet (tray) mode

Quiet mode means the app is ready after sign-in without presenting its normal GUI.

- When an app has a known native tray launch mode, Mich Startup Master uses it directly.
- OpenSpeedy is normalized to `Speedy.exe --minimize-to-tray`; legacy nested VBS launch chains are removed from the managed route.
- OpenAI.Codex's `Codex.exe` launcher alias is normalized to the MSIX manifest entry `ChatGPT.exe`, so quiet startup attaches to the single native tray host instead of creating a second launcher/icon.
- For other apps, the fallback hides only the first startup window batch, and each handle at most once.
- A window opened manually later is never re-hidden.
- If the app has no usable native tray entry, a single app-named Startup Master tray controller provides **Open** and **Exit**.
- Clicking Open while the target is still starting waits for/restores its window; it does not launch a duplicate merely because the window is not ready yet.
- A short-lived `FooLauncher.exe` handoff is followed only to the deterministic `CustomRuntime\Foo.exe`, `Runtime\Foo.exe`, or sibling `Foo.exe` payload. If that payload is already running, the controller attaches to it instead of invoking the launcher again, and any fallback tray icon comes from the real payload.
- Process lineage is event-backed, follows fast multi-hop script launchers, and qualifies process generations so a reused PID cannot control an unrelated process.

Managed startup identity is fail-closed: one exact target-and-arguments route has one canonical task. A sole enabled `Foo` route also retires a disabled `FooLauncher`/`FooLaunch` alias, which prevents an old wrapper from later producing a second tray icon. Two differently configured enabled routes for the same logical slot are reported as a conflict and a new third route is refused; the app never guesses which enabled application the user meant to remove.

Native tray discovery for arbitrary third-party apps is capability-dependent. The audit reports uncertain cases instead of declaring them clean.

## Use the app

Run the portable build:

```powershell
.\build\MichStartupMaster.exe
```

The main actions are:

- **Add startup** — paste a full path (quoted paths and environment variables work) and press Enter. Any file extension, including extensionless files, is accepted; browsing is optional and the name is filled automatically. Documents and shortcuts open through their Windows default app. Window mode is the default. Files must still exist and Windows needs a suitable app to open non-executable files.
- **Enable/Disable at boot** — changes only the selected registration in **All routes**. In **Apps**, **Disable all** and **Enable all** change every matching registration for the selected app atomically.
- **Use Quiet tray / Use Window** — changes startup presentation without conflating it with enabled state.
- **Run now** — opens the selected app without changing startup configuration.
- **All routes** — shows services, drivers, logon hooks, policy scripts, and every distinct registration.
- **Mark and copy any number of routes** — use Control/Shift-click to mark exactly the rows you want, click **Select all visible** (or press `Ctrl+A`) to mark the current tab/search immediately, then press **Copy selected** (or `Ctrl+C`). **Clear selection** starts over without changing a startup setting. Startup-changing controls intentionally lock while more than one row is marked. The Tools menu also offers all-data and visible-data copy actions. Copy output is tab-separated and includes every captured route field—including configured/verified/presentation state, configured command, location, risk, evidence, ownership, mutation metadata, capabilities, and raw StartupApproved metadata—ready to paste into Notepad or Excel.

Machine-wide sources may require an elevated process because Windows protects those registrations.

### Companion tools in the portable release

The portable release ZIP also carries the companion executables used by this workspace:

- `tools\companion\MichAutoClipSyncTray.exe` — Windows/Android clipboard-sync tray entry point.
- `tools\companion\Start-MichAutoClipSync.ps1` — sync runner; it resolves the current user's Android platform-tools location and then falls back to legacy locations and `PATH`.
- `tools\thaw\Thaw.exe` — self-contained Thaw release executable.

These tools are optional; the main startup manager remains under `build\`.

### Keyboard

- `Ctrl+N`: add
- `Ctrl+F`: search
- `F5`: read-only refresh
- `Space`: enable/disable selected route
- `Ctrl+Q`: switch Window/Quiet mode
- `Enter`: edit
- `Ctrl+L`: run now
- `Shift+F10`: context menu

## Command line

| Command | Purpose |
|---|---|
| `--version` | Print the product version |
| `--list` | JSON inventory of captured startup routes |
| `--audit-boot` | Independent boot and tray coverage receipts |
| `--verify-live-inventory` | Prove the default route-level dashboard matches the live OS inventory exactly once per registration |
| `--add-startup <name> <path> [args] [normal\|tray]` | Add or reuse a managed route; default is `normal` |
| `--set-enabled <id\|location> <true\|false>` | Change one unambiguous route and verify the result |
| `--toggle-popup <id\|location>` | Switch a supported app route between Window and Quiet |
| `--register-agent` | Register and verify the one canonical logon agent |
| `--verify-agent` | Read-only verification of the canonical agent task |
| `--protect-disabled` | Explicitly protect the current disabled set |
| `--enforce-disabled` | Repair explicitly protected disabled state |
| `--enforce-quiet` | Repair managed Quiet routes |
| `--enforce-enabled` | Repair the enabled manifest |
| `--reconcile-managed-startups [prefix]` | Transactionally retire managed exact duplicates and stale disabled Launcher/Launch aliases; exits nonzero for enabled conflicts |
| `--managed-startup-dedupe-self-test` | Pure canonical-route, stale-alias, and conflict safety fixtures |
| `--bulk-disable-self-test` | Safe transactional rollback proof for app-level bulk disable |
| `--ui-preview [scale]` | Safe sample UI with system actions disabled |
| `--ui-self-test` | Layout, accessibility, grouping, and safe-preview checks |
| `--inventory-self-test` | Provider/dedupe fixture checks |
| `--state-store-self-test` | Concurrent atomic-state test |
| `--quiet-policy-probe` | One-shot window suppression contract |
| `--quiet-lineage-self-test` | Multi-hop/process-generation contract |
| `--smoke` | Read-only live inventory invariants |

## Build and test

The repository pins .NET SDK `10.0.301` in `global.json` and locks NuGet resolution in `packages.lock.json`. The build can bootstrap the pinned SDK into the ignored project-local `.dotnet` directory when existing SDKs are unhealthy.

Create and validate a stage without touching the running installation:

```powershell
.\scripts\build.ps1 -StageOnly
```

Compile a receipt-bound installer from that validated stage:

```powershell
.\scripts\build.ps1 -StageOnly -CompileInstaller
```

Run the default safe suite against a selected staged executable:

```powershell
.\scripts\test.ps1 -TestAppPath 'C:\path\to\stage\publish\MichStartupMaster.exe'
```

The default suite is read-only or isolated. Disposable service/registry/task tests are opt-in and require an elevated shell:

```powershell
.\scripts\test.ps1 -TestAppPath 'C:\path\to\stage\publish\MichStartupMaster.exe' -AllowLiveMutation
```

Do not treat fixture tests as proof of sign-in behavior. The final Quiet-mode acceptance gate is a real task launch/logon observation: no startup GUI flash, exactly one usable tray entry, and immediate manual opening. A real reboot/sign-in should be performed only with the user's approval.

## Preservation-aware deployment

Running `scripts\build.ps1` without `-StageOnly` performs a guarded deployment. Before replacing the live build it records the exact process command-line multiset and backs up the deployed files, `%LOCALAPPDATA%\MichStartupMaster` state, app-owned task XML, and app-owned shortcut registrations. It stages first, swaps only after validation, restores exact prior state on failure, and never uses a broad process-name kill.

Build receipts and rollback artifacts are under `artifacts\`; these generated directories are intentionally ignored by Git. A clean Git status is therefore not a deployment receipt.

## Project layout

```text
MichStartupMaster.csproj       .NET 10 WinForms project
global.json                    pinned SDK
packages.lock.json             locked dependency graph
src/MichStartupMaster.cs       UI, inventory, state, guards, CLI, and tray runner
assets/MichStartupMaster.ico   application icon
scripts/build.ps1              stage, provenance, rollback, and deploy workflow
scripts/test.ps1               safe default and opt-in live tests
installer/MichStartupMaster.iss receipt-bound per-user installer
build/                          current deployed portable build
```

User state lives in `%LOCALAPPDATA%\MichStartupMaster` and is preserved by build rollback and uninstall. Uninstall removes only registrations proven to belong to that installed executable.

## License

[MIT](LICENSE) © Michael (Michaelunkai)

## Startup evidence (source repair)

The State column uses fresh registration evidence. Unknown and Drifted carry reasons; a package manifest alone does not establish StartupTask authority. The Mode column reports Needs verification until startup presentation is observed. Configuring a wrapper or native tray switch does not prove quiet behavior. Task readback validates its exact action, target, managed intent and applicable trigger. Other surfaces currently remain Unknown when equivalent verification is unavailable. Refresh performs no repair.

CLI `--list` includes `startupState`, `startupReason`, `presentationState`, and `presentationReason`. Legacy `enabled` and `popup` fields describe configuration only, explicitly labeled by their evidence fields; do not use them as successful behavior receipts. `--truth-self-test` checks missing and contradictory evidence and the Codex wrapper regression, and is required by stage and safe-test gates.

Use `scripts\build.ps1 -InstallCurrentUser` to apply the guarded workflow to `%LOCALAPPDATA%\Programs\MichStartupMaster`. Its swaps stay on the installation volume; receipts and backup copies remain under project artifacts. The default still updates the portable `build` directory. A filename such as ChatGPT.exe is no longer proof of native tray capability.
