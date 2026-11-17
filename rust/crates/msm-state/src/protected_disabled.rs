//! ProtectedDisabledService — the boot-disabled guard. Row: 8 B64 columns
//! (Key, Id, Type, Name, Scope, Command, Location, Status).

use crate::{protected_disabled_store, store};
use msm_core::reg::{RegKind, RegRoot, RegView};
use msm_core::util::{b64, eq_ci, unb64};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Row {
    pub key: String,
    pub id: String,
    pub kind: String,
    pub name: String,
    pub scope: String,
    pub command: String,
    pub location: String,
    pub status: String,
}

/// C# `CanonicalizeKey` — normalize stored keys against their live metadata.
fn canonicalize_key(row: &Row) -> String {
    if row.key.to_lowercase().starts_with("advanced|")
        || row.id.to_lowercase().starts_with("disabled|winlogon|")
        || row.id.to_lowercase().starts_with("disabled|appinit|")
        || row.id.to_lowercase().starts_with("disabled|advanced|") {
        let item = msm_core::model::StartupItem { id: row.id.clone(), status: row.status.clone(), ..Default::default() };
        let advanced = advanced_protection_key_for_item(&item);
        if !advanced.is_empty() {
            return advanced;
        }
    }
    let meta: Vec<&str> = row.status.split('\t').collect();
    let lower_key = row.key.to_lowercase();
    if lower_key.starts_with("reg|") || row.kind.to_lowercase().starts_with("registry ") {
        if meta.len() >= 3 && meta[0].starts_with("HKEY_") {
            return msm_core::reg::registry_protection_key(&row.scope, meta[0], meta[1], meta[2], if meta.len() > 6 { meta[6] } else { "" });
        }
    }
    if lower_key.starts_with("active|") || row.kind == "Active Setup" {
        if meta.len() >= 2 {
            return msm_core::ids::active_setup_protection_key(&row.scope, meta[0], meta[1], if meta.len() > 5 { meta[5] } else { "" });
        }
    }
    if lower_key.starts_with("driver|") || row.kind == "System Driver" {
        if !meta.is_empty() {
            return msm_core::ids::service_protection_key("driver", meta[0]);
        }
    }
    if lower_key.starts_with("service|") || row.kind == "Windows Service" {
        if !meta.is_empty() {
            return msm_core::ids::service_protection_key("service", meta[0]);
        }
    }
    if lower_key.starts_with("wmisub|") || row.kind == "WMI Event Consumer" {
        return msm_core::ids::wmi_protection_key(&row.location);
    }
    if lower_key.starts_with("task|") {
        return format!("task|{}", row.location);
    }
    row.key.clone()
}

fn parse_rows(lines: Vec<String>) -> Vec<Row> {
    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() { continue; }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 8 { continue; }
        let mut row = Row {
            key: unb64(parts[0]), id: unb64(parts[1]), kind: unb64(parts[2]), name: unb64(parts[3]),
            scope: unb64(parts[4]), command: unb64(parts[5]), location: unb64(parts[6]), status: unb64(parts[7]),
        };
        row.key = canonicalize_key(&row);
        rows.push(row);
    }
    rows
}

fn serialize(rows: &[Row]) -> Vec<String> {
    rows.iter().map(|r| [
        b64(&r.key), b64(&r.id), b64(&r.kind), b64(&r.name), b64(&r.scope),
        b64(&r.command), b64(&r.location), b64(&r.status),
    ].join("\t")).collect()
}

pub fn load_rows() -> Vec<Row> {
    parse_rows(store::read_all_lines(&protected_disabled_store()))
}

fn mutate(update: impl FnOnce(Vec<Row>) -> Vec<Row>) {
    store::update_lines(&protected_disabled_store(), |lines| serialize(&update(parse_rows(lines))));
}

