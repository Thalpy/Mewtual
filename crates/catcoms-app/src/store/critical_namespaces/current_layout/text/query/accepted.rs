//! Concrete accepted Directory/Actions query admission from actual selected SavedA+Cell.
//! Token Empty, parsed roots and historical A alone cannot issue this affine owner.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextAcceptedQueryLease, V2TextAcceptedQueryLease,
    V2_TEXT_ACCEPTED_QUERY_READER_LAYOUT_BYTES,
};
use crate::store::persistence::v2_text::{
    CapturedV2TextAcceptedQuery, OriginalV2TextAcceptedQueryObservation,
    PreparedV2TextAcceptedQuery, V2TextAcceptedQueryCaptureClaim, V2TextAcceptedQueryResources,
};

/// Imminent active capture only. Source Files/key are FIRST, real native lease then
/// finite IO end, then SAME affine root reader; claims/payments are resource-only LAST.
/// The negative pin and IO end are installed before ANY retained source File can open.
pub(in crate::store) struct PreparedV2TextAcceptedQueryCapture {
    source: Option<CapturedV2TextAcceptedQuery>,
    original: OriginalV2TextAcceptedQueryObservation,
    physical: PreparedV2TextPhysicalLease,
    control_io: Option<V2TextQueryControlIoEnd>,
    reader: Option<V2TextAcceptedQueryLease>,
    _resources: V2TextAcceptedQueryResources,
    _claim: V2TextAcceptedQueryCaptureClaim,
}

/// In-place fill only, obtainable solely by the hard-coded actual capture call below.
/// No caller constructor, owned Option/take or detached Source getter exists.
pub(in crate::store) struct V2TextAcceptedQuerySourceSlot<'capture> {
    source: &'capture mut Option<CapturedV2TextAcceptedQuery>,
}
impl V2TextAcceptedQuerySourceSlot<'_> {
    pub(in crate::store) fn get(&self) -> Option<&CapturedV2TextAcceptedQuery> {
        self.source.as_ref()
    }
    pub(in crate::store) fn get_mut(&mut self) -> Option<&mut CapturedV2TextAcceptedQuery> {
        self.source.as_mut()
    }
    pub(in crate::store) fn cache_initial(&mut self, source: CapturedV2TextAcceptedQuery) {
        assert!(self.source.is_none(), "same accepted source already cached");
        *self.source = Some(source);
    }
}

/// One bounded actual accepted lookup, not a signer, selector, publication or WC permit.
/// No mutable Source loan can detach cross-vault Files from this real native lease.
pub(in crate::store) struct AdmittedV2TextAcceptedQuery {
    active: PreparedV2TextAcceptedQueryCapture,
}
pub(in crate::store) const V2_TEXT_ACCEPTED_QUERY_ADMISSION_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2TextAcceptedQueryCapture>()
    + std::mem::size_of::<V2TextAcceptedQuerySourceSlot<'static>>()
    + std::mem::size_of::<AdmittedV2TextAcceptedQuery>()
    + V2_TEXT_ACCEPTED_QUERY_READER_LAYOUT_BYTES
    + 4 * 64;

impl PreparedV2TextAcceptedQueryCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        query: &PreparedV2TextAcceptedQuery,
    ) -> Result<Self, AppError> {
        query.validate_preparation(store, source)?;
        let claim = query.claim_capture_preparation()?;
        let resources = query.query_resources().clone();
        let physical = PreparedV2TextPhysicalLease::prepare(store, source, query.resources())?;
        query.validate_preparation(store, source)?;
        Ok(Self {
            source: None,
            original: query.observe_original(),
            physical,
            control_io: None,
            reader: None,
            _resources: resources,
            _claim: claim,
        })
    }
    fn source_slot(&mut self) -> V2TextAcceptedQuerySourceSlot<'_> {
        V2TextAcceptedQuerySourceSlot {
            source: &mut self.source,
        }
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn has_source_for_test(&self) -> bool {
        self.source.is_some()
    }
}

