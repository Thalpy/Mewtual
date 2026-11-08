//! Independently paid unsigned Append preparation for the SAME first retained A.
//!
//! This is deliberately DATA only: a selected I, raw signing
//! admission, verifier advance and publication require their separate actual consumers.
//! No arbitrary current-image constructor or raw node/key is exposed. The initial math
//! derives from the whole owner's genuine genesis; operational cold replay is distinct.
use super::super::projection::{V2TextProjectionDomain, V2TextProjectionNegative};
use super::*;
use crate::store::persistence::v2_text::{
    OriginalV2TextRetainTransactionObservation, PreparedV2TextRetainTransaction,
    SelectedV2TextHead, V2TextReadResources,
};
use catcoms_replication::chat::owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE;
use catcoms_replication::chat::V2FounderPrefixVerifier;
use catcoms_sync::owner_v2::{
    PreparedV2AppendData, V2AppendPlanObservation, V2UnsignedAppendDataView,
    APPEND_DATA_BUFFER_BYTES,
};
mod custody;
pub(super) const V2_TEXT_RESIDENT_DOMAIN_LAYOUT_BYTES: usize =
    custody::V2_TEXT_RESIDENT_DOMAIN_LAYOUT_BYTES;
pub(in crate::store) use custody::{
    OriginalV2TextResidentObservation, OriginalV2UnsignedPlanObservation,
    PreparedV2NextUnsignedAppend, PreparedV2TextResidentProjection,
    PreparedV2UnsignedOriginalCustody, V2AdvancedAppendBasis, V2TextResidentParentData,
};
#[cfg(all(test, windows))]
thread_local! {
    static NEXT_UNSIGNED_FEE_NOTICES: std::cell::RefCell<Option<(
        Arc<std::sync::atomic::AtomicBool>, Arc<std::sync::atomic::AtomicBool>
    )>> = const { std::cell::RefCell::new(None) };
}

/// Private grouping formed only by the closed pinned-genesis producer below.
/// The SAME already-paid Box moves to flat residence after actual WC/lane finish;
/// no mutable model/SDK getter, caller-model constructor or re-genesis is provided.
struct ResidentVerifierOwner {
    model: V2FounderPrefixVerifier,
    identity: Arc<()>,
    domain: Arc<V2TextProjectionDomain>,
    // LAST: the original model1MiB outlives its SAME mathematical payload.
    model_fee: CatalogReadReservation,
}

/// One unsigned SDK slot and its original mathematical parent. The owning handoff for
/// later I/S/B must move the WHOLE cell and BOTH payments; no mutable SDK/model extractor.
pub(super) struct FirstUnsignedAppendData {
    // ONE next model is already included in the same paid Core source+candidate1M.
    // No take/advance exists before a genuine SavedB and explicit advanced phase.
    candidate: Option<catcoms_replication::chat::V2VerifiedAppendCandidate>,
    prepared: Option<PreparedV2AppendData>,
    original_plan: Option<V2AppendPlanObservation>,
    // None on the initial pinned-genesis path; next-A custody binds the exact
    // completed-WC residence without retaining its prior transaction/read chain.
    resident_basis: Option<OriginalV2TextResidentObservation>,
    retain: OriginalV2TextRetainTransactionObservation,
    resources: V2TextReadResources,
    // Each actual fee follows its own payload; the model owner is LAST so the
    // candidate drops before model1MiB even if the entire transaction is refused.
    sdk_fee: CatalogReadReservation,
    resident_owner: Option<Box<ResidentVerifierOwner>>,
}

