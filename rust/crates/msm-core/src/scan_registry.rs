//! Primary registry surfaces: Run family, Winlogon, Active Setup, AppInit,
//! BootExecute/LSA, RunOnceEx, per-user loaded hives.

use crate::hive::{is_service_account_sid, loaded_user_hives, service_profile_state, SERVICE_ACCOUNT_PROFILES};
use crate::ids;
use crate::model::{error_item, StartupItem};
use crate::reg::*;
use crate::util::{b64, eq_ci, expand_env, is_elevated};
use crate::views::{root_display, source_for_registry_path, RUN_KEYS};

/// C# `DecideRegistryMutationCapability`.
pub struct RegistryCapability {
    pub can_mutate: bool,
    pub requires_elevation: bool,
    pub reason: String,
    pub external_authority: String,
}

pub fn decide_registry_mutation_capability(scope: &str, root_name: &str, writable: bool, elevated: bool,
                                            external_authority: &str, target_sub_key: Option<&str>) -> RegistryCapability {
    let root = RegRoot::parse(root_name);
    let current_user_hive = matches!(root, Some(RegRoot::CurrentUser));
    let users_hive = matches!(root, Some(RegRoot::Users));
    let mut current_user_hku = false;
    if users_hive && eq_ci(scope, "User") {
        if let Some(sub) = target_sub_key {
            let current_sid = crate::util::current_user_sid();
            if !current_sid.is_empty() {
                current_user_hku = sub.eq_ignore_ascii_case(&current_sid)
                    || sub.to_lowercase().starts_with(&format!("{}\\", current_sid.to_lowercase()));
            }
        }
    }
    let current_user = current_user_hive || current_user_hku;
    let requires_elevation = !current_user;
    if !external_authority.trim().is_empty() {
        return RegistryCapability { can_mutate: false, requires_elevation, external_authority: external_authority.into(),
            reason: format!("Owned by {}; change it through that authority", external_authority) };
    }
    if requires_elevation && !elevated {
        return RegistryCapability { can_mutate: false, requires_elevation: true, external_authority: String::new(),
            reason: "Elevation and a successful registry write preflight are required".into() };
    }
    if writable {
        return RegistryCapability { can_mutate: true, requires_elevation, external_authority: String::new(),
            reason: if requires_elevation { "Writable with the current elevated security token".into() } else { "Writable by the current user".into() } };
    }
    RegistryCapability { can_mutate: false, requires_elevation, external_authority: String::new(),
        reason: if requires_elevation {
            if elevated { "The registry value denied write access even to the elevated process".into() }
            else { "Elevation and write permission are required".into() }
        } else {
            "The current user does not have write permission for this registry value".into()
        } }
}

/// C# `AddRegistryValues` — the primary Run-family collector.
#[allow(clippy::too_many_arguments)]
pub fn add_registry_values(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str,
                            source: &str, can_edit_value: bool, status: &str, value_filter: Option<&[&str]>, view_label: &str) {
    let key = match open_subkey(root, sub_key, view) {
        Some(key) => key,
        None => return,
    };
    for name in key.value_names() {
        if let Some(filter) = value_filter {
            if !filter.iter().any(|f| eq_ci(f, &name)) {
                continue;
            }
        }
        let Some((kind, value)) = key.value(&name) else { continue };
        let command = value.display();
        if command.trim().is_empty() {
            continue;
        }
        let display_name = if name.is_empty() { "(Default)".to_string() } else { name.clone() };
        let status_text = if view_label.is_empty() { status.to_string() } else { format!("{}; {}", status, view_label) };
        items.push(StartupItem {
            id: ids::reg_id(scope, root_display(root), sub_key, &name, view_label),
            name: display_name,
            source: source.into(),
            scope: scope.into(),
            command,
            location: format!("{}\\{}{}", root_display(root), sub_key, if view_label.is_empty() { String::new() } else { format!(" [{}]", view_label) }),
            enabled: true,
            can_disable: can_edit_value && (scope.starts_with("User") || is_elevated()),
            status: status_text,
            registry_view: view_label.into(),
            ..StartupItem::new()
        });
    }
}