/// C# `Key(item)` — canonical protection identity for a route.
pub fn key_for_item(item: &msm_core::model::StartupItem) -> String {
    let id = &item.id;
    let lower = id.to_lowercase();
    if lower.starts_with("disabled|winlogon|") || lower.starts_with("disabled|appinit|") || lower.starts_with("disabled|advanced|") {
        return advanced_protection_key_for_item(item);
    }
    let meta: Vec<&str> = item.status.split('\t').collect();
    if lower.starts_with("disabled|reg|") {
        if meta.len() >= 4 && meta[0].starts_with("HKEY_") {
            return msm_core::reg::registry_protection_key(&item.scope, meta[0], meta[1], meta[2], if meta.len() > 6 { meta[6] } else { "" });
        }
        let root = if item.scope == "Machine" { "HKEY_LOCAL_MACHINE" } else { "HKEY_CURRENT_USER" };
        return msm_core::reg::registry_protection_key(&item.scope, root,
            r"Software\Microsoft\Windows\CurrentVersion\Run", if meta.is_empty() { &item.name } else { meta[0] }, "");
    }
    if lower.starts_with("disabled|active|") && meta.len() >= 2 {
        return msm_core::ids::active_setup_protection_key(&item.scope, meta[0], meta[1], if meta.len() > 5 { meta[5] } else { "" });
    }
    if lower.starts_with("disabled|service|") || lower.starts_with("disabled|driver|") {
        return msm_core::ids::service_protection_key(if lower.starts_with("disabled|driver|") { "driver" } else { "service" }, meta.first().copied().unwrap_or(""));
    }
    if lower.starts_with("disabled|wmisub|") {
        return msm_core::ids::wmi_protection_key(&item.location);
    }
    if lower.starts_with("disabled|folder|") {
        return format!("folder|{}|{}", item.scope, item.status);
    }
    id.clone()
}

fn advanced_protection_key_for_item(item: &msm_core::model::StartupItem) -> String {
    let id = &item.id;
    let lower = id.to_lowercase();
    if lower.starts_with("disabled|winlogon|") || lower.starts_with("disabled|appinit|") || lower.starts_with("disabled|advanced|") {
        if let Ok(record) = crate::disabled::decode_advanced(&item.status) {
            return msm_core::ids::advanced_protection_key(&record.root_name, &record.sub_key, &record.value_name, record.original_index, &record.registry_view);
        }
    }
    if lower.starts_with("winlogon|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() >= 7 {
            return msm_core::ids::advanced_protection_key(&msm_core::util::unb64(parts[2]), &msm_core::util::unb64(parts[3]), &msm_core::util::unb64(parts[4]), parts[5].parse().unwrap_or(-1), parts[6]);
        }
    }
    if lower.starts_with("advanced|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() >= 5 {
            return msm_core::ids::advanced_protection_key(&msm_core::util::unb64(parts[1]), &msm_core::util::unb64(parts[2]), &msm_core::util::unb64(parts[3]), parts[4].parse().unwrap_or(-1), &item.registry_view);
        }
    }
    if lower.starts_with("appinit|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() >= 3 {
            return msm_core::ids::advanced_protection_key("HKEY_LOCAL_MACHINE", &msm_core::util::unb64(parts[1]), "LoadAppInit_DLLs", -1, parts[2]);
        }
    }
    if lower.starts_with("extension|") {
        let parts: Vec<&str> = id.split('|').collect();
        if parts.len() >= 6 {
            let view = if item.registry_view.is_empty() { msm_core::reg::native_registry_view().label().to_string() } else { item.registry_view.clone() };
            return msm_core::ids::advanced_protection_key(&msm_core::util::unb64(parts[2]), &msm_core::util::unb64(parts[3]), &msm_core::util::unb64(parts[4]), -1, &view);
        }
    }
    String::new()
}

pub fn protect(item: &msm_core::model::StartupItem) {
    let key = key_for_item(item);
    if key.is_empty() { return; }
    let _ = std::fs::create_dir_all(crate::app_data());
    let row = Row { key: key.clone(), id: item.id.clone(), kind: item.source.clone(), name: item.name.clone(),
        scope: item.scope.clone(), command: item.command.clone(), location: item.location.clone(), status: item.status.clone() };
    mutate(|mut rows| {
        rows.retain(|r| !eq_ci(&r.key, &key));
        rows.push(row.clone());
        rows
    });
}