impl FirstUnsignedAppendData {
    /// Private owning-family access only. Transfer empties the explicit slot, so
    /// later positive calls refuse rather than constructing a replacement model.
    fn resident_owner_data(&self) -> Result<&ResidentVerifierOwner, AppError> {
        self.resident_owner.as_deref().ok_or_else(refused)
    }
    #[cfg(all(test, windows))]
    fn same_original_for_test(&self) -> bool {
        self.original_plan
            .as_ref()
            .zip(self.prepared.as_ref())
            .is_some_and(|(original, prepared)| original.matches_original_data(prepared))
    }
    /// Cache all completed output before any late Scope failure. This function creates
    /// no acceptance proof and therefore cannot be used as an I or signer constructor.
    pub(super) fn prepare_into<T: MeshTransport, R: CryptoRngCore>(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        selected_genesis: &SelectedV2TextHead,
        retained: &PreparedV2TextRetainTransaction,
        domain: &Arc<V2TextProjectionDomain>,
        slot: &mut Option<Self>,
    ) -> Result<(), AppError> {
        let source = owner.borrow_current()?;
        retained
            .producer_data()
            .validate_resources_original(store, source.source())?;
        // The genuine A cell preserves the public parent and records this original read.
        // This comparison supplies DATA association, never current selected-A authority.
        retained
            .producer_data()
            .require_selected_data(selected_genesis)?;
        if slot.is_none() {
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let model_fee = pool.try_reserve(REPLAY_MATH_PRIVATE_INVOICE)?;
            let sdk_fee = pool.try_reserve(APPEND_DATA_BUFFER_BYTES)?;
            #[cfg(all(test, windows))]
            let (model_fee, sdk_fee) = {
                let mut model_fee = model_fee;
                let mut sdk_fee = sdk_fee;
                NEXT_UNSIGNED_FEE_NOTICES.with(|slot| {
                    if let Some((model, sdk)) = slot.borrow_mut().take() {
                        model_fee.observe_drop_for_test(model);
                        sdk_fee.observe_drop_for_test(sdk);
                    }
                });
                (model_fee, sdk_fee)
            };
            model_fee.validate(pool, REPLAY_MATH_PRIVATE_INVOICE)?;
            sdk_fee.validate(pool, APPEND_DATA_BUFFER_BYTES)?;
            domain.validate_accounts(store)?;
            retained
                .producer_data()
                .validate_resources_original(store, source.source())?;
            let mut anchor = None;
            let mut bound = None;
            let resources = retained.resources().clone();
            store.with_v2_text_current(&source, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                domain.require_current_source(admission)?;
                anchor =
                    Some(selected_genesis.founder_anchor_from_retained_data(&guard, retained)?);
                domain.bind_anchor_into(&guard, &mut anchor, &mut bound)
            })?;
            let genesis = source.source().data();
            // Full Core crypto/math runs OUTSIDE Current writers. The SAME paid live
            // verifier now owns this parent through I/S/B, rather than replay DATA that
            // would require constructing a replacement verifier after the key cut.
            let model = domain.verifier_from_genesis(
                bound.as_ref().ok_or_else(refused)?,
                genesis.profile,
                genesis.admission,
                genesis.header,
                genesis.state,
            )?;
            *slot = Some(Self {
                candidate: None,
                resident_basis: None,
                prepared: None,
                original_plan: None,
                retain: retained.observe_original(),
                resources: retained.resources().clone(),
                sdk_fee,
                // BOTH independent payments and domain metadata are already held.
                resident_owner: Some(Box::new(ResidentVerifierOwner {
                    model,
                    identity: Arc::new(()),
                    domain: domain.clone(),
                    model_fee,
                })),
            });
        }
        let current = slot.as_mut().ok_or_else(refused)?;
        if !Arc::ptr_eq(&current.resident_owner_data()?.domain, domain) {
            return Err(refused());
        }
        current
            .retain
            .require_same_original(&retained.observe_original())?;
        current
            .resources
            .require_same_original(retained.resources())?;
        current.validate_accounts(store)?;
        current.resident_owner_data()?.domain.validate_negative()?;
        current
            .resources
            .require_bound_original_negative(store, source.source())?;
        let saved = retained.saved_retained_action()?;
        let resources = current.resources.clone();
        // An actual cached SavedA is historical. Reprove its selected O_A/H_A/T_A after
        // every account wait before any new SDK work; accepted-history lookup is still
        // a separate prerequisite for persisting I and can never be inferred here.
        store.with_v2_text_current(&source, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            saved.validate_selected_under_current_guard(&mut guard)
        })?;
        drop(source);
        if current.prepared.is_none() {
            // No writer survives the SDK's complete mathematical derivation. The SAME
            // caller-owned slot retains partial/completed SDK output even on refusal.
            let result = owner.prepare_create_append_data(
                store,
                current
                    .resident_owner
                    .as_ref()
                    .ok_or_else(refused)?
                    .model
                    .replay_data(),
                retained.producer_data().action_data()?,
                &mut current.prepared,
            );
            result.map_err(|_| refused())?;
        }
        if current.original_plan.is_none() {
            current.original_plan = Some(
                owner
                    .observe_append_plan(
                        store,
                        current
                            .resident_owner
                            .as_ref()
                            .ok_or_else(refused)?
                            .model
                            .replay_data(),
                        current.prepared.as_ref().ok_or_else(refused)?,
                    )
                    .map_err(|_| refused())?,
            );
        }
        let source = owner.borrow_current()?;
        current
            .resources
            .require_bound_original_negative(store, source.source())
    }

    fn validate_accounts(&self, store: &ServerStore) -> Result<(), AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.resident_owner_data()?
            .model_fee
            .validate(pool, REPLAY_MATH_PRIVATE_INVOICE)?;
        self.sdk_fee.validate(pool, APPEND_DATA_BUFFER_BYTES)?;
        self.resident_owner_data()?.domain.validate_accounts(store)
    }

    /// Borrow the exact original SDK bytes for intent encoding OUTSIDE all writers.
    /// Equality with these bytes is necessary but not sufficient for signing admission.
    pub(super) fn borrow_unsigned<'a, T: MeshTransport, R: CryptoRngCore>(
        &'a self,
        store: &ServerStore,
        owner: &'a V2TextOwnedCurrent<T, R>,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<V2UnsignedAppendDataView<'a>, AppError> {
        self.retain
            .require_same_original(&retained.observe_original())?;
        self.resources.require_same_original(retained.resources())?;
        self.validate_accounts(store)?;
        let source = owner.borrow_current()?;
        self.resources
            .require_bound_original_negative(store, source.source())?;
        self.resident_owner_data()?.domain.validate_negative()?;
        owner
            .borrow_unsigned_append(
                store,
                self.resident_owner_data()?.model.replay_data(),
                self.prepared.as_ref().ok_or_else(refused)?,
            )
            .map_err(|_| refused())
    }
}

