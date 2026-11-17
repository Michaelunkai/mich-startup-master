## What changed in 3.0.0
- Complete rewrite of the entire app in Rust (7 crates), replacing the previous C#/WinForms stack.
- Single native 3.2 MB executable replaces the ~39 MB .NET build; no runtime or SDK required at runtime.
- Startup inventory compatible with 2.10.1: same 31+ surfaces, same state stores, same CLI receipts.
- One-click enable/disable with transactional rollback unchanged from prior versions.
- Real-time registry/folder/task-catalog watchers with 1-second coalesce instead of polling.
- Boot audit independent enumerators report gaps = 0 across every discovered surface.
- Full safe suite passes all 21 tests against the new binary.
- Faster full verify gate completes in seconds while proving the same search contract.
- Same deploy path (`build\MichStartupMaster.exe`); the logon task keeps working without change.
