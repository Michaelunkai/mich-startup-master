//! Explorer StartupApproved overlay: read, match, apply, and toggle.

use crate::ids;
use crate::model::{error_item, StartupItem};
use crate::reg::*;
use crate::scan_registry::decide_registry_mutation_capability;
use crate::util::eq_ci;

pub struct StartupApproval {
    pub root: RegRoot,
    pub root_name: String,
    pub path: String,
    pub scope: String,
    pub kind: String,
    pub name: String,
    pub data: Vec<u8>,
    pub registry_view: String,
}

/// C# `TryDecodeStartupApproval` — exact 12-byte state shape only.
pub fn try_decode_startup_approval(data: &[u8]) -> Result<bool, String> {
    if data.len() != 12 {
        return Err(format!("Unsupported StartupApproved byte length {}; expected 12", data.len()));
    }
    if data[1] != 0 || data[2] != 0 || data[3] != 0 || (data[0] & 0xF8) != 0 {
        return Err(format!("Unsupported StartupApproved state field 0x{:02X}{:02X}{:02X}{:02X}", data[0], data[1], data[2], data[3]));
    }
    Ok((data[0] & 0x01) != 0)
}

pub fn apply_startup_approval_state(original: &[u8], enabled: bool) -> Result<Vec<u8>, String> {
    try_decode_startup_approval(original)?;
    let mut data = original.to_vec();
    data[0] = if enabled { data[0] & 0xFE } else { data[0] | 0x01 };
    Ok(data)
}

/// C# `ReadStartupApproved` — all roots, all kinds, one native view.
pub fn read_startup_approved() -> Vec<StartupApproval> {
    let mut result = Vec::new();
    let view = native_registry_view();
    let view_label = view.label();
    let kinds = ["Run", "Run32", "StartupFolder", "StartupFolder32", "StartupTasks"];
    let mut read_root = |root: RegRoot, scope: &str, prefix: &str, result: &mut Vec<StartupApproval>| {
        for kind in kinds {
            let path = format!("{}Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\{}", prefix, kind);
            let Some(key) = open_subkey(root, &path, view) else { continue };
            for name in key.value_names() {
                let Some((_, value)) = key.value(&name) else { continue };
                let data = match value {
                    RegValue::Bytes(bytes) => bytes,
                    _ => continue,
                };
                if data.is_empty() {
                    continue;
                }
                result.push(StartupApproval {
                    root,
                    root_name: root.name().to_string(),
                    path: path.clone(),
                    scope: scope.into(),
                    kind: kind.into(),
                    name,
                    data,
                    registry_view: view_label.into(),
                });
            }
        }
    };
    read_root(RegRoot::CurrentUser, "User", "", &mut result);
    read_root(RegRoot::LocalMachine, "Machine", "", &mut result);
    for (sid, scope) in crate::hive::loaded_user_hives() {
        read_root(RegRoot::Users, &scope, &format!("{}\\", sid), &mut result);
    }
    result
}

/// C# `StartupApprovalMatches`.
fn startup_approval_matches(item: &StartupItem, approval: &StartupApproval) -> bool {
    if !eq_ci(&item.scope, &approval.scope) {
        return false;
    }
    let item_name = &item.name;
    if item.source == "Startup Folder" {
        if !approval.kind.to_lowercase().starts_with("startupfolder") {
            return false;
        }
        let file_name = item.command.rsplit('\\').next().unwrap_or(item_name);
        return eq_ci(file_name, &approval.name) || eq_ci(item_name, &approval.name);
    }
    if item.source == "Packaged Startup Task" {
        if !eq_ci(&approval.kind, "StartupTasks") {
            return false;
        }
        if eq_ci(item_name, &approval.name) {
            return true;
        }
        return item_name.rfind('!').map(|bang| eq_ci(&item_name[bang + 1..], &approval.name)).unwrap_or(false);
    }
    if item.source == "Startup Command" {
        let wmi_wow = item.location.to_lowercase().contains("wow6432node");
        if wmi_wow != eq_ci(&approval.kind, "Run32") {
            return false;
        }
        if !wmi_wow && !eq_ci(&approval.kind, "Run") {
            return false;
        }
        return eq_ci(item_name, &approval.name);
    }
    if item.source != "Registry Run" {
        return false;
    }
    let wow = eq_ci(&item.registry_view, "Registry32") || item.location.to_lowercase().contains("wow6432node");
    if wow != eq_ci(&approval.kind, "Run32") {
        return false;
    }
    if !wow && !eq_ci(&approval.kind, "Run") {
        return false;
    }
    eq_ci(item_name, &approval.name)
}

