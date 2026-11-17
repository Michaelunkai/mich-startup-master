//! Dedupe: Win32_StartupCommand mirrors, stable-id conflicts, disabled-row rules.

use crate::model::StartupItem;
use crate::util::eq_ci;

/// C# `NormBootKey` — lowercase, trimmed, quotes stripped, all whitespace removed.
pub fn norm_boot_key(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.to_lowercase().trim().trim_matches('"').chars() {
        if !ch.is_whitespace() {
            out.push(ch);
        }
    }
    out
}

/// C# `StartupCommandMirrorsNative`.
pub fn startup_command_mirrors_native(projection: &StartupItem, native: &StartupItem) -> bool {
    if projection.source != "Startup Command" || native.source == "Startup Command" {
        return false;
    }
    let same_name = eq_ci(&projection.name, &native.name)
        || (native.source == "Startup Folder"
            && eq_ci(&projection.name, native.name.rsplit('.').next().unwrap_or(&native.name)));
    if !same_name {
        return false;
    }
    if !eq_ci(&projection.scope, &native.scope) {
        return false;
    }
    let projection_folder = projection.location.to_lowercase().contains("startup");
    if projection_folder != (native.source == "Startup Folder") {
        return false;
    }
    if !projection_folder {
        let projection32 = projection.location.to_lowercase().contains("wow6432node");
        let native32 = eq_ci(&native.registry_view, "Registry32") || native.location.to_lowercase().contains("wow6432node");
        if projection32 != native32 {
            return false;
        }
    }
    if projection.command.trim() == native.command.trim() {
        return true;
    }
    match (mirror_command(projection), mirror_command(native)) {
        (Some((p_path, p_args)), Some((n_path, n_args))) => {
            eq_ci(&p_path, &n_path) && p_args == n_args
        }
        _ => false,
    }
}

fn mirror_command(item: &StartupItem) -> Option<(String, String)> {
    let mut target = String::new();
    let mut args = String::new();
    if item.source == "Startup Folder" && item.command.to_lowercase().ends_with(".lnk") {
        let (t, a) = crate::humans::resolve_shortcut(&item.command);
        target = t;
        args = a;
    }
    if target.trim().is_empty() {
        if let Some((t, a)) = crate::humans::try_decode_tray_payload(&item.command) {
            target = t;
            args = a;
        }
    }
    if target.trim().is_empty() {
        if let Some((t, a)) = crate::humans::try_split_command(&item.command) {
            target = t;
            args = a;
        } else {
            return None;
        }
    }
    let expanded = crate::util::expand_env(target.trim().trim_matches('"'));
    let normalized = crate::paths::expand_short_path_names(&crate::paths::normalize_observed_path(&expanded));
    if normalized.is_empty() {
        None
    } else {
        Some((normalized, args))
    }
}

fn inventory_rows_equivalent(left: &StartupItem, right: &StartupItem) -> bool {
    left.name == right.name && left.source == right.source && left.scope == right.scope
        && left.command == right.command && left.location == right.location && left.enabled == right.enabled
}

fn inventory_identity_conflict(left: &StartupItem, right: &StartupItem, conflict: usize) -> StartupItem {
    let original_id = if left.id.is_empty() { &right.id } else { &left.id };
    let mut item = crate::model::error_item(
        "Inventory Identity Conflict",
        right.scope.as_str(),
        if right.location.is_empty() { left.location.as_str() } else { right.location.as_str() },
        &format!("Two provider rows used the same stable id but disagreed. First={} | second={}", left.command, right.command));
    item.id = format!("error|Inventory Identity Conflict|{}|{}", crate::util::b64(original_id), conflict);
    item.name = "Conflicting startup identity".into();
    item.verified_state = "Unknown".into();
    item.evidence_reason = "Conflicting provider state was preserved as a visible error instead of silently choosing one row".into();
    item.status = format!("Duplicate stable id: {}", original_id);
    item.scope = if right.scope.is_empty() { left.scope.clone() } else { right.scope.clone() };
    item.location = if right.location.is_empty() { left.location.clone() } else { right.location.clone() };
    item
}

