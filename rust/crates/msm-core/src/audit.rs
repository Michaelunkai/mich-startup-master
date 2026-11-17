//! Independent boot-coverage audit + tray coverage audit.

use crate::dedupe::norm_boot_key;
use crate::model::StartupItem;
use crate::util::eq_ci;
use once_cell::sync::Lazy;
use regex::Regex;

/// C# `AuditBootCoverage` — `BOOT_AUDIT` receipt via independent enumerators.
pub fn audit_boot_coverage() -> String {
    let shown = crate::scan::scan_all();
    let raw = scan_independent_boot_sources();
    let mut gaps: Vec<String> = Vec::new();
    let audit_errors: Vec<&StartupItem> = raw.iter().filter(|x| x.id.starts_with("error|")).collect();
    for error in &audit_errors {
        gaps.push(format!("AUDIT_ERROR | {} | {} | {}", error.source, error.location, error.command));
    }
    let candidates: Vec<&StartupItem> = raw.iter().filter(|x| !x.id.starts_with("error|") && !boot_source_covered(x, &shown)).collect();
    let refreshed: Vec<StartupItem> = if candidates.is_empty() { shown.clone() } else { crate::scan::scan_all() };
    let mut refreshed_one_time_registry: Option<Vec<StartupItem>> = None;
    for src in candidates {
        if boot_source_covered(src, &refreshed) {
            continue;
        }
        if is_volatile_one_time_boot_source(src) {
            let entry = refreshed_one_time_registry.get_or_insert_with(|| {
                let mut rows = Vec::new();
                crate::scan_registry::audit_registry_sources(&mut rows);
                rows
            });
            if !boot_source_covered(src, entry) {
                continue;
            }
        }
        gaps.push(format!("{} | {} | {}", src.source,
            if src.name.trim().is_empty() { "?" } else { &src.name },
            src.command));
    }
    let mut surfaces: Vec<(String, usize)> = Vec::new();
    for item in &raw {
        let key = if item.source.trim().is_empty() { "?".to_string() } else { item.source.clone() };
        match surfaces.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&key)) {
            Some((_, count)) => *count += 1,
            None => surfaces.push((key, 1)),
        }
    }
    surfaces.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    let surfaces_text = surfaces.iter().map(|(k, c)| format!("{}:{}", k.replace(' ', "_"), c)).collect::<Vec<_>>().join(",");
    let head = format!("BOOT_AUDIT independent=true independence_scope=authoritative-enumerators sources={} shown={} gaps={} errors={} surfaces={}",
        raw.len(), shown.len(), gaps.len(), audit_errors.len(), surfaces_text);
    if gaps.is_empty() {
        head
    } else {
        format!("{}\r\n{}", head, gaps.iter().take(50).cloned().collect::<Vec<_>>().join("\r\n"))
    }
}

/// C# `ScanIndependentBootSources`.
pub fn scan_independent_boot_sources() -> Vec<StartupItem> {
    let mut raw = Vec::new();
    crate::scan_registry::audit_registry_sources(&mut raw);
    crate::scan_folders::audit_startup_folders(&mut raw);
    let mut task_rows = Vec::new();
    crate::tasks::audit_scheduled_task_files(&mut task_rows);
    raw.extend(task_rows);
    let mut service_rows = Vec::new();
    crate::services::audit_services_native(&mut service_rows, false);
    raw.extend(service_rows);
    let mut driver_rows = Vec::new();
    crate::services::audit_services_native(&mut driver_rows, true);
    raw.extend(driver_rows);
    crate::scan_registry::audit_advanced_registry_sources(&mut raw);
    crate::scan_extensions::audit_extended_autorun_sources(&mut raw);
    crate::scan_registry::audit_group_policy_scripts(&mut raw);
    crate::wmi_scan::audit_wmi_consumers(&mut raw);
    crate::packaged::collect_packaged_startup_tasks(&mut raw, true);
    raw
}

fn is_volatile_one_time_boot_source(item: &StartupItem) -> bool {
    item.source == "Registry RunOnce" || item.source == "Registry RunOnceEx"
}

