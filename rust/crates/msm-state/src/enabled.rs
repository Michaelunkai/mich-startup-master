//! EnabledStartupService — the enabled-intent manifest.
//! Row: 10 B64 columns: Kind, Name, Scope, Command, Location, Status, Target, Arguments, Mode, TaskLocation.

use crate::{enabled_store, migrated_marker_path, store};
use msm_core::util::{b64, eq_ci, unb64};

#[derive(Debug, Clone, Default)]
pub struct Row {
    pub kind: String,
    pub name: String,
    pub scope: String,
    pub command: String,
    pub location: String,
    pub status: String,
    pub target: String,
    pub arguments: String,
    pub mode: String,
    pub task_location: String,
}

pub fn load() -> Vec<Row> {
    let mut list = Vec::new();
    for line in store::read_all_lines(&enabled_store()) {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 10 {
            continue;
        }
        list.push(Row {
            kind: unb64(parts[0]), name: unb64(parts[1]), scope: unb64(parts[2]), command: unb64(parts[3]),
            location: unb64(parts[4]), status: unb64(parts[5]), target: unb64(parts[6]), arguments: unb64(parts[7]),
            mode: unb64(parts[8]), task_location: unb64(parts[9]),
        });
    }
    list
}

fn serialize(rows: &[Row]) -> Vec<String> {
    rows.iter().map(|r| [
        b64(&r.kind), b64(&r.name), b64(&r.scope), b64(&r.command), b64(&r.location), b64(&r.status),
        b64(&r.target), b64(&r.arguments), b64(&r.mode), b64(&r.task_location),
    ].join("\t")).collect()
}

fn mutate(update: impl FnOnce(Vec<Row>) -> Vec<Row>) {
    store::update_lines(&enabled_store(), |lines| {
        let mut rows: Vec<Row> = Vec::new();
        for line in lines {
            if line.trim().is_empty() { continue; }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 10 { continue; }
            rows.push(Row {
                kind: unb64(parts[0]), name: unb64(parts[1]), scope: unb64(parts[2]), command: unb64(parts[3]),
                location: unb64(parts[4]), status: unb64(parts[5]), target: unb64(parts[6]), arguments: unb64(parts[7]),
                mode: unb64(parts[8]), task_location: unb64(parts[9]),
            });
        }
        serialize(&update(rows))
    });
}

fn row_key(row: &Row) -> String {
    if !row.task_location.trim().is_empty() {
        format!("task|{}", row.task_location)
    } else {
        format!("{}|{}|{}", row.kind, row.location, row.name)
    }
}

fn managed_launch_identity(row: &Row) -> String {
    if !eq_ci(&row.kind, "managed-task") {
        return String::new();
    }
    let task_location = if row.task_location.trim().is_empty() { row.location.clone() } else { row.task_location.clone() };
    if eq_ci(&task_location, &format!("{}MichStartupMasterApp", msm_core::scan::MANAGED_TASK_ROOT)) {
        return String::new();
    }
    if row.target.trim().is_empty() {
        return String::new();
    }
    msm_core::audit::normalize_managed_command(&row.target, &row.arguments)
}

/// C# `Upsert` — replace by key and launch identity, then append.
pub fn upsert(row: &Row) {
    let _ = std::fs::create_dir_all(crate::app_data());
    let key = row_key(row);
    let launch_identity = managed_launch_identity(row);
    let new_row = row.clone();
    mutate(|mut rows| {
        rows.retain(|r| {
            !eq_ci(&row_key(r), &key)
                && (launch_identity.is_empty() || !eq_ci(&managed_launch_identity(r), &launch_identity))
        });
        rows.push(new_row.clone());
        rows
    });
}

pub fn remove_by_task(task_location: &str) {
    if task_location.trim().is_empty() {
        return;
    }
    mutate(|rows| rows.into_iter().filter(|r| !eq_ci(&r.task_location, task_location)).collect());
}

/// `UpsertFromItemChecked` first drops the row it replaces.
pub fn remove_item_for_upsert(item: &msm_core::model::StartupItem) {
    remove_item(item);
}