/// C# `AddRunOnceEx`.
pub fn add_run_once_ex(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str, base_path: Option<&str>, view_label: &str) {
    let base = base_path.unwrap_or(r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnceEx");
    let parent = match open_subkey(root, base, view) {
        Some(key) => key,
        None => return,
    };
    for child_name in parent.subkey_names() {
        let child_path = format!("{}\\{}", base, child_name);
        let Some(child) = open_subkey(root, &child_path, view) else { continue };
        for value_name in child.value_names() {
            if ["Title", "Flags", "Depend"].iter().any(|f| eq_ci(f, &value_name)) {
                continue;
            }
            let Some((_, value)) = child.value(&value_name) else { continue };
            let command = value.display();
            if command.trim().is_empty() {
                continue;
            }
            let name = if value_name.is_empty() { child_name.clone() } else { value_name.clone() };
            items.push(StartupItem {
                id: ids::reg_id(scope, root_display(root), &child_path, &value_name, view_label),
                name,
                source: "Registry RunOnceEx".into(),
                scope: scope.into(),
                command,
                location: format!("{}\\{}{}", root_display(root), child_path, if view_label.is_empty() { String::new() } else { format!(" [{}]", view_label) }),
                enabled: true,
                can_disable: scope.starts_with("User") || is_elevated(),
                status: if view_label.is_empty() { "One-time Explorer RunOnceEx command".into() } else { format!("One-time Explorer RunOnceEx command; {}", view_label) },
                registry_view: view_label.into(),
                ..StartupItem::new()
            });
        }
    }
}

/// C# `TrySplitWinlogonComponents` — split on unquoted commas; unbalanced quotes fail.
pub fn try_split_winlogon_components(raw: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in raw.chars() {
        if ch == '"' {
            quoted = !quoted;
        }
        if ch == ',' && !quoted {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if quoted {
        return None;
    }
    parts.push(current);
    Some(parts)
}

/// Protected Windows defaults inside Winlogon values.
pub fn is_protected_windows_winlogon_component(value_name: &str, component: &str) -> bool {
    let text = expand_env(component.trim());
    let text = if text.starts_with('"') {
        match text[1..].find('"') {
            Some(close) if close > 0 => text[1..1 + close].to_string(),
            _ => text.trim_matches('"').to_string(),
        }
    } else {
        match text.find([' ', '\t']) {
            Some(space) if space > 0 => text[..space].to_string(),
            _ => text.clone(),
        }
    };
    let leaf = text.trim().trim_matches('"').rsplit('\\').next().unwrap_or("").to_string();
    if eq_ci(value_name, "Shell") && eq_ci(&leaf, "explorer.exe") {
        return true;
    }
    if eq_ci(value_name, "Userinit") && eq_ci(&leaf, "userinit.exe") {
        return true;
    }
    eq_ci(&leaf, "winlogon.exe") || eq_ci(&leaf, "lsass.exe")
}

/// C# `AddWinlogonValues` — component-level Winlogon rows with capability metadata.
pub fn add_winlogon_values(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str,
                            value_filter: &[&str], view_label: &str) {
    let key = match open_subkey(root, sub_key, view) {
        Some(key) => key,
        None => return,
    };
    for value_name in key.value_names() {
        if !value_filter.iter().any(|f| eq_ci(f, &value_name)) {
            continue;
        }
        let Some((kind, value)) = key.value(&value_name) else { continue };
        if !matches!(kind, RegKind::String | RegKind::ExpandString) {
            items.push(error_item("Winlogon Autostart", scope, &format!("{}\\{}\\{}", root_display(root), sub_key, value_name),
                &format!("Expected REG_SZ or REG_EXPAND_SZ, found {}", kind.as_str())));
            continue;
        }
        let raw = value.display();
        let Some(components) = try_split_winlogon_components(&raw) else {
            items.push(error_item("Winlogon Autostart", scope, &format!("{}\\{}\\{}", root_display(root), sub_key, value_name),
                "Quoted component list is ambiguous; no rewrite is offered"));
            continue;
        };
        let (writable, access_reason) = can_open_registry_value_for_write(root, sub_key, &value_name, view);
        let capability = decide_registry_mutation_capability(scope, root_display(root), writable, is_elevated(), "", Some(sub_key));
        let visible_count = components.iter().filter(|c| !c.trim().is_empty()).count();
        for (index, component) in components.iter().enumerate() {
            let component = component.trim();
            if component.is_empty() {
                continue;
            }
            let protected_default = is_protected_windows_winlogon_component(&value_name, component);
            let reason = if protected_default {
                "Protected Windows Winlogon default; removing it could prevent a usable sign-in".to_string()
            } else if capability.can_mutate {
                "Exact third-party component can be removed transactionally; reboot/sign-in is required".to_string()
            } else {
                format!("{}{}", capability.reason, if access_reason.is_empty() { String::new() } else { format!(": {}", access_reason) })
            };
            items.push(StartupItem {
                id: ids::winlogon_id(scope, root_display(root), sub_key, &value_name, index, view_label),
                name: format!("{}{}", value_name, if visible_count > 1 { format!(" #{}", index + 1) } else { String::new() }),
                source: "Winlogon Autostart".into(),
                scope: scope.into(),
                command: component.to_string(),
                location: format!("{}\\{}\\{} [{}]", root_display(root), sub_key, value_name, view_label),
                enabled: true,
                can_disable: capability.can_mutate && !protected_default,
                status: format!("Winlogon {} component index={}; exact {}; {}", value_name, index, kind.as_str(), reason),
                registry_view: view_label.into(),
                mutation_capability: if protected_default { "ProtectedWindowsDefault".into() }
                    else if capability.can_mutate { "TransactionalRegistryValue".into() } else { "ReadOnly".into() },
                mutation_reason: reason,
                requires_expert_confirmation: !protected_default,
                requires_elevation: capability.requires_elevation,
                requires_reboot: true,
                external_authority: capability.external_authority.clone(),
                ..StartupItem::new()
            });
        }
    }
}

/// C# `AddAppInit` — the loader-toggle surface.
pub fn add_appinit(items: &mut Vec<StartupItem>, view: RegView, view_label: &str) {
    let sub_key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows";
    let key = match open_subkey(RegRoot::LocalMachine, sub_key, view) {
        Some(key) => key,
        None => return,
    };
    if !key.value_names().iter().any(|name| eq_ci(name, "AppInit_DLLs")) {
        return;
    }
    let Some((dll_kind, dll_value)) = key.value("AppInit_DLLs") else { return };
    let dlls = dll_value.display().trim().to_string();
    if dlls.is_empty() {
        return;
    }
    let mut load = 0i64;
    let mut load_exists = false;
    let mut load_kind = RegKind::Unknown;
    if key.value_names().iter().any(|name| eq_ci(name, "LoadAppInit_DLLs")) {
        if let Some((kind, value)) = key.value("LoadAppInit_DLLs") {
            load_kind = kind;
            load = match value {
                RegValue::DWord(v) => v as i64,
                RegValue::QWord(v) => v as i64,
                RegValue::Str(s) => s.trim().parse().unwrap_or(0),
                _ => 0,
            };
            load_exists = true;
        }
    }
    let (mut writable, mut reason_out) = can_open_registry_value_for_write(RegRoot::LocalMachine, sub_key, "AppInit_DLLs", view);
    if writable {
        let (w2, r2) = can_open_registry_value_for_write(RegRoot::LocalMachine, sub_key, "LoadAppInit_DLLs", view);
        writable = w2;
        if !w2 { reason_out = r2; }
    }
    let capability = decide_registry_mutation_capability("Machine", RegRoot::LocalMachine.name(), writable, is_elevated(), "", Some(sub_key));
    let exact_shape = matches!(dll_kind, RegKind::String | RegKind::ExpandString) && load_exists && load_kind == RegKind::DWord;
    let can_disable = load != 0 && exact_shape && capability.can_mutate;
    let reason = if !exact_shape {
        "AppInit registry kinds are ambiguous; refusing to rewrite or invent per-DLL state".to_string()
    } else if load == 0 {
        "LoadAppInit_DLLs is already disabled; only a manager-owned exact snapshot may restore it".to_string()
    } else if can_disable {
        "Disable the complete AppInit surface by toggling LoadAppInit_DLLs only; AppInit_DLLs bytes remain unchanged".to_string()
    } else {
        capability.reason.clone()
    };
    items.push(StartupItem {
        id: ids::appinit_id(sub_key, view_label),
        name: "AppInit DLLs".into(),
        source: "AppInit DLLs".into(),
        scope: "Machine".into(),
        command: dll_value.display(),
        location: format!("HKEY_LOCAL_MACHINE\\{} [{}]", sub_key, view_label),
        enabled: load != 0,
        can_disable,
        status: format!("LoadAppInit_DLLs={}; exact DLL list retained as one surface; per-DLL rewrite is intentionally unavailable; {}", load, view_label),
        registry_view: view_label.into(),
        mutation_capability: if can_disable { "AppInitLoadToggle".into() } else { "ReadOnly".into() },
        mutation_reason: reason,
        requires_expert_confirmation: true,
        requires_elevation: capability.requires_elevation,
        requires_reboot: true,
        ..StartupItem::new()
    });
    let _ = reason_out;
}

/// C# `AddActiveSetup`.
pub fn add_active_setup(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str, sub_key: Option<&str>, view_label: &str) {
    let base = sub_key.unwrap_or(r"SOFTWARE\Microsoft\Active Setup\Installed Components");
    let parent = match open_subkey(root, base, view) {
        Some(key) => key,
        None => return,
    };
    for component in parent.subkey_names() {
        let Some(key) = open_subkey(root, &format!("{}\\{}", base, component), view) else { continue };
        let Some((_, stub)) = key.value("StubPath") else { continue };
        let stub_text = stub.display();
        if stub_text.trim().is_empty() {
            continue;
        }
        let default = key.value("").map(|(_, v)| v.display()).unwrap_or_default();
        let name = if default.trim().is_empty() { component.clone() } else { default };
        items.push(StartupItem {
            id: ids::active_id(scope, root_display(root), &format!("{}\\{}", base, component), view_label),
            name,
            source: "Active Setup".into(),
            scope: scope.into(),
            command: stub_text,
            location: format!("{}\\{}\\{}{}", root_display(root), base, component, if view_label.is_empty() { String::new() } else { format!(" [{}]", view_label) }),
            enabled: true,
            can_disable: scope.starts_with("User") || is_elevated(),
            status: if view_label.is_empty() { "Runs once per user profile through Active Setup StubPath".into() } else { format!("Runs once per user profile through Active Setup StubPath; {}", view_label) },
            registry_view: view_label.into(),
            ..StartupItem::new()
        });
    }
}

/// C# `AddReadOnlyMultiStringEntries` — BootExecute / LSA indexed components.
pub fn add_read_only_multi_string_entries(items: &mut Vec<StartupItem>, root: RegRoot, sub_key: &str, value_names: &[&str], scope: &str, source: &str, status: &str) {
    for value_name in value_names {
        let view = native_registry_view();
        let key = match open_subkey(root, sub_key, view) {
            Some(key) => key,
            None => continue,
        };
        if !key.value_names().iter().any(|name| eq_ci(name, value_name)) {
            continue;
        }
        let Some((kind, value)) = key.value(value_name) else { continue };
        let values: Vec<String> = match value {
            RegValue::MultiString(parts) => parts,
            other => vec![other.display()],
        };
        let (writable, access_reason) = can_open_registry_value_for_write(root, sub_key, value_name, view);
        let capability = decide_registry_mutation_capability(scope, root_display(root), writable, is_elevated(), "", Some(sub_key));
        let exact_shape = kind == RegKind::MultiString;
        let view_label = native_registry_view().label();
        for (index, entry) in values.iter().enumerate() {
            if entry.trim().is_empty() {
                continue;
            }
            let can_disable = exact_shape && capability.can_mutate;
            let reason = if !exact_shape {
                format!("Expected REG_MULTI_SZ, found {}; refusing an ambiguous rewrite", kind.as_str())
            } else if can_disable {
                "Exact indexed component can be removed transactionally; a reboot is required".to_string()
            } else {
                format!("{}{}", capability.reason, if access_reason.is_empty() { String::new() } else { format!(": {}", access_reason) })
            };
            items.push(StartupItem {
                id: ids::advanced_id(root_display(root), sub_key, value_name, index),
                name: format!("{}{}", value_name, if values.len() > 1 { format!(" #{}", index + 1) } else { String::new() }),
                source: source.into(),
                scope: scope.into(),
                command: entry.clone(),
                location: format!("{}\\{}\\{}", root_display(root), sub_key, value_name),
                enabled: true,
                can_disable,
                status: format!("{}; exact index={}; kind={}; {}", status, index, kind.as_str(), reason),
                registry_view: view_label.into(),
                mutation_capability: if can_disable { "TransactionalRegistryValue".into() } else { "ReadOnly".into() },
                mutation_reason: reason,
                requires_expert_confirmation: true,
                requires_elevation: capability.requires_elevation,
                requires_reboot: true,
                ..StartupItem::new()
            });
        }
    }
}

/// C# `AddCommonRegistryStartup` — machine + user primary walk.
pub fn add_common_registry_startup(items: &mut Vec<StartupItem>) {
    for view in startup_registry_views() {
        let label = view.label();
        for sub_key in RUN_KEYS {
            add_registry_values(items, RegRoot::LocalMachine, view, sub_key, "Machine", source_for_registry_path(sub_key), true,
                &format!("Runs from {}", sub_key), None, label);
        }
        add_registry_values(items, RegRoot::LocalMachine, view, r"SYSTEM\CurrentControlSet\Control\Terminal Server\Wds\rdpwd\StartupPrograms",
            "Machine", "Terminal Server Startup", true, "Programs started when a Terminal Services session begins", None, label);
        add_winlogon_values(items, RegRoot::LocalMachine, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon", "Machine",
            &["Shell", "Userinit", "Taskman", "AppSetup", "System", "VmApplet"], label);
        add_registry_values(items, RegRoot::LocalMachine, view, r"SYSTEM\CurrentControlSet\Control\SafeBoot\AlternateShell",
            "Machine", "SafeBoot AlternateShell", true, "Command shell used in Safe Boot", Some(&["AlternateShell"]), label);
        add_registry_values(items, RegRoot::LocalMachine, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows\IconServiceLib",
            "Machine", "Icon Service Library", true, "Icon codec service DLL", None, label);
        add_registry_values(items, RegRoot::LocalMachine, view, r"SYSTEM\CurrentControlSet\Control\Session Manager",
            "Machine", "Boot Execute", true, "Boot-time execute commands",
            Some(&["BootExecute", "SetupExecute", "Execute", "S0InitialCommand"]), label);
        add_registry_values(items, RegRoot::LocalMachine, view, r"SYSTEM\CurrentControlSet\Control\Lsa",
            "Machine", "LSA Startup Package", true, "LSA authentication/notification/security packages",
            Some(&["Authentication Packages", "Notification Packages", "Security Packages"]), label);
        add_winlogon_values(items, RegRoot::LocalMachine, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon", "Machine",
            &["Shell", "Userinit", "Taskman", "AppSetup", "System", "VmApplet"], label);
        add_appinit(items, view, label);
        add_active_setup(items, RegRoot::LocalMachine, view, "Machine", None, label);
        add_active_setup(items, RegRoot::LocalMachine, view, "Machine", Some(r"SOFTWARE\Wow6432Node\Microsoft\Active Setup\Installed Components"), label);
    }
    for view in user_startup_registry_views() {
        let label = view.label();
        for sub_key in RUN_KEYS {
            add_registry_values(items, RegRoot::CurrentUser, view, sub_key, "User", source_for_registry_path(sub_key), true,
                &format!("Runs from {}", sub_key), None, label);
        }
        add_winlogon_values(items, RegRoot::CurrentUser, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon", "User",
            &["Shell", "Userinit", "Taskman"], label);
        add_registry_values(items, RegRoot::CurrentUser, view, "Environment", "User", "User Logon Script", true,
            "UserInitMprLogonScript runs during interactive logon", Some(&["UserInitMprLogonScript"]), label);
        add_active_setup(items, RegRoot::CurrentUser, view, "User", None, label);
    }
}

/// C# `AddSystemBootEntries` — read-only BootExecute/LSA (capability-decorated).
pub fn add_system_boot_entries(items: &mut Vec<StartupItem>) {
    let session_manager = r"SYSTEM\CurrentControlSet\Control\Session Manager";
    for value_name in ["BootExecute", "SetupExecute", "Execute", "S0InitialCommand"] {
        add_read_only_multi_string_entries(items, RegRoot::LocalMachine, session_manager, &[value_name], "Machine", "Boot Execute", "System-critical Session Manager boot command");
    }
    let lsa = r"SYSTEM\CurrentControlSet\Control\Lsa";
    for value_name in ["Authentication Packages", "Notification Packages", "Security Packages"] {
        add_read_only_multi_string_entries(items, RegRoot::LocalMachine, lsa, &[value_name], "Machine", "LSA Startup Package", "System-critical Local Security Authority package");
    }
    add_read_only_multi_string_entries(items, RegRoot::LocalMachine, &format!("{}\\OSConfig", lsa), &["Security Packages"], "Machine", "LSA Startup Package", "System-critical Local Security Authority package");
}

/// C# `AddLoadedUserRegistryStartup` — every loaded profile's surfaces.
pub fn add_loaded_user_registry_startup(items: &mut Vec<StartupItem>) {
    for view in user_startup_registry_views() {
        let label = view.label();
        for (sid, scope) in loaded_user_hives() {
            let prefix = format!("{}\\", sid);
            let run_status = if is_service_account_sid(&sid) {
                format!("Registered in the {} service-account profile; Windows runs it whenever that account starts an interactive session",
                    scope.strip_prefix("User:").unwrap_or(&scope))
            } else {
                format!("Runs for loaded profile {}", scope)
            };
            for path in RUN_KEYS {
                add_registry_values(items, RegRoot::Users, view, &format!("{}{}", prefix, path), &scope, source_for_registry_path(path), true, &run_status, None, label);
            }
            add_registry_values(items, RegRoot::Users, view, &format!("{}SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Windows", prefix), &scope,
                "Legacy Windows Run", true, &format!("Legacy Windows Load/Run value for {}", scope), Some(&["Load", "Run"]), label);
            add_winlogon_values(items, RegRoot::Users, view, &format!("{}SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon", prefix), &scope, &["Shell", "Userinit", "Taskman"], label);
            add_registry_values(items, RegRoot::Users, view, &format!("{}Environment", prefix), &scope, "User Logon Script", true,
                &format!("UserInitMprLogonScript for {}", scope), Some(&["UserInitMprLogonScript"]), label);
            add_run_once_ex(items, RegRoot::Users, view, &scope, Some(&format!("{}SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\RunOnceEx", prefix)), label);
            for path in [r"SOFTWARE\Microsoft\Windows\CurrentVersion\ShellServiceObjectDelayLoad",
                         r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SharedTaskScheduler",
                         r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellServiceObjects"] {
                add_registry_values(items, RegRoot::Users, view, &format!("{}{}", prefix, path), &scope, "Explorer Startup Extension", true,
                    &format!("Explorer startup extension for {}", scope), None, label);
            }
            crate::scan_extensions::add_subkey_default_values(items, RegRoot::Users, view, &format!("{}SOFTWARE\\Classes\\Protocols\\Filter", prefix), &scope, "Protocol Filter", false, "Protocol filter handler", label);
            crate::scan_extensions::add_subkey_default_values(items, RegRoot::Users, view, &format!("{}SOFTWARE\\Classes\\Protocols\\Handler", prefix), &scope, "Protocol Handler", false, "Protocol handler handler", label);
            crate::scan_extensions::add_subkey_default_values(items, RegRoot::Users, view, &format!("{}SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\ShellServiceObjects", prefix), &scope, "Explorer Startup Extension", false, "Explorer loads this COM extension during desktop startup", label);
            crate::scan_extensions::add_subkey_default_values(items, RegRoot::Users, view, &format!(r"{}Software\Classes\*\ShellEx\ContextMenuHandlers", prefix), &scope, "Context Menu Handler", false, "File context menu handler", label);
            crate::scan_extensions::add_subkey_default_values(items, RegRoot::Users, view, &format!(r"{}Software\Classes\Drive\ShellEx\ContextMenuHandlers", prefix), &scope, "Context Menu Handler", false, "Drive context menu handler", label);
            crate::scan_extensions::add_explorer_shell_extensions(items, RegRoot::Users, view, &scope, Some(&prefix), label);
            crate::scan_extensions::add_internet_explorer_addons(items, RegRoot::Users, view, &scope, Some(&prefix), label);
            add_active_setup(items, RegRoot::Users, view, &scope, Some(&format!("{}SOFTWARE\\Microsoft\\Active Setup\\Installed Components", prefix)), label);
        }
    }
    for (sid, name) in SERVICE_ACCOUNT_PROFILES {
        if service_profile_state(sid) != "unreadable" {
            continue;
        }
        items.push(error_item("Registry Run", &format!("User:{}", name),
            &format!("HKEY_USERS\\{}\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", sid),
            &format!("The {} profile can only be read by an administrator; run Startup Master elevated to list and control its startup values", name)));
    }
}

/// C# `AddGroupPolicyScriptTree` — recursive Script/Parameters walk.
pub fn add_group_policy_scripts(items: &mut Vec<StartupItem>) {
    let specs: [(RegRoot, &str, &str); 4] = [
        (RegRoot::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\Scripts\Startup", "Machine"),
        (RegRoot::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\State\Machine\Scripts\Startup", "Machine"),
        (RegRoot::CurrentUser, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\Scripts\Logon", "User"),
        (RegRoot::CurrentUser, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\State\User\Scripts\Logon", "User"),
    ];
    for (root, path, scope) in specs {
        add_group_policy_tree(items, root, RegView::Default, path, scope, 0);
    }
}

fn add_group_policy_tree(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str, depth: usize) {
    if depth > 8 {
        return;
    }
    let Some(key) = open_subkey(root, sub_key, view) else { return };
    let script = key.value("Script").map(|(_, v)| v.display()).unwrap_or_default();
    if !script.trim().is_empty() {
        let args = key.value("Parameters").map(|(_, v)| v.display()).unwrap_or_default();
        let (fs_path, ds_path, gpo_id) = read_group_policy_authority_evidence(root, view, sub_key);
        let authority = classify_group_policy_authority(sub_key, &fs_path, &ds_path, &gpo_id);
        let reason = if authority == "Domain Group Policy" {
            "Domain policy is authoritative; this machine cannot promise a persistent local disable"
        } else if authority == "Local Group Policy" {
            "Local policy is authoritative; disable this script through Local Group Policy"
        } else {
            "The owning Group Policy authority must change this script"
        };
        let leaf = script.rsplit('\\').next().unwrap_or(&script).to_string();
        items.push(StartupItem {
            id: ids::gpscript_id(root_display(root), sub_key),
            name: leaf,
            source: "Group Policy Script".into(),
            scope: scope.into(),
            command: if args.trim().is_empty() { script.clone() } else { format!("{} {}", script, args) },
            location: format!("{}\\{}", root_display(root), sub_key),
            enabled: true,
            can_disable: false,
            status: format!("{} startup/logon script; {}", authority, reason),
            mutation_capability: "ExternalAuthority".into(),
            mutation_reason: reason.into(),
            external_authority: authority.into(),
            requires_elevation: eq_ci(scope, "Machine"),
            ..StartupItem::new()
        });
    }
    for child in key.subkey_names() {
        add_group_policy_tree(items, root, view, &format!("{}\\{}", sub_key, child), scope, depth + 1);
    }
}

fn read_group_policy_authority_evidence(root: RegRoot, view: RegView, leaf: &str) -> (String, String, String) {
    let mut fs = String::new();
    let mut ds = String::new();
    let mut gpo = String::new();
    let mut current = leaf.to_string();
    for _ in 0..=8 {
        if current.is_empty() {
            break;
        }
        if let Some(key) = open_subkey(root, &current, view) {
            if fs.is_empty() { fs = key.value("FileSysPath").map(|(_, v)| v.display()).unwrap_or_default(); }
            if ds.is_empty() { ds = key.value("DSPath").map(|(_, v)| v.display()).unwrap_or_default(); }
            if ds.is_empty() { ds = key.value("SOM-ID").map(|(_, v)| v.display()).unwrap_or_default(); }
            if gpo.is_empty() { gpo = key.value("GPO-ID").map(|(_, v)| v.display()).unwrap_or_default(); }
        }
        match current.rfind('\\') {
            Some(slash) if slash > 0 => current = current[..slash].to_string(),
            _ => break,
        }
        if !current.to_lowercase().contains(r"\group policy\") {
            break;
        }
    }
    (fs, ds, gpo)
}

pub fn classify_group_policy_authority(sub_key: &str, file_system_path: &str, directory_service_path: &str, gpo_id: &str) -> String {
    let evidence = format!("{}\n{}\n{}", file_system_path, directory_service_path, gpo_id).trim().to_string();
    if evidence.to_uppercase().contains("SYSVOL") || evidence.to_uppercase().contains("LDAP://") || evidence.to_uppercase().contains("CN=POLICIES,CN=SYSTEM") {
        return "Domain Group Policy".into();
    }
    let local_policy = format!("{}\\System32\\GroupPolicy", crate::paths::windows_dir().trim_end_matches('\\'));
    if file_system_path.to_lowercase().starts_with(&local_policy.to_lowercase()) || sub_key.to_lowercase().contains(r"\group policy\scripts\") {
        return "Local Group Policy".into();
    }
    "Group Policy authority".into()
}

/// Legacy `Load`/`Run` values, Explorer extensions, RunOnceEx for the primary machine/user walk.
pub fn add_registry_startup_extensions(items: &mut Vec<StartupItem>) {
    for view in startup_registry_views() {
        add_registry_startup_extensions_for_root(items, RegRoot::LocalMachine, view, "Machine");
    }
    for view in user_startup_registry_views() {
        add_registry_startup_extensions_for_root(items, RegRoot::CurrentUser, view, "User");
    }
    add_registry_values(items, RegRoot::LocalMachine, RegView::Default, r"SYSTEM\CurrentControlSet\Control\Session Manager\AppCertDlls",
        "Machine", "AppCert DLLs", true, "DLL injected when applications create a process", None, "");
}

fn add_registry_startup_extensions_for_root(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str) {
    let label = view.label();
    add_registry_values(items, root, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows", scope, "Legacy Windows Run",
        true, "Legacy Windows Load/Run logon value", Some(&["Load", "Run"]), label);
    add_run_once_ex(items, root, view, scope, None, label);
    for path in [r"SOFTWARE\Microsoft\Windows\CurrentVersion\ShellServiceObjectDelayLoad",
                 r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SharedTaskScheduler",
                 r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellServiceObjects"] {
        add_registry_values(items, root, view, path, scope, "Explorer Startup Extension", true,
            "Explorer loads this COM extension during desktop startup", None, label);
    }
    crate::scan_extensions::add_subkey_default_values(items, root, view, r"SOFTWARE\Classes\Protocols\Filter", scope, "Protocol Filter", false, "Protocol filter handler", label);
    crate::scan_extensions::add_subkey_default_values(items, root, view, r"SOFTWARE\Classes\Protocols\Handler", scope, "Protocol Handler", false, "Protocol handler handler", label);
    crate::scan_extensions::add_subkey_default_values(items, root, view, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellServiceObjects", scope, "Explorer Startup Extension", false, "Explorer loads this COM extension during desktop startup", label);
    crate::scan_extensions::add_subkey_default_values(items, root, view, r"Software\Classes\*\ShellEx\ContextMenuHandlers", scope, "Context Menu Handler", false, "File context menu handler", label);
    crate::scan_extensions::add_subkey_default_values(items, root, view, r"Software\Classes\Drive\ShellEx\ContextMenuHandlers", scope, "Context Menu Handler", false, "Drive context menu handler", label);
    crate::scan_extensions::add_explorer_shell_extensions(items, root, view, scope, None, label);
    crate::scan_extensions::add_internet_explorer_addons(items, root, view, scope, None, label);
}

/// Independent audit variants (plain rows, no capability preflight).
pub fn audit_registry_sources(raw: &mut Vec<StartupItem>) {
    for view in startup_registry_views() {
        let label = view.label();
        for path in RUN_KEYS {
            audit_registry_values(raw, RegRoot::LocalMachine, view, path, "Machine", source_for_registry_path(path), None, label);
        }
        audit_registry_values(raw, RegRoot::LocalMachine, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows", "Machine", "Legacy Windows Run", Some(&["Load", "Run"]), label);
        audit_run_once_ex(raw, RegRoot::LocalMachine, view, "Machine", None, label);
        for path in [r"SOFTWARE\Microsoft\Windows\CurrentVersion\ShellServiceObjectDelayLoad", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SharedTaskScheduler"] {
            audit_registry_values(raw, RegRoot::LocalMachine, view, path, "Machine", "Explorer Startup Extension", None, label);
        }
        crate::scan_extensions::audit_explorer_shell_extensions(raw, RegRoot::LocalMachine, view, "Machine", None, label);
        crate::scan_extensions::audit_internet_explorer_addons(raw, RegRoot::LocalMachine, view, "Machine", None, label);
        audit_winlogon_values(raw, RegRoot::LocalMachine, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon", "Machine", &["Shell", "Userinit", "Taskman", "AppSetup", "System"], label);
        audit_appinit(raw, view, label);
        audit_active_setup(raw, RegRoot::LocalMachine, view, "Machine", None, label);
    }
    for view in user_startup_registry_views() {
        let label = view.label();
        for path in RUN_KEYS {
            audit_registry_values(raw, RegRoot::CurrentUser, view, path, "User", source_for_registry_path(path), None, label);
        }
        audit_registry_values(raw, RegRoot::CurrentUser, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows", "User", "Legacy Windows Run", Some(&["Load", "Run"]), label);
        audit_run_once_ex(raw, RegRoot::CurrentUser, view, "User", None, label);
        for path in [r"SOFTWARE\Microsoft\Windows\CurrentVersion\ShellServiceObjectDelayLoad", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SharedTaskScheduler"] {
            audit_registry_values(raw, RegRoot::CurrentUser, view, path, "User", "Explorer Startup Extension", None, label);
        }
        crate::scan_extensions::audit_explorer_shell_extensions(raw, RegRoot::CurrentUser, view, "User", None, label);
        crate::scan_extensions::audit_internet_explorer_addons(raw, RegRoot::CurrentUser, view, "User", None, label);
        audit_winlogon_values(raw, RegRoot::CurrentUser, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon", "User", &["Shell", "Userinit", "Taskman"], label);
        audit_registry_values(raw, RegRoot::CurrentUser, view, "Environment", "User", "User Logon Script", Some(&["UserInitMprLogonScript"]), label);
        audit_active_setup(raw, RegRoot::CurrentUser, view, "User", None, label);
    }
    audit_registry_values(raw, RegRoot::LocalMachine, RegView::Default, r"SYSTEM\CurrentControlSet\Control\Session Manager\AppCertDlls", "Machine", "AppCert DLLs", None, "");
    audit_loaded_user_registry_sources(raw);
}

fn audit_registry_values(raw: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str,
                           source: &str, filter: Option<&[&str]>, view_label: &str) {
    let Some(key) = open_subkey(root, sub_key, view) else { return };
    for name in key.value_names() {
        if let Some(filter) = filter {
            if !filter.iter().any(|f| eq_ci(f, &name)) {
                continue;
            }
        }
        let Some((_, value)) = key.value(&name) else { continue };
        let command = value.display();
        if command.trim().is_empty() {
            continue;
        }
        raw.push(StartupItem {
            id: ids::reg_id(scope, root_display(root), sub_key, &name, view_label),
            name: if name.is_empty() { "(Default)".into() } else { name.clone() },
            source: source.into(),
            scope: scope.into(),
            command,
            location: format!("{}\\{}{}", root_display(root), sub_key, if view_label.is_empty() { String::new() } else { format!(" [{}]", view_label) }),
            enabled: true,
            registry_view: view_label.into(),
            ..StartupItem::new()
        });
    }
}

fn audit_winlogon_values(raw: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str, filter: &[&str], view_label: &str) {
    let Some(key) = open_subkey(root, sub_key, view) else { return };
    for value_name in key.value_names() {
        if !filter.iter().any(|f| eq_ci(f, &value_name)) {
            continue;
        }
        let Some((kind, value)) = key.value(&value_name) else { continue };
        if !matches!(kind, RegKind::String | RegKind::ExpandString) {
            continue;
        }
        let Some(components) = try_split_winlogon_components(&value.display()) else {
            raw.push(error_item("Independent Winlogon Autostart", scope, &format!("{}\\{}\\{}", root_display(root), sub_key, value_name),
                "Winlogon value has an ambiguous quoted component list"));
            continue;
        };
        for (index, component) in components.iter().enumerate() {
            let component = component.trim();
            if component.is_empty() {
                continue;
            }
            raw.push(StartupItem {
                id: ids::winlogon_id(scope, root_display(root), sub_key, &value_name, index, view_label),
                name: format!("{}{}", value_name, if components.iter().filter(|c| !c.trim().is_empty()).count() > 1 { format!(" #{}", index + 1) } else { String::new() }),
                source: "Winlogon Autostart".into(),
                scope: scope.into(),
                command: component.to_string(),
                location: format!("{}\\{}\\{} [{}]", root_display(root), sub_key, value_name, view_label),
                enabled: true,
                registry_view: view_label.into(),
                ..StartupItem::new()
            });
        }
    }
}

fn audit_run_once_ex(raw: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str, base_path: Option<&str>, view_label: &str) {
    let base = base_path.unwrap_or(r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnceEx");
    let Some(parent) = open_subkey(root, base, view) else { return };
    for child_name in parent.subkey_names() {
        let child_path = format!("{}\\{}", base, child_name);
        let Some(child) = open_subkey(root, &child_path, view) else { continue };
        for value_name in child.value_names() {
            if ["Title", "Flags", "Depend"].iter().any(|f| eq_ci(f, &value_name)) {
                continue;
            }
            let Some((_, value)) = child.value(&value_name) else { continue };
            let command = value.display();
            if command.trim().is_empty() {
                continue;
            }
            raw.push(StartupItem {
                id: ids::reg_id(scope, root_display(root), &child_path, &value_name, view_label),
                name: if value_name.is_empty() { child_name.clone() } else { value_name.clone() },
                source: "Registry RunOnceEx".into(),
                scope: scope.into(),
                command,
                location: format!("{}\\{}{}", root_display(root), child_path, if view_label.is_empty() { String::new() } else { format!(" [{}]", view_label) }),
                enabled: true,
                registry_view: view_label.into(),
                ..StartupItem::new()
            });
        }
    }
}

fn audit_appinit(raw: &mut Vec<StartupItem>, view: RegView, view_label: &str) {
    let path = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows";
    let Some(key) = open_subkey(RegRoot::LocalMachine, path, view) else { return };
    let dlls = key.value("AppInit_DLLs").map(|(_, v)| v.display()).unwrap_or_default();
    if dlls.trim().is_empty() {
        return;
    }
    let enabled = key.value("LoadAppInit_DLLs").map(|(_, v)| match v {
        RegValue::DWord(d) => d != 0,
        RegValue::QWord(q) => q != 0,
        RegValue::Str(s) => s.trim().parse::<i64>().map(|n| n != 0).unwrap_or(false),
        _ => false,
    }).unwrap_or(false);
    raw.push(StartupItem {
        id: ids::appinit_id(path, view_label),
        name: "AppInit DLLs".into(),
        source: "AppInit DLLs".into(),
        scope: "Machine".into(),
        command: dlls,
        location: format!("HKEY_LOCAL_MACHINE\\{} [{}]", path, view_label),
        enabled,
        registry_view: view_label.into(),
        ..StartupItem::new()
    });
}

fn audit_active_setup(raw: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str, base_path: Option<&str>, view_label: &str) {
    let base = base_path.unwrap_or(r"SOFTWARE\Microsoft\Active Setup\Installed Components");
    let Some(parent) = open_subkey(root, base, view) else { return };
    for component in parent.subkey_names() {
        let Some(key) = open_subkey(root, &format!("{}\\{}", base, component), view) else { continue };
        let Some((_, stub)) = key.value("StubPath") else { continue };
        let command = stub.display();
        if command.trim().is_empty() {
            continue;
        }
        let sub = format!("{}\\{}", base, component);
        raw.push(StartupItem {
            id: ids::active_id(scope, root_display(root), &sub, view_label),
            name: component.clone(),
            source: "Active Setup".into(),
            scope: scope.into(),
            command,
            location: format!("{}\\{}{}", root_display(root), sub, if view_label.is_empty() { String::new() } else { format!(" [{}]", view_label) }),
            enabled: true,
            registry_view: view_label.into(),
            ..StartupItem::new()
        });
    }
}

fn audit_loaded_user_registry_sources(raw: &mut Vec<StartupItem>) {
    for view in user_startup_registry_views() {
        let label = view.label();
        for (sid, scope) in loaded_user_hives() {
            let prefix = format!("{}\\", sid);
            for path in RUN_KEYS {
                audit_registry_values(raw, RegRoot::Users, view, &format!("{}{}", prefix, path), &scope, source_for_registry_path(path), None, label);
            }
            audit_registry_values(raw, RegRoot::Users, view, &format!("{}SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Windows", prefix), &scope, "Legacy Windows Run", Some(&["Load", "Run"]), label);
            audit_winlogon_values(raw, RegRoot::Users, view, &format!("{}SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon", prefix), &scope, &["Shell", "Userinit", "Taskman"], label);
            audit_registry_values(raw, RegRoot::Users, view, &format!("{}Environment", prefix), &scope, "User Logon Script", Some(&["UserInitMprLogonScript"]), label);
            audit_run_once_ex(raw, RegRoot::Users, view, &scope, Some(&format!("{}SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\RunOnceEx", prefix)), label);
            for path in [r"SOFTWARE\Microsoft\Windows\CurrentVersion\ShellServiceObjectDelayLoad", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SharedTaskScheduler"] {
                audit_registry_values(raw, RegRoot::Users, view, &format!("{}{}", prefix, path), &scope, "Explorer Startup Extension", None, label);
            }
            crate::scan_extensions::audit_explorer_shell_extensions(raw, RegRoot::Users, view, &scope, Some(&prefix), label);
            crate::scan_extensions::audit_internet_explorer_addons(raw, RegRoot::Users, view, &scope, Some(&prefix), label);
            audit_active_setup(raw, RegRoot::Users, view, &scope, Some(&format!("{}SOFTWARE\\Microsoft\\Active Setup\\Installed Components", prefix)), label);
        }
    }
}

/// Advanced registry audit (BootExecute/LSA indexed).
pub fn audit_advanced_registry_sources(raw: &mut Vec<StartupItem>) {
    let mut add = |path: &str, value_name: &str, source: &str| {
        let view = native_registry_view();
        let Some(key) = open_subkey(RegRoot::LocalMachine, path, view) else { return };
        let Some((_, value)) = key.value(value_name) else { return };
        let values: Vec<String> = match value {
            RegValue::MultiString(parts) => parts,
            other => vec![other.display()],
        };
        for (i, entry) in values.iter().enumerate() {
            if entry.trim().is_empty() {
                continue;
            }
            raw.push(StartupItem {
                id: ids::advanced_id(RegRoot::LocalMachine.name(), path, value_name, i),
                name: format!("{}{}", value_name, if values.len() > 1 { format!(" #{}", i + 1) } else { String::new() }),
                source: source.into(),
                scope: "Machine".into(),
                command: entry.clone(),
                location: format!("HKEY_LOCAL_MACHINE\\{}\\{}", path, value_name),
                enabled: true,
                ..StartupItem::new()
            });
        }
    };
    for value in ["BootExecute", "SetupExecute", "Execute", "S0InitialCommand"] {
        add(r"SYSTEM\CurrentControlSet\Control\Session Manager", value, "Boot Execute");
    }
    for value in ["Authentication Packages", "Notification Packages", "Security Packages"] {
        add(r"SYSTEM\CurrentControlSet\Control\Lsa", value, "LSA Startup Package");
    }
    add(r"SYSTEM\CurrentControlSet\Control\Lsa\OSConfig", "Security Packages", "LSA Startup Package");
}

/// Group-policy script audit (plain rows).
pub fn audit_group_policy_scripts(raw: &mut Vec<StartupItem>) {
    let specs: [(RegRoot, &str, &str); 4] = [
        (RegRoot::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\Scripts\Startup", "Machine"),
        (RegRoot::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\State\Machine\Scripts\Startup", "Machine"),
        (RegRoot::CurrentUser, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\Scripts\Logon", "User"),
        (RegRoot::CurrentUser, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\State\User\Scripts\Logon", "User"),
    ];
    for (root, path, scope) in specs {
        audit_group_policy_tree(raw, root, RegView::Default, path, scope, 0);
    }
}

fn audit_group_policy_tree(raw: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str, depth: usize) {
    if depth > 8 {
        return;
    }
    let Some(key) = open_subkey(root, sub_key, view) else { return };
    let script = key.value("Script").map(|(_, v)| v.display()).unwrap_or_default();
    if !script.trim().is_empty() {
        let args = key.value("Parameters").map(|(_, v)| v.display()).unwrap_or_default();
        let leaf = script.rsplit('\\').next().unwrap_or(&script).to_string();
        raw.push(StartupItem {
            id: ids::gpscript_id(root_display(root), sub_key),
            name: leaf,
            source: "Group Policy Script".into(),
            scope: scope.into(),
            command: if args.trim().is_empty() { script.clone() } else { format!("{} {}", script, args) },
            location: format!("{}\\{}", root_display(root), sub_key),
            enabled: true,
            ..StartupItem::new()
        });
    }
    for child in key.subkey_names() {
        audit_group_policy_tree(raw, root, view, &format!("{}\\{}", sub_key, child), scope, depth + 1);
    }
}

#[allow(unused)]
fn _b64_hint(s: &str) -> String {
    b64(s)
}
