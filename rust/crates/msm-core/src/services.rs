//! Windows services and drivers: registry primary walk + native SCM audit.

use crate::ids;
use crate::model::{error_item, StartupItem};
use crate::reg::*;
use crate::util::is_elevated;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_MORE_DATA;
use windows::Win32::System::Services::{
    CloseServiceHandle, EnumServicesStatusExW, OpenSCManagerW, OpenServiceW, QueryServiceConfigW,
    ENUM_SERVICE_STATUS_PROCESSW, QUERY_SERVICE_CONFIGW, SC_ENUM_TYPE, SC_MANAGER_ENUMERATE_SERVICE,
    SERVICE_DRIVER, SERVICE_STATE_ALL, SERVICE_WIN32, SERVICE_QUERY_CONFIG,
};

/// C# `AddConfiguredServiceEntries` — registry-based configured services/drivers.
pub fn add_configured_service_entries(items: &mut Vec<StartupItem>, want_drivers: bool) {
    let source = if want_drivers { "System Driver" } else { "Windows Service" };
    let Some(services) = open_subkey(RegRoot::LocalMachine, r"SYSTEM\CurrentControlSet\Services", RegView::Default) else { return };
    for name in services.subkey_names() {
        let Some(key) = open_subkey(RegRoot::LocalMachine, &format!(r"SYSTEM\CurrentControlSet\Services\{}", name), RegView::Default) else { continue };
        let read_int = |value: &str, fallback: i64| match key.value(value) {
            Some((_, RegValue::DWord(v))) => v as i64,
            Some((_, RegValue::QWord(v))) => v as i64,
            Some((_, RegValue::Str(s))) => s.trim().parse().unwrap_or(fallback),
            _ => fallback,
        };
        let start = read_int("Start", 3);
        let kind = read_int("Type", 0);
        let is_driver = (kind & 0x0F) != 0;
        if is_driver != want_drivers || !(0..=2).contains(&start) {
            // Manual trigger-start services still run on their event: keep them visible.
            if want_drivers || start != 3 || open_subkey(RegRoot::LocalMachine, &format!(r"SYSTEM\CurrentControlSet\Services\{}\TriggerInfo", name), RegView::Default).is_none() {
                continue;
            }
            let mut trigger_display = resolve_indirect_string(&key.value("DisplayName").map(|(_, v)| v.display()).unwrap_or_else(|| name.clone()));
            if trigger_display.trim().is_empty() || trigger_display.starts_with('@') {
                trigger_display = name.clone();
            }
            let trigger_path = key.value("ImagePath").map(|(_, v)| v.display()).unwrap_or_default();
            items.push(StartupItem {
                id: ids::service_id(&name),
                name: trigger_display,
                source: "Service (Trigger Start)".into(),
                scope: "Machine".into(),
                command: trigger_path,
                location: format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{}", name),
                enabled: true,
                can_disable: is_elevated(),
                status: "Service start=Manual (trigger start) registryStart=3".into(),
                ..StartupItem::new()
            });
            continue;
        }
        let mut display_name = resolve_indirect_string(&key.value("DisplayName").map(|(_, v)| v.display()).unwrap_or_else(|| name.clone()));
        if display_name.trim().is_empty() || display_name.starts_with('@') {
            display_name = name.clone();
        }
        let path = key.value("ImagePath").map(|(_, v)| v.display()).unwrap_or_default();
        let delayed = read_int("DelayedAutoStart", 0);
        let mut overrides = String::new();
        if let Some(ov) = open_subkey(RegRoot::LocalMachine, &format!(r"SYSTEM\CurrentControlSet\Services\{}\StartOverride", name), RegView::Default) {
            let pairs: Vec<String> = ov.value_names().iter().map(|n| format!("{}={}", n, ov.value(n).map(|(_, v)| v.display()).unwrap_or_default())).collect();
            if !pairs.is_empty() {
                overrides = format!(" startOverride={}", pairs.join(","));
            }
        }
        let start_text = match start {
            0 => "Boot",
            1 => "System",
            _ => if delayed != 0 { "Automatic (delayed)" } else { "Automatic" },
        };
        let status = format!("{} start={} registryStart={}{}", if want_drivers { "Driver" } else { "Service" }, start_text, start, overrides);
        items.push(StartupItem {
            id: if want_drivers { ids::driver_id(&name) } else { ids::service_id(&name) },
            name: display_name,
            source: source.into(),
            scope: "Machine".into(),
            command: path,
            location: format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{}", name),
            enabled: true,
            can_disable: is_elevated(),
            status,
            ..StartupItem::new()
        });
    }
}

