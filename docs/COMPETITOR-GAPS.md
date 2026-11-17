# Competitor gaps (ranked)

Sources: Autoruns (https://learn.microsoft.com/en-us/sysinternals/downloads/autoruns), Task Manager startup impact (https://www.winhelponline.com/blog/task-manager-startup-impact-calculated-bootckcl/), WinPatrol delayed start (https://www.sysnative.com/forums/threads/startup-programs-what-to-disable.8432/), Autoruns column overview (https://learn.secbyte.org/tools/autoruns-xyie).

Ranked by value for a startup controller; "A"=Autoruns, "W"=WinPatrol, "T"=Task Manager.

1. Startup impact per item (None/Low/Medium/High from boot trace) - T. Plan: box 19.
2. Full Autoruns category coverage: Run/RunOnce (HKLM/HKCU/WOW6432), Startup folders, Scheduled Tasks, Services, Drivers, Winlogon (Shell/Userinit), AppInit_DLLs, IFEO debugger, Explorer shell extensions, BHOs, Known DLLs - A. Plan: box 20.
3. Digital-signature verification + publisher per entry, flag unsigned - A.
4. Hide Microsoft/Windows entries filter - A.
5. VirusTotal hash lookup for image path - A.
6. Delayed start (run item N seconds after logon) - W, Startup Delayer.
7. "Jump to" registry key / file location - A (partly present: --open-location).
8. Compare against saved baseline / highlight new entries - A, W (present: --detect-new).
9. Missing-file / broken-entry detection (target not found) in red - A.
10. Offline / other-user profile analysis - A.
11. Timestamp (image file time) column and sort - A.
12. Search/filter across all columns with category tabs - A.
13. Export to CSV/XML and snapshot diff - A (present: TSV export).
14. Per-item description / "what is this" guidance - W, CCleaner.
15. Undo / change history for every enable-disable - W.

Execution order: 1, 2, then UI polish (boxes 19, 20, 21).
