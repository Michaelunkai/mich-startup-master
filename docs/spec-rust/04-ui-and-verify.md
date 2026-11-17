# Spec 04 — UI model, verify gate, export TSV, suite contract

## Window: title "Mich Startup Master — Startup Control", 1360x820 default, min 900x660, tray NotifyIcon "Mich Startup Master", poll 15000ms.
## Search-toast in UI = status line only. New-item watcher toasts: tray balloon.

## Views/filters: "Apps" (default), "Enabled", "Disabled" (+hidden "All" w/ target drill-down)
- MatchesFilter: Apps→IsAppRoute; Enabled→Enabled||StateText∈{Enabled,Drifted}; Disabled→!Enabled||StateText==Disabled; All(+ApplicationGroupIdentity==target).
- Apps rows: group by ApplicationGroupIdentity (ApplicationIdentity if set else CanonicalTargetIdentity); primary = max RouteSeverity → IsManaged desc → HumanName; display disambiguation "HumanName  ·  parent\file" when repeated names.
- Route rows: one per item. Key route|Id.
- Search: SearchText = join(" ", HumanName, Name, Command, Source, Location, Status, AdviceReason); Apps row matches if ANY route matches.
- Default sort: NewestFirstSeenUtc desc, then HumanName, then TargetIdentity. Columns: Application/Status/Mode/Source/Risk/Startup entry (sortable, click reverses; Status rank: EffectiveEnabled(0)→…).
- StatusText: "● Enabled" / "● Enabled · mixed routes" / "○ Disabled" / "○ Disabled · running now" / "Drifted" / "Unknown" / "◐ Mixed".

## IsAppRoute (exact): not error|; HasConcreteUserApplicationTarget → true; "Windows Service" → ApplicationIdentity starts "installed-product:"; else Source NOT in {System Driver, Winlogon Autostart, Winlogon Notification, AppInit DLLs, Active Setup, Boot Execute, Image Hijack, Known DLL, Network Provider, Winsock Provider, Print Monitor, Print Provider, Security Provider, Terminal Server Startup, Icon Service Library, URL Search Hook, Service Dependency, Media Codec, WMI Event Consumer, Group Policy Script, Explorer Startup Extension, Explorer Shell Extension, Internet Explorer Add-on, AppCert DLLs, LSA Startup Package, Stale Startup Metadata}.

## Bulk semantics: BulkStateCandidates(row, state) = routes where Enabled != state, EXCLUDING source "Scheduled Task (Any Trigger)", "Service (Trigger Start)", and covered Startup Command mirrors (IsCoveredStartupCommandMirror: mirror NormBootKey(command) len>8 covered by another route's command (equal or prefix) or ApplicationTargets). CanBulkSetEnabled = candidates>0 && all CanDisable && no ExternalAuthority.

## CanonicalTargetIdentity: approval rows → "approval:"+id; tray payload → NormalizeTargetIdentity(target); ResolveLaunchTarget; TrySplitCommand; else "raw:"+collapsed-lower-command (or route:+Id).
NormalizeTargetIdentity: expand env, /→\, GetFullPath, trim '\', lower → "path:"+value.

## Export TSV (exact, --export + verify): header 35 cols:
Startup ID\tApplication\tRegistration name\tSource\tScope\tConfigured enabled\tVerified state\tStatus\tEvidence reason\tPresentation state\tPresentation reason\tStartup mode\tPopup enabled\tRisk\tCan disable\tManaged by Startup Master\tLocation\tLaunch command\tApplication identity\tApplication root\tApplication identity reason\tApplication targets\tRuntime evidence\tMutation capability\tMutation reason\tRequires expert confirmation\tRequires elevation\tRequires reboot\tExternal authority\tApproval root\tApproval path\tApproval name\tApproval data hex\tApproval registry view\tRegistry view
Rows: ordered HumanName then Id; \r\n joined; Tsv() = replace \t→" ", \r\n→"\\n", \r→"\\n", \n→"\\n"; TsvBytes = hex uppercase (BitConverter format XX-XX → no dash).

