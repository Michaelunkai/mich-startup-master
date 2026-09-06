# Maintenance

Two companion scripts keep Mich Startup Master self-contained on a budget.
They live in the repository root next to the main workflow scripts and do not
interfere with `scripts\build.ps1`, `scripts\test.ps1`, or the installer.

## Free disk space instantly

`Clean.ps1` deletes the regenerable build/deploy artifacts that
`scripts\build.ps1` recreates on its next deployment:

| Target | Contents |
|---|---|
| `artifacts\build-staging\` | historical staged builds (tens of GB) |
| `artifacts\runtime-output\` | deployment transaction history |
| `artifacts\pre-overhaul-*\` | pre-overhaul snapshots |
| `artifacts\proof\*.png` | proof screenshots (gitignored) |

The live installed app (`%LOCALAPPDATA%\Programs\MichStartupMaster`), user
state (`%LOCALAPPDATA%\MichStartupMaster`), and every tracked source file are
never touched, so cleanup is safe while the app is running. Dry-run first with
`-WhatIf`.

```powershell
powershell.exe -ExecutionPolicy Bypass -File .\Clean.ps1 -WhatIf   # preview
powershell.exe -ExecutionPolicy Bypass -File .\Clean.ps1           # free space
```

## Restore and run everything

`remake.ps1` invokes the project's own canonical guarded deployment
(`scripts\build.ps1`), which stages a fresh validated build, swaps the live
app at `%LOCALAPPDATA%\Programs\MichStartupMaster`, gracefully stops the old
process, and relaunches the new executable. The script then verifies the app
process is running and reports the agent task state. Use `-StageOnly` to
stage and validate without touching the running installation.

```powershell
powershell.exe -ExecutionPolicy Bypass -File .\remake.ps1           # deploy + run
powershell.exe -ExecutionPolicy Bypass -File .\remake.ps1 -StageOnly # validate only
```

The build's own outcome marker (`DEPLOYED_AND_VERIFIED`) is authoritative;
its full log is written to `remake-build.log`.

## Receipts

Both scripts write a short machine-readable receipt next to them
(`clean.receipt`, `remake.receipt`) recording the run time, bytes freed or
regenerated, and the verification verdict. These are local-only artifacts.
