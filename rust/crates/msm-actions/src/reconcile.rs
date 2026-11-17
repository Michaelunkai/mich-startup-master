//! Managed-route reconciliation (`--reconcile-managed-startups`) + BootReconciliation.

use msm_core::model::StartupItem;
use msm_core::util::q;
use msm_state::{enabled, protected_disabled, protected_quiet, store};

struct RouteCandidate {
    task_location: String,
    launch_identity: String,
    slot_identity: String,
    enabled: bool,
}

fn read_route_candidates(prefix: Option<&str>) -> Vec<RouteCandidate> {
    let agent_task = format!("{}MichStartupMasterApp", msm_core::scan::MANAGED_TASK_ROOT);
    let leaf_prefix = prefix.map(|p| p.trim().replace('/', "\\").trim_end_matches('\\').rsplit('\\').next().unwrap_or("").to_lowercase()).unwrap_or_default();
    let mut result: Vec<RouteCandidate> = Vec::new();
    for item in msm_core::scan::scan_all() {
        if !item.is_managed || !item.source.eq_ignore_ascii_case("Scheduled Task") { continue }
        let task_location = item.location.clone();
        if !msm_core::tasks::is_managed_scheduled_task_path(&task_location) || task_location.eq_ignore_ascii_case(&agent_task) { continue }
        if !leaf_prefix.is_empty()
            && !task_location.rsplit('\\').next().unwrap_or("").to_lowercase().starts_with(&leaf_prefix) {
            continue;
        }
        let Some((target, arguments)) = msm_core::humans::try_decode_tray_payload(&item.command)
            .or_else(|| msm_core::humans::try_split_command(&item.command)) else { continue };
        let launch_identity = msm_core::audit::normalize_managed_command(&target, &arguments);
        if launch_identity.trim().is_empty() { continue }
        let slot = crate::managed::canonical_managed_slot(task_location.rsplit('\\').next().unwrap_or(""));
        result.push(RouteCandidate { task_location, launch_identity, slot_identity: slot, enabled: item.enabled });
    }
    // One row per task location (enabled preferred, deterministic order).
    let mut deduped: Vec<RouteCandidate> = Vec::new();
    for candidate in result {
        match deduped.iter_mut().find(|r| r.task_location.eq_ignore_ascii_case(&candidate.task_location)) {
            Some(existing) => {
                if candidate.enabled && !existing.enabled {
                    *existing = candidate;
                }
            }
            None => deduped.push(candidate),
        }
    }
    deduped.sort_by(|a, b| a.task_location.to_lowercase().cmp(&b.task_location.to_lowercase()));
    deduped
}

struct DedupePlan {
    retired: Vec<String>,
    conflicts: Vec<String>,
}

fn plan_dedupe(candidates: &[RouteCandidate]) -> DedupePlan {
    let mut plan = DedupePlan { retired: Vec::new(), conflicts: Vec::new() };
    // Exact launch duplicates: keep the live (or deterministic first) route.
    for group_start in 0..candidates.len() {
        // C# keeper: enabled first, then alphabetically-first task location.
        let keeper_index = (group_start..candidates.len())
            .filter(|&i| !plan.retired.iter().any(|r| r.eq_ignore_ascii_case(&candidates[i].task_location)))
            .filter(|&i| candidates[i].launch_identity.eq_ignore_ascii_case(&candidates[group_start].launch_identity))
            .min_by(|&a, &b| {
                (candidates[b].enabled as u8, candidates[a].task_location.to_lowercase())
                    .cmp(&(candidates[a].enabled as u8, candidates[b].task_location.to_lowercase()))
            })
            .unwrap_or(group_start);
        let duplicates: Vec<String> = (group_start..candidates.len())
            .filter(|&i| i != keeper_index
                && candidates[i].launch_identity.eq_ignore_ascii_case(&candidates[group_start].launch_identity)
                && !plan.retired.iter().any(|r| r.eq_ignore_ascii_case(&candidates[i].task_location)))
            .map(|i| candidates[i].task_location.clone())
            .collect();
        for duplicate in duplicates {
            plan.retired.push(duplicate);
        }
    }
    // Slot identities: a sole enabled route retires disabled aliases; distinct enabled identities conflict.
    let active: Vec<&RouteCandidate> = candidates.iter()
        .filter(|r| !plan.retired.iter().any(|t| t.eq_ignore_ascii_case(&r.task_location)) && !r.slot_identity.is_empty())
        .collect();
    let mut slots: Vec<String> = active.iter().map(|r| r.slot_identity.clone()).collect();
    slots.sort_by_key(|s| s.to_lowercase());
    slots.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    for slot in slots {
        let group: Vec<&&RouteCandidate> = active.iter().filter(|r| r.slot_identity.eq_ignore_ascii_case(&slot)).collect();
        let enabled_count = group.iter().filter(|r| r.enabled).count();
        if enabled_count == 1 {
            for disabled in group.iter().filter(|r| !r.enabled) {
                if !plan.retired.iter().any(|t| t.eq_ignore_ascii_case(&disabled.task_location)) {
                    plan.retired.push(disabled.task_location.clone());
                }
            }
        } else if enabled_count > 1 {
            let identities: std::collections::BTreeSet<String> = group.iter().filter(|r| r.enabled)
                .map(|r| r.launch_identity.to_lowercase()).collect();
            if identities.len() > 1 {
                plan.conflicts.push(slot);
            }
        }
    }
    plan
}

