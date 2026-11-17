//! Agent registration: one authoritative immediate logon task for the hidden agent.

use msm_core::util::{eq_ci, q};
use msm_state::{enabled, protected_quiet, store};

pub const AGENT_TASK: &str = r"\MichStartupMaster\MichStartupMasterApp";

/// C# `VerifyAgentTaskXml` — strict structural verification of the agent task.
pub fn verify_agent_task_xml(xml: &str, expected_exe: &str) -> (bool, String) {
    let mut detail = String::new();
    let Ok(doc) = msm_core::packaged::xml_doc(xml) else {
        return (false, "task XML unparseable".into());
    };
    let Some(root) = doc.root else { return (false, "task XML root missing".into()) };
    let triggers = msm_core::packaged::find_child(&root, "Triggers");
    let trigger_children: Vec<&msm_core::packaged::XmlNode> = triggers.map(|t| t.children.iter().collect()).unwrap_or_default();
    let single_logon = trigger_children.len() == 1 && eq_ci(&trigger_children[0].name.local_name, "LogonTrigger");
    let logon = trigger_children.first().copied();
    let no_delay = logon.map(|l| msm_core::packaged::find_child(l, "Delay").is_none()).unwrap_or(false);
    let current_user = logon
        .and_then(|l| msm_core::packaged::find_child(l, "UserId"))
        .map(|u| is_current_user_identity(u.text.trim()))
        .unwrap_or(false);
    let settings = msm_core::packaged::find_child(&root, "Settings");
    let enabled_flag = |parent: Option<&msm_core::packaged::XmlNode>, name: &str| -> Option<bool> {
        parent.and_then(|s| msm_core::packaged::find_child(s, name))
            .map(|n| n.text.trim().eq_ignore_ascii_case("true"))
    };
    let settings_allows = enabled_flag(settings, "Enabled").unwrap_or(true);
    let trigger_allows = logon.and_then(|l| msm_core::packaged::find_child(l, "Enabled"))
        .map(|n| n.text.trim().eq_ignore_ascii_case("true")).unwrap_or(true);
    let enabled = settings.is_some() && logon.is_some() && settings_allows && trigger_allows;
    let starts_on_battery = settings.and_then(|s| msm_core::packaged::find_child(s, "DisallowStartIfOnBatteries"))
        .map(|n| n.text.trim().eq_ignore_ascii_case("false")).unwrap_or(false);
    let continues_on_battery = settings.and_then(|s| msm_core::packaged::find_child(s, "StopIfGoingOnBatteries"))
        .map(|n| n.text.trim().eq_ignore_ascii_case("false")).unwrap_or(false);
    let start_when_available = settings.and_then(|s| msm_core::packaged::find_child(s, "StartWhenAvailable"))
        .map(|n| n.text.trim().eq_ignore_ascii_case("true")).unwrap_or(false);
    let principals = msm_core::packaged::find_child(&root, "Principals");
    let mut principal_valid = false;
    if let Some(principals) = principals {
        if principals.children.len() == 1 && eq_ci(&principals.children[0].name.local_name, "Principal") {
            let principal = &principals.children[0];
            let user_ok = msm_core::packaged::find_child(principal, "UserId")
                .map(|u| is_current_user_identity(u.text.trim())).unwrap_or(false);
            let logon_type_ok = msm_core::packaged::find_child(principal, "LogonType")
                .map(|l| l.text.trim().eq_ignore_ascii_case("InteractiveToken")).unwrap_or(false);
            let run_level_ok = msm_core::packaged::find_child(principal, "RunLevel")
                .map(|r| r.text.trim().eq_ignore_ascii_case("LeastPrivilege")).unwrap_or(true);
            principal_valid = user_ok && logon_type_ok && run_level_ok;
        }
    }
    let actions = msm_core::packaged::find_child(&root, "Actions");
    let action_children: Vec<&msm_core::packaged::XmlNode> = actions.map(|a| a.children.iter().collect()).unwrap_or_default();
    let exec: Option<&msm_core::packaged::XmlNode> = if action_children.len() == 1 && eq_ci(&action_children[0].name.local_name, "Exec") { Some(action_children[0]) } else { None };
    let mut single_exec = false;
    let mut command_match = false;
    let mut argument_match = false;
    if let Some(exec) = exec {
        single_exec = true;
        if let Some(command) = msm_core::packaged::find_child(exec, "Command") {
            command_match = paths_equal(command.text.trim(), expected_exe);
        }
        if let Some(arguments) = msm_core::packaged::find_child(exec, "Arguments") {
            argument_match = arguments.text.trim().eq_ignore_ascii_case("--agent");
        }
    }
    detail = format!("singleLogonTrigger={} currentUserTrigger={} enabled={} noDelay={} principal={} singleExec={} commandMatch={} argumentMatch={} startsOnBattery={} continuesOnBattery={} startWhenAvailable={}",
        single_logon, current_user, enabled, no_delay, principal_valid, single_exec, command_match, argument_match, starts_on_battery, continues_on_battery, start_when_available);
    (single_logon && current_user && enabled && no_delay && principal_valid && single_exec && command_match && argument_match && starts_on_battery && continues_on_battery && start_when_available, detail)
}

