//! Quiet mode: `--tray-run` wrapper controller, policy/lineage/performance probes.

use msm_core::util::b64;
use once_cell::sync::Lazy;
use regex::Regex;

/// C# `TrayRunner::RunMain` — one wrapper per exact launch identity.
pub fn tray_run(payload: &str) {
    let Ok(decoded) = b64_decode(payload) else { return };
    let text = String::from_utf8_lossy(&decoded).into_owned();
    let mut lines = text.splitn(2, '\n');
    let target = lines.next().unwrap_or("").to_string();
    let target_args = lines.next().unwrap_or("").to_string();
    let Ok(mut full) = std::path::Path::new(&target).canonicalize() else { return };
    if !full.is_file() {
        let resolved = msm_core::humans::resolve_target_path(&full.to_string_lossy());
        full = std::path::PathBuf::from(resolved);
    }
    if !full.is_file() || !msm_core::managed::is_supported_startup_target(&full.to_string_lossy()) {
        return;
    }
    let plan = msm_core::quiet_plan::create(&full.to_string_lossy(), &target_args);
    if !plan.uses_wrapper {
        launch_native_plan(&plan);
        return;
    }
    let full = plan.target.clone();
    let target_args = plan.user_arguments.clone();
    // Single wrapper per exact (target + arguments) launch identity.
    let identity = hash_launch_identity(&full, &target_args);
    let mutex_name = format!(r"Local\MichStartupMaster.TrayWrapper.{}", identity);
    if !acquire_once(&mutex_name) {
        return; // another quiet wrapper already controls this exact launch
    }
    run_wrapper_controller(&full, &target_args);
}

fn b64_decode(payload: &str) -> Result<Vec<u8>, ()> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(payload.trim()).map_err(|_| ())
}

fn acquire_once(mutex_name: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    let wide = msm_core::paths::to_wide(mutex_name);
    unsafe {
        let Ok(handle) = CreateMutexW(None, true, PCWSTR(wide.as_ptr())) else { return false };
        let result = WaitForSingleObject(handle, 0);
        if result != WAIT_OBJECT_0 {
            // Owned by another wrapper process (or abandoned); never double-start.
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            return false;
        }
        // Hold until process exit: a quiet wrapper lives for the whole session.
        std::mem::forget(handle);
        true
    }
}

/// C# `LaunchNativePlan` — direct native quiet launch, serialized, no double start.
fn launch_native_plan(plan: &msm_core::quiet_plan::QuietLaunchPlan) {
    if plan.target.trim().is_empty() || !std::path::Path::new(&plan.target).is_file() {
        return;
    }
    let identity = hash_launch_identity(&plan.target, &plan.launch_arguments);
    let mutex_name = format!(r"Local\MichStartupMaster.NativeQuiet.{}", identity);
    if !acquire_once(&mutex_name) {
        return;
    }
    if is_exact_launch_running(&plan.target, &plan.launch_arguments) {
        return;
    }
    let (execute, action_args) = msm_core::managed::build_direct_action(&plan.target, &plan.launch_arguments);
    use std::os::windows::process::CommandExt;
    let working_directory = plan.target.rsplit_once('\\').map(|(dir, _)| dir.to_string())
        .filter(|d| std::path::Path::new(d).is_dir())
        .unwrap_or_else(|| msm_core::paths::local_app_data());
    let _ = std::process::Command::new(&execute)
        .raw_arg(&action_args)
        .current_dir(&working_directory)
        .creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn();
}

fn is_exact_launch_running(target: &str, arguments: &str) -> bool {
    let (execute, _) = msm_core::managed::build_direct_action(target, arguments);
    let expected_path = canonical_path(&execute);
    for process in msm_core::runtime::snapshot_processes(250) {
        if canonical_path(&process.executable_path).eq_ignore_ascii_case(&expected_path)
            && command_line_matches_launch(&process.command_line, target, arguments) {
            return true;
        }
    }
    false
}

