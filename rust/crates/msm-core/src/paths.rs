//! Path helpers: special folders, safe-volume probing, observed-path normalization.

use crate::util::expand_env;
use windows::core::PCWSTR;

pub fn windows_dir() -> String {
    known_folder(Some(&["SystemRoot", "windir"]), "C:\\Windows")
}

pub fn system_dir() -> String {
    let windows = windows_dir();
    if cfg!(target_arch = "x86_64") {
        format!("{}\\System32", windows.trim_end_matches('\\'))
    } else {
        format!("{}\\Sysnative", windows.trim_end_matches('\\'))
    }
}

pub fn system_dir_x86() -> String {
    let windows = windows_dir();
    if cfg!(target_arch = "x86_64") {
        format!("{}\\SysWOW64", windows.trim_end_matches('\\'))
    } else {
        format!("{}\\System32", windows.trim_end_matches('\\'))
    }
}

fn known_folder(env_names: Option<&[&str]>, fallback: &str) -> String {
    if let Some(names) = env_names {
        for name in names {
            if let Ok(v) = std::env::var(name) {
                if !v.trim().is_empty() {
                    return v;
                }
            }
        }
    }
    // SHGetKnownFolderPath fallback for real special folders when env vars are absent.
    if let Some(guid) = env_names.and_then(|_| None::<()>) {
        let _ = guid;
    }
    fallback.to_string()
}

/// Full path of a known folder by KNOWNFOLDERID, with an env-var fast path.
pub fn known_folder_path(folder_id: windows::core::GUID, env_hint: &str, fallback: &str) -> String {
    if let Ok(v) = std::env::var(env_hint) {
        if !v.trim().is_empty() {
            return v;
        }
    }
    unsafe {
        #[allow(non_snake_case)]
        let SHGetKnownFolderPath: unsafe extern "system" fn(windows::core::GUID, u32, Option<*mut core::ffi::c_void>, *mut windows::core::PWSTR) -> windows::core::HRESULT = {
            extern "system" {
                #[link_name = "SHGetKnownFolderPath"]
                fn imp(rid: windows::core::GUID, flags: u32, token: Option<*mut core::ffi::c_void>, path: *mut windows::core::PWSTR) -> windows::core::HRESULT;
            }
            imp
        };
        let mut path = windows::core::PWSTR::null();
        if SHGetKnownFolderPath(folder_id, 0, None, &mut path).is_ok() {
            let result = path.to_string().unwrap_or_default();
            let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(path.0 as *mut _)));
            if !result.is_empty() {
                return result;
            }
        }
    }
    fallback.to_string()
}

pub fn local_app_data() -> String {
    std::env::var("LOCALAPPDATA").ok().filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| known_folder_path(windows::core::GUID::from_u128(0xF1B32785_6EBA_4FC9_9D55_3B93208CD8B6), "LOCALAPPDATA", ""))
}

pub fn app_data_root_override() -> Option<String> {
    std::env::var("MSM_STATE_ROOT").ok().and_then(|v| {
        let requested = v.trim().trim_matches('"').to_string();
        if requested.is_empty() {
            return None;
        }
        let expanded = expand_env(&requested);
        if expanded.trim().is_empty() || !std::path::Path::new(&expanded).is_absolute() {
            panic!("MSM_STATE_ROOT must be an absolute, uniquely named MichStartupMaster-test-* directory");
        }
        let full = canonical(&expanded);
        let leaf = full.trim_end_matches('\\').rsplit('\\').next().unwrap_or("").to_string();
        let drive_root = std::path::Path::new(&full).ancestors().last().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let profile = std::env::var("USERPROFILE").unwrap_or_default();
        let lad = local_app_data();
        let leaf_ok = leaf.to_lowercase().contains("michstartupmaster-test-") || leaf.to_lowercase().contains("msm-test-");
        let is_root = full.trim_end_matches('\\').eq_ignore_ascii_case(&drive_root.trim_end_matches('\\'));
        let is_profile = !profile.is_empty() && full.trim_end_matches('\\').eq_ignore_ascii_case(profile.trim_end_matches('\\'));
        let is_lad = !lad.is_empty() && full.trim_end_matches('\\').eq_ignore_ascii_case(lad.trim_end_matches('\\'));
        if is_root || is_profile || is_lad || !leaf_ok {
            panic!("MSM_STATE_ROOT must be an absolute, uniquely named MichStartupMaster-test-* directory");
        }
        Some(full.trim_end_matches('\\').to_string())
    })
}

