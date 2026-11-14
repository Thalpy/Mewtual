//! Private namespace7/version3 storage lifecycle. Native consumers are separate.
//!
//! Only the private owning upgrade/strict3 opener registers this grammar.
//! Version1/2 remain closed; decoded rows grant no SDK, join, source or native authority.
//! See CHAT-V2-NATIVE-INDEX3-DESIGN.md for the reviewed upgrade boundary.
pub(super) mod admission;
mod capability;
mod catalog;
pub(super) mod cell;
mod codec;
mod data;
pub(super) mod founder;
mod inspection;
mod metadata;
mod migration;
mod network;
pub(super) mod preparation;
pub(super) mod refresh;
pub(super) mod rename;
pub(super) mod wal;

use super::super::refused;
use crate::AppError;

/// Closed descriptor validation for the explicit version3 DATA grammar only.
/// Only the private owning upgrade/strict3 opener configures its paid handler;
/// parsing a descriptor alone creates no owner.
pub(in crate::store::critical_namespaces) fn validate_descriptor_data(
    raw: &[u8],
    layout: super::super::archive_current_namespace_codec::CurrentLayoutData,
    current: &[u8],
) -> Result<(), AppError> {
    capability::Capability::decode_exact(
        raw,
        capability::Capability::for_current_data(layout, current)?,
    )?;
    Ok(())
}
