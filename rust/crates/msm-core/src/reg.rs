//! Native registry engine: views, kinds, reads, writes, exact value state.

use crate::util::b64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_CREATE_SUB_KEY, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE,
    KEY_READ, KEY_SET_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_BINARY, REG_DWORD,
    REG_EXPAND_SZ, REG_MULTI_SZ, REG_QWORD, REG_SZ, REG_OPEN_CREATE_OPTIONS, REG_SAM_FLAGS,
};

/// REG_OPTION_NON_DIRTY (winreg.h 0x40000000) is not exported by the crate.
/// RegCreateKeyExW rejects it with ERROR_INVALID_PARAMETER, which made every create_subkey
/// (packaged StartupTask State writes included) fail. Use plain non-volatile (0) instead.
const REG_OPTION_NON_DIRTY: REG_OPEN_CREATE_OPTIONS = REG_OPEN_CREATE_OPTIONS(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegView {
    Default,
    Registry32,
    Registry64,
}

impl RegView {
    pub fn label(self) -> &'static str {
        match self {
            RegView::Registry32 => "Registry32",
            RegView::Registry64 => "Registry64",
            RegView::Default => "",
        }
    }
    pub fn wow_flag(self) -> u32 {
        match self {
            RegView::Registry32 => KEY_WOW64_32KEY.0,
            RegView::Registry64 => KEY_WOW64_64KEY.0,
            RegView::Default => 0,
        }
    }
    pub fn parse(value: &str) -> RegView {
        if value.eq_ignore_ascii_case("Registry32") {
            RegView::Registry32
        } else if value.eq_ignore_ascii_case("Registry64") {
            RegView::Registry64
        } else {
            RegView::Default
        }
    }
}

pub fn is_64bit_os() -> bool {
    std::env::consts::ARCH == "x86_64" || std::env::consts::ARCH == "aarch64"
}

/// Machine hive: enumerate both WOW64 views.
pub fn startup_registry_views() -> Vec<RegView> {
    if is_64bit_os() {
        vec![RegView::Registry64, RegView::Registry32]
    } else {
        vec![RegView::Registry32]
    }
}

/// HKCU/HKU per-user keys are redirector-shared: exactly one (native) view.
pub fn user_startup_registry_views() -> Vec<RegView> {
    vec![if is_64bit_os() { RegView::Registry64 } else { RegView::Registry32 }]
}

pub fn native_registry_view() -> RegView {
    if is_64bit_os() { RegView::Registry64 } else { RegView::Registry32 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegKind {
    None,
    Unknown,
    String,
    ExpandString,
    Binary,
    DWord,
    QWord,
    MultiString,
}

impl RegKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RegKind::None => "None",
            RegKind::Unknown => "Unknown",
            RegKind::String => "String",
            RegKind::ExpandString => "ExpandString",
            RegKind::Binary => "Binary",
            RegKind::DWord => "DWord",
            RegKind::QWord => "QWord",
            RegKind::MultiString => "MultiString",
        }
    }
    pub fn parse(value: &str) -> RegKind {
        match value.to_lowercase().as_str() {
            "none" => RegKind::None,
            "unknown" => RegKind::Unknown,
            "string" => RegKind::String,
            "expandstring" => RegKind::ExpandString,
            "binary" => RegKind::Binary,
            "dword" => RegKind::DWord,
            "qword" => RegKind::QWord,
            "multistring" => RegKind::MultiString,
            _ => RegKind::Unknown,
        }
    }
}

/// A raw registry value with its exact kind (never env-expanded on read).
#[derive(Debug, Clone)]
pub enum RegValue {
    Str(String),
    Bytes(Vec<u8>),
    DWord(u32),
    QWord(u64),
    MultiString(Vec<String>),
}

impl RegValue {
    /// C# `Convert.ToString(value)` projection used for display commands.
    pub fn display(&self) -> String {
        match self {
            RegValue::Str(s) => s.clone(),
            RegValue::Bytes(_) => String::new(),
            RegValue::DWord(v) => (*v as i32).to_string(),
            RegValue::QWord(v) => (*v as i64).to_string(),
            RegValue::MultiString(parts) => parts.join(" ; "),
        }
    }

