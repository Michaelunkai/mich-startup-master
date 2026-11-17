//! Quiet-guard enforcement engine (spawns the boot repair used by EnforceProtected).

use crate::protected_quiet::load;
use msm_core::tasks_com::query_task_state;
use msm_core::util::q;

fn task_action_matches(task_location: &str, expected_execute: &str, expected_arguments: &str) -> bool {
    let Some(xml) = msm_core::tasks_com::try_read_task_xml(task_location) else { return false };
    let command = extract_xml_element(&xml, "Command");
    let arguments = extract_xml_element(&xml, "Arguments");
    let decode = |value: String| value.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
    decode(command).eq_ignore_ascii_case(expected_execute)
        && decode(arguments).trim() == expected_arguments.trim()
}

fn extract_xml_element(xml: &str, element: &str) -> String {
    let pattern = format!("<{}>(?<v>.*?)</{}>", element, element);
    regex::Regex::new(&pattern).ok()
        .and_then(|re| re.captures(xml))
        .map(|c| c.name("v").map(|m| m.as_str().trim().to_string()).unwrap_or_default())
        .unwrap_or_default()
}

fn is_quiet_route_running(target_path: &str, arguments: &str) -> bool {
    if msm_core::runtime::is_process_running(target_path) {
        return true;
    }
    for process in msm_core::runtime::snapshot_processes(800) {
        let Some((wrapper_target, wrapper_arguments)) = msm_core::humans::try_decode_tray_payload(&process.command_line) else { continue };
        if normalize_path(&wrapper_target) == normalize_path(target_path) && collapse_spaces(&wrapper_arguments) == collapse_spaces(arguments) {
            return true;
        }
    }
    false
}

fn normalize_path(value: &str) -> String {
    match msm_core::paths::canonical(&msm_core::util::expand_env(value.trim().trim_matches('"'))) {
        full if std::path::Path::new(value.trim().trim_matches('"')).is_absolute() => full.trim_end_matches(['\\', '/']).to_string(),
        _ => value.trim().trim_matches('"').trim_end_matches(['\\', '/']).to_string(),
    }
}

fn collapse_spaces(value: &str) -> String {
    regex::Regex::new(r"\s+").unwrap().replace_all(value, " ").trim().to_string()
}

/// C# `ProtectedQuietService::EnforceProtected` (re-register + immediate run when missing).
pub fn enforce() -> String {
    let mut actions = 0usize;
    let mut started = 0usize;
    let mut failures = 0usize;
    let mut store_changed = false;
    let rows = load();
    let row_count = rows.len();
    for mut row in rows {
        let mut target = row.target_path.clone();
        let mut arguments = row.arguments.clone();
        if msm_core::quiet_plan::normalize_persistent(&mut target, &mut arguments) {
            store_changed = true;
            row.target_path = target.clone();
            row.arguments = arguments.clone();
        }
        if std::path::Path::new(&row.target_path).is_file() {
            let before_state = query_task_state(&row.task_location);
            let (execute, action_args) = msm_core::managed::build_managed_action(&row.target_path, &row.arguments, true);
            let action_was_correct = before_state.is_some() && task_action_matches(&row.task_location, &execute, &action_args);
            if let Err(e) = msm_core::managed::register_logon_task_at(&row.task_location, &execute, &action_args, true) {
                let _ = e;
                failures += 1;
                continue;
            }
            actions += 1;
            if (before_state.is_none() || before_state.as_deref() == Some("disabled") || !action_was_correct)
                && !is_quiet_route_running(&row.target_path, &row.arguments) {
                let _ = run_task(&row.task_location);
                started += 1;
            }
        } else {
            failures += 1;
        }
    }
    if store_changed {
        for row in load() {
            let mut target = row.target_path.clone();
            let mut arguments = row.arguments.clone();
            msm_core::quiet_plan::normalize_persistent(&mut target, &mut arguments);
        }
    }
    format!("ENFORCE_QUIET protected={} actions={} started={} failures={} normalized={}", row_count, actions, started, failures, if store_changed { 1 } else { 0 })
}

fn run_task(task_location: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new("schtasks.exe")
        .args(["/Run", "/TN", &q(task_location)])
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}
