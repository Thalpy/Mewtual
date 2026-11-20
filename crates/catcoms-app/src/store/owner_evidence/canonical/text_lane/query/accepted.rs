//! Concrete accepted Directory/Actions reader owners; no generic root/lease adopter.
//! Only the NEW actual accepted-query producer constructs this private shared core.
use super::*;
use crate::store::persistence::v2_text::{
    OriginalV2TextAcceptedQueryObservation, PreparedV2TextAcceptedQuery,
    V2TextAcceptedQueryReaderClaim,
};

pub(in crate::store) struct PreparedV2TextAcceptedQueryLease {
    core: PreparedV2TextQueryLease,
}
pub(in crate::store) struct V2TextAcceptedQueryLease {
    core: V2TextQueryLease,
}

/// Paid by the accepted producer's NEW query invoice. The fixed shared registry backing
/// still has its separate debit. Token fees do not fund an accepted reader or vice versa.
pub(in crate::store) const V2_TEXT_ACCEPTED_QUERY_READER_LAYOUT_BYTES: usize =
    std::mem::size_of::<QueryGeneration>()
        + std::mem::size_of::<PreparedV2TextAcceptedQueryLease>()
        + std::mem::size_of::<V2TextAcceptedQueryLease>()
        + std::mem::size_of::<QueryReaderEnd>()
        + std::mem::size_of::<V2TextQueryControlIoEnd>()
        + std::mem::size_of::<V2TextAcceptedQueryReaderClaim>()
        + 6 * 64;

impl PreparedV2TextAcceptedQueryLease {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &crate::store::persistence::GenesisSourceDataRef<'_>,
        query: &PreparedV2TextAcceptedQuery,
    ) -> Result<Self, AppError> {
        query.validate_preparation(store, source)?;
        let claim = query.claim_reader_preparation()?;
        let context = source.data().context.spec_data();
        if context.group.is_empty() || context.group.len() > MAX_GROUP_BYTES {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(V2_TEXT_QUERY_REGISTRY_BACKING_BYTES)?;
        fee.validate(pool, V2_TEXT_QUERY_REGISTRY_BACKING_BYTES)?;
        query.query_resources().validate_prepaid_account()?;
        query.validate_preparation(store, source)?;
        let mut group = [0; MAX_GROUP_BYTES];
        group[..context.group.len()].copy_from_slice(context.group);
        let original = QueryOrigin::Accepted(query.observe_original());
        let resources = QueryResourcesOrigin::Accepted(query.query_resources().clone());
        let claim = QueryReaderClaimAlias::Accepted(Arc::new(claim));
        let candidate = Arc::new(QueryGeneration {
            key: LaneKey {
                group,
                group_len: context.group.len(),
                signer: context.signer,
            },
            source: source.data().source,
            parent: original.clone(),
            state: AtomicUsize::new(0),
            control_io: AtomicUsize::new(0),
            wait: Mutex::new(()),
            changed: Condvar::new(),
            _resources: resources.clone(),
        });
        Ok(Self {
            core: PreparedV2TextQueryLease {
                candidate: Some(candidate),
                original: original.clone(),
                reader_original: Some(original),
                expansion: Some(QueryGenerationBuffer {
                    slots: Box::new(std::array::from_fn(|_| None)),
                    fee,
                }),
                expansion_redundant: false,
                candidate_redundant: false,
                displaced: None,
                resources,
                claim,
            },
        })
    }
    pub(in crate::store) fn require_same_query(
        &self,
        original: &OriginalV2TextAcceptedQueryObservation,
    ) -> Result<(), AppError> {
        self.core.original.require_accepted_original(original)
    }
    pub(in crate::store) fn validate_prepaid_account(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        self.core.validate_prepaid_account(store)
    }
    pub(in crate::store) fn drain_unused_outside_writers(&mut self) {
        self.core.drain_unused_outside_writers();
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// The real accepted-query guard has just proved SAME SavedA+Cell selected O/H.
    /// Cache its NEGATIVE pin before any retained Source opens. Sharing counters does
    /// not let token origins match this accepted Directory/Actions generation.
    pub(in crate::store) fn pin_text_accepted_query_capture_into(
        &mut self,
        prepared: &mut PreparedV2TextAcceptedQueryLease,
        retained: &mut Option<V2TextAcceptedQueryLease>,
        control_io: &mut Option<V2TextQueryControlIoEnd>,
    ) -> Result<(), AppError> {
        if let Some(existing) = retained.as_ref() {
            self.require_text_control_write_idle()?;
            control_io
                .as_ref()
                .ok_or_else(refused)?
                .require_same_reader(&existing.core)?;
            return existing
                .core
                .require_origin_under_namespace(self, &prepared.core.original);
        }
        let mut raw = None;
        self.pin_query_origin_capture_into(&mut prepared.core, &mut raw, control_io)?;
        // No fallible work follows the shared actual reservation. Transfer its exact
        // affine owner into the OUTSIDE typed slot before any late Scope postcheck.
        *retained = Some(V2TextAcceptedQueryLease {
            core: raw.expect("same actual accepted pin was cached"),
        });
        Ok(())
    }
}

impl V2TextAcceptedQueryLease {
    pub(in crate::store) fn require_same_query(
        &self,
        original: &OriginalV2TextAcceptedQueryObservation,
    ) -> Result<(), AppError> {
        self.core.original.require_accepted_original(original)
    }
    pub(in crate::store) fn require_live_query(
        &self,
        original: &OriginalV2TextAcceptedQueryObservation,
    ) -> Result<(), AppError> {
        self.require_same_query(original)?;
        self.core.require_live_origin(&self.core.original)
    }
    pub(in crate::store) fn require_under_namespace(
        &self,
        namespace: &V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2TextAcceptedQueryObservation,
    ) -> Result<(), AppError> {
        self.require_same_query(original)?;
        self.core
            .require_origin_under_namespace(namespace, &self.core.original)
    }
}

impl std::fmt::Debug for PreparedV2TextAcceptedQueryLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2TextAcceptedQueryLease")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2TextAcceptedQueryLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextAcceptedQueryLease")
            .finish_non_exhaustive()
    }
}
