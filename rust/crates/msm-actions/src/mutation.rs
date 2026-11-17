//! Single-route and bulk enable/disable with exact snapshots and rollback.

use crate::StoreSnapshotVec;
use msm_core::model::StartupItem;
use msm_core::reg::*;
use msm_core::util::{b64, eq_ci, q, unb64};
use msm_state::{disabled, enabled, protected_disabled, protected_quiet, store};

/// C# `EnsureMutationAuthorized` — fail closed on every blocked route.
pub fn ensure_mutation_authorized(item: &StartupItem, expert_confirmed: bool) -> Result<(), String> {
    if !item.external_authority.trim().is_empty() {
        return Err(if item.mutation_reason.is_empty() {
            format!("This route is owned by {}", item.external_authority)
        } else {
            item.mutation_reason.clone()
        });
    }
    if !item.can_disable {
        return Err(if item.mutation_reason.trim().is_empty() {
            "This startup route is not safely mutable with the current access token".into()
        } else {
            item.mutation_reason.clone()
        });
    }
    if item.requires_expert_confirmation && !expert_confirmed {
        return Err("Explicit expert confirmation is required for this high-impact startup route".into());
    }
    Ok(())
}

pub fn disable(item: &StartupItem) -> Result<(), String> {
    disable_expert(item, false)
}

pub fn disable_expert(item: &StartupItem, expert_confirmed: bool) -> Result<(), String> {
    if !item.mutation_capability.trim().is_empty() || item.requires_expert_confirmation || !item.external_authority.trim().is_empty() {
        ensure_mutation_authorized(item, expert_confirmed)?;
    }
    let owned = item.clone();
    msm_state::coordinator::run(move || execute_with_snapshot("Disable", &owned, || disable_core(&owned)))?
}

pub fn enable(item: &StartupItem) -> Result<(), String> {
    enable_expert(item, false)
}

pub fn enable_expert(item: &StartupItem, expert_confirmed: bool) -> Result<(), String> {
    if !item.mutation_capability.trim().is_empty() || item.requires_expert_confirmation || !item.external_authority.trim().is_empty() {
        ensure_mutation_authorized(item, expert_confirmed)?;
    }
    let owned = item.clone();
    msm_state::coordinator::run(move || execute_with_snapshot("Enable", &owned, || enable_core(&owned)))?
}

/// `disable_forced` / `enable_forced` — the user's no-exceptions control law: the
/// one-click verb works on EVERY row. Authorization gates (expert confirmation,
/// external authority, can-disable) are lifted; the exact snapshot + rollback
/// contract of the underlying mutation is fully preserved.
fn lifted(item: &StartupItem) -> StartupItem {
    let mut free = item.clone();
    free.can_disable = true;
    free.external_authority.clear();
    free.mutation_capability.clear();
    free.mutation_reason.clear();
    free.requires_expert_confirmation = false;
    free.requires_elevation = false;
    free
}

pub fn disable_forced(item: &StartupItem) -> Result<(), String> {
    let owned = lifted(item);
    msm_state::coordinator::run(move || execute_with_snapshot("Disable", &owned, || disable_core(&owned)))?
}

pub fn enable_forced(item: &StartupItem) -> Result<(), String> {
    let owned = lifted(item);
    msm_state::coordinator::run(move || execute_with_snapshot("Enable", &owned, || enable_core(&owned)))?
}

/// Forced bulk variant used by the app-level verb on grouped rows.
pub fn set_all_routes_enabled_forced(routes: &[StartupItem], enabled_state: bool) -> Result<usize, String> {
    let owned: Vec<StartupItem> = routes.iter().filter(|r| r.enabled != enabled_state).map(lifted).collect();
    if owned.is_empty() { return Ok(0); }
    msm_state::coordinator::run(move || {
        let mut applied: Vec<(StartupItem, MutationSnapshots)> = Vec::new();
        for item in &owned {
            let snapshots = MutationSnapshots {
                source: Some(capture_source(item, if enabled_state { "Enable" } else { "Disable" })),
                stores: capture_stores(),
                item_enabled: item.enabled,
                approval_data: item.approval_data.clone(),
            };
            let result = if enabled_state { enable_core(item) } else { disable_core(item) };
            match result {
                Ok(()) => applied.push((item.clone(), snapshots)),
                Err(error) => {
                    let mut rollback_errors = Vec::new();
                    for (_, snapshots) in applied.iter().rev() {
                        rollback_errors.extend(restore_all(snapshots));
                    }
                    let route = if item.location.is_empty() { &item.id } else { &item.location };
                    let action = if enabled_state { "enable" } else { "disable" };
                    if !rollback_errors.is_empty() {
                        return Err(format!("Bulk {} failed at {} and rollback was incomplete: {}. Cause: {}", action, route, rollback_errors.join(" | "), error));
                    }
                    return Err(format!("Bulk {} failed at {}; every earlier startup route and intent store was restored. Cause: {}", action, route, error));
                }
            }
        }
        Ok(applied.len())
    })?
}

/// Snapshots (source + all stores) that restore on failure.
struct SourceSnapshot {
    kind: &'static str,
    // Exact registry states (before, expected mutation, optional guard).
    root: Option<RegRoot>,
    view: RegView,
    sub_key: String,
    value_name: String,
    exact: Option<(RegistryValueState, RegistryValueState)>,
    guard: Option<(String, RegistryValueState)>,
    plain: Option<RegistryValueState>,
    // Task / folder / service / wmi fallback snapshots.
    task_location: Option<String>,
    task_existed: bool,
    task_xml: Option<String>,
    folder_original: Option<std::path::PathBuf>,
    folder_exists: bool,
    service_name: Option<String>,
    service_start: Option<i64>,
    wmi_location: Option<String>,
    wmi_existed: bool,
    wmi_filter: String,
    wmi_consumer: String,
}

struct MutationSnapshots {
    source: Option<SourceSnapshot>,
    stores: Vec<StoreSnapshotVec>,
    item_enabled: bool,
    approval_data: Option<Vec<u8>>,
}

fn capture_stores() -> Vec<StoreSnapshotVec> {
    vec![
        StoreSnapshotVec { path: msm_state::disabled_store(), bytes: std::fs::read(msm_state::disabled_store()).ok() },
        StoreSnapshotVec { path: msm_state::enabled_store(), bytes: std::fs::read(msm_state::enabled_store()).ok() },
        StoreSnapshotVec { path: msm_state::protected_disabled_store(), bytes: std::fs::read(msm_state::protected_disabled_store()).ok() },
        StoreSnapshotVec { path: msm_state::protected_quiet_store(), bytes: std::fs::read(msm_state::protected_quiet_store()).ok() },
        StoreSnapshotVec { path: msm_state::migrated_marker_path(), bytes: std::fs::read(msm_state::migrated_marker_path()).ok() },
    ]
}

fn execute_with_snapshot(operation: &str, item: &StartupItem, mutation: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let snapshots = MutationSnapshots {
        source: Some(capture_source(item, operation)),
        stores: capture_stores(),
        item_enabled: item.enabled,
        approval_data: item.approval_data.clone(),
    };
    match mutation() {
        Ok(()) => Ok(()),
        Err(error) => {
            let rollback_errors = restore_all(&snapshots);
            if rollback_errors.is_empty() {
                Err(format!("{} failed; the Windows source and all intent stores were restored. Cause: {}", operation, error))
            } else {
                Err(format!("{} failed and rollback was incomplete: {}. Cause: {}", operation, rollback_errors.join(" | "), error))
            }
        }
    }
}