fn paths_equal(left: &str, right: &str) -> bool {
    if left.trim().is_empty() || right.trim().is_empty() { return false }
    let normalize = |value: &str| {
        let expanded = msm_core::util::expand_env(value.trim().trim_matches('"'));
        if std::path::Path::new(&expanded).is_absolute() {
            msm_core::paths::canonical(&expanded).trim_end_matches('\\').to_lowercase()
        } else {
            expanded.trim_end_matches('\\').to_lowercase()
        }
    };
    normalize(left) == normalize(right)
}

pub fn is_current_user_identity(candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() { return false }
    let current_sid = msm_core::util::current_user_sid();
    if candidate.eq_ignore_ascii_case(&current_sid) { return true }
    let account = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), msm_core::util::current_user_name());
    if candidate.eq_ignore_ascii_case(&account) { return true }
    // NTAccount → SID resolution through PowerShell would be heavyweight here;
    // the SID and DOMAIN\\user forms cover every generated task.
    false
}

/// C# `EnsureAgentRegistered` — transactional task creation + legacy shortcut retirement.
pub fn ensure_agent_registered() -> Result<String, String> {
    let exe = msm_core::util::process_exe_path();
    if exe.trim().is_empty() || !std::path::Path::new(&exe).is_file() {
        return Err(format!("Startup Master executable was not found: {}", exe));
    }
    msm_state::coordinator::run(move || {
        let prior_xml = msm_core::tasks_com::try_read_task_xml(AGENT_TASK);
        let prior_task_existed = prior_xml.is_some();
        let mut task_mutated = false;
        let retired = enumerate_owned_agent_shortcuts();
        let mut archived: Vec<(String, String)> = Vec::new();
        let result = (|| -> Result<String, String> {
            let mut current_detail = String::new();
            let current_valid = prior_xml.as_deref()
                .map(|xml| { let (ok, detail) = verify_agent_task_xml(xml, &exe); current_detail = detail; ok })
                .unwrap_or(false);
            if !current_valid {
                task_mutated = true;
                msm_core::managed::register_logon_task_at(AGENT_TASK, &exe, "--agent", true)?;
            }
            let Some(registered_xml) = msm_core::tasks_com::try_read_task_xml(AGENT_TASK) else {
                return Err("The scheduled startup-agent task was not present after registration".into());
            };
            let (registered_valid, registered_detail) = verify_agent_task_xml(&registered_xml, &exe);
            if !registered_valid {
                return Err(format!("The scheduled startup-agent task failed verification: {}", registered_detail));
            }
            if !retired.is_empty() {
                retire_agent_shortcuts(&retired, &mut archived)?;
            }
            let (valid, receipt) = verify_agent_registration();
            if !valid {
                return Err(receipt);
            }
            Ok(receipt)
        })();
        match result {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                let mut rollback_errors = Vec::new();
                if !archived.is_empty() {
                    if let Err(e) = restore_agent_shortcuts(&archived) {
                        rollback_errors.push(format!("shortcut restore: {}", e));
                    }
                }
                if task_mutated {
                    let restore_result = match (&prior_xml, prior_task_existed) {
                        (Some(xml), true) => msm_core::managed::restore_task_xml(AGENT_TASK, xml),
                        (_, false) => {
                            if msm_core::tasks_com::query_task_state(AGENT_TASK).is_some() {
                                msm_core::managed::run_checked("schtasks.exe", &format!("/Delete /F /TN {}", q(AGENT_TASK)), 30_000)
                            } else {
                                Ok(())
                            }
                        }
                        (None, true) => Ok(()),
                    };
                    if let Err(e) = restore_result {
                        rollback_errors.push(format!("task restore: {}", e));
                    }
                }
                if rollback_errors.is_empty() {
                    Err(format!("Startup-agent registration failed; the prior task and legacy shortcuts were restored: {}", error))
                } else {
                    Err(format!("Startup-agent registration failed and rollback was incomplete ({}): {}", rollback_errors.join("; "), error))
                }
            }
        }
    })?
}