    /// C# `SerializeRegistryValue` — exact persisted payload.
    pub fn serialize(&self) -> String {
        match self {
            RegValue::MultiString(parts) => serde_json::to_string(parts).unwrap_or_else(|_| "[]".into()),
            RegValue::Bytes(bytes) => crate::util::b64_bytes(bytes),
            RegValue::Str(s) => s.clone(),
            RegValue::DWord(v) => (*v as i32).to_string(),
            RegValue::QWord(v) => (*v as i64).to_string(),
        }
    }
}

pub struct RegKeyHandle(pub HKEY);

impl Drop for RegKeyHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegRoot {
    LocalMachine,
    CurrentUser,
    Users,
    ClassesRoot,
}

impl RegRoot {
    pub fn name(&self) -> &'static str {
        match self {
            RegRoot::LocalMachine => "HKEY_LOCAL_MACHINE",
            RegRoot::CurrentUser => "HKEY_CURRENT_USER",
            RegRoot::Users => "HKEY_USERS",
            RegRoot::ClassesRoot => "HKEY_CLASSES_ROOT",
        }
    }
    fn predefined(&self) -> HKEY {
        match self {
            RegRoot::LocalMachine => HKEY_LOCAL_MACHINE,
            RegRoot::CurrentUser => HKEY_CURRENT_USER,
            RegRoot::Users => HKEY_USERS,
            RegRoot::ClassesRoot => HKEY_CLASSES_ROOT,
        }
    }
    pub fn parse(name: &str) -> Option<RegRoot> {
        if name.eq_ignore_ascii_case("HKEY_LOCAL_MACHINE") || name.eq_ignore_ascii_case("HKLM") {
            Some(RegRoot::LocalMachine)
        } else if name.eq_ignore_ascii_case("HKEY_CURRENT_USER") || name.eq_ignore_ascii_case("HKCU") {
            Some(RegRoot::CurrentUser)
        } else if name.eq_ignore_ascii_case("HKEY_USERS") || name.eq_ignore_ascii_case("HKU") {
            Some(RegRoot::Users)
        } else if name.eq_ignore_ascii_case("HKEY_CLASSES_ROOT") || name.eq_ignore_ascii_case("HKCR") {
            Some(RegRoot::ClassesRoot)
        } else {
            None
        }
    }
}

/// Open `root\subKey` read-only under a specific WOW64 view.
pub fn open_subkey(root: RegRoot, sub_key: &str, view: RegView) -> Option<RegKeyHandle> {
    let wide = crate::paths::to_wide(sub_key);
    let mut hkey = HKEY::default();
    let access = KEY_READ.0 | view.wow_flag();
    let result = unsafe {
        RegOpenKeyExW(root.predefined(), PCWSTR(wide.as_ptr()), None, windows::Win32::System::Registry::REG_SAM_FLAGS(access), &mut hkey)
    };
    if result.is_ok() {
        Some(RegKeyHandle(hkey))
    } else {
        None
    }
}

/// Open with read+write (used for the write preflight and mutations).
pub fn open_subkey_write(root: RegRoot, sub_key: &str, view: RegView) -> Option<RegKeyHandle> {
    let wide = crate::paths::to_wide(sub_key);
    let mut hkey = HKEY::default();
    let access = (KEY_QUERY_VALUE.0 | KEY_ENUMERATE_SUB_KEYS.0 | KEY_SET_VALUE.0) | view.wow_flag();
    let result = unsafe {
        RegOpenKeyExW(root.predefined(), PCWSTR(wide.as_ptr()), None, windows::Win32::System::Registry::REG_SAM_FLAGS(access), &mut hkey)
    };
    if result.is_ok() {
        Some(RegKeyHandle(hkey))
    } else {
        None
    }
}

/// `RegistryKey.CreateSubKey(path)` equivalent (read/write, creates missing keys).
pub fn create_subkey(root: RegRoot, sub_key: &str, view: RegView) -> Option<RegKeyHandle> {
    let wide = crate::paths::to_wide(sub_key);
    let mut hkey = HKEY::default();
    let access = (KEY_QUERY_VALUE.0 | KEY_ENUMERATE_SUB_KEYS.0 | KEY_SET_VALUE.0 | KEY_CREATE_SUB_KEY.0) | view.wow_flag();
    let result = unsafe {
        RegCreateKeyExW(
            root.predefined(),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            REG_OPTION_NON_DIRTY,
            windows::Win32::System::Registry::REG_SAM_FLAGS(access),
            None,
            &mut hkey,
            None,
        )
    };
    if result.is_ok() {
        Some(RegKeyHandle(hkey))
    } else {
        None
    }
}