/// C# `ApplyStartupApproved` — overlay on canonical rows; orphan/malformed handling.
pub fn apply_startup_approved(items: &mut Vec<StartupItem>, approvals: &[StartupApproval]) {
    for approval in approvals {
        let decoded = match try_decode_startup_approval(&approval.data) {
            Ok(disabled) => disabled,
            Err(reason) => {
                items.push(StartupItem {
                    id: format!("error|Startup Approval|{}", crate::util::b64(&format!("{}|{}|{}", approval.root_name, approval.path, approval.name))),
                    name: format!("Unreadable startup approval: {}", approval.name),
                    app_name: approval.name.clone(),
                    source: "Startup Approval".into(),
                    scope: approval.scope.clone(),
                    command: format!("Raw bytes: {}", approval.data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join("-")),
                    location: format!("{}\\{} :: {}", approval.root_name, approval.path, approval.name),
                    enabled: false,
                    can_disable: false,
                    verified_state: "Unknown".into(),
                    evidence_reason: reason.clone(),
                    mutation_capability: "ReadOnly".into(),
                    mutation_reason: reason,
                    status: "Malformed or unsupported Explorer StartupApproved metadata".into(),
                    ..StartupItem::new()
                });
                continue;
            }
        };
        let matches: Vec<usize> = items.iter().enumerate()
            .filter(|(_, item)| startup_approval_matches(item, approval))
            .map(|(i, _)| i)
            .collect();
        let native_matches: Vec<usize> = matches.iter().copied()
            .filter(|&i| items[i].source == "Registry Run" || items[i].source == "Startup Folder")
            .collect();
        let matches: Vec<usize> = if !native_matches.is_empty() {
            matches.into_iter().filter(|&i| items[i].source != "Startup Command"
                || !native_matches.iter().any(|&n| crate::dedupe::startup_command_mirrors_native(&items[i], &items[n]))).collect()
        } else {
            matches
        };
        if matches.len() > 1 {
            let candidate_ids: Vec<String> = matches.iter().map(|&i| items[i].id.clone()).collect();
            items.push(StartupItem {
                id: format!("error|Startup Approval Ambiguity|{}", crate::util::b64(&format!("{}|{}|{}", approval.root_name, approval.path, approval.name))),
                name: format!("Ambiguous startup approval: {}", approval.name),
                app_name: approval.name.clone(),
                source: "Startup Approval".into(),
                scope: approval.scope.clone(),
                command: format!("{} candidate registrations: {}", matches.len(), candidate_ids.join(" | ")),
                location: format!("{}\\{} :: {}", approval.root_name, approval.path, approval.name),
                enabled: false,
                can_disable: false,
                verified_state: "Unknown".into(),
                evidence_reason: "One metadata value matched multiple startup registrations; no arbitrary overlay was applied".into(),
                mutation_capability: "ReadOnly".into(),
                mutation_reason: "Refresh or repair the ambiguous registrations before changing this metadata".into(),
                status: "Ambiguous Explorer StartupApproved metadata".into(),
                ..StartupItem::new()
            });
            continue;
        }
        if let Some(index) = matches.into_iter().next() {
            let item = &mut items[index];
            item.approval_root = approval.root_name.clone();
            item.approval_path = approval.path.clone();
            item.approval_name = approval.name.clone();
            item.approval_data = Some(approval.data.clone());
            item.approval_registry_view = if approval.registry_view.is_empty() { native_registry_view().label().into() } else { approval.registry_view.clone() };
            item.enabled = !decoded;
            let view = RegView::parse(&item.approval_registry_view);
            let (writable, access_reason) = can_open_registry_value_for_write(approval.root, &approval.path, &approval.name, view);
            let capability = decide_registry_mutation_capability(&approval.scope, &approval.root_name, writable, crate::util::is_elevated(), "", Some(&approval.path));
            item.can_disable = capability.can_mutate;
            item.mutation_capability = if capability.can_mutate { "StartupApprovedOverlay".into() } else { "ReadOnly".into() };
            item.mutation_reason = if capability.can_mutate {
                "Toggle the existing exact StartupApproved low enable bit; preserve every other metadata byte".into()
            } else {
                format!("{}{}", capability.reason, if access_reason.is_empty() { String::new() } else { format!(": {}", access_reason) })
            };
            item.requires_elevation = capability.requires_elevation;
            item.external_authority = capability.external_authority;
            let state = if decoded { "disabled".to_string() } else { format!("state 0x{:02X}", approval.data.first().copied().unwrap_or(0)) };
            item.status = format!("{}; Explorer StartupApproved={}", item.status, state);
            continue;
        }
        if !decoded {
            let view = RegView::parse(if approval.registry_view.is_empty() { native_registry_view().label() } else { &approval.registry_view });
            let (writable, access_reason) = can_open_registry_value_for_write(approval.root, &approval.path, &approval.name, view);
            let capability = decide_registry_mutation_capability(&approval.scope, &approval.root_name, writable, crate::util::is_elevated(), "", Some(&approval.path));
            items.push(StartupItem {
                id: ids::startupapproval_id(&approval.scope, &approval.root_name, &approval.path, &approval.name),
                name: approval.name.clone(),
                app_name: approval.name.clone(),
                source: "Startup Approval".into(),
                scope: approval.scope.clone(),
                command: "Windows did not expose the launch command for this enabled startup approval".into(),
                location: format!("{}\\{} :: {}", approval.root_name, approval.path, approval.name),
                enabled: true,
                can_disable: capability.can_mutate,
                approval_root: approval.root_name.clone(),
                approval_path: approval.path.clone(),
                approval_name: approval.name.clone(),
                approval_data: Some(approval.data.clone()),
                approval_registry_view: if approval.registry_view.is_empty() { native_registry_view().label().into() } else { approval.registry_view.clone() },
                mutation_capability: if capability.can_mutate { "StartupApprovedOverlay".into() } else { "ReadOnly".into() },
                mutation_reason: if capability.can_mutate {
                    "Toggle the existing exact StartupApproved low enable bit; preserve every other metadata byte".into()
                } else {
                    format!("{}{}", capability.reason, if access_reason.is_empty() { String::new() } else { format!(": {}", access_reason) })
                },
                requires_elevation: capability.requires_elevation,
                external_authority: capability.external_authority,
                status: format!("Enabled Explorer StartupApproved state=0x{:02X}; native and WMI launch command unavailable", approval.data.first().copied().unwrap_or(0)),
                ..StartupItem::new()
            });
        } else {
            items.push(StartupItem {
                id: ids::staleapproval_id(&approval.scope, &approval.root_name, &approval.path, &approval.name),
                name: format!("Stale metadata: {}", approval.name),
                app_name: "Startup metadata warning".into(),
                source: "Stale Startup Metadata".into(),
                scope: approval.scope.clone(),
                command: format!("No matching live {} registration", approval.kind),
                location: format!("{}\\{}", approval.root_name, approval.path),
                enabled: false,
                can_disable: false,
                status: format!("Stale Explorer StartupApproved metadata; state=0x{:02X}; no Windows startup source was found", approval.data.first().copied().unwrap_or(0)),
                ..StartupItem::new()
            });
        }
    }
}

