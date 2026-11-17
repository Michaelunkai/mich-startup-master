//! Managed startup lifecycle: add/edit/remove/toggle managed logon tasks,
//! duplicate reconciliation, v2 migration.

use msm_core::managed::{build_managed_action, is_supported_startup_target, register_logon_task_at};
use msm_core::model::StartupItem;
use msm_core::util::{b64, eq_ci, q, unb64};
use msm_state::{disabled, enabled, protected_disabled, protected_quiet, store};

pub const MANAGED_TASK_ROOT: &str = msm_core::scan::MANAGED_TASK_ROOT;

/// C# `CanonicalManagedSlot`.
pub fn canonical_managed_slot(task_name: &str) -> String {
    let mut value = task_name.trim().replace('/', "\\").trim_end_matches('\\').to_string();
    if let Some(index) = value.rfind('\\') {
        value = value[index + 1..].to_string();
    }
    let extension = std::path::Path::new(&value).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ["exe", "com", "lnk"].contains(&extension.as_str()) {
        value = value[..value.len() - extension.len() - 1].to_string();
    }
    let re = regex::Regex::new(r"(?i)(?:[\s._-]*(?:launcher|launch))+$").unwrap();
    value = re.replace(&value, "").into_owned();
    let keep: String = value.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    keep.to_lowercase()
}

fn managed_task_leaf(task_location: &str) -> String {
    let value = task_location.trim().replace('/', "\\").trim_end_matches('\\').to_string();
    value.rsplit('\\').next().unwrap_or("").to_string()
}

/// C# `ChooseManagedTaskName` — collision-safe identity.
pub fn choose_managed_task_name(safe_base: &str, target_path: &str, arguments: &str, exists: impl Fn(&str) -> bool) -> Result<String, String> {
    let base = format!("{}{}", MANAGED_TASK_ROOT, safe_base);
    if !exists(&base) {
        return Ok(safe_base.to_string());
    }
    let fingerprint = msm_core::audit::normalize_managed_command(target_path, arguments);
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(fingerprint.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    let suffix = hex[..10].to_string();
    let mut stem = safe_base.to_string();
    if stem.len() > 145 {
        stem = stem[..145].trim_end_matches([' ', '.']).to_string();
    }
    let candidate = format!("{}-{}", stem, suffix);
    if !exists(&format!("{}{}", MANAGED_TASK_ROOT, candidate)) {
        return Ok(candidate);
    }
    for i in 2..10000 {
        let numbered = format!("{}-{}", candidate, i);
        if !exists(&format!("{}{}", MANAGED_TASK_ROOT, numbered)) {
            return Ok(numbered);
        }
    }
    Err(format!("Could not allocate a unique managed startup identity for {}", safe_base))
}

fn sanitize_name(name: &str) -> String {
    let mut safe: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '_' || *c == '.' || *c == '-').collect();
    safe = safe.trim().to_string();
    if safe.is_empty() { safe = "StartupApp".into() }
    let ws = regex::Regex::new(r"\s+").unwrap();
    safe = ws.replace_all(&safe, " ").into_owned();
    safe = safe.trim_matches([' ', '.']).to_string();
    if safe.is_empty() { safe = "StartupApp".into() }
    if safe.len() > 160 { safe = safe[..160].trim_end_matches([' ', '.']).to_string(); }
    safe
}

fn can_upsert_task_state(state: Option<&str>) -> bool {
    state != Some("disabled")
}

/// C# `AddManagedStartup` — one transactional managed-task lifecycle.
pub fn add_managed_startup(name: &str, target_path: &str, arguments: &str, tray_mode: bool, _no_delay: bool) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("Name is required".into());
    }
    if target_path.trim().is_empty() || !std::path::Path::new(target_path).is_file() {
        return Err(format!("Application not found: {}", target_path));
    }
    if !is_supported_startup_target(target_path) {
        return Err("Choose a supported startup target: .exe, .com, .lnk, .cmd, .bat, .ps1, .vbs, .vbe, .js, .jse, .wsf, .wsh, .py, or .pyw".into());
    }
    msm_state::coordinator::run(move || add_managed_startup_locked(name, target_path, arguments, tray_mode))?
}

