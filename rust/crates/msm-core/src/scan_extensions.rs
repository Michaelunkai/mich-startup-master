//! Extended autorun surfaces (Autoruns parity): read-only extension points that
//! execute code at boot/logon, with single-value reversible disable where exact.

use crate::ids;
use crate::model::{error_item, StartupItem};
use crate::reg::*;
use crate::scan_registry::decide_registry_mutation_capability;
use crate::util::{eq_ci, expand_env, is_elevated};
use crate::views::root_display;

/// C# `ReadOnlyExtensionItem` + `ApplyExtensionDisableCapability` + `ProtectedExtensionReason`.
pub fn read_only_extension_item(kind: &str, root: RegRoot, sub_key: &str, value_name: &str, scope: &str, source: &str,
                                 command: &str, status: &str, view_label: &str, requires_reboot: bool) -> StartupItem {
    let key = sub_key.trim_matches('\\');
    let value = value_name;
    let display_value = if value.is_empty() { "(Default)".to_string() } else { value.to_string() };
    let mut item = StartupItem {
        id: ids::extension_id(kind, root_display(root), key, value, view_label),
        name: format!("{} — {}", key.rsplit('\\').next().unwrap_or(key), display_value),
        source: source.into(),
        scope: scope.into(),
        command: command.into(),
        location: format!("{}\\{}\\{}{}", root_display(root), key, display_value, if view_label.is_empty() { String::new() } else { format!(" [{}]", view_label) }),
        enabled: true,
        can_disable: false,
        status: format!("{}; read-only route", status),
        registry_view: view_label.into(),
        mutation_capability: "ReadOnly".into(),
        mutation_reason: "This Windows extension point is shown for complete inventory coverage; Startup Master will not guess at a dangerous registry rewrite.".into(),
        requires_expert_confirmation: true,
        requires_reboot,
        ..StartupItem::new()
    };
    apply_extension_disable_capability(&mut item, kind, root, key, value, scope, source, view_label);
    if !item.can_disable && item.mutation_capability == "ReadOnly"
        && item.mutation_reason.starts_with("This Windows extension point is shown") {
        item.mutation_reason = protected_extension_reason(kind).to_string();
    }
    item
}

fn protected_extension_reason(kind: &str) -> &'static str {
    match kind {
        "known-dll" => "Protected: this system library list is read before any program starts. Removing an entry can stop Windows from booting, so it is never changed from here.",
        "service-dependency" => "Not a separate startup item: it is the start type of a Windows service. Disable the service itself from its own row.",
        "network-provider" => "Protected: removing a network provider can break sign-in or file sharing. Remove it through its own installer.",
        "print-monitor" | "print-provider" => "Protected: the print spooler loads this component. Remove it through the printer or driver installer.",
        "security-provider" => "Protected: removing a security provider can lock the machine out of sign-in.",
        "icon-service-library" => "Protected: Windows icon rendering depends on this library.",
        "terminal-server-startup" => "Protected: part of the remote desktop session host; change it through Remote Desktop configuration.",
        "image-hijack" => "Read-only: this launch redirection is owned by the program that registered it; remove it with that program.",
        _ => "This Windows registration is a key, not a single value; removing it cannot be restored exactly from here, so it is shown for coverage only.",
    }
}

/// C# `ExtensionDisableSurface` — the exact reversible single-value extension set.
pub fn extension_disable_surface(kind: Option<&str>, root_name: &str, sub_key: &str, value_name: &str, source: &str) -> bool {
    let key = sub_key.trim_matches('\\').to_lowercase();
    let value = value_name;
    if value.is_empty() || value.contains('#') || key.is_empty() {
        return false;
    }
    let machine = root_name.eq_ignore_ascii_case("HKEY_LOCAL_MACHINE") || root_name.eq_ignore_ascii_case("HKLM");
    let user = root_name.eq_ignore_ascii_case("HKEY_CURRENT_USER") || root_name.eq_ignore_ascii_case("HKCU");
    let users = root_name.eq_ignore_ascii_case("HKEY_USERS") || root_name.eq_ignore_ascii_case("HKU");
    if !machine && !user && !users {
        return false;
    }
    let kind_ok = |k: &str| kind.map(|kind| eq_ci(kind, k)).unwrap_or(true);
    if kind_ok("media-codec") && machine && source == "Media Codec"
        && key == r"software\microsoft\windows nt\currentversion\drivers32" {
        return true;
    }
    if kind_ok("url-search-hook") && user && source == "URL Search Hook"
        && key == r"software\microsoft\internet explorer\urlsearchhooks" {
        return true;
    }
    if source == "Internet Explorer Add-on" {
        let sid = r"(?<sid>S-1-5-21-(?:\d+-){2,}\d+\\)?";
        let pattern = format!("^{}SOFTWARE\\\\(?:WOW6432Node\\\\)?Microsoft\\\\Internet Explorer\\\\(?<leaf>URLSearchHooks|Toolbar)$", sid);
        if let Ok(re) = regex::RegexBuilder::new(&pattern).case_insensitive(true).build() {
            if let Some(caps) = re.captures(&key) {
                let sid_present = caps.name("sid").is_some();
                if users == sid_present {
                    let toolbar = caps.name("leaf").map(|m| m.as_str()).unwrap_or("").eq_ignore_ascii_case("Toolbar");
                    if kind.map(|k| (toolbar && eq_ci(k, "ie-toolbar")) || (!toolbar && eq_ci(k, "ie-search-hook"))).unwrap_or(true) {
                        return true;
                    }
                }
            }
        }
    }
    if kind_ok("explorer-shell-hook") && source == "Explorer Shell Extension" {
        let sid = r"(?<sid>S-1-5-21-(?:\d+-){2,}\d+\\)?";
        let pattern = format!("^{}SOFTWARE\\\\(?:WOW6432Node\\\\)?Microsoft\\\\Windows\\\\CurrentVersion\\\\Explorer\\\\ShellExecuteHooks$", sid);
        if let Ok(re) = regex::RegexBuilder::new(&pattern).case_insensitive(true).build() {
            if let Some(caps) = re.captures(&key) {
                let sid_present = caps.name("sid").is_some();
                if users == sid_present {
                    return true;
                }
            }
        }
    }
    false
}