/// C# `VerifyAgentRegistration` receipt.
pub fn verify_agent_registration() -> (bool, String) {
    let exe = msm_core::util::process_exe_path();
    let xml = msm_core::tasks_com::try_read_task_xml(AGENT_TASK);
    let task_exists = xml.is_some();
    let mut task_detail = String::new();
    let task_valid = xml.as_deref().map(|xml| { let (ok, detail) = verify_agent_task_xml(xml, &exe); task_detail = detail; ok }).unwrap_or(false);
    if !task_exists {
        task_detail = "taskExists=false".into();
    }
    let legacy_routes = enumerate_owned_agent_shortcuts();
    let owned_routes = (task_exists as usize) + legacy_routes.len();
    let valid = task_valid && owned_routes == 1 && legacy_routes.is_empty();
    let receipt = format!("{} task={} action={} {} ownedRoutes={} legacyRoutes={}",
        if valid { "REGISTER_AGENT_OK" } else { "REGISTER_AGENT_FAILED" },
        q(AGENT_TASK), q(&format!("{} --agent", exe)), task_detail, owned_routes, legacy_routes.len());
    (valid, receipt)
}

fn startup_folders() -> Vec<(String, String)> {
    vec![
        ("CurrentUser".into(), msm_core::scan_folders::user_startup_folder()),
        ("AllUsers".into(), msm_core::scan_folders::common_startup_folder()),
    ]
}

fn enumerate_owned_agent_shortcuts() -> Vec<(String, String, String, String)> {
    // (path, scope, target, arguments)
    let mut results = Vec::new();
    for (_, folder) in startup_folders() {
        if folder.trim().is_empty() || !std::path::Path::new(&folder).is_dir() {
            continue;
        }
        let shortcut_path = format!("{}\\Mich Startup Master Agent.lnk", folder.trim_end_matches('\\'));
        if !std::path::Path::new(&shortcut_path).is_file() {
            continue;
        }
        let (target, arguments) = msm_core::humans::resolve_shortcut(&shortcut_path);
        if !is_exact_owned_agent_launch(&target, &arguments) {
            continue;
        }
        let scope = if eq_ci(&folder, &msm_core::scan_folders::user_startup_folder()) { "CurrentUser".to_string() } else { "AllUsers".to_string() };
        results.push((shortcut_path, scope, target, arguments));
    }
    results.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    results
}

pub fn is_exact_owned_agent_launch_public(target: &str, arguments: &str, expected_exe: &str) -> bool {
    is_exact_owned_agent_launch_with(target, arguments, expected_exe)
}

fn is_exact_owned_agent_launch(target: &str, arguments: &str) -> bool {
    is_exact_owned_agent_launch_with(target, arguments, &msm_core::util::process_exe_path())
}

fn is_exact_owned_agent_launch_with(target: &str, arguments: &str, expected_exe: &str) -> bool {
    let expanded = msm_core::util::expand_env(target.trim().trim_matches('"'));
    let owned_exe = expected_exe.to_string();
    if paths_equal(&expanded, &owned_exe) && arguments.trim().eq_ignore_ascii_case("--agent") {
        return true;
    }
    is_system_script_host(&expanded) && try_get_legacy_agent_script(arguments, &owned_exe).is_some()
}

fn is_system_script_host(target: &str) -> bool {
    let leaf = target.rsplit('\\').next().unwrap_or("");
    if !eq_ci(leaf, "wscript.exe") && !eq_ci(leaf, "cscript.exe") {
        return false;
    }
    let windows = msm_core::paths::canonical(&msm_core::paths::windows_dir()).trim_end_matches('\\').to_string();
    ["System32", "SysWOW64", "Sysnative"].iter().any(|folder| {
        paths_equal(target, &format!("{}\\{}\\{}", windows, folder, leaf))
    })
}

