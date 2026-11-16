//! Actual selected-I raw-key admission. This is neither Stage/Commit nor a signer API.
//!
//! The closed guard stays within the SAME Current callback and borrows its held Root/
//! canonical ownership, the real lane and genuine SavedI. The temporary prepared-I loan
//! is deliberately absent from its lifetime and fields, so Root can then mutably borrow
//! only its whole original custody to cache the bounded raw signature.
use super::*;
use crate::store::persistence::v2_text::SavedV2UnsignedAppendIntent;

/// Non-Clone, non-deserializable, no caller constructor or owning permission conversion.
/// It contains only borrowed real guards/receipt and exact paid original observations;
/// no mutable SDK/model/node/key/source/bytes callback escapes through this type.
pub(in crate::store) struct SavedUnsignedV2TextSigningGuard<'sign, 'guard, 'writer> {
    current: &'sign V2TextCurrentGroupGuard<'guard, 'writer>,
    lane: &'sign V2TextCreateLane,
    _saved: &'sign SavedV2UnsignedAppendIntent,
    original: OriginalV2TextUnsignedIntentObservation,
    unsigned: OriginalV2UnsignedPlanObservation,
    retain: OriginalV2RetainObservation,
}

/// Additional simultaneous metadata in the actual I/A debit. Root's model1M/SDK2M
/// funds its real original payload/sign scratch separately; no guard creates credit.
pub(in crate::store) const V2_TEXT_SIGNING_GUARD_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<SavedUnsignedV2TextSigningGuard<'static, 'static, 'static>>()
        + 2 * std::mem::size_of::<OriginalV2TextUnsignedIntentObservation>()
        + 2 * std::mem::size_of::<OriginalV2UnsignedPlanObservation>()
        + 2 * std::mem::size_of::<OriginalV2RetainObservation>()
        + 4 * 64;

impl<'guard, 'writer> V2TextCurrentGroupGuard<'guard, 'writer> {
    /// `prepared` intentionally has NO `'sign` lifetime. The actual caller must clone
    /// its genuine SavedI Arc into an outside local/cache FIRST, and pass a borrow of
    /// that clone: borrowing a SavedI field directly from prepared would still prevent
    /// the later mutable whole-custody raw cut. No SDK/model/raw getter is needed.
    ///
    /// The concrete physical validator must prove signable workspace as well as exact
    /// O_I/I/H_A/T_A and cached SDK/preimage association. Later S/B or uncertain staged
    /// controls beat a historical I; a DATA Absent/SavedI label is never this permission.
    pub(in crate::store) fn admit_unsigned_signing<'sign>(
        &'sign mut self,
        lane: &'sign V2TextCreateLane,
        saved: &'sign SavedV2UnsignedAppendIntent,
        prepared: &PreparedV2TextUnsignedIntent,
    ) -> Result<SavedUnsignedV2TextSigningGuard<'sign, 'guard, 'writer>, AppError> {
        let original = prepared.observe_original();
        let unsigned = prepared.original_unsigned();
        let retain = prepared.original_retain();
        prepared
            .resources()
            .require_same_original(self.resources())?;
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.owner.require_text_control_io_quiescent()?;
        self.require_text_control_write_idle_for_query()?;
        // Accounts/actual SDK original/preimage/workspace inspection occur INSIDE the
        // concrete issuer BEFORE its FINAL full current source/name proof. This cannot
        // be implemented by a parsed I, historical Saved or general validator callback.
        prepared.validate_signing_under_current_guard(self, saved)?;
        // Only cached native checks remain after that final physical proof. Canonical
        // stays held through the key cut; no new writer can race inspection then sign.
        lane.require_workspace_idle(&mut self.owner, &retain)?;
        self.owner.require_text_control_io_quiescent()?;
        lane.require_signing_negative(&retain)?;
        self.validate_source_only()?;
        unsigned.validate_negative()?;
        Ok(SavedUnsignedV2TextSigningGuard {
            current: self,
            lane,
            _saved: saved,
            original,
            unsigned,
            retain,
        })
    }
}

impl SavedUnsignedV2TextSigningGuard<'_, '_, '_> {
    /// Exact actual I issuer only, for the owning physical delegate before Root's cut.
    /// This does not borrow the mutable prepared-I object or perform any new wait.
    pub(in crate::store) fn require_original_intent(
        &self,
        original: &OriginalV2TextUnsignedIntentObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }
    pub(in crate::store) fn require_original_unsigned(
        &self,
        original: &OriginalV2UnsignedPlanObservation,
    ) -> Result<(), AppError> {
        self.unsigned.require_same_original(original)
    }
    pub(in crate::store) fn require_original_retain(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.retain.require_same_original(original)
    }
    /// Pure cached negatives immediately before/after the existing bounded key call.
    /// No account, filesystem, SDK derivation, new guard, refresh or lifetime upgrade.
    /// A cached raw signature after a late failure survives via the separate DATA path.
    pub(in crate::store) fn validate_negative(&self) -> Result<(), AppError> {
        // This SAME original observer carries the real projection domain and pinned
        // Source lineage. Domain cancellation is independent of lane/Current liveness.
        self.unsigned.validate_negative()?;
        self.current.validate_source_only()?;
        self.lane.require_signing_negative(&self.retain)?;
        self.current.owner.require_text_control_io_quiescent()?;
        self.current.require_text_control_write_idle_for_query()?;
        self.current.validate_source_only()?;
        self.unsigned.validate_negative()
    }
}

impl std::fmt::Debug for SavedUnsignedV2TextSigningGuard<'_, '_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedUnsignedV2TextSigningGuard")
            .finish_non_exhaustive()
    }
}
