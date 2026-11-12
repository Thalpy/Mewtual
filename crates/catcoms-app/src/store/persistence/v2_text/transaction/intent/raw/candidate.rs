//! Closed owning S?Root materialization forwarding. This creates DATA only; no
//! signature, Source/current grant, verifier advance or publication can follow from
//! this method alone. Actual B admission and its Saved barrier remain separate.
use super::*;
use crate::store::persistence::V2TextOwnedCurrent;
use catcoms_rt::MeshTransport;

impl PreparedV2TextRawSignature {
    /// Observe only the actual Root materializer/derive calls; no SDK/model loan.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn observe_candidate_work_for_test(
        &self,
        counts: Arc<[std::sync::atomic::AtomicUsize; 2]>,
        cancel_after_cache: bool,
    ) -> impl Drop + 'static {
        self.intent
            .unsigned
            .observe_candidate_work_for_test(counts, cancel_after_cache)
    }

    /// Existing cached candidate DATA remains observable after negative callbacks.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn verified_candidate_digest_for_test(
        &self,
    ) -> Result<[u8; 32], AppError> {
        self.intent
            .unsigned
            .verified_candidate_digest_for_test(&self.intent.retained)
    }

    /// Pass ONLY this actual S's own receipt and whole original SDK/A custody. Root
    /// proves fresh selected S briefly, releases writers, then materializes/derives
    /// the exact already-signed candidate under its actual separate payments.
    pub(in crate::store) fn materialize_candidate_after_saved_s<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
    ) -> Result<(), AppError> {
        let saved = Arc::clone(self.saved.as_ref().ok_or_else(refused)?);
        saved
            .original
            .require_same_original(&self.observe_original())?;
        saved.resources().require_same_original(self.resources())?;
        self.intent.unsigned.materialize_candidate_after_saved_s(
            store,
            owner,
            &self.intent.retained,
            &saved,
        )
    }
}