/// C# `ReconcileManagedStartupRoutes` — `MANAGED_DEDUPE` receipt.
pub fn reconcile_managed_startup_routes(prefix: Option<&str>) -> Result<String, String> {
    if let Some(prefix) = prefix {
        let leaf = prefix.trim().replace('/', "\\");
        if leaf.contains(['\\', '/', '\0', '\r', '\n']) || leaf.len() > 160 {
            return Err("Managed-task reconciliation prefix is invalid".into());
        }
    }
    msm_state::coordinator::run(move || -> Result<String, String> {
        let candidates = read_route_candidates(prefix);
        let plan = plan_dedupe(&candidates);
        let retired = plan.retired.len();
        let snapshots = capture(&plan.retired);
        let result = (|| -> Result<(), String> {
            let mut ordered = plan.retired.clone();
            ordered.sort_by_key(|t| t.to_lowercase());
            for task_location in &ordered {
                if msm_core::tasks_com::query_task_state(task_location).is_some() {
                    msm_core::managed::run_checked("schtasks.exe", &format!("/Delete /F /TN {}", q(task_location)), 30_000)?;
                }
                enabled::remove_by_task(task_location);
                protected_quiet::unprotect_task(task_location);
                protected_disabled::unprotect_key(&format!("task|{}", task_location));
                enabled::mark_v2_migrated_checked(task_location);
            }
            Ok(())
        })();
        let compacted_enabled = enabled::dedupe_managed_intent_rows();
        let compacted_quiet = protected_quiet::dedupe_exact_routes();
        match result {
            Ok(()) => {
                let mut conflicts = plan.conflicts.clone();
                conflicts.sort_by_key(|c| c.to_lowercase());
                Ok(format!("MANAGED_DEDUPE scanned={} retired={} conflicts={} compacted_enabled={} compacted_quiet={}{}",
                    candidates.len(), retired, plan.conflicts.len(), compacted_enabled, compacted_quiet,
                    if conflicts.is_empty() { String::new() } else { format!(" conflict_slots={}", conflicts.join(",")) }))
            }
            Err(error) => {
                let rollback_errors = restore(&snapshots);
                if rollback_errors.is_empty() {
                    Err(format!("Managed duplicate repair failed; task XML and intent stores were restored: {}", error))
                } else {
                    Err(format!("Managed duplicate repair failed and rollback was incomplete: {}. Cause: {}", rollback_errors.join(" | "), error))
                }
            }
        }
    })?
}

struct ReconcileSnapshot {
    tasks: Vec<(String, bool, Option<String>)>,
    stores: Vec<crate::StoreSnapshotVec>,
}

fn capture(task_locations: &[String]) -> ReconcileSnapshot {
    ReconcileSnapshot {
        tasks: task_locations.iter().map(|location| {
            let xml = msm_core::tasks_com::try_read_task_xml(location);
            (location.clone(), xml.is_some(), xml)
        }).collect(),
        stores: vec![
            crate::StoreSnapshotVec { path: msm_state::enabled_store(), bytes: std::fs::read(msm_state::enabled_store()).ok() },
            crate::StoreSnapshotVec { path: msm_state::protected_quiet_store(), bytes: std::fs::read(msm_state::protected_quiet_store()).ok() },
            crate::StoreSnapshotVec { path: msm_state::disabled_store(), bytes: std::fs::read(msm_state::disabled_store()).ok() },
            crate::StoreSnapshotVec { path: msm_state::protected_disabled_store(), bytes: std::fs::read(msm_state::protected_disabled_store()).ok() },
            crate::StoreSnapshotVec { path: msm_state::migrated_marker_path(), bytes: std::fs::read(msm_state::migrated_marker_path()).ok() },
        ],
    }
}

