//! StartupFirstSeenStore — first-observation timestamps (newest-added-first sort).
//! Row: `B64(identity) \t ticks`.

use crate::{first_seen_store_path, store};
use msm_core::util::{b64, unb64};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn identity(item: &msm_core::model::StartupItem) -> String {
    format!("{}\u{1}{}\u{1}{}\u{1}{}", item.source, item.name.trim(), item.location.trim(), item.command.trim())
}

fn load_map() -> HashMap<String, u128> {
    let mut map = HashMap::new();
    for line in store::read_all_lines(&first_seen_store_path()) {
        if line.trim().is_empty() { continue; }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 { continue; }
        let Ok(ticks) = parts[1].trim().parse::<u128>() else { continue };
        let key = unb64(parts[0]);
        if !key.is_empty() {
            map.insert(key, ticks);
        }
    }
    map
}

fn save_map(map: &HashMap<String, u128>) {
    let mut rows: Vec<(&String, &u128)> = map.iter().collect();
    rows.sort_by_key(|(_, ticks)| **ticks);
    let lines: Vec<String> = rows.iter().map(|(key, ticks)| format!("{}\t{}", b64(key), ticks)).collect();
    if let Some(parent) = first_seen_store_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    store::write_all_lines(&first_seen_store_path(), &lines);
}

/// Record "now" for every identity not seen before.
pub fn observe(items: &[msm_core::model::StartupItem]) {
    let mut map = load_map();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut changed = false;
    for item in items {
        let key = identity(item);
        if map.contains_key(&key) { continue; }
        map.insert(key, now);
        changed = true;
    }
    if changed { save_map(&map); }
}

/// The first-seen timestamp of an item (0 when never observed).
pub fn utc_of(item: &msm_core::model::StartupItem) -> u128 {
    // Every rendered row asks this; a per-process cache keeps list renders off the disk.
    static CACHE: std::sync::Mutex<Option<(i64, std::collections::HashMap<String, u128>)>> =
        std::sync::Mutex::new(None);
    let key = identity(item);
    let generation = crate::store::generation();
    {
        let guard = CACHE.lock().unwrap();
        if let Some((stamp, map)) = guard.as_ref() {
            if *stamp == generation {
                return map.get(&key).copied().unwrap_or(0);
            }
        }
    }
    let map = load_map();
    let value = map.get(&key).copied().unwrap_or(0);
    *CACHE.lock().unwrap() = Some((generation, map));
    value
}

pub fn store_path() -> &'static Path {
    Path::new("")
}
