//! The UI layer: view model, verify gate, receipts. The native window itself is
//! rendered by msm-app (egui); this crate stays headless-testable.

pub mod view;
pub mod verify;

pub use view::*;
pub use verify::*;
