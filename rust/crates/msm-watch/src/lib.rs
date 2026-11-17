//! `StartupChangeMonitor` — near-real-time refresh: registry notifications,
//! Startup-folder and Task-catalog watchers; 1s coalesce, 5s throttle.

use msm_core::reg::{open_subkey, RegRoot, RegView};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Registry::{RegNotifyChangeKeyValue, REG_NOTIFY_FILTER};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

pub const COALESCE_MILLISECONDS: u64 = 1000;
pub const MINIMUM_FIRE_MILLISECONDS: u64 = 5000;

const REG_NOTIFY_LAST_SET: u32 = 0x0000_0004;
const REG_NOTIFY_NAME: u32 = 0x0000_0008;

pub struct StartupChangeMonitor {
    changed: Arc<dyn Fn() + Send + Sync + 'static>,
    poke_state: Arc<PokeState>,
    stopped: Arc<AtomicBool>,
    threads: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
}

struct PokeState {
    queued: std::sync::Mutex<Option<std::time::Instant>>,
}

impl StartupChangeMonitor {
    pub fn new(changed: impl Fn() + Send + Sync + 'static) -> StartupChangeMonitor {
        StartupChangeMonitor {
            changed: Arc::new(changed),
            poke_state: Arc::new(PokeState { queued: std::sync::Mutex::new(None) }),
            stopped: Arc::new(AtomicBool::new(false)),
            threads: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Arm every watched surface (C# `Start`): both views on HKLM, native view on HKCU.
    pub fn start(&self) {
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let mut machine_keys: Vec<String> = Vec::new();
        for key in RUN_KEY_FAMILY {
            machine_keys.push(key.to_string());
        }
        machine_keys.push(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon".into());
        for kind in STARTUP_APPROVED_KINDS {
            machine_keys.push(format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\{}", kind));
        }
        let mut user_keys: Vec<String> = machine_keys.clone();
        user_keys.push("Environment".into());
        user_keys.push(r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData".into());
        user_keys.push(r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages".into());
        for key in &machine_keys {
            self.watch_registry(RegRoot::LocalMachine, key);
        }
        for key in &user_keys {
            self.watch_registry(RegRoot::CurrentUser, key);
        }
        self.watch_folder(&msm_core::scan_folders::user_startup_folder(), false);
        self.watch_folder(&msm_core::scan_folders::common_startup_folder(), false);
        let task_catalog = format!("{}\\Tasks", msm_core::paths::system_dir());
        self.watch_folder(&task_catalog, true);
        self.spawn_coalesce_timer();
    }

    fn spawn_coalesce_timer(&self) {
        let poke_state = self.poke_state.clone();
        let changed = self.changed.clone();
        let stopped = self.stopped.clone();
        if let Ok(handle) = std::thread::Builder::new().name("change-coalesce".into()).spawn(move || {
            let mut last_fire: Option<std::time::Instant> = None;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                let queued = *poke_state.queued.lock().unwrap();
                if let Some(queued_at) = queued {
                    if queued_at.elapsed().as_millis() as u64 >= COALESCE_MILLISECONDS {
                        if let Some(last) = last_fire {
                            if (last.elapsed().as_millis() as u64) < MINIMUM_FIRE_MILLISECONDS {
                                continue;
                            }
                        }
                        *poke_state.queued.lock().unwrap() = None;
                        last_fire = Some(std::time::Instant::now());
                        changed();
                    }
                }
            }
        }) {
            self.threads.lock().unwrap().push(handle);
        }
    }

    fn watch_registry(&self, root: RegRoot, sub_key: &str) {
        // Key handles stay open for the monitor lifetime (C# parity).
        let Some(handle) = open_subkey(root, sub_key, RegView::Default) else { return };
        let raw = handle.0;
        std::mem::forget(handle);
        let Some(event) = (unsafe { CreateEventW(None, false, false, PCWSTR::null()).ok() }) else { return };
        let poke_state = self.poke_state.clone();
        let stopped = self.stopped.clone();
        let raw_key = raw.0 as usize; // HKEY is a pointer; carry it as usize across threads
        let event_addr = event.0 as usize;
        let arm = move |raw_key: usize, event_addr: usize| unsafe {
            RegNotifyChangeKeyValue(windows::Win32::System::Registry::HKEY(raw_key as *mut _), true,
                REG_NOTIFY_FILTER(REG_NOTIFY_LAST_SET | REG_NOTIFY_NAME),
                Some(windows::Win32::Foundation::HANDLE(event_addr as *mut _)), true)
        };
        unsafe { arm(raw_key, event_addr) };
        if let Ok(thread) = std::thread::Builder::new().name("registry-watch".into()).spawn(move || {
            let event = windows::Win32::Foundation::HANDLE(event_addr as *mut _);
            let key = windows::Win32::System::Registry::HKEY(raw_key as *mut _);
            loop {
                if stopped.load(Ordering::SeqCst) { break }
                let result = unsafe { WaitForSingleObject(event, 500) };
                if result == WAIT_OBJECT_0 {
                    *poke_state.queued.lock().unwrap() = Some(std::time::Instant::now());
                    unsafe { arm(raw_key, event_addr) };
                }
            }
            unsafe {
                let _ = CloseHandle(event);
                let _ = windows::Win32::System::Registry::RegCloseKey(key);
            }
        }) {
            self.threads.lock().unwrap().push(thread);
        }
    }

    fn watch_folder(&self, path: &str, recursive: bool) {
        if path.trim().is_empty() || !std::path::Path::new(path).is_dir() {
            return;
        }
        let poke_state = self.poke_state.clone();
        let stopped = self.stopped.clone();
        let path_owned = path.to_string();
        if let Ok(handle) = std::thread::Builder::new().name("folder-watch".into()).spawn(move || {
            watch_directory(&path_owned, recursive, &stopped, &poke_state);
        }) {
            self.threads.lock().unwrap().push(handle);
        }
    }

    /// Any watched surface changed (C# `Poke`).
    pub fn poke(&self) {
        *self.poke_state.queued.lock().unwrap() = Some(std::time::Instant::now());
    }
}

impl Drop for StartupChangeMonitor {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        for thread in self.threads.lock().unwrap().drain(..) {
            let _ = thread.join();
        }
    }
}

const RUN_KEY_FAMILY: [&str; 6] = [
    r"Software\Microsoft\Windows\CurrentVersion\Run",
    r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
    r"Software\Microsoft\Windows\CurrentVersion\RunServices",
    r"Software\Microsoft\Windows\CurrentVersion\RunServicesOnce",
    r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run",
    r"Software\Microsoft\Windows\CurrentVersion\RunOnceEx",
];

const STARTUP_APPROVED_KINDS: [&str; 5] = ["Run", "Run32", "StartupFolder", "StartupFolder32", "StartupTasks"];

type PokePair = Arc<PokeState>;

/// `ReadDirectoryChangesW` loop for one watched folder (64 KiB buffer, C# parity).
fn watch_directory(path: &str, recursive: bool, stopped: &AtomicBool, poke_state: &PokePair) {
    use windows::Win32::Storage::FileSystem::{
        ReadDirectoryChangesW, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_DIR_NAME,
        FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_LIST_DIRECTORY,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE, OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
    };
    use windows::Win32::System::IO::OVERLAPPED;
    let wide_path = msm_core::paths::to_wide(path);
    let directory = unsafe {
        windows::Win32::Storage::FileSystem::CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )
    };
    let Ok(directory) = directory else { return };
    let Some(event) = (unsafe { CreateEventW(None, false, false, PCWSTR::null()).ok() }) else {
        unsafe { let _ = CloseHandle(directory); }
        return;
    };
    let mut buffer = vec![0u8; 64 * 1024];
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.hEvent = event;
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE;
    loop {
        if stopped.load(Ordering::SeqCst) { break }
        let pending = unsafe {
            ReadDirectoryChangesW(directory, buffer.as_mut_ptr() as *mut core::ffi::c_void,
                buffer.len() as u32, recursive, filter, None,
                Some(&mut overlapped), None)
        };
        if pending.is_err() {
            break;
        }
        loop {
            if stopped.load(Ordering::SeqCst) {
                unsafe { let _ = CloseHandle(directory); let _ = CloseHandle(event); }
                return;
            }
            let result = unsafe { WaitForSingleObject(event, 500) };
            if result == WAIT_OBJECT_0 {
                *poke_state.queued.lock().unwrap() = Some(std::time::Instant::now());
                break;
            }
        }
    }
    unsafe { let _ = CloseHandle(directory); let _ = CloseHandle(event); }
}
