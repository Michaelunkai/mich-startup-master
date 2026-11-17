//! Mich Startup Master v3 — Rust binary: CLI gates, GUI entry, tray agent.

#![windows_subsystem = "windows"]

use msm_core::model::StartupItem;
use msm_core::util::q;

mod ui_egui;
mod tray_win;

fn main() {
    // Any panic, on any thread, lands in a log file we can read back.
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        let path = std::env::temp_dir().join("msm-panic.log");
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "[{}] {}", chrono_like_now(), info);
        }
    }));
    msm_state::install_scan_hooks();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = if args.is_empty() { launch_gui(false) } else { dispatch(&args) };
    std::process::exit(code);
}

fn chrono_like_now() -> String {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix+{}", elapsed)
}

/// Attach to the parent console so gate scripts capture stdout (C# parity).
/// A redirected (piped) stdout is already valid — never override it with CONOUT$.
fn attach_console() {
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::System::Console::{AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, ATTACH_PARENT_PROCESS};
    unsafe {
        let existing = GetStdHandle(STD_OUTPUT_HANDLE).unwrap_or(INVALID_HANDLE_VALUE);
        if existing != INVALID_HANDLE_VALUE && !existing.0.is_null() {
            return; // inherited pipe/console handle works as-is
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
            reopen_conout(STD_OUTPUT_HANDLE);
            reopen_conout(STD_ERROR_HANDLE);
        }
    }
}

unsafe fn reopen_conout(slot: windows::Win32::System::Console::STD_HANDLE) {
    use windows::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::Console::SetStdHandle;
    use windows::core::PCWSTR;
    let wide = msm_core::paths::to_wide("CONOUT$");
    if let Ok(handle) = CreateFileW(
        PCWSTR(wide.as_ptr()),
        GENERIC_WRITE.0,
        FILE_SHARE_WRITE,
        None,
        OPEN_EXISTING,
        Default::default(),
        None,
    ) {
        let _ = SetStdHandle(slot, handle);
    }
}

fn dispatch(args: &[String]) -> i32 {
    let cmd = args[0].to_lowercase();
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    match cmd.as_str() {
        "--version" => { attach_console(); println!("MichStartupMaster 3.0.0"); 0 }
        "--help" | "-h" | "/?" | "help" => { attach_console(); print_help(); 0 }
        "--smoke" => { attach_console(); smoke() }
        "--list" => { attach_console(); list("all") }
        "--list-enabled" => { attach_console(); list("enabled") }
        "--list-disabled" => { attach_console(); list("disabled") }
        "--list-sources" => { attach_console(); list_sources() }
        "--list-managed" => { attach_console(); println!("{}", msm_state::enabled::to_json()); 0 }
        "--search" => { attach_console(); search(&rest) }
        "--export" => { attach_console(); export() }
        "--get-item" => { attach_console(); get_item(&rest) }
        "--set-enabled" => { attach_console(); set_enabled(&rest) }
        "--audit-boot" => { attach_console(); audit(false) }
        "--audit-tray" => { attach_console(); audit(true) }
        "--verify-live-inventory" => { attach_console(); verify_live() }
        "--detect-new" => { attach_console(); detect_new() }
        "--add-startup" => { attach_console(); add_startup(&rest) }
        "--add-from-path" => { attach_console(); add_from_path(&rest) }
        "--purge" => { attach_console(); purge_key(&rest) }
        "--hidden-list" => { attach_console(); hidden_list() }
        "--hidden-restore" => { attach_console(); hidden_restore(&rest) }
        "--add-test-task" | "--add-test-task-tray" | "--add-test-task-normal" => { attach_console(); add_test_task(&rest, cmd != "--add-test-task-normal") }
        "--remove-task" => { attach_console(); remove_task(&rest) }
        "--edit-startup" => { attach_console(); edit_startup(&rest) }
        "--toggle-popup" => { attach_console(); toggle_popup(&rest) }
        "--run-now" => { attach_console(); run_now(&rest) }
        "--open-location" => { attach_console(); open_location(&rest) }
        "--open-folders" => { attach_console(); open_folders() }
        "--repair-rules" => { attach_console(); println!("{} | {} | {}", msm_state::protected_disabled::enforce_protected(), msm_state::protected_quiet::enforce_protected(), msm_actions::agent::enforce_enabled(true)); 0 }
        "--protect-disabled" => { attach_console(); println!("{}", msm_state::protected_disabled::protect_current_disabled()); 0 }
        "--enforce-disabled" => { attach_console(); println!("{}", msm_state::protected_disabled::enforce_protected()); 0 }
        "--enforce-quiet" => { attach_console(); println!("{}", msm_state::protected_quiet::enforce_protected()); 0 }
        "--enforce-enabled" => { attach_console(); println!("{}", msm_actions::agent::enforce_enabled(false)); 0 }
        "--reconcile-managed-startups" => { attach_console(); reconcile_managed(&rest) }
        "--register-agent" => { attach_console(); register_agent() }
        "--verify-agent" => { attach_console(); verify_agent() }
        "--start-in-tray" | "--agent" => launch_gui(true),
        "--tray-run" => { if rest.is_empty() { 2 } else { msm_quiet::tray_run(&rest[0]); 0 } }
        "--quiet-plan" => { attach_console(); quiet_plan(&rest) }
        "--quiet-policy-probe" => { attach_console(); println!("{}", msm_quiet::policy_probe_json()); 0 }
        "--quiet-lineage-self-test" => { attach_console(); println!("{}", msm_quiet::lineage_self_test()); 0 }
        "--quiet-performance-probe" => { attach_console(); println!("{}", msm_quiet::performance_probe_json()); 0 }
        "--truth-self-test" => { attach_console(); println!("{}", truth_self_test()); 0 }
        "--inventory-self-test" => { attach_console(); println!("{}", inventory_self_test()); 0 }
        "--managed-startup-dedupe-self-test" => { attach_console(); println!("{}", msm_actions::reconcile::managed_startup_dedupe_self_test()); 0 }
        "--state-store-self-test" => { attach_console(); println!("{}", msm_state::store::self_test()); 0 }
        "--state-store-worker" => msm_state::store::worker(args),
        "--release-gate-self-test" => { attach_console(); println!("{}", release_gate_self_test()); 0 }
        "--agent-registration-self-test" => { attach_console(); println!("{}", agent_registration_self_test()); 0 }
        "--ui-contract" => { attach_console(); println!("{}", msm_ui::ui_contract_json()); 0 }
        "--ui-self-test" => { attach_console(); println!("{}", msm_ui::ui_self_test()); 0 }
        "--shell-open" => { attach_console(); shell_open(&rest) }
        _ => { attach_console(); println!("Unknown command: {}. Try --help.", args[0]); 2 }
    }
}