/// C# `BootSourceCovered`.
pub fn boot_source_covered(src: &StartupItem, shown: &[StartupItem]) -> bool {
    let name = norm_boot_key(&src.name);
    let loc = norm_boot_key(&src.location);
    let cmd = norm_boot_key(&src.command);
    for it in shown {
        if !eq_ci(&it.source, &src.source) {
            continue;
        }
        if !src.id.trim().is_empty() && eq_ci(&it.id, &src.id) {
            return true;
        }
        if src.id.trim().is_empty() && !name.is_empty() && !loc.is_empty()
            && norm_boot_key(&it.name) == name && norm_boot_key(&it.location) == loc
            && (cmd.len() <= 8 || norm_boot_key(&it.command) == cmd) {
            return true;
        }
    }
    if src.source == "Startup Command" && shown.iter().any(|native| {
        (native.source == "Registry Run" || native.source == "Startup Folder")
            && !native.id.starts_with("disabled|")
            && crate::dedupe::startup_command_mirrors_native(src, native)
    }) {
        return true;
    }
    if cmd.len() > 8 && src.source == "Startup Command" {
        for it in shown {
            let mut shown_cmd = it.command.clone();
            if let Some((target, args)) = crate::humans::try_decode_tray_payload(&shown_cmd) {
                shown_cmd = format!("{} {}", target, args);
            }
            if norm_boot_key(&shown_cmd) == cmd {
                return true;
            }
        }
    }
    false
}

// ---- Tray coverage audit ----

pub struct QuietAuditRow {
    pub name: String,
    pub location: String,
    pub target: String,
    pub arguments: String,
}

fn is_definite_tray_icon_class(cls: &str) -> bool {
    if cls.is_empty() {
        return false;
    }
    if cls.contains("SystemTrayIcon") { return true; }
    if cls.contains("TrayIcon") { return true; }
    if cls.contains("NotifyIcon") { return true; }
    if cls.contains("_Notification_Class") { return true; }
    // Rust tray-icon crate (Tauri/tao apps such as handy) creates this hidden message window per NotifyIcon.
    if eq_ci(cls, "tray_icon_app") { return true; }
    eq_ci(cls, "AutoHotkey")
}

/// C# `AuditTrayCoverage` — every managed quiet route must expose exactly one usable icon.
pub fn audit_tray_coverage() -> String {
    match audit_tray_coverage_inner() {
        Ok(result) => result,
        Err(e) => format!("TRAY_AUDIT error: {}", e),
    }
}