/// Paste-and-go: a full program path (optionally with arguments, quoted or
/// bare, env vars allowed) becomes a managed logon startup with an automatic
/// name — the rest is done for you. Runs exactly once at every sign-in.
pub fn add_startup_from_paste(pasted: &str) -> Result<(String, String), String> {
    let trimmed = pasted.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return Err("Paste the full path of the program to start at sign-in.".into());
    }
    let expanded = msm_core::util::expand_env(trimmed);
    let (path_part, arg_part) = if std::path::Path::new(&expanded).is_file() {
        (expanded.clone(), String::new())
    } else if let Some((head, tail)) = expanded.split_once(' ') {
        let head = head.trim().trim_matches('"');
        if std::path::Path::new(head).is_file() {
            (head.to_string(), tail.trim().to_string())
        } else {
            (expanded.clone(), String::new())
        }
    } else {
        (expanded.clone(), String::new())
    };
    if !std::path::Path::new(&path_part).is_file() {
        return Err(format!("Nothing exists at that path: {}", path_part));
    }
    let name = std::path::Path::new(&path_part)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Startup app".into());
    add_managed_startup(&name, &path_part, &arg_part, false, true).map(|task| (task, name))
}

fn add_managed_startup_locked(name: &str, target_path: &str, arguments: &str, tray_mode: bool) -> Result<String, String> {
    // Repair stale pairs first: reconciliation is transaction-locked and fail-closed.
    crate::reconcile::reconcile_managed_startup_routes(None)?;
    let safe_name = sanitize_name(name);
    let named_task_location = format!("{}{}", MANAGED_TASK_ROOT, safe_name);
    let named_state = enabled::query_task_state(&named_task_location);
    if !can_upsert_task_state(named_state.as_deref()) {
        return Err(format!("A matching managed startup task is disabled. Enable it first; add/update left its task XML and stores unchanged: {}", named_task_location));
    }
    let (mut target, mut arguments) = (target_path.to_string(), arguments.to_string());
    normalize_managed_mode(&mut target, &mut arguments, tray_mode)?;
    let original_target = target.clone();
    let original_arguments = arguments.clone();
    let (mut equivalent_target, mut equivalent_arguments) = (original_target.clone(), original_arguments.clone());
    msm_core::quiet_plan::normalize_persistent(&mut equivalent_target, &mut equivalent_arguments);
    equivalent_arguments = msm_core::quiet_plan::strip_known_quiet_arguments(&equivalent_target, &equivalent_arguments);
    let (execute, action_args) = build_managed_action(&target, &arguments, tray_mode);
    ensure_no_managed_slot_conflict(&safe_name, &target, &arguments)?;
    // Reuse an equivalent persisted or live managed route.
    let existing_task = find_equivalent_task(&original_target, &original_arguments, &equivalent_target, &equivalent_arguments);
    if let Some(existing_task) = existing_task {
        if existing_task.starts_with(MANAGED_TASK_ROOT) {
            let state = enabled::query_task_state(&existing_task);
            if !can_upsert_task_state(state.as_deref()) {
                return Err(format!("This managed startup task is disabled. Enable it first; no update, implicit enable, or sibling task was created: {}", existing_task));
            }
            let existing_name = existing_task.trim_end_matches('\\').rsplit('\\').next().unwrap_or("").to_string();
            let row = enabled::Row {
                kind: "managed-task".into(),
                name: if existing_name.trim().is_empty() { safe_name.clone() } else { existing_name },
                scope: "User/System".into(),
                command: format!("{} {}", execute, action_args).trim().to_string(),
                location: existing_task.clone(),
                status: "Managed startup task (existing route updated)".into(),
                target: target.clone(),
                arguments: arguments.to_string(),
                mode: if tray_mode { "tray".into() } else { "normal".into() },
                task_location: existing_task.clone(),
            };
            return commit_managed_task_mutation(&existing_task, &execute, &action_args, &row, tray_mode, false);
        }
    }
    // Never create a second route for an already-starting Windows app package.
    if let Some(conflict) = find_enabled_packaged_conflict(&original_target) {
        return Err(format!("An enabled startup route already launches the same Windows app package: {} at {}. Edit or remove that route instead; no duplicate startup task was created.",
            conflict.human_name(), conflict.location));
    }
    let chosen_name = choose_managed_task_name(&safe_name, &target, &arguments, |candidate| {
        msm_core::tasks_com::query_task_state(candidate).is_some()
    })?;
    let task_location = format!("{}{}", MANAGED_TASK_ROOT, chosen_name);
    let row = enabled::Row {
        kind: "managed-task".into(),
        name: chosen_name.clone(),
        scope: "User/System".into(),
        command: format!("{} {}", execute, action_args).trim().to_string(),
        location: task_location.clone(),
        status: "Managed startup task".into(),
        target: target.clone(),
        arguments: arguments.to_string(),
        mode: if tray_mode { "tray".into() } else { "normal".into() },
        task_location: task_location.clone(),
    };
    commit_managed_task_mutation(&task_location, &execute, &action_args, &row, tray_mode, false)
}

