//! Optional actual selected-I/raw-key boundary. Kept in a separate child so the
//! first-I durable Stage/Commit cohort can be checked without activating a key cut.
//! No historical SavedI, body digest or caller-supplied WAL grants signing authority.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    SavedUnsignedV2TextSigningGuard, V2_TEXT_SIGNING_GUARD_LAYOUT_BYTES,
};
use crate::store::persistence::V2TextOwnedCurrent;
use catcoms_rt::MeshTransport;

// A first-I WORK contains two bounded Head frames and exactly one bounded intent.
// Refuse a future larger shape at this Current-only boundary until its invoice and
// admission contract are reviewed. This is not the general 2MiB WAL scan worker.
const FIRST_I_SIGNING_WAL_CAP: usize = 8531;
const _: () = assert!(
    super::super::V2_TEXT_RETAIN_METADATA_LAYOUT_BYTES
        + INTENT_DATA_METADATA_BYTES
        + V2_TEXT_SIGNING_GUARD_LAYOUT_BYTES
        <= TEXT_TRANSACTION_METADATA_FEE
);

impl PreparedV2TextUnsignedIntent {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn cancel_original_projection_domain_for_test(&self) {
        self.unsigned.cancel_original_projection_domain_for_test();
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn observe_domain_cancel_after_sign_for_test(
        &self,
        dispatches: Arc<std::sync::atomic::AtomicUsize>,
    ) -> impl Drop + 'static {
        self.unsigned
            .observe_domain_cancel_after_sign_for_test(dispatches)
    }
    /// Existing negative DATA only, including raw output cached before a late SDK
    /// failure. The original SDK observation is checked by the closed custody;
    /// absence never falls back to signing, source refresh or Header materialization.
    pub(in crate::store) fn existing_raw_signature_data(&self) -> Result<&[u8; 64], AppError> {
        self.unsigned.existing_raw_signature_data()
    }

    /// Genuine-fixture assertion without exposing SDK/model/view or key ownership.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn assert_existing_raw_valid_for_test(&self) {
        let raw = self.unsigned.existing_raw_signature_data().unwrap();
        let view = self.unsigned.unsigned_data(&self.retained).unwrap();
        assert!(catcoms_crypto::verify_with_public_bytes(
            &self.parent.head.context.spec_data().signer,
            view.signing_bytes,
            raw,
        ));
    }

    /// Account waits and original SDK/preimage checks precede ALL final source
    /// proofs. Capture the SAME exact small WI and reserved-stage absences before
    /// those proofs; after final ID3 only held bytes/names and cached negatives remain.
    pub(in crate::store) fn validate_signing_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        saved: &SavedV2UnsignedAppendIntent,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.retained.custody.validate_accounts(guard.store())?;
        self.unsigned.validate_accounts(guard.store())?;
        self.unsigned.require_original_plan_data(&self.retained)?;
        let actual = self.saved.as_ref().ok_or_else(refused)?;
        if !std::ptr::eq(actual.as_ref(), saved) {
            return Err(refused());
        }
        saved
            .original
            .require_same_original(&self.observe_original())?;
        saved.require_unsigned_preimage_data(
            &self.unsigned.observe_original(),
            &self.unsigned.unsigned_data(&self.retained)?,
        )?;
        let expected = self.wal.as_ref().ok_or_else(refused)?.frame_data();
        if expected.is_empty()
            || expected.len() > FIRST_I_SIGNING_WAL_CAP
            || expected.len() != saved.wal_len
        {
            return Err(refused());
        }
        let mut held = super::super::ready_read::ReadyNamedFiles::capture(
            guard,
            &self.parent.target,
            self.parent.target_id,
            self.parent.pages()?,
            self.parent.pages_id,
        )?;
        // Exact expected bytes are borrowed only inside this validator. Native ID
        // plus length cannot detect same-file/same-length corrupt or later-phase WI.
        held.add_exact_file(
            owned_path(&self.parent.wal_path)?,
            saved.wal_native,
            expected,
        )?;
        held.add_absence(owned_path(&self.parent.wal_stage_path)?)?;
        held.add_absence(fixed_child(&self.parent.target, "head.stage")?)?;
        held.add_absence(fixed_child(&self.parent.target, "text.seed.stage.v1")?)?;
        // Genuine current O_I/I/H_A/T_A closure on the SAME saved/read body. This
        // performs the final original-source/ID3 proof while WI is already held.
        saved.validate_selected_under_current_guard(guard)?;
        // Includes SAME-File full byte comparison again, catching an in-place change
        // at the final source hook without allocating, reopening or refreshing source.
        held.require_unchanged()?;
        guard.validate_source_only()
    }

    /// The only owning Physical delegate to Root's bounded actual key forwarding.
    /// The guard borrows Current/lane/SavedI, not this SDK/model/custody owner.
    pub(in crate::store) fn sign_reserved_original<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        guard: &SavedUnsignedV2TextSigningGuard<'_, '_, '_>,
    ) -> Result<(), AppError> {
        guard.require_original_intent(&self.observe_original())?;
        self.unsigned
            .sign_admitted(store, owner, &self.retained, guard)
    }
}
