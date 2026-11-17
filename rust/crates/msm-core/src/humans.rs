//! Human names and application identity: payload candidates, script entry points,
//! installed products, Windows app packages, friendly names.

use crate::model::StartupItem;
use crate::reg::*;
use crate::util::{b64, eq_ci, expand_env};
use once_cell::sync::Lazy;
use regex::Regex;

// ---- command splitting / tray payload ----

/// C# `TryDecodeTrayPayload` — decode `--tray-run|--shell-open <base64>`.
pub fn try_decode_tray_payload(command: &str) -> Option<(String, String)> {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)--(?:tray-run|shell-open)\s+(?<payload>[A-Za-z0-9+/=]+)").unwrap());
    let captures = RE.captures(command)?;
    let payload = captures.name("payload")?;
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD.decode(payload.as_str()).ok()?;
    let text = String::from_utf8_lossy(&decoded).into_owned();
    let mut lines = text.splitn(2, '\n');
    let target = lines.next().unwrap_or("").to_string();
    let arguments = lines.next().unwrap_or("").to_string();
    Some((target, arguments))
}

/// C# `TrySplitCommand`.
pub fn try_split_command(command: &str) -> Option<(String, String)> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    if command.starts_with('"') {
        if let Some(end) = command[1..].find('"') {
            if end > 0 {
                let exe = command[1..1 + end].to_string();
                let args = command[end + 2..].trim().to_string();
                return Some((exe, args));
            }
        }
    }
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(
        r"(?i)^(?<exe>.+?\.(?:exe|com|lnk|ps1|cmd|bat|vbs|vbe|js|jse|wsf|wsh|py|pyw|msc))(?:\s+(?<args>.*))?$").unwrap());
    if let Some(captures) = RE.captures(command) {
        let exe = captures.name("exe").map(|m| m.as_str().trim().to_string()).unwrap_or_default();
        let args = captures.name("args").map(|m| m.as_str().trim().to_string()).unwrap_or_default();
        return Some((exe, args));
    }
    if std::path::Path::new(command).is_file() {
        return Some((command.to_string(), String::new()));
    }
    None
}

/// Resolve a .lnk shortcut to (target, arguments) via IShellLinkW.
pub fn resolve_shortcut(shortcut_path: &str) -> (String, String) {
    unsafe {
        use windows::core::{Interface, PCWSTR};
        use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, IPersistFile, STGM_READ};
        use windows::Win32::UI::Shell::{IShellLinkW, SLGP_UNCPRIORITY};
        // {00021401-0000-0000-C000-000000000046}
        const CLSID_SHELL_LINK: windows::core::GUID = windows::core::GUID::from_u128(0x00021401_0000_0000_C000_000000000046);
        let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER) else {
            return (shortcut_path.to_string(), String::new());
        };
        let Ok(persist) = link.cast::<IPersistFile>() else {
            return (shortcut_path.to_string(), String::new());
        };
        let wide = crate::paths::to_wide(shortcut_path);
        if persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).is_err() {
            return (shortcut_path.to_string(), String::new());
        }
        let mut target_buf = [0u16; 1024];
        let mut data = WIN32_FIND_DATAW::default();
        if link.GetPath(&mut target_buf, &mut data, SLGP_UNCPRIORITY.0 as u32).is_err() {
            return (shortcut_path.to_string(), String::new());
        }
        let target = crate::paths::wide_to_string(&target_buf);
        let mut args_buf = [0u16; 1024];
        let args = if link.GetArguments(&mut args_buf).is_ok() {
            crate::paths::wide_to_string(&args_buf)
        } else {
            String::new()
        };
        if target.trim().is_empty() {
            (shortcut_path.to_string(), args)
        } else {
            (target, args)
        }
    }
}

// ---- script / payload classification ----

pub fn is_script_path(value: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\.(ps1|cmd|bat|vbs|vbe|js|jse|wsf|wsh|py|pyw)$").unwrap());
    let extension = std::path::Path::new(value.trim().trim_matches('"')).extension()
        .map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    RE.is_match(&extension)
}

pub fn is_application_executable_path(value: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\.(exe|com|lnk|appref-ms)$").unwrap());
    let extension = std::path::Path::new(value.trim().trim_matches('"')).extension()
        .map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    RE.is_match(&extension)
}

pub fn is_generic_application_launcher(value: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(wscript|cscript|powershell|pwsh|cmd|python|pythonw|rundll32|mshta)(\.exe)?$").unwrap());
    let file = value.trim().trim_matches('"').rsplit('\\').next().unwrap_or("");
    RE.is_match(file) || is_script_path(value)
}

/// C# `IsGenericHostExecutable`.
pub fn is_generic_host_executable(value: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(
        r"(?i)^((svchost|lsass|services|wininit|winlogon|csrss|smss|dllhost|rundll32|regsvr32|conhost|explorer|sihost|taskhostw|taskeng|mmc|msiexec|cmd|powershell|pwsh|wscript|cscript|mshta|node|python|pythonw|py|pyw|java|javaw|wsl|bash)(\.exe)?|mscoree\.dll)$").unwrap());
    let expanded = expand_env(value);
    let file = expanded.rsplit('\\').next().unwrap_or("");
    RE.is_match(file)
}