/// Resolve `@shell32.dll,-123` indirect display strings.
pub fn resolve_indirect_string(value: &str) -> String {
    if value.trim().is_empty() || !value.trim_start().starts_with('@') {
        return value.to_string();
    }
    unsafe {
        #[allow(non_snake_case)]
        let SHLoadIndirectString: unsafe extern "system" fn(windows::core::PCWSTR, windows::core::PWSTR, u32, Option<*mut core::ffi::c_void>) -> windows::core::HRESULT = {
            extern "system" {
                #[link_name = "SHLoadIndirectString"]
                fn imp(source: windows::core::PCWSTR, out: windows::core::PWSTR, size: u32, reserved: Option<*mut core::ffi::c_void>) -> windows::core::HRESULT;
            }
            imp
        };
        let wide = crate::paths::to_wide(value.trim());
        let mut buffer = vec![0u16; 1024];
        if SHLoadIndirectString(PCWSTR(wide.as_ptr()), PWSTR(buffer.as_mut_ptr()), buffer.len() as u32, None).is_ok() {
            let resolved = crate::paths::wide_to_string(&buffer).trim().to_string();
            if !resolved.is_empty() {
                return resolved;
            }
        }
    }
    value.to_string()
}

/// Does a service have a TriggerInfo key (trigger-start)?
pub fn has_service_trigger_info(service_name: &str) -> bool {
    open_subkey(RegRoot::LocalMachine, &format!(r"SYSTEM\CurrentControlSet\Services\{}\TriggerInfo", service_name), RegView::Default).is_some()
}

/// Read the exact `Start` value for a service.
pub fn read_service_start_value(service_name: &str, fallback: i32) -> i32 {
    match open_subkey(RegRoot::LocalMachine, &format!(r"SYSTEM\CurrentControlSet\Services\{}", service_name), RegView::Default) {
        Some(key) => match key.value("Start") {
            Some((_, RegValue::DWord(v))) => v as i32,
            Some((_, RegValue::QWord(v))) => v as i32,
            Some((_, RegValue::Str(s))) => s.trim().parse().unwrap_or(fallback),
            _ => fallback,
        },
        None => fallback,
    }
}

/// Write the exact `Start` value for a service.
pub fn set_service_start_value(service_name: &str, start: i32) -> Result<(), String> {
    match open_subkey_write(RegRoot::LocalMachine, &format!(r"SYSTEM\CurrentControlSet\Services\{}", service_name), RegView::Default) {
        Some(key) => {
            if key.set_value("Start", RegKind::DWord, &RegValue::DWord(start as u32)) {
                Ok(())
            } else {
                Err(format!("Service registry key write failed: {}", service_name))
            }
        }
        None => Err(format!("Service registry key missing: {}", service_name)),
    }
}

