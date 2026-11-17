//! ProtectedQuietService — quiet (tray) intent guard. Row: `B64(task) \t B64(target) \t B64(args)`.

use crate::{protected_quiet_store, store};
use msm_core::util::{b64, eq_ci, unb64};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Row {
    pub task_location: String,
    pub target_path: String,
    pub arguments: String,
}

fn parse_rows(lines: Vec<String>) -> Vec<Row> {
    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        rows.push(Row { task_location: unb64(parts[0]), target_path: unb64(parts[1]), arguments: unb64(parts[2]) });
    }
    rows
}

fn serialize(rows: &[Row]) -> Vec<String> {
    rows.iter().map(|r| [b64(&r.task_location), b64(&r.target_path), b64(&r.arguments)].join("\t")).collect()
}

pub fn load() -> Vec<Row> {
    parse_rows(store::read_all_lines(&protected_quiet_store()))
}

fn mutate(update: impl FnOnce(Vec<Row>) -> Vec<Row>) {
    store::update_lines(&protected_quiet_store(), |lines| {
        serialize(&update(parse_rows(lines)))
    });
}

pub fn protect_task(task_location: &str, target_path: &str, arguments: &str) {
    if task_location.trim().is_empty() || target_path.trim().is_empty() {
        return;
    }
    let mut target = target_path.to_string();
    let mut args = arguments.to_string();
    msm_core::quiet_plan::normalize_persistent(&mut target, &mut args);
    let launch_identity = msm_core::audit::normalize_managed_command(&target, &args);
    let task = task_location.to_string();
    mutate(|mut rows| {
        rows.retain(|r| {
            !eq_ci(&r.task_location, &task)
                && (launch_identity.is_empty()
                    || !eq_ci(&msm_core::audit::normalize_managed_command(&r.target_path, &r.arguments), &launch_identity))
        });
        rows.push(Row { task_location: task.to_string(), target_path: target.clone(), arguments: args.clone() });
        rows
    });
}

pub fn unprotect_task(task_location: &str) {
    if task_location.trim().is_empty() {
        return;
    }
    let task = task_location.to_string();
    mutate(|rows| rows.into_iter().filter(|r| !eq_ci(&r.task_location, &task)).collect());
}

pub fn is_protected(task_location: &str) -> bool {
    if task_location.trim().is_empty() {
        return false;
    }
    load().iter().any(|r| eq_ci(&r.task_location, task_location))
}

/// Compact stale quiet-intent duplicates (enabled task preferred). Returns rows removed.
pub fn dedupe_exact_routes() -> usize {
    let mut removed = 0usize;
    mutate(|rows| {
        let mut ordered = rows.clone();
        ordered.sort_by(|a, b| {
            let rank = |r: &Row| match crate::enabled::query_task_state(&r.task_location).as_deref() {
                Some("enabled") => 2,
                Some(_) => 1,
                None => 0,
            };
            rank(b).cmp(&rank(a)).then_with(|| a.task_location.to_lowercase().cmp(&b.task_location.to_lowercase()))
        });
        let mut kept: Vec<Row> = Vec::new();
        let mut locations: Vec<String> = Vec::new();
        let mut identities: Vec<String> = Vec::new();
        for row in ordered {
            let location = row.task_location.clone();
            let identity = msm_core::audit::normalize_managed_command(&row.target_path, &row.arguments);
            let duplicate_location = !location.trim().is_empty() && locations.iter().any(|l| eq_ci(l, &location));
            let duplicate_launch = !identity.trim().is_empty() && identities.iter().any(|i| eq_ci(i, &identity));
            if duplicate_location || duplicate_launch {
                removed += 1;
                continue;
            }
            if !location.trim().is_empty() { locations.push(location); }
            if !identity.trim().is_empty() { identities.push(identity); }
            kept.push(row);
        }
        kept.sort_by(|a, b| a.task_location.to_lowercase().cmp(&b.task_location.to_lowercase()));
        kept
    });
    removed
}

/// C# `EnforceProtected` receipt: `ENFORCE_QUIET protected=N actions=N started=N failures=N normalized=0|1`.
pub fn enforce_protected() -> String {
    crate::protected_quiet_enforce::enforce()
}

pub fn store_path() -> &'static Path {
    Path::new("")
}