fn normalize_managed_mode(target: &mut String, arguments: &mut String, tray_mode: bool) -> Result<(), String> {
    if tray_mode {
        resolve_shortcut_for_managed_startup(target, arguments, tray_mode)?;
    }
    msm_core::quiet_plan::normalize_persistent(target, arguments);
    if !tray_mode {
        *arguments = msm_core::quiet_plan::strip_known_quiet_arguments(target, arguments);
    }
    Ok(())
}

fn resolve_shortcut_for_managed_startup(target_path: &mut String, arguments: &mut String, tray_mode: bool) -> Result<(), String> {
    if !std::path::Path::new(target_path).extension().map(|e| e.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
        return Ok(());
    }
    let (resolved, shortcut_arguments) = msm_core::humans::resolve_shortcut(target_path);
    let prefix = if tray_mode { "Quiet mode cannot reliably identify this shortcut. " } else { "" };
    if resolved.trim().is_empty() || resolved.eq_ignore_ascii_case(target_path) || !std::path::Path::new(&resolved).is_file() {
        return Err(format!("{}Select the shortcut's real target instead: {}", prefix, target_path));
    }
    if !is_supported_startup_target(&resolved) || resolved.to_lowercase().ends_with(".lnk") {
        return Err(format!("{}Choose Window mode or select a directly supported target: {}", prefix, resolved));
    }
    *target_path = resolved;
    *arguments = merge_arguments(&shortcut_arguments, arguments);
    Ok(())
}

fn merge_arguments(first: &str, second: &str) -> String {
    regex::Regex::new(r"\s+").unwrap().replace_all(&format!("{} {}", first.trim(), second.trim()).trim(), " ").into_owned()
}

fn ensure_no_managed_slot_conflict(display_name: &str, target_path: &str, arguments: &str) -> Result<(), String> {
    let slot = canonical_managed_slot(display_name);
    if slot.is_empty() { return Ok(()); }
    let requested = msm_core::audit::normalize_managed_command(target_path, arguments);
    let divergent: Vec<String> = read_managed_candidates(None).into_iter()
        .filter(|route| route.slot_identity.eq_ignore_ascii_case(&slot) && !route.launch_identity.eq_ignore_ascii_case(&requested))
        .map(|route| route.task_location)
        .collect();
    if divergent.is_empty() { return Ok(()); }
    let locations = {
        let mut sorted = divergent.clone();
        sorted.sort_by_key(|l| l.to_lowercase());
        sorted.join(",")
    };
    if divergent.iter().any(|_| false) {
        return Err(format!("A different enabled managed route already owns the '{}' startup slot. Edit or remove that route instead; no duplicate was created: {}", display_name, locations));
    }
    Err(format!("A disabled managed route already owns the '{}' startup slot. Enable, edit, or permanently delete it first; no duplicate was created: {}", display_name, locations))
}

struct ManagedRoute {
    task_location: String,
    #[allow(dead_code)]
    launch_identity: String,
    slot_identity: String,
    enabled: bool,
}