/// C# `SetStartupApproval` — exact-state toggle of the low enable bit.
pub fn set_startup_approval(item: &mut StartupItem, enabled: bool) -> Result<(), String> {
    if item.approval_root.trim().is_empty() || item.approval_path.trim().is_empty() || item.approval_name.trim().is_empty() {
        return Err("Startup approval metadata is incomplete".into());
    }
    let Some(root) = RegRoot::parse(&item.approval_root) else {
        return Err(format!("Unsupported registry root: {}", item.approval_root));
    };
    let view = RegView::parse(if item.approval_registry_view.is_empty() { native_registry_view().label() } else { &item.approval_registry_view });
    let key = open_subkey_write(root, &item.approval_path, view)
        .ok_or_else(|| "Startup approval state is missing; refusing to invent metadata".to_string())?;
    if !key.value_names().iter().any(|name| eq_ci(name, &item.approval_name)) {
        return Err("Startup approval state is missing; refusing to invent metadata".into());
    }
    let Some((kind, RegValue::Bytes(current))) = key.value(&item.approval_name) else {
        return Err("Startup approval state is missing; refusing to invent metadata".into());
    };
    if kind != RegKind::Binary {
        return Err("Startup approval state changed kind; refusing to coerce it".into());
    }
    if current.is_empty() {
        return Err("Startup approval state is empty; refusing to invent metadata".into());
    }
    let expected = item.approval_data.clone().unwrap_or_default();
    if expected != current {
        return Err("Startup approval state changed after scanning; refresh before changing it".into());
    }
    let data = apply_startup_approval_state(&current, enabled)?;
    if !key.set_value(&item.approval_name, RegKind::Binary, &RegValue::Bytes(data.clone())) {
        return Err("Startup approval write failed".into());
    }
    match key.value(&item.approval_name) {
        Some((RegKind::Binary, RegValue::Bytes(verified))) if verified == data => {}
        _ => return Err("Startup approval state did not verify after writing".into()),
    }
    item.approval_data = Some(data);
    item.enabled = enabled;
    Ok(())
}

#[allow(unused)]
fn _error_item_hint(source: &str, scope: &str, location: &str, message: &str) -> StartupItem {
    error_item(source, scope, location, message)
}