fn capture_source(item: &StartupItem, operation: &str) -> SourceSnapshot {
    let id = &item.id;
    let lower = id.to_lowercase();
    let mut snap = SourceSnapshot {
        kind: "none", root: None, view: RegView::Default, sub_key: String::new(), value_name: String::new(),
        exact: None, guard: None, plain: None, task_location: None, task_existed: false, task_xml: None,
        folder_original: None, folder_exists: false, service_name: None, service_start: None,
        wmi_location: None, wmi_existed: false, wmi_filter: String::new(), wmi_consumer: String::new(),
    };
    if !item.approval_root.trim().is_empty() {
        let view = RegView::parse(if item.approval_registry_view.is_empty() { native_registry_view().label() } else { &item.approval_registry_view });
        let root = RegRoot::parse(&item.approval_root);
        if let Some(root) = root {
            let original = read_registry_value_state(root, &item.approval_path, &item.approval_name, view);
            if operation == "Disable" || operation == "Enable" {
                if let Some(bytes) = &item.approval_data {
                    let mutated = RegistryValueState::build(true, RegKind::Binary, Some(&RegValue::Bytes(apply_startup_approval_state(bytes, operation == "Enable").unwrap_or_default())));
                    snap.root = Some(root);
                    snap.view = view;
                    snap.sub_key = item.approval_path.clone();
                    snap.value_name = item.approval_name.clone();
                    snap.exact = Some((original.clone(), mutated));
                }
            }
            if snap.exact.is_none() {
                snap.root = Some(root);
                snap.view = view;
                snap.sub_key = item.approval_path.clone();
                snap.value_name = item.approval_name.clone();
                snap.plain = Some(original);
            }
            snap.kind = "registry";
        }
        return snap;
    }
    if lower.starts_with("winlogon|") || lower.starts_with("advanced|") || lower.starts_with("appinit|") {
        capture_advanced_source(item, &mut snap);
        return snap;
    }
    if lower.starts_with("disabled|winlogon|") || lower.starts_with("disabled|advanced|") || lower.starts_with("disabled|appinit|") {
        if let Ok(record) = disabled::decode_advanced(&item.status) {
            let Some(root) = RegRoot::parse(&record.root_name) else { return snap };
            snap.root = Some(root);
            snap.view = RegView::parse(&record.registry_view);
            snap.sub_key = record.sub_key.clone();
            snap.value_name = record.value_name.clone();
            snap.exact = Some((record.after.clone(), record.before.clone()));
            if let (Some(guard), name) = (&record.guard_before, record.guard_value_name.as_str()) {
                if !name.is_empty() {
                    snap.guard = Some((name.to_string(), guard.clone()));
                }
            }
            snap.kind = "registry";
        }
        return snap;
    }
    if lower.starts_with("task|") || lower.starts_with("disabled|task|") {
        snap.kind = "task";
        snap.task_location = Some(item.location.clone());
        snap.task_xml = msm_core::tasks_com::try_read_task_xml(&item.location);
        snap.task_existed = snap.task_xml.is_some();
        return snap;
    }
    if lower.starts_with("reg|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() >= 5 {
            let root_name = unb64(parts[2]);
            if let Some(root) = RegRoot::parse(&root_name) {
                snap.root = Some(root);
                snap.view = if parts.len() > 5 { RegView::parse(parts[5]) } else { RegView::parse(&item.registry_view) };
                snap.sub_key = unb64(parts[3]);
                snap.value_name = unb64(parts[4]);
                snap.plain = Some(read_registry_value_state(root, &snap.sub_key, &snap.value_name, snap.view));
                snap.kind = "registry";
            }
        }
        return snap;
    }
    if lower.starts_with("disabled|reg|") {
        let meta: Vec<&str> = item.status.split('\t').collect();
        if meta.len() >= 4 && meta[0].starts_with("HKEY_") {
            if let Some(root) = RegRoot::parse(meta[0]) {
                snap.root = Some(root);
                snap.view = RegView::parse(if meta.len() > 6 { meta[6] } else { "" });
                snap.sub_key = meta[1].into();
                snap.value_name = meta[2].into();
                snap.plain = Some(read_registry_value_state(root, &snap.sub_key, &snap.value_name, snap.view));
                snap.kind = "registry";
            }
        }
        return snap;
    }
    if lower.starts_with("active|") || lower.starts_with("disabled|active|") {
        let meta: Vec<&str> = item.status.split('\t').collect();
        let (root_name, sub_key, value_name, view) = if lower.starts_with("active|") {
            let parts: Vec<&str> = id.split('|').collect();
            if parts.len() < 4 { return snap }
            (unb64(parts[2]), unb64(parts[3]), "StubPath".to_string(), if parts.len() > 4 { parts[4].to_string() } else { item.registry_view.clone() })
        } else if meta.len() >= 3 {
            (meta[0].to_string(), meta[1].to_string(), meta[2].to_string(), if meta.len() > 5 { meta[5].to_string() } else { String::new() })
        } else { return snap };
        if let Some(root) = RegRoot::parse(&root_name) {
            snap.root = Some(root);
            snap.view = RegView::parse(&view);
            snap.sub_key = sub_key;
            snap.value_name = value_name;
            snap.plain = Some(read_registry_value_state(root, &snap.sub_key, &snap.value_name, snap.view));
            snap.kind = "registry";
        }
        return snap;
    }
    if lower.starts_with("folder|") || lower.starts_with("disabled|folder|") {
        snap.kind = "folder";
        let original = if lower.starts_with("disabled|folder|") { item.status.clone() } else { item.command.clone() };
        snap.folder_original = Some(std::path::PathBuf::from(&original));
        snap.folder_exists = std::path::Path::new(&original).is_file();
        return snap;
    }
    if lower.starts_with("service|") || lower.starts_with("driver|") || lower.starts_with("disabled|service|") || lower.starts_with("disabled|driver|") {
        let service_name = service_name_from_item(item);
        if !service_name.is_empty() {
            snap.kind = "registry";
            snap.service_name = Some(service_name.clone());
            snap.sub_key = format!(r"SYSTEM\CurrentControlSet\Services\{}", service_name);
            snap.value_name = "Start".into();
            snap.root = Some(RegRoot::LocalMachine);
            let current = read_registry_value_state(RegRoot::LocalMachine, &snap.sub_key, "Start", RegView::Default);
            if !current.exists {
                snap.kind = "none";
                snap.service_name = None;
                return snap;
            }
            snap.plain = Some(current);
        }
        return snap;
    }
    if lower.starts_with("wmisub|") || lower.starts_with("disabled|wmisub|") {
        snap.kind = "wmi";
        snap.wmi_location = Some(item.location.clone());
        // Binding presence is re-checked at rollback time; archived refs come from status.
        let meta: Vec<&str> = item.status.split('\t').collect();
        if meta.len() >= 2 {
            snap.wmi_filter = meta[0].into();
            snap.wmi_consumer = meta[1].into();
        }
        snap.wmi_existed = true;
        return snap;
    }
    if lower.starts_with("packaged|") {
        if let Some(path) = msm_core::packaged::packaged_startup_task_state_path(&item.name) {
            snap.kind = "registry";
            snap.root = Some(RegRoot::CurrentUser);
            snap.sub_key = path;
            snap.value_name = "State".into();
            snap.plain = Some(read_registry_value_state(RegRoot::CurrentUser, &snap.sub_key, &snap.value_name, RegView::Default));
        }
        return snap;
    }
    if lower.starts_with("extension|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() >= 6 {
            if let Some(root) = RegRoot::parse(&unb64(parts[2])) {
                snap.root = Some(root);
                snap.view = RegView::parse(if item.registry_view.is_empty() { native_registry_view().label() } else { &item.registry_view });
                snap.sub_key = unb64(parts[3]);
                snap.value_name = unb64(parts[4]);
                let original = read_registry_value_state(root, &snap.sub_key, &snap.value_name, snap.view);
                snap.exact = Some((original, RegistryValueState::missing()));
                snap.kind = "registry";
            }
        }
        return snap;
    }
    snap.kind = "none";
    snap
}