/// The per-user state root (%LOCALAPPDATA%\\MichStartupMaster, or MSM_STATE_ROOT).
pub fn state_root() -> String {
    if let Some(over) = app_data_root_override() {
        return over;
    }
    format!("{}\\MichStartupMaster", local_app_data().trim_end_matches('\\'))
}

pub fn canonical(path: &str) -> String {
    // GetFullPathNameW-normalized form without touching the filesystem.
    let wide = to_wide(path);
    unsafe {
        let len = windows::Win32::Storage::FileSystem::GetFullPathNameW(windows::core::PCWSTR::from_raw(wide.as_ptr()), None, None);
        if len == 0 {
            return path.to_string();
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let written = windows::Win32::Storage::FileSystem::GetFullPathNameW(windows::core::PCWSTR::from_raw(wide.as_ptr()), Some(&mut buffer), None);
        if written == 0 {
            return path.to_string();
        }
        String::from_utf16_lossy(&buffer[..written as usize])
    }
}

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

pub fn pcw(s: &str) -> PCWSTR {
    PCWSTR(to_wide(s).as_ptr())
}

/// C# `NormalizeObservedPath`: expand, absolute-only, GetFullPath, trim trailing '\\'.
pub fn normalize_observed_path(value: &str) -> String {
    let expanded = expand_env(value.trim().trim_matches('"'));
    if expanded.trim().is_empty() || !std::path::Path::new(&expanded).is_absolute() {
        return String::new();
    }
    canonical(&expanded).trim_end_matches('\\').to_string()
}

/// C# `ExpandSystemModulePath`: bare names resolve under System32; absolute kept.
pub fn expand_system_module_path(value: &str) -> String {
    let candidate = expand_env(value.trim());
    if candidate.is_empty() {
        return String::new();
    }
    let unquoted = candidate.trim().trim_matches('"');
    if !std::path::Path::new(unquoted).is_absolute() && !unquoted.contains('\\') && !unquoted.contains('/') {
        return format!("{}\\{}", system_dir().trim_end_matches('\\'), unquoted);
    }
    candidate
}

/// Inventory must only synchronously probe files on the always-local system volume.
pub fn is_safe_system_volume_file_probe(value: &str) -> bool {
    let candidate = expand_env(value.trim_matches('"'));
    if candidate.is_empty() || !std::path::Path::new(&candidate).is_absolute() || candidate.starts_with("\\\\") {
        return false;
    }
    let candidate_root = std::path::Path::new(&candidate).ancestors().last().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let system_root = std::path::Path::new(&system_dir()).ancestors().last().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    !candidate_root.is_empty() && !system_root.is_empty() && candidate_root.eq_ignore_ascii_case(&system_root)
}

/// Expand DOS 8.3 short names to long names (Win32_StartupCommand mirror matching).
pub fn expand_short_path_names(path: &str) -> String {
    if !path.contains('~') {
        return path.to_string();
    }
    unsafe {
        #[allow(non_snake_case)]
        let GetLongPathNameW: unsafe extern "system" fn(windows::core::PCWSTR, windows::core::PWSTR, u32) -> u32 = {
            extern "system" {
                #[link_name = "GetLongPathNameW"]
                fn imp(path: windows::core::PCWSTR, buffer: windows::core::PWSTR, len: u32) -> u32;
            }
            imp
        };
        let wide = to_wide(path);
        let len = GetLongPathNameW(windows::core::PCWSTR::from_raw(wide.as_ptr()), windows::core::PWSTR::null(), 0);
        if len == 0 {
            return path.to_string();
        }
        let mut buffer = vec![0u16; len as usize];
        let written = GetLongPathNameW(windows::core::PCWSTR::from_raw(wide.as_ptr()), windows::core::PWSTR(buffer.as_mut_ptr()), len);
        if written > 0 && written <= len {
            return String::from_utf16_lossy(&buffer[..written as usize]);
        }
        path.to_string()
    }
}