/// C# `Dedupe` — mirrors collapse; identical rows collapse; conflicts surface.
pub fn dedupe(items: Vec<StartupItem>) -> Vec<StartupItem> {
    let native_registrations: Vec<&StartupItem> = items.iter()
        .filter(|i| !i.id.starts_with("disabled|") && (i.source == "Registry Run" || i.source == "Startup Folder"))
        .collect();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut result: Vec<StartupItem> = Vec::new();
    let mut conflicts = 0usize;
    for item in items.iter().filter(|i| !i.id.starts_with("disabled|")) {
        if item.source == "Startup Command"
            && native_registrations.iter().any(|native| startup_command_mirrors_native(item, native)) {
            continue;
        }
        let key = if !item.id.trim().is_empty() {
            item.id.to_lowercase()
        } else {
            format!("{}|{}|{}", item.source, item.location, item.name).to_lowercase()
        };
        match seen.get(&key) {
            None => {
                seen.insert(key, result.len());
                result.push(item.clone());
            }
            Some(&prior_index) => {
                if !inventory_rows_equivalent(&result[prior_index], item) {
                    conflicts += 1;
                    result.push(inventory_identity_conflict(&result[prior_index], item, conflicts));
                }
            }
        }
    }
    for item in items.iter().filter(|i| i.id.starts_with("disabled|")) {
        let base_key = format!("{}|{}|{}", item.source, item.location, item.name).to_lowercase();
        let covered_live = result.iter().find(|live| {
            format!("{}|{}|{}", live.source, live.location, live.name).to_lowercase() == base_key
        });
        let exact_removed_component = item.id.starts_with("disabled|advanced|") || item.id.starts_with("disabled|winlogon|");
        let mut covered = covered_live.is_some() && !exact_removed_component;
        if let Some(covered_live) = covered_live {
            if covered && item.id.starts_with("disabled|appinit|") && !covered_live.enabled {
                let dead_id = covered_live.id.to_lowercase();
                seen.remove(&dead_id);
                result.retain(|r| r.id.to_lowercase() != dead_id);
                covered = false;
            }
        }
        if covered {
            continue;
        }
        let key = if !item.id.trim().is_empty() {
            item.id.to_lowercase()
        } else {
            format!("{}|{}|{}", item.source, item.location, item.name).to_lowercase()
        };
        match seen.get(&key) {
            None => {
                seen.insert(key, result.len());
                result.push(item.clone());
            }
            Some(&prior_index) => {
                if !inventory_rows_equivalent(&result[prior_index], item) {
                    conflicts += 1;
                    result.push(inventory_identity_conflict(&result[prior_index], item, conflicts));
                }
            }
        }
    }
    result
}

/// C# `SharedUserRegistryPhysicalIdentity` — HKCU/HKU shared-view collapse identity.
pub fn shared_user_registry_physical_identity(item: &StartupItem) -> String {
    let id = &item.id;
    let parts: Vec<&str> = id.split('|').collect();
    let mut root_name = String::new();
    let mut sub_key = String::new();
    let mut value_name = String::new();
    if parts.len() >= 5 && parts[0].eq_ignore_ascii_case("reg") {
        root_name = crate::util::unb64(parts[2]);
        sub_key = crate::util::unb64(parts[3]);
        value_name = crate::util::unb64(parts[4]);
    } else if parts.len() >= 4 && parts[0].eq_ignore_ascii_case("active") {
        root_name = crate::util::unb64(parts[2]);
        sub_key = crate::util::unb64(parts[3]);
        value_name = "StubPath".into();
    } else if id.to_lowercase().starts_with("disabled|reg|") {
        let meta: Vec<&str> = item.status.split('\t').collect();
        if meta.len() >= 3 && meta[0].starts_with("HKEY_") {
            root_name = meta[0].into();
            sub_key = meta[1].into();
            value_name = meta[2].into();
        }
    } else if id.to_lowercase().starts_with("disabled|active|") {
        let meta: Vec<&str> = item.status.split('\t').collect();
        if meta.len() >= 2 && meta[0].starts_with("HKEY_") {
            root_name = meta[0].into();
            sub_key = meta[1].into();
            value_name = "StubPath".into();
        }
    }
    let user_root = root_name.eq_ignore_ascii_case("HKEY_CURRENT_USER") || root_name.eq_ignore_ascii_case("HKCU")
        || root_name.eq_ignore_ascii_case("HKEY_USERS") || root_name.eq_ignore_ascii_case("HKU");
    if !user_root || sub_key.trim().is_empty() || value_name.trim().is_empty() {
        return String::new();
    }
    let lower_sub = sub_key.to_lowercase();
    let classes_redirected = lower_sub.contains(r"\software\classes\") || lower_sub.starts_with("software\\classes\\")
        || regex::Regex::new(r"^[^\\]+_classes(?:\\|$)").unwrap().is_match(&lower_sub);
    if classes_redirected {
        return String::new();
    }
    format!("shared-user-registry|{}|{}|{}",
        root_name.trim().to_uppercase(), sub_key.trim().trim_end_matches('\\').to_uppercase(), value_name.trim().to_uppercase())
}
