//! Native process catalog (bounded, never blocks on wedged volumes) + runtime evidence.

use crate::model::StartupItem;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

#[derive(Debug, Clone, Default)]
pub struct NativeProcessInfo {
    pub process_id: u32,
    pub parent_process_id: u32,
    pub executable_path: String,
    pub command_line: String,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
pub struct ProcessInfoClass(pub i32);

pub const PROCESS_BASIC_INFO: ProcessInfoClass = ProcessInfoClass(0);
pub const PROCESS_COMMAND_LINE_INFO: ProcessInfoClass = ProcessInfoClass(60);

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(process: windows::Win32::Foundation::HANDLE, info_class: ProcessInfoClass, info: *mut core::ffi::c_void, info_length: u32, return_length: *mut u32) -> i32;
}

#[repr(C)]
struct ProcessBasicInformation {
    reserved1: usize,
    peb_base_address: usize,
    reserved2_0: usize,
    reserved2_1: usize,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

fn device_map() -> Vec<(String, String)> {
    static CACHE: once_cell::sync::Lazy<std::sync::Mutex<Option<(u128, Vec<(String, String)>)>>> =
        once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));
    let mut guard = CACHE.lock().unwrap();
    let now = crate::util::tick_ms();
    if let Some((stamp, map)) = guard.as_ref() {
        if now.saturating_sub(*stamp) < 5000 {
            return map.clone();
        }
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalDrives() -> u32;
        fn QueryDosDeviceW(device_name: windows::core::PCWSTR, target_path: windows::core::PWSTR, max_chars: u32) -> u32;
    }
    let mut map = Vec::new();
    unsafe {
        let drives = GetLogicalDrives();
        for i in 0..26u32 {
            if drives & (1 << i) == 0 {
                continue;
            }
            let letter = format!("{}:", (b'A' + i as u8) as char);
            let wide_letter = crate::paths::to_wide(&letter);
            let mut target = vec![0u16; 1024];
            if QueryDosDeviceW(windows::core::PCWSTR(wide_letter.as_ptr()), windows::core::PWSTR(target.as_mut_ptr()), 1024) == 0 {
                continue;
            }
            let device = crate::paths::wide_to_string(&target);
            if device.starts_with("\\Device\\") {
                map.push((letter, device.trim_end_matches('\\').to_string()));
            }
        }
    }
    *guard = Some((now, map.clone()));
    map
}

/// C# `NativeProcessCatalog::NativeToDosPath`.
pub fn native_to_dos_path(native: &str, map: &[(String, String)]) -> String {
    let mut value = native.trim().to_string();
    if value.is_empty() {
        return String::new();
    }
    if let Some(stripped) = value.strip_prefix("\\??\\") {
        value = stripped.to_string();
    }
    if value.to_uppercase().starts_with("UNC\\") {
        return format!("\\\\{}", &value[4..]);
    }
    if value.len() >= 3 && value.as_bytes()[1] == b':' && value.as_bytes()[2] == b'\\' {
        return value;
    }
    let mut best: Option<(&str, &str)> = None;
    for (letter, device) in map {
        let device = device.trim_end_matches('\\');
        if device.is_empty() || value.len() <= device.len() {
            continue;
        }
        if !value[..device.len()].eq_ignore_ascii_case(device) || value.as_bytes()[device.len()] != b'\\' {
            continue;
        }
        if best.map(|(b, _)| device.len() > b.len()).unwrap_or(true) {
            best = Some((device, letter));
        }
    }
    if let Some((device, letter)) = best {
        return format!("{}{}", letter, &value[device.len()..]);
    }
    for redirector in [r"\Device\Mup\", r"\Device\LanmanRedirector"] {
        if !value.to_lowercase().starts_with(&redirector.to_lowercase()) {
            continue;
        }
        let mut rest = value[redirector.len()..].to_string();
        while rest.starts_with(';') {
            match rest.find('\\') {
                Some(slash) => rest = rest[slash + 1..].to_string(),
                None => return String::new(),
            }
        }
        if rest.is_empty() {
            return String::new();
        }
        return format!("\\\\{}", rest);
    }
    String::new()
}