pub fn unprotect(item: &msm_core::model::StartupItem) {
    let key = key_for_item(item);
    if key.is_empty() { return; }
    unprotect_key(&key);
}

pub fn unprotect_key(key: &str) {
    if key.trim().is_empty() { return; }
    let owned = key.to_string();
    mutate(|rows| rows.into_iter().filter(|r| !eq_ci(&r.key, &owned)).collect());
}

pub fn is_protected(key: &str) -> bool {
    if key.trim().is_empty() { return false; }
    load_rows().iter().any(|r| eq_ci(&r.key, key))
}

/// `--protect-disabled` receipt.
pub fn protect_current_disabled() -> String {
    let mut count = 0usize;
    let agent_task = format!("{}MichStartupMasterApp", msm_core::scan::MANAGED_TASK_ROOT);
    let items = msm_core::scan::scan_all();
    for item in items.iter().filter(|x| !x.enabled
        && x.approval_root.trim().is_empty()
        && !x.id.starts_with("error|")
        && !x.id.starts_with("legacy|")
        && !eq_ci(&x.location, &agent_task)) {
        protect(item);
        count += 1;
    }
    format!("PROTECT_DISABLED count={} store={}", count, protected_disabled_store().display())
}

/// `--enforce-disabled` receipt + the guard engine itself.
pub fn enforce_protected() -> String {
    let mut actions = 0usize;
    let mut failures = 0usize;
    let rows = load_rows();
    for row in &rows {
        match enforce_row(row) {
            Ok(true) => actions += 1,
            Ok(false) => {}
            Err(_) => failures += 1,
        }
    }
    format!("ENFORCE_DISABLED protected={} actions={} failures={}", rows.len(), actions, failures)
}