fn apply_extension_disable_capability(item: &mut StartupItem, kind: &str, root: RegRoot, key: &str, value: &str,
                                       scope: &str, source: &str, view: &str) {
    if !extension_disable_surface(Some(kind), root_display(root), key, value, source) {
        return;
    }
    let view_parsed = RegView::parse(if view.is_empty() { native_registry_view().label() } else { view });
    let (writable, access_reason) = can_open_registry_value_for_write(root, key, value, view_parsed);
    let capability = decide_registry_mutation_capability(scope, root_display(root), writable, is_elevated(), "", Some(key));
    if !capability.can_mutate {
        item.mutation_reason = format!("{}{}", capability.reason, if access_reason.is_empty() { String::new() } else { format!(" ({})", access_reason) });
        return;
    }
    let current_user = matches!(root, RegRoot::CurrentUser);
    item.can_disable = true;
    item.mutation_capability = "TransactionalRegistryValue".into();
    item.mutation_reason = "Disabling removes only this one registry value after saving its exact data; Enable writes it back unchanged.".into();
    item.requires_expert_confirmation = !current_user;
    const READ_ONLY_SUFFIX: &str = "; read-only route";
    if let Some(stripped) = item.status.strip_suffix(READ_ONLY_SUFFIX) {
        item.status = stripped.to_string();
    }
}

/// C# `AddRegistrySubKeyDefaultValues` — subkey registrations read via their default value.
pub fn add_subkey_default_values(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, parent_path: &str,
                                  scope: &str, source: &str, _can_edit: bool, status: &str, view_label: &str) {
    let Some(parent) = open_subkey(root, parent_path, view) else { return };
    for child_name in parent.subkey_names() {
        let child_path = format!("{}\\{}", parent_path, child_name);
        let Some(child) = open_subkey(root, &child_path, view) else { continue };
        let default = child.value("").map(|(_, v)| v.display()).unwrap_or_default();
        if default.trim().is_empty() {
            continue;
        }
        items.push(StartupItem {
            id: ids::reg_id(scope, root_display(root), &child_path, &child_name, view_label),
            name: child_name.clone(),
            source: source.into(),
            scope: scope.into(),
            command: default,
            location: format!("{}\\{}{}", root_display(root), child_path, if view_label.is_empty() { String::new() } else { format!(" [{}]", view_label) }),
            enabled: true,
            can_disable: false,
            status: if view_label.is_empty() { status.into() } else { format!("{}; {}", status, view_label) },
            registry_view: view_label.into(),
            mutation_capability: "ReadOnly".into(),
            mutation_reason: "This entry is a subkey registration; disabling requires removing the subkey which may affect system components".into(),
            ..StartupItem::new()
        });
    }
}

fn add_registry_extension_value(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str,
                                  source: &str, kind: &str, value_name: &str, status: &str, view_label: &str, expand_system_module: bool) {
    let Some(key) = open_subkey(root, sub_key, view) else { return };
    let raw = key.value(value_name).map(|(_, v)| v.display()).unwrap_or_default();
    if raw.trim().is_empty() {
        return;
    }
    let command = if expand_system_module { crate::paths::expand_system_module_path(&raw) } else { raw.clone() };
    items.push(read_only_extension_item(kind, root, sub_key, value_name, scope, source, &command, status, view_label, true));
}

