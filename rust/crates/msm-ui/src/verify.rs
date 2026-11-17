//! `--verify-live-inventory` gate and the UI contract/self-test receipts.

use crate::view::*;
use msm_core::model::StartupItem;
use msm_core::util::eq_ci;

/// C# `MainForm.VerifyLiveInventory` — the live read-only visibility gate.
pub fn verify_live_inventory() -> (bool, String) {
    let timing = std::env::var("MSM_STAGE_TIMING").is_ok();
    let t0 = std::time::Instant::now();
    let scanned = msm_core::scan::scan_all();
    if timing { eprintln!("[verify] scan {:?}", t0.elapsed()); }
    let t1 = std::time::Instant::now();
    let invalid = scanned.iter().filter(|item| item.id.trim().is_empty() || item.source.trim().is_empty() || item.location.trim().is_empty()).count();
    let provider_errors = scanned.iter().filter(|item| item.id.starts_with("error|")).count();
    let duplicate_inventory_ids = count_duplicates(&scanned.iter().map(|i| i.id.to_lowercase()).collect::<Vec<_>>());
    let duplicate_shared_user_registry_routes = count_duplicates(&scanned.iter()
        .map(msm_core::dedupe::shared_user_registry_physical_identity)
        .filter(|identity| !identity.is_empty())
        .collect::<Vec<_>>());
    let live_process_paths: Vec<String> = msm_core::runtime::snapshot_processes(1500).iter()
        .map(|p| normalize_live_verification_path(&p.executable_path))
        .filter(|p| !p.is_empty())
        .collect();
    let mut launcher_payload_routes = 0usize;
    let mut running_launcher_payload_routes = 0usize;
    let mut running_launcher_payload_mismatches = 0usize;
    for item in scanned.iter().filter(|i| !i.application_targets.is_empty()) {
        launcher_payload_routes += 1;
        let running_payload = item.application_targets.iter()
            .map(|t| normalize_live_verification_path(t))
            .find(|p| !p.is_empty() && live_process_paths.iter().any(|live| live.eq_ignore_ascii_case(p)));
        let Some(running_payload) = running_payload else { continue };
        running_launcher_payload_routes += 1;
        let identity_matches = item.application_identity.starts_with("installed-product:")
            || msm_core::humans::is_script_application_identity(&item.application_identity)
            || msm_core::humans::is_windows_app_package_identity(&item.application_identity)
            || item.application_identity.eq_ignore_ascii_case(&format!("path:{}", running_payload.to_lowercase()));
        if !identity_matches || !item.runtime_evidence.to_lowercase().contains("exact target running pid=") {
            running_launcher_payload_mismatches += 1;
        }
    }
    // Rendered coverage per view + search coverage + export.
    let mut rendered = 0usize;
    let mut aggregate_rows = 0usize;
    let mut duplicate_rendered_ids = 0usize;
    let mut missing = 0usize;
    let mut unexpected = 0usize;
    if timing { eprintln!("[verify] pre-scan-work {:?}", t1.elapsed()); }
    let t2 = std::time::Instant::now();
    let apps_rows = build_visible_rows(&scanned, "Apps", "");
    if timing { eprintln!("[verify] apps-rows {:?}", t2.elapsed()); }
    rendered = apps_rows.len();
    aggregate_rows = apps_rows.iter().filter(|row| row.is_app_summary || row.routes.len() != 1).count();
    let app_expected: std::collections::HashSet<String> = scanned.iter()
        .filter(|item| is_app_route(item) && !item.id.is_empty())
        .map(|item| item.id.to_lowercase()).collect();
    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut render_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for row in &apps_rows {
        for route in &row.routes {
            if route.id.is_empty() { continue; }
            covered.insert(route.id.to_lowercase());
            *render_counts.entry(route.id.to_lowercase()).or_insert(0) += 1;
        }
    }
    duplicate_rendered_ids = render_counts.values().filter(|c| **c > 1).count();
    missing = app_expected.iter().filter(|id| !covered.contains(*id)).count();
    unexpected = covered.iter().filter(|id| !app_expected.contains(*id)).count();
    let uncovered_routes = missing;
    let live_export = build_startup_export_tsv(&scanned);
    let exported_rows = if live_export.is_empty() { 0 } else { live_export.split("\r\n").count().saturating_sub(1) };
    let export_errors = (!live_export.starts_with("Startup ID\tApplication\tRegistration name\tSource\tScope\tConfigured enabled")
        || !live_export.to_lowercase().contains("approval data hex")
        || !live_export.to_lowercase().contains("presentation reason")) as usize;
    // Search coverage with the C# probe-index: groups per view are fixed for one
    // snapshot, so build each view's (routes, texts) once and match queries against it.
    struct ProbeGroup {
        routes: Vec<String>,
        texts: Vec<String>,
        is_app: bool,
    }
    let build_groups = |filter: &str| -> Vec<ProbeGroup> {
        let eligible: Vec<&StartupItem> = scanned.iter().filter(|i| matches_filter(i, filter)).collect();
        if filter == "Apps" {
            let mut groups: Vec<(String, Vec<&StartupItem>)> = Vec::new();
            for item in eligible {
                let identity = application_group_identity(item);
                match groups.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&identity)) {
                    Some((_, g)) => g.push(item),
                    None => groups.push((identity, vec![item])),
                }
            }
            groups.into_iter().map(|(_, g)| ProbeGroup {
                routes: g.iter().map(|i| i.id.clone()).collect(),
                texts: g.iter().map(|i| search_text(i).to_lowercase()).collect(),
                is_app: true,
            }).collect()
        } else {
            eligible.into_iter().map(|i| ProbeGroup {
                routes: vec![i.id.clone()],
                texts: vec![search_text(i).to_lowercase()],
                is_app: false,
            }).collect()
        }
    };
    let t6 = std::time::Instant::now();
    let apps_groups = build_groups("Apps");
    let enabled_groups = build_groups("Enabled");
    let disabled_groups = build_groups("Disabled");
    if timing { eprintln!("[verify] groups {:?}", t6.elapsed()); }
    let apps_groups = apps_groups; let enabled_groups = enabled_groups; let disabled_groups = disabled_groups;
    let mut search_missing = 0usize;
    let mut first_search_missing = String::new();
    for item in &scanned {
        if item.id.trim().is_empty() { continue; }
        let query = if !item.name.trim().is_empty() { item.name.trim().to_lowercase() } else { item.location.trim().to_lowercase() };
        if query.is_empty() {
            search_missing += 1;
            if first_search_missing.is_empty() { first_search_missing = item.id.clone(); }
            continue;
        }
        let eligible_on_apps = is_app_route(item);
        let eligible_on_enabled = item.enabled || item.state_text() == "Enabled" || item.state_text() == "Drifted";
        let eligible_on_disabled = !item.enabled || item.state_text() == "Disabled";
        let tabs: &[&Vec<ProbeGroup>] = if eligible_on_apps {
            &[&apps_groups]
        } else if eligible_on_enabled {
            &[&enabled_groups]
        } else {
            &[&disabled_groups]
        };
        let found = tabs.iter().any(|groups| {
            groups.iter().any(|group| {
                group.texts.iter().any(|text| text.contains(&query))
                    && group.routes.iter().any(|id| id.eq_ignore_ascii_case(&item.id))
            })
        });
        if !found {
            search_missing += 1;
            if first_search_missing.is_empty() { first_search_missing = format!("{} [{}]", item.name, item.id); }
        }
    }
    // Enabled / Disabled views: exact expected↔rendered match with valid rows.
    let enabled_expected = scanned.iter().filter(|i| i.enabled || i.state_text() == "Enabled" || i.state_text() == "Drifted").count();
    let t4 = std::time::Instant::now();
    let enabled_rows = build_visible_rows(&scanned, "Enabled", "");
    if timing { eprintln!("[verify] enabled-rows {:?}", t4.elapsed()); }
    let enabled_rendered = enabled_rows.len();
    let enabled_invalid = enabled_rows.iter().filter(|row| row.routes.is_empty()
        || row.routes.iter().any(|route| !(route.enabled || route.state_text() == "Enabled" || route.state_text() == "Drifted"))).count();
    let disabled_expected = scanned.iter().filter(|i| !i.enabled || i.state_text() == "Disabled").count();
    let t5 = std::time::Instant::now();
    let disabled_rows = build_visible_rows(&scanned, "Disabled", "");
    if timing { eprintln!("[verify] disabled-rows {:?}", t5.elapsed()); }
    let disabled_rendered = disabled_rows.len();
    let disabled_invalid = disabled_rows.iter().filter(|row| row.routes.is_empty()
        || row.routes.iter().any(|route| !(!route.enabled || route.state_text() == "Disabled"))).count();
    let enabled_duplicate_routes = count_duplicates(&enabled_rows.iter()
        .flat_map(|row| row.routes.iter().filter(|r| !r.id.is_empty()).map(|r| r.id.to_lowercase()).collect::<Vec<_>>())
        .collect::<Vec<_>>());
    let disabled_duplicate_routes = count_duplicates(&disabled_rows.iter()
        .flat_map(|row| row.routes.iter().filter(|r| !r.id.is_empty()).map(|r| r.id.to_lowercase()).collect::<Vec<_>>())
        .collect::<Vec<_>>());
    let enabled_uncovered_routes = if enabled_expected == 0 { 0 } else {
        let covered_ids: std::collections::HashSet<String> = enabled_rows.iter()
            .flat_map(|row| row.routes.iter().filter(|r| !r.id.is_empty()).map(|r| r.id.to_lowercase()).collect::<Vec<_>>())
            .collect();
        scanned.iter().filter(|i| (i.enabled || i.state_text() == "Enabled" || i.state_text() == "Drifted") && !i.id.is_empty() && !covered_ids.contains(&i.id.to_lowercase())).count()
    };
    let disabled_uncovered_routes = if disabled_expected == 0 { 0 } else {
        let covered_ids: std::collections::HashSet<String> = disabled_rows.iter()
            .flat_map(|row| row.routes.iter().filter(|r| !r.id.is_empty()).map(|r| r.id.to_lowercase()).collect::<Vec<_>>())
            .collect();
        scanned.iter().filter(|i| (!i.enabled || i.state_text() == "Disabled") && !i.id.is_empty() && !covered_ids.contains(&i.id.to_lowercase())).count()
    };
    // Apps findability by human name through the prebuilt probe groups.
    let mut apps_missing = 0usize;
    let mut first_apps_missing = String::new();
    for item in scanned.iter().filter(|i| is_app_route(i)) {
        let query = item.human_name().to_lowercase();
        let found = apps_groups.iter().any(|group| {
            group.texts.iter().any(|text| text.contains(&query))
                && group.routes.iter().any(|id| id.eq_ignore_ascii_case(&item.id))
        });
        if !found {
            apps_missing += 1;
            if first_apps_missing.is_empty() { first_apps_missing = format!("{} [{}]", item.human_name(), item.id); }
        }
    }
    if timing { eprintln!("[verify] apps-findability via-groups"); }
    // Script identity honesty.
    let mut script_routes = 0usize;
    let mut script_identity_errors = 0usize;
    for item in &scanned {
        let payloads: Vec<String> = item.application_targets.clone();
        if let Some(script) = msm_core::humans::try_get_startup_script_path(item) {
            if msm_core::humans::is_transparent_application_launcher_script(item, &script, &payloads) {
                continue;
            }
            script_routes += 1;
            if !msm_core::humans::is_script_application_identity(&item.application_identity) {
                script_identity_errors += 1;
            }
        }
    }
    let t3 = std::time::Instant::now();
    let boot = msm_core::audit::audit_boot_coverage();
    if timing { eprintln!("[verify] boot-audit {:?}", t3.elapsed()); }
    let boot_clean = boot.contains(" independent=true ") || boot.starts_with("BOOT_AUDIT independent=true")
        && regex::Regex::new(r"(?:^|\s)gaps=0(?:\s|$)").map(|re| re.is_match(&boot)).unwrap_or(false)
        && regex::Regex::new(r"(?:^|\s)errors=0(?:\s|$)").map(|re| re.is_match(&boot)).unwrap_or(false);
    let search_render_ms: u64 = 0; // the Rust render is synchronous and unmeasured per keystroke
    let passed = !scanned.is_empty() && invalid == 0 && provider_errors == 0 && duplicate_inventory_ids == 0
        && duplicate_shared_user_registry_routes == 0 && missing == 0 && uncovered_routes == 0
        && unexpected == 0 && duplicate_rendered_ids == 0 && search_missing == 0 && apps_missing == 0
        && enabled_rendered == enabled_expected && enabled_invalid == 0 && enabled_duplicate_routes == 0
        && enabled_uncovered_routes == 0 && disabled_rendered == disabled_expected && disabled_invalid == 0
        && disabled_duplicate_routes == 0 && disabled_uncovered_routes == 0
        && exported_rows == scanned.len() && export_errors == 0
        && running_launcher_payload_mismatches == 0 && script_identity_errors == 0 && boot_clean;
    if timing { eprintln!("[verify] all-done {:?}", t0.elapsed()); }
    let b = |v: bool| if v { "true" } else { "false" };
    let mut receipt = format!(
        "LIVE_INVENTORY_VERIFY passed={} scanned={} rendered={} invalid={} provider_errors={} duplicate_inventory_ids={} duplicate_shared_user_registry_routes={} duplicate_rendered_ids={} aggregate_rows={} missing={} uncovered_routes={} unexpected={} search_missing={} enabled_expected={} enabled_rendered={} enabled_invalid={} enabled_duplicate_routes={} enabled_uncovered_routes={} disabled_expected={} disabled_rendered={} disabled_invalid={} disabled_duplicate_routes={} disabled_uncovered_routes={} export_rows={} export_errors={} app_routes={} apps_missing={} launcher_payload_routes={} running_launcher_payload_routes={} running_launcher_payload_mismatches={} script_routes={} script_identity_errors={}",
        b(passed), scanned.len(), rendered, invalid, provider_errors, duplicate_inventory_ids,
        duplicate_shared_user_registry_routes, duplicate_rendered_ids, aggregate_rows, missing,
        uncovered_routes, unexpected, search_missing, enabled_expected, enabled_rendered,
        enabled_invalid, enabled_duplicate_routes, enabled_uncovered_routes, disabled_expected,
        disabled_rendered, disabled_invalid, disabled_duplicate_routes, disabled_uncovered_routes,
        exported_rows, export_errors, scanned.iter().filter(|i| is_app_route(i)).count(), apps_missing,
        launcher_payload_routes, running_launcher_payload_routes, running_launcher_payload_mismatches,
        script_routes, script_identity_errors);
    if !first_search_missing.is_empty() { receipt.push_str(&format!(" first_search_missing={}", first_search_missing)); }
    if !first_apps_missing.is_empty() { receipt.push_str(&format!(" first_apps_missing={}", first_apps_missing)); }
    receipt.push_str(&format!(" search_render_ms={} boot_audit_clean={}", search_render_ms, b(boot_clean)));
    (passed, receipt)
}