/// C# `ExtractLiteralPaths` — quoted and bare path tokens (scanner form: the
/// Rust regex engine has no lookaround/backrefs, so this walks the text once).
pub fn extract_literal_paths(value: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    let chars: Vec<char> = value.chars().collect();
    let is_path_char = |c: char| !(c == '"' || c == '\'' || c == '\r' || c == '\n' || c == '\t');
    let mut i = 0usize;
    while i < chars.len() {
        // Quoted form: "X:\...\.exe" or 'X:\...\.exe' (also %VAR%\ roots).
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            if let Some(path) = scan_path_token(&chars[i + 1..], quote, true) {
                let length = path.chars().count();
                push_unique(&mut result, path);
                i += 1 + length + 1;
                continue;
            }
        }
        // Bare form: X:\path.exe preceded by a non-[A-Za-z0-9_] boundary.
        let boundary_ok = i == 0 || !chars[i - 1].is_ascii_alphanumeric() && chars[i - 1] != '_';
        if boundary_ok {
            if let Some(path) = scan_path_token(&chars[i..], '\0', false) {
                let length = path.chars().count();
                let end = i + length;
                let terminator_ok = end == chars.len() || chars[end].is_whitespace();
                if terminator_ok {
                    push_unique(&mut result, path.trim().to_string());
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    result
}

fn is_supported_path_leaf(name: &str) -> bool {
    const EXTENSIONS: [&str; 15] = ["exe", "com", "lnk", "appref-ms", "ps1", "cmd", "bat", "vbs", "vbe", "js", "jse", "wsf", "wsh", "py", "pyw"];
    let lower = name.to_lowercase();
    EXTENSIONS.iter().any(|extension| lower.ends_with(&format!(".{}", extension)))
}

/// Scan one path token from the start of `chars` up to `stop` (quote) or the
/// first whitespace for the bare form. Returns the token when it ends in a
/// supported extension and starts with a drive/env root.
fn scan_path_token(chars: &[char], stop: char, quoted: bool) -> Option<String> {
    if chars.is_empty() { return None }
    let rooted = (chars[0].is_ascii_alphabetic() && chars.get(1) == Some(&':'))
        || chars[0] == '%';
    if !rooted { return None }
    let mut token = String::new();
    for &c in chars {
        if quoted && c == stop { break }
        if !quoted && c.is_whitespace() { break }
        if !quoted && (c == '"' || c == '\'') { return None }
        token.push(c);
        if token.len() > 2048 { return None }
    }
    if quoted && token.chars().count() == chars.len() { return None } // unterminated
    let leaf = token.rsplit('.').next().unwrap_or("");
    if !is_supported_path_leaf(leaf) || token.len() < 4 { return None }
    Some(token)
}

fn push_unique(result: &mut Vec<String>, path: String) {
    if !result.iter().any(|existing| existing.eq_ignore_ascii_case(&path)) {
        result.push(path);
    }
}

fn add_application_payload(payloads: &mut Vec<String>, value: &str) {
    if payloads.len() >= 32 || !is_application_executable_path(value) {
        return;
    }
    let normalized = crate::paths::normalize_observed_path(&expand_env(value.trim().trim_matches(['"', '\'', ' ', '\t'])));
    if normalized.is_empty() || payloads.iter().any(|p| eq_ci(p, &normalized)) {
        return;
    }
    payloads.push(normalized);
}

fn add_script_payload_candidates(script_path: &str, payloads: &mut Vec<String>, visited: &mut Vec<String>, depth: usize) {
    if depth > 3 || payloads.len() >= 32 {
        return;
    }
    let script = crate::paths::normalize_observed_path(&expand_env(script_path));
    if script.is_empty() || visited.iter().any(|v| eq_ci(v, &script)) || !crate::paths::is_safe_system_volume_file_probe(&script) {
        return;
    }
    visited.push(script.clone());
    let Ok(source) = std::fs::read_to_string(&script) else { return };
    if source.len() > 1024 * 1024 {
        return;
    }
    let literals = extract_literal_paths(&source);
    for candidate in &literals {
        add_application_payload(payloads, candidate);
    }
    for nested in literals.iter().filter(|l| is_script_path(l)) {
        add_script_payload_candidates(nested, payloads, visited, depth + 1);
    }
}

/// C# `ApplicationPayloadCandidates`.
pub fn application_payload_candidates(item: &StartupItem) -> Vec<String> {
    // Rendering asks this per row per pass; memoize by (source, command).
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);
    let cache_key = format!("{}{}", item.source, item.command);
    {
        let guard = CACHE.lock().unwrap();
        if let Some(map) = guard.as_ref() {
            if let Some(hit) = map.get(&cache_key) {
                return hit.clone();
            }
        }
    }
    let computed = application_payload_candidates_uncached(item);
    CACHE.lock().unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(cache_key, computed.clone());
    computed
}

fn application_payload_candidates_uncached(item: &StartupItem) -> Vec<String> {
    let mut payloads: Vec<String> = Vec::new();
    let mut target = String::new();
    let mut arguments = String::new();
    if item.source == "Startup Folder" && item.command.to_lowercase().ends_with(".lnk") {
        let (t, a) = resolve_shortcut(&item.command);
        target = t;
        arguments = a;
    }
    if target.trim().is_empty() {
        if let Some((t, a)) = try_decode_tray_payload(&item.command) {
            target = t;
            arguments = a;
        }
    }
    if target.trim().is_empty() {
        if let Some((t, a)) = try_split_command(&expand_env(&item.command)) {
            target = t;
            arguments = a;
        }
    }
    if !target.trim().is_empty() && !is_generic_application_launcher(&target) {
        add_application_payload(&mut payloads, &target);
    }
    for candidate in extract_literal_paths(&arguments) {
        add_application_payload(&mut payloads, &candidate);
    }
    let mut scripts = Vec::new();
    if is_script_path(&target) {
        scripts.push(target.clone());
    }
    scripts.extend(extract_literal_paths(&arguments).into_iter().filter(|p| is_script_path(p)));
    let mut visited = Vec::new();
    for script in scripts {
        add_script_payload_candidates(&script, &mut payloads, &mut visited, 0);
    }
    payloads.truncate(32);
    payloads
}

// ---- script identity ----

pub fn is_script_application_identity(identity: &str) -> bool {
    identity.starts_with("script:") || identity.starts_with("script-project:")
}

pub fn is_windows_app_package_identity(identity: &str) -> bool {
    identity.starts_with("windows-app-package:")
}

/// C# `TryGetStartupScriptPath`.
pub fn try_get_startup_script_path(item: &StartupItem) -> Option<String> {
    let mut target = String::new();
    let mut arguments = String::new();
    if item.source == "Startup Folder" && item.command.to_lowercase().ends_with(".lnk") {
        let (t, a) = resolve_shortcut(&item.command);
        target = t;
        arguments = a;
    }
    if target.trim().is_empty() {
        match try_decode_tray_payload(&item.command) {
            Some((t, a)) => { target = t; arguments = a; }
            None => {
                if let Some((t, a)) = try_split_command(&expand_env(&item.command)) {
                    target = t;
                    arguments = a;
                }
            }
        }
    }
    let mut candidates = Vec::new();
    if is_script_path(&target) {
        candidates.push(target.clone());
    } else if is_generic_application_launcher(&target) {
        candidates.extend(extract_literal_paths(&arguments).into_iter().filter(|p| is_script_path(p)));
    }
    for candidate in candidates {
        let normalized = crate::paths::normalize_observed_path(&expand_env(&candidate));
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }
    None
}

/// C# `IsTransparentApplicationLauncherScript`.
pub fn is_transparent_application_launcher_script(item: &StartupItem, script: &str, payloads: &[String]) -> bool {
    let file = script.rsplit('\\').next().unwrap_or("");
    if eq_ci(file, "openspeedy-silent.vbs") {
        return true;
    }
    let mut target = String::new();
    let mut arguments = String::new();
    if let Some((t, a)) = try_decode_tray_payload(&item.command) {
        target = t;
        arguments = a;
    }
    if target.trim().is_empty() {
        if let Some((t, a)) = try_split_command(&expand_env(&item.command)) {
            target = t;
            arguments = a;
        }
    }
    if !is_generic_application_launcher(&target) {
        return false;
    }
    let literal_arguments = extract_literal_paths(&arguments);
    let script_index = literal_arguments.iter().position(|p| is_script_path(p));
    match script_index {
        Some(index) => literal_arguments[index + 1..].iter().any(|p| is_application_executable_path(p)),
        None => false,
    }
}

fn try_get_codex_script_project(script: &str) -> Option<(String, String)> {
    let codex_root = crate::paths::normalize_observed_path(&format!("{}\\Codex", crate::paths::local_app_data().trim_end_matches('\\')));
    let normalized = crate::paths::normalize_observed_path(script);
    if codex_root.is_empty() || normalized.is_empty() || !normalized.to_lowercase().starts_with(&format!("{}\\", codex_root.to_lowercase())) {
        return None;
    }
    let relative = &normalized[codex_root.len() + 1..];
    let first = relative.split(['\\', '/']).filter(|p| !p.is_empty()).next()?;
    Some((format!("{}\\{}", codex_root, first), first.to_string()))
}

// ---- clean names ----

/// C# `CleanName`.
pub fn clean_name(value: &str) -> String {
    let mut s = value.trim().to_string();
    if s.starts_with('@') {
        s = s[1..].to_string();
    }
    if let Some(comma) = s.find(',') {
        if s.contains('\\') || s.to_lowercase().ends_with(".dll") || s.to_lowercase().ends_with(".exe") {
            let head = &s[..comma];
            s = head.rsplit('\\').next().unwrap_or(head)
                .rsplit('.').next().unwrap_or(head).to_string();
        }
    }
    static GUID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{[0-9A-Fa-f-]{20,}\}").unwrap());
    static TAIL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[_-]?[0-9a-fA-F]{8,}$").unwrap());
    s = GUID_RE.replace_all(&s, " ").into_owned();
    s = TAIL_RE.replace_all(&s, " ").into_owned();
    s = s.replace('_', " ").replace('-', " ");
    static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    s = WS_RE.replace_all(&s, " ").trim().to_string();
    s
}

pub fn safe_file_stem(value: &str) -> String {
    std::path::Path::new(value).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| value.to_string())
}


/// Strip a trailing machine hex id (>=8 hex chars containing a digit, preceded
/// by a separator). Replaces the C# lookaround regex in this crate.
pub fn strip_trailing_hex_id(value: &str) -> String {
    let trimmed = value;
    let chars: Vec<char> = trimmed.chars().collect();
    let mut end = chars.len();
    while end > 0 {
        let c = chars[end - 1];
        if c.is_ascii_hexdigit() { end -= 1 } else { break }
    }
    let run = &trimmed[..end];
    let head = &trimmed[end..];
    let run_len = run.chars().count();
    let has_digit = run.chars().any(|c| c.is_ascii_digit());
    if run_len >= 8 && has_digit && head.is_empty() == false {
        // Consume one separator before the run.
        let before = &head[..head.len()];
        let stripped = before
            .strip_suffix([' ', '_', '-'])
            .unwrap_or(before);
        return stripped.to_string();
    }
    trimmed.to_string()
}

pub fn humanize_identifier(value: &str) -> String {
    let mut spaced = String::with_capacity(value.len() + 8);
    let chars: Vec<char> = value.chars().collect();
    for (index, &c) in chars.iter().enumerate() {
        if index > 0 && c.is_ascii_uppercase() {
            let previous = chars[index - 1];
            if previous.is_ascii_lowercase() || previous.is_ascii_digit() {
                spaced.push(' ');
            }
        }
        spaced.push(c);
    }
    clean_name(&spaced)
}

/// C# `FriendlyNameFromFile` — version-info friendly name with the known-map fast path.
pub fn friendly_name_from_file(path: &str) -> String {
    let expanded = expand_env(path);
    let exe_name = expanded.rsplit('\\').next().unwrap_or("").to_string();
    if let Some(friendly) = known_windows_executable_name(&exe_name) {
        return friendly.to_string();
    }
    if !crate::paths::is_safe_system_volume_file_probe(&expanded) || !std::path::Path::new(&expanded).is_file() {
        return String::new();
    }
    version_info_name(&expanded)
}

fn known_windows_executable_name(name: &str) -> Option<&'static str> {
    Some(match name.to_lowercase().as_str() {
        "mpdefendercoreservice.exe" => "Microsoft Defender Antivirus Service",
        "msmpeng.exe" => "Antimalware Service Executable",
        "msseces.exe" => "Microsoft Security Essentials",
        "devicecensus.exe" => "Device Census Tool",
        "dxgiadaptercache.exe" => "DirectX DXGI Adapter Cache",
        "mdmagent.exe" => "Mobile Device Management Agent",
        "provtool.exe" => "Windows Provisioning Tool",
        "rundll32.exe" => "Run DLL Command Interface",
        "sc.exe" => "Service Control Manager",
        "ucpdmgr.exe" => "User-Mode Font Driver Manager",
        "wermgr.exe" => "Windows Error Reporting Manager",
        "gameinputsvc.exe" => "Game Input Service",
        "ctfmon.exe" => "Text Input Service",
        "dismhost.exe" => "DISM Host Process",
        "musnotification.exe" => "Windows Update Notification",
        "musnotificationux.exe" => "Windows Update Notification UI",
        "usoclient.exe" => "Windows Update Client",
        "wuauclt.exe" => "Windows Update Client",
        "sihost.exe" => "Shell Infrastructure Host",
        "taskhostw.exe" => "Host Process for Windows Tasks",
        "runtimebroker.exe" => "Runtime Broker",
        "searchprotocolhost.exe" => "Search Protocol Host",
        "searchfilterhost.exe" => "Search Filter Host",
        "searchindexer.exe" => "Windows Search Indexer",
        "searchui.exe" => "Search UI",
        "startmenuexperiencehost.exe" => "Start Menu Experience Host",
        "shellexperiencehost.exe" => "Shell Experience Host",
        "backgroundtaskhost.exe" => "Background Task Host",
        "compptc.exe" => "Credential Manager",
        "dllhost.exe" => "DLL Host Process",
        "lsass.exe" => "Local Security Authority Process",
        "svchost.exe" => "Service Host Process",
        "csrss.exe" => "Client Server Runtime Process",
        "smss.exe" => "Session Manager Process",
        "wininit.exe" => "Windows Init Process",
        "winlogon.exe" => "Windows Logon Application",
        "conhost.exe" => "Console Window Host",
        "dashost.exe" => "Device Association Framework Host",
        "fontdrvhost.exe" => "User Mode Font Driver Host",
        "memory compression" => "Memory Compression Service",
        "spoolsv.exe" => "Print Spooler Service",
        "wmiprvse.exe" => "WMI Provider Host",
        "trustedinstaller.exe" => "Windows Module Installer",
        "tiworker.exe" => "Windows Module Installer Worker",
        "fveux.exe" => "BitLocker User Experience",
        "audiodg.exe" => "Windows Audio Device Graph Isolation",
        "atieclxx.exe" => "AMD External Events Client",
        "atiesrxx.exe" => "AMD External Events Service",
        "rtkauduservice64.exe" => "Realtek Audio Service",
        "nvbackendcontainer.exe" => "NVIDIA Backend Container",
        "nvsphelper64.exe" => "NVIDIA Stereo Helper",
        "securityhealthservice.exe" => "Windows Security Service",
        "securityhealthsystray.exe" => "Windows Security System Tray",
        "senseir.exe" => "Microsoft Defender for Endpoint Sensor",
        "sensendr.exe" => "Microsoft Defender for Endpoint Sensor",
        "mpcmdrun.exe" => "Microsoft Defender Command Line Scanner",
        "cmd.exe" => "Windows Command Processor",
        "msconfig.exe" => "System Configuration",
        "regedit.exe" => "Registry Editor",
        "notepad.exe" => "Notepad",
        "mspaint.exe" => "Paint",
        "calc.exe" => "Calculator",
        "charmap.exe" => "Character Map",
        "snippingtool.exe" => "Snipping Tool",
        "mstsc.exe" => "Remote Desktop Connection",
        "wuaclt.exe" => "Windows Update Client",
        _ => return None,
    })
}

