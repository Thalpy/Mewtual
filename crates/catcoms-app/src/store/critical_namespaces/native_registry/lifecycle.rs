//! Namespace7/version3 production lifecycle contract, under construction.
//!
//! This module is not yet registered by any opener. Version1/2 remain closed;
//! no decoded descriptor/row here grants SDK, join, source or native authority.
//! See CHAT-V2-NATIVE-INDEX3-DESIGN.md for the reviewed upgrade boundary.
mod capability;
mod catalog;
mod codec;
mod data;
mod migration;
pub(super) mod preparation;

use super::super::refused;
use crate::AppError;

/// Closed descriptor validation for the explicit version3 DATA grammar only.
/// No configured opener/handler selects it yet, and parsing creates no owner.
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