/// C# `Remove(item)` — only the rows matching the item's own kind.
pub fn remove_item(item: &msm_core::model::StartupItem) {
    let id = &item.id;
    if id.starts_with("task|") {
        remove_by_task(&item.location);
        return;
    }
    if id.starts_with("reg|") || id.starts_with("disabled|reg|") {
        let name = item.name.clone();
        let location = item.location.clone();
        mutate(|rows| rows.into_iter().filter(|r| {
            !(r.kind == "registry" && eq_ci(&r.name, &name) && eq_ci(&r.location, &location))
        }).collect());
        return;
    }
    if id.starts_with("folder|") || id.starts_with("disabled|folder|") {
        let command = item.command.clone();
        mutate(|rows| rows.into_iter().filter(|r| !(r.kind == "folder" && eq_ci(&r.command, &command))).collect());
        return;
    }
    if id.starts_with("service|") || id.starts_with("driver|") || id.starts_with("disabled|service|") || id.starts_with("disabled|driver|") {
        let name = item.name.clone();
        mutate(|rows| rows.into_iter().filter(|r| !((r.kind == "service" || r.kind == "driver") && eq_ci(&r.name, &name))).collect());
        return;
    }
}

/// C# `QueryTaskState` — COM task state: Some(enabled/disabled) | None (missing).
pub fn query_task_state(task_location: &str) -> Option<String> {
    msm_core::tasks_com::query_task_state(task_location)
}

/// `TryParseExactServiceIntent` — `name \t start` with start in 0..=3.
pub fn try_parse_exact_service_intent(status: &str) -> Option<(String, i64)> {
    let meta: Vec<&str> = status.split('\t').collect();
    if meta.len() < 2 || meta[0].trim().is_empty() {
        return None;
    }
    meta[1].trim().parse::<i64>().ok().filter(|start| (0..=3).contains(start)).map(|start| (meta[0].to_string(), start))
}

/// `MarkV2MigratedChecked`.
pub fn mark_v2_migrated_checked(task_location: &str) {
    if task_location.trim().is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(crate::app_data());
    store::update_lines(&migrated_marker_path(), |mut lines| {
        if !lines.iter().any(|l| l.trim().trim_start_matches('\u{feff}').eq_ignore_ascii_case(task_location)) {
            lines.push(task_location.to_string());
        }
        lines
    });
}

pub fn mark_v2_migrated(task_location: &str) {
    mark_v2_migrated_checked(task_location);
}

pub fn is_v2_migrated(task_location: &str) -> bool {
    if task_location.trim().is_empty() {
        return false;
    }
    store::read_all_lines(&migrated_marker_path()).iter()
        .any(|l| l.trim().trim_start_matches('\u{feff}').eq_ignore_ascii_case(task_location))
}

/// `DedupeManagedIntentRows` — compact by launch identity (enabled task preferred).
pub fn dedupe_managed_intent_rows() -> usize {
    let mut removed = 0usize;
    mutate(|rows| {
        let mut managed: Vec<Row> = rows.iter().filter(|r| !managed_launch_identity(r).trim().is_empty()).cloned().collect();
        managed.sort_by(|a, b| {
            let rank = |r: &Row| match query_task_state(if r.task_location.trim().is_empty() { &r.location } else { &r.task_location }).as_deref() {
                Some("enabled") => 2,
                Some(_) => 1,
                None => 0,
            };
            let a_loc = if a.task_location.trim().is_empty() { &a.location } else { &a.task_location };
            let b_loc = if b.task_location.trim().is_empty() { &b.location } else { &b.task_location };
            rank(b).cmp(&rank(a)).then_with(|| a_loc.to_lowercase().cmp(&b_loc.to_lowercase()))
        });
        let mut keep: Vec<Row> = Vec::new();
        let mut identities: Vec<String> = Vec::new();
        for row in managed {
            let identity = managed_launch_identity(&row);
            if identities.iter().any(|i| eq_ci(i, &identity)) {
                removed += 1;
                continue;
            }
            identities.push(identity);
            keep.push(row);
        }
        rows.into_iter().filter(|r| managed_launch_identity(r).trim().is_empty() || keep.iter().any(|k| row_key(k) == row_key(r))).collect()
    });
    removed
}

/// `--list-managed` JSON.
pub fn to_json() -> String {
    let rows = load();
    let mut out = String::from("[");
    for (i, r) in rows.iter().enumerate() {
        if i > 0 { out.push(','); }
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\r', " ").replace('\n', " ");
        out.push_str(&format!("{{\"kind\":\"{}\",\"name\":\"{}\",\"target\":\"{}\",\"mode\":\"{}\",\"task\":\"{}\"}}",
            esc(&r.kind), esc(&r.name), esc(&r.target), esc(&r.mode), esc(&r.task_location)));
    }
    out.push(']');
    out
}