/// Version information (ProductName / FileDescription / InternalName / stem).
fn version_info_name(path: &str) -> String {
    unsafe {
        let mut handle = 0u32;
        let wide = crate::paths::to_wide(path);
        #[allow(non_snake_case)]
        let GetFileVersionInfoSizeW: unsafe extern "system" fn(windows::core::PCWSTR, *mut u32) -> u32 = {
            extern "system" {
                #[link_name = "GetFileVersionInfoSizeW"]
                fn imp(path: windows::core::PCWSTR, handle: *mut u32) -> u32;
            }
            imp
        };
        #[allow(non_snake_case)]
        let GetFileVersionInfoW: unsafe extern "system" fn(windows::core::PCWSTR, u32, u32, *mut core::ffi::c_void) -> i32 = {
            extern "system" {
                #[link_name = "GetFileVersionInfoW"]
                fn imp(path: windows::core::PCWSTR, handle: u32, len: u32, data: *mut core::ffi::c_void) -> i32;
            }
            imp
        };
        #[allow(non_snake_case)]
        let VerQueryValueW: unsafe extern "system" fn(*const core::ffi::c_void, windows::core::PCWSTR, *mut *mut core::ffi::c_void, *mut u32) -> i32 = {
            extern "system" {
                #[link_name = "VerQueryValueW"]
                fn imp(data: *const core::ffi::c_void, block: windows::core::PCWSTR, buffer: *mut *mut core::ffi::c_void, len: *mut u32) -> i32;
            }
            imp
        };
        let size = GetFileVersionInfoSizeW(windows::core::PCWSTR(wide.as_ptr()), &mut handle);
        if size == 0 {
            return String::new();
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(windows::core::PCWSTR(wide.as_ptr()), handle, size, data.as_mut_ptr() as *mut _) == 0 {
            return String::new();
        }
        let mut buffer: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut length = 0u32;
        let subblock = crate::paths::to_wide(r"\StringFileInfo\040904B0\ProductName");
        if VerQueryValueW(data.as_ptr() as *const _, windows::core::PCWSTR(subblock.as_ptr()), &mut buffer, &mut length) != 0 && length > 0 {
            let text = crate::paths::wide_to_string(std::slice::from_raw_parts(buffer as *const u16, length as usize));
            let cleaned = clean_name(&strip_brand_markers(text.trim()));
            if !cleaned.is_empty() {
                return cleaned;
            }
        }
        let subblock = crate::paths::to_wide(r"\StringFileInfo\040904B0\FileDescription");
        if VerQueryValueW(data.as_ptr() as *const _, windows::core::PCWSTR(subblock.as_ptr()), &mut buffer, &mut length) != 0 && length > 0 {
            let text = crate::paths::wide_to_string(std::slice::from_raw_parts(buffer as *const u16, length as usize));
            let cleaned = clean_name(&strip_brand_markers(text.trim()));
            if !cleaned.is_empty() {
                return cleaned;
            }
        }
        String::new()
    }
}

fn strip_brand_markers(value: &str) -> String {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)[\u{00ae}\u{2122}\u{00a9}]|\((r|tm|c)\)").unwrap());
    static WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    WS.replace_all(&RE.replace_all(value, " "), " ").trim().to_string()
}

