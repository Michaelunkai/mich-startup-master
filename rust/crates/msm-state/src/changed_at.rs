//! Changed-at store — per-key timestamps bumped on every user mutation so the
//! default view can show enabled items first, most-recently-changed first.
//! Row: `B64(key) \t ticks`.

use crate::store;
use crate::changed_at_store_path;
use msm_core::util::{b64, unb64};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn load_map() -> HashMap<String, u128> {
    let mut map = HashMap::new();
    for line in store::read_all_lines(&changed_at_store_path()) {
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
    if let Some(parent) = changed_at_store_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    store::write_all_lines(&changed_at_store_path(), &lines);
}

/// Stamp "now" on every key a mutation touched (route ids + group identity).
pub fn bump(keys: &[String]) {
    if keys.is_empty() { return; }
    let mut map = load_map();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut changed = false;
    for key in keys.iter().filter(|k| !k.trim().is_empty()) {
        map.insert(key.trim().to_lowercase(), now);
        changed = true;
    }
    if changed { save_map(&map); }
}

/// The last-change timestamp of a key (0 when never touched).
pub fn utc_of(key: &str) -> u128 {
    if key.trim().is_empty() { return 0; }
    static CACHE: std::sync::Mutex<Option<(i64, HashMap<String, u128>)>> =
        std::sync::Mutex::new(None);
    let wanted = key.trim().to_lowercase();
    let generation = store::generation();
    {
        let guard = CACHE.lock().unwrap();
        if let Some((stamp, map)) = guard.as_ref() {
            if *stamp == generation {
                return map.get(&wanted).copied().unwrap_or(0);
            }
        }
    }
    let map = load_map();
    let value = map.get(&wanted).copied().unwrap_or(0);
    *CACHE.lock().unwrap() = Some((generation, map));
    value
}

pub fn store_path() -> &'static Path {
    Path::new("")
}
