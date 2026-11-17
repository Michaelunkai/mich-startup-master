//! Small shared helpers: case-insensitive compare, quoting, base64, elevation.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STD;
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// C# `string.Equals(a, b, StringComparison.OrdinalIgnoreCase)` for the strings
/// this app handles (paths, ids, source names — all ASCII-case-stable).
pub fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// C# `Q()` — quote an argument for a command line.
pub fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// C# `WinArg()` — quote a path argument the way the manager writes task actions.
pub fn win_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

pub fn b64(s: &str) -> String {
    B64_STD.encode(s.as_bytes())
}

pub fn unb64(s: &str) -> String {
    B64_STD.decode(s.trim()).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default()
}

pub fn b64_bytes(bytes: &[u8]) -> String {
    B64_STD.encode(bytes)
}

/// Is the current process running with an elevated (Administrator) token?
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let _ = windows::Win32::Foundation::CloseHandle(token);
        ok.is_ok() && elevation.TokenIsElevated != 0
    }
}

/// The current process's own executable path (main module).
pub fn process_exe_path() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `Process.GetCurrentProcess().MainModule.FileName` equivalent for the current SID.
pub fn current_user_sid() -> String {
    use windows::core::PWSTR;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return String::new();
        }
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = windows::Win32::Foundation::CloseHandle(token);
            return String::new();
        }
        let mut buffer = vec![0u8; needed as usize];
        let ok = GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr() as *mut _), needed, &mut needed);
        let _ = windows::Win32::Foundation::CloseHandle(token);
        if ok.is_err() {
            return String::new();
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        // ConvertSidToStringSidW
        let mut sid_str = PWSTR::null();
        #[allow(non_snake_case)]
        let ConvertSidToStringSidW: unsafe extern "system" fn(*const core::ffi::c_void, *mut PWSTR) -> i32 = {
            extern "system" {
                #[link_name = "ConvertSidToStringSidW"]
                fn imp(psids: *const core::ffi::c_void, str: *mut PWSTR) -> i32;
            }
            imp
        };
        if ConvertSidToStringSidW(user.User.Sid.0 as *const _, &mut sid_str) == 0 {
            return String::new();
        }
        let result = sid_str.to_string().unwrap_or_default();
        let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(sid_str.0 as *mut _)));
        result
    }
}

/// Current user `DOMAIN\\user` account name.
pub fn current_user_name() -> String {
    std::env::var("USERNAME").unwrap_or_default()
}

/// Expand %ENV% tokens like `Environment.ExpandEnvironmentVariables`.
pub fn expand_env(value: &str) -> String {
    if !value.contains('%') {
        return value.to_string();
    }
    let chars: Vec<char> = value.chars().collect();
    let mut result = String::with_capacity(value.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == '%') {
                let name: String = chars[i + 1..i + 1 + rel].iter().collect();
                if name.is_empty() {
                    result.push('%');
                    i += 2;
                    continue;
                }
                if let Ok(v) = std::env::var(&name) {
                    result.push_str(&v);
                    i += rel + 2;
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Simple wall-clock milliseconds since process start-ish epoch (for budgets).
pub fn tick_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}


