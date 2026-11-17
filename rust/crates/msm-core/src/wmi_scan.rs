//! WMI surfaces: Win32_StartupCommand fallback and permanent event consumers.

use crate::hive::loaded_user_hives;
use crate::ids;
use crate::model::{error_item, StartupItem};
use crate::util::{current_user_sid, eq_ci};
use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct StartupCommandRow {
    #[serde(default)] name: Option<String>,
    #[serde(default)] command: Option<String>,
    #[serde(default)] location: Option<String>,
    #[serde(default)] user: Option<String>,
    #[serde(default)] user_sid: Option<String>,
}

fn wmi_connection(namespace: &str) -> Result<wmi::COMLibrary, String> {
    wmi::COMLibrary::new().map_err(|e| format!("WMI COM initialization failed: {}", e))
}

/// C# `AddWmiStartupCommands` — Win32_StartupCommand compatibility projection.
pub fn add_wmi_startup_commands(items: &mut Vec<StartupItem>) {
    let joined: Result<Vec<StartupItem>, String> = std::thread::Builder::new()
        .name("wmi-startup-command".into())
        .spawn(|| -> Result<Vec<StartupItem>, String> {
            let com = wmi::COMLibrary::new().map_err(|e| format!("WMI COM initialization failed: {}", e))?;
            let wmi = wmi::WMIConnection::with_namespace_path(r"ROOT\CIMV2", com)
                .map_err(|e| format!("WMI connection failed: {}", e))?;
            let rows: Vec<StartupCommandRow> = wmi.raw_query(
                "SELECT Name, Command, Location, User, UserSID FROM Win32_StartupCommand")
                .map_err(|e| format!("Win32_StartupCommand query failed: {}", e))?;
            let installed = crate::humans::read_installed_product_name_prefixes(None);
            let mut out = Vec::new();
            for row in rows {
                let empty = || String::new();
                let name_raw = row.name.unwrap_or_else(empty);
                let command = row.command.unwrap_or_else(empty);
                let location = row.location.unwrap_or_else(empty);
                let user = row.user.unwrap_or_else(empty);
                let user_sid = row.user_sid.unwrap_or_else(empty);
                let name = if name_raw.trim().is_empty() { "(startup)".to_string() } else { name_raw.clone() };
                let scope = wmi_startup_command_scope(&user, &user_sid, &location);
                let app_name = crate::humans::installed_product_name_for_command(&command, &installed);
                let status = format!("Reported by Windows Win32_StartupCommand fallback{}{}",
                    if user.trim().is_empty() { String::new() } else { format!("; user={}", user) },
                    if user_sid.trim().is_empty() { String::new() } else { format!("; sid={}", user_sid) });
                out.push(StartupItem {
                    id: ids::wmi_startup_command_id(&location, &name, &user_sid, &user, &command),
                    name,
                    app_name,
                    source: "Startup Command".into(),
                    scope,
                    command,
                    location,
                    enabled: true,
                    can_disable: false,
                    status,
                    ..StartupItem::new()
                });
            }
            Ok(out)
        })
        .map_err(|e: std::io::Error| format!("WMI worker spawn failed: {}", e))
        .and_then(|handle| handle.join().map_err(|_| "WMI worker panicked".to_string()))
        .and_then(|inner| inner);
    match joined {
        Ok(rows) => items.extend(rows),
        Err(e) => items.push(error_item("Startup Command", "System", "Win32_StartupCommand", &e)),
    }
}