fn read_managed_candidates(prefix: Option<&str>) -> Vec<ManagedRoute> {
    let leaf_prefix = prefix.map(managed_task_leaf).unwrap_or_default();
    let mut result = Vec::new();
    let agent_task = format!("{}MichStartupMasterApp", MANAGED_TASK_ROOT);
    for item in msm_core::scan::scan_all() {
        if !item.is_managed || !eq_ci(&item.source, "Scheduled Task") {
            continue;
        }
        let task_location = item.location.clone();
        if !msm_core::tasks::is_managed_scheduled_task_path(&task_location) || eq_ci(&task_location, &agent_task) {
            continue;
        }
        let leaf = managed_task_leaf(&task_location);
        if !leaf_prefix.trim().is_empty() && !leaf.to_lowercase().starts_with(&leaf_prefix.to_lowercase()) {
            continue;
        }
        let Some((target, arguments)) = msm_core::humans::try_decode_tray_payload(&item.command)
            .or_else(|| msm_core::humans::try_split_command(&item.command).map(|(t, a)| (t, a))) else { continue };
        let launch = msm_core::audit::normalize_managed_command(&target, &arguments);
        if launch.trim().is_empty() { continue; }
        result.push(ManagedRoute {
            task_location,
            launch_identity: launch,
            slot_identity: canonical_managed_slot(&leaf),
            enabled: item.enabled,
        });
    }
    result
}

fn find_equivalent_task(target: &str, arguments: &str, equivalent_target: &str, equivalent_arguments: &str) -> Option<String> {
    let requested = [
        msm_core::audit::normalize_managed_command(target, arguments),
        msm_core::audit::normalize_managed_command(equivalent_target, equivalent_arguments),
    ];
    for row in enabled::load().into_iter().filter(|r| eq_ci(&r.kind, "managed-task")) {
        let mut row_target = row.target.clone();
        let mut row_arguments = row.arguments.clone();
        msm_core::quiet_plan::normalize_persistent(&mut row_target, &mut row_arguments);
        if requested.iter().any(|req| req.eq_ignore_ascii_case(&msm_core::audit::normalize_managed_command(&row.target, &row.arguments)))
            || requested.iter().any(|req| req.eq_ignore_ascii_case(&msm_core::audit::normalize_managed_command(&row_target, &row_arguments))) {
            let task_location = if row.task_location.trim().is_empty() { row.location } else { row.task_location };
            if !task_location.trim().is_empty() {
                return Some(task_location);
            }
        }
    }
    // Live scan fallback.
    for item in msm_core::scan::scan_all().into_iter()
        .filter(|i| i.is_managed && eq_ci(&i.source, "Scheduled Task")
            && msm_core::tasks::is_managed_scheduled_task_path(&i.location)) {
        let (live_target, live_arguments) = match crate::mutation::resolve_launch_target_or_fail(&item) {
            Ok(pair) => pair,
            Err(_) => continue,
        };
        let mut normalized_target = live_target.clone();
        let mut normalized_arguments = live_arguments.clone();
        msm_core::quiet_plan::normalize_persistent(&mut normalized_target, &mut normalized_arguments);
        normalized_arguments = msm_core::quiet_plan::strip_known_quiet_arguments(&normalized_target, &normalized_arguments);
        if requested.iter().any(|req| req.eq_ignore_ascii_case(&msm_core::audit::normalize_managed_command(&live_target, &live_arguments))
            || req.eq_ignore_ascii_case(&msm_core::audit::normalize_managed_command(&normalized_target, &normalized_arguments))) {
            return Some(item.location);
        }
    }
    None
}

fn find_enabled_packaged_conflict(target_path: &str) -> Option<StartupItem> {
    let Some((requested_identity, _, _)) = msm_core::humans::try_get_windows_app_package_identity(target_path) else { return None };
    for item in msm_core::scan::scan_all().into_iter()
        .filter(|c| c.enabled && !c.id.starts_with("error|")) {
        let mut candidates: Vec<String> = Vec::new();
        if let Some((target, _)) = msm_core::humans::try_decode_tray_payload(&item.command) {
            candidates.push(target);
        }
        candidates.extend(msm_core::humans::application_payload_candidates(&item));
        for candidate in candidates.iter().filter(|c| !c.trim().is_empty()) {
            if let Some((identity, _, _)) = msm_core::humans::try_get_windows_app_package_identity(candidate) {
                if identity.eq_ignore_ascii_case(&requested_identity) {
                    return Some(item);
                }
            }
        }
    }
    None
}