/// C# `NativeProcessCatalog::Snapshot` — bounded full-system catalog.
pub fn snapshot_processes(maximum_milliseconds: u128) -> Vec<NativeProcessInfo> {
    let ids = process_ids();
    let deadline = crate::util::tick_ms() + maximum_milliseconds.max(50);
    let map = device_map();
    ids.into_iter().map(|pid| {
        if crate::util::tick_ms() > deadline {
            return NativeProcessInfo { process_id: pid, parent_process_id: 0, executable_path: String::new(), command_line: String::new() };
        }
        read_process(pid, &map)
    }).collect()
}

fn process_ids() -> Vec<u32> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok();
        let Some(snapshot) = snapshot else { return Vec::new() };
        let mut ids = Vec::new();
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                ids.push(entry.th32ProcessID);
                entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        ids
    }
}

/// C# `NativeProcessCatalog::ParentSnapshot` — cheap PID/PPID pairs.
pub fn parent_snapshot() -> Vec<NativeProcessInfo> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok();
        let Some(snapshot) = snapshot else { return Vec::new() };
        let mut rows = Vec::new();
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                rows.push(NativeProcessInfo { process_id: entry.th32ProcessID, parent_process_id: entry.th32ParentProcessID, ..Default::default() });
                entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        rows
    }
}

fn read_process(pid: u32, map: &[(String, String)]) -> NativeProcessInfo {
    let mut row = NativeProcessInfo { process_id: pid, ..Default::default() };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return row };
        row.executable_path = read_image_path(handle, map);
        row.command_line = read_command_line(handle);
        row.parent_process_id = read_parent_pid(handle);
        let _ = CloseHandle(handle);
    }
    row
}

unsafe fn read_image_path(handle: windows::Win32::Foundation::HANDLE, map: &[(String, String)]) -> String {
    #[link(name = "kernel32")]
    extern "system" {
        fn QueryFullProcessImageNameW(process: windows::Win32::Foundation::HANDLE, flags: u32, exe_name: windows::core::PWSTR, size: *mut u32) -> i32;
    }
    let mut buffer = vec![0u16; 32768];
    let mut size = buffer.len() as u32;
    if QueryFullProcessImageNameW(handle, 1, windows::core::PWSTR(buffer.as_mut_ptr()), &mut size) != 0 {
        let native = crate::paths::wide_to_string(&buffer[..size as usize]);
        return native_to_dos_path(&native, map);
    }
    String::new()
}

unsafe fn read_command_line(handle: windows::Win32::Foundation::HANDLE) -> String {
    // ProcessCommandLineInformation copies the string into the caller buffer
    // right after the UNICODE_STRING header; allocate generously and validate
    // against the buffer, not against returnLength (which counts only bytes
    // strictly required and under-reports the copied payload).
    const MAXIMUM_COMMAND_LINE_BYTES: usize = 1024 * 1024;
    let mut buffer = vec![0u8; MAXIMUM_COMMAND_LINE_BYTES];
    let mut returned = 0u32;
    if NtQueryInformationProcess(handle, PROCESS_COMMAND_LINE_INFO, buffer.as_mut_ptr() as *mut _, buffer.len() as u32, &mut returned) != 0 {
        return String::new();
    }
    if returned as usize > buffer.len() {
        return String::new();
    }
    let value = std::ptr::read(buffer.as_ptr() as *const UnicodeString);
    if value.buffer.is_null() || value.length == 0 || value.length > value.maximum_length {
        return String::new();
    }
    // The string may live inline (after the header) or be pointed at anywhere in-buffer.
    let start = value.buffer as usize - buffer.as_ptr() as usize;
    if value.length as usize > buffer.len() || start + value.length as usize > buffer.len() {
        return String::new();
    }
    let bytes = &buffer[start..start + value.length as usize];
    String::from_utf16_lossy(std::slice::from_raw_parts(bytes.as_ptr() as *const u16, bytes.len() / 2))
}

