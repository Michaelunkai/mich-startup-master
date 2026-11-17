# Mich Startup Master — Rust edition (3.0.0)

A complete native rewrite of the C#/.NET 10 WinForms app as a single static Rust binary
(≈3.2 MB, no .NET runtime). Same inventory guarantees, same state stores, same CLI.

## Build

```text
set PATH=C:\Users\Admin\.cargo\bin;%PATH%
cd rust
cargo build --release --target x86_64-pc-windows-msvc -p msm-app
```

Binary: `F:\backup\rust\target\x86_64-pc-windows-msvc\release\MichStartupMaster.exe`
(`CARGO_TARGET_DIR` env redirects the target dir). Requires MSVC Build Tools (installed).

## Crate map

| Crate | Owns |
|---|---|
| `msm-core` | StartupItem model, stable ids, registry engine (WOW64 views), every scan surface (Run family, Winlogon, Active Setup, AppInit, BootExecute/LSA, extensions, loaded hives, folders, scheduled tasks COM+XML, services/drivers, WMI, packaged tasks, StartupApproved overlay), dedupe/mirror rules, truth evidence, runtime process catalog, boot+tray audits, JSON receipts |
| `msm-state` | `StateStoreFile` (named-mutex + atomic write + .bak recovery), disabled/protected-disabled/protected-quiet/enabled stores (byte-compatible TSV), first-seen + known stores, mutation coordinator (deployment→mutation two-level mutex) |
| `msm-actions` | One-click disable/enable per route type with exact snapshots + rollback, bulk transactions, managed-task lifecycle, agent registration/verification, boot reconciliation |
| `msm-watch` | `StartupChangeMonitor`: RegNotifyChangeKeyValue + ReadDirectoryChangesW (Startup folders, Tasks catalog), 1s coalesce, 5s throttle |
| `msm-quiet` | Quiet plan (native-tray / self-tray / managed-proxy), `--tray-run` wrapper, policy/lineage/performance probe receipts |
| `msm-ui` | View model (Apps aggregation, filters, sorting), `--verify-live-inventory` gate, export TSV, UI contract + self-test |
| `msm-app` | `MichStartupMaster.exe`: CLI dispatch, console attach for gate scripts, native dark window + tray agent |

## Gates

```text
MichStartupMaster.exe --smoke                  -> SMOKE passed=true ...
MichStartupMaster.exe --verify-live-inventory  -> LIVE_INVENTORY_VERIFY passed=true ...
MichStartupMaster.exe --audit-boot             -> BOOT_AUDIT ... gaps=0 errors=0 + TRAY_AUDIT
powershell -NoProfile -File ..\scripts\test.ps1 -TestAppPath <exe>   -> SAFE_TESTS passed
```

## Preserved invariants

- **Live registration is authoritative** — stale disabled records retire against live routes; nothing appears twice.
- **Mirrors follow twins; authorities block** — covered Win32_StartupCommand mirrors never block or mutate.
- **Refresh is read-only** — repairs only via explicit actions or boot reconciliation.
- **State stores are byte-compatible** with the C# app (same TSV columns, CRLF, no BOM, `.bak` recovery).
- **Two-level mutation mutex** (`Local\MichStartupMaster.ReleaseDeployment` → `...ManagedStartupStartupMutation`).
- **Identical CLI receipts** — every suite-asserted line matches the C# output byte-for-byte.

Deploy runbook: `docs/spec-rust/05-quiet-and-deploy.md`; store-format audit: `docs/spec-rust/07-store-compat.md`.