fn try_get_legacy_agent_script(arguments: &str, expected_exe: &str) -> Option<String> {
    let value = arguments.trim();
    let prefixes = ["//B //NoLogo ", "//NoLogo //B ", "//B ", "//NoLogo ", ""];
    for prefix in prefixes {
        let Some(mut candidate) = value.strip_prefix(prefix) else { continue };
        candidate = candidate.trim();
        if candidate.len() >= 2 && candidate.starts_with('"') && candidate.ends_with('"') {
            candidate = &candidate[1..candidate.len() - 1];
        } else if candidate.contains('"') {
            continue;
        }
        let candidate = msm_core::util::expand_env(candidate.trim());
        if !std::path::Path::new(&candidate).is_absolute() { continue }
        if !eq_ci(candidate.rsplit('\\').next().unwrap_or(""), "MichStartupMasterAgent.vbs") { continue }
        if !std::path::Path::new(&candidate).is_file() { continue }
        let Ok(script) = std::fs::read_to_string(&candidate) else { continue };
        let normalized_script = script.replace('/', "\\");
        let normalized_exe = expected_exe.replace('/', "\\");
        if normalized_exe.trim().is_empty() { continue }
        if !normalized_script.to_lowercase().contains(&normalized_exe.to_lowercase()) { continue }
        if !normalized_script.to_lowercase().contains("--agent") { continue }
        return Some(candidate);
    }
    None
}

fn retire_agent_shortcuts(routes: &[(String, String, String, String)], archived: &mut Vec<(String, String)>) -> Result<(), String> {
    if routes.is_empty() { return Ok(()); }
    use std::time::{SystemTime, UNIX_EPOCH};
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let archive_root = format!("{}\\RetiredAgentLaunchers\\{}-{}", msm_state::app_data(), stamp, msm_actions_guid());
    for (path, scope, _, _) in routes {
        let scope_folder = format!("{}\\{}", archive_root, scope);
        std::fs::create_dir_all(&scope_folder).map_err(|e| e.to_string())?;
        let mut destination = format!("{}\\{}", scope_folder, path.rsplit('\\').next().unwrap_or("shortcut.lnk"));
        if std::path::Path::new(&destination).exists() {
            destination = format!("{}\\{}{}", scope_folder, msm_actions_guid(), ".lnk");
        }
        std::fs::rename(path, &destination).map_err(|e| e.to_string())?;
        archived.push((path.clone(), destination));
    }
    Ok(())
}

fn msm_actions_guid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:032x}", nanos ^ ((std::process::id() as u128) << 96))
}

