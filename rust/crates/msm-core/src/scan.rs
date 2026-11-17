//! ScanAll — the authoritative inventory pipeline (exact C# order and semantics).

use crate::model::StartupItem;
use crate::util::eq_ci;

pub const MANAGED_TASK_ROOT: &str = r"\MichStartupMaster\";

/// C# `ScanAll` — full pipeline in the exact order.
pub fn scan_all() -> Vec<StartupItem> {
    let timing = std::env::var("MSM_STAGE_TIMING").is_ok();
    let mut stamp = std::time::Instant::now();
    let mut stage = |name: &str, timing: bool, stamp: &mut std::time::Instant| {
        if timing {
            eprintln!("[stage] {} {:?} ({} rows so far)", name, stamp.elapsed(), 0);
            *stamp = std::time::Instant::now();
        }
    };
    let mut items: Vec<StartupItem> = Vec::new();
    crate::scan_registry::add_common_registry_startup(&mut items);
    stage("common-registry", timing, &mut stamp);
    crate::scan_registry::add_registry_startup_extensions(&mut items);
    stage("registry-extensions", timing, &mut stamp);
    crate::scan_registry::add_loaded_user_registry_startup(&mut items);
    stage("loaded-user-registry", timing, &mut stamp);
    crate::scan_folders::add_all_startup_folders(&mut items);
    stage("startup-folders", timing, &mut stamp);
    crate::wmi_scan::add_wmi_startup_commands(&mut items);
    stage("wmi-startup-commands", timing, &mut stamp);
    crate::tasks_com::enumerate_scheduled_tasks_com(&mut items);
    stage("task-com-walk", timing, &mut stamp);
    crate::scan_registry::add_group_policy_scripts(&mut items);
    stage("group-policy", timing, &mut stamp);
    crate::services::add_configured_service_entries(&mut items, false);
    crate::services::add_configured_service_entries(&mut items, true);
    stage("services+drivers", timing, &mut stamp);
    crate::scan_registry::add_system_boot_entries(&mut items);
    stage("system-boot-entries", timing, &mut stamp);
    crate::scan_extensions::add_extended_autorun_sources(&mut items);
    stage("extended-autoruns", timing, &mut stamp);
    crate::wmi_scan::add_executable_wmi_consumers(&mut items);
    stage("wmi-consumers", timing, &mut stamp);
    crate::packaged::collect_packaged_startup_tasks(&mut items, false);
    stage("packaged-tasks", timing, &mut stamp);
    let approvals = crate::approvals::read_startup_approved();
    crate::approvals::apply_startup_approved(&mut items, &approvals);
    stage("startup-approved", timing, &mut stamp);
    let disabled = retire_disabled_records_matching_live_routes(&items, (crate::hooks::hooks().load_disabled_items)());
    items.extend(disabled);
    stage("retire-disabled", timing, &mut stamp);
    add_legacy_v2_items(&mut items);
    stage("legacy-v2", timing, &mut stamp);
    crate::humans::hydrate_human_names(&mut items);
    stage("hydrate-human-names", timing, &mut stamp);
    let intent = (crate::hooks::hooks().load_enabled_intent)();
    crate::truth::apply_truth_evidence(&mut items, &intent);
    stage("truth-evidence", timing, &mut stamp);
    crate::runtime::apply_runtime_evidence(&mut items);
    stage("runtime-evidence", timing, &mut stamp);
    let items = crate::dedupe::dedupe(items);
    let mut sorted = items;
    sorted.sort_by(|a, b| {
        (if a.enabled { 0 } else { 1 }).cmp(&(if b.enabled { 0 } else { 1 }))
            .then_with(|| a.source.to_lowercase().cmp(&b.source.to_lowercase()))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    stage("sort", timing, &mut stamp);
    sorted
}

/// C# `ScanAllWithin` — bounded scan with stall tracking.
pub fn scan_all_within(maximum_milliseconds: u64) -> Result<Vec<StartupItem>, String> {
    static STALLED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if STALLED.load(std::sync::atomic::Ordering::SeqCst) >= 2 {
        return Err("Earlier startup scans are still waiting on a drive or system component that stopped answering. The list shows the last complete scan.".into());
    }
    let limit = maximum_milliseconds.max(1000);
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new().name("Startup scan".into())
        .spawn(move || { let _ = sender.send(scan_all()); })
        .map_err(|e| e.to_string())?;
    match receiver.recv_timeout(std::time::Duration::from_millis(limit)) {
        Ok(items) => Ok(items),
        Err(_) => {
            STALLED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let result = handle.join();
            // The worker finished after the deadline; once it drains, scanning recovers.
            if result.is_ok() {
                STALLED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                if let Ok(items) = receiver.recv() {
                    return Ok(items);
                }
            }
            Err(format!("The startup scan did not finish within {} seconds because a drive or system component stopped answering. The list shows the last complete scan.", limit / 1000))
        }
    }
}

/// C# `RetireDisabledRecordsMatchingLiveRoutes` — live registration is authoritative.
pub fn retire_disabled_records_matching_live_routes(live_items: &[StartupItem], disabled_items: Vec<StartupItem>) -> Vec<StartupItem> {
    if live_items.is_empty() || disabled_items.is_empty() {
        return disabled_items;
    }
    let live_identities: Vec<String> = live_items.iter()
        .map(crate::dedupe::shared_user_registry_physical_identity)
        .filter(|identity| !identity.is_empty())
        .collect();
    if live_identities.is_empty() {
        return disabled_items;
    }
    let mut stale: Vec<String> = Vec::new();
    let mut kept: Vec<StartupItem> = Vec::new();
    for disabled in disabled_items {
        let mut identity = crate::dedupe::shared_user_registry_physical_identity(&disabled);
        if identity.is_empty() {
            identity = legacy_disabled_registry_identity(&disabled, live_items);
        }
        if !identity.is_empty() && live_identities.iter().any(|l| l.eq_ignore_ascii_case(&identity)) {
            stale.push(disabled.id.clone());
        } else {
            kept.push(disabled);
        }
    }
    if stale.is_empty() {
        return kept;
    }
    for id in stale {
        (crate::hooks::hooks().retire_disabled_record)(&id);
    }
    kept
}

fn legacy_disabled_registry_identity(disabled: &StartupItem, live_items: &[StartupItem]) -> String {
    if disabled.id.trim().is_empty() {
        return String::new();
    }
    if !disabled.id.to_lowercase().starts_with("disabled|reg|") {
        return String::new();
    }
    let name = disabled.name.trim().to_string();
    static LOCATION_RE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(
        r"(?i)^(?<root>HKEY_[A-Z_]+|HKCU|HKU|HKLM)\\(?<subkey>[^:]+?)(?:\s+\[(Registry\d+)\])?$").unwrap());
    if let Some(captures) = LOCATION_RE.captures(&disabled.location) {
        if !name.is_empty() {
            let root = captures.name("root").map(|m| m.as_str().trim().to_uppercase()).unwrap_or_default();
            let sub_key = captures.name("subkey").map(|m| m.as_str().trim().trim_end_matches('\\').to_uppercase()).unwrap_or_default();
            return format!("shared-user-registry|{}|{}|{}", root, sub_key, name.to_uppercase());
        }
    }
    let command = crate::dedupe::norm_boot_key(&disabled.command);
    if command.len() < 8 || name.is_empty() {
        return String::new();
    }
    for live in live_items {
        if live.id.trim().is_empty() || !eq_ci(&live.source, &disabled.source) {
            continue;
        }
        if !eq_ci(live.name.trim(), &name) {
            continue;
        }
        if crate::dedupe::norm_boot_key(&live.command) != command {
            continue;
        }
        let live_identity = crate::dedupe::shared_user_registry_physical_identity(live);
        if !live_identity.is_empty() {
            return live_identity;
        }
    }
    String::new()
}

/// C# `AddLegacyV2Items` — ghost rows from the legacy v2 state file.
pub fn add_legacy_v2_items(items: &mut Vec<StartupItem>) {
    let v2_path = format!("{}\\state-v2.json", crate::paths::state_root());
    let Ok(raw) = std::fs::read_to_string(&v2_path) else { return };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&raw) else { return };
    let Some(array) = doc.get("Items").and_then(|v| v.as_array()) else { return };
    let live_task_locations: Vec<String> = items.iter().filter(|i| i.source == "Scheduled Task").map(|i| i.location.to_lowercase()).collect();
    let manifest = (crate::hooks::hooks().load_enabled_intent)();
    for entry in array {
        let string = |key: &str| entry.get(key).and_then(|v| v.as_str()).unwrap_or("");
        let int = |key: &str| entry.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
        let mut name = string("LegacyTaskName").to_string();
        if name.trim().is_empty() {
            name = string("Name").to_string();
        }
        let target = string("Target");
        if name.trim().is_empty() || target.trim().is_empty() {
            continue;
        }
        let mut safe_name: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '_' || *c == '.' || *c == '-').collect();
        safe_name = safe_name.trim().to_string();
        if safe_name.is_empty() {
            safe_name = "StartupApp".into();
        }
        let task_location = format!("{}{}", MANAGED_TASK_ROOT, safe_name);
        if manifest.iter().any(|r| r.task_location.eq_ignore_ascii_case(&task_location)) {
            continue;
        }
        if live_task_locations.contains(&task_location.to_lowercase()) {
            continue;
        }
        if (crate::hooks::hooks().is_v2_migrated)(&task_location) {
            continue;
        }
        let enabled = entry.get("Enabled").and_then(|v| v.as_bool()).unwrap_or(false);
        let launch_mode = int("LaunchMode");
        let args = string("Arguments");
        let command = format!("{}{}", target, if args.trim().is_empty() { String::new() } else { format!(" {}", args) }).trim().to_string();
        items.push(StartupItem {
            id: format!("legacy|{}|{}", crate::util::b64(&safe_name), launch_mode),
            name: safe_name,
            source: "Legacy v2".into(),
            scope: "User/System".into(),
            command,
            location: "Legacy v2 state".into(),
            enabled: false,
            can_disable: false,
            status: format!("{} in legacy v2 state — not currently registered; Restore to start it at every boot", if enabled { "Enabled" } else { "Disabled" }),
            ..StartupItem::new()
        });
    }
}

/// C# `IsQuietLaunch` re-export for model.rs.
pub fn is_quiet_launch(command: &str, location: &str) -> bool {
    crate::quiet_plan::is_quiet_launch(command, location)
}
