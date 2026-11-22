//! Exact covered native Pending and first index insertion. Covering these
//! files grants historical request custody. Welcome has its own entry fence.
use super::*;
use catcoms_replication::chat::join_bindings::SignedInviteBindingData;
use catcoms_sync::{DirectJoinLimits, PreparedV2JoinRequest, MAX_V2_FIRST_JOIN_DELIVERY_BYTES};
use critical_namespaces::joined_pin::pending::{
    PreparedPinnedPending, RecoveredPinnedPending, MAX_FRAME,
};

const PENDING_NAME: &str = "chat-critical-namespaces.joined-pending.bin";
const INDEX_STAGE: &str = "chat-critical-namespaces.joined-pin.stage";
const SOURCE_FEE: usize = 12 * MAX_FRAME + 4 * 1024 * 1024;
const SOURCE_ID_DOMAIN: &str = "catcoms/private-chat-v2-native-pending-source-id/v1";

fn limits() -> Result<DirectJoinLimits, AppError> {
    DirectJoinLimits::new(
        512 * 1024,
        2 * 1024 * 1024,
        MAX_V2_FIRST_JOIN_DELIVERY_BYTES,
    )
    .map_err(|_| denied())
}

fn original_pending_id(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    selected_index: &[u8],
    request: &[u8],
) -> [u8; 32] {
    let key = Zeroizing::new(blake3::derive_key(SOURCE_ID_DOMAIN, key));
    let mut source_id = blake3::Hasher::new_keyed(&key);
    source_id.update(&layout.vault);
    source_id.update(&layout.root);
    source_id.update(blake3::hash(selected_index).as_bytes());
    source_id.update(blake3::hash(request).as_bytes());
    *source_id.finalize().as_bytes()
}

pub(super) struct SelectedPending {
    original: OpenedArchiveObject,
    _pending: RecoveredPinnedPending,
    _fee: CatalogReadReservation,
}

pub(super) fn require_pending_names(
    store: &ServerStore,
    pending: Option<&SelectedPending>,
) -> Result<(), AppError> {
    absent(&fixed(store, false, INDEX_STAGE)?)?;
    let path = fixed(store, false, PENDING_NAME)?;
    if let Some(pending) = pending {
        let (file, bytes) = &pending.original;
        exact_held(&path, file, session_id(file).map_err(io)?, bytes)
    } else {
        absent(&path)
    }
}

pub(super) fn select(
    store: &ServerStore,
    key: &[u8; 32],
    layout: CurrentLayoutData,
    directory: &GovernanceDirectoryCapture,
    catalog: (&File, &[u8]),
    index_fee: CatalogReadReservation,
) -> Result<SelectedPinIndex, AppError> {
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    let fee = policy
        .current
        .archive
        .pending
        .pool
        .try_reserve(SOURCE_FEE)?;
    let index_path = fixed(store, false, INDEX_NAME)?;
    let stage_path = fixed(store, false, INDEX_STAGE)?;
    let pending_path = fixed(store, false, PENDING_NAME)?;
    let current_path = fixed(store, false, critical_namespaces::Slot::Current.name())?;
    let current_id = session_id(catalog.0).map_err(io)?;
    let mut index = SelectedPinIndex::open(store, key, layout, None, index_fee)?;
    let Some(original) = read(&pending_path, MAX_FRAME)? else {
        require_pending_names(store, None)?;
        return Ok(index);
    };
    let pending_id = session_id(&original.0).map_err(io)?;
    let pending = RecoveredPinnedPending::open(key, layout, &original.1, limits()?)?;
    let (before, after) = pending.index_frames();
    let (request, context) = pending.pending_data();
    if context.original_pending
        != original_pending_id(key, layout, before, request.request_bytes_data())
    {
        return Err(denied());
    }
    let selected = index.selected.1.as_slice();
    if selected == before {
        let mut stage = match read(&stage_path, after.len())? {
            Some((file, bytes)) if after.starts_with(&bytes) => Some(CreatedStage {
                identity: Some(session_id(&file).map_err(io)?),
                file,
            }),
            Some(_) => return Err(denied()),
            None => None,
        };
        let before_selection = || {
            store.require_governance_directory_matches_root_if_mounted(directory)?;
            exact_held(&pending_path, &original.0, pending_id, &original.1)?;
            exact_held(&current_path, catalog.0, current_id, catalog.1)?;
            index.require_file(store)
        };
        before_selection()?;
        if stage.is_none() {
            ensure_native_claim(&stage_path, &mut stage)?;
        }
        ensure(
            &stage_path,
            after,
            &mut stage,
            ArchiveOutputMode::SeedRepair,
            #[cfg(test)]
            0,
            #[cfg(test)]
            0,
        )?;
        sync_directory(&store.dir).map_err(io)?;
        before_selection()?;
        let stage = stage.ok_or_else(denied)?;
        exact_retained(&stage_path, &stage, after)?;
        held(&current_path, catalog.0, current_id)?;
        held(
            &index_path,
            &index.selected.0,
            session_id(&index.selected.0).map_err(io)?,
        )?;
        fs::rename(&stage_path, &index_path).map_err(io)?;
        index = SelectedPinIndex::open(store, key, layout, Some(&stage), index._fee)?;
    } else if selected != after {
        return Err(denied());
    }
    absent(&stage_path)?;
    if index.selected.1.as_slice() != after {
        return Err(denied());
    }
    let durable = |error: std::io::Error| AppError::CommittedButNotDurable(error.to_string());
    index.selected.0.sync_all().map_err(durable)?;
    sync_directory(&store.dir).map_err(durable)?;
    index.pending = Some(SelectedPending {
        original,
        _pending: pending,
        _fee: fee,
    });
    index.require(store)?;
    Ok(index)
}