impl PendingRetain {
    fn retry_unsigned_data<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        selected_genesis: &SelectedV2TextHead,
        domain: &Arc<V2TextProjectionDomain>,
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        self.require_absent_accepted_data()?;
        if let Some(custody) = self.unsigned_custody.as_ref() {
            let source = owner.borrow_current()?;
            return custody.validate_original(store, source.source(), self.transaction_data()?);
        }
        let (retained, unsigned) = self.unsigned_inputs()?;
        FirstUnsignedAppendData::prepare_into(
            store,
            owner,
            selected_genesis,
            retained,
            domain,
            unsigned,
        )
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn retry_prepare_projection_domain(
        &mut self,
        layout: &V2CurrentLayoutStore,
    ) -> Result<(), AppError> {
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        if self.projection.is_none() {
            self.projection = Some(Arc::new(V2TextProjectionDomain::prepare(
                store,
                &owner,
                self.observer.as_ref().ok_or_else(refused)?,
            )?));
        }
        let domain = self.projection.as_ref().ok_or_else(refused)?;
        domain.validate_accounts(store)?;
        store.with_v2_text_current(&owner, |admission| domain.require_current_source(admission))
    }
    /// Move the complete original zero-sign plan into closed I-input custody. This
    /// has no filesystem or key side effect and preserves the SAME SDK through errors.
    pub(in crate::store) fn retry_prepare_unsigned_custody_data(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.retry_prepare_unsigned_create_data(layout, request, rng)?;
        let retained = self
            .request_preparation
            .as_mut()
            .ok_or_else(refused)?
            .retained
            .as_mut()
            .ok_or_else(refused)?;
        let (cell, unsigned, custody) = retained.unsigned_custody_inputs()?;
        PreparedV2UnsignedOriginalCustody::take_complete_into(
            layout.checked_store()?,
            &self.owner,
            cell,
            unsigned,
            custody,
        )
    }
    /// Prepare the exact zero-sign unsigned SDK child after durable A and a fresh
    /// accepted-history miss. Durable I is still required before any raw key call.
    pub(in crate::store) fn retry_prepare_unsigned_create_data(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.retry_accepted_create_lookup(layout, request, rng)?;
        self.retry_prepare_projection_domain(layout)?;
        self.request_preparation
            .as_mut()
            .ok_or_else(refused)?
            .retained
            .as_mut()
            .ok_or_else(refused)?
            .retry_unsigned_data(
                layout.checked_store()?,
                &self.owner,
                self.lookup
                    .as_ref()
                    .ok_or_else(refused)?
                    .selected_data()
                    .ok_or_else(refused)?,
                self.projection.as_ref().ok_or_else(refused)?,
            )
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn unsigned_original_retained_for_test(&self) -> bool {
        self.request_preparation
            .as_ref()
            .and_then(|pending| pending.retained.as_ref())
            .is_some_and(|retained| {
                retained
                    .unsigned
                    .as_ref()
                    .is_some_and(FirstUnsignedAppendData::same_original_for_test)
                    || retained.unsigned_custody.as_ref().is_some_and(|custody| {
                        retained
                            .transaction_data()
                            .is_ok_and(|a| custody.require_original_plan_data(a).is_ok())
                    })
            })
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn prepared_unsigned_digest_for_test(
        &self,
        layout: &V2CurrentLayoutStore,
    ) -> Result<[u8; 32], AppError> {
        let retained = self
            .request_preparation
            .as_ref()
            .ok_or_else(refused)?
            .retained
            .as_ref()
            .ok_or_else(refused)?;
        let view = if let Some(custody) = retained.unsigned_custody.as_ref() {
            custody.unsigned_data(retained.transaction_data()?)?
        } else {
            retained
                .unsigned
                .as_ref()
                .ok_or_else(refused)?
                .borrow_unsigned(
                    layout.checked_store()?,
                    &self.owner,
                    retained.transaction_data()?,
                )?
        };
        let mut hash = blake3::Hasher::new();
        for bytes in [
            view.signing_bytes,
            view.image,
            &view.record.encode(),
            &view.body.encode(),
        ] {
            hash.update(&(bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        Ok(*hash.finalize().as_bytes())
    }
}