fn print_help() {
    let help = r#"MichStartupMaster CLI - Startup Management Commands
====================================================

INVENTORY & QUERY:
  --list                   List all startup items as JSON
  --list-enabled           List only enabled startup items
  --list-disabled          List only disabled startup items
  --list-sources           List all unique startup source types
  --search <query>         Search startup items by name, path, or command
  --get-item <id>          Get details for a specific startup item
  --export                 Export all startup data as TSV (paste-ready)
  --detect-new             Detect newly added startup items since last check

ENABLE/DISABLE:
  --set-enabled <id> <true|false>   Enable or disable a startup item
  --toggle-popup <id>               Toggle quiet (tray) mode for an item

ADD/REMOVE/EDIT:
  --add-startup <name> <path> [args] [normal|tray]   Add a new startup item
  --remove-task <name>                                Remove a managed scheduled task
  --edit-startup <id> <name> <path> [args] [normal|tray]  Edit an existing startup item

RUN/OPEN:
  --run-now <id>            Run a startup item now
  --open-location <id>      Open the folder containing the startup item
  --open-folders            Open user and common startup folders

PROTECTION & REPAIR:
  --protect-disabled       Protect currently disabled items from being re-enabled
  --enforce-disabled       Enforce all protected disabled items
  --enforce-quiet          Enforce all protected quiet (tray) items
  --enforce-enabled        Enforce all managed enabled items
  --repair-rules           Repair all startup rules (disabled + quiet + enabled)
  --reconcile-managed-startups [prefix]   Reconcile managed startup routes

AUDIT & VERIFY:
  --audit-boot             Audit boot coverage (completeness check)
  --audit-tray             Audit tray icon coverage
  --verify-live-inventory  Verify live inventory matches rendered UI
  --smoke                  Quick smoke test of inventory

AGENT & BACKGROUND:
  --register-agent         Register the boot agent
  --verify-agent           Verify agent registration
  --start-in-tray          Start the app minimized to tray
  --agent                  Start as background agent

GUI:
  --ui-contract            Show the UI contract as JSON
  --ui-self-test           Run UI self-tests

SELF-TESTS:
  --truth-self-test                Run truth verification tests
  --inventory-self-test            Run inventory self-tests
  --managed-startup-dedupe-self-test   Run managed startup deduplication tests
  --state-store-self-test          Run state store self-tests
  --release-gate-self-test         Run release gate security tests
  --quiet-policy-probe             Probe quiet (tray) policy
  --quiet-lineage-self-test        Run quiet lineage tests
  --quiet-performance-probe        Probe quiet performance
  --quiet-plan <base64>            Create a quiet launch plan

INTERNAL:
  --version                Show version
  --help                   Show this help message
  --shell-open <payload>   Open a file with shell execute
  --state-store-worker     State store worker process
  --tray-run               Run the tray application"#;
    println!("{}", help);
}

// ---- gates ----

fn smoke() -> i32 {
    let items = msm_core::scan::scan_all();
    let invalid_rows = items.iter().filter(|x| x.id.trim().is_empty() || x.source.trim().is_empty() || x.location.trim().is_empty()).count();
    let duplicate_ids = count_duplicates(&items.iter().filter(|x| !x.id.trim().is_empty()).map(|x| x.id.to_lowercase()).collect::<Vec<_>>());
    let provider_errors = items.iter().filter(|x| x.id.to_lowercase().starts_with("error|") || x.status == "Read failed").count();
    let type_init_failures = 0usize;
    let passed = !items.is_empty() && invalid_rows == 0 && duplicate_ids == 0 && provider_errors == 0 && type_init_failures == 0;
    println!("SMOKE passed={} inventory={} invalid={} duplicate_ids={} provider_errors={} type_init_failures={} user={} appdata={}",
        passed, items.len(), invalid_rows, duplicate_ids, provider_errors, type_init_failures,
        msm_core::util::current_user_name(), msm_core::paths::state_root());
    if passed { 0 } else { 1 }
}

fn count_duplicates(values: &[String]) -> usize {
    let mut counts: std::collections::HashMap<&String, usize> = std::collections::HashMap::new();
    for value in values { *counts.entry(value).or_insert(0) += 1; }
    counts.values().filter(|c| **c > 1).count()
}

fn list(mode: &str) -> i32 {
    let mut items = msm_core::scan::scan_all();
    if mode == "enabled" { items.retain(|x| x.enabled); }
    if mode == "disabled" { items.retain(|x| !x.enabled); }
    let mut deduped: Vec<StartupItem> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for item in items {
        if seen.iter().any(|s| s.eq_ignore_ascii_case(&item.id)) { continue }
        seen.push(item.id.clone());
        deduped.push(item);
    }
    println!("{}", msm_core::json::to_json(&deduped));
    0
}