// ---- installed products ----

pub struct InstalledProductPrefix {
    pub root: String,
    pub family_root: String,
    pub display_name: String,
    pub registration: String,
}

/// C# `ReadInstalledProductNamePrefixes`.
pub fn read_installed_product_name_prefixes(diagnostics: Option<&mut Vec<StartupItem>>) -> Vec<InstalledProductPrefix> {
    let mut result: Vec<InstalledProductPrefix> = Vec::new();
    let uninstall_path = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let scopes: [(RegRoot, bool); 2] = [(RegRoot::CurrentUser, true), (RegRoot::LocalMachine, false)];
    for (root, is_user) in scopes {
        let views = if is_user { user_startup_registry_views() } else { startup_registry_views() };
        for view in views {
            let Some(uninstall) = open_subkey(root, uninstall_path, view) else { continue };
            for child_name in uninstall.subkey_names() {
                let Some(child) = open_subkey(root, &format!("{}\\{}", uninstall_path, child_name), view) else { continue };
                let display_name = child.value("DisplayName").map(|(_, v)| v.display()).unwrap_or_default();
                if display_name.trim().is_empty() {
                    continue;
                }
                let install_location = expand_env(&child.value("InstallLocation").map(|(_, v)| v.display()).unwrap_or_default())
                    .trim().trim_matches('"').trim_end_matches('\\').to_string();
                let mut display_icon = expand_env(&child.value("DisplayIcon").map(|(_, v)| v.display()).unwrap_or_default());
                static INDEX_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r",\s*-?\d+\s*$").unwrap());
                display_icon = INDEX_RE.replace(&display_icon, "").trim().trim_matches('"').to_string();
                let icon_directory = if display_icon.trim().is_empty() { String::new() } else {
                    crate::paths::canonical(&display_icon).rsplit_once('\\').map(|(dir, _)| dir.to_string()).unwrap_or_default()
                };
                for prefix in [install_location, icon_directory] {
                    if prefix.trim().is_empty() || !std::path::Path::new(&prefix).is_absolute() {
                        continue;
                    }
                    let full = match { let c = crate::paths::canonical(&prefix); c.trim_end_matches('\\').to_string() } {
                        f => f,
                    };
                    if !is_specific_installed_product_root(&full) {
                        continue;
                    }
                    let registration = format!("{}\\{}\\{}{}", root.name(), uninstall_path, child_name,
                        if is_user { String::new() } else { format!(" [{}]", view.label()) });
                    result.push(InstalledProductPrefix { root: full, family_root: String::new(), display_name: display_name.trim().to_string(), registration });
                }
            }
        }
    }
    // Deduplicate by root+registration.
    let mut deduped: Vec<InstalledProductPrefix> = Vec::new();
    for item in result {
        if !deduped.iter().any(|d| eq_ci(&d.root, &item.root) && eq_ci(&d.registration, &item.registration)) {
            deduped.push(item);
        }
    }
    // Family root: shortest specific root per registration.
    let mut registrations: Vec<String> = deduped.iter().map(|d| d.registration.clone()).collect();
    registrations.sort();
    registrations.dedup();
    for registration in registrations {
        let mut roots: Vec<&str> = deduped.iter().filter(|d| d.registration == registration).map(|d| d.root.as_str()).collect();
        roots.retain(|r| is_specific_installed_product_root(r));
        roots.sort_by_key(|r| r.len());
        let family_root = roots.first().map(|r| r.to_string()).unwrap_or_default();
        for item in deduped.iter_mut().filter(|d| d.registration == registration) {
            item.family_root = family_root.clone();
        }
    }
    deduped.sort_by(|a, b| b.root.len().cmp(&a.root.len()).then(a.registration.to_lowercase().cmp(&b.registration.to_lowercase())));
    let _ = diagnostics;
    deduped
}

