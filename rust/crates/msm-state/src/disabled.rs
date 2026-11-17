//! DisabledStoreService — the preserved disabled routes (`disabled-items.tsv`).
//! Row: `id \t type \t B64(name) \t B64(scope) \t B64(command) \t B64(location) \t B64(status)`.

use crate::{disabled_store, store};
use msm_core::model::StartupItem;
use msm_core::reg::{registry_value_state_equals, RegistryValueState};
use msm_core::util::{b64, eq_ci, unb64};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub fn add(kind: &str, name: &str, scope: &str, command: &str, location: &str, status: &str) -> String {
    let _ = std::fs::create_dir_all(crate::app_data());
    let id = format!("disabled|{}|{}", kind, new_guid_n());
    let record = [id.as_str(), kind, &b64(name), &b64(scope), &b64(command), &b64(location), &b64(status)].join("\t");
    store::update_lines(&disabled_store(), |mut lines| {
        lines.push(record.clone());
        lines
    });
    id
}

fn new_guid_n() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() as u128 + (d.as_secs() as u128) << 32).unwrap_or(0);
    let mixed = nanos ^ ((std::process::id() as u128) << 96) ^ msm_core::util::tick_ms() << 41;
    format!("{:032x}", mixed)
}

pub fn remove(id: &str) {
    if id.trim().is_empty() {
        return;
    }
    let prefix = format!("{}\t", id);
    store::update_lines(&disabled_store(), |lines| {
        lines.into_iter().filter(|l| !l.starts_with(&prefix)).collect()
    });
}

/// The stored advanced disabled-state record (System.Text.Json-compatible, PascalCase).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedDisabledState {
    #[serde(rename = "Version")]
    pub version: i32,
    #[serde(rename = "RouteType")]
    pub route_type: String,
    #[serde(rename = "Source")]
    pub source: String,
    #[serde(rename = "RootName")]
    pub root_name: String,
    #[serde(rename = "SubKey")]
    pub sub_key: String,
    #[serde(rename = "ValueName")]
    pub value_name: String,
    #[serde(rename = "RegistryView")]
    pub registry_view: String,
    #[serde(rename = "OriginalIndex")]
    pub original_index: i32,
    #[serde(rename = "Component")]
    pub component: String,
    #[serde(rename = "Before")]
    pub before: RegistryValueState,
    #[serde(rename = "After")]
    pub after: RegistryValueState,
    #[serde(rename = "GuardValueName")]
    #[serde(default)]
    pub guard_value_name: String,
    #[serde(rename = "GuardBefore")]
    #[serde(default)]
    pub guard_before: Option<RegistryValueState>,
    #[serde(rename = "RequiresExpertConfirmation")]
    pub requires_expert_confirmation: bool,
    #[serde(rename = "RequiresReboot")]
    pub requires_reboot: bool,
}

pub fn encode_advanced(state: &AdvancedDisabledState) -> String {
    serde_json::to_string(state).unwrap_or_default()
}

pub fn decode_advanced(status: &str) -> Result<AdvancedDisabledState, String> {
    serde_json::from_str(status).map_err(|e| format!("Stored advanced startup metadata is invalid: {}", e))
}

/// Source label for a stored record (spec 03 `SourceName`).
pub fn source_name(kind: &str, status: &str) -> String {
    match kind {
        "winlogon" => decode_advanced(status).map(|r| r.source).unwrap_or_else(|_| "Winlogon Autostart".into()),
        "appinit" => decode_advanced(status).map(|r| r.source).unwrap_or_else(|_| "AppInit DLLs".into()),
        "advanced" => decode_advanced(status).map(|r| r.source).unwrap_or_else(|_| "Advanced startup".into()),
        "folder" => "Startup Folder".into(),
        "service" => "Windows Service".into(),
        "driver" => "System Driver".into(),
        "active" => "Active Setup".into(),
        "wmisub" => "WMI Event Consumer".into(),
        "reg" => {
            let meta: Vec<&str> = status.split('\t').collect();
            if meta.len() >= 5 && !meta[4].trim().is_empty() { meta[4].to_string() } else { "Registry Run".into() }
        }
        other => other.to_string(),
    }
}

