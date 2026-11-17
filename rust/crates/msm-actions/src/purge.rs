//! Purge — remove an application from EVERY startup surface forever and make
//! it never render again. Two-phase: structural disable (the existing exact,
//! reversible machinery) then total store cleanup + quarantine + deny-list.
//! Every purge is backed up and restorable from Tools → Hidden.

use msm_core::denykey;
use msm_core::model::StartupItem;
use msm_state::{denied, disabled, enabled, protected_disabled, protected_quiet, store};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct PurgedTask {
    pub location: String,
    pub xml: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PurgedFile {
    /// Path the disabled record now points at (inside the quarantine folder).
    pub quarantined: String,
    /// Path the file must return to if this purge is undone.
    pub original: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PurgeBackup {
    pub ticks: u128,
    pub label: String,
    /// Exact disabled-store lines that recreate the disabled records on undo.
    pub disabled_lines: Vec<String>,
    /// Scheduled tasks deleted outright; re-registered from XML on undo.
    pub tasks: Vec<PurgedTask>,
    /// Quarantined startup-folder files moved under the purge backup dir.
    pub files: Vec<PurgedFile>,
    /// Deny keys written for this purge (lifted on undo).
    pub deny_keys: Vec<String>,
    /// Folders each still disabled at the service level (Start=4).
    pub services: Vec<String>,
}

pub struct PurgeReceipt {
    pub label: String,
    pub removed: usize,
    pub backup_id: String,
    pub errors: Vec<String>,
}

fn backup_dir() -> std::path::PathBuf {
    let dir = msm_state::purge_backup_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Purge every route of an application everywhere.
pub fn purge_all(routes: &[StartupItem], label: &str) -> Result<PurgeReceipt, String> {
    let owned: Vec<StartupItem> = routes.to_vec();
    if owned.is_empty() {
        return Err("Nothing to purge.".into());
    }
    let mut receipt = PurgeReceipt { label: label.to_string(), removed: 0, backup_id: String::new(), errors: Vec::new() };
    let mut tasks: Vec<PurgedTask> = Vec::new();
    let mut files: Vec<PurgedFile> = Vec::new();
    let mut services: Vec<String> = Vec::new();

    // Phase 1 — structural disable per route (exact snapshot + rollback inside).
    for route in &owned {
        match super::disable_forced(route) {
            Ok(()) => receipt.removed += 1,
            Err(e) => receipt.errors.push(format!("{}: {}", route.human_name(), e)),
        }
    }

    // Phase 2 — deepen to total removal for the kinds that keep a live artifact.
    for route in &owned {
        let lower = route.id.to_lowercase();
        if lower.starts_with("task|") || lower.starts_with("disabled|task|") {
            if let Some(xml) = msm_core::tasks_com::try_read_task_xml(&route.location) {
                tasks.push(PurgedTask { location: route.location.clone(), xml });
                if let Err(e) = msm_core::managed::run_checked("schtasks.exe",
                    &format!("/Delete /F /TN {}", msm_core::util::q(&route.location)), 30_000) {
                    receipt.errors.push(format!("delete task {}: {}", route.location, e));
                }
            } else if let Err(e) = msm_core::managed::run_checked("schtasks.exe",
                &format!("/Delete /F /TN {}", msm_core::util::q(&route.location)), 30_000) {
                receipt.errors.push(format!("delete task {}: {}", route.location, e));
            }
            if !route.location.trim().is_empty() {
                protected_quiet::unprotect_task(&route.location);
            }
        }
        if lower.starts_with("service|") || lower.starts_with("driver|")
            || lower.starts_with("disabled|service|") || lower.starts_with("disabled|driver|") {
            let name = super::service_name_from_item(route);
            if !name.is_empty() {
                // Hard-off (4): zero leftover autostart even on demand paths we own.
                let _ = msm_core::services::set_service_start_value(&name, 4);
                services.push(name);
            }
        }
        enabled::remove_item(route);
        enabled::remove_item_for_upsert(route);
        protected_disabled::unprotect(route);
    }

    // Phase 3 — capture the disabled records our disables created, then wipe them.
    let disabled_now = disabled::load_disabled_items();
    let mut deny_keys: Vec<String> = Vec::new();
    let mut disabled_lines: Vec<String> = Vec::new();
    let all_store_lines = store::read_all_lines(&msm_state::disabled_store());
    for record in &disabled_now {
        let matches = owned.iter().any(|route| {
            route.id.eq_ignore_ascii_case(&record.id)
                || (route.command.trim().eq_ignore_ascii_case(record.command.trim())
                    && route.name.trim().eq_ignore_ascii_case(record.name.trim()))
        });
        if !matches { continue; }
        if let Some(line) = all_store_lines.iter().find(|l| line_for_record(l, record)) {
            disabled_lines.push(line.clone());
        }
        disabled::remove(&record.id);
        // Quarantine folder files out of the live quarantine dir into the purge vault.
        let rl = record.id.to_lowercase();
        if rl.starts_with("disabled|folder|") {
            let original = record.status.clone();
            let quarantined = record.command.clone();
            if std::path::Path::new(&quarantined).is_file() {
                let leaf = quarantined.rsplit('\\').next().unwrap_or("startup-file");
                let vault = backup_dir().join(format!("{}.{}", leaf, msm_core::util::tick_ms()));
                if std::fs::rename(&quarantined, &vault).is_ok() {
                    files.push(PurgedFile { quarantined: vault.to_string_lossy().into_owned(), original });
                }
            }
        }
        deny_keys.extend(denykey::all_keys(record));
    }
    for route in &owned {
        deny_keys.extend(denykey::all_keys(route));
    }
    deny_keys.sort();
    deny_keys.dedup();
    for key in &deny_keys {
        denied::add(key, label);
    }

    // Phase 4 — durable backup file (restorable from Tools → Hidden).
    let ticks = msm_core::util::tick_ms();
    let backup = PurgeBackup { ticks, label: label.to_string(), disabled_lines, tasks, files, deny_keys, services };
    let backup_id = format!("purged-{}", ticks);
    let path = backup_dir().join(format!("{}.json", backup_id));
    serde_json::to_string(&backup)
        .map_err(|e| format!("Purge completed but the backup could not be written: {}", e))
        .and_then(|json| std::fs::write(&path, json).map_err(|e| format!("Purge completed but the backup could not be written: {}", e)))?;
    receipt.backup_id = backup_id;
    // Phase 5: permanent erase of everything left in Windows (folders, services, registry).
    for line in super::erase::erase_traces(&owned) {
        if line.starts_with("FAILED") { receipt.errors.push(line); } else { receipt.removed += 1; }
    }
    if receipt.removed == 0 && receipt.errors.len() == owned.len() {
        return Err(format!("Nothing could be purged. {}", receipt.errors.join(" | ")));
    }
    Ok(receipt)
}

/// Does this raw store line represent this hydrated record? (id column match)
fn line_for_record(line: &str, record: &StartupItem) -> bool {
    line.split('\t').next().map(|first| first.trim().eq_ignore_ascii_case(&record.id)).unwrap_or(false)
}

pub fn list_backups() -> Vec<PurgeBackup> {
    let mut backups: Vec<PurgeBackup> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(backup_dir()) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(backup) = serde_json::from_slice::<PurgeBackup>(&bytes) {
                        backups.push(backup);
                    }
                }
            }
        }
    }
    backups.sort_by_key(|b| std::cmp::Reverse(b.ticks));
    backups
}