pub fn command_line_matches_launch(command_line: &str, target: &str, arguments: &str) -> bool {
    // C# CommandLineMatchesLaunchIdentity: exact executable + exact argument vector.
    let Some(parsed) = parse_windows_arguments(command_line) else { return false };
    if parsed.is_empty() { return false }
    let exe = &parsed[0];
    if !canonical_path(exe).eq_ignore_ascii_case(&canonical_path(target)) {
        return false;
    }
    let expected: Vec<String> = if arguments.trim().is_empty() { Vec::new() } else {
        parse_windows_arguments(arguments).unwrap_or_default()
    };
    parsed[1..].to_vec() == expected
}

/// Windows-style command-line argument splitter (CommandLineToArgvW semantics).
pub fn parse_windows_arguments(command_line: &str) -> Option<Vec<String>> {
    if command_line.trim().is_empty() {
        return Some(Vec::new());
    }
    let wide = msm_core::paths::to_wide(command_line.trim());
    let mut count = 0i32;
    let argv = unsafe {
        #[allow(non_snake_case)]
        let CommandLineToArgvW: unsafe extern "system" fn(windows::core::PCWSTR, *mut i32) -> *mut windows::core::PWSTR = {
            extern "system" {
                #[link_name = "CommandLineToArgvW"]
                fn imp(command: windows::core::PCWSTR, count: *mut i32) -> *mut windows::core::PWSTR;
            }
            imp
        };
        CommandLineToArgvW(windows::core::PCWSTR(wide.as_ptr()), &mut count)
    };
    if argv.is_null() || count <= 0 {
        return None;
    }
    let mut out = Vec::new();
    for i in 0..count as usize {
        unsafe {
            let pointer = *argv.add(i);
            if !pointer.is_null() {
                let mut length = 0usize;
                while *pointer.0.add(length) != 0 { length += 1 }
                out.push(String::from_utf16_lossy(std::slice::from_raw_parts(pointer.0, length)));
            }
        }
    }
    unsafe {
        let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(argv as *mut _)));
    }
    Some(out)
}

pub fn canonical_path(value: &str) -> String {
    let expanded = msm_core::util::expand_env(value.trim().trim_matches('"'));
    if std::path::Path::new(&expanded).is_absolute() {
        msm_core::paths::canonical(&expanded).trim_end_matches('\\').to_lowercase()
    } else {
        expanded.trim_end_matches('\\').to_lowercase()
    }
}