fn audit_tray_coverage_inner() -> Result<String, String> {
    let rows = quiet_audit_rows();
    let processes = crate::runtime::snapshot_processes(15000);
    let windows = snapshot_top_level_windows();
    let mut findings: Vec<String> = Vec::new();
    let mut uncertain: Vec<String> = Vec::new();
    let mut running = 0usize;
    for row in &rows {
        let mut candidates: Vec<String> = vec![row.target.trim().to_lowercase()];
        candidates.retain(|c| !c.is_empty());
        for payload in launcher_payload_candidates(&row.target) {
            let normalized = payload.trim().to_lowercase();
            if !normalized.is_empty() && !candidates.iter().any(|c| c.eq_ignore_ascii_case(&normalized)) {
                candidates.push(normalized);
            }
        }
        static SCRIPT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
            r#"(?i)["']?([A-Za-z]:[^"'\s]+?\.(?:ps1|pyw|py|vbs|bat|cmd|exe|lnk))["']?"#).unwrap());
        for captures in SCRIPT_RE.captures_iter(&row.arguments) {
            let script = captures[1].to_string();
            if let Some(dir) = script.rsplit_once('\\').map(|(dir, _)| dir) {
                let dir = dir.to_lowercase();
                if !dir.is_empty() && !candidates.contains(&dir) {
                    candidates.push(dir);
                }
            }
        }
        if std::env::var("MSM_STAGE_TIMING").is_ok() {
            eprintln!("[tray] row={} target={:?} args={:?} candidates={}", row.name, row.target, row.arguments, candidates.len());
        }
        if !quiet_route_has_auditable_target(row) || candidates.is_empty() {
            findings.push(format!("MALFORMED_ROUTE {} | no auditable target path", if row.name.trim().is_empty() { "(unnamed quiet route)" } else { &row.name }));
            continue;
        }
        let wrappers: Vec<&crate::runtime::NativeProcessInfo> = processes.iter().filter(|p| {
            if !p.command_line.to_lowercase().contains("--tray-run") || p.command_line.to_lowercase().contains("--agent") {
                return false;
            }
            let Some((wrapper_target, wrapper_args)) = crate::humans::try_decode_tray_payload(&p.command_line) else { return false };
            normalize_managed_command(&wrapper_target, &wrapper_args) == normalize_managed_command(&row.target, &row.arguments)
        }).collect();
        let native: Vec<&crate::runtime::NativeProcessInfo> = processes.iter().filter(|p| {
            !p.command_line.to_lowercase().contains("--tray-run")
                && !p.command_line.to_lowercase().contains("--type=")
                && audit_process_matches_candidates(&p.executable_path, &p.command_line, &candidates)
        }).collect();
        let native_icons: Vec<&&crate::runtime::NativeProcessInfo> = native.iter().filter(|p| windows.iter().any(|w| w.pid == p.process_id && is_definite_tray_icon_class(&w.class))).collect();
        let native_winforms_only: Vec<&&crate::runtime::NativeProcessInfo> = native.iter().filter(|p| {
            !windows.iter().any(|w| w.pid == p.process_id && is_definite_tray_icon_class(&w.class))
                && windows.iter().any(|w| w.pid == p.process_id && w.class.starts_with("WindowsForms10.Window"))
        }).collect();
        let proxy_icons: Vec<&&crate::runtime::NativeProcessInfo> = wrappers.iter().filter(|p| {
            windows.iter().any(|w| w.pid == p.process_id && (w.class.starts_with("WindowsForms10.Window") || is_definite_tray_icon_class(&w.class)))
        }).collect();
        let declared_native_winforms_icon = native_icons.is_empty() && !native_winforms_only.is_empty()
            && crate::quiet_plan::declares_native_tray_capability(&row.target);
        if std::env::var("MSM_STAGE_TIMING").is_ok() {
            eprintln!("[tray] row={} native={} wrappers={}", row.name, native.len(), wrappers.len());
            for p in native.iter().take(3) {
                eprintln!("[tray]   nativeproc exe={:?} cmd={:?}", p.executable_path, p.command_line.get(..120.min(p.command_line.len())).unwrap_or(""));
            }
        }
        let route_running = !native.is_empty() || !wrappers.is_empty();
        if route_running {
            running += 1;
        } else {
            findings.push(format!("MISSING_PROCESS {} | no target or fallback controller process is running", row.name));
        }
        if proxy_icons.len() > 1 {
            findings.push(format!("DUP_PROXY {} | fallback icons={} pids={}", row.name, proxy_icons.len(),
                proxy_icons.iter().map(|p| p.process_id.to_string()).collect::<Vec<_>>().join(",")));
        }
        if native_icons.len() > 1 {
            findings.push(format!("DUP_NATIVE {} | native icons={} pids={}", row.name, native_icons.len(),
                native_icons.iter().map(|p| p.process_id.to_string()).collect::<Vec<_>>().join(",")));
        }
        if (!native_icons.is_empty() || declared_native_winforms_icon) && !proxy_icons.is_empty() {
            findings.push(format!("NATIVE_PLUS_PROXY {} | native and fallback icons coexist", row.name));
        }
        let definite_icons = native_icons.len() + proxy_icons.len() + if declared_native_winforms_icon { 1 } else { 0 };
        if definite_icons == 0 {
            if !native_winforms_only.is_empty() {
                uncertain.push(format!("UNCERTAIN {} | WinForms windows cannot prove a native NotifyIcon", row.name));
            } else if route_running {
                findings.push(format!("MISSING_ICON {} | route is running but no tray icon host was found", row.name));
            }
        }
    }
    let head = format!("TRAY_AUDIT apps={} running={} findings={} uncertain={}", rows.len(), running, findings.len(), uncertain.len());
    let mut details = findings;
    details.extend(uncertain);
    Ok(if details.is_empty() { head } else { format!("{}\r\n{}", head, details.join("\r\n")) })
}

fn quiet_route_has_auditable_target(row: &QuietAuditRow) -> bool {
    if !row.target.trim().is_empty() {
        return true;
    }
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)[A-Za-z]:[^\\\"'\s]+?\.(?:ps1|pyw|py|vbs|bat|cmd|exe|lnk)"#).unwrap());
    RE.is_match(&row.arguments)
}

/// C# `LauncherPayloadCandidates` — FooLauncher → CustomRuntime\\Runtime\\sibling payloads.
pub fn launcher_payload_candidates(target_path: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let target = crate::util::expand_env(target_path.trim().trim_matches('"'));
    if target.is_empty() || !target.to_lowercase().ends_with(".exe") {
        return candidates;
    }
    let full = crate::paths::canonical(&target);
    let leaf = std::path::Path::new(&full).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if !leaf.to_lowercase().ends_with("launcher") {
        return candidates;
    }
    let payload_stem = leaf[..leaf.len() - "Launcher".len()].trim_end_matches([' ', '.', '_', '-']).to_string();
    let directory = full.rsplit_once('\\').map(|(dir, _)| dir).unwrap_or("");
    if payload_stem.is_empty() || directory.is_empty() {
        return candidates;
    }
    for payload_directory in [format!("{}\\CustomRuntime", directory), format!("{}\\Runtime", directory), directory.to_string()] {
        let candidate = format!("{}\\{}.exe", payload_directory, payload_stem);
        if !candidates.iter().any(|c| c.eq_ignore_ascii_case(&candidate)) {
            candidates.push(candidate);
        }
    }
    candidates
}