fn is_specific_installed_product_root(value: &str) -> bool {
    let full = crate::paths::normalize_observed_path(value);
    if full.is_empty() {
        return false;
    }
    let broad_roots = [
        std::path::Path::new(&full).ancestors().last().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        std::env::var("ProgramFiles").unwrap_or_default(),
        std::env::var("ProgramFiles(x86)").unwrap_or_default(),
        std::env::var("USERPROFILE").unwrap_or_default(),
        crate::paths::local_app_data(),
        std::env::var("APPDATA").unwrap_or_default(),
        crate::paths::windows_dir(),
    ];
    let broad_roots: Vec<String> = broad_roots.iter().map(|r| crate::paths::normalize_observed_path(r)).filter(|r| !r.is_empty()).collect();
    !broad_roots.iter().any(|r| r.eq_ignore_ascii_case(&full))
}

fn installed_product_for_target(target: &str, prefixes: &[InstalledProductPrefix]) -> Option<(String, String, String, String)> {
    if target.trim().is_empty() {
        return None;
    }
    let target = crate::paths::canonical(&expand_env(target.trim().trim_matches('"')));
    let matches: Vec<&InstalledProductPrefix> = prefixes.iter().filter(|candidate| {
        let prefix = candidate.root.trim_end_matches('\\');
        target.eq_ignore_ascii_case(prefix) || target.to_lowercase().starts_with(&format!("{}\\", prefix.to_lowercase()))
    }).collect();
    if matches.is_empty() {
        return None;
    }
    let longest = matches.iter().map(|m| m.root.len()).max().unwrap_or(0);
    let winners: Vec<&InstalledProductPrefix> = matches.into_iter().filter(|m| m.root.len() == longest).collect();
    let mut registrations: Vec<&str> = winners.iter().map(|w| w.registration.as_str()).collect();
    registrations.sort();
    registrations.dedup();
    if registrations.len() == 1 {
        winners.into_iter().next().map(|w| (w.root.clone(), w.family_root.clone(), w.display_name.clone(), w.registration.clone()))
    } else {
        None
    }
}

pub fn installed_product_name_for_command(command: &str, prefixes: &[InstalledProductPrefix]) -> String {
    match try_split_command(command) {
        Some((target, _)) if !target.trim().is_empty() => {
            installed_product_for_target(&target, prefixes).map(|(_, _, display, _)| display).unwrap_or_default()
        }
        _ => String::new(),
    }
}

// ---- Windows app package identity ----

/// C# `TryGetWindowsAppPackageIdentity`.
pub fn try_get_windows_app_package_identity(target: &str) -> Option<(String, String, String)> {
    let normalized = crate::paths::normalize_observed_path(target);
    let windows_apps = crate::paths::normalize_observed_path(&format!("{}\\WindowsApps",
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into())));
    if normalized.is_empty() || windows_apps.is_empty()
        || !normalized.to_lowercase().starts_with(&format!("{}\\", windows_apps.to_lowercase())) {
        return None;
    }
    let relative = &normalized[windows_apps.len() + 1..];
    let folder = relative.split(['\\', '/']).next()?;
    if folder.trim().is_empty() {
        return None;
    }
    let first_separator = folder.find('_')?;
    let publisher_separator = folder.rfind("__")?;
    if first_separator == 0 || publisher_separator <= first_separator + 1 || publisher_separator + 2 >= folder.len() {
        return None;
    }
    let package_name = &folder[..first_separator];
    let publisher_id = &folder[publisher_separator + 2..];
    if package_name.trim().is_empty() || publisher_id.trim().is_empty() {
        return None;
    }
    let identity = format!("windows-app-package:{}_{}", package_name.to_lowercase(), publisher_id.to_lowercase());
    let package_root = format!("{}\\{}", windows_apps, folder);
    Some((identity, package_root, package_name.to_string()))
}

/// C# `HasConcreteUserApplicationTarget`.
pub fn has_concrete_user_application_target(item: &StartupItem) -> bool {
    if item.application_identity.starts_with("installed-product:") {
        return true;
    }
    if is_script_application_identity(&item.application_identity) {
        return true;
    }
    if !item.application_identity.starts_with("path:") {
        return false;
    }
    let root = crate::paths::normalize_observed_path(&item.application_root);
    let windows = crate::paths::normalize_observed_path(&crate::paths::windows_dir());
    !root.is_empty() && (windows.is_empty()
        || (!root.eq_ignore_ascii_case(&windows) && !root.to_lowercase().starts_with(&format!("{}\\", windows.to_lowercase()))))
}

// ---- hydration ----

/// C# `HydrateHumanNames` — assign application identity and friendly names.
pub fn hydrate_human_names(items: &mut Vec<StartupItem>) {
    let installed_products = read_installed_product_name_prefixes(None);
    for item in items.iter_mut() {
        let payloads = application_payload_candidates(item);
        item.application_targets = payloads.clone();
        let application_target = payloads.first().cloned().unwrap_or_default();
        let script_owned = try_get_startup_script_path(item);
        let payloads_for_script = payloads.clone();
        let script_owned = script_owned.filter(|script| !is_transparent_application_launcher_script(item, script, &payloads_for_script));
        let script_owned = if let Some(script) = script_owned {
            let normalized = crate::paths::normalize_observed_path(&script);
            if normalized.is_empty() { None } else { Some(normalized) }
        } else {
            None
        };
        if let Some(script) = &script_owned {
            if let Some((project_root, project_name)) = try_get_codex_script_project(script) {
                item.application_identity = format!("script-project:{}", project_root.to_lowercase());
                item.application_root = project_root.clone();
                item.app_name = humanize_identifier(&project_name);
                item.application_identity_reason = format!("Startup script is owned by the exact Codex project root {}: {}", project_root, script);
                continue;
            }
            item.application_identity = format!("script:{}", script.to_lowercase());
            item.application_root = crate::paths::normalize_observed_path(script.rsplit_once('\\').map(|(dir, _)| dir).unwrap_or(""));
            let route_name = if item.source.contains("Scheduled Task") {
                friendly_scheduled_task_name(if item.location.starts_with('\\') { &item.location } else { &item.name })
            } else {
                clean_name(&item.name)
            };
            item.app_name = if !route_name.trim().is_empty() { route_name } else { humanize_identifier(&safe_file_stem(script)) };
            item.application_identity_reason = format!("Startup script itself is the application entry point; invoked interpreters and child payloads do not own this route: {}", script);
            continue;
        }
        let packaged = try_get_windows_app_package_identity(&application_target);
        if let Some((identity, root, name)) = &packaged {
            item.application_identity = identity.clone();
            item.application_root = root.clone();
            item.application_identity_reason = format!("Executable belongs to the Windows app package family {}; all launchers in that package share one application identity", name);
            let friendly = friendly_name_for_payload(item, &application_target);
            let sanitized = clean_name(&item.name);
            let generic = item.app_name.trim().is_empty() || eq_ci(&item.app_name, &sanitized);
            if generic && !friendly.is_empty() {
                item.app_name = friendly;
            } else if generic {
                item.app_name = humanize_identifier(&name.replace('.', " "));
            }
            continue;
        }
        if !application_target.trim().is_empty() {
            if let Some((owner_root, owner_family_root, owner_display_name, owner_registration)) = installed_product_for_target(&application_target, &installed_products) {
                let root = crate::paths::normalize_observed_path(&owner_root);
                if !root.is_empty() {
                    item.application_identity = format!("installed-product:{}|registration:{}", clean_name(&owner_display_name).to_lowercase(), b64(&owner_registration));
                    item.application_root = crate::paths::normalize_observed_path(&owner_family_root);
                    item.application_identity_reason = format!("Executable is inside the uniquely owning registered product path {} ({})", root, owner_registration);
                    if !owner_display_name.trim().is_empty() {
                        item.app_name = owner_display_name.trim().to_string();
                    }
                    continue;
                }
            }
            set_portable_application_identity(item, &application_target,
                if payloads.len() > 1 { format!("Startup launcher exposes {} exact executable candidates; current runtime evidence selects the active fallback when present", payloads.len()) }
                else { "Startup registration resolves to this exact executable payload".to_string() });
            if item.app_name.trim().is_empty() {
                item.app_name = friendly_name_for_payload(item, &application_target);
            }
            continue;
        }
        if item.app_name.trim().is_empty() {
            item.app_name = friendly_name_for(item);
        }
    }
    correlate_fallback_application_identities(items);
}