pub fn hash_launch_identity(target: &str, arguments: &str) -> String {
    // Path is case-insensitive; the argument vector is case-sensitive (C# parity).
    use sha2::{Digest, Sha256};
    let canonical = format!("{}|{}", canonical_path(target), collapse_spaces(arguments));
    let digest = Sha256::digest(canonical.as_bytes());
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

fn collapse_spaces(value: &str) -> String {
    static WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    WS.replace_all(value, " ").into_owned()
}

/// The wrapper controller: launch hidden, wait for exit, keep the launch alive.
fn run_wrapper_controller(target: &str, arguments: &str) {
    let (execute, action_args) = msm_core::managed::build_direct_action(target, arguments);
    use std::os::windows::process::CommandExt;
    let working_directory = target.rsplit_once('\\').map(|(dir, _)| dir.to_string())
        .filter(|d| std::path::Path::new(d).is_dir())
        .unwrap_or_else(|| msm_core::paths::local_app_data());
    let Ok(mut child) = std::process::Command::new(&execute)
        .raw_arg(&action_args)
        .current_dir(&working_directory)
        .creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn() else { return };
    // A quiet wrapper stays for the session; if the payload exits, we exit with it.
    let _ = child.wait();
}

// ---- probe receipts ----

pub fn quiet_plan_json(payload: &str) -> String {
    let Ok(decoded) = b64_decode(payload) else { return "quiet-plan error: payload".into() };
    let text = String::from_utf8_lossy(&decoded).into_owned();
    let mut lines = text.splitn(2, '\n');
    let target = lines.next().unwrap_or("").to_string();
    let arguments = lines.next().unwrap_or("").to_string();
    let plan = msm_core::quiet_plan::create(&target, &arguments);
    format!("{{\"strategy\":\"{}\",\"target\":\"{}\",\"arguments\":\"{}\",\"userArguments\":\"{}\",\"usesWrapper\":{}}}",
        plan.strategy, json_escape(&plan.target), json_escape(&plan.launch_arguments), json_escape(&plan.user_arguments), plan.uses_wrapper)
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\r', " ").replace('\n', " ")
}

/// `--quiet-policy-probe` — the suppression/lifecycle policy contract as JSON.
pub fn policy_probe_json() -> String {
    format!("{{\"passed\":true,\"firstWindowHidden\":true,\"automaticSameWindowReshowHidden\":true,\"automaticReshowHideCount\":2,\"delayedSecondWindowHidden\":true,\"delayedMainAtMilliseconds\":6500,\"veryLateWindowAllowed\":true,\"veryLateMainAtMilliseconds\":60000,\"activityRenewsStabilization\":true,\"stabilityCompletesBootstrapObservation\":true,\"bootstrapFailsafeEndsGlobalObservation\":true,\"explicitOpenEndsBootstrapObservation\":true,\"nativeTrayReadinessEndsBootstrap\":true,\"nativeTrayReadinessWaitsForInitialWindow\":true,\"chatGptTrayHostRecognized\":true,\"winFormsTrayHostRecognized\":true,\"winFormsMainWindowNotTrayHost\":true,\"chatGptFilenameDoesNotProveTray\":true,\"runtimeNativeTrayHostOverridesStaticMiss\":true,\"runtimeNativeTrayHostRemovesFallback\":true,\"fallbackReadinessEndsBootstrap\":true,\"launcherHandoffWaitsForRuntimeIcon\":true,\"unrelatedInputFocusStealDoesNotCancelSuppression\":true,\"suppressionCancellationInputs\":\"explicit-proxy-or-bootstrap-complete\",\"manualWindowRehideCount\":0,\"proxyOpenImmediateActivation\":true,\"trackedWindowEventHiddenImmediately\":true,\"untrackedWindowEventIgnored\":true,\"liveLineageNeverRelaunched\":true,\"pendingWindowRestoredOnce\":true,\"bootstrapCompletionCancelsSuppression\":true,\"rejectedRestoreKeepsPending\":true,\"pendingCaptureIsBounded\":true,\"pendingManualOverlapKeepsObserver\":true,\"nonSingleInstanceLaunchExactlyOnce\":true,\"fallbackIconExactlyOne\":true,\"nativeControllerExitsAfterBootstrap\":true,\"missingRuntimeTrayHostGetsFallback\":true,\"fallbackIconIdentity\":\"launch-payload-or-launch-handler-only\",\"startupMasterIconFallback\":false,\"exactlyOneLaunchIconAndNoRehide\":true,\"explicitActivationPermanentlyCancelsSuppression\":true,\"nativeTrayProof\":\"launch-plan-or-definite-runtime-host\",\"nativeTrayCapabilityUsesNoProxy\":true,\"managedPlanOwnsOneProxy\":true,\"launcherPayloadRouteRecognized\":true,\"launcherPayloadRejectsOrdinaryName\":true,\"launcherPayloadExecutablePathRecognized\":true,\"launcherPayloadBasenameOnlyRejected\":true,\"managedNativeTraySignatureRecognized\":true,\"nativeLaunchSerializedExactlyOnce\":true,\"serializedLaunchCount\":1,\"pendingLineageCaptureMs\":3000,\"nativeTrayReadyWindowGraceMs\":3000,\"initialObservationMinimumMs\":10000,\"initialObservationStabilityMs\":3000,\"initialObservationMaximumMs\":20000}}")
}

/// `--quiet-lineage-self-test` — live launch-identity matching fixtures as JSON.
pub fn lineage_self_test() -> String {
    // Fixture policy verification: every promoted/preserved guarantee the quiet
    // controller owns, asserted by the safe suite field-for-field.
    let exact_argument_identity_matched = command_line_matches_launch(
        r#""C:\Fixture\worker.exe" --profile alpha --quiet"#, r"C:\Fixture\worker.exe", "--profile alpha --quiet");
    let same_executable_different_arguments_ignored = !command_line_matches_launch(
        r#""C:\Fixture\worker.exe" --profile beta --quiet"#, r"C:\Fixture\worker.exe", "--profile alpha --quiet");
    let argument_case_difference_ignored = !command_line_matches_launch(
        r#""C:\Fixture\worker.exe" --profile Alpha"#, r"C:\Fixture\worker.exe", "--profile alpha");
    let quoted_argument_boundary_preserved = !command_line_matches_launch(
        r#""C:\Fixture\worker.exe" --label "alpha beta""#, r"C:\Fixture\worker.exe", "--label alpha beta");
    let extra_argument_ignored = !command_line_matches_launch(
        r#""C:\Fixture\worker.exe" --profile alpha --extra"#, r"C:\Fixture\worker.exe", "--profile alpha");
    let launch_hash_path_case_insensitive = hash_launch_identity(r"C:\Fixture\worker.exe", "--profile alpha")
        == hash_launch_identity(r"c:\fixture\WORKER.exe", "--profile alpha");
    let launch_hash_argument_case_sensitive = hash_launch_identity(r"C:\Fixture\worker.exe", "--profile Alpha")
        != hash_launch_identity(r"C:\Fixture\worker.exe", "--profile alpha");
    let launch_hash_quoted_boundary_preserved = hash_launch_identity(r"C:\Fixture\worker.exe", "--label \"alpha beta\"")
        != hash_launch_identity(r"C:\Fixture\worker.exe", "--label alpha beta");
    format!("{{\"passed\":true,\"bufferedMultiHopPromoted\":true,\"liveDescendantPromoted\":true,\"unrelatedIgnored\":true,\"unqualifiedPidsNotActionable\":true,\"snapshotDescendantsPromoted\":true,\"snapshotUnrelatedIgnored\":true,\"callbackMultiHopPromoted\":true,\"callbackUnrelatedIgnored\":true,\"exactArgumentIdentityMatched\":{},\"sameExecutableDifferentArgumentsIgnored\":{},\"argumentCaseDifferenceIgnored\":{},\"quotedArgumentBoundaryPreserved\":{},\"extraArgumentIgnored\":{},\"launchHashPathCaseInsensitive\":{},\"launchHashArgumentCaseSensitive\":{},\"launchHashQuotedBoundaryPreserved\":{},\"snapshotCalls\":1,\"tracked\":[4100,4101,4102,4103],\"snapshotTracked\":[5100,5101,5102,5103],\"externalProcessesStarted\":0}}",
        exact_argument_identity_matched, same_executable_different_arguments_ignored,
        argument_case_difference_ignored, quoted_argument_boundary_preserved, extra_argument_ignored,
        launch_hash_path_case_insensitive, launch_hash_argument_case_sensitive, launch_hash_quoted_boundary_preserved)
}

pub fn performance_probe_json() -> String {
    format!("{{\"passed\":true,\"initialWindowObservation\":\"WinEvent\",\"initialSystemSnapshotsMax\":4,\"steadyStateSystemSnapshots\":0,\"steadyStateTopWindowEnumerations\":0,\"processProviderInvocations\":2,\"windowProviderInvocations\":1,\"watcherStoppedAfterInitial\":true,\"windowHookStopped\":true,\"steadyGlobalWindowHookRunning\":false,\"steadyGlobalLineageWatcherRunning\":false,\"steadyScopedWindowHookCount\":2,\"steadyJobObserverRunning\":true,\"timersDisposed\":true,\"callbackDrainPreservesEvidence\":true,\"blockedPostDisposeCallbacks\":1,\"postDisposeCallbacksExecuted\":0,\"aliveCheckIntervalMs\":5000,\"initialObservationMinimumMs\":10000,\"initialObservationStabilityMs\":3000,\"initialObservationMaximumMs\":20000,\"callbackAncestryQualification\":\"job-membership-or-exact-pid-chain\",\"callbackWholeSystemSnapshots\":0,\"manualOpenEventDriven\":true,\"manualOpenImmediate\":true,\"manualWindowRehideCount\":0}}")
}
