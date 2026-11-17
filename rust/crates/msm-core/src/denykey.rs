//! Shared deny/changed-at keys — one identity vocabulary shared by the purge
//! action (writes keys) and the view filter (reads keys), so a purged app can
//! never re-render from any surface.

use crate::model::StartupItem;

pub fn route_key(item: &StartupItem) -> String {
    item.id.trim().to_lowercase()
}

pub fn name_key(item: &StartupItem) -> String {
    format!("name:{}", item.human_name().trim().to_lowercase())
}

/// Canonical executable identity: tray payload → shortcut target → split
/// command → installed payload candidates, normalized to a lower-cased path.
pub fn target_key(item: &StartupItem) -> String {
    let mut target = String::new();
    if item.source == "Startup Folder" && item.command.to_lowercase().ends_with(".lnk") {
        let (resolved, _) = crate::humans::resolve_shortcut(&item.command);
        target = resolved;
    }
    if target.trim().is_empty() {
        if let Some((resolved, _)) = crate::humans::try_decode_tray_payload(&item.command) {
            target = resolved;
        }
    }
    if target.trim().is_empty() {
        if let Some(first) = crate::humans::application_payload_candidates(item).first() {
            target = first.clone();
        }
    }
    if target.trim().is_empty() {
        if let Some((resolved, _)) = crate::humans::try_split_command(&item.command) {
            target = resolved;
        }
    }
    if target.trim().is_empty() {
        return format!("raw:{}", item.command.trim().to_lowercase());
    }
    let value = crate::util::expand_env(target.trim().trim_matches('"')).replace('/', "\\");
    let value = if std::path::Path::new(&value).is_absolute() { crate::paths::canonical(&value) } else { value };
    format!("target:{}", value.trim_end_matches('\\').to_lowercase())
}

/// Every key that identifies this item for deny-listing and change tracking.
pub fn all_keys(item: &StartupItem) -> Vec<String> {
    vec![route_key(item), name_key(item), target_key(item)]
}
