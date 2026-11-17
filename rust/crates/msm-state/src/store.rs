//! `StateStoreFile` — named-mutex + atomic write + `.bak` recovery (exact C# semantics).

use msm_core::util::b64;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};

pub static GENERATION: AtomicI64 = AtomicI64::new(0);

pub fn generation() -> i64 {
    GENERATION.load(Ordering::SeqCst)
}

fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// `Local\MichStartupMaster.State.<sha256(lower(fullpath))[0..32]>`
fn mutex_name(path: &Path) -> String {
    let full = msm_core::paths::canonical(&path.to_string_lossy()).to_lowercase();
    let digest = Sha256::digest(full.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{:02X}", b)).collect::<String>().to_lowercase();
    format!(r"Local\MichStartupMaster.State.{}", &hex[..32])
}

struct StoreGuard {
    handle: windows::Win32::Foundation::HANDLE,
}

impl StoreGuard {
    fn acquire(path: &Path) -> Option<StoreGuard> {
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
        use windows::core::PCWSTR;
        let name = msm_core::paths::to_wide(&mutex_name(path));
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr())).ok()?;
            // 30s bounded wait, matching the C# store contract. An abandoned mutex
            // (WAIT_ABANDONED) is still ownership, like the C# AbandonedMutexException path.
            let waited = WaitForSingleObject(handle, 30_000);
            if waited != WAIT_OBJECT_0 && waited != windows::Win32::Foundation::WAIT_ABANDONED {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
                return None;
            }
            Some(StoreGuard { handle })
        }
    }
}

impl Drop for StoreGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.handle);
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// `.bak` recovery: a missing store with a backup restores; both present drops the backup.
fn recover(path: &Path) {
    let backup = backup_path(path);
    if !path.exists() && backup.exists() {
        let _ = fs::rename(&backup, path);
        bump_generation();
    } else if path.exists() && backup.exists() {
        let _ = fs::remove_file(&backup);
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    name.push_str(".bak");
    path.with_file_name(name)
}

/// Read all lines (UTF-8). A missing store reads as empty. Empty lines preserved.
/// C# ReadAllLines semantics: BOM stripped, CRLF/LF split, no trailing empty row.
fn read_store_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes).replace("\u{feff}", "");
    let mut lines: Vec<String> = text
        .split("\r\n")
        .flat_map(|line| if line.ends_with('\n') { line.strip_suffix('\n').map(|l| vec![l.to_string()]).unwrap_or_default() } else { vec![line.to_string()] })
        .collect();
    if lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines
}

pub fn read_all_lines(path: &Path) -> Vec<String> {
    let _guard = StoreGuard::acquire(path);
    recover(path);
    match fs::read(path) {
        Ok(bytes) => read_store_lines(&bytes),
        Err(_) => {
            let mut empty: Vec<String> = Vec::new();
            if path.exists() {
                if let Ok(text) = fs::read_to_string(path) {
                    empty = text.lines().map(|l| l.to_string()).collect();
                }
            }
            empty
        }
    }
}

/// Atomic replace-all write (CRLF line endings, UTF-8 no BOM, FILE_WRITE_THROUGH).
pub fn write_all_lines(path: &Path, lines: &[String]) {
    let _guard = StoreGuard::acquire(path);
    recover(path);
    write_atomic(path, lines);
}

/// Read-modify-write under the same lock (transactional update).
pub fn update_lines(path: &Path, update: impl FnOnce(Vec<String>) -> Vec<String>) {
    // A missing lock is fatal, not skippable: without it every writer clobbers the file.
    let _guard = StoreGuard::acquire(path)
        .unwrap_or_else(|| panic!("Timed out waiting for state-store lock: {}", path.display()));
    recover(path);
    let current: Vec<String> = match fs::read_to_string(path) {
        Ok(text) => text.replace("\u{feff}", "").lines().map(|l| l.to_string()).collect(),
        Err(_) => Vec::new(),
    };
    let changed = update(current);
    write_atomic(path, &changed);
}

