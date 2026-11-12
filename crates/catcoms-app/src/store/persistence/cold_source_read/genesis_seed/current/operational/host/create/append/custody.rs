//! Closed unsigned-plan handoff into the actual I transaction.
//!
//! The original model/SDK and both independent payments move together. Immutable
//! cached bytes let Physical encode I without a node loan, SDK call or mutable owner
//! escape. A plan observation is association DATA only; it is never signing admission.
use super::*;
use crate::store::critical_namespaces::read_pool::CriticalCatalogReadPool;
use crate::store::persistence::v2_text::V2TextTransactionMetadataResources;
use catcoms_replication::chat::owner_bodies::{AcceptanceRecord, AppendBody};
use std::sync::Arc;
use zeroize::Zeroizing;

mod advance;
pub(in crate::store) use advance::V2AdvancedAppendBasis;
mod candidate;
mod resident;
pub(super) use resident::V2_TEXT_RESIDENT_DOMAIN_LAYOUT_BYTES;
pub(in crate::store) use resident::{
    OriginalV2TextResidentObservation, PreparedV2NextUnsignedAppend,
    PreparedV2TextResidentProjection, V2TextResidentParentData,
};
#[cfg(all(test, windows))]
mod domain_tests;
mod sign;

// This is part of A's new metadata allowance, additional to resident math1M/SDK2M.
// Include both pre-transfer/cache layouts, typed codec allocation and observation tails.
pub(in crate::store) const V2_TEXT_UNSIGNED_CUSTODY_METADATA_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2UnsignedOriginalCustody>()
    + 4 * std::mem::size_of::<UnsignedPlanIdentity>()
    + 8 * std::mem::size_of::<OriginalV2UnsignedPlanObservation>()
    + 4 * (520 + 676 + 567 + 548)
    + candidate::CANDIDATE_BRIDGE_METADATA_BYTES
    + 4 * 1024;
// The transaction invoice charges this EXACT compiled layout, including the
// advance-cell pointer, rather than this upper bound. The closed advance-cell
// allocation is separately included in NEW B1MiB before allocation, not old A/I/S.
const _: () = assert!(V2_TEXT_UNSIGNED_CUSTODY_METADATA_BYTES <= 48 * 1024);

struct UnsignedPlanIdentity {
    original: V2AppendPlanObservation,
    retain: OriginalV2TextRetainTransactionObservation,
    resources: V2TextReadResources,
    domain: V2TextProjectionNegative,
    // LAST: observations must not refund their small actual metadata debit early.
    _metadata: V2TextTransactionMetadataResources,
}

/// Tiny exact-original identity, with no model, SDK payload, Source, File or session.
/// Sharing it cannot renew Current or substitute another equal-looking unsigned plan.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2UnsignedPlanObservation {
    body: Arc<UnsignedPlanIdentity>,
}
impl OriginalV2UnsignedPlanObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.body, &other.body) {
            return Err(refused());
        }
        self.body.retain.require_same_original(&other.body.retain)?;
        self.body
            .resources
            .require_same_original(&other.body.resources)?;
        self.body.domain.require_same_original(&other.body.domain)
    }
    /// Cached original domain/Source negatives only, for the actual short key guard.
    /// Ordinary original-association DATA remains comparable after cancellation.
    pub(in crate::store) fn validate_negative(&self) -> Result<(), AppError> {
        self.body.domain.validate_negative()
    }
}

struct CachedUnsignedData {
    signing: Zeroizing<Vec<u8>>,
    image: Zeroizing<Vec<u8>>,
    record: AcceptanceRecord,
    body: AppendBody,
    intent: [u8; 32],
}

