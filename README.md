# Mich Startup Master

Mich Startup Master is a Windows-native startup-control application. It gives you one polished place to see and manage what runs when Windows starts or when you log in.

The app was created after the original Startup Manager build showed only one startup item. This organized version contains the fixed implementation that discovers the full Windows startup surface used on this PC.

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
- Lets you add an executable to logon startup with no Task Scheduler delay.
- Supports normal startup mode and a tray-wrapper mode that launches the target minimized and keeps a controller tray icon.
- Includes a tray icon for Mich Startup Master itself; closing the window hides it to the tray instead of killing it.

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

## Test / verification

```powershell
& 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\scripts\test.ps1'
```

The test script verifies:

1. The executable starts in smoke mode.
2. `--list` returns many startup entries, not just one.
3. Known startup items are present: `Autorun_current_ahk`, `AIMemoryBoost`, `FullScreenSnip`, and `TVStartupCheck`.
4. A managed test logon task can be created.
5. The task XML contains a `LogonTrigger` and no `<Delay>` element.
6. The managed test task can be removed and is actually gone.

## Open on monitor 2

To avoid disturbing monitor 1, use:

```powershell
& 'F:\study\Windows\Applications\Desktop\Utilities\System\Startup\Managers\mich-startup-master\scripts\open-on-monitor2.ps1'
```

This script stops only previous Mich Startup Master instances, opens the app, and moves it to the non-primary monitor.

## Project layout

- `src/MichStartupMaster.cs` — C# WinForms source code.
- `build/MichStartupMaster.exe` — compiled runnable app.
- `scripts/build.ps1` — reproducible Windows build script.
- `scripts/test.ps1` — smoke/regression test script.
- `scripts/open-on-monitor2.ps1` — opens the GUI on the second monitor.
- `artifacts/proof/` — screenshots captured during verification.
- `artifacts/runtime-output/` — moved temporary verification scripts/output from the completed Telegram mission.

## Troubleshooting

- If only one startup item appears, run `scripts/test.ps1`; the fixed build should report a list count around the current machine's full startup inventory.
- If adding/disabling a system-owned scheduled task fails, run the app elevated or choose a user-owned startup entry.
- If the app cannot compile, confirm .NET Framework `csc.exe` exists at the prerequisite paths.
- If an app added in tray-wrapper mode still opens a window, that target application likely forces its own UI; try its own command-line arguments for minimized/tray behavior if it supports them.