/// C# `WmiStartupCommandScope`.
pub fn wmi_startup_command_scope(user: &str, sid: &str, location: &str) -> String {
    if eq_ci(location.trim(), "Common Startup") || eq_ci(location.trim(), "Common AltStartup") {
        return "Machine".into();
    }
    if location.to_uppercase().starts_with("HKLM") {
        return "Machine".into();
    }
    let mut sid = sid.to_string();
    if sid.trim().is_empty() && (eq_ci(user.trim(), ".DEFAULT")
        || location.starts_with(r"HKU\.DEFAULT\") || location.starts_with(r"HKEY_USERS\.DEFAULT\")) {
        sid = "S-1-5-18".into();
    }
    let current_sid = current_user_sid();
    if !sid.trim().is_empty() && !current_sid.is_empty() && sid.eq_ignore_ascii_case(&current_sid) {
        return "User".into();
    }
    if let Some((_, scope)) = loaded_user_hives().into_iter().find(|(s, _)| s.eq_ignore_ascii_case(&sid)) {
        return scope;
    }
    let leaf = user.split(['\\', '/']).filter(|p| !p.is_empty()).last().unwrap_or("").to_string();
    if !leaf.trim().is_empty() {
        if eq_ci(&leaf, &crate::util::current_user_name()) {
            return "User".into();
        }
        return format!("User:{}", leaf);
    }
    "Machine/User".into()
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct ConsumerBindingRow {
    #[serde(default)] filter: String,
    #[serde(default)] consumer: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct ConsumerRow {
    #[serde(default)] name: String,
    #[serde(default)] command_line_template: String,
    #[serde(default)] executable_path: String,
    #[serde(default)] script_text: String,
    #[serde(default)] __class: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct FilterRow {
    #[serde(default)] query: String,
}

/// C# `AddExecutableWmiConsumers` — root\subscription permanent consumers.
pub fn add_executable_wmi_consumers(items: &mut Vec<StartupItem>) {
    let joined: Result<Vec<StartupItem>, String> = std::thread::Builder::new()
        .name("wmi-consumers".into())
        .spawn(|| -> Result<Vec<StartupItem>, String> {
            let com = wmi::COMLibrary::new().map_err(|e| format!("WMI COM initialization failed: {}", e))?;
            let wmi = wmi::WMIConnection::with_namespace_path(r"ROOT\SUBSCRIPTION", com)
                .map_err(|e| format!("WMI connection failed: {}", e))?;
            let bindings: Vec<ConsumerBindingRow> = wmi.raw_query("SELECT Filter, Consumer FROM __FilterToConsumerBinding")
                .map_err(|e| format!("WMI consumer query failed: {}", e))?;
            let mut out = Vec::new();
            for binding in bindings {
                if binding.consumer.trim().is_empty() {
                    continue;
                }
                let consumers: Vec<ConsumerRow> = match wmi.raw_query::<ConsumerRow>(&format!("SELECT * FROM {}", object_path(&binding.consumer))) {
                    Ok(rows) => rows,
                    Err(_) => continue,
                };
                let Some(consumer) = consumers.into_iter().next() else { continue };
                let class = consumer.__class.clone();
                if class != "CommandLineEventConsumer" && class != "ActiveScriptEventConsumer" {
                    continue;
                }
                let name = if consumer.name.trim().is_empty() { binding.consumer.clone() } else { consumer.name.clone() };
                let command = if class == "CommandLineEventConsumer" {
                    if !consumer.command_line_template.trim().is_empty() { consumer.command_line_template.clone() } else { consumer.executable_path.clone() }
                } else {
                    consumer.script_text.clone()
                };
                let mut query = String::new();
                if !binding.filter.trim().is_empty() {
                    if let Ok(filters) = wmi.raw_query::<FilterRow>(&format!("SELECT * FROM {}", object_path(&binding.filter))) {
                        if let Some(filter) = filters.into_iter().next() {
                            query = filter.query;
                        }
                    }
                }
                let location = format!("__FilterToConsumerBinding.Filter=\"{}\",Consumer=\"{}\"",
                    binding.filter.replace('\\', "\\\\").replace('"', "\\\""),
                    binding.consumer.replace('\\', "\\\\").replace('"', "\\\""));
                let status = format!("{}\t{}\t{}{}", binding.filter, binding.consumer, class,
                    if query.trim().is_empty() { String::new() } else { format!("\t{}", query) });
                out.push(StartupItem {
                    id: ids::wmisub_id(&location),
                    name,
                    source: "WMI Event Consumer".into(),
                    scope: "Machine".into(),
                    command,
                    location,
                    enabled: true,
                    can_disable: crate::util::is_elevated(),
                    status,
                    ..StartupItem::new()
                });
            }
            Ok(out)
        })
        .map_err(|e: std::io::Error| format!("WMI worker spawn failed: {}", e))
        .and_then(|handle| handle.join().map_err(|_| "WMI worker panicked".to_string()))
        .and_then(|inner| inner);
    match joined {
        Ok(rows) => items.extend(rows),
        Err(e) => items.push(error_item("WMI Event Consumer", "Machine", r"root\subscription", &e)),
    }
}

fn object_path(ref_path: &str) -> String {
    // `\\.\root\subscription:Consumer.Name="x"` → query by the trailing object descriptor.
    if let Some(colon) = ref_path.find(':') {
        ref_path[colon + 1..].to_string()
    } else {
        ref_path.to_string()
    }
}

/// Audit variant (same walk, plain rows).
pub fn audit_wmi_consumers(raw: &mut Vec<StartupItem>) {
    add_executable_wmi_consumers(raw)
}