/// C# `CommitManagedTaskMutation` — task + quiet/enabled stores as one transaction.
pub fn commit_managed_task_mutation(task_location: &str, execute: &str, action_args: &str,
                                     row: &msm_state::enabled::Row, tray_mode: bool, preserve_disabled: bool) -> Result<String, String> {
    let owned_task = task_location.to_string();
    let owned_execute = execute.to_string();
    let owned_args = action_args.to_string();
    let owned_row = row.clone();
    msm_state::coordinator::run(move || {
        let snapshots = capture_managed_snapshot(&owned_task);
        let result = (|| -> Result<(), String> {
            let task_existed = msm_core::tasks_com::query_task_state(&owned_task).is_some();
            register_logon_task_at(&owned_task, &owned_execute, &owned_args, task_existed)?;
            if preserve_disabled {
                msm_core::managed::run_checked("schtasks.exe", &format!("/Change /TN {} /Disable", q(&owned_task)), 30_000)?;
                protected_quiet::unprotect_task(&owned_task);
                enabled::remove_by_task(&owned_task);
            } else {
                if owned_row.mode.eq_ignore_ascii_case("tray") {
                    protected_quiet::protect_task(&owned_task, &owned_row.target, &owned_row.arguments);
                } else {
                    protected_quiet::unprotect_task(&owned_task);
                }
                enabled::upsert(&owned_row);
            }
            enabled::mark_v2_migrated_checked(&owned_task);
            Ok(())
        })();
        match result {
            Ok(()) => Ok(owned_task.clone()),
            Err(error) => {
                let rollback_errors = restore_managed_snapshot(&snapshots);
                let suffix = if rollback_errors.is_empty() { " rollback=complete".to_string() }
                    else { format!(" rollback-errors={}", rollback_errors.join(" | ")) };
                Err(format!("Managed startup change failed; prior task and intent were restored.{} Cause: {}", suffix, error))
            }
        }
    })?
}

struct ManagedSnapshot {
    task_location: String,
    task_existed: bool,
    task_xml: Option<String>,
    stores: Vec<crate::StoreSnapshotVec>,
}

fn capture_managed_snapshot(task_location: &str) -> ManagedSnapshot {
    let xml = msm_core::tasks_com::try_read_task_xml(task_location);
    ManagedSnapshot {
        task_location: task_location.to_string(),
        task_existed: xml.is_some(),
        task_xml: xml,
        stores: vec![
            crate::StoreSnapshotVec { path: msm_state::enabled_store(), bytes: std::fs::read(msm_state::enabled_store()).ok() },
            crate::StoreSnapshotVec { path: msm_state::protected_quiet_store(), bytes: std::fs::read(msm_state::protected_quiet_store()).ok() },
            crate::StoreSnapshotVec { path: msm_state::disabled_store(), bytes: std::fs::read(msm_state::disabled_store()).ok() },
            crate::StoreSnapshotVec { path: msm_state::protected_disabled_store(), bytes: std::fs::read(msm_state::protected_disabled_store()).ok() },
            crate::StoreSnapshotVec { path: msm_state::migrated_marker_path(), bytes: std::fs::read(msm_state::migrated_marker_path()).ok() },
        ],
    }
}