unsafe fn read_parent_pid(handle: windows::Win32::Foundation::HANDLE) -> u32 {
    let mut info = ProcessBasicInformation { reserved1: 0, peb_base_address: 0, reserved2_0: 0, reserved2_1: 0, unique_process_id: 0, inherited_from_unique_process_id: 0 };
    let mut returned = 0u32;
    if NtQueryInformationProcess(handle, PROCESS_BASIC_INFO, &mut info as *mut _ as *mut _, std::mem::size_of::<ProcessBasicInformation>() as u32, &mut returned) != 0 {
        return 0;
    }
    let parent = info.inherited_from_unique_process_id as u64;
    if parent > 0 && parent <= u32::MAX as u64 { parent as u32 } else { 0 }
}

/// C# `ApplyRuntimeEvidence` — exact/related running-process evidence.
pub fn apply_runtime_evidence(items: &mut [StartupItem]) {
    let processes = snapshot_processes(1500);
    let process_paths: Vec<(u32, String)> = processes.iter()
        .map(|p| (p.process_id, crate::paths::normalize_observed_path(&p.executable_path)))
        .filter(|(_, path)| !path.is_empty())
        .collect();
    for item in items.iter_mut() {
        let mut target = String::new();
        let mut arguments = String::new();
        if let Some((t, a)) = try_friendly_target_static(item) {
            target = t;
            arguments = a;
        } else if let Some((t, a)) = crate::humans::try_split_command(&item.command) {
            target = t;
            arguments = a;
        }
        let mut payloads: Vec<String> = item.application_targets.iter().filter(|p| !p.trim().is_empty()).cloned().collect();
        if payloads.is_empty() {
            payloads = crate::humans::application_payload_candidates(item);
        }
        let observed_payload = select_observed_application_target(&payloads, process_paths.iter().map(|(_, p)| p));
        if !observed_payload.is_empty() {
            target = observed_payload.clone();
            if !crate::humans::is_script_application_identity(&item.application_identity)
                && !crate::humans::is_windows_app_package_identity(&item.application_identity)
                && !item.application_identity.starts_with("installed-product:") {
                let reason = "Live exact-path correlation selected the active executable behind this startup launcher".to_string();
                set_portable_identity(item, &observed_payload, reason);
                item.app_name = crate::humans::friendly_name_for_payload(item, &observed_payload);
            }
        }
        let exact_path = crate::paths::normalize_observed_path(&target);
        let exact: Vec<&(u32, String)> = if exact_path.is_empty() { Vec::new() } else {
            process_paths.iter().filter(|(_, path)| path.eq_ignore_ascii_case(&exact_path)).collect()
        };
        let root = crate::paths::normalize_observed_path(&item.application_root);
        let related: Vec<&(u32, String)> = if root.is_empty() { Vec::new() } else {
            process_paths.iter().filter(|(_, path)| path.eq_ignore_ascii_case(&root) || path.to_lowercase().starts_with(&format!("{}\\", root.to_lowercase()))).collect()
        };
        if exact.is_empty() && related.is_empty() {
            continue;
        }
        let exact_text = if exact.is_empty() {
            format!("exact target not currently observed{}", if exact_path.is_empty() { String::new() } else { format!(" [{}]", exact_path) })
        } else {
            format!("exact target running pid={}", exact.iter().map(|(pid, _)| pid.to_string()).collect::<Vec<_>>().join(","))
        };
        let related_text = if related.is_empty() { String::new() } else {
            let mut sorted: Vec<&(u32, String)> = related.clone();
            sorted.sort_by_key(|(pid, _)| *pid);
            format!("; related installed-product processes={}", sorted.iter().take(16).map(|(pid, path)| format!("{}({})", path.rsplit('\\').next().unwrap_or(""), pid)).collect::<Vec<_>>().join(", "))
        };
        item.runtime_evidence = format!("{}. This is a current-process observation, not proof of the next boot.", format!("{}{}", exact_text, related_text));
    }
}

