//! Stable id formats — byte-exact with the C# implementation (stores depend on them).

use crate::util::b64;

pub fn reg_id(scope: &str, root_name: &str, sub_key: &str, value_name: &str, view_label: &str) -> String {
    let view = if view_label.trim().is_empty() { String::new() } else { format!("|{}", view_label) };
    format!("reg|{}|{}|{}|{}{}", scope, b64(root_name), b64(sub_key), b64(value_name), view)
}

pub fn folder_id(scope: &str, file: &str) -> String {
    format!("folder|{}|{}", scope, file)
}

pub fn task_id(task_path: &str) -> String {
    format!("task|{}", task_path)
}

pub fn winlogon_id(scope: &str, root_name: &str, sub_key: &str, value_name: &str, index: usize, view_label: &str) -> String {
    format!("winlogon|{}|{}|{}|{}|{}|{}", scope, b64(root_name), b64(sub_key), b64(value_name), index, view_label)
}

pub fn active_id(scope: &str, root_name: &str, sub_key: &str, view_label: &str) -> String {
    let view = if view_label.trim().is_empty() { String::new() } else { format!("|{}", view_label) };
    format!("active|{}|{}|{}{}", scope, b64(root_name), b64(sub_key), view)
}

pub fn appinit_id(sub_key: &str, view_label: &str) -> String {
    format!("appinit|{}|{}", b64(sub_key), view_label)
}

pub fn advanced_id(root_name: &str, sub_key: &str, value_name: &str, index: usize) -> String {
    format!("advanced|{}|{}|{}|{}", b64(root_name), b64(sub_key), b64(value_name), index)
}

/// The protected-disabled store key for advanced indexed routes.
pub fn advanced_protection_key(root_name: &str, sub_key: &str, value_name: &str, index: i32, view_label: &str) -> String {
    format!("advanced|{}|{}|{}|{}|{}", b64(root_name), b64(sub_key), b64(value_name), index, view_label)
}

pub fn extension_id(kind: &str, root_name: &str, sub_key: &str, value_name: &str, view_label: &str) -> String {
    format!("extension|{}|{}|{}|{}|{}", kind, b64(root_name), b64(sub_key.trim_matches('\\')), b64(value_name), b64(view_label))
}

pub fn service_id(name: &str) -> String {
    format!("service|{}", b64(name))
}

pub fn driver_id(name: &str) -> String {
    format!("driver|{}", b64(name))
}

pub fn gpscript_id(root_name: &str, sub_key: &str) -> String {
    format!("gpscript|{}|{}", b64(root_name), b64(sub_key))
}

pub fn wmisub_id(location: &str) -> String {
    format!("wmisub|{}", b64(location))
}

pub fn wmi_startup_command_id(location: &str, name: &str, sid: &str, user: &str, command: &str) -> String {
    format!("wmi|{}", b64(&format!("{}|{}|{}|{}|{}", location, name, sid, user, command)))
}

pub fn packaged_id(scope: &str, manifest_path: &str, registration_name: &str) -> String {
    format!("packaged|{}|{}|{}", b64(scope), b64(manifest_path), b64(registration_name))
}

pub fn startupapproval_id(scope: &str, root: &str, path: &str, name: &str) -> String {
    format!("startupapproval|{}|{}|{}|{}", b64(scope), b64(root), b64(path), b64(name))
}

pub fn staleapproval_id(scope: &str, root: &str, path: &str, name: &str) -> String {
    format!("staleapproval|{}|{}|{}|{}", b64(scope), b64(root), b64(path), b64(name))
}

pub fn service_protection_key(kind: &str, service_name: &str) -> String {
    let prefix = if kind.eq_ignore_ascii_case("driver") || kind.eq_ignore_ascii_case("System Driver") { "driver" } else { "service" };
    format!("{}|{}", prefix, b64(service_name))
}

pub fn wmi_protection_key(location: &str) -> String {
    format!("wmisub|{}", b64(location))
}

pub fn active_setup_protection_key(scope: &str, root_name: &str, sub_key: &str, view_label: &str) -> String {
    let view = if view_label.trim().is_empty() { String::new() } else { format!("|{}", view_label) };
    format!("active|{}|{}|{}{}", scope, b64(root_name), b64(sub_key), view)
}
