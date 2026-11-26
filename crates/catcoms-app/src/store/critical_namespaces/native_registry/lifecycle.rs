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
pub(super) mod joined;
mod metadata;
mod migration;
pub(super) mod network;
pub(super) mod preparation;
pub(in crate::store) use preparation::{
    verify_governance_native, verify_joined_pin_parent, verify_shared_history_parent,
    VerifiedGovernanceNative,
};

/// Finish only the actual constructor-private native opening observation.
/// Caller keeps any new declaration witness overlapping these same files
/// before this method consumes/destroys the original cold observation.
pub(in crate::store::critical_namespaces) fn finish_governance_opening(
    store: &crate::store::ServerStore,
) -> Result<(), AppError> {
    match store
        .current_layout_policy
        .as_ref()
        .and_then(|p| p.native_registry.as_ref())
    {
        Some(super::preparation::NativeRegistryHandler::GovernanceOpening(handler)) => {
            handler.finish_cold(store).map(drop)
        }
        _ => Err(refused()),
    }
}

/// Consume the already-held cold observation only after the combined owner is
/// installed. A newly declared4 uses its separate retained declaration instead.
pub(in crate::store::critical_namespaces) fn finish_governed_cold_observation(
    store: &crate::store::ServerStore,
) -> Result<
    Vec<std::sync::Arc<crate::store::persistence::native_registry::network::NetworkDependency>>,
    AppError,
> {
    match store
        .current_layout_policy
        .as_ref()
        .and_then(|p| p.native_registry.as_ref())
    {
        Some(super::preparation::NativeRegistryHandler::GovernedLifecycle(handler)) => {
            handler.finish_cold(store)
        }
        _ => Err(refused()),
    }
}
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