impl AdmittedV2TextAcceptedQuery {
    pub(in crate::store) fn require_same_query(
        &self,
        original: &OriginalV2TextAcceptedQueryObservation,
    ) -> Result<(), AppError> {
        self.active.original.require_same_original(original)?;
        self.active
            .reader
            .as_ref()
            .ok_or_else(refused)?
            .require_same_query(original)
    }
    /// Immutable source only; all page IO helpers retain their local Files until
    /// completion inside this SAME marker's actual physical lease/end lifetime.
    pub(in crate::store) fn source(&self) -> &CapturedV2TextAcceptedQuery {
        self.active
            .source
            .as_ref()
            .expect("same actual accepted source admitted")
    }
    /// Outside all writers, after COMPLETE path/result DATA is already cached. Drain
    /// Source/key→OS lease→actual finite IO before retaining the SAME logical reader.
    /// On an earlier Err, ordinary field order drains all of them before the claim.
    pub(in crate::store) fn finish_into(self, retained: &mut Option<V2TextAcceptedQueryLease>) {
        assert!(retained.is_none(), "accepted result already owns a reader");
        let mut active = self.active;
        let reader = active.reader.take().expect("same actual accepted reader");
        drop(active);
        *retained = Some(reader);
    }
}

impl V2TextCurrentGroupGuard<'_, '_> {
    pub(in crate::store) fn admit_text_accepted_query_into(
        &mut self,
        query: &mut PreparedV2TextAcceptedQuery,
        reader: &mut PreparedV2TextAcceptedQueryLease,
        capture: &mut Option<PreparedV2TextAcceptedQueryCapture>,
        admitted: &mut Option<AdmittedV2TextAcceptedQuery>,
    ) -> Result<(), AppError> {
        let original = query.observe_original();
        if let Some(existing) = admitted.as_ref() {
            existing.require_same_query(&original)?;
            return self.validate_source_only();
        }
        reader.require_same_query(&original)?;
        reader.validate_prepaid_account(self.store())?;
        let active = capture.as_mut().ok_or_else(refused)?;
        active.original.require_same_original(&original)?;
        active
            .physical
            .validate_accepted_query_capture_under_guard(self)?;
        self.require_text_control_write_idle_for_query()?;
        // Actual SavedA's SAME original read body and exact selected O_A/H_A closure
        // are proved BEFORE pin insertion. A stale historical receipt cannot resurrect
        // an old generation or open retired controls while a newer O is selected.
        query.validate_parent_under_current_guard(self)?;
        self.owner.pin_text_accepted_query_capture_into(
            reader,
            &mut active.reader,
            &mut active.control_io,
        )?;
        query.capture(self, &mut active.source_slot())?;
        active
            .source
            .as_ref()
            .ok_or_else(refused)?
            .validate_under_current_guard(self)?;
        active
            .physical
            .require_accepted_query_capture_negative(self)?;
        self.validate_source_only()?;
        active
            .reader
            .as_ref()
            .ok_or_else(refused)?
            .require_under_namespace(&self.owner, &original)?;
        *admitted = Some(AdmittedV2TextAcceptedQuery {
            active: capture
                .take()
                .expect("same outside accepted source checked"),
        });
        Ok(())
    }
}

impl V2TextAcceptedQueryLease {
    /// Final cached namespace/closing tail AFTER the physical completed query's fresh
    /// SAME SavedA/current-O proof. No account/FS/codec wait follows this negative tail.
    pub(in crate::store) fn require_live_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
        original: &OriginalV2TextAcceptedQueryObservation,
    ) -> Result<(), AppError> {
        self.require_under_namespace(&guard.owner, original)
    }
}

macro_rules! redacted_accepted_query_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_accepted_query_debug!(
    PreparedV2TextAcceptedQueryCapture,
    V2TextAcceptedQuerySourceSlot<'_>,
    AdmittedV2TextAcceptedQuery,
);