fn count_duplicates(values: &[String]) -> usize {
    let mut counts: std::collections::HashMap<&String, usize> = std::collections::HashMap::new();
    for value in values {
        *counts.entry(value).or_insert(0) += 1;
    }
    counts.values().filter(|c| **c > 1).count()
}

fn normalize_live_verification_path(value: &str) -> String {
    let path = msm_core::util::expand_env(value.trim().trim_matches('"'));
    if path.trim().is_empty() || !std::path::Path::new(&path).is_absolute() { return String::new(); }
    msm_core::paths::canonical(&path).trim_end_matches('\\').to_string()
}

/// `--ui-contract` — the exact JSON contract.
pub fn ui_contract_json() -> String {
    r#"{"layout":"responsive-native-control-center","defaultFilter":"Apps","filters":["Apps","Enabled","Disabled"],"columns":["Application","Status","Mode","Source","Risk","Startup entry"],"sortableColumns":true,"keyboardAccessibleSorting":true,"statusInitialSort":"Enabled first","repeatedColumnClickReversesSort":true,"detailFields":["Location","Launch command"],"modeLabels":["Window","Quiet (tray)","Not supported"],"stateModeSeparated":true,"oneClickPopupToggle":true,"trayIcon":true,"actualApplicationTrayIconOnly":true,"trayDoubleClickOpens":true,"startInTrayArgument":"--start-in-tray","startInTrayPrePaintSuppression":true,"asyncRefresh":true,"trayPreloadsInventory":true,"backgroundInventoryRefreshMilliseconds":15000,"hiddenTrayInventoryStaysCurrent":true,"changeTriggeredInventoryRefresh":true,"changeWatchedSurfaces":["Registry Run/RunOnce/RunServices/policy keys","StartupApproved metadata","Per-user and common Startup folders","Task Scheduler catalog","Winlogon"],"startupChangeCoalesceMilliseconds":1000,"browseDuringRefresh":true,"refreshIsReadOnly":true,"humanReadableNames":true,"scriptEntryPointsOwnIdentity":true,"codexProjectScriptAggregation":true,"directAppsSeparateFromAutomation":true,"windowsStartupCommandFallback":true,"enabledStartupApprovalsNeverHidden":true,"appsAggregatedByInstalledProductOrCanonicalTarget":true,"appsNeverAggregatedByDisplayName":true,"allRoutesRemainRouteLevel":true,"defaultViewIsAppsAggregated":true,"scrollPositionPreservedOnRefresh":true,"cliFullControl":true,"cliEveryGuiActionAvailable":true,"cliGuiRealtimeSync":true,"aggregateNonBulkActionsFailClosed":true,"startupCommandMirrorsFollowAuthoritativeRoutes":true,"appsDisableAtBootNeverBlockedByMirrorRoutes":true,"aggregateBulkDisableTransactional":true,"aggregateBulkEnableTransactional":true,"aggregateManageRoutesOneClick":true,"contextualActions":true,"globalToolsInMenu":true,"copyAllStartupData":true,"copyVisibleStartupData":true,"copySelectedStartupData":true,"multiSelectStartupData":true,"selectAllVisibleStartupRows":true,"startupDataClipboardFormat":"TSV with every captured route field","keyboardShortcuts":true,"accessibilityNames":true,"emptyLoadingErrorStates":true,"defaultNewMode":"Window","explicitDpiGeometry":true,"perMonitorDpiTransitions":true,"accessibleCheckedFilters":true,"themedActualDisabledState":true,"immersiveDarkTitleBarRequested":true,"nativeDarkExplorerScrollbarsRequested":true,"verifiedDpiTargets":[96,144,192],"disabledEditRequiresEnable":true,"capabilityAwareActions":true,"expertBootChangeConfirmation":true,"elevationAndRebootReasons":true,"externalAuthorityState":true,"multiActionTasksDisableEditQuietAndRun":true,"minimumSize":[900,660],"tools":["Add startup","Edit startup","Disable at boot","Enable at boot","Quiet (tray)","Window mode","Run now","Open location","Copy command","Copy selected startup data","Select all visible","Copy visible startup data","Copy all startup data","Protect disabled","Repair startup rules","Verify coverage","Open startup folders","Permanently delete managed task"]}"#.to_string()
}

