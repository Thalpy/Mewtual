//! Namespace7/version2 DATA and the forthcoming owning index admission path.
//!
//! Version1 stays a closed migration-only contract. In particular, adding birth
//! receipts here must not silently enlarge its grammar or authorize its reader.
//! A decoded receipt is historical DATA, never permission to instantiate an SDK,
//! construct a network transport, sign, or install a native backend.
// Internal DATA visibility for the owning version3 conversion only. This does
// not expose a physical Index/Store loan or change the closed version2 grammar.
pub(super) mod capability;
mod catalog;
mod cell;
pub(super) mod data;
mod facade;
mod interlock;
mod mount;
mod native_ui;
mod opening;
mod payment;
mod upgrade;
pub(in crate::store::critical_namespaces) mod upgrade_source;
pub use facade::{
    NativeTextVault, NativeTextVaultAuthentication, NativeTextVaultAuthenticationCheck,
    NativeTextVaultUpgrade, NativeTextVaultUpgradeRefusal,
};
pub use mount::{NativeCreationStateData, NativeCreationStatusData, NativeIndexStore};
pub use native_ui::{NativeUiInvocationBudget, NativeUiInvocationMetadata};
pub use native_ui::{
    RetirementBatch as NativeTextUiRetirementBatch, RetirementDrain as NativeTextUiRetirementDrain,
    UiCandidate as NativeTextUiCandidate, UiCapture as NativeTextUiObservation,
    UiGarbage as NativeTextUiGarbage, UiPreparation as NativeTextUiPreparation,
    UiRetirement as NativeTextUiRetirement, UiRetirementState as NativeTextUiRetirementState,
};
pub use upgrade::{NativeIndexPolicy, NativeIndexUpgrade};

use super::super::{read_pool::CatalogReadReservation, refused};
use crate::store::CriticalCatalogReadPool;
use crate::AppError;
use std::sync::Arc;

// Independent v2 upgrade, selected/candidate, decode/seal and fixed metadata
// overlap. This does not reuse a v1 installation fee or a member outbox account.
const CATALOG_FRAME: usize = super::preparation::MAX_CATALOG_FRAME;
const UPGRADE_SEED_FRAME: usize =
    2 * CATALOG_FRAME + 2 * data::MAX_FRAME + 2 * super::preparation::MAX_MARKER_FRAME + 4096;
// At most four decoded selected/staged/migration vectors of EACH schema plus
// eight fixed identity/invocation scratch vectors. Payload bytes/AEAD/encoding
// copies belong to the separate frame terms; this is their actual Rust metadata.
const ROW_METADATA_FEE: usize = 4 * super::data::MAX_ROWS * 1536;
const CONTROL_METADATA_FEE: usize = 1024 * 1024;
const _: () = assert!(
    std::mem::size_of::<data::Row>()
        + std::mem::size_of::<super::data::RowData>()
        + 8 * std::mem::size_of::<[u8; 32]>()
        <= 1536
);
const _: () = assert!(
    8 * (std::mem::size_of::<mount::NativeIndexStore>()
        + std::mem::size_of::<mount::NativeIndexPhysical>()
        + std::mem::size_of::<facade::NativeTextVault>()
        + std::mem::size_of::<upgrade::NativeIndexUpgrade>()
        + std::mem::size_of::<crate::store::persistence::native_registry::operational::UpgradeIo>(
        )
        + std::mem::size_of::<
            crate::store::persistence::native_registry::operational::UpgradeInventory,
        >()
        + std::mem::size_of::<
            crate::store::persistence::native_registry::operational::IndexCommitIo,
        >()
        + std::mem::size_of::<
            crate::store::persistence::native_registry::operational::IndexStageCleanup,
        >()
        + std::mem::size_of::<interlock::IndexInterlock>()
        + std::mem::size_of::<Handler>()
        + std::mem::size_of::<payment::IndexPayment>())
        + cell::CELL_METADATA_BYTES
        + 32 * (super::preparation::MAX_BASE_PATH + 256)
        + 64 * 1024
        <= CONTROL_METADATA_FEE
);
const OPERATIONAL_FEE: usize = 10 * CATALOG_FRAME
    + 12 * data::MAX_FRAME
    + 12 * super::preparation::MAX_MARKER_FRAME
    + 4 * UPGRADE_SEED_FRAME
    + ROW_METADATA_FEE
    + CONTROL_METADATA_FEE;

#[derive(Clone)]
pub(in crate::store) struct Handler {
    // Claims retain this gate AND its exact payer. Gate metadata drains before
    // the handler's final fee reference; a late claim can never outlive payment.
    gate: Arc<interlock::IndexInterlock>,
    pool: CriticalCatalogReadPool,
    fee: Arc<payment::IndexPayment>,
}
impl Handler {
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.gate, &other.gate) && Arc::ptr_eq(&self.fee, &other.fee)
    }
    pub(in crate::store) fn require(&self) -> Result<(), AppError> {
        self.fee
            .debit
            .validate(&self.pool, OPERATIONAL_FEE)
            .map_err(|_| refused())
    }
    // Catalog-held JOIN checks must be cached/negative only. In particular do
    // not call require(), acquire a pool, read files or take the index cell here.
    pub(in crate::store) fn require_index_idle_cached(&self) -> Result<(), AppError> {
        if self.gate.idle() {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

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

impl crate::store::ServerStore {
    /// Negative-only JOIN fence. Runtime callers already hold the actual catalog
    /// mutex; startup callers are still private. Never take the index cell, read
    /// files, acquire payment or call an admission/SDK getter under that mutex.
    pub(in crate::store) fn require_native_index_idle_cached(&self) -> Result<(), AppError> {
        if let Some(handler) = self
            .current_layout_policy
            .as_ref()
            .and_then(|policy| policy.native_registry.as_ref())
        {
            if self.chat_mount.observer().is_cancelled() {
                return Err(refused());
            }
            match handler {
                super::preparation::NativeRegistryHandler::Operational(handler) => {
                    handler.require_index_idle_cached()?
                }
                super::preparation::NativeRegistryHandler::Lifecycle(handler) => {
                    handler.require_idle_cached()?
                }
                super::preparation::NativeRegistryHandler::Migration(_) => {}
            }
        }
        Ok(())
    }
}