pub fn audit_process_matches_candidates(executable_path: &str, command_line: &str, candidates: &[String]) -> bool {
    let image = executable_path.trim().trim_matches('"');
    candidates.iter().any(|candidate| {
        let expected = candidate.trim().trim_matches('"');
        !expected.is_empty() && (image.eq_ignore_ascii_case(expected) || command_line.to_lowercase().contains(&expected.to_lowercase()))
    })
}

pub fn normalize_managed_command(target: &str, args: &str) -> String {
    let mut path = crate::util::expand_env(target.trim().trim_matches('"')).replace('/', "\\");
    if std::path::Path::new(&path).is_absolute() {
        path = crate::paths::canonical(&path);
    }
    let path = path.trim_end_matches('\\').to_lowercase();
    static WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    let arguments = WS.replace_all(args, " ").trim().to_string();
    if arguments.is_empty() { path } else { format!("{} {}", path, arguments.to_lowercase()) }
}

/// Quiet rows: persisted manifest ∪ live managed quiet tasks.
pub fn quiet_audit_rows() -> Vec<QuietAuditRow> {
    use std::collections::BTreeMap;
    let mut by_task: BTreeMap<String, QuietAuditRow> = BTreeMap::new();
    let mut malformed = 0usize;
    for row in (crate::hooks::hooks().load_enabled_intent)() {
        if !eq_ci(&row.mode, "tray") {
            continue;
        }
        let task_location = if row.task_location.trim().is_empty() { row.location.clone() } else { row.task_location.clone() };
        let key = if task_location.trim().is_empty() { format!("#malformed-store-{}", { malformed += 1; malformed }) } else { task_location };
        by_task.insert(key.clone(), QuietAuditRow { name: row.task_name(), location: key, target: row.target.clone(), arguments: row.arguments.clone() });
    }
    for item in crate::scan::scan_all().into_iter().filter(|i| i.enabled && i.is_managed && i.source == "Scheduled Task" && crate::quiet_plan::is_quiet_launch(&i.command, &i.location)) {
        if item.location.trim().is_empty() {
            continue;
        }
        let (target, arguments) = resolve_launch_target_or_split(&item);
        by_task.insert(item.location.clone(), QuietAuditRow {
            name: if item.name.trim().is_empty() { item.location.trim_start_matches('\\').to_string() } else { item.name.clone() },
            location: item.location.clone(),
            target, arguments,
        });
    }
    by_task.into_values().collect()
}

fn resolve_launch_target_or_split(item: &StartupItem) -> (String, String) {
    // ResolveLaunchTarget raises when the file is gone; fall back to a plain split.
    if let Some((target, args)) = crate::humans::try_decode_tray_payload(&item.command) {
        if std::path::Path::new(&target).is_file() {
            return (target, args);
        }
    }
    match crate::humans::try_split_command(&item.command) {
        Some(pair) => pair,
        None => (String::new(), String::new()),
    }
}

#[derive(Clone)]
pub struct WinInfo {
    pub pid: u32,
    pub class: String,
}

pub fn snapshot_top_level_windows() -> Vec<WinInfo> {
    use std::sync::Mutex;
    static COLLECTED: Mutex<Vec<WinInfo>> = Mutex::new(Vec::new());
    COLLECTED.lock().unwrap().clear();
    unsafe {
        use windows::core::BOOL;
        use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowThreadProcessId};
        unsafe extern "system" fn callback(window: windows::Win32::Foundation::HWND, lparam: windows::Win32::Foundation::LPARAM) -> BOOL {
            let collector = &*(lparam.0 as *const Mutex<Vec<WinInfo>>);
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            let mut buffer = [0u16; 256];
            GetClassNameW(window, &mut buffer);
            if let Ok(mut guard) = collector.lock() {
                guard.push(WinInfo { pid, class: crate::paths::wide_to_string(&buffer) });
            }
            BOOL(1)
        }
        let lparam = windows::Win32::Foundation::LPARAM(&COLLECTED as *const _ as isize);
        let _ = EnumWindows(Some(callback), lparam);
    }
    COLLECTED.lock().unwrap().clone()
}
