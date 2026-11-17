//! Mutations: one-click enable/disable for every route type, bulk transactions,
//! managed startup lifecycle, agent registration, boot reconciliation.

pub mod mutation;
pub mod managed;
pub mod agent;
pub mod reconcile;
pub mod purge;

pub use mutation::*;

/// A raw store snapshot (present bytes, or an absent store) for rollback.
pub struct StoreSnapshotVec {
    pub path: std::path::PathBuf,
    pub bytes: Option<Vec<u8>>,
}