fn add_registry_extension_subkey_value(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, parent_path: &str,
                                        scope: &str, source: &str, kind: &str, value_name: &str, status: &str, view_label: &str, expand_system_module: bool) {
    let Some(parent) = open_subkey(root, parent_path, view) else { return };
    let mut children = parent.subkey_names();
    children.sort_by_key(|c| c.to_lowercase());
    for child in children {
        add_registry_extension_value(items, root, view, &format!("{}\\{}", parent_path, child), scope, source, kind, value_name, status, view_label, expand_system_module);
    }
}

fn add_registry_extension_values(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str,
                                   source: &str, kind: &str, status: &str, view_label: &str, excluded: Option<&[&str]>, expand_system_module: bool) {
    let Some(key) = open_subkey(root, sub_key, view) else { return };
    let mut names = key.value_names();
    names.sort_by_key(|n| n.to_lowercase());
    for value_name in names {
        if let Some(excluded) = excluded {
            if excluded.iter().any(|x| eq_ci(x, &value_name)) {
                continue;
            }
        }
        let raw = key.value(&value_name).map(|(_, v)| v.display()).unwrap_or_default();
        if raw.trim().is_empty() {
            continue;
        }
        let command = if expand_system_module { crate::paths::expand_system_module_path(&raw) } else { raw.clone() };
        items.push(read_only_extension_item(kind, root, sub_key, &value_name, scope, source, &command, status, view_label, true));
    }
}

fn add_registry_extension_list_value(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, scope: &str,
                                       source: &str, kind: &str, value_name: &str, status: &str, view_label: &str) {
    let Some(key) = open_subkey(root, sub_key, view) else { return };
    let raw = key.value(value_name).map(|(_, v)| v.display()).unwrap_or_default();
    let parts: Vec<String> = raw.split([',', ';', '\n']).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        items.push(read_only_extension_item(kind, root, sub_key,
            &format!("{}{}", value_name, if parts.len() > 1 { format!("#{}", i + 1) } else { String::new() }),
            scope, source, &crate::paths::expand_system_module_path(part), status, view_label, true));
    }
}

/// C# `AddWinlogonNotificationDlls`.
pub fn add_winlogon_notification_dlls(items: &mut Vec<StartupItem>) {
    for view in startup_registry_views() {
        add_registry_extension_subkey_value(items, RegRoot::LocalMachine, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\Notify",
            "Machine", "Winlogon Notification", "winlogon-notify", "DllName", "Winlogon loads this notification DLL during logon events", view.label(), true);
    }
}

/// C# `AddImageHijackEntries` (IFEO + SilentProcessExit).
pub fn add_image_hijack_entries(items: &mut Vec<StartupItem>) {
    const IFEO: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options";
    const SILENT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\SilentProcessExit";
    for view in startup_registry_views() {
        let label = view.label();
        add_registry_extension_subkey_value(items, RegRoot::LocalMachine, view, IFEO, "Machine", "Image Hijack", "image-hijack", "Debugger", "Launches the configured debugger whenever the named image starts", label, false);
        add_registry_extension_subkey_value(items, RegRoot::LocalMachine, view, IFEO, "Machine", "Image Hijack", "image-hijack", "VerifierDlls", "Loads the configured verifier DLLs whenever the named image starts", label, false);
        add_registry_extension_subkey_value(items, RegRoot::LocalMachine, view, SILENT, "Machine", "Image Hijack", "image-hijack", "MonitorProcess", "Launches the configured monitor when the named image exits silently", label, false);
    }
}

/// C# `AddKnownDllEntries`.
pub fn add_known_dll_entries(items: &mut Vec<StartupItem>) {
    let session_manager = r"SYSTEM\CurrentControlSet\Control\Session Manager";
    add_registry_extension_values(items, RegRoot::LocalMachine, RegView::Default, &format!("{}\\KnownDLLs", session_manager),
        "Machine", "Known DLL", "known-dll", "Known DLL loaded into processes that import it", native_registry_view().label(), Some(&["DllDirectory"]), true);
    add_registry_extension_values(items, RegRoot::LocalMachine, RegView::Default, &format!("{}\\KnownDLLs32", session_manager),
        "Machine", "Known DLL", "known-dll", "Known DLL loaded into 32-bit processes that import it", native_registry_view().label(), Some(&["DllDirectory"]), true);
}

/// C# `AddNetworkProviderEntries`.
pub fn add_network_provider_entries(items: &mut Vec<StartupItem>) {
    let order_path = r"SYSTEM\CurrentControlSet\Control\NetworkProvider\Order";
    let Some(key) = open_subkey(RegRoot::LocalMachine, order_path, RegView::Default) else { return };
    let order = key.value("ProviderOrder").map(|(_, v)| v.display()).unwrap_or_default();
    for provider in parse_network_provider_order(&order) {
        let path = format!(r"SYSTEM\CurrentControlSet\Services\{}\NetworkProvider", provider);
        add_registry_extension_value(items, RegRoot::LocalMachine, RegView::Default, &path, "Machine", "Network Provider",
            "network-provider", "ProviderPath", "Network provider loaded during interactive logon", native_registry_view().label(), true);
    }
}

