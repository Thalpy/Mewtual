//! Owning pre-A preparation from a genuinely completed resident.
//!
//! The first request's Clear path remains closed after resident handoff. This
//! separate driver reads the actual completed WC, binds its own new lane, then
//! prepares one original Create packet. It neither rebuilds genesis nor borrows
//! the resident's mathematical owner into a caller-controlled SDK/model slot.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextCompletedWorkspaceRead, BoundV2TextCompletedCreatePredecessor,
    PreparedV2TextCompletedWorkspaceReadCapture,
};
use crate::store::persistence::v2_text::PreparedV2TextCompletedWorkspaceCapture;

/// Original pre-packet fields and their payments share the existing bounded root
/// phase. Completed-WC facts additionally own their separate parent metadata fee;
/// no old B, old reader, SDK or model payment is substituted for that obligation.
pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational::host)
struct PendingCompletedCreate
{
    base: PendingCreate,
    workspace: Option<PreparedV2TextCompletedWorkspaceCapture>,
    admitted: Option<AdmittedV2TextCompletedWorkspaceRead>,
    predecessor: Option<BoundV2TextCompletedCreatePredecessor>,
}

const _: () = assert!(
    std::mem::size_of::<PendingCompletedCreate>()
        <= crate::store::persistence::v2_text::V2_TEXT_RETAIN_ROOT_PHASE_BYTES
);

