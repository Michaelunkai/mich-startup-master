//! Startup-folder surfaces: user, common, and every other profile's folder.

use crate::model::{error_item, StartupItem};
use crate::reg::{open_subkey, RegRoot, RegView};
use crate::util::{eq_ci, is_elevated};

fn startup_folder_known(guid: windows::core::GUID, env_path: Option<String>) -> String {
    if let Some(path) = env_path {
        if !path.trim().is_empty() {
            return path;
        }
    }
    crate::paths::known_folder_path(guid, "", "")
}

pub fn user_startup_folder() -> String {
    let env_path = std::env::var("APPDATA").ok()
        .map(|appdata| format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\Startup", appdata.trim_end_matches('\\')));
    startup_folder_known(windows::Win32::UI::Shell::FOLDERID_Startup, env_path)
}

pub fn common_startup_folder() -> String {
    let env_path = std::env::var("ProgramData").ok()
        .map(|pd| format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp", pd.trim_end_matches('\\')));
    startup_folder_known(windows::Win32::UI::Shell::FOLDERID_CommonStartup, env_path)
}

fn add_folder(items: &mut Vec<StartupItem>, folder: &str, scope: &str) {
    let dir = match std::fs::read_dir(folder) {
        Ok(dir) => dir,
        Err(_) => return,
    };
    let mut files: Vec<String> = dir.filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    files.sort_by_key(|f| f.to_lowercase());
    for file in files {
        let leaf = file.rsplit('\\').next().unwrap_or("").to_string();
        if eq_ci(&leaf, "desktop.ini") {
            continue;
        }
        items.push(StartupItem {
            id: crate::ids::folder_id(scope, &file),
            name: leaf,
            source: "Startup Folder".into(),
            scope: scope.into(),
            command: file.clone(),
            location: folder.to_string(),
            enabled: true,
            can_disable: scope == "User" || is_elevated(),
            status: "Starts through Startup folder".into(),
            ..StartupItem::new()
        });
    }
}

/// C# `AddAllStartupFolders` — dedupes full paths across every profile folder.
pub fn add_all_startup_folders(items: &mut Vec<StartupItem>) {
    let mut seen: Vec<String> = Vec::new();
    let mut add = |items: &mut Vec<StartupItem>, folder: &str, scope: &str, seen: &mut Vec<String>| {
        if folder.trim().is_empty() {
            return;
        }
        let full = crate::paths::canonical(&crate::util::expand_env(folder)).trim_end_matches('\\').to_string();
        if seen.iter().any(|s| s.eq_ignore_ascii_case(&full)) {
            return;
        }
        seen.push(full.clone());
        add_folder(items, &full, scope);
    };
    add(items, &user_startup_folder(), "User", &mut seen);
    add(items, &common_startup_folder(), "Machine", &mut seen);
    if let Some(profiles) = open_subkey(RegRoot::LocalMachine, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList", RegView::Default) {
        for sid in profiles.subkey_names() {
            if let Some(profile) = open_subkey(RegRoot::LocalMachine, &format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{}", sid), RegView::Default) {
                let raw = profile.value("ProfileImagePath").map(|(_, v)| v.display()).unwrap_or_default();
                let root = crate::util::expand_env(&raw);
                if root.trim().is_empty() {
                    continue;
                }
                let user_name = root.trim_end_matches('\\').rsplit('\\').next().unwrap_or("").to_string();
                if ["Default", "Public", "Default User", "All Users"].iter().any(|x| eq_ci(&user_name, x)) {
                    continue;
                }
                let scope = if eq_ci(&user_name, &crate::util::current_user_name()) { "User".to_string() } else { format!("User:{}", user_name) };
                let folder = format!("{}\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup", root.trim_end_matches('\\'));
                add(items, &folder, &scope, &mut seen);
            }
        }
    } else {
        items.push(error_item("Startup Folder", "Machine", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList", "ProfileList was not readable"));
    }
}

/// Independent audit variant: same folder set, plain error rows.
pub fn audit_startup_folders(raw: &mut Vec<StartupItem>) {
    let mut seen: Vec<String> = Vec::new();
    let mut remember = |path: &str, scope: &str, seen: &mut Vec<String>| -> Option<(String, String)> {
        if path.trim().is_empty() {
            return None;
        }
        let full = crate::paths::canonical(&crate::util::expand_env(path)).trim_end_matches('\\').to_string();
        if seen.iter().any(|s| s.eq_ignore_ascii_case(&full)) {
            return None;
        }
        seen.push(full.clone());
        Some((full, scope.to_string()))
    };
    let mut folders = Vec::new();
    if let Some(pair) = remember(&user_startup_folder(), "User", &mut seen) { folders.push(pair); }
    if let Some(pair) = remember(&common_startup_folder(), "Machine", &mut seen) { folders.push(pair); }
    if let Some(profiles) = open_subkey(RegRoot::LocalMachine, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList", RegView::Default) {
        for sid in profiles.subkey_names() {
            if let Some(profile) = open_subkey(RegRoot::LocalMachine, &format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{}", sid), RegView::Default) {
                let raw = profile.value("ProfileImagePath").map(|(_, v)| v.display()).unwrap_or_default();
                let root = crate::util::expand_env(&raw);
                let user_name = root.trim_end_matches('\\').rsplit('\\').next().unwrap_or("").to_string();
                if root.trim().is_empty() || ["Default", "Public", "Default User", "All Users"].iter().any(|x| eq_ci(&user_name, x)) {
                    continue;
                }
                let scope = if eq_ci(&user_name, &crate::util::current_user_name()) { "User".to_string() } else { format!("User:{}", user_name) };
                let folder = format!("{}\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup", root.trim_end_matches('\\'));
                if let Some(pair) = remember(&folder, &scope, &mut seen) { folders.push(pair); }
            }
        }
    }
    for (folder, scope) in folders {
        if !std::path::Path::new(&folder).is_dir() {
            continue;
        }
        if let Ok(dir) = std::fs::read_dir(&folder) {
            for entry in dir.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let leaf = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if eq_ci(&leaf, "desktop.ini") {
                    continue;
                }
                raw.push(StartupItem {
                    id: crate::ids::folder_id(&scope, &path.to_string_lossy()),
                    name: leaf,
                    source: "Startup Folder".into(),
                    scope: scope.clone(),
                    command: path.to_string_lossy().into_owned(),
                    location: folder.clone(),
                    enabled: true,
                    ..StartupItem::new()
                });
            }
        }
    }
}