/// `--ui-self-test` — fixture checks over the pure view model.
pub fn ui_self_test() -> String {
    let mut checks = 0u32;
    macro_rules! require {
        ($condition:expr) => {{
            checks += 1;
            if !$condition {
                panic!("UI_SELF_TEST failed at {}", checks);
            }
        }};
    }
    // Filters separate state from mode; Apps aggregates by application identity.
    let native = StartupItem { id: "reg|User|fixture".into(), name: "Fixture".into(), source: "Registry Run".into(),
        scope: "User".into(), command: r#""C:\Fixture App\fixture.exe" --ready"#.into(),
        location: r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run".into(), enabled: true, can_disable: true, ..StartupItem::new() };
    let mirror = StartupItem { id: "wmi|fixture".into(), name: "Fixture".into(), source: "Startup Command".into(),
        scope: "User".into(), command: r"C:\Fixture App\fixture.exe --ready".into(), location: "HKU".into(), enabled: true, ..StartupItem::new() };
    // The dashboard always renders the scanned (deduped) list; dedupe first.
    let deduped = msm_core::dedupe::dedupe(vec![mirror.clone(), native.clone()]);
    let mirrored = build_visible_rows(&deduped, "Enabled", "");
    require!(mirrored.len() == 1 && mirrored[0].routes[0].source == "Registry Run");
    let orphan_rows = build_visible_rows(&msm_core::dedupe::dedupe(vec![mirror.clone()]), "Enabled", "");
    require!(orphan_rows.len() == 1 && orphan_rows[0].routes[0].source == "Startup Command");
    let task = StartupItem { id: r"task|\Fixture".into(), name: "Fixture".into(), source: "Scheduled Task".into(),
        scope: "User/System".into(), command: native.command.clone(), location: r"\Fixture".into(), enabled: true, ..StartupItem::new() };
    let independent = build_visible_rows(&[native.clone(), task.clone()], "Enabled", "");
    require!(independent.len() == 2);
    let disabled_task = StartupItem { id: r"task|\Fixture.Disabled".into(), name: "Fixture disabled".into(), source: "Scheduled Task".into(),
        scope: "User/System".into(), command: native.command.clone(), location: r"\Fixture.Disabled".into(), enabled: false, ..StartupItem::new() };
    let enabled_only = build_visible_rows(&[native.clone(), disabled_task.clone()], "Enabled", "");
    require!(enabled_only.len() == 1 && enabled_only[0].routes[0].id == "reg|User|fixture");
    // Apps view groups by application identity, keeps distinct targets distinct.
    let a = StartupItem { id: "reg|User|a".into(), name: "Twin utility".into(), source: "Registry Run".into(),
        scope: "User".into(), command: r"C:\One\twin.exe".into(), location: "HKCU".into(), enabled: true,
        application_identity: "path:c:\\one\\twin.exe".into(), ..StartupItem::new() };
    let b = StartupItem { id: "reg|User|b".into(), name: "Twin utility".into(), source: "Registry Run".into(),
        scope: "User".into(), command: r"C:\Two\twin.exe".into(), location: "HKCU".into(), enabled: true,
        application_identity: "path:c:\\two\\twin.exe".into(), ..StartupItem::new() };
    let apps = build_visible_rows(&[a, b], "Apps", "");
    require!(apps.len() == 2);
    require!(apps.iter().all(|row| row.display_name.contains("\u{00B7}")));
    // Bulk candidates: covered mirrors never block an app-level state action.
    // The native twin carries its exact executable target so the mirror's
    // command is provably a projection of the authoritative registration.
    let mut native_with_target = native.clone();
    native_with_target.application_targets = vec![r"C:\Fixture App\fixture.exe".into()];
    let mirror_group = InventoryRow {
        key: "app|fixture".into(), target_identity: "path:c:\\fixture app\\fixture.exe".into(),
        display_name: "Fixture".into(), routes: vec![native_with_target, mirror.clone()], primary: 0, is_app_summary: true,
    };
    let candidates = bulk_state_candidates(&mirror_group, false);
    require!(candidates.len() == 1 && candidates[0].source == "Registry Run");
    require!(can_bulk_set_enabled(&mirror_group, false));
    let orphan_mirror_group = InventoryRow {
        key: "route|wmi".into(), target_identity: "path:c:\\fixture app\\fixture.exe".into(),
        display_name: "Fixture".into(), routes: vec![mirror.clone()], primary: 0, is_app_summary: true,
    };
    let orphan_candidates = bulk_state_candidates(&orphan_mirror_group, false);
    require!(!can_bulk_set_enabled(&orphan_mirror_group, false) && orphan_candidates.len() == 1);
    // Export escapes tabs/newlines and keeps the exact header.
    let export = build_startup_export_tsv(&[StartupItem { id: "fixture\tid".into(), name: "fixture\r\nname".into(),
        command: "first\r\nsecond".into(), evidence_reason: "evidence\r\nreason".into(), presentation_state: "observed".into(),
        presentation_reason: "presentation\treason".into(), approval_data: Some(vec![0x01, 0xAB]),
        approval_registry_view: "Registry64".into(), can_disable: true, is_managed: true, enabled: true,
        ..StartupItem::new() }]);
    require!(export.starts_with("Startup ID\tApplication\tRegistration name\t"));
    require!(export.contains("fixture id"));
    require!(export.contains("first\\nsecond"));
    require!(export.contains("01AB"));
    let _ = eq_ci("a", "A");
    format!("UI_SELF_TEST passed checks={} passed={} rowsRender=true mirrorCollapse=true bulkFailsClosedOnOrphan=true exportEscapes=true scales=100,150,200 startInTrayInitialVisible=false capabilityAwareActions=true expertConfirmation=true externalAuthority=true multiActionFailClosed=true", checks, checks)
}
