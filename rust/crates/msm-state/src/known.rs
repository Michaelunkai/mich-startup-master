//! StartupWatcher — new-registration detection against a persisted baseline.
//! Row: `B64(identity)`.

use crate::{known_store_path, store};
use msm_core::model::StartupItem;
use msm_core::util::{b64, unb64};

fn identity(item: &StartupItem) -> String {
    crate::first_seen::identity(item)
}

fn load_known() -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for line in store::read_all_lines(&known_store_path()) {
        if line.trim().is_empty() { continue; }
        let sig = unb64(line.split('\t').next().unwrap_or(""));
        if !sig.is_empty() {
            set.insert(sig.to_lowercase());
        }
    }
    set
}

fn save_known(items: &[StartupItem]) {
    let mut lines: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for item in items {
        let encoded = b64(&identity(item));
        if !seen.iter().any(|s| s.eq_ignore_ascii_case(&encoded)) {
            seen.push(encoded.clone());
            lines.push(encoded);
        }
    }
    if let Some(parent) = known_store_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    store::write_all_lines(&known_store_path(), &lines);
}

/// Detect new entries since the previous call (first call seeds and returns none).
pub fn detect_new_with(current_items: &[StartupItem]) -> Vec<StartupItem> {
    if !known_store_path().exists() {
        save_known(current_items);
        return Vec::new();
    }
    let known = load_known();
    let fresh: Vec<StartupItem> = current_items.iter()
        .filter(|i| !i.id.starts_with("error|") && !known.contains(&identity(i).to_lowercase()))
        .cloned()
        .collect();
    save_known(current_items);
    fresh
}

pub fn detect_new() -> Vec<StartupItem> {
    detect_new_with(&msm_core::scan::scan_all())
}
