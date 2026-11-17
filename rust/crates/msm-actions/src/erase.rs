//! Erase — after a purge, delete everything the application left in Windows:
//! its processes, services, install folders, per-user data folders and its
//! registry keys (software + Uninstall registrations). Permanent. Guarded so
//! system locations can never be touched.

use msm_core::model::StartupItem;
use msm_core::reg::{native_registry_view, open_subkey, RegRoot};
use std::path::{Path, PathBuf};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn norm(s: &str) -> String {
    s.replace('/', "\\").to_lowercase().trim_end_matches('\\').to_string()
}

/// True when `p` is a safe application folder to delete.
pub fn is_safe_root(p: &Path) -> bool {
    let s = norm(&p.to_string_lossy());
    let protected: Vec<String> = ["SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)", "ProgramData", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "SystemDrive"]
        .iter().map(|n| norm(&env(n))).filter(|x| !x.is_empty()).collect();
    if s.len() < 8 || protected.iter().any(|x| *x == s) {
        return false;
    }
    let sys = norm(&env("SystemRoot"));
    if !sys.is_empty() && (s == sys || s.starts_with(&format!("{}\\", sys))) {
        return false;
    }
    if s.contains("\\windowsapps") || s.contains("\\microsoft\\") || s.ends_with("\\microsoft") {
        return false;
    }
    let under = |base: String| {
        let b = norm(&base);
        !b.is_empty() && s.starts_with(&format!("{}\\", b))
    };
    if !(under(env("ProgramFiles")) || under(env("ProgramFiles(x86)")) || under(env("ProgramData"))
        || under(env("LOCALAPPDATA")) || under(env("APPDATA")) || under(env("USERPROFILE")))
    {
        return false;
    }
    !s.ends_with("\\common files") && !s.ends_with("\\windows nt")
}

fn rm(cmd: &str, args: &str, log: &mut Vec<String>, what: &str) {
    match msm_core::managed::run_checked(cmd, args, 30_000) {
        Ok(()) => log.push(format!("removed {}", what)),
        Err(e) => {
            let l = e.to_lowercase();
            if !l.contains("unable to find") && !l.contains("cannot find") && !l.contains("1060") && !l.contains("does not exist") {
                log.push(format!("FAILED {}: {}", what, e.lines().next().unwrap_or("")));
            }
        }
    }
}

fn reg_delete(key: &str, log: &mut Vec<String>) {
    rm("reg.exe", &format!("delete \"{}\" /f", key), log, &format!("registry {}", key));
}

fn key_exists(root: RegRoot, sub: &str) -> bool {
    open_subkey(root, sub, native_registry_view()).is_some()
}

/// Delete every leftover of the apps behind `routes`. Returns human-readable lines;
/// lines starting with FAILED are real failures.
pub fn erase_traces(routes: &[StartupItem]) -> Vec<String> {
    let mut log: Vec<String> = Vec::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for r in routes {
        let mut cand = r.application_root.trim().to_string();
        if cand.is_empty() {
            if let Some((exe, _)) = msm_core::humans::try_split_command(&r.command) {
                if let Some(parent) = Path::new(exe.trim_matches('"')).parent() {
                    cand = parent.to_string_lossy().into_owned();
                }
            }
        }
        if cand.is_empty() {
            continue;
        }
        let p = PathBuf::from(cand);
        if is_safe_root(&p) {
            if !roots.iter().any(|x| x == &p) {
                roots.push(p);
            }
        } else {
            log.push(format!("kept (protected location) {}", p.display()));
        }
    }
    // Services first, so files are not locked.
    for r in routes {
        let id = r.id.to_lowercase();
        if id.contains("service|") || id.contains("driver|") {
            let name = super::service_name_from_item(r);
            if !name.is_empty() {
                let _ = msm_core::managed::run_checked("sc.exe", &format!("stop \"{}\"", name), 30_000);
                rm("sc.exe", &format!("delete \"{}\"", name), &mut log, &format!("service {}", name));
            }
        }
    }
    // Processes running from the install roots.
    for root in &roots {
        let pat = root.to_string_lossy().replace('\'', "''");
        let ps = format!(
            "-NoProfile -WindowStyle Hidden -Command \"Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith('{}', 'OrdinalIgnoreCase') }} | Stop-Process -Force -ErrorAction SilentlyContinue\"",
            pat
        );
        let _ = msm_core::managed::run_checked("powershell.exe", &ps, 60_000);
    }
    // Folders: install roots, then per-user/shared data folders named like the app.
    let mut dirs: Vec<PathBuf> = roots.clone();
    let mut tokens: Vec<String> = roots
        .iter()
        .filter_map(|r| r.file_name().map(|x| x.to_string_lossy().into_owned()))
        .filter(|t| t.len() >= 4)
        .collect();
    tokens.sort();
    tokens.dedup();
    for t in &tokens {
        for base in ["APPDATA", "LOCALAPPDATA", "ProgramData"] {
            let b = env(base);
            if b.is_empty() {
                continue;
            }
            let d = PathBuf::from(b).join(t);
            if d.is_dir() && is_safe_root(&d) {
                dirs.push(d);
            }
        }
    }
    for d in &dirs {
        if !d.exists() {
            continue;
        }
        match std::fs::remove_dir_all(d) {
            Ok(()) => log.push(format!("deleted folder {}", d.display())),
            Err(e) => {
                let _ = msm_core::managed::run_checked("cmd.exe", &format!("/c rd /s /q \"{}\"", d.display()), 60_000);
                if d.exists() {
                    log.push(format!("FAILED folder {}: {}", d.display(), e));
                } else {
                    log.push(format!("deleted folder {}", d.display()));
                }
            }
        }
    }
    // Empty vendor folders left behind.
    for root in &roots {
        if let Some(parent) = root.parent() {
            if is_safe_root(parent) {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
    // Registry: software keys named like the app.
    for t in &tokens {
        for (hive, rr) in [("HKCU", RegRoot::CurrentUser), ("HKLM", RegRoot::LocalMachine)] {
            for sub in [format!("Software\\{}", t), format!("Software\\WOW6432Node\\{}", t)] {
                if key_exists(rr, &sub) {
                    reg_delete(&format!("{}\\{}", hive, sub), &mut log);
                }
            }
        }
    }
    // Uninstall registrations that point into a deleted root.
    let unins = [
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ];
    let lowered: Vec<String> = roots.iter().map(|r| norm(&r.to_string_lossy())).collect();
    for (hive, rr) in [("HKCU", RegRoot::CurrentUser), ("HKLM", RegRoot::LocalMachine)] {
        for base in unins {
            let Some(k) = open_subkey(rr, base, native_registry_view()) else { continue };
            for name in k.subkey_names() {
                let sub = format!("{}\\{}", base, name);
                let Some(sk) = open_subkey(rr, &sub, native_registry_view()) else { continue };
                let hit = ["InstallLocation", "UninstallString", "DisplayIcon"].iter().any(|v| {
                    sk.value(v)
                        .map(|(_, val)| {
                            let d = val.display().to_lowercase().replace('/', "\\");
                            lowered.iter().any(|r| d.contains(r.as_str()))
                        })
                        .unwrap_or(false)
                });
                if hit {
                    reg_delete(&format!("{}\\{}", hive, sub), &mut log);
                }
            }
        }
    }
    log
}