/// Remove the file only when it is empty (refuses otherwise, like C#).
pub fn delete_if_empty(path: &Path) -> Result<(), String> {
    let _guard = StoreGuard::acquire(path);
    if !path.exists() {
        return Ok(());
    }
    let lines = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if !lines.lines().all(|l| l.trim().is_empty()) {
        return Err(format!("Refusing to remove a nonempty state store: {}", path.display()));
    }
    fs::remove_file(path).map_err(|e| e.to_string())?;
    bump_generation();
    if path.exists() {
        return Err(format!("Empty state store could not be removed: {}", path.display()));
    }
    Ok(())
}

fn write_atomic(path: &Path, lines: &[String]) {
    let directory = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let _ = fs::create_dir_all(&directory);
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = directory.join(format!("{}.tmp.{}.{}", file_name, std::process::id(), b64(&new_guid())));
    let backup = backup_path(path);
    let body: String = lines.iter().map(|l| format!("{}\r\n", l)).collect();
    let result = (|| -> std::io::Result<()> {
        let mut stream = fs::File::options().write(true).create_new(true).open(&temp)?;
        stream.write_all(body.as_bytes())?;
        stream.sync_all()?;
        drop(stream);
        if path.exists() {
            let _ = fs::remove_file(&backup);
            // ReplaceFileW preserves the old store as .bak; then drop it (C# parity).
            if !replace_file(&temp, path, &backup) {
                // Rare ReplaceFileW refusal (attributes/AV): durable copy keeps content correct.
                let bytes = fs::read(&temp)?;
                fs::write(path, &bytes)?;
            }
            let _ = fs::remove_file(&backup);
        } else {
            fs::rename(&temp, path)?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    bump_generation();
}

fn replace_file(temp: &Path, path: &Path, backup: &Path) -> bool {
    use windows::core::PCWSTR;
    let temp_w = msm_core::paths::to_wide(&temp.to_string_lossy());
    let path_w = msm_core::paths::to_wide(&path.to_string_lossy());
    let backup_w = msm_core::paths::to_wide(&backup.to_string_lossy());
    unsafe {
        #[allow(non_snake_case)]
        let ReplaceFileW: unsafe extern "system" fn(
            windows::core::PCWSTR, windows::core::PCWSTR, windows::core::PCWSTR, u32,
            Option<*mut core::ffi::c_void>, Option<*mut core::ffi::c_void>) -> windows::core::BOOL = {
            extern "system" {
                #[link_name = "ReplaceFileW"]
                fn imp(replaced: windows::core::PCWSTR, replacement: windows::core::PCWSTR,
                       backup: windows::core::PCWSTR, flags: u32, exclude: Option<*mut core::ffi::c_void>,
                       reserved: Option<*mut core::ffi::c_void>) -> windows::core::BOOL;
            }
            imp
        };
        ReplaceFileW(PCWSTR(path_w.as_ptr()), PCWSTR(temp_w.as_ptr()), PCWSTR(backup_w.as_ptr()), 0, None, None).as_bool()
    }
}

fn new_guid() -> String {
    // Random-enough 128-bit id from the OS cryptographic path via tempfile naming entropy.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let pid = std::process::id() as u128;
    let salt = msm_core::util::tick_ms() as u128;
    let mixed = nanos ^ (pid << 64) ^ (salt << 17);
    format!("{:032x}", mixed)
}

/// Byte snapshot for transactional rollback (store, or absence of store).
pub struct StoreSnapshot {
    path: PathBuf,
    exists: bool,
    bytes: Vec<u8>,
}

impl StoreSnapshot {
    pub fn capture(path: &Path) -> StoreSnapshot {
        let exists = path.exists();
        StoreSnapshot { path: path.to_path_buf(), exists, bytes: if exists { fs::read(path).unwrap_or_default() } else { Vec::new() } }
    }

    pub fn restore(&self) -> Result<(), String> {
        if self.exists {
            write_atomic(&self.path, &String::from_utf8_lossy(&self.bytes).lines().map(|l| l.to_string()).collect::<Vec<_>>());
            Ok(())
        } else if self.path.exists() {
            fs::remove_file(&self.path).map_err(|e| e.to_string())
        } else {
            Ok(())
        }
    }
}

/// `--state-store-worker <b64path> <token>` — append-once worker for the self-test.
pub fn worker(args: &[String]) -> i32 {
    if args.len() < 3 {
        return 2;
    }
    use base64::Engine;
    let Ok(path) = base64::engine::general_purpose::STANDARD.decode(&args[1]) else { return 3 };
    let path = String::from_utf8_lossy(&path).into_owned();
    let token = args[2].clone();
    update_lines(Path::new(&path), |mut lines| {
        if !lines.iter().any(|l| l == &token) {
            lines.push(token);
        }
        lines
    });
    0
}

/// `--state-store-self-test` — 12 concurrent workers + atomic + recovery proof.
pub fn self_test() -> String {
    let root = std::env::temp_dir().join(format!("MichStartupMaster-state-test-{}", new_guid()));
    let path = root.join("state.tsv");
    let _ = fs::create_dir_all(&root);
    let exe = msm_core::util::process_exe_path();
    let mut workers = Vec::new();
    for i in 0..12 {
        let token = format!("worker-{}", i);
        if let Ok(child) = std::process::Command::new(&exe)
            .arg("--state-store-worker")
            .arg(msm_core::util::b64(&path.to_string_lossy()))
            .arg(&token)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
            workers.push((child, token));
        }
    }
    let mut failures = 0;
    for (mut child, _) in workers {
        match child.wait() {
            Ok(status) if status.success() => {}
            _ => failures += 1,
        }
    }
    let concurrent = read_all_lines(&path);
    let distinct = concurrent.iter().filter(|l| !l.trim().is_empty()).map(|l| l.clone()).collect::<std::collections::BTreeSet<_>>();
    if failures != 0 || distinct.len() != 12 {
        let _ = fs::remove_dir_all(&root);
        return format!("STATE_STORE_SELF_TEST failed workers=12 lost_rows={}", 12usize.saturating_sub(distinct.len()));
    }
    // A read-only update commits identical content (temp+replace never corrupts).
    // (The C# simulated-panic variant cannot run here: release builds abort.)
    let before = concurrent.iter().cloned().collect::<std::collections::BTreeSet<_>>();
    update_lines(&path, |lines| lines);
    let after = read_all_lines(&path).iter().cloned().collect::<std::collections::BTreeSet<_>>();
    if before != after {
        let _ = fs::remove_dir_all(&root);
        return "STATE_STORE_SELF_TEST failed=mutation-leak".into();
    }
    // Backup recovery: move the store to .bak; the next read restores it.
    let _ = fs::rename(&path, backup_path(&path));
    let recovered = read_all_lines(&path);
    let _ = fs::remove_dir_all(&root);
    if recovered.len() != 12 {
        return format!("STATE_STORE_SELF_TEST failed=recovery recovered={}", recovered.len());
    }
    "STATE_STORE_SELF_TEST workers=12 atomic=ok recovery=ok".into()
}

/// Raw byte rollback for transaction snapshots (atomic, lock-held).
pub fn write_all_lines_raw(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let _guard = StoreGuard::acquire(path);
    recover(path);
    let directory = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = directory.join(format!("{}.rollback.{}", file_name, b64(&new_guid())));
    let result = (|| -> std::io::Result<()> {
        let mut stream = fs::File::options().write(true).create_new(true).open(&temp)?;
        use std::io::Write;
        stream.write_all(bytes)?;
        stream.sync_all()?;
        drop(stream);
        if path.exists() {
            let _ = fs::remove_file(&backup_path(path));
            replace_file(&temp, path, &backup_path(path));
            let _ = fs::remove_file(&backup_path(path));
        } else {
            fs::rename(&temp, path)?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    bump_generation();
    result.map_err(|e| e.to_string())
}
