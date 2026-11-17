//! Managed startup task primitives: direct action building, logon-task
//! registration (PowerShell Register-ScheduledTask), schtasks runners.

use crate::util::{b64, q, win_arg};

pub fn power_shell_exe() -> String {
    let full = format!("{}\\WindowsPowerShell\\v1.0\\powershell.exe", crate::paths::windows_dir().trim_end_matches('\\'));
    if std::path::Path::new(&full).is_file() { full } else { "powershell.exe".into() }
}

/// C# `RunCapture` — capture stdout with a hard timeout.
pub fn run_capture(exe: &str, args: &str, timeout_ms: u32) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let child = std::process::Command::new(exe)
        .raw_arg(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW — zero popups, ever
        .spawn()
        .map_err(|e| e.to_string())?;
    let output = wait_with_timeout(child, timeout_ms)?;
    let text = format!("{} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)).trim().to_string();
    if !output.status.success() {
        return Err(text);
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// C# `RunChecked` — run, drain pipes, kill on timeout, fail on nonzero exit.
pub fn run_checked(exe: &str, args: &str, timeout_ms: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let child = std::process::Command::new(exe)
        .raw_arg(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW — zero popups, ever
        .spawn()
        .map_err(|e| e.to_string())?;
    let output = wait_with_timeout(child, timeout_ms)?;
    if !output.status.success() {
        let text = format!("{} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)).trim().to_string();
        return Err(text);
    }
    Ok(())
}

struct KilledOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn wait_with_timeout(mut child: std::process::Child, timeout_ms: u32) -> Result<KilledOutput, String> {
    use std::io::Read;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut stdout);
        }
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut stderr);
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(KilledOutput { status, stdout, stderr }),
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Command timed out after {} ms: {}", timeout_ms, child.id()));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// C# `IsSupportedStartupTarget`.
pub fn is_supported_startup_target(path: &str) -> bool {
    !path.trim().is_empty() && !path.contains(['\0', '\r', '\n'])
}

/// C# `BuildDirectAction` — script hosts and direct executables.
pub fn build_direct_action(target_path: &str, arguments: &str) -> (String, String) {
    let extension = std::path::Path::new(target_path).extension()
        .map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let append = |args: &str| if args.trim().is_empty() { String::new() } else { format!(" {}", args.trim()) };
    match extension.as_str() {
        "ps1" => {
            let execute = format!("{}\\WindowsPowerShell\\v1.0\\powershell.exe", crate::paths::windows_dir().trim_end_matches('\\'));
            (execute, format!("-NoProfile -ExecutionPolicy Bypass -File {}{}", win_arg(target_path), append(arguments)))
        }
        "cmd" | "bat" => {
            let execute = format!("{}\\System32\\cmd.exe", crate::paths::windows_dir().trim_end_matches('\\'));
            (execute, format!("/d /c {}{}", win_arg(target_path), append(arguments)))
        }
        "vbs" | "vbe" | "js" | "jse" | "wsf" | "wsh" => {
            let execute = format!("{}\\System32\\wscript.exe", crate::paths::windows_dir().trim_end_matches('\\'));
            (execute, format!("//B //NoLogo {}{}", win_arg(target_path), append(arguments)))
        }
        "py" | "pyw" => {
            let windowless = extension == "pyw";
            let execute = resolve_python_host(windowless);
            let launcher = {
                let leaf = execute.rsplit('\\').next().unwrap_or("").to_lowercase();
                leaf == "py.exe" || leaf == "pyw.exe"
            };
            (execute, format!("{}{}{}", if launcher { "-3 " } else { "" }, win_arg(target_path), append(arguments)))
        }
        "exe" | "com" => (target_path.to_string(), arguments.to_string()),
        _ => {
            // Documents launch through the app's own shell-open shim.
            let execute = crate::util::process_exe_path();
            (execute, format!("--shell-open {}", b64(&format!("{}\n{}", target_path, arguments))))
        }
    }
}

fn resolve_python_host(windowless: bool) -> String {
    let names: &[&str] = if windowless { &["pythonw.exe", "pyw.exe", "python.exe", "py.exe"] } else { &["python.exe", "py.exe"] };
    for name in names {
        if let Some(exact) = resolve_executable_on_path(name) {
            return exact;
        }
    }
    let local_programs = format!("{}\\Programs\\Python", crate::paths::local_app_data().trim_end_matches('\\'));
    if let Ok(dir) = std::fs::read_dir(&local_programs) {
        let mut directories: Vec<_> = dir.filter_map(|e| e.ok())
            .map(|e| e.path().to_string_lossy().into_owned())
            .filter(|p| p.to_lowercase().contains("python"))
            .collect();
        directories.sort_by(|a, b| b.to_lowercase().cmp(&a.to_lowercase()));
        for directory in directories {
            for name in names {
                let candidate = format!("{}\\{}", directory, name);
                if std::path::Path::new(&candidate).is_file() {
                    return candidate;
                }
            }
        }
    }
    let launcher = format!("{}\\{}", crate::paths::windows_dir().trim_end_matches('\\'), if windowless { "pyw.exe" } else { "py.exe" });
    if std::path::Path::new(&launcher).is_file() {
        return launcher;
    }
    String::new()
}

fn resolve_executable_on_path(file_name: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for directory in path.split(';').filter(|d| !d.trim().is_empty()) {
        let directory = crate::util::expand_env(directory.trim().trim_matches('"'));
        let candidate = format!("{}\\{}", directory.trim_end_matches('\\'), file_name);
        if std::path::Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }
    None
}

/// C# `BuildManagedAction` — quiet plans wrap unless the target is natively quiet.
pub fn build_managed_action(target_path: &str, arguments: &str, tray_mode: bool) -> (String, String) {
    if tray_mode {
        let plan = crate::quiet_plan::create(target_path, arguments);
        if !plan.uses_wrapper {
            if plan.strategy == "self-tray" {
                return (crate::util::process_exe_path(), "--start-in-tray".into());
            }
            return build_direct_action(&plan.target, &plan.launch_arguments);
        }
        let payload = b64(&format!("{}\n{}", plan.target, plan.user_arguments));
        return (crate::util::process_exe_path(), format!("--tray-run {}", payload));
    }
    build_direct_action(target_path, arguments)
}

fn run_power_shell_script(script: &str) -> Result<(), String> {
    let temp = format!("{}\\task-register-{}.ps1", crate::paths::state_root(), b64(&format!("{}", crate::util::tick_ms())));
    let _ = std::fs::create_dir_all(crate::paths::state_root());
    let _ = std::fs::write(&temp, script);
    let result = run_checked(&power_shell_exe(), &format!("-NoProfile -ExecutionPolicy Bypass -File {}", q(&temp)), 30_000);
    let _ = std::fs::remove_file(&temp);
    result
}

/// C# `RegisterLogonTaskAt` — immediate per-user logon task via Schedule.Service.
pub fn register_logon_task_at(full_task_name: &str, execute: &str, arguments: &str, replace_existing: bool) -> Result<String, String> {
    let managed_root = crate::scan::MANAGED_TASK_ROOT;
    let normalized = if full_task_name.starts_with('\\') {
        full_task_name.to_string()
    } else {
        format!("{}{}", managed_root, full_task_name.trim_matches('\\'))
    };
    let split = normalized.rfind('\\');
    let (path, task_name) = match split {
        Some(index) => (normalized[..index + 1].to_string(), normalized[index + 1..].to_string()),
        None => (managed_root.to_string(), normalized.trim_matches('\\').to_string()),
    };
    let path = if path.trim().is_empty() { managed_root.to_string() } else { path };
    if task_name.trim().is_empty() {
        return Err("Task name is required".into());
    }
    if execute.trim().is_empty() || !std::path::Path::new(execute).is_file() {
        return Err(format!("Startup executable was not found: {}", execute));
    }
    let script = format!(
        "$ErrorActionPreference='Stop';\
function D($s){{[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s))}};\
$path = D '{path_b64}';\
$taskName = D '{task_name_b64}';\
$execute = D '{execute_b64}';\
$arguments = D '{arguments_b64}';\
if(-not (Test-Path -LiteralPath $execute)){{throw ('Startup executable was not found: ' + $execute)}};\
$folderPath=$path.TrimEnd('\\');\
$schedule=New-Object -ComObject 'Schedule.Service';$schedule.Connect();$folder=$schedule.GetFolder('\\');$currentPath='';\
if($folderPath -ne ''){{foreach($segment in ($folderPath.Trim('\\') -split '\\\\')){{if([string]::IsNullOrWhiteSpace($segment)){{continue}};$currentPath += '\\' + $segment;try{{$folder=$schedule.GetFolder($currentPath)}}catch{{$null=$folder.CreateFolder($segment);$folder=$schedule.GetFolder($currentPath)}}}}}};\
$identity=[Security.Principal.WindowsIdentity]::GetCurrent();if([string]::IsNullOrWhiteSpace($identity.Name)){{throw 'The current Windows user is unavailable'}};$currentUser=$identity.Name;\
$action=if([string]::IsNullOrWhiteSpace($arguments)){{New-ScheduledTaskAction -Execute $execute}}else{{New-ScheduledTaskAction -Execute $execute -Argument $arguments}};\
$trigger=New-ScheduledTaskTrigger -AtLogOn -User $currentUser;\
$principal=New-ScheduledTaskPrincipal -UserId $currentUser -LogonType Interactive -RunLevel Limited;\
$settings=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 0) -Hidden;\
Register-ScheduledTask -TaskPath $path -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings{force} | Out-Null;",
        path_b64 = b64(&path),
        task_name_b64 = b64(&task_name),
        execute_b64 = b64(execute),
        arguments_b64 = b64(arguments),
        force = if replace_existing { " -Force" } else { "" }
    );
    run_power_shell_script(&script)?;
    Ok(format!("{}{}", path, task_name))
}

/// C# `RestoreTaskXml` — Register-ScheduledTask from archived XML.
pub fn restore_task_xml(full_task_name: &str, xml: &str) -> Result<(), String> {
    let (folder_path, task_name) = split_task_location(full_task_name)?;
    let task_path = if folder_path == "\\" { "\\".to_string() } else { format!("{}\\", folder_path.trim_end_matches('\\')) };
    let script = format!(
        "$ErrorActionPreference='Stop';\
function D($s){{[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s))}};\
$path=D '{}';\
$name=D '{}';\
$xml=D '{}';\
Register-ScheduledTask -TaskPath $path -TaskName $name -Xml $xml -Force | Out-Null;",
        b64(&task_path), b64(&task_name), b64(xml)
    );
    run_power_shell_script(&script)
}

pub fn split_task_location(full_task_name: &str) -> Result<(String, String), String> {
    crate::tasks_com::split_task_location_pub(full_task_name)
}
