//! Opaque version2 storage owner. Its DATA projections deliberately cannot
//! construct a native backend, borrow Current/Store, or grant SDK/key entry.
use super::*;
use crate::store::persistence::native_registry::operational as io;
use crate::store::{PreparedV2CurrentLayoutStore, V2CurrentLayoutPolicy, V2CurrentLayoutStore};
use catcoms_rt::CryptoRngCore;
use std::{path::Path, sync::Mutex};
use zeroize::Zeroizing;

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
    // Retained cells/read metadata must drain before the actual mount and fee.
    pub(super) read: Mutex<()>,
    pub(super) current: V2CurrentLayoutStore,
    pub(super) handler: Handler,
}
impl std::fmt::Debug for NativeIndexStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeIndexStore").finish_non_exhaustive()
    }
}
impl NativeIndexStore {
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
            current,
            handler,
            read: Mutex::new(()),
        };
        value.row_count_data()?;
        Ok(value)
    }
    /// Irreversible whole-vault index/JOIN retirement fence. Do not call this
    /// for UI lock or one group's departure: those close their own issuers.
    /// Historical DATA reads may remain available; session drainage is separate.
    pub fn close_index_admission(&self) {
        self.handler.gate.close();
    }

    fn read_index(&self) -> Result<data::Index, AppError> {
        self.handler.require()?;
        let store = self.current.checked_store()?;
        let snapshot = io::IndexSnapshot::capture(store)?;
        if snapshot.stage().is_some() {
            return Err(refused());
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = upgrade::catalog_plain(&key, snapshot.catalog())?;
        let cap = upgrade::capability_data(&plain, &store.chat_vault_domain()?)?;
        capability::Capability::decode_exact(
            &upgrade::authenticate(
                &key,
                upgrade::MARKER_KEY,
                snapshot.marker(),
                super::super::preparation::MAX_MARKER_FRAME,
            )?,
            cap,
        )?;
        let index = data::Index::decode_data(
            &upgrade::authenticate(&key, upgrade::INDEX_KEY, snapshot.index(), data::MAX_FRAME)?,
            cap,
        )?;
        snapshot.require_selected(store)?;
        self.current.cached_store()?;
        Ok(index)
    }
    pub fn row_count_data(&self) -> Result<usize, AppError> {
        let _read = self.read.try_lock().map_err(|_| refused())?;
        Ok(self.read_index()?.rows.len())
    }
    /// Exact immutable receipt lookup. A conflicting request refuses; absence
    /// grants no permission to create a row, draw RNG, sign or start an SDK.
    pub fn lookup_creation_data(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<NativeCreationStatusData>, AppError> {
        let _read = self.read.try_lock().map_err(|_| refused())?;
        let index = self.read_index()?;
        let Some(row) = index.lookup_data(invocation, request)? else {
            return Ok(None);
        };
        let state = match &row.backend {
            data::Backend::Pending { .. } => NativeCreationStateData::Pending,
            data::Backend::Founder { .. } => NativeCreationStateData::Founder,
            data::Backend::Retired { .. } => NativeCreationStateData::Retired,
            data::Backend::Abandoned { .. } => NativeCreationStateData::Abandoned,
            data::Backend::Legacy { .. } => return Err(refused()),
        };
        Ok(Some(NativeCreationStatusData {
            id: row.id,
            identity: row.identity,
            state,
        }))
    }
}