/// Undo a purge: disabled records return (as Disabled), tasks re-register from
/// XML, quarantined files return to the quarantine dir, denials lift.
pub fn restore(backup_id: &str) -> Result<String, String> {
    let path = backup_dir().join(format!("{}.json", backup_id.trim()));
    let bytes = std::fs::read(&path).map_err(|e| format!("Backup not found: {}", e))?;
    let backup: PurgeBackup = serde_json::from_slice(&bytes).map_err(|e| format!("Backup unreadable: {}", e))?;
    for task in &backup.tasks {
        msm_core::managed::restore_task_xml(&task.location, &task.xml)?;
    }
    for file in &backup.files {
        if std::path::Path::new(&file.quarantined).is_file() {
            let _ = std::fs::rename(&file.quarantined, &file.original);
        }
    }
    if !backup.disabled_lines.is_empty() {
        store::update_lines(&msm_state::disabled_store(), |mut lines| {
            for line in &backup.disabled_lines {
                if !lines.iter().any(|existing| existing == line) {
                    lines.push(line.clone());
                }
            }
            lines
        });
    }
    for key in &backup.deny_keys {
        denied::remove(key);
    }
    let _ = std::fs::remove_file(&path);
    Ok(backup.label)
}

/// Hide an item without structural changes (visibility deny-list only).
pub fn hide(routes: &[StartupItem], label: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for route in routes {
        keys.extend(denykey::all_keys(route));
    }
    keys.sort();
    keys.dedup();
    for key in &keys {
        denied::add(key, label);
    }
    keys
}
