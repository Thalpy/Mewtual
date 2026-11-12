//! Actual Current admissions for the paired bounded token-index reader.
//!
//! The active outside bundle structurally couples captured Files/key with the original
//! physical OS lease even BEFORE admission or after a late capture refusal. A real
//! affine root reader follows them. Full result DATA alone is never a Create miss.
use super::*;
mod accepted;
pub(in crate::store) use accepted::{
    AdmittedV2TextAcceptedQuery, PreparedV2TextAcceptedQueryCapture, V2TextAcceptedQuerySourceSlot,
    V2_TEXT_ACCEPTED_QUERY_ADMISSION_LAYOUT_BYTES,
};
mod io;
use crate::store::owner_evidence::canonical::text_lane::V2TextQueryControlIoEnd;
use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextQueryLease, V2TextQueryLease, V2_TEXT_QUERY_READER_LAYOUT_BYTES,
};
use crate::store::persistence::v2_text::{
    CapturedV2TextTokenQuery, OriginalV2TextTokenQueryObservation, PreparedV2TextTokenQuery,
    V2TextTokenQueryCaptureClaim, V2TextTokenQueryResources,
};

/// Temporary active capture only, prepared immediately outside all host writers. Never
/// put it in dormant debt. Source is cached in-place BEFORE the first actual File/key.
pub(in crate::store) struct PreparedV2TextTokenQueryCapture {
    source: Option<CapturedV2TextTokenQuery>,
    original: OriginalV2TextTokenQueryObservation,
    // Every source File/key drains BEFORE same-vault physical unlock.
    physical: PreparedV2TextPhysicalLease,
    // Ends after Source/OS cleanup, while its logical reader may remain in result.
    control_io: Option<V2TextQueryControlIoEnd>,
    // Actual root-reader end follows all IO/OS cleanup, even on failed capture/worker Err.
    reader: Option<V2TextQueryLease>,
    // LAST: also pays this shell while source=None and the Prepared query has dropped.
    _resources: V2TextTokenQueryResources,
    // LAST: no second same-query source=None owner exists before this actual source,
    // reader and physical lease have drained, including all pre-admission error paths.
    _claim: V2TextTokenQueryCaptureClaim,
}

/// Borrowed in-place fill seam. No owned source/take/Option getter can detach Files/key
/// from their active same-store OS lease, including an error after the final File fill.
pub(in crate::store) struct V2TextTokenQuerySourceSlot<'capture> {
    source: &'capture mut Option<CapturedV2TextTokenQuery>,
}
impl V2TextTokenQuerySourceSlot<'_> {
    pub(in crate::store) fn get(&self) -> Option<&CapturedV2TextTokenQuery> {
        self.source.as_ref()
    }
    pub(in crate::store) fn get_mut(&mut self) -> Option<&mut CapturedV2TextTokenQuery> {
        self.source.as_mut()
    }
    pub(in crate::store) fn cache_initial(&mut self, source: CapturedV2TextTokenQuery) {
        assert!(self.source.is_none(), "actual query source already cached");
        *self.source = Some(source);
    }
}

/// Permission for one exact bounded physical lookup. It cannot select O, sign, create,
/// ACK or delete. Non-Clone, non-deserializable, with no raw frame/lease adopter.
pub(in crate::store) struct AdmittedV2TextTokenQuery {
    active: PreparedV2TextTokenQueryCapture,
}

/// Included in the physical query's independent1MiB invoice before ANY aliases/copies.
/// The fixed strong registry backing is separately reserved; source/SDK/read costs are
/// additional. This does not borrow any16MiB producer/stage work credit.
pub(in crate::store) const V2_TEXT_TOKEN_QUERY_ADMISSION_LAYOUT_BYTES: usize = 2
    * std::mem::size_of::<PreparedV2TextTokenQueryCapture>()
    + std::mem::size_of::<V2TextTokenQuerySourceSlot<'static>>()
    + std::mem::size_of::<AdmittedV2TextTokenQuery>()
    + V2_TEXT_QUERY_READER_LAYOUT_BYTES
    + 4 * 64;