/// Hydrate one stored row into a StartupItem (advanced rows get live capability checks).
pub fn hydrate_row(parts: &[&str]) -> StartupItem {
    let id = parts[0].to_string();
    let kind = parts[1].to_string();
    let status = unb64(parts[6]);
    let mut item = StartupItem {
        id: id.clone(),
        name: unb64(parts[2]),
        source: source_name(&kind, &status),
        scope: unb64(parts[3]),
        command: unb64(parts[4]),
        location: unb64(parts[5]),
        status: status.clone(),
        enabled: false,
        can_disable: true,
        is_managed: true,
        ..StartupItem::new()
    };
    if matches!(kind.as_str(), "winlogon" | "appinit" | "advanced") {
        match hydrate_advanced(&mut item, &status) {
            Ok(()) => {}
            Err(message) => {
                item.can_disable = false;
                item.mutation_capability = "ReadOnly".into();
                item.mutation_reason = format!("Could not validate this advanced startup record against Windows: {}", message);
                item.requires_expert_confirmation = true;
            }
        }
    }
    item
}

fn hydrate_advanced(item: &mut StartupItem, status: &str) -> Result<(), String> {
    use msm_core::reg::{can_open_registry_value_for_write, read_registry_value_state, RegRoot, RegView};
    let record = decode_advanced(status)?;
    let Some(root) = RegRoot::parse(&record.root_name) else { return Err("unsupported root".into()) };
    let view = RegView::parse(&record.registry_view);
    let (writable, access_reason) = can_open_registry_value_for_write(root, &record.sub_key, &record.value_name, view);
    let current = read_registry_value_state(root, &record.sub_key, &record.value_name, view);
    let exact_guard = match (&record.guard_before, record.guard_value_name.as_str()) {
        (Some(guard), name) if !name.is_empty() => {
            registry_value_state_equals(&read_registry_value_state(root, &record.sub_key, name, view), guard)
        }
        _ => true,
    };
    let capability = msm_core::scan_registry::decide_registry_mutation_capability(&item.scope, &record.root_name, writable, msm_core::util::is_elevated(), "", Some(&record.sub_key));
    let exact_disabled_state = registry_value_state_equals(&current, &record.after);
    let can_restore = capability.can_mutate && exact_disabled_state && exact_guard;
    item.source = record.source.clone();
    item.registry_view = record.registry_view.clone();
    item.can_disable = can_restore;
    item.mutation_capability = if can_restore { "TransactionalRegistryRestore".into() } else { "ReadOnly".into() };
    item.mutation_reason = if !exact_guard {
        "A guarded registry value changed externally; refresh and review before restoring".into()
    } else if !exact_disabled_state {
        "The registry startup value no longer matches the manager-created disabled snapshot; refresh and review before restoring".into()
    } else if can_restore {
        "Restore the exact original registry value only while the disabled state and guard still match".into()
    } else {
        format!("{}{}", capability.reason, if access_reason.is_empty() { String::new() } else { format!(": {}", access_reason) })
    };
    item.requires_expert_confirmation = record.requires_expert_confirmation;
    item.requires_elevation = capability.requires_elevation;
    item.requires_reboot = record.requires_reboot;
    item.external_authority = capability.external_authority;
    Ok(())
}

pub fn load_disabled_items() -> Vec<StartupItem> {
    let mut list = Vec::new();
    for line in store::read_all_lines(&disabled_store()) {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 7 {
            continue;
        }
        list.push(hydrate_row(&parts));
    }
    list
}

/// `AdvancedDisabledSourceName` helper re-export.
pub fn advanced_disabled_source_name(kind: &str, status: &str) -> String {
    if !matches!(kind, "winlogon" | "appinit" | "advanced") {
        return kind.to_string();
    }
    decode_advanced(status).map(|r| r.source).unwrap_or_else(|_| match kind {
        "winlogon" => "Winlogon Autostart".into(),
        "appinit" => "AppInit DLLs".into(),
        _ => "Advanced startup".into(),
    })
}

/// Retire a stale record only when it matches a live physical registration.
pub fn legacy_disabled_registry_identity(disabled: &StartupItem) -> String {
    if !disabled.id.to_lowercase().starts_with("disabled|reg|") {
        return String::new();
    }
    let name = disabled.name.trim().to_string();
    if name.is_empty() {
        return String::new();
    }
    let location = disabled.location.clone();
    if let Some(captures) = regex::Regex::new(r"(?i)^(?<root>HKEY_[A-Z_]+|HKCU|HKU|HKLM)\\(?<subkey>[^:]+?)(?:\s+\[(Registry\d+)\])?$").unwrap().captures(&location) {
        let root = captures.name("root").map(|m| m.as_str().trim().to_uppercase()).unwrap_or_default();
        let sub_key = captures.name("subkey").map(|m| m.as_str().trim().trim_end_matches('\\').to_uppercase()).unwrap_or_default();
        return format!("shared-user-registry|{}|{}|{}", root, sub_key, name.to_uppercase());
    }
    String::new()
}

/// Path hint for callers that need the store file location.
pub fn store_path() -> &'static Path {
    Path::new("")
}
