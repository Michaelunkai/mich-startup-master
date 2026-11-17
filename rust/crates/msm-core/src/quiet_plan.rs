//! Quiet-launch planning (pure logic shared by scan truth, actions, and the tray).

use crate::util::eq_ci;
use once_cell::sync::Lazy;
use regex::Regex;

pub struct QuietLaunchPlan {
    pub target: String,
    pub user_arguments: String,
    pub launch_arguments: String,
    pub strategy: String,
    pub uses_wrapper: bool,
}

const OPEN_SPEEDY_TRAY_SWITCH: &str = "--minimize-to-tray";

fn is_open_speedy(target_path: &str) -> bool {
    let leaf = target_path.rsplit('\\').next().unwrap_or("");
    eq_ci(leaf, "Speedy.exe") || eq_ci(leaf, "OpenSpeedy.exe")
}

pub fn is_self_target(target_path: &str) -> bool {
    match (crate::paths::canonical(target_path), crate::paths::canonical(&crate::util::process_exe_path())) {
        (a, b) if !a.is_empty() && !b.is_empty() => a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\')),
        _ => false,
    }
}

/// C# `QuietLaunchPlanner.Create`.
pub fn create(target_path: &str, arguments: &str) -> QuietLaunchPlan {
    let mut target = target_path.to_string();
    let mut user_arguments = arguments.to_string();
    if let Some(native) = try_unwrap_legacy_tray_quiet(&target, &user_arguments) {
        target = native;
        user_arguments = String::new();
    }
    if let Some(native) = try_unwrap_legacy_open_speedy(&target, &user_arguments) {
        target = native;
        user_arguments = String::new();
    }
    if let Some(native) = try_normalize_codex_launcher(&target) {
        target = native;
    }
    if is_open_speedy(&target) {
        user_arguments = remove_switch(&user_arguments, OPEN_SPEEDY_TRAY_SWITCH);
        return QuietLaunchPlan {
            launch_arguments: append_switch(&user_arguments, OPEN_SPEEDY_TRAY_SWITCH),
            target, user_arguments,
            strategy: "native-tray".into(),
            uses_wrapper: false,
        };
    }
    if is_self_target(&target) {
        return QuietLaunchPlan { target, user_arguments, launch_arguments: "--start-in-tray".into(), strategy: "self-tray".into(), uses_wrapper: false };
    }
    QuietLaunchPlan { target, user_arguments: user_arguments.clone(), launch_arguments: user_arguments, strategy: "managed-proxy".into(), uses_wrapper: true }
}

pub fn normalize_persistent(target_path: &mut String, arguments: &mut String) -> bool {
    let plan = create(target_path, arguments);
    let changed = !target_path.eq_ignore_ascii_case(&plan.target) || *arguments != plan.user_arguments;
    *target_path = plan.target;
    *arguments = plan.user_arguments;
    changed
}

fn try_unwrap_legacy_tray_quiet(target_path: &str, arguments: &str) -> Option<String> {
    let host = target_path.rsplit('\\').next().unwrap_or("");
    if !eq_ci(host, "wscript.exe") && !eq_ci(host, "cscript.exe") {
        return None;
    }
    let windows = crate::paths::canonical(&crate::paths::windows_dir()).trim_end_matches('\\').to_string();
    let host_full = crate::paths::canonical(&crate::util::expand_env(target_path.trim().trim_matches('"')));
    for prefix in [format!("{}\\System32\\", windows), format!("{}\\SysWOW64\\", windows), format!("{}\\Sysnative\\", windows)] {
        if !host_full.to_lowercase().starts_with(&prefix.to_lowercase()) {
            continue;
        }
        static TOKEN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#""([^"]+)"|(\S+)"#).unwrap());
        let tokens: Vec<String> = TOKEN_RE.captures_iter(arguments).map(|c| c.get(1).or_else(|| c.get(2)).map(|m| m.as_str().to_string()).unwrap_or_default()).collect();
        let script_index = tokens.iter().position(|t| eq_ci(t.rsplit('\\').next().unwrap_or(""), "trayquiet-start.vbs"))?;
        if script_index + 1 >= tokens.len() {
            return None;
        }
        let script = crate::util::expand_env(&tokens[script_index]);
        if !std::path::Path::new(&script).is_absolute() {
            return None;
        }
        let script = crate::paths::canonical(&script);
        if !std::path::Path::new(&script).is_file() {
            return None;
        }
        let Ok(source) = std::fs::read_to_string(&script) else { return None };
        if !source.to_lowercase().contains("wscript.arguments")
            || (!source.contains(".Run") && !source.contains(".Exec")) {
            return None;
        }
        let candidate = crate::util::expand_env(&tokens[script_index + 1]);
        if !std::path::Path::new(&candidate).is_absolute() {
            return None;
        }
        let candidate = crate::paths::canonical(&candidate);
        let extension = candidate.rsplit('.').next().unwrap_or("");
        if !std::path::Path::new(&candidate).is_file() || (!eq_ci(extension, "exe") && !eq_ci(extension, "com")) {
            return None;
        }
        return Some(candidate);
    }
    None
}