impl PendingCompletedCreate {
    fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lookup: &PendingTokenLookup,
        request: &V2CreateRequest<'_>,
        resident: &PreparedV2TextResidentProjection,
    ) -> Result<Self, AppError> {
        // This first acquires the new A1.25MiB/work16MiB obligations before any
        // producer or enclosing Root allocation. No SDK action exists yet.
        let base = PendingCreate::prepare(store, owner, lookup, request)?;
        let workspace = PreparedV2TextCompletedWorkspaceCapture::prepare(
            store,
            owner.source(),
            base.producer_data()?,
            lookup.selected_data().ok_or_else(refused)?,
            lookup.query_data().ok_or_else(refused)?,
            resident,
        )?;
        Ok(Self {
            base,
            workspace: Some(workspace),
            admitted: None,
            predecessor: None,
        })
    }

    /// Consume already admitted finite reads before any live Source/Current loan.
    /// Ready/Bound outputs remain in their actual slots after an uncertain reply.
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current::operational::host) fn drain_originals(
        &mut self,
    ) -> Result<(), AppError> {
        if let Some(admitted) = self.admitted.take() {
            self.workspace
                .as_mut()
                .ok_or_else(refused)?
                .run_read(admitted)?;
        }
        self.base.drain_originals()
    }

    fn prepare_predecessor(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        observer: &V2TextSourceNegativeObserver,
        lookup: &PendingTokenLookup,
        request: &V2CreateRequest<'_>,
        resident: &PreparedV2TextResidentProjection,
    ) -> Result<(), AppError> {
        self.drain_originals()?;
        let producer = self.base.producer_data()?;
        producer.require_request_data(request)?;
        let resources = producer.resources().clone();
        let original = producer.observe_original();
        resident.validate_completed_parent_data(store)?;
        if self.base.raw_preparation.is_none() && self.base.lane.is_none() {
            self.base.raw_preparation = Some(PreparedV2TextLane::prepare(store, owner, observer)?);
        }
        if self.base.raw_lane.is_none() && self.base.lane.is_none() {
            let prepared = self.base.raw_preparation.as_mut().ok_or_else(refused)?;
            let result = store.with_v2_text_current(owner, |admission| {
                store
                    .lock_v2_text_owner_namespace(admission)?
                    .claim_text_lane_into(prepared, &mut self.base.raw_lane)
            });
            prepared.drain_unused_outside_writers();
            result?;
        }
        if self.base.lane_preparation.is_none() && self.base.lane.is_none() {
            self.base.lane_preparation = Some(PreparedV2TextCreateLane::prepare(
                store,
                owner,
                self.base.raw_lane.as_ref().ok_or_else(refused)?,
            )?);
        }
        if self.base.lane.is_none() {
            store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.convert_text_create_lane_into(
                    self.base.lane_preparation.as_mut().ok_or_else(refused)?,
                    &mut self.base.raw_lane,
                    &mut self.base.lane,
                )
            })?;
        }
        let lane = self.base.lane.as_ref().ok_or_else(refused)?;
        let query = lookup.query_data().ok_or_else(refused)?;
        if self.predecessor.is_none() {
            let workspace = self.workspace.as_mut().ok_or_else(refused)?;
            if !workspace.has_ready_data() {
                let mut capture = Some(PreparedV2TextCompletedWorkspaceReadCapture::prepare(
                    store,
                    owner.source(),
                    workspace,
                )?);
                let admission = store.with_v2_text_current(owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard.admit_text_completed_workspace_read_into(
                        lane,
                        workspace,
                        query,
                        resident,
                        &mut capture,
                        &mut self.admitted,
                    )
                });
                // Both unused capture disposal and actual admitted read work run
                // outside every writer, even if the final admission reply failed.
                drop(capture.take());
                if let Some(admitted) = self.admitted.take() {
                    workspace.run_read(admitted)?;
                }
                admission?;
            }
            store.with_v2_text_current(owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_completed_create_predecessor_into(
                    lane,
                    workspace.ready_completed()?,
                    query,
                    resident,
                    &original,
                    &mut self.predecessor,
                )
            })?;
        }
        drop(self.base.raw_preparation.take());
        drop(self.base.lane_preparation.take());
        self.predecessor
            .as_ref()
            .ok_or_else(refused)?
            .require_same_lane_original(lane, &original)?;
        self.predecessor
            .as_ref()
            .ok_or_else(refused)?
            .require_same_resident(&resident.observe_original())?;
        self.predecessor
            .as_ref()
            .ok_or_else(refused)?
            .validate_negative()
    }

    fn materialize<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        owner: &mut V2TextOwnedCurrent<T, R>,
        store: &ServerStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
        resident: &PreparedV2TextResidentProjection,
    ) -> Result<(), AppError> {
        self.drain_originals()?;
        self.base.producer_data()?.require_request_data(&request)?;
        self.predecessor
            .as_ref()
            .ok_or_else(refused)?
            .validate_negative()?;
        resident.validate_completed_parent_data(store)?;
        if let Some(custody) = self.base.custody.as_ref() {
            custody.validate_accounts(store)?;
            let source = owner.borrow_current()?;
            return custody
                .producer()
                .validate_resources_original(store, source.source());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        if self.base.sdk_fee.is_none() {
            self.base.sdk_fee = Some(pool.try_reserve(CREATE_DATA_BUFFER_BYTES)?);
        }
        self.base
            .sdk_fee
            .as_ref()
            .ok_or_else(refused)?
            .validate(pool, CREATE_DATA_BUFFER_BYTES)?;
        {
            let source = owner.borrow_current()?;
            self.base
                .producer_data()?
                .validate_resources_original(store, source.source())?;
        }
        if !self.base.producer_data()?.has_packet_data() && self.base.detached.is_none() {
            // There is no native/SourceRef loan across this mutable original-node
            // call. Its original outside slot preserves partial and complete SDK data.
            let result = owner.prepare_create_data(store, request, &mut self.base.sdk);
            self.drain_originals()?;
            result.map_err(|_| refused())?;
        }
        let result = {
            let source = owner.borrow_current()?;
            let producer = self.base.producer.as_mut().ok_or_else(refused)?;
            if !producer.has_complete_data() {
                producer.attach_packet(store, source.source(), &mut self.base.detached)?;
                producer.prepare_pages(store, source.source(), rng)
            } else {
                Ok(())
            }
        };
        self.drain_originals()?;
        result?;
        resident.validate_completed_parent_data(store)
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn completed_create_digest_for_test(&self) -> Option<[u8; 32]> {
        self.next_request_preparation
            .as_ref()?
            .base
            .complete_digest_for_test()
    }

    /// Prepare one original next packet through the distinct completed-WC role.
    /// This is pre-A only; selection, unsigned append, signing and publication need
    /// their actual subsequent owners. It cannot restart a first-only Clear path.
    pub(in crate::store) fn retry_prepare_completed_create_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if let Some(next) = self.next_request_preparation.as_mut() {
            next.drain_originals()?;
        }
        // An ordinary-member lane excludes every founder Create entry on this host, not only
        // through the first-request token lookup: one token never authors in two namespaces.
        if self.request_preparation.is_some() || self.selected.is_some() || self.member.is_some() {
            return Err(refused());
        }
        let resident = self.resident.as_ref().ok_or_else(refused)?;
        let store = layout.checked_store()?;
        resident.validate_completed_parent_data(store)?;
        {
            let owner = self.owner.borrow_current()?;
            if self.lookup.is_none() {
                self.lookup = Some(PendingTokenLookup::prepare(store, &owner, request.token)?);
            }
            let lookup = self.lookup.as_mut().ok_or_else(refused)?;
            lookup.retry(store, &owner, request.token)?;
            if self.next_request_preparation.is_none() {
                self.next_request_preparation = Some(Box::new(PendingCompletedCreate::prepare(
                    store, &owner, lookup, &request, resident,
                )?));
            }
            self.next_request_preparation
                .as_mut()
                .ok_or_else(refused)?
                .prepare_predecessor(
                    store,
                    &owner,
                    self.observer.as_ref().ok_or_else(refused)?,
                    lookup,
                    &request,
                    resident,
                )?;
        }
        self.next_request_preparation
            .as_mut()
            .ok_or_else(refused)?
            .materialize(&mut self.owner, store, request, rng, resident)
    }
}
