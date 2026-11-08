//! Closed internal choice of clock for Add proposal lifetime validation.
//!
//! There is deliberately no skip or caller callback variant. Legacy callers
//! still sample their clock only at the existing final leaf lifetime check.

#[cfg(not(target_arch = "wasm32"))]
use std::time::SystemTime;
#[cfg(target_arch = "wasm32")]
use web_time::SystemTime;

#[derive(Debug, Clone, Copy)]
pub(crate) enum AddedKeyPackageValidationTime {
    Legacy,
    Explicit(SystemTime),
}