fn enforce_row(row: &Row) -> Result<bool, String> {
    let key = row.key.to_lowercase();
    if key.starts_with("advanced|")
        || row.id.to_lowercase().starts_with("disabled|winlogon|")
        || row.id.to_lowercase().starts_with("disabled|appinit|")
        || row.id.to_lowercase().starts_with("disabled|advanced|") {
        return enforce_advanced(row.status.clone());
    }
    if key.starts_with("task|") {
        msm_core::managed::run_checked("schtasks.exe", &format!("/Change /TN {} /Disable", msm_core::util::q(&row.location)), 30_000)?;
        return Ok(true);
    }
    if key.starts_with("reg|") || row.kind == "Registry Run" {
        let meta: Vec<&str> = row.status.split('\t').collect();
        let (root, sub_key, value_name) = if meta.len() >= 4 && meta[0].starts_with("HKEY_") {
            (root_from(meta[0], if meta.len() > 6 { meta[6] } else { "" }), meta[1].to_string(), meta[2].to_string())
        } else {
            let value_name = meta.first().copied().unwrap_or(&row.name).to_string();
            let root = if row.scope.eq_ignore_ascii_case("Machine") { RegRoot::LocalMachine } else { RegRoot::CurrentUser };
            (root, r"Software\Microsoft\Windows\CurrentVersion\Run".to_string(), value_name)
        };
        if let Some(handle) = msm_core::reg::open_subkey_write(root, &sub_key, RegView::Default) {
            if handle.value_names().iter().any(|n| n.eq_ignore_ascii_case(&value_name)) {
                if handle.delete_value(&value_name) {
                    return Ok(true);
                }
            }
        }
        return Ok(false);
    }
    if key.starts_with("active|") || row.kind == "Active Setup" {
        let meta: Vec<&str> = row.status.split('\t').collect();
        if meta.len() < 3 { return Ok(false); }
        let root = root_from(meta[0], if meta.len() > 5 { meta[5] } else { "" });
        if let Some(handle) = msm_core::reg::open_subkey_write(root, meta[1], RegView::Default) {
            if handle.value_names().iter().any(|n| n.eq_ignore_ascii_case(meta[2])) {
                if handle.delete_value(meta[2]) {
                    return Ok(true);
                }
            }
        }
        return Ok(false);
    }
    if key.starts_with("service|") || key.starts_with("driver|") || row.kind == "Windows Service" || row.kind == "System Driver" {
        let service_name = row.status.split('\t').next().unwrap_or("").to_string();
        if service_name.is_empty() { return Ok(false); }
        let path = format!(r"SYSTEM\CurrentControlSet\Services\{}", service_name);
        if let Some(handle) = msm_core::reg::open_subkey_write(RegRoot::LocalMachine, &path, RegView::Default) {
            if let Some((_, msm_core::reg::RegValue::DWord(current))) = handle.value("Start") {
                if current == 4 {
                    return Ok(false);
                }
            }
            if handle.set_value("Start", RegKind::DWord, &msm_core::reg::RegValue::DWord(4)) {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if key.starts_with("folder|") || row.kind == "Startup Folder" {
        let original = if !row.status.trim().is_empty() { row.status.clone() } else { row.command.clone() };
        if !original.trim().is_empty() && std::path::Path::new(&original).is_file() {
            let _ = std::fs::create_dir_all(crate::disabled_startup_folder());
            let dest = crate::disabled_startup_folder().join(format!("{}.protected.{}.disabled",
                original.rsplit('\\').next().unwrap_or("file"), ticks()));
            if std::fs::rename(&original, &dest).is_ok() {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if key.starts_with("wmisub|") || row.kind == "WMI Event Consumer" {
        // Bindings are deleted through the WMI store; the quiet engine recreates them on enable.
        return crate::wmi_guard::delete_binding(&row.location).map(|deleted| deleted);
    }
    Ok(false)
}

fn ticks() -> u128 {
    msm_core::util::tick_ms()
}

fn root_from(name: &str, view_label: &str) -> RegRoot {
    let _ = view_label;
    RegRoot::parse(name).unwrap_or(RegRoot::LocalMachine)
}

fn enforce_advanced(status: String) -> Result<bool, String> {
    let record = crate::disabled::decode_advanced(&status)?;
    let Some(root) = RegRoot::parse(&record.root_name) else { return Err("unsupported root".into()) };
    let view = RegView::parse(&record.registry_view);
    use msm_core::reg::{read_registry_value_state, registry_value_state_equals, write_registry_value_state_exact};
    if let (Some(guard), name) = (&record.guard_before, record.guard_value_name.as_str()) {
        if !name.is_empty() && !registry_value_state_equals(&read_registry_value_state(root, &record.sub_key, name, view), guard) {
            return Err("Guarded registry value changed externally; protected disable was not enforced".into());
        }
    }
    let current = read_registry_value_state(root, &record.sub_key, &record.value_name, view);
    if registry_value_state_equals(&current, &record.after) {
        return Ok(false);
    }
    if !registry_value_state_equals(&current, &record.before) {
        return Err("Registry startup value changed externally; protected disable refuses to overwrite it".into());
    }
    write_registry_value_state_exact(root, &record.sub_key, &record.value_name, view, &record.before, &record.after)?;
    Ok(true)
}

/// Retire a disabled protection by its exact key (used when a task is deleted).
pub fn protect_key(key: &str, location: &str, kind: &str, status: &str) {
    if key.trim().is_empty() { return; }
    let _ = std::fs::create_dir_all(crate::app_data());
    let row = Row { key: key.to_string(), id: format!("task|{}", location), kind: kind.to_string(),
        name: location.rsplit('\\').next().unwrap_or(location).to_string(), scope: "User/System".into(),
        command: String::new(), location: location.to_string(), status: status.to_string() };
    mutate(|mut rows| {
        rows.retain(|r| !eq_ci(&r.key, key));
        rows.push(row.clone());
        rows
    });
}

pub fn store_path() -> &'static Path {
    Path::new("")
}