pub fn parse_network_provider_order(value: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for name in value.split(',') {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if !result.iter().any(|r| r.as_str().eq_ignore_ascii_case(name)) {
            result.push(name.to_string());
        }
    }
    result
}

/// C# `AddPrintMonitorEntries`.
pub fn add_print_monitor_entries(items: &mut Vec<StartupItem>) {
    add_registry_extension_subkey_value(items, RegRoot::LocalMachine, RegView::Default, r"SYSTEM\CurrentControlSet\Control\Print\Monitors",
        "Machine", "Print Monitor", "print-monitor", "Driver", "Print Spooler loads this monitor DLL when the spooler service starts", native_registry_view().label(), true);
}

/// C# `CollectLogonExtensionPoints` — RDP startup, security providers, icon service,
/// print providers, URL search hooks, service dependencies.
pub fn collect_logon_extension_points(items: &mut Vec<StartupItem>) {
    add_registry_extension_list_value(items, RegRoot::LocalMachine, RegView::Default, r"SYSTEM\CurrentControlSet\Control\Terminal Server\Wds\rdpwd",
        "Machine", "Terminal Server Startup", "terminal-server-startup", "StartupPrograms", "Programs started in every Remote Desktop session at sign-in", native_registry_view().label());
    add_registry_extension_list_value(items, RegRoot::LocalMachine, RegView::Default, r"SYSTEM\CurrentControlSet\Control\SecurityProviders",
        "Machine", "Security Provider", "security-provider", "SecurityProviders", "Security package loaded by the Local Security Authority", native_registry_view().label());
    for view in startup_registry_views() {
        add_registry_extension_value(items, RegRoot::LocalMachine, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows",
            "Machine", "Icon Service Library", "icon-service-library", "IconServiceLib", "Icon codec library loaded into every interactive process", view.label(), true);
    }
    add_registry_extension_subkey_value(items, RegRoot::LocalMachine, RegView::Default, r"SYSTEM\CurrentControlSet\Control\Print\Providers",
        "Machine", "Print Provider", "print-provider", "Name", "Print Spooler loads this provider DLL when the spooler service starts", native_registry_view().label(), true);
    add_url_search_hook_entries(items);
    add_service_dependency_entries(items);
}

/// C# `AddUrlSearchHookEntries`.
pub fn add_url_search_hook_entries(items: &mut Vec<StartupItem>) {
    let hooks = r"Software\Microsoft\Internet Explorer\UrlSearchHooks";
    let Some(key) = open_subkey(RegRoot::CurrentUser, hooks, RegView::Default) else { return };
    let mut names = key.value_names();
    names.sort_by_key(|n| n.to_lowercase());
    for clsid in names {
        if clsid.trim().is_empty() {
            continue;
        }
        let mut server = String::new();
        for view in startup_registry_views() {
            if let Some(inproc) = open_subkey(RegRoot::LocalMachine, &format!(r"SOFTWARE\Classes\CLSID\{}\InprocServer32", normalize_com_class_id(&clsid)), view) {
                server = inproc.value("").map(|(_, v)| v.display()).unwrap_or_default();
            }
            if !server.trim().is_empty() {
                break;
            }
        }
        let command = if server.trim().is_empty() { clsid.clone() } else { crate::paths::expand_system_module_path(&server) };
        items.push(read_only_extension_item("url-search-hook", RegRoot::CurrentUser, hooks, &clsid, "User", "URL Search Hook",
            &command, &format!("Internet Explorer URL search hook (COM class {})", clsid), native_registry_view().label(), true));
    }
}

