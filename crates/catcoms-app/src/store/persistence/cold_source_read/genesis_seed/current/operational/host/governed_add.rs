//! Payments for the original native governed Add candidate. The candidate
//! keeps this debit for as long as its SDK graph or pending output owns it.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::v2_text::{PreparedGovernedAddFence, PreparedV2TextParentCapture};
use crate::store::ServerStore;
use catcoms_mls::persist::text_current_add::{
    TextCurrentAddAdmission, TextCurrentAddLimits, TextCurrentAddReservation,
};
use catcoms_sync::{V2JoinRequestPacketData, MAX_V2_JOIN_REQUEST_PACKET_BYTES};

pub(super) struct PendingGovernedAdd {
    // Physical write handles drain before the associated selected capture.
    pub(super) fence: PreparedGovernedAddFence,
    pub(super) parent: PreparedV2TextParentCapture,
    pub(super) sdk: Option<TextCurrentAddAdmission>,
    request: Vec<u8>,
    fee: CatalogReadReservation,
}

impl PendingGovernedAdd {
    pub(super) fn prepare(
        store: &ServerStore,
        owner: &super::super::owner::V2TextOwnerRef<'_>,
        parent: PreparedV2TextParentCapture,
        packet: &V2JoinRequestPacketData<'_>,
        rng: &mut dyn catcoms_rt::CryptoRngCore,
    ) -> Result<Self, AppError> {
        let frame = packet.packet_bytes_data();
        let selected = parent.selected_data().ok_or_else(refused)?;
        if frame.is_empty() || frame.len() > MAX_V2_JOIN_REQUEST_PACKET_BYTES {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let bytes = 4 * std::mem::size_of::<Self>() + frame.len() + 4096;
        let fee = pool.try_reserve(bytes)?;
        fee.validate(pool, bytes)?;
        let sdk = prepare_sdk_admission(store)?;
        let fence =
            PreparedGovernedAddFence::prepare(store, owner.source(), selected, packet, rng)?;
        Ok(Self {
            fence,
            parent,
            sdk: Some(sdk),
            request: frame.to_vec(),
            fee,
        })
    }

    pub(super) fn require_request(
        &self,
        store: &ServerStore,
        packet: &V2JoinRequestPacketData<'_>,
    ) -> Result<(), AppError> {
        if self.request != packet.packet_bytes_data() {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.fee.validate(
            pool,
            4 * std::mem::size_of::<Self>() + self.request.len() + 4096,
        )
    }
}

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Key use follows exact Outcome/Attempt coverage under the actual Current
    /// and original native source. The Sync loan caches the result in place.
    pub(super) fn sign_governed_header(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.reserve_governed_add(layout, packet, routing_plan, keys, rng, negative)?;
        let store = layout.checked_store()?;
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let resources = p.parent.resources().clone();
        if self.owner.cached_governed_header_raw(store)?.is_none() {
            let owner = self.owner.borrow_current()?;
            p.fence.prepare_header_attempt(store, owner.source(), rng)?;
            let (owner, mut primitive) = self.owner.borrow_governed_header_parts()?;
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                require_live()?;
                p.fence.cover_header_attempt(&guard, selected)?;
                require_live()?;
                primitive.sign_native()?;
                require_live()
            })?;
        }

        let raw = self
            .owner
            .cached_governed_header_raw(store)?
            .ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        p.fence
            .prepare_raw_header(store, owner.source(), &outcome, &raw, rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            p.fence.cover_raw_header(&guard, selected)?;
            require_live()
        })?;
        require_live()
    }

    /// The same original runtime retains the request, paid SDK input, Entry,
    /// SDK Cell and outcome. Any SDK work follows the original durable Entry.
    /// The caller still owns the remaining signing and selection transaction.
    pub(super) fn execute_governed_add_sdk(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        cuts: [catcoms_rt::NativeChatPreparedKeyCut; 3],
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.reserve_governed_add(layout, packet, routing_plan, keys, rng, negative)?;
        self.require_member_quiescent_for_add(layout)?;
        let store = layout.checked_store()?;
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        let replay = self
            .member
            .as_ref()
            .ok_or_else(refused)?
            .governed_add_replay()?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        p.require_request(store, packet)?;
        require_live()?;
        self.owner.prepare_governed_node(store)?;
        self.owner.prepare_governed_sdk(
            store,
            packet.founder_add_data(),
            keys.original.clone(),
            &mut p.sdk,
            cuts,
        )?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let resources = p.parent.resources().clone();
        let owner = self.owner.borrow_current()?;
        p.fence.prepare_sdk_entry(store, owner.source(), rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            require_live()?;
            p.fence.cover_sdk_entry(&guard, selected)?;
            require_live()
        })?;
        self.owner
            .run_governed_sdk(store, packet.founder_add_data(), replay)?;
        let owner = self.owner.borrow_current()?;
        if !p.fence.has_prepared_outcome_data() {
            let outcome = self.owner.governed_outcome_data(store)?;
            p.fence
                .prepare_outcome(store, owner.source(), selected, packet, &outcome, rng)?;
        }
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            p.fence.cover_outcome(&guard, selected)?;
            require_live()
        })?;
        require_live()
    }

    pub(in crate::store) fn retire_add_for_closed_ui(
        &mut self,
        closed: &[Option<&catcoms_rt::NativeChatKeyDrain>],
    ) -> Result<(), AppError> {
        self.owner.retire_add_for_closed_ui(closed)
    }

    /// Whole native Source retirement has exclusive custody of this idle host.
    /// It must retire existing Add Ends before waiting for drained keys.
    pub(in crate::store) fn discard_add_for_closed_source(
        &mut self,
        closed: &catcoms_rt::NativeChatKeyDrain,
    ) -> Result<(), AppError> {
        self.owner.discard_add_for_closed_source(closed)
    }
}

struct GovernedAddSdkReservation {
    _debit: CatalogReadReservation,
}

impl TextCurrentAddReservation for GovernedAddSdkReservation {
    fn limits(&self) -> TextCurrentAddLimits {
        TextCurrentAddLimits::governed_text_v2()
    }
}

impl GovernedAddSdkReservation {
    fn required_bytes() -> Result<usize, AppError> {
        TextCurrentAddLimits::governed_text_v2()
            .buffer_reservation_bytes()
            .and_then(|buffers| {
                buffers.checked_add(TextCurrentAddAdmission::metadata_layout_bytes::<Self>()?)
            })
            .ok_or_else(refused)
    }

    fn prepare(pool: &CriticalCatalogReadPool) -> Result<TextCurrentAddAdmission, AppError> {
        let bytes = Self::required_bytes()?;
        let debit = pool.try_reserve(bytes)?;
        debit.validate(pool, bytes)?;
        TextCurrentAddAdmission::new(Self { _debit: debit }).map_err(|_| refused())
    }
}

/// Must run outside all writers, before the planned reservation can commit.
/// It acquires candidate credit from this Store's actual mounted account.
pub(super) fn prepare_sdk_admission(
    store: &ServerStore,
) -> Result<TextCurrentAddAdmission, AppError> {
    let policy = store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?;
    GovernedAddSdkReservation::prepare(&policy.pending.pool)
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
