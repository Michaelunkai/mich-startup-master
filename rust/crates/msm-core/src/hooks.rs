//! Scan hooks: state-store providers injected by higher crates (keeps core pure).

use crate::model::StartupItem;
use crate::truth::ManagedIntentRow;
use once_cell::sync::OnceCell;
use std::sync::OnceLock;

pub struct ScanHooks {
    /// EnabledStartupService::IsQuietTask — is this task location quiet (tray)?
    pub is_quiet_task: fn(&str) -> bool,
    /// DisabledStoreService::LoadDisabledItems — preserved disabled routes.
    pub load_disabled_items: fn() -> Vec<StartupItem>,
    /// EnabledStartupService::Load — managed intent rows (target/arguments/mode).
    pub load_enabled_intent: fn() -> Vec<ManagedIntentRow>,
    /// DisabledStoreService::Remove — retire a stale disabled record exactly once.
    pub retire_disabled_record: fn(&str),
    /// EnabledStartupService::IsV2Migrated — one-shot v2 migration marker.
    pub is_v2_migrated: fn(&str) -> bool,
}

static HOOKS: OnceLock<ScanHooks> = OnceLock::new();
static INIT: OnceCell<()> = OnceCell::new();

/// Install the store-backed hooks exactly once (called by the app entrypoint).
pub fn install(hooks: ScanHooks) {
    let _ = INIT.set(()).map(|_| HOOKS.set(hooks));
}

/// The installed hooks, or inert defaults (empty stores) before installation.
pub fn hooks() -> &'static ScanHooks {
    static DEFAULT: OnceCell<ScanHooks> = OnceCell::new();
    HOOKS.get().unwrap_or_else(|| DEFAULT.get_or_init(|| ScanHooks {
        is_quiet_task: |_| false,
        load_disabled_items: Vec::new,
        load_enabled_intent: Vec::new,
        retire_disabled_record: |_| {},
        is_v2_migrated: |_| false,
    }))
}