/// Native SCM audit — `EnumServicesStatusExW` + `QueryServiceConfigW`.
pub fn audit_services_native(raw: &mut Vec<StartupItem>, drivers: bool) {
    let source = if drivers { "System Driver" } else { "Windows Service" };
    unsafe {
        let manager = match OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ENUMERATE_SERVICE) {
            Ok(handle) => handle,
            Err(_) => {
                raw.push(error_item(&format!("Independent {}", source), "Machine", "Service Control Manager", "OpenSCManager failed"));
                return;
            }
        };
        let mut needed = 0u32;
        let mut returned = 0u32;
        let mut resume = 0u32;
        let service_type = if drivers { SERVICE_DRIVER } else { SERVICE_WIN32 };
        let _ = EnumServicesStatusExW(manager, SC_ENUM_TYPE(0), service_type, SERVICE_STATE_ALL, None, &mut needed, &mut returned, Some(&mut resume), PCWSTR::null());
        if needed == 0 || needed > 32 * 1024 * 1024 {
            raw.push(error_item(&format!("Independent {}", source), "Machine", "Service Control Manager", "Service catalog size is invalid"));
            let _ = CloseServiceHandle(manager);
            return;
        }
        let mut buffer = vec![0u8; needed as usize];
        let ok = EnumServicesStatusExW(manager, SC_ENUM_TYPE(0), service_type, SERVICE_STATE_ALL,
            Some(buffer.as_mut_slice()), &mut needed, &mut returned, Some(&mut resume), PCWSTR::null());
        if ok.is_err() {
            raw.push(error_item(&format!("Independent {}", source), "Machine", "Service Control Manager", "EnumServicesStatusEx failed"));
            let _ = CloseServiceHandle(manager);
            return;
        }
        let record_size = std::mem::size_of::<ENUM_SERVICE_STATUS_PROCESSW>();
        for index in 0..returned as usize {
            let record = std::ptr::read(buffer.as_ptr().add(index * record_size) as *const ENUM_SERVICE_STATUS_PROCESSW);
            let name = record.lpServiceName.to_string().unwrap_or_default();
            if name.trim().is_empty() {
                continue;
            }
            let display = record.lpDisplayName.to_string().unwrap_or_else(|_| name.clone());
            let state = record.ServiceStatusProcess.dwCurrentState.0;
            let service = match OpenServiceW(manager, PCWSTR(crate::paths::to_wide(&name).as_ptr()), SERVICE_QUERY_CONFIG) {
                Ok(handle) => handle,
                Err(_) => {
                    raw.push(error_item(&format!("Independent {}", source), "Machine", &name, "OpenService failed"));
                    continue;
                }
            };
            let mut config_needed = 0u32;
            let _ = QueryServiceConfigW(service, None, 0, &mut config_needed);
            if config_needed == 0 || config_needed > 1024 * 1024 {
                raw.push(error_item(&format!("Independent {}", source), "Machine", &name, "QueryServiceConfig sizing failed"));
                let _ = CloseServiceHandle(service);
                continue;
            }
            let mut config_buffer = vec![0u8; config_needed as usize];
            let config = if QueryServiceConfigW(service, Some(config_buffer.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW), config_needed, &mut config_needed).is_ok() {
                std::ptr::read(config_buffer.as_ptr() as *const QUERY_SERVICE_CONFIGW)
            } else {
                raw.push(error_item(&format!("Independent {}", source), "Machine", &name, "QueryServiceConfig failed"));
                let _ = CloseServiceHandle(service);
                continue;
            };
            let binary = if config.lpBinaryPathName.is_null() { String::new() } else { config.lpBinaryPathName.to_string().unwrap_or_default() };
            let start_type = config.dwStartType.0;
            if start_type > 2 {
                if !drivers && start_type != 4 && has_service_trigger_info(&name) {
                    raw.push(StartupItem {
                        id: ids::service_id(&name),
                        name: display,
                        source: "Service (Trigger Start)".into(),
                        scope: "Machine".into(),
                        command: binary,
                        location: format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{}", name),
                        enabled: true,
                        can_disable: is_elevated(),
                        status: format!("SCM start={} trigger-start state={}", start_type, state),
                        ..StartupItem::new()
                    });
                }
                let _ = CloseServiceHandle(service);
                continue;
            }
            raw.push(StartupItem {
                id: if drivers { ids::driver_id(&name) } else { ids::service_id(&name) },
                name: display,
                source: source.into(),
                scope: "Machine".into(),
                command: binary,
                location: format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{}", name),
                enabled: true,
                can_disable: is_elevated(),
                status: format!("SCM start={} state={}", start_type, state),
                ..StartupItem::new()
            });
            let _ = CloseServiceHandle(service);
        }
        let _ = CloseServiceHandle(manager);
    }
}
