//! Opaque version2 storage owner. Its DATA projections deliberately cannot
//! construct a native backend, borrow Current/Store, or grant SDK/key entry.
use super::*;
use crate::store::{PreparedV2CurrentLayoutStore, V2CurrentLayoutPolicy, V2CurrentLayoutStore};
use catcoms_rt::CryptoRngCore;
use std::{path::Path, sync::Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeCreationStateData {
    Pending,
    Founder,
    Retired,
    Abandoned,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeCreationStatusData {
    pub id: u64,
    pub identity: [u8; 32],
    pub state: NativeCreationStateData,
}

pub struct NativeIndexStore {
    pub(super) _admission: interlock::AdmissionOwner,
    // Only this affine wrapper owns close. An admitted worker may retain the
    // private physical allocation, never a second admission owner or raw Store.
    pub(super) physical: Arc<NativeIndexPhysical>,
}

/// Physical custody for admitted finite work. This is deliberately private to
/// the concrete native-index implementation; no public loan/getter/clone API
/// exists. Retained transaction cells must not own an Arc back to this object.
pub(super) struct NativeIndexPhysical {
    // Retained cells/read metadata must drain before the actual mount and fee.
    pub(super) cell: Mutex<super::cell::IndexCell>,
    pub(super) current: V2CurrentLayoutStore,
    pub(super) handler: Handler,
}
impl std::fmt::Debug for NativeIndexStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeIndexStore").finish_non_exhaustive()
    }
}
impl NativeIndexStore {
    /// Real mounted storage access for the genuine JOIN regression only.
    /// Absent from production; no native command or source authority is issued.
    #[cfg(test)]
    pub(in crate::store) fn actual_store_for_test(
        &self,
    ) -> Result<&crate::store::ServerStore, AppError> {
        self.physical.current.cached_store()
    }

    pub fn open(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        mut current: V2CurrentLayoutPolicy,
        policy: NativeIndexPolicy,
    ) -> Result<Self, AppError> {
        let handler = policy.handler(&current)?; // Paid before the opener can repair anything.
        current.native_registry =
            Some(super::super::preparation::NativeRegistryHandler::Operational(handler.clone()));
        let mut prepared = PreparedV2CurrentLayoutStore::open(dir, passphrase, rng, current)?;
        prepared.retry(rng)?;
        let current = prepared
            .into_current_layout_store()
            .map_err(|_| refused())?;
        let value = Self {
            _admission: interlock::AdmissionOwner::new(handler.gate.clone()),
            physical: Arc::new(NativeIndexPhysical {
                current,
                handler,
                cell: Mutex::new(super::cell::IndexCell::default()),
            }),
        };
        value.row_count_data()?;
        Ok(value)
    }
    /// Irreversible whole-vault index/JOIN retirement fence. Do not call this
    /// for UI lock or one group's departure: those close their own issuers.
    /// Historical DATA reads may remain available; session drainage is separate.
    pub fn close_index_admission(&self) {
        self.physical.handler.gate.close();
    }

    pub fn row_count_data(&self) -> Result<usize, AppError> {
        self.physical.row_count_data()
    }
    /// Exact immutable receipt lookup. A conflicting request refuses; absence
    /// grants no permission to create a row, draw RNG, sign or start an SDK.
    pub fn lookup_creation_data(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<NativeCreationStatusData>, AppError> {
        self.physical.lookup_creation_data(invocation, request)
    }
}

/// Historical projection only; never a runtime/SDK admission.
pub(super) fn status_data(row: &data::Row) -> Result<NativeCreationStatusData, AppError> {
    let state = match &row.backend {
        data::Backend::Pending { .. } => NativeCreationStateData::Pending,
        data::Backend::Founder { .. } => NativeCreationStateData::Founder,
        data::Backend::Retired { .. } => NativeCreationStateData::Retired,
        data::Backend::Abandoned { .. } => NativeCreationStateData::Abandoned,
        data::Backend::Legacy { .. } => return Err(refused()),
    };
    Ok(NativeCreationStatusData {
        id: row.id,
        identity: row.identity,
        state,
    })
}
