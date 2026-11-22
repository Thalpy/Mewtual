//! Exact covered native Pending and first index insertion. Covering these
//! files grants historical request custody. Welcome has its own entry fence.
use super::*;
use catcoms_replication::chat::join_bindings::{GenesisAnchorBindingData, SignedInviteBindingData};
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_sync::{DirectJoinLimits, PreparedV2JoinRequest, MAX_V2_FIRST_JOIN_DELIVERY_BYTES};
use critical_namespaces::joined_pin::pending::{
    JoinedTransportReservation, PreparedPinnedPending, RecoveredPinnedPending, MAX_FRAME,
};

const PENDING_NAME: &str = "chat-critical-namespaces.joined-pending.bin";
const INDEX_STAGE: &str = "chat-critical-namespaces.joined-pin.stage";
const SOURCE_FEE: usize = 12 * MAX_FRAME + 4 * 1024 * 1024;
const SOURCE_ID_DOMAIN: &str = "catcoms/private-chat-v2-native-pending-source-id/v1";
mod welcome;

fn first_join_limits() -> Result<DirectJoinLimits, AppError> {
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
    transport: Option<(catcoms_rt::PeerId, u16, [u8; 32])>,
) -> [u8; 32] {
    let key = Zeroizing::new(blake3::derive_key(SOURCE_ID_DOMAIN, key));
    let mut source_id = blake3::Hasher::new_keyed(&key);
    source_id.update(&layout.vault);
    source_id.update(&layout.root);
    source_id.update(blake3::hash(selected_index).as_bytes());
    source_id.update(blake3::hash(request).as_bytes());
    if let Some((_, port, identity)) = transport {
        source_id.update(b"/native-tcp/v2\0");
        source_id.update(&identity);
        source_id.update(&port.to_be_bytes());
    }
    *source_id.finalize().as_bytes()
}

pub(super) struct SelectedPending {
    original: OpenedArchiveObject,
    _pending: RecoveredPinnedPending,
    welcome: Option<welcome::RecoveredWelcome>,
    _fee: CatalogReadReservation,
}

impl SelectedPending {
    pub(super) fn finish_core(&mut self) -> Result<(), AppError> {
        if let Some(welcome) = self.welcome.as_mut() {
            welcome.finish_core()?;
        }
        Ok(())
    }
}

pub(super) struct WarmSelection<'a> {
    original_pending: &'a CreatedStage,
    previous_index: &'a SelectedPinIndex,
    stage: &'a mut Option<CreatedStage>,
}

pub(super) fn require_pending_names(
    store: &ServerStore,
    pending: Option<&SelectedPending>,
) -> Result<(), AppError> {
    absent(&fixed(store, false, INDEX_STAGE)?)?;
    let path = fixed(store, false, PENDING_NAME)?;
    if let Some(pending) = pending {
        let (file, bytes) = &pending.original;
        exact_held(&path, file, session_id(file).map_err(io)?, bytes)?;
        if let Some(welcome) = &pending.welcome {
            welcome.require(store)?;
        }
        Ok(())
    } else {
        absent(&path)?;
        welcome::require_absent(store)
    }
}