fn capture_advanced_source(item: &StartupItem, snap: &mut SourceSnapshot) {
    let id = &item.id;
    let lower = id.to_lowercase();
    if lower.starts_with("winlogon|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() < 7 { return }
        let Some(root) = RegRoot::parse(&unb64(parts[2])) else { return };
        snap.root = Some(root);
        snap.view = RegView::parse(parts[6]);
        snap.sub_key = unb64(parts[3]);
        snap.value_name = unb64(parts[4]);
        let original = read_registry_value_state(root, &snap.sub_key, &snap.value_name, snap.view);
        if let Some(after) = build_winlogon_removal(&original, parts[5].parse().unwrap_or(0), &item.command) {
            snap.exact = Some((original, after));
            snap.kind = "registry";
        }
    } else if lower.starts_with("advanced|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() < 5 { return }
        snap.view = RegView::parse(if item.registry_view.is_empty() { native_registry_view().label() } else { &item.registry_view });
        let Some(root) = RegRoot::parse(&unb64(parts[1])) else { return };
        snap.root = Some(root);
        snap.sub_key = unb64(parts[2]);
        snap.value_name = unb64(parts[3]);
        let original = read_registry_value_state(root, &snap.sub_key, &snap.value_name, snap.view);
        if let Some(after) = build_multi_string_removal(&original, parts[4].parse().unwrap_or(0), &item.command) {
            snap.exact = Some((original, after));
            snap.kind = "registry";
        }
    } else if lower.starts_with("appinit|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() < 3 { return }
        let Some(root) = RegRoot::parse(RegRoot::LocalMachine.name()) else { return };
        snap.root = Some(root);
        snap.view = RegView::parse(parts[2]);
        snap.sub_key = unb64(parts[1]);
        let dlls = read_registry_value_state(root, &snap.sub_key, "AppInit_DLLs", snap.view);
        let load = read_registry_value_state(root, &snap.sub_key, "LoadAppInit_DLLs", snap.view);
        if let Some(after) = build_appinit_toggle(&dlls, &load) {
            snap.exact = Some((load, after));
            snap.guard = Some(("AppInit_DLLs".into(), dlls));
            snap.value_name = "LoadAppInit_DLLs".into();
            snap.kind = "registry";
        }
    }
}

fn apply_startup_approval_state(original: &[u8], enabled: bool) -> Result<Vec<u8>, String> {
    msm_core::approvals::apply_startup_approval_state(original, enabled)
}

/// C# `BuildWinlogonRemovalPlan` — remove one exact unquoted-comma component.
pub fn build_winlogon_removal(before: &RegistryValueState, index: usize, expected: &str) -> Option<RegistryValueState> {
    if !before.exists || !matches!(before.kind(), RegKind::String | RegKind::ExpandString) {
        return None;
    }
    let parts = msm_core::scan_registry::try_split_winlogon_components(&before.payload)?;
    if index >= parts.len() || parts[index].trim().is_empty() || parts[index].trim() != expected.trim() {
        return None;
    }
    let mut remaining = parts.clone();
    remaining.remove(index);
    Some(RegistryValueState::build(true, before.kind(), Some(&RegValue::Str(remaining.join(","))))
        .normalized(before.kind()))
}

/// C# `BuildIndexedMultiStringRemovalPlan`.
pub fn build_multi_string_removal(before: &RegistryValueState, index: usize, expected: &str) -> Option<RegistryValueState> {
    if !before.exists || before.kind() != RegKind::MultiString {
        return None;
    }
    let values: Vec<String> = serde_json::from_str(&before.payload).ok()?;
    if index >= values.len() || values[index] != expected {
        return None;
    }
    let after: Vec<String> = values.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, v)| v.clone()).collect();
    Some(RegistryValueState::build(true, RegKind::MultiString, Some(&RegValue::MultiString(after))))
}

/// C# `BuildAppInitTogglePlan` — toggle only LoadAppInit_DLLs, guard the DLL list.
pub fn build_appinit_toggle(dlls: &RegistryValueState, load: &RegistryValueState) -> Option<RegistryValueState> {
    if !dlls.exists || dlls.payload.trim().is_empty() { return None }
    if !matches!(dlls.kind(), RegKind::String | RegKind::ExpandString) { return None }
    if !load.exists || load.kind() != RegKind::DWord { return None }
    let current: i64 = load.payload.trim().parse().ok()?;
    if current == 0 { return None }
    Some(RegistryValueState::build(true, RegKind::DWord, Some(&RegValue::DWord(0))))
}

fn restore_all(snapshots: &MutationSnapshots) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(source) = &snapshots.source {
        if let Err(e) = restore_source(source) {
            errors.push(format!("source={}", e));
        }
    }
    for store_snapshot in &snapshots.stores {
        if let Err(e) = restore_store_bytes(store_snapshot) {
            errors.push(e);
        }
    }
    errors
}

fn restore_store_bytes(snapshot: &StoreSnapshotVec) -> Result<(), String> {
    match &snapshot.bytes {
        Some(bytes) => msm_state::store::write_all_lines_raw(&snapshot.path, bytes),
        None => {
            if snapshot.path.exists() {
                std::fs::remove_file(&snapshot.path).map_err(|e| e.to_string())
            } else {
                Ok(())
            }
        }
    }
}