fn set_portable_application_identity(item: &mut StartupItem, target: &str, reason: String) {
    let normalized = crate::paths::normalize_observed_path(target);
    if normalized.is_empty() {
        return;
    }
    item.application_identity = format!("path:{}", normalized.to_lowercase());
    item.application_root = crate::paths::normalize_observed_path(normalized.rsplit_once('\\').map(|(dir, _)| dir).unwrap_or(""));
    item.application_identity_reason = format!("{}: {}", reason, normalized);
}

fn correlate_fallback_application_identities(items: &mut [StartupItem]) {
    use std::collections::HashMap;
    let mut exact: HashMap<String, usize> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if item.application_targets.len() == 1 && !item.application_identity.trim().is_empty() && !is_script_application_identity(&item.application_identity) {
            let target = crate::paths::normalize_observed_path(&item.application_targets[0]);
            if !target.is_empty() && !exact.contains_key(&target.to_lowercase()) {
                exact.insert(target.to_lowercase(), index);
            }
        }
    }
    // Resolve owner metadata with immutable access first, then apply with a mutable pass.
    let mut assignments: Vec<(usize, String, String, String)> = Vec::new();
    for (item_index, item) in items.iter().enumerate() {
        if item.application_targets.len() > 1 && !is_script_application_identity(&item.application_identity) {
            let mut owner: Option<usize> = None;
            for candidate in &item.application_targets {
                if let Some(&owner_index) = exact.get(&crate::paths::normalize_observed_path(candidate).to_lowercase()) {
                    owner = Some(owner_index);
                    break;
                }
            }
            if let Some(owner_index) = owner {
                let owner_item = &items[owner_index];
                let route = if !owner_item.id.is_empty() { owner_item.id.clone() } else { owner_item.location.clone() };
                let reason = format!("Fallback launcher shares an exact executable candidate with route {}: {}",
                    route, owner_item.application_targets.first().cloned().unwrap_or_default());
                assignments.push((item_index, owner_item.application_identity.clone(), owner_item.application_root.clone(), reason));
            }
        }
    }
    for (item_index, identity, root, reason) in assignments {
        if let Some(item) = items.get_mut(item_index) {
            item.application_identity = identity;
            item.application_root = root;
            item.application_identity_reason = reason;
        }
    }
}

// ---- friendly names ----

pub fn friendly_name_for_payload(item: &StartupItem, target: &str) -> String {
    let route = registration_label(item);
    let keyed = route.contains('\u{2014}');
    if !keyed && looks_like_human_name(&route) {
        return route;
    }
    let from_file = informative_file_name(target);
    if !from_file.is_empty() {
        return from_file;
    }
    if !keyed && !route.is_empty() {
        return route;
    }
    if !is_generic_host_executable(target) && !is_disabled_startup_parking(target) {
        let file_only = friendly_name_from_file(target);
        if !file_only.is_empty() {
            return file_only;
        }
    }
    if keyed {
        return route;
    }
    let expanded = expand_env(target);
    if let Some((dir, _)) = expanded.rsplit_once('\\') {
        let parent = dir.rsplit('\\').next().unwrap_or("");
        static BORING: Lazy<Regex> = Lazy::new(|| Regex::new(
            r"(?i)^(application|app|bin|x64|x86|win32|win64|system32|syswow64|sysnative|windows|drivers|common files|program files|program files \(x86\)|programdata|appdata|local|roaming|temp)$").unwrap());
        static VERSION_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d+(\.\d+)+$").unwrap());
        if !parent.is_empty() && !BORING.is_match(parent) && !VERSION_RE.is_match(parent) {
            return clean_name(parent);
        }
    }
    clean_name(&safe_file_stem(if is_disabled_startup_parking(target) { "" } else { target }))
}

fn is_disabled_startup_parking(value: &str) -> bool {
    value.trim().trim_matches('"').to_lowercase().ends_with(".disabled")
}

fn looks_like_human_name(value: &str) -> bool {
    !value.trim().is_empty() && value.trim().contains(' ')
        && Regex::new(r"\p{L}{2,}").map(|re| re.is_match(value)).unwrap_or(false)
}

fn informative_file_name(target: &str) -> String {
    if target.trim().is_empty() || is_generic_host_executable(target) || is_disabled_startup_parking(target) {
        return String::new();
    }
    let name = friendly_name_from_file(target);
    if name.is_empty() {
        return String::new();
    }
    let stem = clean_name(&safe_file_stem(&expand_env(target)));
    if eq_ci(&name, &stem) {
        return String::new();
    }
    if !name.chars().any(|c| c.is_uppercase()) {
        return String::new();
    }
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(installer|setup|uninstaller|bootstrapper)\b").unwrap());
    if RE.is_match(&name) {
        return String::new();
    }
    name
}