pub(super) fn select(
    store: &ServerStore,
    key: &[u8; 32],
    layout: CurrentLayoutData,
    directory: &GovernanceDirectoryCapture,
    catalog: (&File, &[u8]),
    index_fee: CatalogReadReservation,
    mut warm: Option<WarmSelection<'_>>,
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
        // This profile inserts its first pin only through the exact covered
        // Pending. A selected pin already names its original acceptance.
        if warm.is_some() || !index.index.is_empty() {
            return Err(denied());
        }
        require_pending_names(store, None)?;
        return Ok(index);
    };
    let pending_id = session_id(&original.0).map_err(io)?;
    if warm
        .as_ref()
        .is_some_and(|owned| owned.original_pending.identity != Some(pending_id))
    {
        return Err(denied());
    }
    let pending = RecoveredPinnedPending::open(key, layout, &original.1, first_join_limits()?)?;
    let (before, after) = pending.index_frames();
    let (request, context) = pending.pending_data();
    if context.original_pending
        != original_pending_id(
            key,
            layout,
            before,
            request.request_bytes_data(),
            pending.transport_data()?,
        )
    {
        return Err(denied());
    }
    let selected = index.selected.1.as_slice();
    let welcome = if selected == after && warm.is_none() {
        absent(&stage_path)?;
        store.require_governance_directory_matches_root_if_mounted(directory)?;
        exact_held(&pending_path, &original.0, pending_id, &original.1)?;
        exact_held(&current_path, catalog.0, current_id, catalog.1)?;
        index.require_file(store)?;
        welcome::recover(store, key, &original.1, &pending)?
    } else {
        welcome::require_absent(store)?;
        None
    };
    if selected == before {
        let mut cold_stage;
        let stage = if let Some(owned) = warm.as_mut() {
            owned.previous_index.require_file(store)?;
            &mut *owned.stage
        } else {
            cold_stage = match read(&stage_path, after.len())? {
                Some((file, bytes)) if after.starts_with(&bytes) => Some(CreatedStage {
                    identity: Some(session_id(&file).map_err(io)?),
                    file,
                }),
                Some(_) => return Err(denied()),
                None => None,
            };
            &mut cold_stage
        };
        let before_selection = || {
            store.require_governance_directory_matches_root_if_mounted(directory)?;
            exact_held(&pending_path, &original.0, pending_id, &original.1)?;
            exact_held(&current_path, catalog.0, current_id, catalog.1)?;
            index.require_file(store)
        };
        before_selection()?;
        if stage.is_none() {
            ensure_native_claim(&stage_path, stage)?;
        }
        ensure(
            &stage_path,
            after,
            stage,
            ArchiveOutputMode::SeedRepair,
            #[cfg(test)]
            0,
            #[cfg(test)]
            0,
        )?;
        sync_directory(&store.dir).map_err(io)?;
        before_selection()?;
        let stage = stage.as_ref().ok_or_else(denied)?;
        exact_retained(&stage_path, stage, after)?;
        held(&current_path, catalog.0, current_id)?;
        held(
            &index_path,
            &index.selected.0,
            session_id(&index.selected.0).map_err(io)?,
        )?;
        fs::rename(&stage_path, &index_path).map_err(io)?;
        index = SelectedPinIndex::open(store, key, layout, Some(stage), index._fee)?;
    } else if selected != after {
        return Err(denied());
    } else if let Some(owned) = warm {
        exact_retained(&index_path, owned.stage.as_ref().ok_or_else(denied)?, after)?;
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
        welcome,
        _fee: fee,
    });
    index.require(store)?;
    Ok(index)
}

pub(in crate::store) struct JoinedPendingIo {
    prepared: PreparedPinnedPending,
    exact: Zeroizing<Vec<u8>>,
    pending: Option<CreatedStage>,
    staged: bool,
    stage: Option<CreatedStage>,
    directory: GovernanceDirectoryCapture,
    catalog: OpenedArchiveObject,
    welcome: Option<welcome::WarmWelcome>,
    _fee: CatalogReadReservation,
    _catalog_fee: CatalogReadReservation,
}

pub(in crate::store) struct AcceptedFirstJoin<'a> {
    pub(in crate::store) invite: &'a crate::store::NativeTextInvitePacketData<'a>,
    pub(in crate::store) anchor: &'a GenesisAnchorBindingData,
    pub(in crate::store) token: [u8; 32],
    pub(in crate::store) now_ms: u64,
    pub(in crate::store) transport: AcceptedJoinTransport,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum AcceptedJoinTransport {
    External,
    Native,
}

#[cfg(test)]
std::thread_local! {
    pub(in crate::store) static AFTER_PIN_INDEX_SELECT: std::cell::Cell<Option<Box<dyn FnOnce()>>> = const { std::cell::Cell::new(None) };
}