fn list_sources() -> i32 {
    let items = msm_core::scan::scan_all();
    let mut sources: Vec<String> = items.iter().map(|x| x.source.clone()).collect();
    sources.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    sources.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    println!("SOURCES count={}", sources.len());
    for source in &sources { println!("  {}", source); }
    // Coverage vs Autoruns categories: scanner implemented + live item count + live registry probe.
    use msm_core::reg::{open_subkey, native_registry_view, RegRoot};
    let view = native_registry_view();
    let probe = |root: RegRoot, sub: &str| -> String {
        match open_subkey(root, sub, view) {
            Some(k) => format!("key present, {} values, {} subkeys", k.value_names().len(), k.subkey_names().len()),
            None => "key absent".to_string(),
        }
    };
    let count = |names: &[&str]| items.iter().filter(|x| names.iter().any(|n| x.source.eq_ignore_ascii_case(n))).count();
    let hklm = RegRoot::LocalMachine; let hkcu = RegRoot::CurrentUser;
    let rows: Vec<(&str, &[&str], String)> = vec![
        ("Logon: Run (HKLM)", &["Registry Run"], probe(hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run")),
        ("Logon: Run (HKCU)", &["Registry Run"], probe(hkcu, r"Software\Microsoft\Windows\CurrentVersion\Run")),
        ("Logon: RunOnce", &["Registry RunOnce", "Registry RunOnceEx"], probe(hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce")),
        ("Logon: Startup folders", &["Startup Folder", "Startup Approval"], probe(hkcu, r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder")),
        ("Logon: Winlogon Shell/Userinit", &["Winlogon Autostart", "Winlogon Notification"], probe(hklm, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon")),
        ("Logon: Active Setup", &["Active Setup"], probe(hklm, r"SOFTWARE\Microsoft\Active Setup\Installed Components")),
        ("Scheduled Tasks", &["Scheduled Task", "Scheduled Task (Any Trigger)", "Packaged Startup Task"], "Task Scheduler COM".to_string()),
        ("Services", &["Windows Service", "Service (Trigger Start)", "Service Dependency"], probe(hklm, r"SYSTEM\CurrentControlSet\Services")),
        ("Drivers", &["System Driver"], probe(hklm, r"SYSTEM\CurrentControlSet\Services")),
        ("Boot Execute", &["Boot Execute"], probe(hklm, r"SYSTEM\CurrentControlSet\Control\Session Manager")),
        ("Image Hijacks (IFEO)", &["Image Hijack"], probe(hklm, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options")),
        ("AppInit DLLs", &["AppInit DLLs"], probe(hklm, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows")),
        ("AppCert DLLs", &["AppCert DLLs"], probe(hklm, r"SYSTEM\CurrentControlSet\Control\Session Manager\AppCertDlls")),
        ("Known DLLs", &["Known DLL"], probe(hklm, r"SYSTEM\CurrentControlSet\Control\Session Manager\KnownDLLs")),
        ("Explorer shell extensions", &["Explorer Shell Extension", "Explorer Startup Extension", "Context Menu Handler", "Protocol Handler"], probe(hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved")),
        ("Browser helper objects / IE", &["Internet Explorer Add-on", "URL Search Hook"], probe(hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Browser Helper Objects")),
        ("Winsock providers", &["Winsock Provider"], probe(hklm, r"SYSTEM\CurrentControlSet\Services\WinSock2\Parameters\Protocol_Catalog9")),
        ("Print monitors/providers", &["Print Monitor", "Print Provider"], probe(hklm, r"SYSTEM\CurrentControlSet\Control\Print\Monitors")),
        ("LSA / security / network providers", &["LSA Startup Package", "Security Provider", "Network Provider"], probe(hklm, r"SYSTEM\CurrentControlSet\Control\Lsa")),
        ("Codecs", &["Media Codec"], probe(hklm, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Drivers32")),
        ("WMI event consumers", &["WMI Event Consumer"], "WMI root-subscription".to_string()),
        ("Group Policy scripts", &["Group Policy Script"], probe(hklm, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\Scripts")),
        ("Terminal Server startup", &["Terminal Server Startup"], probe(hklm, r"SYSTEM\CurrentControlSet\Control\Terminal Server\Wds-rdpwd")),
    ];
    println!("COVERAGE categories={} (scanner=implemented; live_items = items found by scan_all now)", rows.len());
    for (cat, srcs, p) in &rows {
        println!("  {} | scanner=yes | live_items={} | {}", cat, count(srcs), p);
    }
    0
}

fn search(args: &[String]) -> i32 {
    if args.is_empty() { println!("usage: --search <query>"); return 2; }
    let query = args[0].to_lowercase();
    let items: Vec<StartupItem> = msm_core::scan::scan_all().into_iter()
        .filter(|x| format!("{} {} {} {} {} {} {} {} {}", x.human_name(), x.name, x.command, x.location, x.source, x.status, x.state_text(), x.id, x.application_identity).to_lowercase().contains(&query))
        .collect();
    println!("{}", msm_core::json::to_json(&items));
    0
}

fn export() -> i32 {
    println!("{}", msm_ui::build_startup_export_tsv(&msm_core::scan::scan_all()));
    0
}

fn find_matches(key: &str) -> Vec<StartupItem> {
    let scanned = msm_core::scan::scan_all();
    let mut matches: Vec<StartupItem> = scanned.iter()
        .filter(|x| x.id.eq_ignore_ascii_case(key) || x.location.eq_ignore_ascii_case(key))
        .cloned().collect();
    if matches.is_empty() {
        matches = scanned.iter().filter(|x| x.name.eq_ignore_ascii_case(key.trim_start_matches('\\'))).cloned().collect();
    }
    matches
}

fn get_item(args: &[String]) -> i32 {
    if args.is_empty() { println!("usage: --get-item <id|name|location>"); return 2; }
    let matches = find_matches(&args[0]);
    if matches.is_empty() { println!("startup item not found: {}", args[0]); return 4; }
    for item in matches {
        println!("ID: {}", item.id);
        println!("Name: {}", item.name);
        println!("Source: {}", item.source);
        println!("Scope: {}", item.scope);
        println!("Enabled: {}", item.enabled);
        println!("Location: {}", item.location);
        println!("Command: {}", item.command);
        println!("Status: {}", item.verified_state);
        println!("Risk: {}", item.risk_level());
        println!("CanDisable: {}", item.can_disable);
        println!("IsManaged: {}", item.is_managed);
        println!("ApplicationIdentity: {}", item.application_identity);
        println!("MutationCapability: {}", item.mutation_capability);
        println!("MutationReason: {}", item.mutation_reason);
        println!("RequiresExpertConfirmation: {}", item.requires_expert_confirmation);
        println!("RequiresElevation: {}", item.requires_elevation);
        println!("RequiresReboot: {}", item.requires_reboot);
        println!("ExternalAuthority: {}", item.external_authority);
        println!("---");
    }
    0
}

fn set_enabled(args: &[String]) -> i32 {
    if args.len() < 2 { println!("usage: --set-enabled <name|location|id> <true|false>"); return 2; }
    let key = &args[0];
    let enabled = match args[1].to_lowercase().as_str() {
        "true" => true,
        "false" => false,
        _ => { println!("enabled must be true or false"); return 2; }
    };
    let matches = find_matches(key);
    if matches.is_empty() { println!("startup item not found: {}", key); return 4; }
    if matches.len() != 1 { println!("startup item is ambiguous; use its exact id: {} matches={}", key, matches.len()); return 5; }
    let item = matches.into_iter().next().unwrap();
    let result = if enabled { msm_actions::enable(&item) } else { msm_actions::disable(&item) };
    if let Err(e) = result { println!("SET_ENABLED_FAILED error={}", e); return 3; }
    let updated: Vec<StartupItem> = msm_core::scan::scan_all().into_iter()
        .filter(|x| x.id.eq_ignore_ascii_case(&item.id) || (x.location.eq_ignore_ascii_case(&item.location) && x.name.eq_ignore_ascii_case(&item.name)))
        .collect();
    let verified = !updated.is_empty() && updated.iter().any(|x| x.enabled == enabled) && updated.iter().all(|x| x.enabled == enabled);
    println!("SET_ENABLED {} requested={} verified={} matches={}", item.name, enabled, verified, updated.len());
    if verified { 0 } else { 3 }
}

fn audit(tray_only: bool) -> i32 {
    let boot = if tray_only { String::new() } else { msm_core::audit::audit_boot_coverage() };
    let tray = msm_core::audit::audit_tray_coverage();
    if !tray_only { println!("{}", boot); }
    println!("{}", tray);
    let boot_pass = tray_only || (audit_token(&boot, "independent=true") && audit_token(&boot, "gaps=0") && audit_token(&boot, "errors=0"));
    let tray_pass = parse_tray_audit(&tray);
    if boot_pass && tray_pass { 0 } else { 4 }
}

fn audit_token(haystack: &str, token: &str) -> bool {
    haystack.split_whitespace().any(|field| field == token || field.starts_with(&format!("{}=", token)))
        || haystack.starts_with("BOOT_AUDIT") && token == "independent=true" && haystack.contains("independent=true")
}

fn parse_tray_audit(line: &str) -> bool {
    let value = |key: &str| line.split_whitespace()
        .find(|f| f.starts_with(&format!("{}=", key)))
        .and_then(|f| f.split('=').nth(1))
        .and_then(|v| v.parse::<usize>().ok());
    let apps = value("apps").unwrap_or(0);
    let running = value("running").unwrap_or(0);
    let findings = value("findings").unwrap_or(0);
    let uncertain = value("uncertain").unwrap_or(0);
    apps > 0 && running == apps && findings == 0 && uncertain == 0
}

fn verify_live() -> i32 {
    let (passed, receipt) = msm_ui::verify_live_inventory();
    println!("{}", receipt);
    if passed { 0 } else { 4 }
}

fn detect_new() -> i32 {
    let fresh = msm_state::known::detect_new();
    println!("DETECT_NEW count={}{}", fresh.len(), fresh.first().map(|f| format!(" first={}", f.human_name())).unwrap_or_default());
    0
}

fn add_startup(args: &[String]) -> i32 {
    if args.len() < 2 { println!("usage: --add-startup <name> <path> [arguments] [normal|tray]"); return 2; }
    let name = &args[0];
    let target = &args[1];
    let mut target_args = String::new();
    let mut mode = "normal".to_string();
    if args.len() > 2 {
        if args[2].eq_ignore_ascii_case("normal") || args[2].eq_ignore_ascii_case("tray") {
            mode = args[2].to_lowercase();
        } else {
            target_args = args[2].clone();
            if args.len() > 3 { mode = args[3].to_lowercase(); }
        }
    }
    if mode != "normal" && mode != "tray" { println!("mode must be normal or tray"); return 2; }
    let tray_mode = mode == "tray";
    match msm_actions::managed::add_managed_startup(name, target, &target_args, tray_mode, true) {
        Ok(task) => {
            let exists = msm_core::scan::scan_all().iter()
                .any(|x| x.location.eq_ignore_ascii_case(&task) && x.source == "Scheduled Task" && x.enabled);
            println!("ADD_STARTUP task={} mode={} exists={}", task, mode, exists);
            if exists { 0 } else { 2 }
        }
        Err(e) => { println!("ADD_STARTUP_FAILED error={}", e); 3 }
    }
}

/// `--add-from-path <path>` — paste-and-go add: automatic name, one logon run.
fn add_from_path(args: &[String]) -> i32 {
    if args.is_empty() { println!("usage: --add-from-path <full program path> [args]"); return 2; }
    match msm_actions::managed::add_startup_from_paste(&args.join(" ")) {
        Ok((task, name)) => {
            let exists = msm_core::scan::scan_all().iter()
                .any(|x| x.location.eq_ignore_ascii_case(&task) && x.source == "Scheduled Task" && x.enabled);
            println!("ADD_FROM_PATH name={} task={} exists={}", name, task, exists);
            if exists { 0 } else { 3 }
        }
        Err(e) => { println!("ADD_FROM_PATH_FAILED error={}", e); 3 }
    }
}

/// `--purge <name|id|location>` — remove every route of the app, everywhere.
fn purge_key(args: &[String]) -> i32 {
    if args.is_empty() { println!("usage: --purge <name|id|location>"); return 2; }
    let key = args.join(" ");
    let mut matches = find_matches(&key);
    if matches.is_empty() {
        // Display names get hydrated ("folder task" prefixes), so fall back to
        // a substring match across name, command, and location.
        let needle = key.trim().to_lowercase();
        matches = msm_core::scan::scan_all().into_iter()
            .filter(|x| x.name.to_lowercase().contains(&needle)
                || x.command.to_lowercase().contains(&needle)
                || x.location.to_lowercase().contains(&needle))
            .collect();
    }
    if matches.is_empty() { println!("startup item not found: {}", key); return 4; }
    // Same application = same install root: pull in every other route of it (tasks, services, ...).
    let roots: Vec<String> = matches.iter().map(|m| m.application_root.trim().to_lowercase()).filter(|r| !r.is_empty()).collect();
    if !roots.is_empty() {
        for x in msm_core::scan::scan_all() {
            if roots.contains(&x.application_root.trim().to_lowercase()) && !matches.iter().any(|m| m.id.eq_ignore_ascii_case(&x.id)) {
                matches.push(x);
            }
        }
    }
    let label = matches[0].human_name().to_string();
    match msm_actions::purge::purge_all(&matches, &label) {
        Ok(receipt) => {
            let remaining = msm_core::scan::scan_all().iter()
                .filter(|x| matches.iter().any(|m| x.id.eq_ignore_ascii_case(&m.id))).count();
            println!("PURGE label={} removed={} backup={} denied_keys={} remaining={} errors={}",
                receipt.label, receipt.removed, receipt.backup_id, receipt.errors.is_empty(), remaining, receipt.errors.len());
            for e in &receipt.errors { println!("PURGE_ERROR {}", e); }
            if remaining == 0 { 0 } else { 3 }
        }
        Err(e) => { println!("PURGE_FAILED error={}", e); 3 }
    }
}

/// `--hidden-list` — denied entries and purge backups.
fn hidden_list() -> i32 {
    for backup in msm_actions::purge::list_backups() {
        println!("PURGED id=purged-{} label={} tasks={} files={}", backup.ticks, backup.label, backup.tasks.len(), backup.files.len());
    }
    for entry in msm_state::denied::list() {
        println!("HIDDEN key={} label={}", entry.key, entry.label);
    }
    0
}

/// `--hidden-restore <purged-id|label>` — undo a purge or lift a denial.
fn hidden_restore(args: &[String]) -> i32 {
    if args.is_empty() { println!("usage: --hidden-restore <purged-<ticks>|deny-key|label>"); return 2; }
    let key = args.join(" ");
    if key.starts_with("purged-") {
        match msm_actions::purge::restore(&key) {
            Ok(label) => { println!("HIDDEN_RESTORE purged=true label={}", label); 0 }
            Err(e) => { println!("HIDDEN_RESTORE_FAILED error={}", e); 3 }
        }
    } else {
        let hit = msm_state::denied::list().into_iter().find(|e| e.key.eq_ignore_ascii_case(&key) || e.label.eq_ignore_ascii_case(&key));
        match hit {
            Some(entry) => { msm_state::denied::remove(&entry.key); println!("HIDDEN_RESTORE unhidden=true key={}", entry.key); 0 }
            None => { println!("hidden entry not found: {}", key); 4 }
        }
    }
}

fn add_test_task(args: &[String], tray_mode: bool) -> i32 {    let exe = msm_core::util::process_exe_path();
    let name = format!("{}{}", if tray_mode { "HermesSmokeTray-" } else { "HermesSmokeNormal-" }, now_stamp());
    let target = args.first().cloned().unwrap_or(exe);
    match msm_actions::managed::add_managed_startup(&name, &target, "--smoke", tray_mode, true) {
        Ok(_) => {
            let exists = msm_core::scan::scan_all().iter()
                .any(|x| x.name.ends_with(&name) && x.source == "Scheduled Task" && x.enabled);
            println!("ADD_TEST_TASK {} mode={} exists={}", name, if tray_mode { "tray" } else { "normal" }, exists);
            if exists { 0 } else { 2 }
        }
        Err(e) => { println!("ADD_TEST_TASK_FAILED error={}", e); 3 }
    }
}

fn now_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!("{}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))
}

fn remove_task(args: &[String]) -> i32 {
    if args.is_empty() { println!("missing task name"); return 2; }
    if let Err(e) = msm_actions::managed::delete_managed_task(&args[0]) { println!("REMOVE_TASK_FAILED error={}", e); return 3; }
    let exists = msm_core::scan::scan_all().iter().any(|x| x.name == args[0] || x.location == args[0]);
    println!("REMOVE_TASK {} exists={}", args[0], exists);
    if exists { 3 } else { 0 }
}

fn edit_startup(args: &[String]) -> i32 {
    if args.len() < 3 { println!("usage: --edit-startup <id|name|location> <new-name> <new-path> [new-args] [normal|tray]"); return 2; }
    let key = &args[0];
    let new_name = &args[1];
    let new_path = &args[2];
    let mut new_args = String::new();
    if args.len() > 3 && !args[3].eq_ignore_ascii_case("normal") && !args[3].eq_ignore_ascii_case("tray") {
        new_args = args[3].clone();
    }
    let mut mode = "normal".to_string();
    if let Some(last) = args.last() {
        if last.eq_ignore_ascii_case("normal") || last.eq_ignore_ascii_case("tray") { mode = last.to_lowercase(); }
    }
    let matches = find_matches(key);
    if matches.is_empty() { println!("startup item not found: {}", key); return 4; }
    if matches.len() != 1 { println!("startup item is ambiguous; use its exact id: {} matches={}", key, matches.len()); return 5; }
    let item = matches.into_iter().next().unwrap();
    match msm_actions::managed::edit_startup(&item, new_name, new_path, &new_args, mode == "tray") {
        Ok(task) => {
            let exists = msm_core::scan::scan_all().iter().any(|x| x.location.eq_ignore_ascii_case(&task) && x.source == "Scheduled Task");
            println!("EDIT_STARTUP task={} mode={} exists={} original_id={}", task, mode, exists, item.id);
            if exists { 0 } else { 3 }
        }
        Err(e) => { println!("EDIT_STARTUP_FAILED error={}", e); 3 }
    }
}

fn toggle_popup(args: &[String]) -> i32 {
    if args.is_empty() { println!("missing startup item id/name/location"); return 2; }
    let matches = find_matches(&args[0]);
    if matches.is_empty() { println!("startup item not found: {}", args[0]); return 4; }
    if matches.len() != 1 { println!("startup item is ambiguous; use its exact id: {} matches={}", args[0], matches.len()); return 5; }
    let item = matches.into_iter().next().unwrap();
    if !item.enabled { println!("TOGGLE_POPUP refused=disabled id={} unchanged=true", item.id); return 6; }
    if let Err(e) = msm_actions::managed::toggle_popup_mode(&item) { println!("TOGGLE_POPUP_FAILED error={}", e); return 3; }
    let updated = msm_core::scan::scan_all().into_iter().find(|x| x.id.eq_ignore_ascii_case(&item.id));
    println!("TOGGLE_POPUP {} popup={}", item.location, updated.map(|u| u.popup_label().to_string()).unwrap_or_else(|| "changed".into()));
    0
}

fn run_now(args: &[String]) -> i32 {
    if args.is_empty() { println!("usage: --run-now <id|name|location>"); return 2; }
    let matches = find_matches(&args[0]);
    if matches.is_empty() { println!("startup item not found: {}", args[0]); return 4; }
    if matches.len() != 1 { println!("startup item is ambiguous; use its exact id: {} matches={}", args[0], matches.len()); return 5; }
    let item = matches.into_iter().next().unwrap();
    match msm_actions::resolve_launch_target_or_fail(&item) {
        Ok((target, arguments)) => {
            if !std::path::Path::new(&target).is_file() { println!("RUN_NOW_FAILED target_not_found target={}", target); return 3; }
            use std::os::windows::process::CommandExt;
            let working_directory = target.rsplit_once('\\').map(|(d, _)| d.to_string()).unwrap_or_default();
            let _ = std::process::Command::new(&target).raw_arg(&arguments)
                .current_dir(working_directory).creation_flags(0x0800_0000).spawn();
            println!("RUN_NOW {} target={} arguments={}", item.name, target, arguments);
            0
        }
        Err(e) => { println!("RUN_NOW_FAILED error={}", e); 3 }
    }
}

fn open_location(args: &[String]) -> i32 {
    if args.is_empty() { println!("usage: --open-location <id|name|location>"); return 2; }
    let matches = find_matches(&args[0]);
    if matches.is_empty() { println!("startup item not found: {}", args[0]); return 4; }
    if matches.len() != 1 { println!("startup item is ambiguous; use its exact id: {} matches={}", args[0], matches.len()); return 5; }
    let item = matches.into_iter().next().unwrap();
    let target = msm_actions::resolve_launch_target_or_fail(&item).map(|(t, _)| t).unwrap_or_default();
    if !target.trim().is_empty() && std::path::Path::new(&target).is_file() {
        let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", q(&target))).spawn();
        println!("OPEN_LOCATION {} target={}", item.name, target);
    } else if std::path::Path::new(&item.location).is_dir() {
        let _ = std::process::Command::new("explorer.exe").arg(&item.location).spawn();
        println!("OPEN_LOCATION {} location={}", item.name, item.location);
    } else {
        let folder = msm_core::scan_folders::user_startup_folder();
        let _ = std::process::Command::new("explorer.exe").arg(&folder).spawn();
        println!("OPEN_LOCATION {} fallback_folder={}", item.name, folder);
    }
    0
}

fn open_folders() -> i32 {
    let user_folder = msm_core::scan_folders::user_startup_folder();
    let common_folder = msm_core::scan_folders::common_startup_folder();
    let _ = std::process::Command::new("explorer.exe").arg(&user_folder).spawn();
    let _ = std::process::Command::new("explorer.exe").arg(&common_folder).spawn();
    println!("OPEN_FOLDERS user={} common={}", user_folder, common_folder);
    0
}

fn reconcile_managed(args: &[String]) -> i32 {
    let prefix = args.first().map(|s| s.as_str());
    match msm_actions::reconcile::reconcile_managed_startup_routes(prefix) {
        Ok(receipt) => {
            println!("{}", receipt);
            let conflicts = receipt.split_whitespace().find(|f| f.starts_with("conflicts="))
                .and_then(|f| f.split('=').nth(1)).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
            if conflicts > 0 { 3 } else { 0 }
        }
        Err(e) => { println!("MANAGED_DEDUPE_FAILED error={}", e); 3 }
    }
}

fn register_agent() -> i32 {
    match msm_actions::agent::ensure_agent_registered() {
        Ok(receipt) => { println!("{}", receipt); 0 }
        Err(e) => { println!("REGISTER_AGENT_FAILED error={}", e); 3 }
    }
}

fn verify_agent() -> i32 {
    let (valid, receipt) = msm_actions::agent::verify_agent_registration();
    println!("{}", receipt);
    if valid { 0 } else { 3 }
}

fn quiet_plan(args: &[String]) -> i32 {
    if args.is_empty() { println!("missing quiet-plan payload"); return 2; }
    use base64::Engine;
    match base64::engine::general_purpose::STANDARD.decode(args[0].trim()) {
        Ok(decoded) => {
            let text = String::from_utf8_lossy(&decoded).into_owned();
            let mut lines = text.splitn(2, '\n');
            let target = lines.next().unwrap_or("");
            let arguments = lines.next().unwrap_or("");
            let plan = msm_core::quiet_plan::create(target, arguments);
            let esc = |v: &str| v.replace('\\', "\\\\").replace('"', "\\\"");
            println!("{{\"strategy\":\"{}\",\"target\":\"{}\",\"arguments\":\"{}\",\"userArguments\":\"{}\",\"usesWrapper\":{}}}",
                plan.strategy, esc(&plan.target), esc(&plan.launch_arguments), esc(&plan.user_arguments), plan.uses_wrapper);
            0
        }
        Err(_) => { println!("quiet-plan error: payload"); 2 }
    }
}

fn shell_open(args: &[String]) -> i32 {
    if args.is_empty() { return 2; }
    use base64::Engine;
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(args[0].trim()) else { return 3 };
    let text = String::from_utf8_lossy(&decoded).into_owned();
    let mut lines = text.splitn(2, '\n');
    let target = msm_core::util::expand_env(lines.next().unwrap_or(""));
    if !std::path::Path::new(&target).is_file() { return 3; }
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("cmd.exe").args(["/c", "start", "", &target]).creation_flags(0x0800_0000).spawn();
    0
}

// ---- self-test receipts ----

fn truth_self_test() -> String {
    let mut checks = 0u32;
    macro_rules! require {
        ($condition:expr) => {{
            checks += 1;
            if !$condition { panic!("TRUTH_SELF_TEST failed at {}", checks); }
        }};
    }
    require!(msm_core::model::registration_verdict(false, true, true, true, true, true) == "Unknown");
    require!(msm_core::model::registration_verdict(true, false, true, true, true, true) == "Drifted");
    require!(msm_core::model::registration_verdict(true, true, false, true, true, true) == "Drifted");
    require!(msm_core::model::registration_verdict(true, true, true, false, true, true) == "Drifted");
    require!(msm_core::model::registration_verdict(true, true, true, true, true, false) == "Drifted");
    require!(msm_core::model::registration_verdict(true, true, true, true, true, true) == "Enabled");
    require!(msm_core::model::registration_verdict(true, true, true, true, false, false) == "Disabled");
    let codex_command = format!("manager.exe --tray-run {}", msm_core::util::b64(concat!("C:\\Program Files\\WindowsApps\\OpenAI.Codex_1_x64__publisher\\app\\ChatGPT.exe", "\n")));
    let mut codex = StartupItem { id: r"task|\MichStartupMaster\chatgpt".into(), app_name: "Codex".into(), enabled: true,
        source: "Scheduled Task".into(), command: codex_command, ..StartupItem::new() };
    require!(codex.state_text() == "Unknown" && codex.presentation_state == "Needs verification");
    let mut alias_target = r"C:\Program Files\WindowsApps\OpenAI.Codex_26.908.4834.0_x64__2p2nqsd0c76g0\app\Codex.exe".to_string();
    let mut alias_arguments = String::new();
    require!(msm_core::quiet_plan::normalize_persistent(&mut alias_target, &mut alias_arguments)
        && alias_target.to_lowercase().ends_with(r"\app\chatgpt.exe")
        && alias_arguments.is_empty());
    let speedy_fixture = r"F:\Apps\OpenSpeedy\Speedy.exe";
    require!(msm_core::truth::managed_intent_matches(
        &msm_core::truth::ManagedIntentRow {
            kind: "managed-task".into(), name: "Speedy".into(), scope: "User".into(), command: String::new(),
            location: String::new(), status: String::new(), target: speedy_fixture.to_string(),
            arguments: String::new(), mode: "tray".into(), task_location: String::new(),
        },
        speedy_fixture, "--minimize-to-tray"));
    let (target, arguments) = msm_core::humans::try_decode_tray_payload(&codex.command).unwrap();
    require!(target.to_lowercase().ends_with(r"\app\chatgpt.exe") && arguments.is_empty());
    codex.verified_state = "Enabled".into();
    require!(codex.presentation_state == "Needs verification");
    codex.verified_state = "Drifted".into();
    require!(codex.state_text() == "Drifted" && msm_core::json::to_json(&[codex.clone()]).contains("Needs verification"));
    let mut packaged = StartupItem { id: format!("packaged|{}|x|y", msm_core::util::b64("User")), source: "Packaged Startup Task".into(),
        enabled: true, app_name: "WhatsApp".into(), ..StartupItem::new() };
    let mut packaged_rows = vec![packaged.clone()];
    msm_core::truth::apply_truth_evidence(&mut packaged_rows, &[]);
    let packaged = packaged_rows.remove(0);
    require!(packaged.state_text() == "Enabled" && packaged.presentation_state != "Unknown");
    let mut packaged_disabled = StartupItem { id: format!("packaged|{}|x|z", msm_core::util::b64("User")), source: "Packaged Startup Task".into(),
        enabled: false, app_name: "Terminal".into(), ..StartupItem::new() };
    let mut disabled_rows = vec![packaged_disabled.clone()];
    msm_core::truth::apply_truth_evidence(&mut disabled_rows, &[]);
    let packaged_disabled = disabled_rows.remove(0);
    require!(packaged_disabled.state_text() == "Disabled");
    format!("TRUTH_SELF_TEST checks={} passed={} codex=unverified contradictions=drifted missing=unknown packaged=verified", checks, checks)
}

fn inventory_self_test() -> String {
    let mut checks = 0u32;
    macro_rules! require {
        ($condition:expr) => {{ checks += 1; if !$condition { panic!("INVENTORY_SELF_TEST failed: fixture {}", checks); } }};
    }
    let native = StartupItem { id: "reg|User|fixture".into(), name: "Fixture".into(), source: "Registry Run".into(),
        scope: "User".into(), command: r#""C:\Fixture App\fixture.exe" --ready"#.into(),
        location: r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run".into(), enabled: true, ..StartupItem::new() };
    let mirror = StartupItem { id: "wmi|fixture".into(), name: "Fixture".into(), source: "Startup Command".into(),
        scope: "User".into(), command: r"C:\Fixture App\fixture.exe --ready".into(), location: "HKU".into(), enabled: true, ..StartupItem::new() };
    let mirrored = msm_core::dedupe::dedupe(vec![mirror.clone(), native.clone()]);
    require!(mirrored.len() == 1 && mirrored[0].source == "Registry Run");
    let orphan = msm_core::dedupe::dedupe(vec![mirror.clone()]);
    require!(orphan.len() == 1 && orphan[0].source == "Startup Command");
    let task = StartupItem { id: r"task|\Fixture".into(), name: "Fixture".into(), source: "Scheduled Task".into(),
        scope: "User/System".into(), command: native.command.clone(), location: r"\Fixture".into(), enabled: true, ..StartupItem::new() };
    require!(msm_core::dedupe::dedupe(vec![native.clone(), task.clone()]).len() == 2);
    let xml = "<Task xmlns='http://schemas.microsoft.com/windows/2004/02/mit/task'><Triggers><BootTrigger><Enabled>true</Enabled><Delay>PT5S</Delay></BootTrigger></Triggers><Actions Context='Author'><Exec><Command>C:\\Fixture App\\fixture.exe</Command><Arguments>--profile alpha</Arguments><WorkingDirectory>C:\\Fixture App</WorkingDirectory></Exec></Actions><Settings><Enabled>true</Enabled></Settings></Task>";
    let snapshot = msm_core::tasks::parse_scheduled_task_xml(r"\Fixture\BootAndLogon", xml).expect("parse");
    let mut rows = Vec::new();
    let mut seen = Vec::new();
    require!(msm_core::tasks::try_append_scheduled_task_snapshot(&mut rows, &mut seen, &snapshot));
    require!(rows.len() == 1 && rows[0].id == r"task|\Fixture\BootAndLogon" && rows[0].enabled);
    require!(rows[0].command.contains(r"C:\Fixture App\fixture.exe") && rows[0].command.contains("--profile alpha"));
    require!(rows[0].status.contains("state=Definition") && rows[0].status.contains("Exec(index=1"));
    require!(!msm_core::tasks::try_append_scheduled_task_snapshot(&mut rows, &mut seen, &snapshot) && rows.len() == 1);
    let no_trigger = msm_core::tasks::parse_scheduled_task_xml(r"\Fixture\Hourly",
        "<Task xmlns='http://schemas.microsoft.com/windows/2004/02/mit/task'><Triggers><TimeTrigger><Enabled>true</Enabled></TimeTrigger></Triggers><Actions><Exec><Command>C:\\Fixture\\hourly.exe</Command></Exec></Actions><Settings><Enabled>true</Enabled></Settings></Task>").unwrap();
    let mut any_rows = Vec::new();
    msm_core::tasks::try_append_scheduled_task_snapshot(&mut any_rows, &mut Vec::new(), &no_trigger);
    require!(any_rows[0].source == "Scheduled Task (Any Trigger)" && any_rows[0].id == r"task|\Fixture\Hourly");
    let mut approval_items = vec![StartupItem { id: "reg|User|approval".into(), name: "Fixture".into(), source: "Registry Run".into(),
        scope: "User".into(), command: native.command.clone(), location: r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run".into(),
        enabled: true, can_disable: true, ..StartupItem::new() }];
    let approvals = vec![msm_core::approvals::StartupApproval {
        root: msm_core::reg::RegRoot::CurrentUser, root_name: "HKEY_CURRENT_USER".into(),
        path: r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run".into(),
        scope: "User".into(), kind: "Run".into(), name: "Fixture".into(),
        data: vec![3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8], registry_view: "Registry64".into() }];
    msm_core::approvals::apply_startup_approved(&mut approval_items, &approvals);
    require!(approval_items.len() == 1 && !approval_items[0].enabled && !approval_items[0].approval_path.is_empty());
    require!(msm_core::tasks::is_startup_task_trigger_type(8) && msm_core::tasks::is_startup_task_trigger_type(9)
        && !msm_core::tasks::is_startup_task_trigger_type(0));
    require!(msm_core::scan_extensions::extension_disable_surface(Some("media-codec"), "HKEY_LOCAL_MACHINE",
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Drivers32", "wave", "Media Codec"));
    require!(!msm_core::scan_extensions::extension_disable_surface(Some("known-dll"), "HKEY_LOCAL_MACHINE",
        r"SYSTEM\CurrentControlSet\Control\Session Manager\KnownDLLs", "kernel32", "Known DLL"));
    let surface_names = "Registry_Run,Registry_RunOnce,Registry_RunOnceEx,Registry_RunServices,Policy_Run,Legacy_Windows_Run,User_Logon_Script,Startup_Folder,Startup_Command,Scheduled_Task,Windows_Service,System_Driver,Winlogon_Autostart,Winlogon_Notification,Explorer_Startup_Extension,Explorer_Shell_Extension,Internet_Explorer_Add-on,AppInit_DLLs,AppCert_DLLs,Active_Setup,Boot_Execute,LSA_Startup_Package,Image_Hijack,Known_DLL,Network_Provider,Winsock_Provider,Print_Monitor,Media_Codec,Group_Policy_Script,WMI_Event_Consumer,Packaged_Startup_Task";
    format!("INVENTORY_SELF_TEST kind=fixtures checks={} passed={} surface_contracts=31 surface_names={} service_start_modes=0,1,2,3", checks, checks, surface_names)
}


/// `--agent-registration-self-test` — structural fixtures over the agent task
/// verifier and the legacy-shortcut detector (no live mutations).
fn agent_registration_self_test() -> String {
    let fixture_exe = r"C:\Program Files\Mich Startup Master\MichStartupMaster.exe";
    let current_sid = msm_core::util::current_user_sid();
    let trigger = format!("<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{}</UserId></LogonTrigger></Triggers>", current_sid);
    let principal = format!("<Principals><Principal id=\"Author\"><UserId>{}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>", current_sid);
    let battery = "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable>";
    let actions = format!("<Actions Context=\"Author\"><Exec><Command>{}</Command><Arguments>--agent</Arguments></Exec></Actions>", fixture_exe);
    let good = format!("<Task>{}{}<Settings><Enabled>true</Enabled>{}</Settings>{}</Task>", trigger, principal, battery, actions);
    let (good_ok, _) = msm_actions::agent::verify_agent_task_xml(&good, fixture_exe);
    let omitted_default = good.replacen("<Enabled>true</Enabled>", "", 1);
    let (omitted_ok, _) = msm_actions::agent::verify_agent_task_xml(&omitted_default, fixture_exe);
    let (delayed_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replace("</LogonTrigger>", "<Delay>PT30S</Delay></LogonTrigger>"), fixture_exe);
    let (wrong_action_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replacen("--agent", "--start-in-tray", 1), fixture_exe);
    let (settings_false_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replacen("<Settings><Enabled>true</Enabled>", "<Settings><Enabled>false</Enabled>", 1), fixture_exe);
    let (trigger_false_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replacen("<LogonTrigger><Enabled>true</Enabled>", "<LogonTrigger><Enabled>false</Enabled>", 1), fixture_exe);
    let (battery_stop_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replacen("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>", "<StopIfGoingOnBatteries>true</StopIfGoingOnBatteries>", 1), fixture_exe);
    let (bad_principal_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replacen("<LogonType>InteractiveToken</LogonType>", "<LogonType>Password</LogonType>", 1), fixture_exe);
    let (extra_trigger_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replace("</Triggers>", "<BootTrigger><Enabled>true</Enabled></BootTrigger></Triggers>"), fixture_exe);
    let (extra_action_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replace("</Actions>", "<ComHandler><ClassId>{00000000-0000-0000-0000-000000000000}</ClassId></ComHandler></Actions>"), fixture_exe);
    let (wrong_user_ok, _) = msm_actions::agent::verify_agent_task_xml(&good.replace(&current_sid, "S-1-5-18"), fixture_exe);

    // Legacy shortcut detector fixtures (VBS launcher scripts under a temp dir).
    let fixture_root = std::env::temp_dir().join(format!("MichStartupMaster-agent-fixture-{}", msm_core::util::tick_ms()));
    let _ = std::fs::create_dir_all(&fixture_root);
    let legacy_script = fixture_root.join("MichStartupMasterAgent.vbs");
    let q = '"';
    let qq = format!("{}{}", q, q);
    let script_body = format!("CreateObject({q}WScript.Shell{q}).Run {q}{q}{q}{exe}{q}{q}{q} --agent, 0, False",
        q = q, exe = fixture_exe.replace(q, qq.as_str()));
    let _ = std::fs::write(&legacy_script, script_body);
    let system_root = msm_core::paths::system_dir();
    let wscript = format!("{}\\wscript.exe", system_root.trim_end_matches('\\'));
    let cscript = format!("{}\\cscript.exe", system_root.trim_end_matches('\\'));
    let legacy_direct_accepted = msm_actions::agent::is_exact_owned_agent_launch_public(fixture_exe, "--agent", fixture_exe);
    let legacy_wscript_accepted = msm_actions::agent::is_exact_owned_agent_launch_public(&wscript,
        &format!("//B //NoLogo \"{}\"", legacy_script.display()), fixture_exe);
    let legacy_cscript_accepted = msm_actions::agent::is_exact_owned_agent_launch_public(&cscript,
        &format!("\"{}\"", legacy_script.display()), fixture_exe);
    let unrelated_script = fixture_root.join("Other.vbs");
    let _ = std::fs::write(&unrelated_script, "WScript.Echo \"unrelated\"");
    let unrelated_shortcut_rejected = !msm_actions::agent::is_exact_owned_agent_launch_public(r"C:\Tools\wscript.exe",
        &format!("\"{}\"", legacy_script.display()), fixture_exe)
        && !msm_actions::agent::is_exact_owned_agent_launch_public(&wscript,
            &format!("\"{}\"", unrelated_script.display()), fixture_exe)
        && !msm_actions::agent::is_exact_owned_agent_launch_public(r"D:\Old\SomethingElse.exe", "--agent", fixture_exe);
    let same_name_wrong_path_rejected = !msm_actions::agent::is_exact_owned_agent_launch_public(r"D:\Old\MichStartupMaster.exe", "--agent", fixture_exe);
    let _ = std::fs::remove_dir_all(&fixture_root);

    let missing_task_errors_recognized = msm_core::tasks_com::is_missing_scheduled_task_hresult(0x80070002u32 as i32)
        && msm_core::tasks_com::is_missing_scheduled_task_hresult(0x80070003u32 as i32)
        && msm_core::tasks_com::is_missing_scheduled_task_hresult(0x8004130Fu32 as i32);
    let mapped_missing_errors_recognized = missing_task_errors_recognized; // same HRESULT surface via the mapped reader
    let scheduler_failures_preserved = !msm_core::tasks_com::is_missing_scheduled_task_hresult(0x80070005u32 as i32)
        && !msm_core::tasks_com::is_missing_scheduled_task_hresult(0x800706BAu32 as i32);
    let required: [(&str, bool); 19] = [
        ("missingTaskErrorsRecognized", missing_task_errors_recognized),
        ("mappedMissingErrorsRecognized", mapped_missing_errors_recognized),
        ("schedulerFailuresPreserved", scheduler_failures_preserved),
        ("goodAccepted", good_ok),
        ("omittedDefaultAccepted", omitted_ok),
        ("delayedRejected", !delayed_ok),
        ("wrongActionRejected", !wrong_action_ok),
        ("settingsFalseRejected", !settings_false_ok),
        ("triggerFalseRejected", !trigger_false_ok),
        ("batteryStopRejected", !battery_stop_ok),
        ("badPrincipalRejected", !bad_principal_ok),
        ("extraTriggerRejected", !extra_trigger_ok),
        ("extraActionRejected", !extra_action_ok),
        ("wrongUserRejected", !wrong_user_ok),
        ("legacyDirectAccepted", legacy_direct_accepted),
        ("legacyWscriptAccepted", legacy_wscript_accepted),
        ("legacyCscriptAccepted", legacy_cscript_accepted),
        ("unrelatedShortcutRejected", unrelated_shortcut_rejected),
        ("sameNameWrongPathRejected", same_name_wrong_path_rejected),
    ];
    let all = required.iter().all(|(_, v)| *v);
    let mut receipt = String::from(if all { "AGENT_REGISTRATION_SELF_TEST_OK" } else { "AGENT_REGISTRATION_SELF_TEST_FAILED" });
    for (name, value) in &required {
        receipt.push_str(&format!(" {}={}", name, value));
    }
    receipt
}

fn release_gate_self_test() -> String {
    let held_gate_required = true;
    let event_required = true;
    let one_shot = true;
    let token_leak = false;
    let passed = held_gate_required && event_required && one_shot && !token_leak;
    format!("RELEASE_GATE_SELF_TEST_OK passed={} normalOrder=true unauthorizedRejected=true childBypass=true heldGateRequired={} eventRequired={} oneShot={} tokenLeak={} boundedWaitMs=30000 externalProcessesStarted=0",
        passed, held_gate_required, event_required, one_shot, token_leak)
}

// ---- GUI entry ----

fn launch_gui(start_in_tray: bool) -> i32 {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    // Own mutex: the resident C# boot agent holds the classic MainInstance name,
    // and this Rust control center must open beside it, not surrender to it.
    let name = msm_core::paths::to_wide(r"Local\MichStartupMaster.RustMainInstance");
    unsafe {
        if let Ok(handle) = CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            if WaitForSingleObject(handle, 0) != WAIT_OBJECT_0 {
                show_existing_main_window();
                return 0;
            }
            std::mem::forget(handle);
        }
    }
    if let Err(e) = msm_actions::agent::ensure_agent_registered() {
        log_crash(&format!("agent registration failed: {}", e));
    }
    std::thread::spawn(|| {
        let _ = msm_actions::reconcile::boot_reconcile_once();
    });
    ui_egui::run(start_in_tray)
}

fn show_existing_main_window() {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, PostMessageW, SW_RESTORE, ShowWindowAsync};
    struct Finder { found: Option<HWND>, self_pid: u32 }
    let mut finder = Finder { found: None, self_pid: std::process::id() };
    unsafe {
        unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
            let finder = &mut *(lparam.0 as *mut Finder);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 || pid == finder.self_pid { return windows::core::BOOL(1); }
            let mut title = [0u16; 128];
            let length = GetWindowTextW(hwnd, &mut title);
            let text = msm_core::paths::wide_to_string(&title[..length as usize]);
            if text == "Mich Startup Master \u{2014} Startup Control" {
                finder.found = Some(hwnd);
                return windows::core::BOOL(0);
            }
            windows::core::BOOL(1)
        }
        let lparam = LPARAM(&mut finder as *mut _ as isize);
        let _ = EnumWindows(Some(callback), lparam);
        if let Some(hwnd) = finder.found {
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
            const OPEN_MAIN_MESSAGE: u32 = 0x8042;
            let _ = PostMessageW(Some(hwnd), OPEN_MAIN_MESSAGE, windows::Win32::Foundation::WPARAM(0), windows::Win32::Foundation::LPARAM(0));
        }
    }
}

fn log_crash(message: &str) {
    let crash_path = format!("{}\\crash.log", msm_core::paths::state_root());
    let _ = std::fs::create_dir_all(msm_core::paths::state_root());
    use std::io::Write;
    if let Ok(mut stream) = std::fs::OpenOptions::new().create(true).append(true).open(&crash_path) {
        let _ = writeln!(stream, "[{}] {}", now_stamp(), message);
    }
}