/// Non-extractable whole original SDK/model custody. No mutable SDK/model/slot loan
/// exists. The narrow raw-sign operation requires the actual selected-I signing guard.
pub(in crate::store) struct PreparedV2UnsignedOriginalCustody {
    cached: CachedUnsignedData,
    original: OriginalV2UnsignedPlanObservation,
    // Empty until actual B pays/preallocates its closed advance cell outside writers.
    // After Core takes candidateNone is explained only by this explicit phase.
    advance: Option<Box<advance::AppendAdvanceCell>>,
    // Resource-only original account; never a Store, live Source or SDK/session owner.
    pool: CriticalCatalogReadPool,
    // LAST: all cached copies and observers above precede the actual model/SDK refunds.
    data: FirstUnsignedAppendData,
}
impl PreparedV2UnsignedOriginalCustody {
    /// Exact compiled metadata layout charged by the combined owning A/I invoice.
    /// SDK2M/resident1M/domain16KiB remain separate actual payments, not overlap credit.
    pub(in crate::store) const fn metadata_layout_bytes() -> usize {
        V2_TEXT_UNSIGNED_CUSTODY_METADATA_BYTES
    }
    #[cfg(all(test, windows))]
    fn actual_domain_for_test<T: MeshTransport, R: CryptoRngCore>(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
    ) -> Result<Arc<V2TextProjectionDomain>, AppError> {
        use crate::store::persistence::PreparedV2TextSourceObserver;
        let actual = owner.borrow_current()?;
        let prepared = PreparedV2TextSourceObserver::prepare(store, &actual)?;
        let mut observer = None;
        store.with_v2_text_current(&actual, |admission| {
            observer = Some(prepared.capture(admission)?);
            Ok(())
        })?;
        Ok(Arc::new(V2TextProjectionDomain::prepare(
            store,
            &actual,
            observer.as_ref().ok_or_else(refused)?,
        )?))
    }
    /// Adversarial fixture: substitute a second genuine opaque SDK child while keeping
    /// the first child's original observation. Rejection must preserve every input.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn assert_replaced_plan_rejected_for_test<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        selected_genesis: &SelectedV2TextHead,
        retained: &PreparedV2TextRetainTransaction,
    ) {
        let mut first = None;
        let mut second = None;
        let domain = Self::actual_domain_for_test(store, owner).unwrap();
        FirstUnsignedAppendData::prepare_into(
            store,
            owner,
            selected_genesis,
            retained,
            &domain,
            &mut first,
        )
        .unwrap();
        FirstUnsignedAppendData::prepare_into(
            store,
            owner,
            selected_genesis,
            retained,
            &domain,
            &mut second,
        )
        .unwrap();
        let replacement = second.as_mut().unwrap().prepared.take().unwrap();
        let original = first
            .as_mut()
            .unwrap()
            .prepared
            .replace(replacement)
            .unwrap();
        let mut output = None;
        assert!(Self::take_complete_into(store, owner, retained, &mut first, &mut output).is_err());
        assert!(output.is_none());
        let preserved = first
            .as_ref()
            .expect("refused handoff retains original slot");
        assert!(preserved
            .original_plan
            .as_ref()
            .unwrap()
            .matches_original_data(&original));
        assert!(!preserved
            .original_plan
            .as_ref()
            .unwrap()
            .matches_original_data(preserved.prepared.as_ref().unwrap()));
        // Restore the exact original SDK child; the same owning handoff now succeeds.
        let replacement = first.as_mut().unwrap().prepared.replace(original).unwrap();
        second.as_mut().unwrap().prepared = Some(replacement);
        Self::take_complete_into(store, owner, retained, &mut first, &mut output).unwrap();
        assert!(first.is_none());
        output
            .unwrap()
            .require_original_plan_data(retained)
            .unwrap();
    }
    /// Genuine-owner fixture adapter: derives through the same Root SDK preparation,
    /// then takes the complete original. It cannot adopt caller bytes or plan labels.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn prepare_for_test<T: MeshTransport, R: CryptoRngCore>(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        selected_genesis: &SelectedV2TextHead,
        retained: &PreparedV2TextRetainTransaction,
        model_notice: Arc<std::sync::atomic::AtomicBool>,
        sdk_notice: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Self, AppError> {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                NEXT_UNSIGNED_FEE_NOTICES.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        NEXT_UNSIGNED_FEE_NOTICES.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "nested exact unsigned fee observation"
            );
            *slot.borrow_mut() = Some((model_notice, sdk_notice));
        });
        let _reset = Reset;
        let mut input = None;
        let domain = Self::actual_domain_for_test(store, owner)?;
        FirstUnsignedAppendData::prepare_into(
            store,
            owner,
            selected_genesis,
            retained,
            &domain,
            &mut input,
        )?;
        NEXT_UNSIGNED_FEE_NOTICES.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "fixture missed actual unsigned payments"
            )
        });
        let mut output = None;
        Self::take_complete_into(store, owner, retained, &mut input, &mut output)?;
        assert!(input.is_none());
        output.ok_or_else(refused)
    }
    /// Complete all fallible checks and copies before moving the caller's unique slot.
    /// Rejection leaves the SAME opaque SDK plan and both real payments in that slot.
    pub(super) fn take_complete_into<T: MeshTransport, R: CryptoRngCore>(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        retained: &PreparedV2TextRetainTransaction,
        input: &mut Option<FirstUnsignedAppendData>,
        output: &mut Option<Self>,
    ) -> Result<(), AppError> {
        if let Some(existing) = output.as_ref() {
            if input.is_some() {
                return Err(refused());
            }
            existing.validate_accounts(store)?;
            return existing.require_original_plan_data(retained);
        }
        let data = input.as_ref().ok_or_else(refused)?;
        data.validate_accounts(store)?;
        retained
            .producer_data()
            .metadata_resources()
            .validate_prepaid_account()?;
        let original = data.original_plan.as_ref().ok_or_else(refused)?;
        if !original.matches_original_data(data.prepared.as_ref().ok_or_else(refused)?) {
            return Err(refused());
        }
        let view = data.borrow_unsigned(store, owner, retained)?;
        let record = Zeroizing::new(view.record.encode());
        let body = Zeroizing::new(view.body.encode());
        // These are the frozen first-Append codec maxima, not caller-expandable limits.
        if view.signing_bytes.is_empty()
            || view.signing_bytes.len() > 520
            || view.image.is_empty()
            || view.image.len() > 676
            || record.len() > 567
            || body.len() > 548
        {
            return Err(refused());
        }
        let cached = CachedUnsignedData {
            signing: Zeroizing::new(view.signing_bytes.to_vec()),
            image: Zeroizing::new(view.image.to_vec()),
            record: AcceptanceRecord::decode(&record).map_err(|_| refused())?,
            body: AppendBody::decode(&body).map_err(|_| refused())?,
            intent: view.intent,
        };
        let retain = retained.observe_original();
        let resources = retained.resources().clone();
        let metadata = retained.producer_data().metadata_resources().clone();
        let domain = data.resident_owner_data()?.domain.negative();
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let source = owner.borrow_current()?;
        resources.require_bound_original_negative(store, source.source())?;
        domain.validate_negative()?;
        drop(source);
        // Only infallible original moves follow. The SDK Weak observation never gets
        // replaced by encoded-byte equality, and its original identity is not renewed.
        let mut data = input.take().expect("same checked original unsigned slot");
        let original = OriginalV2UnsignedPlanObservation {
            body: Arc::new(UnsignedPlanIdentity {
                original: data
                    .original_plan
                    .take()
                    .expect("same checked SDK observation"),
                retain,
                resources,
                domain,
                _metadata: metadata,
            }),
        };
        *output = Some(Self {
            cached,
            original,
            advance: None,
            pool,
            data,
        });
        Ok(())
    }

    pub(in crate::store) fn validate_accounts(&self, store: &ServerStore) -> Result<(), AppError> {
        self.validate_accounts_only()?;
        self.data.validate_accounts(store)
    }

    /// Resource-only check for finite work that was genuinely admitted before Stop.
    /// No live Source, Scope, Store, key or writer is reacquired by that completion.
    pub(in crate::store) fn validate_accounts_only(&self) -> Result<(), AppError> {
        self.original.body._metadata.validate_prepaid_account()?;
        self.validate_model_account_or_transferred()?;
        self.data
            .sdk_fee
            .validate(&self.pool, APPEND_DATA_BUFFER_BYTES)?;
        Ok(())
    }

    pub(in crate::store) fn validate_original(
        &self,
        store: &ServerStore,
        source: &crate::store::persistence::GenesisSourceDataRef<'_>,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<(), AppError> {
        self.validate_accounts(store)?;
        self.require_original_plan_data(retained)?;
        self.data
            .resident_owner_data()?
            .domain
            .validate_negative()?;
        self.data
            .resources
            .require_bound_original_negative(store, source)
    }

    /// Pure original association; no allocation, account wait, SDK call or filesystem.
    pub(in crate::store) fn require_original_plan_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<(), AppError> {
        self.original
            .body
            .retain
            .require_same_original(&retained.observe_original())?;
        self.data
            .retain
            .require_same_original(&self.original.body.retain)?;
        self.data
            .resources
            .require_same_original(retained.resources())?;
        self.original
            .body
            .resources
            .require_same_original(&self.data.resources)?;
        if !self
            .original
            .body
            .original
            .matches_original_data(self.data.prepared.as_ref().ok_or_else(refused)?)
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store) fn observe_original(&self) -> OriginalV2UnsignedPlanObservation {
        self.original.clone()
    }

    /// Existing raw DATA only, including a failed post-cut SDK continuation. Absence
    /// stays absence and cannot invoke a signer or select the durable S transaction.
    pub(in crate::store) fn existing_raw_signature_data(&self) -> Result<&[u8; 64], AppError> {
        self.data
            .prepared
            .as_ref()
            .ok_or_else(refused)?
            .borrow_existing_raw_signature_data(&self.original.body.original)
            .map_err(|_| refused())
    }

    /// Borrow only the exact cached original DATA and action from its SAME A owner.
    /// This cannot create a selected-I receipt, key permission or current-source proof.
    pub(in crate::store) fn unsigned_data<'a>(
        &'a self,
        retained: &'a PreparedV2TextRetainTransaction,
    ) -> Result<V2UnsignedAppendDataView<'a>, AppError> {
        self.require_original_plan_data(retained)?;
        Ok(V2UnsignedAppendDataView {
            action: retained.producer_data().action_data()?,
            record: &self.cached.record,
            body: &self.cached.body,
            signing_bytes: &self.cached.signing,
            image: &self.cached.image,
            intent: self.cached.intent,
        })
    }
}