/// C# `RegistrationLabel` — the registration's own name with launcher noise removed.
fn registration_label(item: &StartupItem) -> String {
    let mut raw = item.name.clone();
    if item.source == "Startup Folder" {
        static EXT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\.(lnk|url|pif|cmd|bat|exe|vbs|vbe|js|jse|wsf|ps1|py|pyw)$").unwrap());
        raw = EXT_RE.replace(&raw, "").into_owned();
    }
    static GUID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{?[0-9a-fA-F]{8}-(?:[0-9a-fA-F]{4}-){3}[0-9a-fA-F]{12}\}?").unwrap());
    let guid = GUID_RE.find(&raw).map(|m| m.as_str().to_string());
    let raw_dash = raw.find('\u{2014}').unwrap_or(usize::MAX);
    let head = if raw_dash == usize::MAX { raw.clone() } else { raw[..raw_dash].trim().to_string() };
    let human = guid.is_none() && !raw.contains('\\') && !raw.trim_start().starts_with('@') && looks_like_human_name(&head);
    let mut route = if human { light_clean_name(&raw) } else { clean_name(&raw) };
    static MSM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^MichStartupMaster[\\/\s]+").unwrap());
    static CUSTOM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^CustomStartup\s+").unwrap());
    route = MSM_RE.replace(&route, "").into_owned();
    route = CUSTOM_RE.replace(&route, "").into_owned();
    if !human {
        static NOISE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(startup|autostart|tray|silent|launcher|boot)\b").unwrap());
        route = NOISE_RE.replace(&route, " ").into_owned();
    }
    static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    route = WS_RE.replace_all(&route, " ").trim().to_string();
    let dash = route.find('\u{2014}');
    if dash.is_none() {
        let guid = guid.unwrap_or_default();
        return if route.is_empty() && !guid.is_empty() { com_class_name(&guid) } else { route };
    }
    let dash = dash.unwrap();
    let key = route[..dash].trim().to_string();
    let value = route[dash + '\u{2014}'.len_utf8()..].trim().to_string();
    static CONTAINER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
        r"(?i)^(drivers32|knowndlls|networkprovider|securityproviders|windows|urlsearchhooks|appinit[_ ]dlls|appcert[_ ]dlls|shellserviceobjects|shellexecutehooks|inprocserver32|localserver32)$").unwrap());
    static GENERIC_VALUE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
        r"(?i)^(\(default\)|providerpath|name|driver|dllname|dll|path|library|module|server|stubpath)$").unwrap());
    let container_key = key.is_empty() || CONTAINER_RE.is_match(&key);
    let generic_value = value.is_empty() || eq_ci(&key, &value) || GENERIC_VALUE_RE.is_match(&value);
    if (container_key || generic_value) && guid.is_some() {
        let com = com_class_name(guid.as_deref().unwrap_or(""));
        if !com.is_empty() {
            return com;
        }
    }
    if container_key {
        return String::new();
    }
    if generic_value {
        return key;
    }
    route
}

fn light_clean_name(value: &str) -> String {
    static GUID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{[0-9A-Fa-f-]{20,}\}").unwrap());
    static SID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s_-]?S-1-\d+(-\d+){2,}").unwrap());
    static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    let s = GUID_RE.replace_all(value, " ").into_owned();
    let s = SID_RE.replace_all(&s, " ").into_owned();
    let s = strip_trailing_hex_id(s.trim());
    WS_RE.replace_all(&s, " ").trim().to_string()
}

/// Resolve a COM class display name from CLSID.
pub fn com_class_name(guid: &str) -> String {
    let clsid = format!("{{{}}}", guid.trim().trim_matches(['{', '}']));
    if clsid.len() != 38 {
        return String::new();
    }
    for view in [RegView::Registry64, RegView::Registry32] {
        if let Some(key) = open_subkey(RegRoot::ClassesRoot, &format!(r"CLSID\{}", clsid), view) {
            let mut name = key.value("LocalizedString").map(|(_, v)| v.display()).unwrap_or_default();
            name = crate::services::resolve_indirect_string(&name);
            if name.trim().is_empty() || name.trim_start().starts_with('@') {
                name = key.value("").map(|(_, v)| v.display()).unwrap_or_default();
            }
            let mut name = name.trim().to_string();
            if !name.contains(' ') && Regex::new(r"^[A-Za-z_]\w*\.[\w.]+$").map(|re| re.is_match(&name)).unwrap_or(false) {
                name = name[..name.find('.').unwrap_or(name.len())].to_string();
            }
            let name = strip_brand_markers(&name);
            if !name.is_empty() {
                return name;
            }
        }
    }
    String::new()
}

pub fn friendly_name_for(item: &StartupItem) -> String {
    if let Some((target, _)) = try_friendly_target(item) {
        let module = hosted_module_target(&target);
        let payload = if module.is_empty() { target.clone() } else { module };
        return friendly_name_for_payload(item, &payload);
    }
    let label = registration_label(item);
    if !label.is_empty() && !label.contains('\u{2014}') {
        return label;
    }
    if let Ok(stem) = (|| {
        let command = expand_env(item.command.trim().trim_matches('"')).trim().trim_matches('"').to_string();
        Ok::<String, ()>(safe_file_stem(&command))
    })() {
        if Regex::new(r"\p{L}.*\p{L}").map(|re| re.is_match(&stem)).unwrap_or(false)
            && !Regex::new(r"[{}\s]").map(|re| re.is_match(&stem)).unwrap_or(false) {
            return stem.trim().to_string();
        }
    }
    let named = clean_name(&item.name);
    if let Some(dash) = named.find('\u{2014}') {
        let value = named[dash + '\u{2014}'.len_utf8()..].trim().to_string();
        static GENERIC_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(\(default\)|providerpath|name|driver|dllname|dll|path|library|module|server|stubpath)$").unwrap());
        if !value.is_empty() && !GENERIC_RE.is_match(&value) {
            return value;
        }
    }
    if !label.is_empty() {
        return label;
    }
    if !named.is_empty() {
        return named;
    }
    clean_name(&item.location)
}

fn try_friendly_target(item: &StartupItem) -> Option<(String, String)> {
    if item.source == "Startup Folder" && item.command.to_lowercase().ends_with(".lnk") {
        let (target, args) = resolve_shortcut(&item.command);
        if !target.trim().is_empty() {
            return Some((target, args));
        }
    }
    if let Some(pair) = try_decode_tray_payload(&item.command) {
        return Some(pair);
    }
    let payloads = application_payload_candidates(item);
    if let Some(first) = payloads.first() {
        return Some((first.clone(), String::new()));
    }
    try_split_command(&expand_env(&item.command))
}

fn hosted_module_target(target: &str) -> String {
    let file = expand_env(target).rsplit('\\').next().unwrap_or("").to_string();
    if !Regex::new(r"(?i)^(rundll32|regsvr32)\.exe$").map(|re| re.is_match(&file)).unwrap_or(false) {
        return String::new();
    }
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)"?([a-z]:\\[^",]+?\.(dll|cpl|ocx|ax))"?\s*(,|$|\s)"#).unwrap());
    RE.captures(&expand_env(target)).map(|c| c[1].to_string()).unwrap_or_default()
}