## VerifyLiveInventory receipt (must print `LIVE_INVENTORY_VERIFY passed=true ` + fields; <60s):
passed = scanned>0 & invalid=0 & provider_errors=0 & duplicate_inventory_ids=0 & duplicate_shared_user_registry_routes=0 & missing=0 & uncovered_routes=0 & unexpected=0 & duplicate_rendered_ids=0 & search_missing=0 & apps_missing=0 & enabled_rendered==enabled_expected & enabled_invalid=0 & enabled_duplicate_routes=0 & enabled_uncovered_routes=0 & disabled_rendered==disabled_expected & disabled_invalid=0 & disabled_duplicate_routes=0 & disabled_uncovered_routes=0 & export_rows==scanned & export_errors=0 & running_launcher_payload_mismatches=0 & logitech_visible & script_identity_errors=0 & game_auto_visible & qbittorrent_visible & boot_audit_clean.
Checks: per-tab render coverage via probe groups (same eligibility+query filtering as BuildVisibleRows w/o ordering); every route findable by Name (fallback Location) on an eligible tab; enabled/disable exact-match; app routes findable by HumanName on Apps; Logitech regression (search "logitech" + "lghub" find every logitech route, one row per group); GameAutoInstall (one app row, every identity probe incl. fitgirl/rune finds it); qBittorrent direct routes; search_render_ms timing; boot audit embedded.
Field order in receipt: passed scanned rendered invalid provider_errors duplicate_inventory_ids duplicate_shared_user_registry_routes duplicate_rendered_ids aggregate_rows missing uncovered_routes unexpected search_missing enabled_expected enabled_rendered enabled_invalid enabled_duplicate_routes enabled_uncovered_routes disabled_expected disabled_rendered disabled_invalid disabled_duplicate_routes disabled_uncovered_routes export_rows export_errors app_routes apps_missing launcher_payload_routes running_launcher_payload_routes running_launcher_payload_mismatches logitech_routes logitech_expected_app_rows logitech_name_rows logitech_lghub_rows logitech_missing logitech_visible script_routes script_identity_errors game_auto_routes game_auto_expected_app_rows game_auto_name_rows game_auto_fitgirl_rows game_auto_fitgirl_required game_auto_identity_probes game_auto_identity_rows game_auto_missing game_auto_visible qbittorrent_routes qbittorrent_expected_app_rows qbittorrent_rows qbittorrent_missing qbittorrent_visible [first_search_missing=..] [first_apps_missing=..] search_render_ms boot_audit_clean

## UiContractJson: exact JSON keys (camelCase) — filters ["Apps","Enabled","Disabled"], columns 6, backgroundInventoryRefreshMilliseconds=15000, startupChangeCoalesceMilliseconds=1000, minimumSize [900,660], tools list (17 items), etc. Suite asserts columns count 6, filters count 3 (+every GUI action in CLI).

## Suite (test.ps1, SAFE mode = 18 PASS):
static-app-isolation, pure-state-isolation (harness), ui-contract, ui-self-test (exit 0 + receipt), quiet-plan (OpenSpeedy native-tray / generic managed-proxy), quiet-policy, quiet-lineage, quiet-performance, startup-truth, inventory-self-test, managed-startup-dedupe, state-store-self-test (workers=12), agent-registration-self-test, release-gate-self-test, smoke (`SMOKE passed=true inventory=\d+ invalid=0 duplicate_ids=0 ` TRAILING SPACE), list (JSON parse; unique ids; OpenSpeedy live check: if enabled managed OpenSpeedy exists → command must be direct Speedy.exe --minimize-to-tray, no --tray-run/wscript), independent-audit (BOOT_AUDIT fields incl. independence_scope=authoritative-enumerators, gaps=0 errors=0, surfaces nonempty; TRAY_AUDIT apps>=1 running==apps findings=0 uncertain=0; exactly 2 lines total), icon (ExtractAssociatedIcon works).
Live-gate contracts (also in safe): --verify-live-inventory passed=true; --audit-boot gaps=0 errors=0 (-AllowFailure on exit code).
Isolation: product honors MSM_STATE_ROOT; must NOT touch real %LOCALAPPDATA%\MichStartupMaster during safe probes; MSM_KNOWN_STORE/MSM_FIRST_SEEN_STORE overrides must stay inside state root.

## Self-test receipts to reproduce (exit 0):
- STATE_STORE_SELF_TEST workers=12 atomic=ok recovery=ok
- TRUTH_SELF_TEST checks=N passed=N codex=unverified contradictions=drifted missing=unknown packaged=verified
- INVENTORY_SELF_TEST kind=fixtures checks=N passed=N surface_contracts=30 surface_names=... service_start_modes=0,1,2,3
- MANAGED_DEDUPE_SELF_TEST checks=3 passed=3 exactOne=true staleAliasRetired=true conflictsFailClosed=true
- AGENT_REGISTRATION_SELF_TEST[_OK] ... (booleans all=true; suite checks required fields present)
- RELEASE_GATE_SELF_TEST_OK ...
- UI_SELF_TEST ... (suite just echoes; must exit 0)
- quiet-plan JSON {strategy,target,arguments,userArguments,usesWrapper}
- quiet-policy-probe / quiet-lineage-self-test / quiet-performance-probe JSONs with passed=true + named booleans asserted by suite:
  quiet-policy: bootOnlySuppression trayReadyRelease nativeControllerExit runtimeFallback runtimeNativeWins chatGptNative veryLateAllowed explicitProxyCancel focusStealIgnored oneLaunch oneIcon manualRehideCount=0
  quiet-lineage: tracked=4100,4101,4102,4103 exactArgv callbackMultiHop snapshotCalls=1 externalProcessesStarted=0
  quiet-performance: steadySnapshots=0 steadyGlobalHooks=0 scopedHooks jobObserver disposed manualOpenImmediate

## build.ps1 (guarded deploy): stops prior modes via Stop-MichStartupMaster.ps1, stages exe, runs suite against stage, deploys to build\ transactionally, writes artifacts\runtime-output\deployment-active.json outcome DEPLOYED_AND_VERIFIED, relaunches via canonical task. Adapt for cargo build (keep same contract).