impl RegKeyHandle {
    /// All value names ("" represents the default value), in enumeration order.
    pub fn value_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        unsafe {
            for index in 0.. {
                let mut buffer = [0u16; 16384];
                let mut size = buffer.len() as u32;
                let result = RegEnumValueW(self.0, index, Some(windows::core::PWSTR(buffer.as_mut_ptr())), &mut size, None, None, None, None);
                if result == ERROR_NO_MORE_ITEMS {
                    break;
                }
                if result.is_err() {
                    break;
                }
                names.push(crate::paths::wide_to_string(&buffer[..size as usize]));
            }
        }
        names
    }

    /// Subkey names in enumeration order.
    pub fn subkey_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        unsafe {
            for index in 0.. {
                let mut buffer = [0u16; 16384];
                let mut size = buffer.len() as u32;
                let result = RegEnumKeyExW(self.0, index, Some(windows::core::PWSTR(buffer.as_mut_ptr())), &mut size, None, None, None, None);
                if result == ERROR_NO_MORE_ITEMS || result.is_err() {
                    break;
                }
                names.push(crate::paths::wide_to_string(&buffer[..size as usize]));
            }
        }
        names
    }

    /// Raw value read (no environment expansion); returns kind + value.
    pub fn value(&self, name: &str) -> Option<(RegKind, RegValue)> {
        let wide_name = crate::paths::to_wide(name);
        let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
        let mut needed = 0u32;
        let name_ptr = if name.is_empty() { PCWSTR::null() } else { PCWSTR(wide_name.as_ptr()) };
        let size_result = unsafe {
            RegQueryValueExW(self.0, name_ptr, None, Some(&mut kind), None, Some(&mut needed))
        };
        if size_result.is_err() {
            return None;
        }
        let mut buffer = vec![0u8; needed as usize];
        let result = unsafe {
            RegQueryValueExW(self.0, name_ptr, None, Some(&mut kind), Some(buffer.as_mut_ptr()), Some(&mut needed))
        };
        if result.is_err() {
            return None;
        }
        buffer.truncate(needed as usize);
        decode_value(kind.0, &buffer)
    }

    pub fn value_kind(&self, name: &str) -> Option<RegKind> {
        self.value(name).map(|(kind, _)| kind)
    }

    /// Write a value with an explicit kind.
    pub fn set_value(&self, name: &str, kind: RegKind, value: &RegValue) -> bool {
        let wide_name = crate::paths::to_wide(name);
        let name_ptr = if name.is_empty() { PCWSTR::null() } else { PCWSTR(wide_name.as_ptr()) };
        let (type_code, bytes): (u32, Vec<u8>) = match (&kind, value) {
            (RegKind::String, RegValue::Str(s)) => (REG_SZ.0, crate::paths::to_wide(s).into_iter().flat_map(|c| c.to_le_bytes()).collect()),
            (RegKind::ExpandString, RegValue::Str(s)) => (REG_EXPAND_SZ.0, crate::paths::to_wide(s).into_iter().flat_map(|c| c.to_le_bytes()).collect()),
            (RegKind::Binary, RegValue::Bytes(b)) => (REG_BINARY.0, b.clone()),
            (RegKind::DWord, RegValue::DWord(v)) => (REG_DWORD.0, v.to_le_bytes().to_vec()),
            (RegKind::QWord, RegValue::QWord(v)) => (REG_QWORD.0, v.to_le_bytes().to_vec()),
            (RegKind::MultiString, RegValue::MultiString(parts)) => {
                let mut wide: Vec<u16> = Vec::new();
                for part in parts {
                    wide.extend(part.encode_utf16());
                    wide.push(0);
                }
                wide.push(0);
                (REG_MULTI_SZ.0, wide.into_iter().flat_map(|c| c.to_le_bytes()).collect())
            }
            _ => return false,
        };
        let result = unsafe {
            RegSetValueExW(self.0, name_ptr, None, windows::Win32::System::Registry::REG_VALUE_TYPE(type_code),
                Some(&bytes))
        };
        result.is_ok()
    }

    pub fn delete_value(&self, name: &str) -> bool {
        let wide_name = crate::paths::to_wide(name);
        let name_ptr = if name.is_empty() { PCWSTR::null() } else { PCWSTR(wide_name.as_ptr()) };
        unsafe { RegDeleteValueW(self.0, name_ptr).is_ok() }
    }
}

