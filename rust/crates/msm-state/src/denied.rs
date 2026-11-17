//! Deny-list store — signatures of purged apps that must never render again.
//! Row: `B64(key) \t ticks \t label`.

use crate::store;
use crate::denied_store_path;
use msm_core::util::{b64, unb64};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct DeniedEntry {
    pub key: String,
    pub ticks: u128,
    pub label: String,
}

fn load_map() -> HashMap<String, (u128, String)> {
    let mut map = HashMap::new();
    for line in store::read_all_lines(&denied_store_path()) {
        if line.trim().is_empty() { continue; }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 { continue; }
        let Ok(ticks) = parts[1].trim().parse::<u128>() else { continue };
        let key = unb64(parts[0]);
        if !key.is_empty() {
            map.insert(key, (ticks, parts.get(2).unwrap_or(&"").to_string()));
        }
    }
    map
}

fn save_map(map: &HashMap<String, (u128, String)>) {
    let mut rows: Vec<(&String, &(u128, String))> = map.iter().collect();
    rows.sort_by_key(|(_, (ticks, _))| *ticks);
    let lines: Vec<String> = rows.iter()
        .map(|(key, (ticks, label))| format!("{}\t{}\t{}", b64(key), ticks, label.replace('\t', " ")))
        .collect();
    if let Some(parent) = denied_store_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    store::write_all_lines(&denied_store_path(), &lines);
}

/// Deny a key (group identity, route id, or name key) so nothing matching it renders again.
pub fn add(key: &str, label: &str) {
    if key.trim().is_empty() { return; }
    let mut map = load_map();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    map.insert(key.trim().to_lowercase(), (now, label.trim().to_string()));
    save_map(&map);
}

/// Lift a denial (restore visibility).
pub fn remove(key: &str) {
    let mut map = load_map();
    map.remove(&key.trim().to_lowercase());
    save_map(&map);
}

pub fn list() -> Vec<DeniedEntry> {
    let mut entries: Vec<DeniedEntry> = load_map().into_iter()
        .map(|(key, (ticks, label))| DeniedEntry { key, ticks, label })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.ticks));
    entries
}

pub fn clear() {
    save_map(&HashMap::new());
}

/// Membership check with a per-generation cache (the list renders every frame).
pub fn contains(key: &str) -> bool {
    if key.trim().is_empty() { return false; }
    static CACHE: std::sync::Mutex<Option<(i64, std::collections::HashSet<String>)>> =
        std::sync::Mutex::new(None);
    let wanted = key.trim().to_lowercase();
    let generation = store::generation();
    {
        let guard = CACHE.lock().unwrap();
        if let Some((stamp, set)) = guard.as_ref() {
            if *stamp == generation {
                return set.contains(&wanted);
            }
        }
    }
    let set: std::collections::HashSet<String> = load_map().keys().cloned().collect();
    let hit = set.contains(&wanted);
    *CACHE.lock().unwrap() = Some((generation, set));
    hit
}

pub fn store_path() -> &'static Path {
    Path::new("")
}