/// C# `AddServiceDependencyEntries` — manual services pulled in at boot by auto services.
pub fn add_service_dependency_entries(items: &mut Vec<StartupItem>) {
    let services_path = r"SYSTEM\CurrentControlSet\Services";
    let Some(services) = open_subkey(RegRoot::LocalMachine, services_path, RegView::Default) else { return };
    struct Info { start: i64, kind: i64, depends: Vec<String>, image: String }
    let mut info: std::collections::HashMap<String, Info> = std::collections::HashMap::new();
    for name in services.subkey_names() {
        let Some(key) = open_subkey(RegRoot::LocalMachine, &format!("{}\\{}", services_path, name), RegView::Default) else { continue };
        let start = match key.value("Start") {
            Some((_, RegValue::DWord(v))) => v as i64,
            Some((_, RegValue::QWord(v))) => v as i64,
            Some((_, RegValue::Str(s))) => s.trim().parse().unwrap_or(3),
            _ => 3,
        };
        let kind = match key.value("Type") {
            Some((_, RegValue::DWord(v))) => v as i64,
            Some((_, RegValue::QWord(v))) => v as i64,
            Some((_, RegValue::Str(s))) => s.trim().parse().unwrap_or(0),
            _ => 0,
        };
        let depends = match key.value("DependOnService") {
            Some((_, RegValue::MultiString(parts))) => parts,
            _ => Vec::new(),
        };
        let image = key.value("ImagePath").map(|(_, v)| v.display()).unwrap_or_default();
        info.insert(name, Info { start, kind, depends, image });
    }
    let mut caused_by: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut pending: std::collections::VecDeque<(String, String)> = std::collections::VecDeque::new();
    for (name, i) in &info {
        if (0..=2).contains(&i.start) && (i.kind & 0x0F) != 0x0F {
            for dep in &i.depends {
                pending.push_back((dep.clone(), name.clone()));
            }
        }
    }
    while let Some((dep, via)) = pending.pop_front() {
        let dep = dep.trim().to_string();
        if dep.is_empty() || dep.starts_with('+') || caused_by.contains_key(&dep) {
            continue;
        }
        let Some(dep_info) = info.get(&dep) else { continue };
        if (0..=2).contains(&dep_info.start) {
            for inner in &dep_info.depends {
                pending.push_back((inner.clone(), via.clone()));
            }
            continue;
        }
        if dep_info.start != 3 {
            continue;
        }
        caused_by.insert(dep.clone(), via.clone());
        for inner in &dep_info.depends {
            pending.push_back((inner.clone(), via.clone()));
        }
    }
    let mut caused: Vec<(String, String)> = caused_by.into_iter().collect();
    caused.sort_by_key(|(k, _)| k.to_lowercase());
    for (dep, via) in caused {
        let Some(dep_info) = info.get(&dep) else { continue };
        let driver = (dep_info.kind & 0x0F) != 0;
        let mut item = read_only_extension_item("service-dependency", RegRoot::LocalMachine, &format!("{}\\{}", services_path, dep),
            "Start", "Machine", "Service Dependency", &dep_info.image,
            &format!("{} {} is set to Manual but starts at every boot because auto-start service {} depends on it; disable {} to stop it",
                if driver { "Driver" } else { "Service" }, dep, via, via),
            native_registry_view().label(), true);
        item.name = format!("{} (needed by {})", dep, via);
        items.push(item);
    }
}

/// C# `AddMediaCodecEntries`.
pub fn add_media_codec_entries(items: &mut Vec<StartupItem>) {
    for view in startup_registry_views() {
        add_registry_extension_values(items, RegRoot::LocalMachine, view, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Drivers32",
            "Machine", "Media Codec", "media-codec", "Media codec loaded when its media class is used", view.label(), None, true);
    }
}

/// C# `AddExplorerShellExtensions`.
pub fn add_explorer_shell_extensions(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str, path_prefix: Option<&str>, view_label: &str) {
    let prefix = path_prefix.unwrap_or("");
    let explorer = format!(r"{}SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer", prefix);
    collect_com_extension_value_names(items, root, view, &format!("{}\\ShellExecuteHooks", explorer), scope, "Explorer Shell Extension",
        "explorer-shell-hook", "Explorer invokes this COM hook when it executes a shell command", view_label);
    collect_com_extension_subkeys(items, root, view, &format!("{}\\ShellIconOverlayIdentifiers", explorer), scope, "Explorer Shell Extension",
        "explorer-icon-overlay", "Explorer loads this COM icon-overlay handler during desktop activity", view_label);
}

pub fn audit_explorer_shell_extensions(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str, path_prefix: Option<&str>, view_label: &str) {
    add_explorer_shell_extensions(items, root, view, scope, path_prefix, view_label);
}

/// C# `AddInternetExplorerAddons`.
pub fn add_internet_explorer_addons(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str, path_prefix: Option<&str>, view_label: &str) {
    let prefix = path_prefix.unwrap_or("");
    collect_com_extension_subkeys(items, root, view, &format!(r"{}SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Browser Helper Objects", prefix),
        scope, "Internet Explorer Add-on", "ie-bho", "Internet Explorer loads this browser helper object", view_label);
    collect_com_extension_value_names(items, root, view, &format!(r"{}SOFTWARE\Microsoft\Internet Explorer\Toolbar", prefix),
        scope, "Internet Explorer Add-on", "ie-toolbar", "Internet Explorer loads this registered toolbar COM object", view_label);
    collect_com_extension_value_names(items, root, view, &format!(r"{}SOFTWARE\Microsoft\Internet Explorer\URLSearchHooks", prefix),
        scope, "Internet Explorer Add-on", "ie-search-hook", "Internet Explorer loads this registered URL search hook", view_label);
    collect_internet_explorer_extension_subkeys(items, root, view, &format!(r"{}SOFTWARE\Microsoft\Internet Explorer\Extensions", prefix), scope, view_label);
}