fn try_unwrap_legacy_open_speedy(target_path: &str, arguments: &str) -> Option<String> {
    let host = target_path.rsplit('\\').next().unwrap_or("");
    if !eq_ci(host, "wscript.exe") && !eq_ci(host, "cscript.exe") {
        return None;
    }
    static SCRIPT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)"([^"]+\.vbs)"|([^\s"]+\.vbs)"#).unwrap());
    let captures = SCRIPT_RE.captures(arguments)?;
    let script = crate::util::expand_env(captures.get(1).or_else(|| captures.get(2))?.as_str());
    if !eq_ci(script.rsplit('\\').next().unwrap_or(""), "openspeedy-silent.vbs") || !std::path::Path::new(&script).is_file() {
        return None;
    }
    let source = std::fs::read_to_string(&script).ok()?;
    static EXE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?im)^\s*exe\s*=\s*"(?<exe>[^"\r\n]+)"\s*$"#).unwrap());
    let exe_match = EXE_RE.captures(&source)?;
    let mut candidate = crate::util::expand_env(exe_match.name("exe")?.as_str());
    if !std::path::Path::new(&candidate).is_absolute() {
        candidate = format!("{}\\{}", script.rsplit_once('\\').map(|(dir, _)| dir).unwrap_or(""), candidate);
    }
    let candidate = crate::paths::canonical(&candidate);
    if !std::path::Path::new(&candidate).is_file() || !is_open_speedy(&candidate) {
        return None;
    }
    Some(candidate)
}

fn try_normalize_codex_launcher(target_path: &str) -> Option<String> {
    let expanded = crate::util::expand_env(target_path.trim().trim_matches('"'));
    if !eq_ci(expanded.rsplit('\\').next().unwrap_or(""), "Codex.exe") {
        return None;
    }
    let (identity, _root, package_name) = crate::humans::try_get_windows_app_package_identity(&expanded)?;
    let _ = identity;
    if !eq_ci(&package_name, "OpenAI.Codex") {
        return None;
    }
    let directory = crate::paths::canonical(&expanded).rsplit_once('\\').map(|(dir, _)| dir)?.to_string();
    if !eq_ci(directory.rsplit('\\').next().unwrap_or(""), "app") {
        return None;
    }
    Some(format!("{}\\ChatGPT.exe", directory))
}

/// C# `DeclaresNativeTrayCapability` — static binary signature probe.
pub fn declares_native_tray_capability(target_path: &str) -> bool {
    let expanded = crate::util::expand_env(target_path.trim().trim_matches('"'));
    let leaf = expanded.rsplit('\\').next().unwrap_or("");
    if is_open_speedy(&expanded) || leaf.starts_with("AutoHotkey") {
        return true;
    }
    if !std::path::Path::new(&expanded).is_absolute() {
        return false;
    }
    let full = crate::paths::canonical(&expanded);
    let sidecar = format!("{}.dll", full.trim_end_matches(".exe").to_string());
    for candidate in [sidecar, full] {
        if !std::path::Path::new(&candidate).is_file() {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(&candidate) else { continue };
        if metadata.len() == 0 || metadata.len() > 16 * 1024 * 1024 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&candidate) else { continue };
        if contains_native_tray_signature(&bytes) {
            return true;
        }
    }
    false
}

pub fn contains_native_tray_signature(bytes: &[u8]) -> bool {
    for signature in ["NotifyIcon", "QSystemTrayIcon", "Shell_NotifyIcon", "NOTIFYICONDATA"] {
        let marker = signature.as_bytes();
        if bytes.windows(marker.len()).any(|window| window == marker) {
            return true;
        }
    }
    false
}

/// C# `IsQuietLaunch` — quiet-mode semantic detection.
pub fn is_quiet_launch(command: &str, task_location: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)--tray-run\s+[A-Za-z0-9+/=]+|--start-in-tray\b|--minimize-to-tray\b").unwrap());
    if RE.is_match(command) {
        return true;
    }
    !task_location.trim().is_empty() && (crate::hooks::hooks().is_quiet_task)(task_location)
}

/// Strip the manager's own quiet switches from persistent arguments.
pub fn strip_known_quiet_arguments(target_path: &str, arguments: &str) -> String {
    let _ = target_path;
    remove_switch(arguments, "--minimize-to-tray");
    let once = remove_switch(arguments, "--minimize-to-tray");
    remove_switch(&once, "--start-in-tray")
}

/// Remove one exact token (case-insensitive) from an argument string, preserving
/// every other token byte-for-byte. Replaces the C# lookahead form.
fn remove_switch(arguments: &str, value: &str) -> String {
    let lower_arguments = arguments.to_lowercase();
    let needle = value.to_lowercase();
    let mut search_from = 0usize;
    while let Some(found) = lower_arguments[search_from..].find(&needle) {
        let start = search_from + found;
        let end = start + needle.len();
        let left_ok = start == 0 || lower_arguments.as_bytes()[start - 1].is_ascii_whitespace();
        let right_ok = end == lower_arguments.len() || lower_arguments.as_bytes()[end].is_ascii_whitespace();
        if left_ok && right_ok {
            let mut merged = String::with_capacity(arguments.len());
            merged.push_str(&arguments[..start]);
            merged.push(' ');
            merged.push_str(&arguments[end..]);
            let ws = Regex::new(r"\s+").unwrap();
            return ws.replace_all(merged.trim(), " ").into_owned();
        }
        search_from = start + 1;
    }
    let ws = Regex::new(r"\s+").unwrap();
    ws.replace_all(arguments.trim(), " ").into_owned()
}

fn append_switch(arguments: &str, value: &str) -> String {
    let cleaned = arguments.trim();
    if cleaned.is_empty() { value.to_string() } else { format!("{} {}", cleaned, value) }
}
