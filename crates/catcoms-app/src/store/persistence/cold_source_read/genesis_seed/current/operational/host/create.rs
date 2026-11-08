//! Actual original-owner preparation BEFORE A writes. Kept separate from an accepted
//! message: a complete in-memory packet is not a covering save or publication receipt.
//!
//! Root owns every original slot across errors. It drains already-admitted finite reads
//! and detaches already-materialized DATA before taking another live Source/Current loan.
//! Clear binding is the closed Factory role; a structural token miss is insufficient.
use super::super::owner::V2TextOwnerRef;
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextWorkspaceRead, BoundV2TextCreatePredecessor, PreparedV2TextWorkspaceReadCapture,
};
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextCreateLane, V2TextCreateLane,
};
use crate::store::persistence::v2_text::{
    PreparedV2RetainData, PreparedV2RetainOriginalCustody, PreparedV2TextWorkspaceCapture,
};
use crate::store::ServerStore;
use catcoms_rt::{CryptoRngCore, MeshTransport};
use catcoms_sync::owner_v2::{
    DetachedV2CreateData, PreparedV2CreateData, V2CreateRequest, CREATE_DATA_BUFFER_BYTES,
};
mod next;
pub(super) use next::PendingCompletedCreate;
mod retain;
use retain::PendingRetain;
mod append;
pub(super) const V2_TEXT_RESIDENT_DOMAIN_LAYOUT_BYTES: usize =
    append::V2_TEXT_RESIDENT_DOMAIN_LAYOUT_BYTES;
pub(in crate::store) use append::{
    OriginalV2TextResidentObservation, OriginalV2UnsignedPlanObservation,
    PreparedV2NextUnsignedAppend, PreparedV2TextResidentProjection,
    PreparedV2UnsignedOriginalCustody, V2AdvancedAppendBasis, V2TextResidentParentData,
};

/// No Source/node/key/Store/session is cached here. Same original request/packet and
/// payments remain owned even if a final callback fails after creating a real result.
pub(super) struct PendingCreate {
    producer: Option<PreparedV2RetainData>,
    raw_preparation: Option<PreparedV2TextLane>,
    raw_lane: Option<V2TextLane>,
    lane_preparation: Option<PreparedV2TextCreateLane>,
    lane: Option<V2TextCreateLane>,
    workspace: Option<PreparedV2TextWorkspaceCapture>,
    admitted_workspace: Option<AdmittedV2TextWorkspaceRead>,
    predecessor: Option<BoundV2TextCreatePredecessor>,
    sdk: Option<PreparedV2CreateData>,
    detached: Option<DetachedV2CreateData>,
    // Once complete, actual SDK credit moves WITH the packet, never independently.
    custody: Option<PreparedV2RetainOriginalCustody>,
    retained: Option<PendingRetain>,
    // LAST: covers actual SDK originals in producer, sdk AND detached, independently of
    // transaction metadata/work, original source/SDK, selected-read and query payments.
    sdk_fee: Option<CatalogReadReservation>,
}
// Include the concrete A cell as well as pre-transfer originals. The SAME bound is a
// named term in Physical's 1.25MiB transaction metadata invoice, not uncharged stack/Box storage.
const _: () = assert!(
    std::mem::size_of::<PendingCreate>()
        <= crate::store::persistence::v2_text::V2_TEXT_RETAIN_ROOT_PHASE_BYTES
);

