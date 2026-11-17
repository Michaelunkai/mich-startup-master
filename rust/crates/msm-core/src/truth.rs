//! Truth evidence: rebuild VerifiedState per source with exact readback for tasks.

use crate::model::{is_direct_configured_startup_source, registration_verdict, StartupItem};
use crate::util::eq_ci;

pub struct ManagedIntentRow {
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

impl ManagedIntentRow {
    pub fn task_name(&self) -> String {
        if !self.name.trim().is_empty() { self.name.clone() } else { self.task_location.trim_start_matches('\\').to_string() }
    }
}

/// C# `ApplyTruthEvidence` (task readback through the exact COM readback helper).
pub fn apply_truth_evidence(items: &mut [StartupItem], intent: &[ManagedIntentRow]) {
    // One COM walk serves every task readback (per-task connections stall the scan).
    let task_locations: Vec<String> = items.iter()
        .filter(|item| item.id.starts_with("task|") && item.source == "Scheduled Task")
        .map(|item| item.location.clone())
        .collect();
    let readback = crate::tasks_com::readback_tasks_batch(&task_locations);
    for item in items.iter_mut() {
        item.verified_state = "Unknown".into();
        item.evidence_reason = "Exact registration, target and authority readback is not fully verified".into();
        item.presentation_state = "Needs verification".into();
        item.presentation_reason = "Configured launch mode does not prove startup window or tray presentation".into();
        if item.popup_label() == "N/A" {
            item.presentation_state = "Unsupported".into();
            item.presentation_reason = "This startup surface has no application presentation contract".into();
        }
        if item.id.starts_with("error|") {
            item.evidence_reason = "The provider failed visibly; no startup-state claim is possible until refresh succeeds".into();
            continue;
        }
        if item.id.starts_with("disabled|") {
            item.verified_state = "Disabled".into();
            item.evidence_reason = "Startup Master preserved this disabled route and validates its exact restore state before any mutation".into();
            continue;
        }
        if item.id.starts_with("legacy|") || item.source == "Legacy v2" {
            item.evidence_reason = "Legacy intent is retained for recovery but no live Windows startup registration exists".into();
            continue;
        }
        if item.source == "Startup Command" {
            item.evidence_reason = if !item.approval_root.trim().is_empty() {
                "Win32_StartupCommand and Explorer StartupApproved are compatibility projections; their state can be stale or conflict with another app-owned startup route"
            } else {
                "Win32_StartupCommand exposed a launch command whose authoritative registration was not enumerable; configuration state is unknown"
            }.into();
            continue;
        }
        if item.source == "Startup Approval" || item.source == "Stale Startup Metadata" {
            item.evidence_reason = "Explorer StartupApproved is metadata, not an independently firing startup registration".into();
            continue;
        }
        if item.source == "Packaged Startup Task" {
            item.verified_state = if item.enabled { "Enabled" } else { "Disabled" }.into();
            item.evidence_reason = "Fresh AppModel StartupTask State read from the packaged startup authority; execution at the next boot/sign-in is not yet observed".into();
            continue;
        }
        if item.source != "Scheduled Task" || !item.id.starts_with("task|") {
            if is_direct_configured_startup_source(&item.source) {
                item.verified_state = if item.enabled { "Enabled" } else { "Disabled" }.into();
                item.evidence_reason = "Fresh configured state read from the owning Windows startup surface; execution at the next boot/sign-in is not yet observed".into();
            } else if item.source == "Winlogon Notification" {
                item.evidence_reason = "Legacy Winlogon notification packages are ignored on current Windows; retained as compatibility metadata".into();
            } else if item.source == "AppInit DLLs" {
                item.evidence_reason = "AppInit loading is conditional and is disabled by Windows when Secure Boot is enabled".into();
            } else {
                item.evidence_reason = "This registered extension is conditional or demand-loaded; registration alone does not mean it runs at every boot/sign-in".into();
            }
            continue;
        }
        apply_task_truth(item, intent, &readback);
    }
}

fn apply_task_truth(item: &mut StartupItem, intent: &[ManagedIntentRow], readback: &std::collections::HashMap<String, (String, String, bool, Vec<bool>)>) {
    match readback_task_truth(item, intent, readback) {
        Ok((verdict, reason)) => {
            item.verified_state = verdict.to_string();
            item.evidence_reason = reason;
        }
        Err(reason) => {
            item.verified_state = "Unknown".into();
            item.evidence_reason = format!("Readback unavailable: {}", reason);
        }
    }
}

fn readback_task_truth(item: &StartupItem, intent: &[ManagedIntentRow], readback: &std::collections::HashMap<String, (String, String, bool, Vec<bool>)>) -> Result<(&'static str, String), String> {
    let Some((execute, arguments, task_enabled, triggers)) = readback.get(&item.location) else {
        return Err("Readback unavailable: GetTask failed".into());
    };
    let (execute, arguments) = (execute.clone(), arguments.clone());
    let normalized_execute = crate::tasks::normalize_scheduled_task_executable(&execute);
    let Some((scanned_execute, scanned_arguments)) = crate::humans::try_split_command(&item.command) else {
        return Ok(("Drifted", "Task action changed between enumeration and readback".into()));
    };
    if !task_executable_paths_equal(&normalized_execute, &scanned_execute) || arguments != scanned_arguments {
        return Ok(("Drifted", "Task action changed between enumeration and readback".into()));
    }
    let (mut target, mut target_arguments) = match crate::humans::try_decode_tray_payload(&item.command) {
        Some(pair) => pair,
        None => (normalized_execute.clone(), arguments.clone()),
    };
    target = crate::util::expand_env(&target);
    let normalized_execute = crate::util::expand_env(&normalized_execute);
    if !std::path::Path::new(&target).is_absolute() {
        target = crate::paths::canonical(&target);
    }
    if !std::path::Path::new(&normalized_execute).is_absolute() {
        return Ok(("Drifted", "Exact registered launcher or target is not an absolute path".into()));
    }
    let target_probe_allowed = crate::paths::is_safe_system_volume_file_probe(&target)
        && crate::paths::is_safe_system_volume_file_probe(&normalized_execute);
    if target_probe_allowed && (!std::path::Path::new(&target).is_file() || !std::path::Path::new(&normalized_execute).is_file()) {
        return Ok(("Drifted", "Exact registered launcher or target is missing or cannot be read".into()));
    }
    let expected: Vec<&ManagedIntentRow> = intent.iter()
        .filter(|r| eq_ci(if r.task_location.trim().is_empty() { &r.location } else { &r.task_location }, &item.location))
        .collect();
    if expected.len() > 1 {
        return Err("Contradictory managed intent records".into());
    }
    if let Some(expected) = expected.first() {
        if !managed_intent_matches(expected, &target, &target_arguments) {
            return Ok(("Drifted", "Live task target or arguments contradict managed intent".into()));
        }
    }
    let active_trigger = triggers.iter().any(|enabled| *enabled);
    let enabled = *task_enabled && active_trigger;
    if enabled != item.enabled {
        return Ok(("Drifted", "Task enabled flag or applicable logon trigger contradicts inventory".into()));
    }
    let verdict = registration_verdict(true, true, true, true, item.enabled, enabled);
    let reason = if target_probe_allowed {
        "Fresh exact task action, target, arguments and applicable startup trigger readback; execution at next sign-in remains unverified"
    } else {
        "Fresh exact task action, arguments, enabled flag and applicable startup trigger readback; target availability outside the local system volume was not synchronously probed"
    }.to_string();
    Ok((verdict, reason))
}

pub fn task_executable_paths_equal(left: &str, right: &str) -> bool {
    let mut left = crate::tasks::normalize_scheduled_task_executable(left);
    let mut right = crate::tasks::normalize_scheduled_task_executable(right);
    left = crate::util::expand_env(&left).replace('/', "\\");
    right = crate::util::expand_env(&right).replace('/', "\\");
    if std::path::Path::new(&left).is_absolute() {
        left = crate::paths::canonical(&left);
    }
    if std::path::Path::new(&right).is_absolute() {
        right = crate::paths::canonical(&right);
    }
    left.trim_end_matches('\\').eq_ignore_ascii_case(right.trim_end_matches('\\'))
}

pub fn managed_intent_matches(expected: &ManagedIntentRow, target: &str, arguments: &str) -> bool {
    let mut intended_target = expected.target.clone();
    let mut intended_arguments = expected.arguments.clone();
    let mut observed_target = target.to_string();
    let mut observed_arguments = arguments.to_string();
    crate::quiet_plan::normalize_persistent(&mut intended_target, &mut intended_arguments);
    crate::quiet_plan::normalize_persistent(&mut observed_target, &mut observed_arguments);
    intended_target.eq_ignore_ascii_case(&observed_target) && intended_arguments == observed_arguments
}
