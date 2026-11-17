//! State stores: cross-process locked, atomic, `.bak`-recoverable TSV files that
//! stay byte-compatible with the C# implementation (spec docs/spec-rust/01+03).

pub mod store;
pub mod coordinator;
pub mod disabled;
pub mod protected_disabled;
pub mod protected_quiet;
pub mod protected_quiet_enforce;

pub mod enabled;
pub mod first_seen;
pub mod known;
pub mod wmi_guard;
pub mod denied;
pub mod changed_at;
pub mod cache;

use msm_core::util::eq_ci;
use once_cell::sync::Lazy;
use std::path::PathBuf;

pub fn app_data() -> String {
    msm_core::paths::state_root()
}

pub fn disabled_store() -> PathBuf { PathBuf::from(app_data()).join("disabled-items.tsv") }
pub fn protected_disabled_store() -> PathBuf { PathBuf::from(app_data()).join("protected-disabled-items.tsv") }
pub fn protected_quiet_store() -> PathBuf { PathBuf::from(app_data()).join("protected-quiet-popup-items.tsv") }
pub fn enabled_store() -> PathBuf { PathBuf::from(app_data()).join("enabled-startup-items.tsv") }
pub fn migrated_marker_path() -> PathBuf { PathBuf::from(app_data()).join("migrated-v2-items.tsv") }
pub fn disabled_startup_folder() -> PathBuf { PathBuf::from(app_data()).join("DisabledStartupFolderItems") }

fn guarded_store_path(env_name: &str, default_leaf: &str) -> PathBuf {
    if let Ok(over) = std::env::var(env_name) {
        let expanded = msm_core::util::expand_env(over.trim().trim_matches('"'));
        if !expanded.trim().is_empty() && std::path::Path::new(&expanded).is_absolute() {
            let full = msm_core::paths::canonical(&expanded);
            let root = format!("{}\\", msm_core::paths::canonical(&app_data()).trim_end_matches(['\\', '/']));
            if full.to_lowercase().starts_with(&root.to_lowercase()) {
                return PathBuf::from(full);
            }
        }
    }
    PathBuf::from(app_data()).join(default_leaf)
}

pub fn known_store_path() -> PathBuf { guarded_store_path("MSM_KNOWN_STORE", "known-startup-items.tsv") }
pub fn first_seen_store_path() -> PathBuf { guarded_store_path("MSM_FIRST_SEEN_STORE", "startup-first-seen.tsv") }
pub fn denied_store_path() -> PathBuf { PathBuf::from(app_data()).join("denied-items.tsv") }
pub fn changed_at_store_path() -> PathBuf { PathBuf::from(app_data()).join("startup-changed-at.tsv") }
pub fn purge_backup_dir() -> PathBuf { PathBuf::from(app_data()).join("PurgedBackups") }

/// Install the scan hooks so core's scans see the persisted intent stores.
pub fn install_scan_hooks() {
    msm_core::hooks::install(msm_core::hooks::ScanHooks {
        is_quiet_task,
        load_disabled_items: disabled::load_disabled_items,
        load_enabled_intent: || enabled::load().into_iter().map(|row| msm_core::truth::ManagedIntentRow {
            kind: row.kind, name: row.name, scope: row.scope, command: row.command,
            location: row.location, status: row.status, target: row.target,
            arguments: row.arguments, mode: row.mode, task_location: row.task_location,
        }).collect(),
        retire_disabled_record: |id| { let _ = disabled::remove(id); },
        is_v2_migrated: |task_location| enabled::is_v2_migrated(task_location),
    });
}

fn quiet_task_cache() -> &'static Lazy<std::sync::Mutex<Option<(String, u128, Vec<String>)>>> {
    static CACHE: Lazy<std::sync::Mutex<Option<(String, u128, Vec<String>)>>> = Lazy::new(|| std::sync::Mutex::new(None));
    &CACHE
}

pub fn is_quiet_task(task_location: &str) -> bool {
    if task_location.trim().is_empty() {
        return false;
    }
    let path = enabled_store();
    let mut stamp = format!("{}|{}", path.display(), store::generation());
    match std::fs::metadata(&path) {
        Ok(meta) => {
            use std::os::windows::fs::MetadataExt;
            stamp = format!("{}|{}|{}|{}", stamp, meta.last_write_time(), meta.len(), meta.creation_time());
        }
        Err(_) => stamp.push_str("|missing"),
    }
    let now = msm_core::util::tick_ms();
    {
        let guard = quiet_task_cache().lock().unwrap();
        if let Some((stamp_at, loaded_at, tasks)) = guard.as_ref() {
            if stamp_at == &stamp && now.saturating_sub(*loaded_at) < 2000 {
                return tasks.iter().any(|t| t.eq_ignore_ascii_case(task_location));
            }
        }
    }
    let quiet: Vec<String> = enabled::load().into_iter()
        .filter(|r| eq_ci(&r.mode, "tray"))
        .map(|r| if r.task_location.trim().is_empty() { r.location } else { r.task_location })
        .collect();
    let result = quiet.iter().any(|t| t.eq_ignore_ascii_case(task_location));
    *quiet_task_cache().lock().unwrap() = Some((stamp, now, quiet));
    result
}