pub fn audit_internet_explorer_addons(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, scope: &str, path_prefix: Option<&str>, view_label: &str) {
    add_internet_explorer_addons(items, root, view, scope, path_prefix, view_label);
}

fn collect_com_extension_value_names(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str,
                                      scope: &str, source: &str, kind: &str, status: &str, view_label: &str) {
    let Some(key) = open_subkey(root, sub_key, view) else { return };
    let mut names = key.value_names();
    names.sort_by_key(|n| n.to_lowercase());
    for value_name in names {
        let value = key.value(&value_name).map(|(_, v)| v.display()).unwrap_or_default();
        let clsid = normalize_com_class_id(&value_name);
        let clsid = if clsid.is_empty() { normalize_com_class_id(&value) } else { clsid };
        if clsid.is_empty() {
            continue;
        }
        append_com_extension_item(items, root, view, sub_key, &value_name, scope, source, kind, &clsid, status, view_label);
    }
}

fn collect_com_extension_subkeys(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, parent_path: &str,
                                  scope: &str, source: &str, kind: &str, status: &str, view_label: &str) {
    let Some(parent) = open_subkey(root, parent_path, view) else { return };
    let mut children = parent.subkey_names();
    children.sort_by_key(|c| c.to_lowercase());
    for child_name in children {
        let child_path = format!("{}\\{}", parent_path, child_name);
        let Some(child) = open_subkey(root, &child_path, view) else { continue };
        let clsid_value = child.value("CLSID").map(|(_, v)| v.display()).unwrap_or_default();
        let default_value = child.value("").map(|(_, v)| v.display()).unwrap_or_default();
        let mut clsid = normalize_com_class_id(&clsid_value);
        let mut value_name = "CLSID".to_string();
        if clsid.is_empty() {
            clsid = normalize_com_class_id(&default_value);
            value_name = String::new();
        }
        if clsid.is_empty() {
            clsid = normalize_com_class_id(&child_name);
            value_name = String::new();
        }
        if clsid.is_empty() {
            continue;
        }
        append_com_extension_item(items, root, view, &child_path, &value_name, scope, source, kind, &clsid, status, view_label);
    }
}

fn collect_internet_explorer_extension_subkeys(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, parent_path: &str,
                                                scope: &str, view_label: &str) {
    let Some(parent) = open_subkey(root, parent_path, view) else { return };
    let mut children = parent.subkey_names();
    children.sort_by_key(|c| c.to_lowercase());
    for child_name in children {
        let child_path = format!("{}\\{}", parent_path, child_name);
        let Some(child) = open_subkey(root, &child_path, view) else { continue };
        let clsid = normalize_com_class_id(&child.value("CLSID").map(|(_, v)| v.display()).unwrap_or_default());
        let mut clsid_value_name = "CLSID".to_string();
        let clsid = if clsid.is_empty() {
            clsid_value_name = String::new();
            normalize_com_class_id(&child_name)
        } else {
            clsid
        };
        if !clsid.is_empty() {
            append_com_extension_item(items, root, view, &child_path, &clsid_value_name, scope, "Internet Explorer Add-on", "ie-extension",
                &clsid, "Internet Explorer loads this registered extension COM object", view_label);
        }
        for value_name in ["Exec", "Script"] {
            let command = child.value(value_name).map(|(_, v)| v.display()).unwrap_or_default();
            if command.trim().is_empty() {
                continue;
            }
            let mut item = read_only_extension_item("ie-extension", root, &child_path, value_name, scope, "Internet Explorer Add-on",
                &expand_env(&command), &format!("Internet Explorer extension {} registration", value_name), view_label, false);
            item.requires_reboot = false;
            items.push(item);
        }
    }
}

fn append_com_extension_item(items: &mut Vec<StartupItem>, root: RegRoot, view: RegView, sub_key: &str, value_name: &str,
                               scope: &str, source: &str, kind: &str, class_id: &str, status: &str, view_label: &str) {
    let path = resolve_com_server_path(root, view, sub_key, class_id);
    let unresolved = path.trim().is_empty();
    let command = if unresolved { format!("COM {}", class_id) } else { path };
    let item_status = format!("{}; CLSID={}{}", status, class_id, if unresolved { "; server path unresolved" } else { "" });
    let mut item = read_only_extension_item(kind, root, sub_key, value_name, scope, source, &command, &item_status, view_label, false);
    item.requires_reboot = false;
    items.push(item);
}

