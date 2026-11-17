//! View/scope helpers shared by every registry surface.

use crate::reg::{RegRoot, RegView};

/// Source label for a Run-family subkey path (exact C# mapping).
pub fn source_for_registry_path(sub_key: &str) -> &'static str {
    let lower = sub_key.to_lowercase();
    if lower.contains("runonceex") {
        return "Registry RunOnceEx";
    }
    if lower.contains("runonce") {
        return "Registry RunOnce";
    }
    if lower.contains("runservices") {
        return "Registry RunServices";
    }
    if lower.contains(r"policies\explorer\run") {
        return "Policy Run";
    }
    "Registry Run"
}

/// The Run-key family walked for every scope (machine, user, loaded hives).
pub const RUN_KEYS: [&str; 5] = [
    r"Software\Microsoft\Windows\CurrentVersion\Run",
    r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
    r"Software\Microsoft\Windows\CurrentVersion\RunServices",
    r"Software\Microsoft\Windows\CurrentVersion\RunServicesOnce",
    r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run",
];

pub fn root_display(root: RegRoot) -> &'static str {
    root.name()
}
