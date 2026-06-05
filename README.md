# Mich Startup Master

Mich Startup Master is a Windows-native startup-control application. It gives you one polished place to see and manage what runs when Windows starts or when you log in.

This version fixes the taskbar/pinned-app icon by embedding a real native `.ico` into the compiled EXE, upgrades the UI into a friendlier dashboard-style control room, and verifies both startup-add modes: normal launch and quiet tray-wrapper launch.

## Runnable app

```powershell
& 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\build\MichStartupMaster.exe'
```

## What it does

- Shows Windows startup entries from `Win32_StartupCommand`.
- Shows Registry Run entries from HKCU and HKLM.
- Shows user/common Startup-folder entries.
- Shows enabled and disabled logon Scheduled Tasks by inspecting real `Get-ScheduledTask` trigger objects instead of fragile localized `schtasks` schedule text.
- Lets you disable/enable supported startup entries.
- Lets you add any `.exe` to logon startup with no Task Scheduler delay.
- Lets every added app choose one of two startup modes:
  - **Start normally** — runs the selected executable directly at Windows logon.
  - **Start quietly in tray mode** — starts through Mich Startup Master’s GUI-subsystem tray wrapper, launches minimized, avoids terminal popups, and keeps a controller tray icon.
- The Add dialog explicitly offers both choices for every executable you add: **Start normally** or **Start quietly in tray mode**.
- Includes a real embedded application icon for the EXE, window, tray icon, and taskbar/pinned shortcut path.
- Includes a tray icon for Mich Startup Master itself; closing the window hides it to the tray instead of killing it.

## UI upgrades

The main window is now a dashboard-style control room:

- Embedded branded app icon in the title bar, tray, and EXE/taskbar resource.
- Large friendly header with plain-language explanation.
- Metric cards for visible, enabled, disabled, review, and managed items.
- Cleaner toolbar with search, clear, add, refresh, enable/disable, and managed-delete controls.
- More readable startup table with status/trust emphasis and approachable helper copy.
- Safer confirmation dialogs for disabling and deleting startup entries.
- Reworked Add Startup dialog with explicit Normal vs Quiet Tray startup choices.

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
4. The EXE icon can be extracted by Windows.
5. A tray-wrapper test logon task can be created.
6. A normal-mode test logon task can be created.
7. Both tasks contain a `LogonTrigger` and no `<Delay>` element.
8. Tray-mode XML uses `MichStartupMaster.exe --tray-run ...`.
9. Normal-mode XML runs the target executable directly.
10. Both regression tasks are removed and verified missing.

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
- If the app cannot compile, confirm .NET Framework `csc.exe` exists at the prerequisite paths.
- If an app added in tray-wrapper mode still opens a window, that target application likely forces its own UI; try its own command-line arguments for minimized/tray behavior if it supports them.