/// C# `FriendlyScheduledTaskName` — folder-merged human task name.
pub fn friendly_scheduled_task_name(task_path: &str) -> String {
    let path = task_path.trim().trim_start_matches('\\');
    if path.is_empty() {
        return path.to_string();
    }
    let raw: Vec<&str> = path.split('\\').filter(|s| !s.is_empty()).collect();
    let mut segments: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    for (i, segment) in raw.iter().enumerate() {
        let cleaned = clean_task_segment(segment, i == raw.len() - 1);
        if cleaned.is_empty() {
            continue;
        }
        segments.push(cleaned);
        sources.push(segment.to_string());
    }
    if segments.is_empty() {
        return clean_name(path);
    }
    let mut prefix = String::new();
    if segments.len() >= 3 && segments[0].eq_ignore_ascii_case("Microsoft") && segments[1].eq_ignore_ascii_case("Windows") {
        prefix = "Windows ".into();
        segments.drain(0..2);
        sources.drain(0..2);
    } else if segments.len() == 2 && segments[0].eq_ignore_ascii_case("Microsoft") {
        segments.remove(0);
        sources.remove(0);
    }
    if segments.len() >= 2 && segments[0].eq_ignore_ascii_case("MichStartupMaster")
        && sources[1].to_lowercase().starts_with("customstartup") {
        segments.remove(0);
        sources.remove(0);
    }
    let mut output: Vec<String> = Vec::new();
    for part in &segments {
        static CUSTOM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^CustomStartup\s+").unwrap());
        let mut segment = CUSTOM_RE.replace(part, "").trim().to_string();
        if !prefix.is_empty() {
            let lower = segment.to_lowercase();
            if let Some(rest) = lower.strip_prefix("microsoft windows") {
                let stripped_length = "microsoft windows".len();
                let remainder_start = stripped_length;
                let remainder = &segment[remainder_start..];
                segment = remainder.trim_start().to_string();
            }
        }
        if segment.is_empty() {
            continue;
        }
        if let Some(last) = output.last() {
            if let Some(merged) = merge_repeated_folder(last, &segment) {
                let index = output.len() - 1;
                output[index] = merged;
                continue;
            }
        }
        output.push(segment);
    }
    let mut text = output.join(" ");
    if !prefix.is_empty() && text.to_lowercase().starts_with("windows ") {
        prefix = String::new();
    }
    static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    text = WS_RE.replace_all(&format!("{}{}", prefix, text), " ").trim().to_string();
    if text.is_empty() { clean_name(path) } else { text }
}

fn merge_repeated_folder(folder: &str, segment: &str) -> Option<String> {
    let compact = folder.replace(' ', "");
    if compact.is_empty() || segment.is_empty() {
        return None;
    }
    let segment_chars: Vec<char> = segment.chars().collect();
    let compact_chars: Vec<char> = compact.chars().collect();
    let mut matched = 0;
    let mut i = 0;
    while i < segment_chars.len() && matched < compact_chars.len() {
        if segment_chars[i] == ' ' {
            i += 1;
            continue;
        }
        if segment_chars[i].to_lowercase().next() != compact_chars[matched].to_lowercase().next() {
            return None;
        }
        matched += 1;
        i += 1;
    }
    if matched < compact_chars.len() {
        return None;
    }
    if i < segment_chars.len() && segment_chars[i].is_lowercase() {
        return None;
    }
    let acronym = !folder.contains(' ') && folder.len() <= 5 && folder.chars().all(|c| c.is_uppercase() || c.is_numeric());
    if acronym && segment_chars[..i].iter().collect::<String>() != folder {
        return Some(format!("{}{}", folder, segment_chars[i..].iter().collect::<String>()));
    }
    Some(segment.to_string())
}

fn clean_task_segment(segment: &str, last: bool) -> String {
    static SID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^S-1-\d+(-\d+)+$").unwrap());
    static GUID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s_-]?\{[0-9A-Fa-f-]{20,}\}").unwrap());
    static SID_TAIL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s_-]?S-1-\d+(-\d+){2,}").unwrap());
    static EXE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\.exe$").unwrap());
    static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    let mut s = segment.trim().to_string();
    if SID_RE.is_match(&s) {
        return String::new();
    }
    s = GUID_RE.replace(&s, " ").into_owned();
    s = SID_TAIL_RE.replace(&s, " ").into_owned();
    s = strip_trailing_hex_id(s.trim());
    if last {
        s = EXE_RE.replace(&s, "").into_owned();
    }
    s = s.replace('_', " ");
    s = split_camel_dashes(&s);
    WS_RE.replace_all(&s, " ").trim_matches([' ', '-', '_']).to_string()
}

/// `(?<=[\p{L}\p{N}]{2})-(?=\p{Lu}[\p{L}\p{N}])` without lookaround: turn a
/// dash into a space when 2+ word characters precede it and an
/// uppercase-started word follows.
fn split_camel_dashes(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len() + 4);
    for (index, &c) in chars.iter().enumerate() {
        if c == '-'
            && index >= 2
            && chars[..index].iter().rev().take(2).all(|p| p.is_alphanumeric())
            && chars.get(index + 1).map(|n| n.is_uppercase()).unwrap_or(false)
            && chars.get(index + 2).map(|n| n.is_alphanumeric()).unwrap_or(false) {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

pub fn resolve_target_path(target: &str) -> String {
    if target.trim().is_empty() || std::path::Path::new(target).is_file() {
        return target.to_string();
    }
    let windows_apps = format!("{}\\WindowsApps", std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into()));
    let full = crate::paths::canonical(target);
    if !full.to_lowercase().starts_with(&format!("{}\\", windows_apps.to_lowercase())) {
        return target.to_string();
    }
    let relative = full[windows_apps.len() + 1..].trim_start_matches(['\\', '/']);
    let Some(slash) = relative.find(['\\', '/']) else { return target.to_string() };
    let package_folder = &relative[..slash];
    let sub_path = &relative[slash + 1..];
    let Some(dd) = package_folder.rfind("__") else { return target.to_string() };
    let publisher = &package_folder[dd + 2..];
    let before = &package_folder[..dd];
    let Some(us) = before.find('_') else { return target.to_string() };
    if us == 0 {
        return target.to_string();
    }
    let name = &before[..us];
    let Ok(dirs) = std::fs::read_dir(&windows_apps) else { return target.to_string() };
    let mut best: Option<(Vec<u64>, String)> = None;
    for entry in dirs.filter_map(|e| e.ok()) {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !file_name.to_lowercase().starts_with(&format!("{}_", name.to_lowercase())) {
            continue;
        }
        if !file_name.to_lowercase().ends_with(&format!("__{}", publisher.to_lowercase())) {
            continue;
        }
        let mid = &file_name[name.len() + 1..file_name.len() - (publisher.len() + 2)];
        let Some(sep) = mid.find('_') else { continue };
        let ver_text = &mid[..sep];
        let version: Vec<u64> = match ver_text.split('.').map(|p| p.parse::<u64>()).collect::<Result<Vec<_>, _>>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let candidate = format!("{}\\{}", entry.path().to_string_lossy(), sub_path);
        if !std::path::Path::new(&candidate).is_file() {
            continue;
        }
        if best.as_ref().map(|(v, _)| version > v.clone()).unwrap_or(true) {
            best = Some((version, candidate));
        }
    }
    best.map(|(_, path)| path).unwrap_or_else(|| target.to_string())
}