/// `Guid.TryParse(...).ToString("B")` — lowercase canonical GUID form.
pub fn normalize_com_class_id(value: &str) -> String {
    let trimmed = value.trim();
    let digits: String = trimmed.chars().filter(|c| c.is_ascii_hexdigit() || *c == '-').collect();
    let bare = trimmed.trim_matches(|c| c == '{' || c == '}');
    let parts: Vec<&str> = bare.split('-').collect();
    if parts.len() == 5 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_hexdigit())) {
        return format!("{{{}}}", digits.to_lowercase());
    }
    String::new()
}

fn resolve_com_server_path(root: RegRoot, view: RegView, registration_path: &str, class_id: &str) -> String {
    let path = registration_path;
    let software_index = path.to_lowercase().find("software\\");
    let profile_prefix = software_index.map(|i| &path[..i]).unwrap_or("");
    let classes_path = format!("{}SOFTWARE\\Classes\\CLSID\\{}", profile_prefix, class_id);
    for server_kind in ["InprocServer32", "LocalServer32", "InprocHandler32"] {
        if let Some(server) = open_subkey(root, &format!("{}\\{}", classes_path, server_kind), view) {
            let command = server.value("").map(|(_, v)| v.display()).unwrap_or_default();
            if !command.trim().is_empty() {
                return expand_env(command.trim());
            }
        }
    }
    String::new()
}

/// C# `AddExtendedAutorunSources`.
pub fn add_extended_autorun_sources(items: &mut Vec<StartupItem>) {
    add_winlogon_notification_dlls(items);
    add_image_hijack_entries(items);
    add_known_dll_entries(items);
    add_network_provider_entries(items);
    add_winsock_provider_entries(items);
    add_print_monitor_entries(items);
    collect_logon_extension_points(items);
    add_media_codec_entries(items);
}

/// Independent audit walk of the extended sources.
pub fn audit_extended_autorun_sources(raw: &mut Vec<StartupItem>) {
    add_winlogon_notification_dlls(raw);
    add_image_hijack_entries(raw);
    add_known_dll_entries(raw);
    add_network_provider_entries(raw);
    add_winsock_provider_entries(raw);
    add_print_monitor_entries(raw);
    collect_logon_extension_points(raw);
    add_media_codec_entries(raw);
}

// ---- Winsock catalog ----

#[repr(C)]
struct WinsockProtocolChain {
    chain_length: i32,
    chain_entries: [u32; 7],
}

#[repr(C)]
struct WinsockProtocolInfo {
    service_flags1: u32,
    service_flags2: u32,
    service_flags3: u32,
    service_flags4: u32,
    provider_flags: u32,
    provider_id: windows::core::GUID,
    catalog_entry_id: u32,
    protocol_chain: WinsockProtocolChain,
    version: i32,
    address_family: i32,
    max_sockaddr: i32,
    min_sockaddr: i32,
    socket_type: i32,
    protocol: i32,
    protocol_max_offset: i32,
    network_byte_order: i32,
    security_scheme: i32,
    message_size: u32,
    provider_reserved: u32,
    protocol_name: [u16; 256],
}

#[link(name = "ws2_32")]
extern "system" {
    fn WSAStartup(version_requested: u16, wsa_data: *mut u8) -> i32;
    fn WSACleanup() -> i32;
    fn WSCEnumProtocols(protocols: *mut u32, buffer: *mut WinsockProtocolInfo, length: *mut i32, error: *mut i32) -> i32;
    fn WSCEnumProtocols32(protocols: *mut u32, buffer: *mut WinsockProtocolInfo, length: *mut i32, error: *mut i32) -> i32;
    fn WSCGetProviderPath(provider_id: *const windows::core::GUID, path: *mut u16, length: *mut i32, error: *mut i32) -> i32;
    fn WSCGetProviderPath32(provider_id: *const windows::core::GUID, path: *mut u16, length: *mut i32, error: *mut i32) -> i32;
}

pub fn add_winsock_provider_entries(items: &mut Vec<StartupItem>) {
    add_winsock_catalog(items, false);
    if is_64bit_os() {
        add_winsock_catalog(items, true);
    }
}

fn winsock_catalog_label(catalog32: bool) -> &'static str {
    if catalog32 { "Winsock 32-bit Catalog" } else { "Winsock 64-bit Catalog" }
}

fn add_winsock_catalog(items: &mut Vec<StartupItem>, catalog32: bool) {
    let catalog = winsock_catalog_label(catalog32);
    match enumerate_winsock_infos(catalog32) {
        Ok(infos) => {
            for info in infos {
                let path = get_winsock_provider_path(info.provider_id, catalog32)
                    .unwrap_or_else(|e| { items.push(error_item("Winsock Provider", "Machine", catalog, &e)); String::new() });
                if path.is_empty() {
                    continue;
                }
                items.push(winsock_item(&info, catalog32, &path));
            }
        }
        Err(e) => items.push(error_item("Winsock Provider", "Machine", catalog, &e)),
    }
}