impl ServerStore {
    pub(in crate::store) fn cover_accepted_first_join(
        &self,
        pending: &mut Option<JoinedPendingIo>,
        accepted: AcceptedFirstJoin<'_>,
        require_caller: impl Fn() -> Result<(), AppError>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<([u8; 32], catcoms_sync::V2PendingJoinContextData), AppError> {
        let binding = accepted.invite.binding();
        if accepted.token == [0; 32] || accepted.anchor != &binding.spec_data().anchor {
            return Err(denied());
        }
        let mount = self.chat_mount.observer();
        let require_live = || {
            if mount.is_cancelled() {
                return Err(denied());
            }
            require_caller()
        };
        require_live()?;
        if let Some(original) = pending.as_ref() {
            original.require_original_join_intent(binding, accepted.token)?;
            if original.transport_data()?.is_some()
                != (accepted.transport == AcceptedJoinTransport::Native)
            {
                return Err(denied());
            }
        } else {
            if binding.expired_at_data(accepted.now_ms)
                || self.selected_pending_join_data()?.is_some()
                || self
                    .selected_joined_pin_context_data(accepted.anchor)?
                    .is_some()
            {
                return Err(denied());
            }
            // Temporary device, KeyPackage and provider snapshot overlap. A
            // sealed Pending retains its separate persistent payment.
            const PREPARATION_BYTES: usize = 16 * 1024 * 1024;
            let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
            let pool = &policy.current.archive.pending.pool;
            let _fee = pool.try_reserve(PREPARATION_BYTES)?;
            require_live()?;
            let source = PreparedV2JoinRequest::prepare(
                catcoms_mls::MlsDevice::generate()?,
                accepted.anchor,
                binding,
                accepted.token,
                accepted.now_ms,
                first_join_limits()?,
            )?;
            require_live()?;
            let transport = match accepted.transport {
                AcceptedJoinTransport::External => None,
                AcceptedJoinTransport::Native => Some(JoinedTransportReservation::generate(rng)?),
            };
            require_live()?;
            *pending = Some(self.prepare_joined_pending(source, binding, transport, rng)?);
        }
        let original = pending.as_mut().ok_or_else(denied)?;
        if original.request_data(self).is_err() {
            require_live()?;
            original.stage(self)?;
            require_live()?;
            original.install(self)?;
        }
        let selected = self.selected_pending_join_data()?.ok_or_else(denied)?;
        if blake3::hash(original.request_data(self)?).as_bytes() != &selected.0 {
            return Err(denied());
        }
        require_live()?;
        Ok(selected)
    }

    pub(in crate::store) fn prepare_joined_pending(
        &self,
        source: PreparedV2JoinRequest,
        invite: &SignedInviteBindingData,
        transport: Option<JoinedTransportReservation>,
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
        let catalog_fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(governance::codec::DECLARATION_FEE)?;
        let inventory = self.read_governance_inventory(&catalog_fee)?;
        if inventory.stage.is_some() || inventory.seed.is_some() {
            return Err(denied());
        }
        let catalog = inventory.current.ok_or_else(denied)?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        selected.require_file(self)?;
        let layout = selected.index.layout_data();
        let key = Zeroizing::new(self.keys.db_key()?);
        let catalog_plain = Zeroizing::new(unseal(&key, &unframe(&catalog.1)?)?);
        if self.parse_joined_pin_catalog(&catalog_plain)? != layout {
            return Err(denied());
        }
        drop(catalog_plain);
        let source_id = original_pending_id(
            &key,
            layout,
            &selected.selected.1,
            source.request_bytes_data(),
            transport
                .as_ref()
                .map(JoinedTransportReservation::identity_data)
                .transpose()?,
        );
        let mut prepared = PreparedPinnedPending::prepare(
            source,
            &selected.selected.1,
            invite,
            &key,
            layout,
            (source_id, transport),
            rng,
        )?;
        let exact = Zeroizing::new(prepared.seal(&key, rng)?.to_vec());
        let recovered = RecoveredPinnedPending::open(&key, layout, &exact, first_join_limits()?)?;
        if recovered.index_frames().0 != selected.selected.1.as_slice() {
            return Err(denied());
        }
        selected.require_file(self)?;
        require_pending_names(self, None)?;
        let output = JoinedPendingIo {
            prepared,
            exact,
            pending: None,
            staged: false,
            stage: None,
            directory: inventory.directory,
            catalog,
            welcome: None,
            _fee: fee,
            _catalog_fee: catalog_fee,
        };
        output.require_catalog(self)?;
        Ok(output)
    }

    pub(in crate::store) fn recovered_join_transport_data(
        &self,
    ) -> Result<Option<(catcoms_rt::PeerId, u16)>, AppError> {
        self.validate_mounted_governance_layout()?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        selected.require(self)?;
        let transport = selected
            .pending
            .as_ref()
            .map(|pending| pending._pending.transport_data())
            .transpose()?
            .flatten();
        selected.require(self)?;
        Ok(transport.map(|(peer, port, _)| (peer, port)))
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

    pub(in crate::store) fn recovered_join_outcome_data(
        &self,
    ) -> Result<Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)>, AppError>
    {
        self.validate_mounted_governance_layout()?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let index = slot.as_ref().ok_or_else(denied)?;
        let result = index
            .pending
            .as_ref()
            .and_then(|pending| pending.welcome.as_ref())
            .and_then(welcome::RecoveredWelcome::outcome_data);
        index.require(self)?;
        Ok(result)
    }

    pub(in crate::store) fn copy_recovered_governing_proof(
        &self,
        headers: (OwnerHeaderId, OwnerHeaderId),
        output: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        self.validate_mounted_governance_layout()?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        selected.require(self)?;
        selected
            .pending
            .as_ref()
            .and_then(|pending| pending.welcome.as_ref())
            .ok_or_else(denied)?
            .copy_selected_governing_proof(headers, output)?;
        selected.require(self)
    }

    fn require_recovered_request<'a>(
        &self,
        selected: &'a SelectedPinIndex,
    ) -> Result<&'a catcoms_sync::RecoveredV2PendingJoinSource, AppError> {
        selected.require(self)?;
        welcome::require_absent(self)?;
        let pending = selected
            .pending
            .as_ref()
            .filter(|pending| pending.welcome.is_none())
            .ok_or_else(denied)?;
        Ok(pending._pending.pending_data().0)
    }

    pub(in crate::store) fn copy_recovered_join_request(
        &self,
        output: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        self.validate_mounted_governance_layout()?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        let bytes = self
            .require_recovered_request(selected)?
            .request_bytes_data();
        if !output.is_empty() || bytes.len() > output.capacity() {
            return Err(denied());
        }
        output.extend_from_slice(bytes);
        self.require_recovered_request(selected).map(|_| ())
    }

    pub(in crate::store) fn validate_recovered_join_delivery(
        &self,
        exact_request: &[u8],
        delivery: &[u8],
    ) -> Result<(), AppError> {
        self.validate_mounted_governance_layout()?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        let source = self.require_recovered_request(selected)?;
        if source.request_bytes_data() != exact_request {
            return Err(denied());
        }
        source
            .validate_first_join_delivery_data(delivery)
            .map_err(|_| denied())?;
        self.require_recovered_request(selected).map(|_| ())
    }

    /// Exclusive access keeps the original held index and Welcome outside
    /// mutex guards during runtime assembly.
    pub(in crate::store) fn attach_recovered_joined_node<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &mut self,
        inputs: &mut Option<JoinedRuntimeInputs<T, R>>,
        output: &mut Option<catcoms_sync::ClosedV2JoinedNode<T, R>>,
    ) -> Result<(), AppError> {
        let original = self.recovered_join_outcome_data()?.ok_or_else(denied)?;
        if output.is_none() {
            let welcome = self
                .joined_pin_index
                .get_mut()
                .map_err(|_| denied())?
                .as_mut()
                .and_then(|index| index.pending.as_mut())
                .and_then(|pending| pending.welcome.as_mut())
                .ok_or_else(denied)?;
            *output = Some(welcome.attach_node(inputs)?);
        }
        if self.recovered_join_outcome_data()? != Some(original) {
            return Err(denied());
        }
        Ok(())
    }
}