impl PendingCreate {
    fn producer_data(&self) -> Result<&PreparedV2RetainData, AppError> {
        match (&self.producer, &self.custody) {
            (Some(producer), None) => Ok(producer),
            (None, Some(custody)) => Ok(custody.producer()),
            _ => Err(refused()),
        }
    }
    #[cfg(all(test, windows))]
    fn complete_digest_for_test(&self) -> Option<[u8; 32]> {
        let producer = self.producer_data().ok()?;
        if !producer.has_complete_data() || self.sdk.is_some() || self.detached.is_some() {
            return None;
        }
        let mut hash = blake3::Hasher::new();
        for (role, id, frame) in producer.artifact_frames_data() {
            hash.update(&[role as u8]);
            hash.update(&id);
            hash.update(&(frame.len() as u64).to_be_bytes());
            hash.update(frame);
        }
        Some(*hash.finalize().as_bytes())
    }
    pub(super) fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lookup: &PendingTokenLookup,
        request: &V2CreateRequest<'_>,
    ) -> Result<Self, AppError> {
        let producer = PreparedV2RetainData::prepare(
            store,
            owner.source(),
            lookup.selected_data().ok_or_else(refused)?,
            V2CreateRequest {
                token: request.token,
                channel: request.channel,
                text: request.text,
            },
        )?;
        Ok(Self {
            producer: Some(producer),
            raw_preparation: None,
            raw_lane: None,
            lane_preparation: None,
            lane: None,
            workspace: None,
            admitted_workspace: None,
            predecessor: None,
            sdk: None,
            detached: None,
            custody: None,
            retained: None,
            sdk_fee: None,
        })
    }

    /// Negative DATA dispatch only. A failed handoff restores the SAME unique SDK value;
    /// no signer, exporter, clock or RNG is called to discover/cache completed originals.
    pub(super) fn drain_originals(&mut self) -> Result<(), AppError> {
        if let Some(work) = self.admitted_workspace.take() {
            self.workspace
                .as_mut()
                .ok_or_else(refused)?
                .run_read(work)?;
        }
        if self.detached.is_none() {
            if let Some(prepared) = self.sdk.take() {
                match prepared.into_handoff_data() {
                    Ok(packet) => self.detached = Some(packet),
                    Err((original, _)) => self.sdk = Some(original),
                }
            }
        }
        if self.predecessor.is_some() {
            // Bound was cached only after the actual read End drained. Release inert
            // source/Ready copies even when a late Scope failure now forbids another
            // positive borrow. No IO, key, new admission or scope validation occurs.
            drop(self.workspace.take());
            drop(self.raw_preparation.take());
            drop(self.lane_preparation.take());
        }
        // Pure owning handoff is also possible after a late Scope refusal: it neither
        // repeats the signer nor acquires positive authority. Do it before any retry
        // borrows Current, so complete originals never escape their SDK payment.
        if self.sdk.is_none()
            && self.detached.is_none()
            && (self.custody.is_some()
                || self
                    .producer
                    .as_ref()
                    .is_some_and(|data| data.has_complete_data()))
        {
            PreparedV2RetainOriginalCustody::take_complete_into(
                &mut self.producer,
                &mut self.sdk_fee,
                &mut self.custody,
            )?;
        }
        Ok(())
    }

    pub(super) fn prepare_predecessor(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        observer: &V2TextSourceNegativeObserver,
        lookup: &PendingTokenLookup,
        request: &V2CreateRequest<'_>,
    ) -> Result<(), AppError> {
        self.drain_originals()?;
        let producer = self.producer_data()?;
        producer.require_request_data(request)?;
        let resources = producer.resources().clone();
        let original = producer.observe_original();
        if self.raw_preparation.is_none() && self.lane.is_none() {
            self.raw_preparation = Some(PreparedV2TextLane::prepare(store, owner, observer)?);
        }
        if self.raw_lane.is_none() && self.lane.is_none() {
            let prepared = self.raw_preparation.as_mut().ok_or_else(refused)?;
            let result = store.with_v2_text_current(owner, |admission| {
                store
                    .lock_v2_text_owner_namespace(admission)?
                    .claim_text_lane_into(prepared, &mut self.raw_lane)
            });
            prepared.drain_unused_outside_writers();
            result?;
        }
        if self.lane_preparation.is_none() && self.lane.is_none() {
            self.lane_preparation = Some(PreparedV2TextCreateLane::prepare(
                store,
                owner,
                self.raw_lane.as_ref().ok_or_else(refused)?,
            )?);
        }
        if self.lane.is_none() {
            store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.convert_text_create_lane_into(
                    self.lane_preparation.as_mut().ok_or_else(refused)?,
                    &mut self.raw_lane,
                    &mut self.lane,
                )
            })?;
        }
        let lane = self.lane.as_ref().ok_or_else(refused)?;
        let query = lookup.query_data().ok_or_else(refused)?;
        if self.predecessor.is_none() {
            if self.workspace.is_none() {
                self.workspace = Some(PreparedV2TextWorkspaceCapture::prepare(
                    store,
                    owner.source(),
                    self.producer_data()?,
                    lookup.selected_data().ok_or_else(refused)?,
                    query,
                )?);
            }
            let workspace = self.workspace.as_mut().ok_or_else(refused)?;
            if !workspace.has_ready_data() {
                let mut capture = Some(PreparedV2TextWorkspaceReadCapture::prepare(
                    store,
                    owner.source(),
                    workspace,
                )?);
                let admission = store.with_v2_text_current(owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard.admit_text_workspace_read_into(
                        lane,
                        workspace,
                        query,
                        &mut capture,
                        &mut self.admitted_workspace,
                    )
                });
                drop(capture.take());
                if let Some(work) = self.admitted_workspace.take() {
                    workspace.run_read(work)?;
                }
                admission?;
            }
            store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_create_predecessor_into(
                    lane,
                    workspace.ready_clear()?,
                    query,
                    &original,
                    &mut self.predecessor,
                )
            })?;
        }
        // The completed physical read/actual End already drained. Once Bound is cached,
        // large workspace/Ready tails can drain OUTSIDE all writers before SDK buffers.
        drop(self.workspace.take());
        drop(self.raw_preparation.take());
        drop(self.lane_preparation.take());
        let predecessor = self.predecessor.as_ref().ok_or_else(refused)?;
        store.with_v2_text_current(owner, |admission| {
            let mut namespace = store.lock_v2_text_owner_namespace(admission)?;
            lane.require_registered(&mut namespace)?;
            predecessor.require_same_lane_original(lane, &original)
        })
    }

    pub(super) fn materialize<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        owner: &mut V2TextOwnedCurrent<T, R>,
        store: &ServerStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.drain_originals()?;
        if let Some(custody) = self.custody.as_ref() {
            custody.validate_accounts(store)?;
            custody.producer().require_request_data(&request)?;
            let source = owner.borrow_current()?;
            return custody
                .producer()
                .validate_resources_original(store, source.source());
        }
        let producer = self.producer.as_mut().ok_or_else(refused)?;
        producer.require_request_data(&request)?;
        self.predecessor
            .as_ref()
            .ok_or_else(refused)?
            .require_same_lane_original(
                self.lane.as_ref().ok_or_else(refused)?,
                &producer.observe_original(),
            )?;
        if self.sdk_fee.is_none() {
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            self.sdk_fee = Some(pool.try_reserve(CREATE_DATA_BUFFER_BYTES)?);
        }
        self.sdk_fee.as_ref().ok_or_else(refused)?.validate(
            &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool,
            CREATE_DATA_BUFFER_BYTES,
        )?;
        {
            // The generic whole owner must be this producer's ORIGINAL read/SDK, not
            // merely an equal realm/signer from a replacement owner. End this SourceRef
            // loan before the mutable retained-RNG call, after every account wait.
            let source = owner.borrow_current()?;
            producer.validate_resources_original(store, source.source())?;
        }
        if !producer.has_packet_data() && self.detached.is_none() {
            // No SourceRef or AD/pins/canonical/Root guard survives this mutable original
            // node call. Its caller-owned slot caches partial/complete output on refusal.
            let result = owner.prepare_create_data(store, request, &mut self.sdk);
            self.drain_originals()?;
            result.map_err(|_| refused())?;
        }
        let result = {
            let source = owner.borrow_current()?;
            let producer = self.producer.as_mut().ok_or_else(refused)?;
            if !producer.has_complete_data() {
                producer.attach_packet(store, source.source(), &mut self.detached)?;
                producer.prepare_pages(store, source.source(), rng)
            } else {
                Ok(())
            }
        };
        self.drain_originals()?;
        result
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Negative test observation of cached complete DATA, never a posting receipt.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn prepared_create_digest_for_test(&self) -> Option<[u8; 32]> {
        self.request_preparation
            .as_ref()?
            .complete_digest_for_test()
    }
    /// Prepare one original packet under this host's actual clean-workspace predecessor.
    /// No A/I/S/B save, acceptance, publication or native success follows from returning
    /// Ok. The subsequent transaction consumes the SAME retained originals and payments.
    pub(in crate::store) fn retry_prepare_create_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if let Some(pending) = self.request_preparation.as_mut() {
            pending.drain_originals()?;
        }
        // A completed resident requires its distinct WC-backed next-request path.
        // It cannot reenter the first-only Clear/genesis producer after cleanup.
        if self.resident.is_some() || self.next_request_preparation.is_some() {
            return Err(refused());
        }
        self.retry_token_lookup(layout, request.token)?;
        let store = layout.checked_store()?;
        {
            let owner = self.owner.borrow_current()?;
            let lookup = self.lookup.as_ref().ok_or_else(refused)?;
            if self.request_preparation.is_none() {
                self.request_preparation =
                    Some(PendingCreate::prepare(store, &owner, lookup, &request)?);
            }
            self.request_preparation
                .as_mut()
                .ok_or_else(refused)?
                .prepare_predecessor(
                    store,
                    &owner,
                    self.observer.as_ref().ok_or_else(refused)?,
                    lookup,
                    &request,
                )?;
        }
        // The complete owner loan/Current callback above has ended before borrowing its
        // retained RNG mutably. All fallible SDK outcomes stay in the same outside phase.
        self.request_preparation
            .as_mut()
            .ok_or_else(refused)?
            .materialize(&mut self.owner, store, request, rng)
    }
}