pub(in crate::store) struct JoinedPendingIo {
    _prepared: PreparedPinnedPending,
    exact: Zeroizing<Vec<u8>>,
    pending: Option<CreatedStage>,
    _fee: CatalogReadReservation,
}

impl ServerStore {
    pub(in crate::store) fn prepare_joined_pending(
        &self,
        source: PreparedV2JoinRequest,
        invite: &SignedInviteBindingData,
        rng: &mut impl CryptoRngCore,
    ) -> Result<JoinedPendingIo, AppError> {
        require_pending_names(self, None)?;
        self.validate_mounted_governance_layout()?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(SOURCE_FEE)?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        selected.require_file(self)?;
        let layout = selected.index.layout_data();
        let key = Zeroizing::new(self.keys.db_key()?);
        let source_id = original_pending_id(
            &key,
            layout,
            &selected.selected.1,
            source.request_bytes_data(),
        );
        let mut prepared = PreparedPinnedPending::prepare(
            source,
            &selected.selected.1,
            invite,
            &key,
            layout,
            source_id,
            rng,
        )?;
        let exact = Zeroizing::new(prepared.seal(&key, rng)?.to_vec());
        let recovered = RecoveredPinnedPending::open(&key, layout, &exact, limits()?)?;
        if recovered.index_frames().0 != selected.selected.1.as_slice() {
            return Err(denied());
        }
        selected.require_file(self)?;
        require_pending_names(self, None)?;
        Ok(JoinedPendingIo {
            _prepared: prepared,
            exact,
            pending: None,
            _fee: fee,
        })
    }

    pub(in crate::store) fn selected_pending_join_data(
        &self,
    ) -> Result<Option<([u8; 32], catcoms_sync::V2PendingJoinContextData)>, AppError> {
        self.validate_mounted_governance_layout()?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let index = slot.as_ref().ok_or_else(denied)?;
        let data = index.pending.as_ref().map(|pending| {
            let (source, context) = pending._pending.pending_data();
            (
                *blake3::hash(source.request_bytes_data()).as_bytes(),
                context,
            )
        });
        index.require(self)?;
        Ok(data)
    }
}

impl JoinedPendingIo {
    pub(in crate::store) fn stage(&mut self, store: &ServerStore) -> Result<(), AppError> {
        store.validate_mounted_governance_layout()?;
        let path = fixed(store, false, PENDING_NAME)?;
        let slot = store.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let expected = RecoveredPinnedPending::open(
            &key,
            selected.index.layout_data(),
            &self.exact,
            limits()?,
        )?;
        if expected.index_frames().0 != selected.selected.1.as_slice() {
            return Err(denied());
        }
        selected.require_file(store)?;
        absent(&fixed(store, false, INDEX_STAGE)?)?;
        ensure_native_claim(&path, &mut self.pending)?;
        sync_directory(&store.dir).map_err(io)?;
        ensure(
            &path,
            &self.exact,
            &mut self.pending,
            ArchiveOutputMode::Original,
            #[cfg(test)]
            0,
            #[cfg(test)]
            0,
        )?;
        sync_directory(&store.dir).map_err(io)?;
        selected.require_file(store)?;
        exact_retained(
            &path,
            self.pending.as_ref().ok_or_else(denied)?,
            &self.exact,
        )
    }
}