fn decode_value(kind: u32, buffer: &[u8]) -> Option<(RegKind, RegValue)> {
    match kind {
        1 => Some((RegKind::String, RegValue::Str(crate::paths::wide_to_string(&to_u16(buffer))))),
        2 => Some((RegKind::ExpandString, RegValue::Str(crate::paths::wide_to_string(&to_u16(buffer))))),
        3 => Some((RegKind::Binary, RegValue::Bytes(buffer.to_vec()))),
        4 if buffer.len() >= 4 => Some((RegKind::DWord, RegValue::DWord(u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]])))),
        11 if buffer.len() >= 8 => Some((RegKind::QWord, RegValue::QWord(u64::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3], buffer[4], buffer[5], buffer[6], buffer[7]])))),
        7 => {
            let wide = to_u16(buffer);
            let mut parts = Vec::new();
            let mut current = Vec::new();
            for ch in wide {
                if ch == 0 {
                    if current.is_empty() {
                        break;
                    }
                    parts.push(String::from_utf16_lossy(&current));
                    current.clear();
                } else {
                    current.push(ch);
                }
            }
            Some((RegKind::MultiString, RegValue::MultiString(parts)))
        }
        0 => Some((RegKind::None, RegValue::Bytes(Vec::new()))),
        _ => Some((RegKind::Unknown, RegValue::Bytes(buffer.to_vec()))),
    }
}

fn to_u16(bytes: &[u8]) -> Vec<u16> {
    bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect()
}

/// C# `RegistryValueState` — exact persisted snapshot with a content hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryValueState {
    #[serde(rename = "Exists")]
    pub exists: bool,
    #[serde(rename = "Kind")]
    pub kind: String,
    #[serde(rename = "Payload")]
    pub payload: String,
    #[serde(rename = "Hash")]
    pub hash: String,
}

impl RegistryValueState {
    pub fn build(exists: bool, kind: RegKind, value: Option<&RegValue>) -> RegistryValueState {
        let payload = if exists {
            value.map(|v| v.serialize()).unwrap_or_default()
        } else {
            String::new()
        };
        let kind_name = if exists { kind.as_str() } else { "None" };
        let hash = hash_state(exists, kind_name, &payload);
        RegistryValueState { exists, kind: kind_name.to_string(), payload, hash }
    }

    pub fn missing() -> RegistryValueState {
        Self::build(false, RegKind::None, None)
    }

    pub fn kind(&self) -> RegKind {
        RegKind::parse(&self.kind)
    }

    pub fn decode(&self) -> Option<RegValue> {
        deserialize_registry_value_exact(&self.payload, self.kind())
    }

    pub fn hash_ok(&self) -> bool {
        eq_hash(&self.hash, &hash_state(self.exists, &self.kind, &self.payload))
    }
}

fn hash_state(exists: bool, kind: &str, payload: &str) -> String {
    let canonical = format!("{}\n{}\n{}", if exists { 1 } else { 0 }, kind, payload);
    let digest = Sha256::digest(canonical.as_bytes());
    digest.iter().map(|b| format!("{:02X}", b)).collect::<String>().to_lowercase()
}

fn eq_hash(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// C# `RegistryValueStateEquals` (with hash self-validation).
pub fn registry_value_state_equals(left: &RegistryValueState, right: &RegistryValueState) -> bool {
    if !left.hash_ok() || !right.hash_ok() {
        return false;
    }
    left.exists == right.exists
        && left.kind == right.kind
        && left.payload == right.payload
        && eq_hash(&left.hash, &right.hash)
}

/// Exact payload decode (fails closed on malformed stored data).
pub fn deserialize_registry_value_exact(payload: &str, kind: RegKind) -> Option<RegValue> {
    match kind {
        RegKind::MultiString => {
            let values: Vec<String> = serde_json::from_str(payload).ok()?;
            Some(RegValue::MultiString(values))
        }
        RegKind::Binary => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(payload).ok().map(RegValue::Bytes)
        }
        RegKind::DWord => payload.trim().parse::<i32>().ok().map(|v| RegValue::DWord(v as u32)),
        RegKind::QWord => payload.trim().parse::<i64>().ok().map(|v| RegValue::QWord(v as u64)),
        RegKind::String | RegKind::ExpandString => Some(RegValue::Str(payload.to_string())),
        _ => None,
    }
}

