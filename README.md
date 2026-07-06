# Mich Startup Master

Mich Startup Master is a Windows-native startup-control application. It gives you one polished place to see and manage what runs when Windows starts or when you log in.

This version fixes the taskbar/pinned-app icon by embedding a real native `.ico` into the compiled EXE, upgrades the UI into a friendlier dashboard-style control room, and verifies both startup-add modes: normal launch and quiet tray-wrapper launch.

## Runnable app

```powershell
& 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe'
```

## What it does

- Shows Windows startup entries from `Win32_StartupCommand`.
- Shows Registry Run, RunOnce, RunServices, policy Run, and 32-bit Wow6432Node startup entries from HKCU and HKLM.
- Shows user/common Startup-folder entries.
- Shows enabled and disabled logon Scheduled Tasks by inspecting real `Get-ScheduledTask` trigger objects instead of fragile localized `schtasks` schedule text.
- Shows auto-start Windows services and boot/system/auto-start drivers, including service-backed apps such as Malwarebytes.
- Shows Active Setup, Winlogon autostart, AppInit DLL, and Explorer StartupApproved disabled metadata so obscure startup mechanisms are visible instead of hidden.
- Resolves a human-readable application name for every row using shortcut targets, tray-wrapper payloads, executable metadata, service display names, and cleaned fallbacks, while preserving the raw startup entry name for exact control.
- Lets you disable/enable supported startup entries.
- Lets you disable/enable reversible startup sources, including Registry startup values, Startup folder items, Scheduled Tasks, Active Setup StubPath values, Windows services, and system drivers when running elevated.
- Lets you add executable startup targets with no Task Scheduler delay: `.exe`, `.cmd`, `.bat`, `.ps1`, and `.lnk`.
- Lets you edit app startup entries from the main window. Managed scheduled tasks are updated in place; editable registry/folder/task entries can be removed and replaced with a managed entry.
- Lets every added app choose one of two startup modes:
  - **Start normally** — runs the selected executable directly at Windows logon.
  - **Start quietly in tray mode** — starts through Mich Startup Master’s GUI-subsystem tray wrapper, launches minimized, avoids terminal popups, and keeps a controller tray icon.
- Protects quiet popup-disabled startup tasks in `%LOCALAPPDATA%\MichStartupMaster\protected-quiet-popup-items.tsv`; if another tool changes a protected task back to normal launch, `--enforce-quiet` restores the tray-wrapper action.
- Keeps manual opening separate from quiet startup: launching `MichStartupMaster.exe` normally opens the GUI, while its startup self-task uses `--start-in-tray`.
- Marks genuinely high-consequence startup mechanisms in red: boot/system/auto drivers, Winlogon/AppInit startup points, security/core services, and Microsoft boot/startup infrastructure tasks. Normal user app startup is not marked red just because it starts with Windows.
- Marks conservative green cleanup suggestions only when the entry is enabled, non-critical, and strongly matches optional-startup patterns such as updater wake tasks, tray-icon helpers, telemetry/crash reporters, or installer/watchdog helpers. Normal useful app startup is not marked green by default.
- Loads the startup inventory asynchronously so the window remains responsive while services, drivers, scheduled tasks, and registry startup surfaces are scanned.
- The Add/Edit dialog explicitly offers both choices for every startup target you add: **Start normally** or **Start quietly in tray mode**.
- Includes a real embedded application icon for the EXE, window, tray icon, and taskbar/pinned shortcut path.
- Includes a tray icon for Mich Startup Master itself; closing the window hides it to the tray instead of killing it.

## UI upgrades

The main window is now a dashboard-style control room:

- Embedded branded app icon in the title bar, tray, and EXE/taskbar resource.
- Large friendly header with plain-language explanation.
- Metric cards for visible, enabled, disabled, review, and managed items.
- Cleaner command center with search, all/high-risk/suggested-cleanup/disabled filters, add quiet startup, refresh, make quiet, disable, enable, managed-task delete, protect disabled, enforce now, and open startup folders.
- More readable startup table with status/trust emphasis and approachable helper copy.
- Application-first table: `Application`, `Startup entry`, `Source`, `Risk`, `Cleanup`, `Popup`, `Location`, and `Launch command`.
- Right-click menu on startup rows for edit, remove, restore, make quiet, launch now, open location, copy command, and refresh.
- Keyboard shortcuts for common work: `Ctrl+N` add, `Enter` edit, `Delete` remove, `F5` refresh, `Ctrl+L` launch now, `Ctrl+O` open location, `Ctrl+C` copy command, and `Esc` clear search.
- Red high-risk rows and green suggested-cleanup rows with non-color text labels plus reasons in `--list` JSON.
- Safer confirmation dialogs for disabling and deleting startup entries.
- Reworked Add/Edit Startup dialog with explicit Normal vs Quiet Tray startup choices plus clipboard path paste for fast entry creation.