fn restore(snapshot: &ReconcileSnapshot) -> Vec<String> {
    let mut errors = Vec::new();
    for (location, existed, xml) in snapshot.tasks.iter().rev() {
        if *existed {
            if let Some(xml) = xml {
                if let Err(e) = msm_core::managed::restore_task_xml(location, xml) {
                    errors.push(format!("task={}", e));
                }
            }
        } else if msm_core::tasks_com::query_task_state(location).is_some() {
            if let Err(e) = msm_core::managed::run_checked("schtasks.exe", &format!("/Delete /F /TN {}", q(location)), 30_000) {
                errors.push(format!("task-delete={}", e));
            }
        }
    }
    for store_snapshot in &snapshot.stores {
        let result = match &store_snapshot.bytes {
            Some(bytes) => msm_state::store::write_all_lines_raw(&store_snapshot.path, bytes),
            None => if store_snapshot.path.exists() { std::fs::remove_file(&store_snapshot.path).map_err(|e| e.to_string()) } else { Ok(()) },
        };
        if let Err(e) = result {
            errors.push(e);
        }
    }
    errors
}

/// `--managed-startup-dedupe-self-test` receipt (pure fixtures).
pub fn managed_startup_dedupe_self_test() -> String {
    let mut checks = 0u32;
    let mut require = |condition: bool| {
        checks += 1;
        if !condition {
            panic!("MANAGED_DEDUPE_SELF_TEST failed at {}", checks);
        }
    };
    require(crate::managed::canonical_managed_slot("OpenWhisprLauncher.exe") == "openwhispr"
        && crate::managed::canonical_managed_slot("OpenWhispr") == "openwhispr");
    let mk = |location: &str, target: &str, args: &str, enabled: bool| RouteCandidate {
        task_location: location.into(),
        launch_identity: msm_core::audit::normalize_managed_command(target, args),
        slot_identity: crate::managed::canonical_managed_slot(location.rsplit('\\').next().unwrap_or("")),
        enabled,
    };
    let exact = plan_dedupe(&[
        mk(r"\MichStartupMaster\Fixture", r"C:\Fixture\app.exe", "--ready", true),
        mk(r"\MichStartupMaster\FixtureLauncher", r"C:\Fixture\app.exe", "--ready", true),
    ]);
    require(exact.retired.len() == 1 && exact.retired[0].eq_ignore_ascii_case(r"\MichStartupMaster\FixtureLauncher") && exact.conflicts.is_empty());
    let stale_alias = plan_dedupe(&[
        mk(r"\MichStartupMaster\OpenWhisprLauncher.exe", r"C:\OpenWhispr\OpenWhisprLauncher.exe", "", true),
        mk(r"\MichStartupMaster\OpenWhispr", r"C:\OpenWhispr\CustomRuntime\OpenWhispr.exe", "", false),
    ]);
    require(stale_alias.retired.len() == 1 && stale_alias.retired[0].eq_ignore_ascii_case(r"\MichStartupMaster\OpenWhispr") && stale_alias.conflicts.is_empty());
    let conflict = plan_dedupe(&[
        mk(r"\MichStartupMaster\Dashboard", r"C:\Fixture\one.exe", "", true),
        mk(r"\MichStartupMaster\DashboardLauncher", r"C:\Fixture\two.exe", "", true),
    ]);
    require(conflict.retired.is_empty() && conflict.conflicts.len() == 1 && conflict.conflicts[0].eq_ignore_ascii_case("dashboard"));
    format!("MANAGED_DEDUPE_SELF_TEST checks={} passed={} exactOne=true staleAliasRetired=true conflictsFailClosed=true", checks, checks)
}

/// `BootReconciliation::RunOnce` — one shot per process: repair + enforce everything.
pub fn boot_reconcile_once() -> String {
    use std::sync::atomic::{AtomicBool, Ordering};
    static HAS_RUN: AtomicBool = AtomicBool::new(false);
    if HAS_RUN.swap(true, Ordering::SeqCst) {
        return "BOOT_RECONCILE already-ran".into();
    }
    msm_state::coordinator::run(move || {
        let mut results = Vec::new();
        match reconcile_managed_startup_routes(None) {
            Ok(receipt) => results.push(receipt),
            Err(e) => results.push(format!("managed-dedupe-error={}", error_kind(&e))),
        }
        results.push(protected_disabled::enforce_protected());
        results.push(protected_quiet::enforce_protected());
        results.push(crate::agent::enforce_enabled(true));
        format!("BOOT_RECONCILE {}", results.join(" | "))
    }).unwrap_or_else(|e| format!("BOOT_RECONCILE error={}", e))
}

fn error_kind(message: &str) -> String {
    // The C# receipt surfaces the exception type name; keep the closest stable token.
    let trimmed = message.split(':').next().unwrap_or(message).trim();
    if trimmed.is_empty() { "Error".into() } else { trimmed.to_string() }
}

#[allow(unused)]
fn _hint(item: StartupItem) -> bool { item.enabled }