/// Lenient decode used for legacy restore payloads.
pub fn deserialize_registry_value(payload: &str, kind: RegKind) -> Option<RegValue> {
    deserialize_registry_value_exact(payload, kind).or_else(|| Some(RegValue::Str(payload.to_string())))
}

/// Read the exact current state of one registry value.
pub fn read_registry_value_state(root: RegRoot, sub_key: &str, value_name: &str, view: RegView) -> RegistryValueState {
    match open_subkey(root, sub_key, view) {
        None => RegistryValueState::missing(),
        Some(key) => {
            if !key.value_names().iter().any(|name| name.eq_ignore_ascii_case(value_name)) {
                return RegistryValueState::missing();
            }
            match key.value(value_name) {
                Some((kind, value)) => RegistryValueState::build(true, kind, Some(&value)),
                None => RegistryValueState::missing(),
            }
        }
    }
}

/// Write a value only when its current state still equals `expected_before`.
pub fn write_registry_value_state_exact(root: RegRoot, sub_key: &str, value_name: &str, view: RegView,
                                          expected_before: &RegistryValueState, desired_after: &RegistryValueState) -> Result<(), String> {
    if !expected_before.hash_ok() || !desired_after.hash_ok() {
        return Err("Registry mutation state is hash-invalid".into());
    }
    let current = read_registry_value_state(root, sub_key, value_name, view);
    if !registry_value_state_equals(&current, expected_before) {
        return Err("Registry startup value changed after scanning; refresh before changing it".into());
    }
    if desired_after.exists {
        let kind = desired_after.kind();
        let value = desired_after.decode().ok_or_else(|| "Stored registry value kind is invalid".to_string())?;
        let key = open_subkey_write(root, sub_key, view)
            .ok_or_else(|| format!("Registry startup key is missing or not writable: {}", sub_key))?;
        if !key.set_value(value_name, kind, &value) {
            return Err("Registry startup value write failed".into());
        }
    } else {
        let key = open_subkey_write(root, sub_key, view)
            .ok_or_else(|| format!("Registry startup key is missing or not writable: {}", sub_key))?;
        if !key.delete_value(value_name) {
            return Err("Registry startup value delete failed".into());
        }
    }
    let verified = read_registry_value_state(root, sub_key, value_name, view);
    if !registry_value_state_equals(&verified, desired_after) {
        return Err("Registry startup value did not verify after writing".into());
    }
    Ok(())
}

/// C# `CanOpenRegistryValueForWrite` — write preflight with an exact value check.
pub fn can_open_registry_value_for_write(root: RegRoot, sub_key: &str, value_name: &str, view: RegView) -> (bool, String) {
    let key = match open_subkey_write(root, sub_key, view) {
        None => return (false, "registry key is missing or not writable".into()),
        Some(key) => key,
    };
    if !key.value_names().iter().any(|name| name.eq_ignore_ascii_case(value_name)) {
        return (false, "registry value is missing".into());
    }
    if key.value_kind(value_name).is_none() {
        return (false, "registry value kind is unreadable".into());
    }
    (true, String::new())
}

/// Guard helper — `#[allow]` for the unused WIN32_ERROR import set kept for parity.
#[allow(unused)]
fn _win32_error_hint(e: WIN32_ERROR) -> WIN32_ERROR {
    let ok = ERROR_SUCCESS;
    let _ = CloseHandle; let _ = e; ok
}

/// Registry protection key identity: `reg|scope|b64(root)|b64(sub)|b64(value)[|view]`.
pub fn registry_protection_key(scope: &str, root_name: &str, sub_key: &str, value_name: &str, view_label: &str) -> String {
    let view = if view_label.trim().is_empty() { String::new() } else { format!("|{}", view_label) };
    format!("reg|{}|{}|{}|{}{}", scope, b64(root_name), b64(sub_key), b64(value_name), view)
}

pub fn root_from_name(root_name: &str) -> Option<RegRoot> {
    RegRoot::parse(root_name)
}

/// Keep `unb64` linked for state-record round-trips in this module's callers.
#[allow(dead_code)]
fn _unb64_hint(s: &str) -> String {
    crate::util::unb64(s)
}