fn restore_managed_snapshot(snapshot: &ManagedSnapshot) -> Vec<String> {
    let mut errors = Vec::new();
    if snapshot.task_existed {
        if let Some(xml) = &snapshot.task_xml {
            if let Err(e) = msm_core::managed::restore_task_xml(&snapshot.task_location, xml) {
                errors.push(format!("task={}", e));
            }
        }
    } else if msm_core::tasks_com::query_task_state(&snapshot.task_location).is_some() {
        if let Err(e) = msm_core::managed::run_checked("schtasks.exe", &format!("/Delete /F /TN {}", q(&snapshot.task_location)), 30_000) {
            errors.push(format!("task-delete={}", e));
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

/// C# `DeleteManagedTask`.
pub fn delete_managed_task(name_or_task: &str) -> Result<(), String> {
    let tn = if name_or_task.starts_with('\\') {
        name_or_task.to_string()
    } else {
        let trimmed_root = MANAGED_TASK_ROOT.trim_matches('\\');
        format!("{}{}", MANAGED_TASK_ROOT, name_or_task.replace(trimmed_root, "").trim_matches('\\'))
    };
    msm_state::coordinator::run(move || {
        let snapshots = capture_managed_snapshot(&tn);
        let result = (|| -> Result<(), String> {
            if snapshots.task_existed {
                msm_core::managed::run_checked("schtasks.exe", &format!("/Delete /F /TN {}", q(&tn)), 30_000)?;
            }
            enabled::remove_by_task(&tn);
            protected_quiet::unprotect_task(&tn);
            enabled::mark_v2_migrated_checked(&tn);
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                let rollback_errors = restore_managed_snapshot(&snapshots);
                if rollback_errors.is_empty() {
                    Err(format!("Managed task deletion failed; prior task and intent were restored: {}", error))
                } else {
                    Err(format!("Managed task deletion failed and rollback was incomplete: {}. Cause: {}", rollback_errors.join(" | "), error))
                }
            }
        }
    })?
}

/// C# `EditStartup` / `SetPopupMode` share the same commit path.
pub fn edit_startup(item: &StartupItem, name: &str, target_path: &str, arguments: &str, tray_mode: bool) -> Result<String, String> {
    if item.id.is_empty() {
        return Err("No startup item selected".into());
    }
    if name.trim().is_empty() { return Err("Name is required".into()) }
    if target_path.trim().is_empty() || !std::path::Path::new(target_path).is_file() {
        return Err(format!("Application not found: {}", target_path));
    }
    if !is_supported_startup_target(target_path) {
        return Err("Choose a supported startup target: .exe, .com, .lnk, .cmd, .bat, .ps1, .vbs, .vbe, .js, .jse, .wsf, .wsh, .py, or .pyw".into());
    }
    if !item.enabled {
        return Err("This startup entry is disabled. Enable it first, then edit it; no enabled duplicate was created.".into());
    }
    if !can_mutate_startup_mode(item) {
        return Err("This scheduled task has multiple or non-executable actions. Editing is disabled so its original action set remains intact.".into());
    }
    let (mut target, mut arguments) = (target_path.to_string(), arguments.to_string());
    normalize_managed_mode(&mut target, &mut arguments, tray_mode)?;
    if item.id.starts_with("task|") && item.is_managed {
        let (execute, action_args) = build_managed_action(&target, &arguments, tray_mode);
        let row = enabled::Row {
            kind: "managed-task".into(), name: name.to_string(), scope: item.scope.clone(),
            command: format!("{} {}", execute, action_args).trim().to_string(),
            location: item.location.clone(), status: item.status.clone(),
            target: target.clone(), arguments: arguments.clone(),
            mode: if tray_mode { "tray".into() } else { "normal".into() },
            task_location: item.location.clone(),
        };
        return commit_managed_task_mutation(&item.location, &execute, &action_args, &row, tray_mode, false);
    }
    if item.popup_label() == "N/A" {
        return Err("This startup source cannot be edited as an application launch. Services, drivers, Winlogon, AppInit, and Active Setup rows should be changed from their owning tool.".into());
    }
    if item.enabled && !item.can_disable {
        return Err("This startup source is read-only here. Run elevated or choose its matching Registry Run, Startup Folder, or Scheduled Task row.".into());
    }
    let disabled_state = item.enabled;
    let original = item.clone();
    let name_owned = name.to_string();
    msm_state::coordinator::run(move || -> Result<String, String> {
        // Disable the original route after the replacement is durable (C# afterIntentCommitted).
        let task = add_managed_startup_locked(&name_owned, &target, &arguments, tray_mode)?;
        if disabled_state {
            if let Err(e) = crate::mutation::disable_core(&original) {
                return Err(format!("Replacement registered at {} but the original route could not be retired: {}", task, e));
            }
        }
        Ok(task)
    })?
}

/// C# `CanMutateStartupMode`.
pub fn can_mutate_startup_mode(item: &StartupItem) -> bool {
    if !item.enabled { return false }
    if eq_ci(&item.source, "Scheduled Task (Any Trigger)") { return false }
    !eq_ci(&item.source, "Scheduled Task") || !item.status.to_lowercase().contains("modeeditable=false")
}

/// C# `TogglePopupMode` (quiet ↔ window).
pub fn toggle_popup_mode(item: &StartupItem) -> Result<(), String> {
    let popup_enabled = item.popup_enabled();
    set_popup_mode(item, !popup_enabled)
}

pub fn set_popup_mode(item: &StartupItem, popup_enabled: bool) -> Result<(), String> {
    if item.popup_label() == "N/A" {
        return Err("Popup control applies to app/task/folder/registry launch commands, not service, driver, Winlogon, AppInit, or Active Setup rows.".into());
    }
    if !item.enabled {
        return Err("This startup entry is disabled. Enable it first, then change Window/Quiet mode; no enabled duplicate was created.".into());
    }
    if !can_mutate_startup_mode(item) {
        return Err("This scheduled task has multiple or non-executable actions. Window/Quiet mode editing is disabled so its original action set remains intact.".into());
    }
    let (target, arguments) = crate::mutation::resolve_launch_target_or_fail(item)?;
    let (mut normalized_target, mut normalized_arguments) = (target.clone(), arguments.clone());
    normalize_managed_mode(&mut normalized_target, &mut normalized_arguments, !popup_enabled)?;
    let (execute, action_args) = if popup_enabled {
        msm_core::managed::build_direct_action(&normalized_target, &normalized_arguments)
    } else {
        build_managed_action(&normalized_target, &normalized_arguments, true)
    };
    if item.id.starts_with("task|") && item.is_managed {
        let row = enabled::Row {
            kind: "managed-task".into(), name: item.name.clone(), scope: item.scope.clone(),
            command: format!("{} {}", execute, action_args).trim().to_string(),
            location: item.location.clone(), status: item.status.clone(),
            target: normalized_target.clone(), arguments: normalized_arguments.clone(),
            mode: if popup_enabled { "normal".into() } else { "tray".into() },
            task_location: item.location.clone(),
        };
        commit_managed_task_mutation(&item.location, &execute, &action_args, &row, !popup_enabled, false)?;
        return Ok(());
    }
    if item.enabled && !item.can_disable {
        return Err("This startup source is read-only here. Select its matching Registry Run, Startup Folder, or Scheduled Task row if Windows exposes one.".into());
    }
    let owned = item.clone();
    let target_owned = normalized_target.clone();
    let args_owned = normalized_arguments.clone();
    msm_state::coordinator::run(move || {
        // Managed replacement first; retire the original route only after success.
        let task = add_managed_startup_locked(&owned.name, &target_owned, &args_owned, !popup_enabled)?;
        if owned.enabled {
            crate::mutation::disable_core(&owned)?;
        }
        Ok::<String, String>(task)
    })?;
    Ok(())
}

/// C# `EnableLegacyV2`.
pub fn enable_legacy_v2(item: &StartupItem) -> Result<(), String> {
    let parts: Vec<&str> = item.id.split('|').collect();
    let legacy_name = if parts.len() > 1 { unb64(parts[1]) } else { String::new() };
    if legacy_name.trim().is_empty() {
        return Err("Legacy item name is missing".into());
    }
    let v2_path = format!("{}\\state-v2.json", msm_state::app_data());
    let raw = std::fs::read_to_string(&v2_path).map_err(|_| "Legacy v2 state file is missing".to_string())?;
    let doc: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("Legacy v2 state is unreadable: {}", e))?;
    let Some(items) = doc.get("Items").and_then(|v| v.as_array()) else {
        return Err("Legacy v2 state file is missing".into());
    };
    let mut target = String::new();
    let mut args = String::new();
    let mut launch_mode = 0i64;
    for entry in items {
        let string = |key: &str| entry.get(key).and_then(|v| v.as_str()).unwrap_or("");
        let mut name = string("LegacyTaskName").to_string();
        if name.trim().is_empty() { name = string("Name").to_string(); }
        if !name.eq_ignore_ascii_case(&legacy_name) { continue }
        target = string("Target").to_string();
        args = string("Arguments").to_string();
        launch_mode = entry.get("LaunchMode").and_then(|v| v.as_i64()).unwrap_or(0);
        break;
    }
    if target.trim().is_empty() {
        return Err("Legacy item target is missing in v2 state".into());
    }
    if !std::path::Path::new(&target).is_file() {
        return Err(format!("Application not found: {}", target));
    }
    add_managed_startup(&legacy_name, &target, &args, launch_mode == 2, true)?;
    enabled::mark_v2_migrated(&format!("{}{}", MANAGED_TASK_ROOT, legacy_name));
    Ok(())
}

#[allow(unused)]
fn _hints(a: &str) -> String {
    let _ = (b64(a), disabled::load_disabled_items, protected_disabled::load_rows, store::generation);
    String::new()
}