fn winsock_item(info: &WinsockProtocolInfo, catalog32: bool, provider_path: &str) -> StartupItem {
    let entry = info.catalog_entry_id.to_string();
    let provider = format!("{{{}}}", guid_to_string(&info.provider_id));
    let protocol = crate::paths::wide_to_string(&info.protocol_name).trim().to_string();
    StartupItem {
        id: format!("winsock|{}|{}|{}", if catalog32 { 32 } else { 64 }, entry, provider),
        name: format!("{} - catalog {}", if protocol.is_empty() { "Winsock provider".to_string() } else { protocol }, entry),
        source: "Winsock Provider".into(),
        scope: "Machine".into(),
        command: provider_path.to_string(),
        location: format!("{}\\Entry {} [{}]", winsock_catalog_label(catalog32), entry, provider),
        enabled: true,
        can_disable: false,
        status: format!("Winsock service provider; catalog entry={}; chainLength={}; read-only route", entry, info.protocol_chain.chain_length),
        registry_view: if catalog32 { "Winsock32".into() } else { "Winsock64".into() },
        mutation_capability: "ReadOnly".into(),
        mutation_reason: "This Windows network provider is shown for complete inventory coverage; Startup Master will not guess at a potentially connection-breaking catalog rewrite.".into(),
        requires_expert_confirmation: true,
        requires_reboot: true,
        ..StartupItem::new()
    }
}

fn guid_to_string(guid: &windows::core::GUID) -> String {
    format!("{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        guid.data1, guid.data2, guid.data3, guid.data4[0], guid.data4[1],
        guid.data4[2], guid.data4[3], guid.data4[4], guid.data4[5], guid.data4[6], guid.data4[7]).to_lowercase()
}

fn enumerate_winsock_infos(catalog32: bool) -> Result<Vec<WinsockProtocolInfo>, String> {
    if catalog32 && !is_64bit_os() {
        return Ok(Vec::new());
    }
    let mut startup_data = [0u8; 512];
    let startup = unsafe { WSAStartup(0x0202, startup_data.as_mut_ptr()) };
    if startup != 0 {
        return Err(format!("WSAStartup failed with {}", startup));
    }
    let result = (|| {
        let mut capacity: i32 = 16 * 1024;
        for _ in 0..3 {
            let mut buffer = vec![0u8; capacity as usize];
            let mut requested = capacity;
            let mut error = 0i32;
            let count = unsafe {
                if catalog32 {
                    WSCEnumProtocols32(std::ptr::null_mut(), buffer.as_mut_ptr() as *mut WinsockProtocolInfo, &mut requested, &mut error)
                } else {
                    WSCEnumProtocols(std::ptr::null_mut(), buffer.as_mut_ptr() as *mut WinsockProtocolInfo, &mut requested, &mut error)
                }
            };
            if count != -1 {
                let size = std::mem::size_of::<WinsockProtocolInfo>() as i32;
                if count < 0 || (count as i64 * size as i64) > capacity as i64 {
                    return Err("Winsock catalog returned an invalid entry count".into());
                }
                let ptr = buffer.as_ptr() as *const WinsockProtocolInfo;
                let mut infos = Vec::with_capacity(count as usize);
                for i in 0..count as usize {
                    infos.push(unsafe { std::ptr::read(ptr.add(i)) });
                }
                return Ok(infos);
            }
            if error != 10055 || requested <= capacity || requested > 1024 * 1024 {
                return Err(format!("WSCEnumProtocols{} failed with {}", if catalog32 { "32" } else { "" }, error));
            }
            capacity = requested;
        }
        Err("Winsock catalog changed repeatedly while it was being enumerated".into())
    })();
    unsafe { WSACleanup() };
    result
}

fn get_winsock_provider_path(provider_id: windows::core::GUID, catalog32: bool) -> Result<String, String> {
    let mut capacity = 512i32;
    for _ in 0..3 {
        let mut buffer = vec![0u16; capacity as usize];
        let mut requested = capacity;
        let mut error = 0i32;
        let result = unsafe {
            if catalog32 {
                WSCGetProviderPath32(&provider_id, buffer.as_mut_ptr(), &mut requested, &mut error)
            } else {
                WSCGetProviderPath(&provider_id, buffer.as_mut_ptr(), &mut requested, &mut error)
            }
        };
        if result != -1 {
            let expanded = expand_env(crate::paths::wide_to_string(&buffer).trim());
            if expanded.is_empty() {
                return Err("Winsock provider returned an empty DLL path".into());
            }
            return Ok(expanded);
        }
        if error != 10014 || requested <= capacity || requested > 32768 {
            return Err(format!("WSCGetProviderPath{} failed with {}", if catalog32 { "32" } else { "" }, error));
        }
        capacity = requested;
    }
    Err("Winsock provider path changed repeatedly while it was being queried".into())
}