impl JoinedPendingIo {
    pub(in crate::store) fn native_transport_seed(&self) -> Result<Zeroizing<[u8; 32]>, AppError> {
        self.prepared.native_transport_seed()
    }

    pub(in crate::store) fn native_transport_data(
        &self,
    ) -> Result<(catcoms_rt::PeerId, u16), AppError> {
        self.transport_data()?
            .map(|(peer, port, _)| (peer, port))
            .ok_or_else(denied)
    }

    fn transport_data(&self) -> Result<Option<(catcoms_rt::PeerId, u16, [u8; 32])>, AppError> {
        self.prepared.transport_data()
    }

    pub(in crate::store) fn require_original_join_intent(
        &self,
        invite: &SignedInviteBindingData,
        request_token: [u8; 32],
    ) -> Result<(), AppError> {
        let packet =
            catcoms_sync::V2JoinRequestPacketData::decode(self.prepared.request_bytes_data())
                .map_err(|_| denied())?;
        let original = packet.founder_add_data();
        if original.invite.encode() != invite.encode()
            || original.request.spec_data().request_token != request_token
        {
            return Err(denied());
        }
        Ok(())
    }

    fn require_catalog(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_governance_directory_matches_root(&self.directory)?;
        absent(&fixed(store, false, governance::GOVERNANCE_SEED)?)?;
        absent(&fixed(
            store,
            false,
            critical_namespaces::Slot::Stage.name(),
        )?)?;
        exact_held(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            &self.catalog.0,
            session_id(&self.catalog.0).map_err(io)?,
            &self.catalog.1,
        )
    }

