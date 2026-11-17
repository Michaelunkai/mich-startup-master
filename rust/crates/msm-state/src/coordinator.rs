//! `StartupMutationCoordinator` — the two-level cross-process mutation gate.

use std::sync::atomic::{AtomicU32, Ordering};
use windows::core::PCWSTR;

const DEPLOYMENT_MUTEX: &str = r"Local\MichStartupMaster.ReleaseDeployment";
const MUTATION_MUTEX: &str = r"Local\MichStartupMaster.ManagedStartupMutation";

thread_local! {
    static DEPTH: AtomicU32 = const { AtomicU32::new(0) };
}

struct Handle(windows::Win32::Foundation::HANDLE);

impl Handle {
    fn open(name: &str) -> Option<Handle> {
        use windows::Win32::System::Threading::CreateMutexW;
        let wide = msm_core::paths::to_wide(name);
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(wide.as_ptr())).ok()?;
            Some(Handle(handle))
        }
    }
    fn wait(&self, timeout_ms: u32) -> bool {
        use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::WaitForSingleObject;
        unsafe {
            let result = WaitForSingleObject(self.0, timeout_ms);
            result == WAIT_OBJECT_0 || result == WAIT_ABANDONED
        }
    }
    fn try_wait_zero(&self) -> bool {
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::System::Threading::WaitForSingleObject;
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
    fn release(&self) {
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.0);
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Run `work` under the deployment→mutation gate (re-entrant on the same thread).
pub fn run<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, String> {
    let depth = DEPTH.with(|d| d.load(Ordering::SeqCst));
    if depth > 0 {
        DEPTH.with(|d| d.store(depth + 1, Ordering::SeqCst));
        let result = work();
        DEPTH.with(|d| d.store(depth, Ordering::SeqCst));
        return Ok(result);
    }
    let deployment = Handle::open(DEPLOYMENT_MUTEX).ok_or("deployment gate unavailable")?;
    if !deployment.wait(30_000) {
        return Err("Timed out waiting for the release deployment gate".into());
    }
    let mutation = Handle::open(MUTATION_MUTEX).ok_or("mutation gate unavailable")?;
    if !mutation.wait(30_000) {
        deployment.release();
        return Err("Timed out waiting for the startup transaction lock".into());
    }
    // Continuous handoff: release the outer gate only after the inner one is owned.
    deployment.release();
    DEPTH.with(|d| d.store(1, Ordering::SeqCst));
    let result = work();
    DEPTH.with(|d| d.store(0, Ordering::SeqCst));
    mutation.release();
    Ok(result)
}

/// Does any process currently hold the deployment gate? (release-child parent proof)
pub fn deployment_gate_is_held() -> bool {
    let Some(handle) = Handle::open(DEPLOYMENT_MUTEX) else { return false };
    let acquired = handle.try_wait_zero();
    if acquired {
        handle.release();
        false
    } else {
        true
    }
}