impl PreparedV2TextTokenQueryCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        query: &PreparedV2TextTokenQuery,
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

    fn source_slot(&mut self) -> V2TextTokenQuerySourceSlot<'_> {
        V2TextTokenQuerySourceSlot {
            source: &mut self.source,
        }
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn has_source_for_test(&self) -> bool {
        self.source.is_some()
    }
}

impl AdmittedV2TextTokenQuery {
    pub(in crate::store) fn require_same_query(
        &self,
        original: &OriginalV2TextTokenQueryObservation,
    ) -> Result<(), AppError> {
        self.active.original.require_same_original(original)?;
        self.active
            .reader
            .as_ref()
            .ok_or_else(refused)?
            .require_same_query(original)
    }

    pub(in crate::store) fn source(&self) -> &CapturedV2TextTokenQuery {
        self.active
            .source
            .as_ref()
            .expect("real captured source admitted")
    }

    /// OUTSIDE all writers after the physical worker cached its COMPLETE path/result.
    /// This destroys source Files/key and the physical lease before caching the SAME
    /// affine reader in the result. An earlier Err simply drops this marker in the same
    /// structural order. No positive admission or account validation is renewed here.
    pub(in crate::store) fn finish_into(self, retained: &mut Option<V2TextQueryLease>) {
        assert!(
            retained.is_none(),
            "completed query already owns its reader"
        );
        let mut active = self.active;
        let reader = active.reader.take().expect("real admitted reader owned");
        drop(active);
        *retained = Some(reader);
    }
}

impl V2TextCurrentGroupGuard<'_, '_> {
    pub(in crate::store) fn admit_text_token_query_into(
        &mut self,
        query: &mut PreparedV2TextTokenQuery,
        reader: &mut PreparedV2TextQueryLease,
        capture: &mut Option<PreparedV2TextTokenQueryCapture>,
        admitted: &mut Option<AdmittedV2TextTokenQuery>,
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
            .validate_token_query_capture_under_guard(self)?;
        // Validate the SAME selected O/H BEFORE installing/recreating a negative pin.
        // The physical proof uses bounded ephemeral Files which all close before it
        // returns under this held canonical writer. A stale genuine parent therefore
        // cannot resurrect an already-closed generation after a newer O was selected.
        self.require_text_control_write_idle_for_query()?;
        query.validate_parent_under_current_guard(self)?;
        self.owner.pin_text_query_capture_into(
            reader,
            &mut active.reader,
            &mut active.control_io,
        )?;
        // No retained Source File/key can open until its SAME actual pin was cached.
        // A late capture refusal keeps that pin after Files/OSlease in the outside
        // bundle; future control/root drains cannot mistake failed capture for zero work.
        query.capture(self, &mut active.source_slot())?;
        active
            .source
            .as_ref()
            .ok_or_else(refused)?
            .validate_under_current_guard(self)?;
        // The last real source/ID3/name chain is complete. Only cached original and
        // canonical registry/closing controls may be sampled after this point.
        active.physical.require_token_query_capture_negative(self)?;
        self.validate_source_only()?;
        active
            .reader
            .as_ref()
            .ok_or_else(refused)?
            .require_under_namespace(&self.owner, &original)?;
        *admitted = Some(AdmittedV2TextTokenQuery {
            active: capture.take().expect("same outside active capture checked"),
        });
        Ok(())
    }
}

impl V2TextQueryLease {
    /// Call only AFTER physical completed-query validation proved the SAME selected O/H
    /// and roots. This final tail performs no account/FS/codec wait or root replacement.
    pub(in crate::store) fn require_live_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
        original: &OriginalV2TextTokenQueryObservation,
    ) -> Result<(), AppError> {
        self.require_under_namespace(&guard.owner, original)
    }
}

macro_rules! redacted_token_query_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_token_query_debug!(
    PreparedV2TextTokenQueryCapture,
    V2TextTokenQuerySourceSlot<'_>,
    AdmittedV2TextTokenQuery,
);