    pub(in crate::store) fn stage(&mut self, store: &ServerStore) -> Result<(), AppError> {
        store.validate_mounted_governance_layout()?;
        self.require_catalog(store)?;
        let path = fixed(store, false, PENDING_NAME)?;
        if self.staged {
            return exact_retained(
                &path,
                self.pending.as_ref().ok_or_else(denied)?,
                &self.exact,
            );
        }
        let slot = store.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let expected = RecoveredPinnedPending::open(
            &key,
            selected.index.layout_data(),
            &self.exact,
            first_join_limits()?,
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
        self.require_catalog(store)?;
        selected.require_file(store)?;
        exact_retained(
            &path,
            self.pending.as_ref().ok_or_else(denied)?,
            &self.exact,
        )?;
        self.staged = true;
        Ok(())
    }

    pub(in crate::store) fn install(&mut self, store: &ServerStore) -> Result<(), AppError> {
        if !self.staged {
            return Err(denied());
        }
        store.validate_mounted_governance_layout()?;
        self.require_catalog(store)?;
        let claim = self.pending.as_ref().ok_or_else(denied)?;
        exact_retained(&fixed(store, false, PENDING_NAME)?, claim, &self.exact)?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let index_fee = policy.current.archive.pending.pool.try_reserve(INDEX_FEE)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let mut slot = store.joined_pin_index.lock().map_err(|_| denied())?;
        let previous_index = slot.as_ref().ok_or_else(denied)?;
        let selected = select(
            store,
            &key,
            previous_index.index.layout_data(),
            &self.directory,
            (&self.catalog.0, &self.catalog.1),
            index_fee,
            Some(WarmSelection {
                original_pending: claim,
                previous_index,
                stage: &mut self.stage,
            }),
        )?;
        #[cfg(test)]
        AFTER_PIN_INDEX_SELECT.with(|slot| {
            if let Some(hook) = slot.take() {
                hook();
            }
        });
        self.require_catalog(store)?;
        *slot = Some(selected);
        Ok(())
    }

    pub(in crate::store) fn request_data<'a>(
        &'a self,
        store: &ServerStore,
    ) -> Result<&'a [u8], AppError> {
        store.validate_mounted_governance_layout()?;
        self.require_catalog(store)?;
        let slot = store.joined_pin_index.lock().map_err(|_| denied())?;
        let selected = slot.as_ref().ok_or_else(denied)?;
        let chosen = selected.pending.as_ref().ok_or_else(denied)?;
        let claim = self.pending.as_ref().ok_or_else(denied)?;
        if claim.identity != Some(session_id(&chosen.original.0).map_err(io)?) {
            return Err(denied());
        }
        let (recovered, _) = chosen._pending.pending_data();
        let request = self.prepared.request_bytes_data();
        if request != recovered.request_bytes_data() {
            return Err(denied());
        }
        selected.require(store)?;
        Ok(request)
    }

    pub(in crate::store) fn finish_welcome(
        &mut self,
        store: &ServerStore,
        delivery: &[u8],
        rng: &mut impl CryptoRngCore,
        negative: Option<&catcoms_rt::RequestCancellation>,
    ) -> Result<(), AppError> {
        let require_current = || {
            if negative.is_some_and(catcoms_rt::RequestCancellation::is_cancelled) {
                Err(denied())
            } else {
                Ok(())
            }
        };
        require_current()?;
        self.request_data(store)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        if self.welcome.is_none() {
            self.welcome = Some(welcome::WarmWelcome::prepare(
                store,
                &self.prepared,
                &key,
                &self.exact,
                delivery,
                rng,
            )?);
        }
        let welcome = self.welcome.as_mut().ok_or_else(denied)?;
        if welcome.delivery.as_slice() != delivery {
            return Err(denied());
        }
        require_current()?;
        welcome.cover_entry(store)?;
        self.request_data(store)?;
        #[cfg(test)]
        BEFORE_WELCOME_SDK.with(|slot| {
            if let Some(run) = slot.take() {
                run();
            }
        });
        require_current()?;
        self.welcome.as_mut().ok_or_else(denied)?.prepare_outcome(
            store,
            &mut self.prepared,
            &key,
            rng,
            negative,
        )?;
        self.request_data(store)?;
        require_current()?;
        self.welcome
            .as_mut()
            .ok_or_else(denied)?
            .cover_outcome(store)?;
        self.request_data(store)?;
        let welcome = self.welcome.as_mut().ok_or_else(denied)?;
        welcome.require_outcome(store)?;
        require_current()?;
        welcome.finish_core()?;
        self.request_data(store)?;
        self.welcome
            .as_ref()
            .ok_or_else(denied)?
            .require_outcome(store)?;
        require_current()
    }

    pub(in crate::store) fn warm_outcome_data(
        &self,
        store: &ServerStore,
    ) -> Result<Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)>, AppError>
    {
        self.request_data(store)?;
        let Some(welcome) = &self.welcome else {
            return Ok(None);
        };
        let data = welcome.outcome_data();
        if data.is_some() {
            welcome.require_outcome(store)?;
        }
        Ok(data)
    }

    pub(in crate::store) fn copy_selected_governing_proof(
        &self,
        store: &ServerStore,
        headers: (OwnerHeaderId, OwnerHeaderId),
        output: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        self.request_data(store)?;
        let welcome = self.welcome.as_ref().ok_or_else(denied)?;
        welcome.require_outcome(store)?;
        welcome.copy_selected_governing_proof(headers, output)?;
        self.request_data(store)?;
        welcome.require_outcome(store)
    }

    pub(in crate::store) fn attach_warm_node<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &mut self,
        store: &ServerStore,
        inputs: &mut Option<JoinedRuntimeInputs<T, R>>,
        output: &mut Option<catcoms_sync::ClosedV2JoinedNode<T, R>>,
    ) -> Result<(), AppError> {
        self.request_data(store)?;
        let welcome = self.welcome.as_ref().ok_or_else(denied)?;
        welcome.require_outcome(store)?;
        if output.is_none() {
            *output = Some(welcome.attach_node(&mut self.prepared, inputs)?);
        }
        self.request_data(store)?;
        welcome.require_outcome(store)
    }
}

#[cfg(test)]
std::thread_local! {
    pub(in crate::store) static BEFORE_WELCOME_SDK: std::cell::Cell<Option<Box<dyn FnOnce()>>> = const { std::cell::Cell::new(None) };
}