## Important limitation

Windows cannot generically force every third-party app into that app's own native tray icon. Tray-wrapper mode starts the target minimized and provides a controller tray icon. Apps that intentionally force their own UI can still show a window.

## Prerequisites

- Windows 11 or Windows 10.
- Windows PowerShell 5.1.
- .NET Framework compiler available at one of:
  - `C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe`
  - `C:\Windows\Microsoft.NET\Framework\v4.0.30319\csc.exe`

## Build

```powershell
& 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\scripts\build.ps1'
```

Build output:

```text
F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe
```

The build embeds:

```text
assets\MichStartupMaster.ico
```

into the EXE using `csc.exe /win32icon`.

## Test / verification

```powershell
& 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\scripts\test.ps1'
```

The test script verifies:

1. The executable starts in smoke mode.
2. `--list` returns many startup entries, not just one.
3. Known startup items are present: `Autorun_current_ahk`, `AIMemoryBoost`, `FullScreenSnip`, and `TVStartupCheck`.
4. Service and driver startup categories are present.
5. Malwarebytes startup entries are visible when installed: `Malwarebytes Service`, `MBAMChameleon`, and `MbamElam`.
6. A disposable auto-start service can be disabled and restored through the app.
7. Every row has a non-empty human-readable `appName`.
8. At least one boot/system/auto driver is marked high-risk while a normal user app is not falsely marked high-risk.
9. Green cleanup suggestions exist on this machine, but no high-risk row or normal `FullScreenSnip` app row is falsely marked as cleanup.
10. Quiet popup protection restores a tampered startup task back to `--tray-run ...` or `--start-in-tray`.
11. The EXE icon can be extracted by Windows.
12. A tray-wrapper test logon task can be created.
13. A normal-mode test logon task can be created.
14. Both tasks contain a `LogonTrigger` and no `<Delay>` element.
15. Tray-mode XML uses `MichStartupMaster.exe --tray-run ...` or `--start-in-tray`.
16. Normal-mode XML runs the target executable directly.
17. Both regression tasks are removed and verified missing.
18. The exact PiperVoicePaste executable at `F:\study\AI_ML\AI_and_Machine_Learning\Artificial_Intelligence\Speech\Windows\Dictation\Tray\PiperVoicePaste\PiperVoicePaste.exe` can be added in both tray and normal modes.
19. `.ps1` and `.cmd` startup targets are registered through the correct Windows host executable instead of relying on fragile raw task actions.

## Open on monitor 2

To avoid disturbing monitor 1, use:

```powershell
& 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\scripts\open-on-monitor2.ps1'
```

This script stops only previous Mich Startup Master instances, opens the app, and moves it to the non-primary monitor.

## Project layout

- `src/MichStartupMaster.cs` — C# WinForms source code.
- `assets/MichStartupMaster.ico` — native app/taskbar icon.
- `build/MichStartupMaster.exe` — compiled runnable app.
- `scripts/build.ps1` — reproducible Windows build script.
- `scripts/test.ps1` — smoke/regression test script.
- `scripts/open-on-monitor2.ps1` — opens the GUI on the second monitor.
- `scripts/capture-monitor2-proof.ps1` — captures second-monitor proof screenshot.
- `artifacts/proof/` — screenshots captured during verification.
- `artifacts/runtime-output/` — runtime verification output from tests.

## Troubleshooting

- If the taskbar still shows an old icon after pinning, unpin/re-pin the rebuilt EXE or restart Explorer; Windows caches pinned shortcut icons aggressively.
- If only one startup item appears, run `scripts/test.ps1`; the fixed build should report the current machine's full startup inventory.
- If adding/disabling a system-owned scheduled task fails, run the app elevated or choose a user-owned startup entry.
- If disabling a machine-wide service or driver fails, run the app elevated; Windows requires admin rights for those controls.
- If the app cannot compile, confirm .NET Framework `csc.exe` exists at the prerequisite paths.
- If an app added in tray-wrapper mode still opens a window, that target application likely forces its own UI; try its own command-line arguments for minimized/tray behavior if it supports them.