fn restore_agent_shortcuts(archived: &[(String, String)]) -> Result<(), String> {
    let mut errors = Vec::new();
    for (original, archive) in archived.iter().rev() {
        if let Err(e) = (|| -> Result<(), String> {
            if !std::path::Path::new(archive).is_file() {
                return Err(format!("Archived startup shortcut is missing during rollback: {}", archive));
            }
            if std::path::Path::new(original).exists() {
                return Err(format!("A file now exists at the legacy shortcut restore path: {}", original));
            }
            if let Some(parent) = std::path::Path::new(original).parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::rename(archive, original).map_err(|e| e.to_string())
        })() {
            errors.push(format!("{}: {}", original, e));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

/// C# `EnabledStartupService::EnforceEnabled` — adopt + register + run every enabled intent.
pub fn enforce_enabled(include_import: bool) -> String {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    static BUSY: AtomicBool = AtomicBool::new(false);
    static LAST_IMPORT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    if BUSY.swap(true, Ordering::SeqCst) {
        return "ENFORCE_ENABLED busy".into();
    }
    let result = msm_state::coordinator::run(move || {
        if include_import {
            let mut last_import = LAST_IMPORT.lock().unwrap();
            let due = last_import.map(|stamp| stamp.elapsed().as_secs() >= 5 * 60).unwrap_or(true);
            if due {
                import_existing_managed_tasks();
                *last_import = Some(std::time::Instant::now());
            }
        }
        let mut actions = 0usize;
        let mut failures = 0usize;
        // The app's own hidden boot agent must always be registered, enabled, and correct.
        let exe = msm_core::util::process_exe_path();
        if !exe.trim().is_empty() && std::path::Path::new(&exe).is_file() {
            if let Ok(()) = enforce_agent_row(&exe) {
                actions += 1;
            } else {
                failures += 1;
            }
        }
        for row in enabled::load() {
            match enforce_row(&row) {
                Ok(true) => actions += 1,
                Ok(false) => {}
                Err(_) => failures += 1,
            }
        }
        let protected = enabled::load().len();
        format!("ENFORCE_ENABLED protected={} actions={} failures={}", protected, actions, failures)
    });
    BUSY.store(false, Ordering::SeqCst);
    result.unwrap_or_else(|e| format!("ENFORCE_ENABLED error={}", e))
}

fn enforce_agent_row(exe: &str) -> Result<(), String> {
    let existing_state = msm_core::tasks_com::query_task_state(AGENT_TASK);
    let (_execute, action_args) = msm_core::managed::build_direct_action(exe, "--agent");
    let correct = existing_state.is_some() && task_matches(AGENT_TASK, exe, &action_args);
    if !correct || existing_state.as_deref() != Some("enabled") {
        msm_core::managed::register_logon_task_at(AGENT_TASK, exe, "--agent", true)?;
    }
    Ok(())
}

fn task_matches(task_location: &str, expected_execute: &str, expected_arguments: &str) -> bool {
    let Some(xml) = msm_core::tasks_com::try_read_task_xml(task_location) else { return false };
    let extract = |element: &str| -> Option<String> {
        let pattern = format!("<{}>(?<v>.*?)</{}>", element, element);
        regex::Regex::new(&pattern).ok()?.captures(&xml)?.name("v").map(|m| m.as_str().trim().to_string())
    };
    let decode = |value: String| value.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
    let Some(command) = extract("Command") else { return false };
    let arguments = extract("Arguments").unwrap_or_default();
    decode(command).eq_ignore_ascii_case(expected_execute) && decode(arguments).trim() == expected_arguments.trim()
}

fn enforce_row(row: &msm_state::enabled::Row) -> Result<bool, String> {
    if row.kind == "managed-task" || row.kind == "task" {
        let mut task_location = row.task_location.clone();
        if task_location.trim().is_empty() { task_location = row.location.clone(); }
        if task_location.trim().is_empty() { return Ok(false); }
        let is_managed = row.kind == "managed-task";
        let tray = row.mode.eq_ignore_ascii_case("tray");
        let (mut expected_exec, mut expected_args) = (row.target.clone(), row.arguments.clone());
        if is_managed {
            let mut target = row.target.clone();
            if target.trim().is_empty() { return Ok(false); }
            if !std::path::Path::new(&target).is_file() {
                target = msm_core::humans::resolve_target_path(&target);
            }
            if !std::path::Path::new(&target).is_file() { return Ok(false); }
            let mut arguments = row.arguments.clone();
            if tray && msm_core::quiet_plan::normalize_persistent(&mut target, &mut arguments) {
                // Persist the resolved packaged path so the manifest self-heals.
                let mut updated = row.clone();
                updated.target = target.clone();
                updated.arguments = arguments.clone();
                updated.command = {
                    let (execute, action_args) = msm_core::managed::build_managed_action(&target, &arguments, tray);
                    format!("{}{}", execute, if action_args.trim().is_empty() { String::new() } else { format!(" {}", action_args) })
                };
                enabled::upsert(&updated);
            }
            let (execute, action_args) = msm_core::managed::build_managed_action(&target, &row.arguments, tray);
            expected_exec = execute;
            expected_args = action_args;
        }
        let state = msm_core::tasks_com::query_task_state(&task_location);
        if state.is_none() {
            if is_managed {
                msm_core::managed::register_logon_task_at(&task_location, &expected_exec, &expected_args, true)?;
            } else {
                msm_core::managed::run_checked("schtasks.exe", &format!("/Create /F /TN {} /SC ONLOGON /TR {}", q(&task_location),
                    q(&msm_core::util::win_arg(&expected_exec), )), 30_000)
                    .map_err(|e| format!("task create failed: {}", e))?;
            }
            try_run_task(&task_location);
            return Ok(true);
        }
        if state.as_deref() == Some("disabled") {
            msm_core::managed::run_checked("schtasks.exe", &format!("/Change /TN {} /Enable", q(&task_location)), 30_000)?;
            try_run_task(&task_location);
            return Ok(true);
        }
        if is_managed {
            let correct = task_matches(&task_location, &expected_exec, &expected_args);
            if !correct {
                msm_core::managed::register_logon_task_at(&task_location, &expected_exec, &expected_args, true)?;
                try_run_task(&task_location);
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if row.kind == "registry" {
        use msm_core::reg::{create_subkey, open_subkey, RegKind, RegRoot, RegView, RegValue};
        let meta: Vec<&str> = row.status.split('\t').collect();
        let (root, sub_key, value_name, kind) = if meta.len() >= 4 && meta[0].starts_with("HKEY_") {
            (RegRoot::parse(meta[0]).ok_or("bad root")?, meta[1].to_string(), meta[2].to_string(), RegKind::parse(meta[3]))
        } else {
            (if row.scope.eq_ignore_ascii_case("Machine") { RegRoot::LocalMachine } else { RegRoot::CurrentUser },
             r"Software\Microsoft\Windows\CurrentVersion\Run".to_string(), row.name.clone(), RegKind::String)
        };
        if value_name.trim().is_empty() || row.command.trim().is_empty() { return Ok(false); }
        let view = RegView::parse(if meta.len() > 6 { meta[6] } else { "" });
        let key = msm_core::ids::reg_id(&row.scope, root.name(), &sub_key, &value_name, if meta.len() > 6 { meta[6] } else { "" });
        msm_state::protected_disabled::unprotect_key(&key);
        if let Some(handle) = open_subkey(root, &sub_key, view) {
            if handle.value_names().iter().any(|n| n.eq_ignore_ascii_case(&value_name)) {
                return Ok(false);
            }
        }
        let Some(handle) = create_subkey(root, &sub_key, view) else { return Ok(false) };
        let value = match kind {
            RegKind::MultiString => RegValue::MultiString(vec![row.command.clone()]),
            RegKind::Binary => RegValue::Bytes(row.command.clone().into_bytes()),
            RegKind::DWord => RegValue::DWord(row.command.trim().parse().unwrap_or(0)),
            RegKind::QWord => RegValue::QWord(row.command.trim().parse().unwrap_or(0)),
            _ => RegValue::Str(row.command.clone()),
        };
        if !handle.set_value(&value_name, kind, &value) { return Ok(false); }
        try_launch_target(&row.target, &row.arguments);
        return Ok(true);
    }
    if row.kind == "folder" {
        let file = row.command.clone();
        if file.trim().is_empty() { return Ok(false); }
        msm_state::protected_disabled::unprotect_key(&format!("folder|{}|{}", row.scope, file));
        if std::path::Path::new(&file).is_file() { return Ok(false); }
        let file_name = file.rsplit('\\').next().unwrap_or("").to_string();
        let quarantine = msm_state::disabled_startup_folder();
        if let Ok(entries) = std::fs::read_dir(&quarantine) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.to_lowercase().starts_with(&format!("{}.", file_name.to_lowercase())) {
                    if let Some(parent) = std::path::Path::new(&file).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if std::fs::rename(entry.path(), &file).is_ok() {
                        try_launch_target(&row.target, &row.arguments);
                        return Ok(true);
                    }
                }
            }
        }
        return Ok(false);
    }
    if row.kind == "service" || row.kind == "driver" {
        let Some((service_name, desired_start)) = enabled::try_parse_exact_service_intent(&row.status) else { return Ok(false) };
        let path = format!(r"SYSTEM\CurrentControlSet\Services\{}", service_name);
        let Some(handle) = msm_core::reg::open_subkey_write(msm_core::reg::RegRoot::LocalMachine, &path, msm_core::reg::RegView::Default) else { return Ok(false) };
        if let Some((_, msm_core::reg::RegValue::DWord(current))) = handle.value("Start") {
            if current == 4 {
                if !handle.set_value("Start", msm_core::reg::RegKind::DWord, &msm_core::reg::RegValue::DWord(desired_start as u32)) {
                    return Ok(false);
                }
                msm_state::protected_disabled::unprotect_key(&msm_core::ids::service_protection_key(&row.kind, &service_name));
                let _ = msm_core::managed::run_checked("sc.exe", &format!("start {}", service_name), 20_000);
                return Ok(true);
            }
        }
        return Ok(false);
    }
    Ok(false)
}

fn try_run_task(task_location: &str) {
    if eq_ci(task_location, AGENT_TASK) { return }
    if let Some(row) = enabled::load().into_iter().find(|r| r.task_location.eq_ignore_ascii_case(task_location)) {
        if !row.target.trim().is_empty() && msm_core::runtime::is_process_running(&row.target) {
            return;
        }
    }
    let _ = msm_core::managed::run_checked("schtasks.exe", &format!("/Run /TN {}", q(task_location)), 30_000);
}

fn try_launch_target(target: &str, arguments: &str) {
    if target.trim().is_empty() || !std::path::Path::new(target).is_file() { return }
    let (execute, action_args) = msm_core::managed::build_direct_action(target, arguments);
    use std::os::windows::process::CommandExt;
    let working_directory = target.rsplit_once('\\').map(|(dir, _)| dir.to_string()).filter(|d| std::path::Path::new(d).is_dir())
        .unwrap_or_else(|| msm_core::paths::local_app_data());
    let _ = std::process::Command::new(&execute)
        .raw_arg(&action_args)
        .current_dir(&working_directory)
        .creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn();
}

fn import_existing_managed_tasks() {
    // Adopt every enabled \MichStartupMaster\ task not yet tracked (agent excluded,
    // user-disabled tasks excluded unless quiet-protected).
    let known: Vec<String> = enabled::load().into_iter()
        .filter(|r| r.kind == "managed-task" && !r.task_location.trim().is_empty())
        .map(|r| r.task_location.to_lowercase()).collect();
    let script = "foreach($t in Get-ScheduledTask){$full=($t.TaskPath + $t.TaskName);if(-not $full.StartsWith('\\MichStartupMaster\\')){continue};$enabled=if($t.Settings.Enabled){'true'}else{'false'};$actions=(@($t.Actions)|ForEach-Object{if($_){(($_.Execute)+' '+($_.Arguments)).Trim()}})-join ' || ';$full + \"`t\" + $enabled + \"`t\" + $actions}";
    use std::os::windows::process::CommandExt;
    let Ok(output) = std::process::Command::new(msm_core::managed::power_shell_exe())
        .raw_arg(format!("-NoProfile -ExecutionPolicy Bypass -EncodedCommand {}", encode_command(script)))
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output() else { return };
    if !output.status.success() { return }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 { continue }
        let task_location = parts[0].to_string();
        if parts[1] != "true" { continue }
        if eq_ci(&task_location, AGENT_TASK) { continue }
        if msm_state::protected_disabled::is_protected(&format!("task|{}", task_location))
            && !msm_state::protected_quiet::is_protected(&task_location) {
            continue;
        }
        if known.iter().any(|k| k.eq_ignore_ascii_case(&task_location)) { continue }
        let action = parts[2].to_string();
        let mut mode = "normal".to_string();
        let mut target = String::new();
        let mut args = String::new();
        if action.to_lowercase().contains("--tray-run") {
            mode = "tray".into();
            if let Some((t, a)) = msm_core::humans::try_decode_tray_payload(&action) {
                target = t;
                args = a;
            }
        } else {
            if msm_core::quiet_plan::is_quiet_launch(&action, "") { mode = "tray".into(); }
            if let Some((t, a)) = msm_core::humans::try_split_command(&action) {
                target = t;
                args = a;
            }
        }
        if target.trim().is_empty() || !std::path::Path::new(&target).is_file() { continue }
        if !msm_core::managed::is_supported_startup_target(&target) { continue }
        enabled::upsert(&msm_state::enabled::Row {
            kind: "managed-task".into(),
            name: task_location.trim_start_matches('\\').to_string(),
            scope: "User/System".into(),
            command: action,
            location: task_location.clone(),
            status: "Managed startup task".into(),
            target, arguments: args, mode, task_location,
        });
    }
}

fn encode_command(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(&utf16)
}

#[allow(unused)]
fn _hint() {
    let _ = (protected_quiet::is_protected, store::generation);
}