fn restore_source(source: &SourceSnapshot) -> Result<(), String> {
    match source.kind {
        "none" => Ok(()),
        "task" => {
            let location = source.task_location.clone().unwrap_or_default();
            if source.task_existed {
                if let Some(xml) = &source.task_xml {
                    msm_core::managed::restore_task_xml(&location, xml)
                } else {
                    Ok(())
                }
            } else if msm_core::tasks_com::query_task_state(&location).is_some() {
                msm_core::managed::run_checked("schtasks.exe", &format!("/Delete /F /TN {}", q(&location)), 30_000)
            } else {
                Ok(())
            }
        }
        "registry" => {
            let Some(root) = source.root else { return Ok(()) };
            if let Some((before, expected_mutation)) = &source.exact {
                if let Some((guard_name, guard)) = &source.guard {
                    let current_guard = read_registry_value_state(root, &source.sub_key, guard_name, source.view);
                    if !registry_value_state_equals(&current_guard, guard) {
                        return Err("Guarded registry value changed concurrently; rollback refuses to overwrite startup state".into());
                    }
                }
                let current = read_registry_value_state(root, &source.sub_key, &source.value_name, source.view);
                if registry_value_state_equals(current_ref(&current), before) {
                    return Ok(());
                }
                if !registry_value_state_equals(&current, expected_mutation) {
                    return Err("Registry startup value changed concurrently; rollback refuses to overwrite the unexpected state".into());
                }
                write_registry_value_state_exact(root, &source.sub_key, &source.value_name, source.view, expected_mutation, before)
            } else if let Some(plain) = &source.plain {
                if let Some(handle) = create_subkey(root, &source.sub_key, source.view) {
                    if plain.exists {
                        if let Some(value) = plain.decode() {
                            if !handle.set_value(&source.value_name, plain.kind(), &value) {
                                return Err("registry rollback write failed".into());
                            }
                        }
                    } else if handle.value_names().iter().any(|n| n.eq_ignore_ascii_case(&source.value_name)) {
                        let _ = handle.delete_value(&source.value_name);
                    }
                }
                Ok(())
            } else {
                Ok(())
            }
        }
        "folder" => {
            if let Some(original) = &source.folder_original {
                if source.folder_exists && !original.exists() {
                    // Locate the quarantine file created by this mutation.
                    let prefix = format!("{}.", original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    if let Ok(entries) = std::fs::read_dir(msm_state::disabled_startup_folder()) {
                        let mut candidates: Vec<_> = entries.filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("disabled")).unwrap_or(false)
                                && p.file_name().map(|n| n.to_string_lossy().starts_with(&prefix)).unwrap_or(false))
                            .collect();
                        candidates.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).map(|t| t).unwrap_or(std::time::SystemTime::UNIX_EPOCH));
                        if let Some(last) = candidates.last() {
                            let _ = std::fs::rename(last, original);
                        }
                    }
                }
            }
            Ok(())
        }
        "wmi" => {
            let location = source.wmi_location.clone().unwrap_or_default();
            let exists = msm_state::wmi_guard::binding_exists(&location).unwrap_or(false);
            if source.wmi_existed && !exists && !source.wmi_filter.is_empty() && !source.wmi_consumer.is_empty() {
                return msm_state::wmi_guard::create_binding(&source.wmi_filter, &source.wmi_consumer);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn current_ref(state: &RegistryValueState) -> &RegistryValueState {
    state
}

pub fn service_name_from_item(item: &StartupItem) -> String {
    let id = &item.id;
    let lower = id.to_lowercase();
    if lower.starts_with("disabled|service|") || lower.starts_with("disabled|driver|") {
        return item.status.split('\t').next().unwrap_or("").to_string();
    }
    if lower.starts_with("service|") || lower.starts_with("driver|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() > 1 {
            return unb64(parts[1]);
        }
    }
    String::new()
}

// ---- core mutations ----

pub fn disable_core(item: &StartupItem) -> Result<(), String> {
    let id = &item.id;
    let lower = id.to_lowercase();
    if lower.starts_with("legacy|") {
        enabled::remove_item(item);
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() > 1 && !unb64(parts[1]).trim().is_empty() {
            enabled::mark_v2_migrated_checked(&format!("{}{}", msm_core::scan::MANAGED_TASK_ROOT, unb64(parts[1])));
        }
        return Ok(());
    }
    if !item.approval_root.trim().is_empty() {
        let mut mutable = item.clone();
        msm_core::approvals::set_startup_approval(&mut mutable, false)?;
        enabled::remove_item(&mutable);
        return Ok(());
    }
    if lower.starts_with("packaged|") {
        let mut mutable = item.clone();
        msm_core::packaged::set_packaged_startup_task_state(&mut mutable, false)?;
        enabled::remove_item(&mutable);
        return Ok(());
    }
    if lower.starts_with("winlogon|") {
        return disable_winlogon(item);
    }
    if lower.starts_with("appinit|") {
        return disable_appinit(item);
    }
    if lower.starts_with("advanced|") {
        return disable_advanced_multi(item);
    }
    if lower.starts_with("extension|") {
        return disable_extension(item);
    }
    if lower.starts_with("reg|") {
        return disable_registry(item);
    }
    if lower.starts_with("active|") {
        return disable_active_setup(item);
    }
    if lower.starts_with("folder|") {
        return disable_startup_folder(item);
    }
    if lower.starts_with("task|") {
        msm_core::managed::run_checked("schtasks.exe", &format!("/Change /TN {} /Disable", q(&item.location)), 30_000)?;
        if !item.location.trim().is_empty() {
            protected_quiet::unprotect_task(&item.location);
        }
        enabled::remove_item(item);
        return Ok(());
    }
    if lower.starts_with("service|") || lower.starts_with("driver|") {
        return disable_service_or_driver(item, if lower.starts_with("driver|") { "driver" } else { "service" });
    }
    if lower.starts_with("wmisub|") {
        return disable_wmi_binding(item);
    }
    Err(unsupported_message(item))
}

/// Understandable replacement for the old raw "Unsupported item: <id>" error.
fn unsupported_message(item: &StartupItem) -> String {
    let what = if item.app_name.trim().is_empty() { item.name.as_str() } else { item.app_name.as_str() };
    let why = if item.mutation_reason.trim().is_empty() {
        "it is a Windows system registration (not a normal startup entry) that Startup Master does not switch off, because removing it can break sign-in, networking or printing".to_string()
    } else {
        item.mutation_reason.clone()
    };
    format!("{} ({}) cannot be switched off from here: {}.", what, item.source, why.trim_end_matches('.'))
}

pub fn enable_core(item: &StartupItem) -> Result<(), String> {
    let id = &item.id;
    let lower = id.to_lowercase();
    if lower.starts_with("legacy|") {
        return crate::managed::enable_legacy_v2(item);
    }
    if !item.approval_root.trim().is_empty() {
        let mut mutable = item.clone();
        msm_core::approvals::set_startup_approval(&mut mutable, true)?;
        enabled::remove_item_for_upsert(&mutable);
        upsert_item_checked(&mutable)?;
        protected_disabled::unprotect(&mutable);
        return Ok(());
    }
    if lower.starts_with("packaged|") {
        let mut mutable = item.clone();
        msm_core::packaged::set_packaged_startup_task_state(&mut mutable, true)?;
        enabled::remove_item_for_upsert(&mutable);
        upsert_item_checked(&mutable)?;
        protected_disabled::unprotect(&mutable);
        return Ok(());
    }
    if lower.starts_with("disabled|winlogon|") || lower.starts_with("disabled|appinit|") || lower.starts_with("disabled|advanced|") {
        return restore_advanced(item);
    }
    if lower.starts_with("disabled|reg|") {
        return restore_registry(item);
    }
    if lower.starts_with("disabled|active|") {
        return restore_active_setup(item);
    }
    if lower.starts_with("disabled|folder|") {
        return restore_startup_folder(item);
    }
    if lower.starts_with("disabled|service|") || lower.starts_with("disabled|driver|") {
        return restore_service_or_driver(item);
    }
    if lower.starts_with("disabled|wmisub|") {
        return restore_wmi_binding(item);
    }
    if lower.starts_with("task|") {
        msm_core::managed::run_checked("schtasks.exe", &format!("/Change /TN {} /Enable", q(&item.location)), 30_000)?;
        upsert_item_checked(item)?;
        protected_disabled::unprotect(item);
        if lower.starts_with("task|") && msm_core::quiet_plan::is_quiet_launch(&item.command, &item.location) && !item.location.trim().is_empty() {
            let (target, arguments) = resolve_launch_target_or_fail(item)?;
            if !target.trim().is_empty() {
                protected_quiet::protect_task(&item.location, &target, &arguments);
            }
        }
        return Ok(());
    }
    Err(format!("Unsupported disabled item: {}", item.id))
}

pub fn resolve_launch_target_or_fail(item: &StartupItem) -> Result<(String, String), String> {
    // C# ResolveLaunchTarget with bare-name PATH resolution.
    let command = &item.command;
    if item.source == "Startup Folder" && command.to_lowercase().ends_with(".lnk") {
        let (target, args) = msm_core::humans::resolve_shortcut(command);
        if std::path::Path::new(&target).is_file() {
            return Ok((target, args));
        }
    }
    if let Some((target, arguments)) = msm_core::humans::try_decode_tray_payload(command) {
        if std::path::Path::new(&target).is_file() {
            return Ok((target, arguments));
        }
    }
    let Some((mut target, mut arguments)) = msm_core::humans::try_split_command(command) else {
        return Err("Could not parse executable from command".into());
    };
    if target.to_lowercase().ends_with(".lnk") {
        let command_arguments = arguments.clone();
        let (resolved, shortcut_arguments) = msm_core::humans::resolve_shortcut(&target);
        target = resolved;
        arguments = merge_arguments(&shortcut_arguments, &command_arguments);
    }
    if !target.contains('\\') && !target.contains('/') && !target.contains(':') {
        if let Some(resolved) = resolve_bare_executable(&target) {
            target = resolved;
        }
    }
    if !std::path::Path::new(&target).is_file() || !msm_core::managed::is_supported_startup_target(&target) {
        return Err(format!("Resolved startup target is not a supported startup target: {}", target));
    }
    Ok((target, arguments))
}

fn merge_arguments(first: &str, second: &str) -> String {
    regex::Regex::new(r"\s+").unwrap()
        .replace_all(&format!("{} {}", first.trim(), second.trim()).trim(), " ")
        .into_owned()
}

fn resolve_bare_executable(name: &str) -> Option<String> {
    let mut roots = vec![msm_core::paths::system_dir(), msm_core::paths::system_dir_x86(), msm_core::paths::windows_dir()];
    roots.extend(std::env::var("PATH").unwrap_or_default().split(';').filter(|d| !d.trim().is_empty()).map(|d| d.to_string()));
    let extensions: Vec<String> = if std::path::Path::new(name).extension().is_none() {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE".into()).split(';').map(|e| e.to_string()).collect()
    } else {
        vec![String::new()]
    };
    for root in roots {
        for extension in &extensions {
            let candidate = format!("{}\\{}{}", root.trim_end_matches('\\'), name, extension);
            if std::path::Path::new(&candidate).is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn upsert_item_checked(item: &StartupItem) -> Result<(), String> {
    // C# UpsertFromItemChecked — resolve target and write the exact intent row.
    let id = &item.id;
    let lower = id.to_lowercase();
    let kind;
    let mut task_location = String::new();
    let mut mode = "normal".to_string();
    let mut target = String::new();
    let mut args = String::new();
    let mut status = item.status.clone();
    let mut command = item.command.clone();
    if lower.starts_with("task|") || lower.starts_with("disabled|task|") {
        task_location = item.location.clone();
        kind = if item.is_managed { "managed-task" } else { "task" }.to_string();
        let (t, a) = resolve_launch_target_or_fail(item)?;
        target = t;
        args = a;
        mode = if msm_core::quiet_plan::is_quiet_launch(&item.command, &task_location) { "tray".into() } else { "normal".into() };
    } else if lower.starts_with("reg|") || lower.starts_with("disabled|reg|") {
        kind = "registry".into();
        let (t, a) = resolve_launch_target_or_fail(item)?;
        target = t;
        args = a;
        if lower.starts_with("reg|") {
            status = registry_intent_status(item)?;
        }
    } else if lower.starts_with("folder|") || lower.starts_with("disabled|folder|") {
        kind = "folder".into();
        if lower.starts_with("disabled|folder|") {
            command = item.status.clone();
        }
        let launch_item = StartupItem { id: "folder|intent".into(), name: item.name.clone(), source: "Startup Folder".into(),
            scope: item.scope.clone(), command: command.clone(), location: item.location.clone(), enabled: true, ..StartupItem::new() };
        let (t, a) = resolve_launch_target_or_fail(&launch_item)?;
        target = t;
        args = a;
    } else if lower.starts_with("service|") || lower.starts_with("disabled|service|") {
        kind = "service".into();
        status = exact_service_intent(item)?;
    } else if lower.starts_with("driver|") || lower.starts_with("disabled|driver|") {
        kind = "driver".into();
        status = exact_service_intent(item)?;
    } else {
        return Ok(());
    }
    enabled::upsert(&enabled::Row {
        kind, name: item.name.clone(), scope: item.scope.clone(), command, location: item.location.clone(),
        status, target, arguments: args, mode, task_location,
    });
    Ok(())
}

fn registry_intent_status(item: &StartupItem) -> Result<String, String> {
    let parts: Vec<&str> = item.id.split('|').collect();
    if parts.len() < 5 || !parts[0].eq_ignore_ascii_case("reg") {
        return Err("Registry startup identity is incomplete".into());
    }
    let root_name = unb64(parts[2]);
    let sub_key = unb64(parts[3]);
    let value_name = unb64(parts[4]);
    let view_label = if parts.len() > 5 { parts[5].to_string() } else { item.registry_view.clone() };
    let Some(root) = RegRoot::parse(&root_name) else { return Err("unsupported root".into()) };
    let view = RegView::parse(&view_label);
    let Some(handle) = open_subkey(root, &sub_key, view) else {
        return Err(format!("Registry startup value is missing: {}", value_name));
    };
    if !handle.value_names().iter().any(|n| n.eq_ignore_ascii_case(&value_name)) {
        return Err(format!("Registry startup value is missing: {}", value_name));
    }
    let Some(kind) = handle.value_kind(&value_name) else {
        return Err(format!("Registry startup value is missing: {}", value_name));
    };
    Ok(format!("{}\t{}\t{}\t{}\t{}\t\t{}", root_name, sub_key, value_name, kind.as_str(), item.source, view_label))
}

fn exact_service_intent(item: &StartupItem) -> Result<String, String> {
    let service_name = service_name_from_item(item);
    if service_name.is_empty() {
        return Err("Service identity is missing from the enabled intent".into());
    }
    if let Some((stored_name, start)) = enabled::try_parse_exact_service_intent(&item.status) {
        if stored_name.eq_ignore_ascii_case(&service_name) {
            return Ok(format!("{}\t{}", service_name, start));
        }
    }
    let start = msm_core::services::read_service_start_value(&service_name, -1);
    if !(0..=3).contains(&start) {
        return Err(format!("Could not preserve the exact startup type for service {}", service_name));
    }
    Ok(format!("{}\t{}", service_name, start))
}

fn disable_winlogon(item: &StartupItem) -> Result<(), String> {
    let parts: Vec<&str> = item.id.split('|').collect();
    if parts.len() < 7 { return Err("Malformed Winlogon component identity".into()) }
    let scope = parts[1];
    let root_name = unb64(parts[2]);
    let sub_key = unb64(parts[3]);
    let value_name = unb64(parts[4]);
    let index: usize = parts[5].parse().map_err(|_| "Malformed Winlogon component index".to_string())?;
    let view_label = parts[6];
    let Some(root) = RegRoot::parse(&root_name) else { return Err("unsupported root".into()) };
    let view = RegView::parse(view_label);
    let before = read_registry_value_state(root, &sub_key, &value_name, view);
    let Some(after) = build_winlogon_removal(&before, index, &item.command) else {
        return Err("Winlogon component changed externally; refusing to remove another component".into());
    };
    let record = disabled::AdvancedDisabledState {
        version: 1, route_type: "winlogon".into(), source: "Winlogon Autostart".into(),
        root_name: root_name.clone(), sub_key: sub_key.clone(), value_name: value_name.clone(),
        registry_view: view_label.to_string(), original_index: index as i32, component: item.command.clone(),
        before: before.clone(), after: after.clone(), guard_value_name: String::new(), guard_before: None,
        requires_expert_confirmation: true, requires_reboot: true,
    };
    let disabled_id = disabled::add("winlogon", &item.name, scope, &item.command, &item.location, &disabled::encode_advanced(&record));
    if let Err(e) = write_registry_value_state_exact(root, &sub_key, &value_name, view, &before, &after) {
        disabled::remove(&disabled_id);
        return Err(e);
    }
    Ok(())
}

fn disable_advanced_multi(item: &StartupItem) -> Result<(), String> {
    let parts: Vec<&str> = item.id.split('|').collect();
    if parts.len() < 5 { return Err("Malformed advanced startup identity".into()) }
    let root_name = unb64(parts[1]);
    let sub_key = unb64(parts[2]);
    let value_name = unb64(parts[3]);
    let index: usize = parts[4].parse().map_err(|_| "Malformed advanced startup component index".to_string())?;
    let view_label = if item.registry_view.is_empty() { native_registry_view().label().to_string() } else { item.registry_view.clone() };
    let Some(root) = RegRoot::parse(&root_name) else { return Err("unsupported root".into()) };
    let view = RegView::parse(&view_label);
    let before = read_registry_value_state(root, &sub_key, &value_name, view);
    let Some(after) = build_multi_string_removal(&before, index, &item.command) else {
        return Err("Advanced startup component changed externally; refusing to remove another component".into());
    };
    let record = disabled::AdvancedDisabledState {
        version: 1, route_type: "advanced".into(), source: item.source.clone(),
        root_name: root_name.clone(), sub_key: sub_key.clone(), value_name: value_name.clone(),
        registry_view: view_label.clone(), original_index: index as i32, component: item.command.clone(),
        before: before.clone(), after: after.clone(), guard_value_name: String::new(), guard_before: None,
        requires_expert_confirmation: true, requires_reboot: true,
    };
    let disabled_id = disabled::add("advanced", &item.name, &item.scope, &item.command, &item.location, &disabled::encode_advanced(&record));
    if let Err(e) = write_registry_value_state_exact(root, &sub_key, &value_name, view, &before, &after) {
        disabled::remove(&disabled_id);
        return Err(e);
    }
    Ok(())
}

fn disable_extension(item: &StartupItem) -> Result<(), String> {
    let parts: Vec<&str> = item.id.split('|').collect();
    if parts.len() < 6 { return Err("Malformed extension startup identity".into()) }
    let kind = parts[1];
    let root_name = unb64(parts[2]);
    let sub_key = unb64(parts[3]);
    let value_name = unb64(parts[4]);
    let view_label = if item.registry_view.is_empty() { native_registry_view().label().to_string() } else { item.registry_view.clone() };
    if !msm_core::scan_extensions::extension_disable_surface(Some(kind), &root_name, &sub_key, &value_name, &item.source) {
        return Err("This extension point is read-only; it is outside the reversible single-value surface".into());
    }
    let Some(root) = RegRoot::parse(&root_name) else { return Err("unsupported root".into()) };
    let view = RegView::parse(&view_label);
    let before = read_registry_value_state(root, &sub_key, &value_name, view);
    if !before.exists {
        return Err("Registry startup value changed after scanning; refresh before changing it".into());
    }
    let after = RegistryValueState::missing();
    let record = disabled::AdvancedDisabledState {
        version: 1, route_type: "extension".into(), source: item.source.clone(),
        root_name: root_name.clone(), sub_key: sub_key.clone(), value_name: value_name.clone(),
        registry_view: view_label.clone(), original_index: -1, component: item.command.clone(),
        before: before.clone(), after: after.clone(), guard_value_name: String::new(), guard_before: None,
        requires_expert_confirmation: item.requires_expert_confirmation, requires_reboot: item.requires_reboot,
    };
    let disabled_id = disabled::add("advanced", &item.name, &item.scope, &item.command, &item.location, &disabled::encode_advanced(&record));
    if let Err(e) = write_registry_value_state_exact(root, &sub_key, &value_name, view, &before, &after) {
        disabled::remove(&disabled_id);
        return Err(e);
    }
    Ok(())
}

fn disable_appinit(item: &StartupItem) -> Result<(), String> {
    let parts: Vec<&str> = item.id.split('|').collect();
    if parts.len() < 3 { return Err("Malformed AppInit identity".into()) }
    let sub_key = unb64(parts[1]);
    let view_label = parts[2];
    let Some(root) = RegRoot::parse(RegRoot::LocalMachine.name()) else { return Err("unsupported root".into()) };
    let view = RegView::parse(view_label);
    let dlls = read_registry_value_state(root, &sub_key, "AppInit_DLLs", view);
    let load = read_registry_value_state(root, &sub_key, "LoadAppInit_DLLs", view);
    let Some(after) = build_appinit_toggle(&dlls, &load) else {
        return Err("AppInit DLL loading is already disabled or its shape is unsupported".into());
    };
    let record = disabled::AdvancedDisabledState {
        version: 1, route_type: "appinit".into(), source: "AppInit DLLs".into(),
        root_name: RegRoot::LocalMachine.name().into(), sub_key: sub_key.clone(), value_name: "LoadAppInit_DLLs".into(),
        registry_view: view_label.to_string(), original_index: -1, component: item.command.clone(),
        before: load.clone(), after: after.clone(), guard_value_name: "AppInit_DLLs".into(), guard_before: Some(dlls.clone()),
        requires_expert_confirmation: true, requires_reboot: true,
    };
    let disabled_id = disabled::add("appinit", &item.name, &item.scope, &item.command, &item.location, &disabled::encode_advanced(&record));
    let result = (|| -> Result<(), String> {
        if !registry_value_state_equals(&read_registry_value_state(root, &sub_key, "AppInit_DLLs", view), &dlls) {
            return Err("AppInit_DLLs changed after scanning; refresh before changing it".into());
        }
        write_registry_value_state_exact(root, &sub_key, "LoadAppInit_DLLs", view, &load, &after)?;
        if !registry_value_state_equals(&read_registry_value_state(root, &sub_key, "AppInit_DLLs", view), &dlls) {
            return Err("AppInit_DLLs changed while toggling its loader; rollback is required".into());
        }
        Ok(())
    })();
    if result.is_err() {
        disabled::remove(&disabled_id);
    }
    result
}

fn restore_advanced(item: &StartupItem) -> Result<(), String> {
    let record = disabled::decode_advanced(&item.status)?;
    let Some(root) = RegRoot::parse(&record.root_name) else { return Err("unsupported root".into()) };
    let view = RegView::parse(&record.registry_view);
    if let (Some(guard), name) = (&record.guard_before, record.guard_value_name.as_str()) {
        if !name.is_empty() {
            let current_guard = read_registry_value_state(root, &record.sub_key, name, view);
            if !registry_value_state_equals(&current_guard, guard) {
                return Err("A guarded registry value changed externally; refusing to restore over it".into());
            }
        }
    }
    write_registry_value_state_exact(root, &record.sub_key, &record.value_name, view, &record.after, &record.before)?;
    if let (Some(guard), name) = (&record.guard_before, record.guard_value_name.as_str()) {
        if !name.is_empty() && !registry_value_state_equals(&read_registry_value_state(root, &record.sub_key, name, view), guard) {
            return Err("Guarded registry value changed during restore; rollback is required".into());
        }
    }
    disabled::remove(&item.id);
    Ok(())
}

fn disable_registry(item: &StartupItem) -> Result<(), String> {
    let parts: Vec<&str> = item.id.split('|').collect();
    let scope = parts.get(1).copied().unwrap_or("User");
    let (root_name, sub_key, name, view_label) = if parts.len() >= 5 {
        (unb64(parts[2]), unb64(parts[3]), unb64(parts[4]),
         if parts.len() > 5 { parts[5].to_string() } else { item.registry_view.clone() })
    } else {
        (if scope == "Machine" { RegRoot::LocalMachine.name().to_string() } else { RegRoot::CurrentUser.name().to_string() },
         r"Software\Microsoft\Windows\CurrentVersion\Run".to_string(), unb64(parts.get(2).copied().unwrap_or("")), item.registry_view.clone())
    };
    let Some(root) = RegRoot::parse(&root_name) else { return Err("unsupported root".into()) };
    let view = RegView::parse(&view_label);
    let Some(handle) = open_subkey_write(root, &sub_key, view) else {
        return Err(format!("Registry startup key missing: {}", sub_key));
    };
    let Some((kind, value)) = handle.value(&name) else {
        return Err(format!("Registry startup value missing: {}", name));
    };
    let payload = value.serialize();
    let metadata = format!("{}\t{}\t{}\t{}\t{}\t{}\t{}", root.name(), sub_key, name, kind.as_str(), item.source, b64(&payload), view_label);
    let disabled_id = disabled::add("reg", &item.name, scope, &item.command, &item.location, &metadata);
    if !handle.delete_value(&name) {
        disabled::remove(&disabled_id);
        return Err(format!("Registry value delete failed: {}", name));
    }
    Ok(())
}

fn restore_registry(item: &StartupItem) -> Result<(), String> {
    let meta: Vec<&str> = item.status.split('\t').collect();
    let (root, sub_key, value_name, kind_index, view_label) = if meta.len() >= 4 && meta[0].starts_with("HKEY_") {
        (RegRoot::parse(meta[0]).ok_or("unsupported root")?, meta[1].to_string(), meta[2].to_string(), 3usize, if meta.len() > 6 { meta[6].to_string() } else { String::new() })
    } else {
        (if item.scope == "Machine" { RegRoot::LocalMachine } else { RegRoot::CurrentUser },
         r"Software\Microsoft\Windows\CurrentVersion\Run".to_string(),
         meta.first().copied().unwrap_or(&item.name).to_string(), 1usize, String::new())
    };
    let view = RegView::parse(&view_label);
    let kind = if meta.len() > kind_index { RegKind::parse(meta[kind_index]) } else { RegKind::String };
    let restored: RegValue = if meta.len() > 5 && !meta[5].trim().is_empty() {
        deserialize_registry_value(&unb64(meta[5]), kind).ok_or("stored payload undecodable")?
    } else {
        RegValue::Str(item.command.clone())
    };
    let Some(handle) = create_subkey(root, &sub_key, view) else {
        return Err(format!("Could not open the registry startup key: {}", sub_key));
    };
    if handle.value_names().iter().any(|n| n.eq_ignore_ascii_case(&value_name)) {
        // Vendor re-created the value: the live registration is authoritative; retire the record.
        disabled::remove(&item.id);
        return Ok(());
    }
    if !handle.set_value(&value_name, kind, &restored) {
        return Err("Registry value write failed during restore".into());
    }
    disabled::remove(&item.id);
    Ok(())
}

fn disable_active_setup(item: &StartupItem) -> Result<(), String> {
    let parts: Vec<&str> = item.id.split('|').collect();
    if parts.len() < 4 { return Err("Malformed Active Setup id".into()) }
    let scope = parts[1];
    let view_label = if parts.len() > 4 { parts[4].to_string() } else { item.registry_view.clone() };
    let Some(root) = RegRoot::parse(&unb64(parts[2])) else { return Err("unsupported root".into()) };
    let view = RegView::parse(&view_label);
    let sub_key = unb64(parts[3]);
    let Some(handle) = open_subkey_write(root, &sub_key, view) else {
        return Err("Active Setup key missing".into());
    };
    let Some((kind, value)) = handle.value("StubPath") else {
        return Err("Active Setup StubPath missing".into());
    };
    let metadata = format!("{}\t{}\tStubPath\t{}\t{}\t{}", root.name(), sub_key, kind.as_str(), b64(&value.serialize()), view_label);
    let disabled_id = disabled::add("active", &item.name, scope, &item.command, &item.location, &metadata);
    if !handle.delete_value("StubPath") {
        disabled::remove(&disabled_id);
        return Err("Active Setup StubPath delete failed".into());
    }
    Ok(())
}

fn restore_active_setup(item: &StartupItem) -> Result<(), String> {
    let meta: Vec<&str> = item.status.split('\t').collect();
    if meta.len() < 4 { return Err("Stored Active Setup metadata is incomplete".into()) }
    let Some(root) = RegRoot::parse(meta[0]) else { return Err("unsupported root".into()) };
    let view = RegView::parse(if meta.len() > 5 { meta[5] } else { "" });
    let kind = if meta.len() > 3 { RegKind::parse(meta[3]) } else { RegKind::String };
    let restored: RegValue = if meta.len() > 4 && !meta[4].trim().is_empty() {
        deserialize_registry_value(&unb64(meta[4]), kind).ok_or("stored payload undecodable")?
    } else {
        RegValue::Str(item.command.clone())
    };
    let Some(handle) = create_subkey(root, meta[1], view) else {
        return Err(format!("Could not open the registry startup key: {}", meta[1]));
    };
    if handle.value_names().iter().any(|n| n.eq_ignore_ascii_case(meta[2])) {
        return Err("Active Setup StubPath was recreated externally; refusing to overwrite it".into());
    }
    if !handle.set_value(meta[2], kind, &restored) {
        return Err("Active Setup restore write failed".into());
    }
    disabled::remove(&item.id);
    Ok(())
}

fn disable_startup_folder(item: &StartupItem) -> Result<(), String> {
    let _ = std::fs::create_dir_all(msm_state::disabled_startup_folder());
    let source = &item.command;
    let file_name = source.rsplit('\\').next().unwrap_or("startup-file");
    let dest = msm_state::disabled_startup_folder().join(format!("{}.{}.disabled", file_name, now_ticks()));
    let disabled_id = disabled::add("folder", &item.name, &item.scope, &dest.to_string_lossy(), &item.location, source);
    if let Err(e) = std::fs::rename(source, &dest) {
        disabled::remove(&disabled_id);
        return Err(format!("Startup folder move failed: {}", e));
    }
    Ok(())
}

fn restore_startup_folder(item: &StartupItem) -> Result<(), String> {
    let original = &item.status;
    let disabled_path = &item.command;
    if std::path::Path::new(original).exists() {
        return Err(format!("A startup file now exists at the restore location; refusing to overwrite it: {}", original));
    }
    if let Some(parent) = std::path::Path::new(original).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::rename(disabled_path, original).map_err(|e| format!("Startup folder restore failed: {}", e))?;
    disabled::remove(&item.id);
    Ok(())
}

fn disable_service_or_driver(item: &StartupItem, kind: &str) -> Result<(), String> {
    let name = service_name_from_item(item);
    let original_start = msm_core::services::read_service_start_value(&name, -1);
    if !(0..=3).contains(&original_start) {
        return Err(format!("Could not read the exact startup type for service {}; nothing was changed", name));
    }
    let disabled_id = disabled::add(kind, &item.name, &item.scope, &item.command, &item.location, &format!("{}\t{}", name, original_start));
    // Demand start (3) keeps the service usable on demand; a hard 4 would make it unstartable.
    if let Err(e) = msm_core::services::set_service_start_value(&name, 3) {
        disabled::remove(&disabled_id);
        return Err(e);
    }
    Ok(())
}

fn restore_service_or_driver(item: &StartupItem) -> Result<(), String> {
    let meta: Vec<&str> = item.status.split('\t').collect();
    if meta.len() < 2 { return Err("Stored service metadata is incomplete".into()) }
    let start: i64 = meta[1].trim().parse().map_err(|_| "Stored service startup type is invalid; refusing to guess Automatic".to_string())?;
    if !(0..=3).contains(&start) { return Err("Stored service startup type is invalid".into()) }
    let current = msm_core::services::read_service_start_value(meta[0], -1);
    if !(0..=4).contains(&current) {
        return Err("Could not read the current service startup type; nothing was changed".into());
    }
    if current as i64 == start {
        disabled::remove(&item.id);
        return Ok(());
    }
    if current != 3 && current != 4 {
        return Err(format!("Service startup type changed externally; refusing to overwrite it (current={})", current));
    }
    msm_core::services::set_service_start_value(meta[0], start as i32)?;
    disabled::remove(&item.id);
    Ok(())
}

fn disable_wmi_binding(item: &StartupItem) -> Result<(), String> {
    let disabled_id = disabled::add("wmisub", &item.name, &item.scope, &item.command, &item.location, &item.status);
    if let Err(e) = msm_state::wmi_guard::delete_binding(&item.location) {
        disabled::remove(&disabled_id);
        return Err(e);
    }
    Ok(())
}

fn restore_wmi_binding(item: &StartupItem) -> Result<(), String> {
    let meta: Vec<&str> = item.status.split('\t').collect();
    if meta.len() < 2 || meta[0].trim().is_empty() || meta[1].trim().is_empty() {
        return Err("Stored WMI binding metadata is incomplete".into());
    }
    if msm_state::wmi_guard::binding_exists(&item.location).unwrap_or(false) {
        disabled::remove(&item.id);
        return Ok(());
    }
    msm_state::wmi_guard::create_binding(meta[0], meta[1])?;
    disabled::remove(&item.id);
    Ok(())
}

fn now_ticks() -> u128 {
    msm_core::util::tick_ms()
}

// ---- bulk transactions ----

fn bulk_mutation_identity(item: &StartupItem) -> String {
    let shared = msm_core::dedupe::shared_user_registry_physical_identity(item);
    if shared.is_empty() { item.id.clone() } else { shared }
}

fn bulk_source_key(item: &StartupItem) -> String {
    let id = &item.id;
    let parts: Vec<&str> = id.split('|').collect();
    let lower = id.to_lowercase();
    if lower.starts_with("winlogon|") && parts.len() >= 5 {
        return format!("winlogon|{}|{}|{}", parts[2], parts[3], parts[4]);
    }
    if lower.starts_with("advanced|") && parts.len() >= 4 {
        return format!("advanced|{}|{}|{}", parts[1], parts[2], parts[3]);
    }
    id.clone()
}

fn bulk_component_index(item: &StartupItem) -> i64 {
    let parts: Vec<&str> = item.id.split('|').collect();
    if parts.len() > 5 && parts[0].eq_ignore_ascii_case("winlogon") {
        return parts[5].parse().unwrap_or(-1);
    }
    if parts.len() > 4 && parts[0].eq_ignore_ascii_case("advanced") {
        return parts[4].parse().unwrap_or(-1);
    }
    -1
}

pub fn disable_all_routes(routes: &[StartupItem], expert_confirmed: bool) -> Result<usize, String> {
    set_all_routes_enabled(routes, false, expert_confirmed)
}

pub fn enable_all_routes(routes: &[StartupItem], expert_confirmed: bool) -> Result<usize, String> {
    set_all_routes_enabled(routes, true, expert_confirmed)
}

fn set_all_routes_enabled(routes: &[StartupItem], enabled_state: bool, expert_confirmed: bool) -> Result<usize, String> {
    // Collapse shared physical identities; order by source then component index descending.
    let mut candidates: Vec<StartupItem> = Vec::new();
    let mut identities: Vec<String> = Vec::new();
    for route in routes.iter().filter(|r| r.enabled != enabled_state) {
        let identity = bulk_mutation_identity(route);
        if identities.iter().any(|i| i.eq_ignore_ascii_case(&identity)) { continue }
        identities.push(identity);
        candidates.push(route.clone());
    }
    candidates.sort_by(|a, b| {
        bulk_source_key(a).to_lowercase().cmp(&bulk_source_key(b).to_lowercase())
            .then(bulk_component_index(b).cmp(&bulk_component_index(a)))
            .then_with(|| a.id.to_lowercase().cmp(&b.id.to_lowercase()))
    });
    if candidates.is_empty() {
        return Ok(0);
    }
    // Fail closed before the first write.
    for item in &candidates {
        ensure_mutation_authorized(item, expert_confirmed)?;
    }
    let owned = candidates.clone();
    msm_state::coordinator::run(move || {
        let mut applied: Vec<(StartupItem, MutationSnapshots)> = Vec::new();
        for item in &owned {
            let snapshots = MutationSnapshots {
                source: Some(capture_source(item, if enabled_state { "Enable" } else { "Disable" })),
                stores: capture_stores(),
                item_enabled: item.enabled,
                approval_data: item.approval_data.clone(),
            };
            let result = if enabled_state { enable_core(item) } else { disable_core(item) };
            match result {
                Ok(()) => applied.push((item.clone(), snapshots)),
                Err(error) => {
                    let mut rollback_errors = Vec::new();
                    for (_, snapshots) in applied.iter().rev() {
                        let errors = restore_all(snapshots);
                        rollback_errors.extend(errors);
                    }
                    let route = if item.location.is_empty() { &item.id } else { &item.location };
                    let action = if enabled_state { "enable" } else { "disable" };
                    if !rollback_errors.is_empty() {
                        return Err(format!("Bulk {} failed at {} and rollback was incomplete: {}. Cause: {}", action, route, rollback_errors.join(" | "), error));
                    }
                    return Err(format!("Bulk {} failed at {}; every earlier startup route and intent store was restored. Cause: {}", action, route, error));
                }
            }
        }
        Ok(applied.len())
    })?
}


/// C# `RegistryValueState::build` normalization shim for multi-string payloads.
trait RegistryValueStateExt {
    fn normalized(self, kind: RegKind) -> RegistryValueState;
}

impl RegistryValueStateExt for RegistryValueState {
    fn normalized(self, kind: RegKind) -> RegistryValueState {
        self
    }
}
