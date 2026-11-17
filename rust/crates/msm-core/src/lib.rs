//! Mich Startup Master — core inventory engine (Rust rewrite, v3).
//!
//! This crate owns the authoritative startup scan: every Windows surface that can
//! run code at boot or sign-in, the truthful enabled/disabled state of each
//! registration, dedupe/mirror rules, and the independent boot-coverage audit.
//! Layout mirrors the behavioral contract distilled in `docs/spec-rust/`.

pub mod model;
pub mod ids;
pub mod paths;
pub mod reg;
pub mod views;
pub mod hive;
pub mod hooks;
pub mod managed;
pub mod quiet_plan;
pub mod scan_registry;
pub mod scan_extensions;
pub mod scan_folders;
pub mod tasks;
pub mod tasks_com;
pub mod services;
pub mod wmi_scan;
pub mod packaged;
pub mod approvals;
pub mod dedupe;
pub mod humans;
pub mod truth;
pub mod runtime;
pub mod audit;
pub mod json;
pub mod util;
pub mod scan;
pub mod denykey;

pub use model::*;
pub use scan::{scan_all, scan_all_within};