fn set_portable_identity(item: &mut StartupItem, target: &str, reason: String) {
    let normalized = crate::paths::normalize_observed_path(target);
    if normalized.is_empty() {
        return;
    }
    item.application_identity = format!("path:{}", normalized.to_lowercase());
    item.application_root = crate::paths::normalize_observed_path(normalized.rsplit_once('\\').map(|(dir, _)| dir).unwrap_or(""));
    item.application_identity_reason = format!("{}: {}", reason, normalized);
}

fn try_friendly_target_static(item: &StartupItem) -> Option<(String, String)> {
    if item.source == "Startup Folder" && item.command.to_lowercase().ends_with(".lnk") {
        let (target, args) = crate::humans::resolve_shortcut(&item.command);
        if !target.trim().is_empty() {
            return Some((target, args));
        }
    }
    if let Some(pair) = crate::humans::try_decode_tray_payload(&item.command) {
        return Some(pair);
    }
    let payloads = crate::humans::application_payload_candidates(item);
    payloads.first().map(|first| (first.clone(), String::new()))
}

pub fn select_observed_application_target<'a>(candidates: &[String], observed_paths: impl Iterator<Item = &'a String>) -> String {
    use std::collections::HashSet;
    let observed: HashSet<String> = observed_paths.map(|p: &String| crate::paths::normalize_observed_path(p)).filter(|p: &String| !p.is_empty()).map(|p: String| p.to_lowercase()).collect();
    for candidate in candidates {
        let normalized = crate::paths::normalize_observed_path(candidate).to_lowercase();
        if !normalized.is_empty() && observed.contains(&normalized) {
            return normalized;
        }
    }
    String::new()
}

/// C# `EnabledStartupService::IsProcessRunning`.
pub fn is_process_running(target: &str) -> bool {
    if target.trim().is_empty() {
        return false;
    }
    let expected = normalize_process_path(target);
    let extension = std::path::Path::new(&expected).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let script = ["ps1", "cmd", "bat", "vbs", "py", "pyw"].contains(&extension.as_str());
    if extension == "lnk" {
        let (resolved, _) = crate::humans::resolve_shortcut(target);
        if resolved.trim().is_empty() {
            return false;
        }
        return is_process_running(&resolved);
    }
    for process in snapshot_processes(800) {
        if !script {
            if !process.executable_path.trim().is_empty() && normalize_process_path(&process.executable_path).eq_ignore_ascii_case(&expected) {
                return true;
            }
        } else if command_line_owns_path(&process.command_line, &expected) {
            return true;
        }
    }
    false
}

fn normalize_process_path(value: &str) -> String {
    let expanded = crate::util::expand_env(value.trim().trim_matches('"'));
    match crate::paths::canonical(&expanded) {
        full if std::path::Path::new(&expanded).is_absolute() => full.trim_end_matches(['\\', '/']).replace('/', "\\"),
        _ => expanded.trim().trim_matches('"').trim_end_matches(['\\', '/']).replace('/', "\\"),
    }
}

/// C# `CommandLineOwnsPath`.
pub fn command_line_owns_path(command_line: &str, expected_path: &str) -> bool {
    let command = command_line.replace('/', "\\");
    let expected = expected_path.replace('/', "\\");
    if expected.is_empty() {
        return false;
    }
    let mut offset = 0;
    while offset + expected.len() <= command.len() {
        let haystack = &command[offset..];
        let hit = match haystack.to_lowercase().find(&expected.to_lowercase()) {
            Some(hit) => offset + hit,
            None => return false,
        };
        let end = hit + expected.len();
        let left_ok = hit == 0 || command[..hit].chars().last().map(|c| c.is_whitespace() || c == '"' || c == '\'').unwrap_or(false);
        let right_ok = end == command.len() || command[end..].chars().next().map(|c| c.is_whitespace() || c == '"' || c == '\'').unwrap_or(false);
        if left_ok && right_ok {
            return true;
        }
        offset = hit + 1;
    }
    false
}
