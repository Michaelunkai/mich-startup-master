//! Inventory cache — the last complete scan serialized to disk so the app
//! renders a full list instantly on launch and refreshes live a moment later.

use msm_core::model::StartupItem;
use std::path::PathBuf;

pub fn cache_path() -> PathBuf {
    PathBuf::from(crate::app_data()).join("inventory-cache.json")
}

pub fn save(items: &[StartupItem]) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(items) {
        let temp = path.with_extension("json.tmp");
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
    }
}

/// The last complete inventory, when a cache exists and parses.
pub fn load() -> Option<Vec<StartupItem>> {
    let bytes = std::fs::read(cache_path()).ok()?;
    serde_json::from_slice(&bytes).ok()
}
