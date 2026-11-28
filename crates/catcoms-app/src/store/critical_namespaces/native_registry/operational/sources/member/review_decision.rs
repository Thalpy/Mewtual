//! The original native row lends a review-specific key cut together with
//! the actual whole recovered RuntimeLoan. No UI guard spans the IO.
use super::*;
use crate::store::NativeTextInitialReviewRequest;
use catcoms_replication::chat::owner_bodies::ReviewStreamEntry;
use catcoms_rt::{NativeChatKeyEnds, NativeChatKeyPurpose};

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberPreparation<T, R> {
    /// Cover the exact initial review signature on the original recovered
    /// founder. The returned entry is still private journal data. Selecting
    /// a case and releasing an approved original are separate operations.
    pub fn cover_initial_founder_review(
        self,
        request: NativeTextInitialReviewRequest,
        caller: &RequestCancellation,
    ) -> Result<ReviewStreamEntry, V2MemberSaveRefusal> {
        self.finish_initial_founder_review((request, false), caller)
    }

    /// Save the first exact founder decision in the selected private
    /// journal. This Rust entry point neither changes the ordinary
    /// projection nor releases a decision to another member.
    pub fn select_initial_founder_review(
        self,
        request: NativeTextInitialReviewRequest,
        caller: &RequestCancellation,
    ) -> Result<ReviewStreamEntry, V2MemberSaveRefusal> {
        self.finish_initial_founder_review((request, true), caller)
    }

    fn finish_initial_founder_review(
        self,
        request: (NativeTextInitialReviewRequest, bool),
        caller: &RequestCancellation,
    ) -> Result<ReviewStreamEntry, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let (cell, (keys, mut review_cut)) = self.prepare_using_issuer(|source, command| {
            let keys = NativeFounderKeys::prepare(source, command)?;
            let review = source
                .prepare_cut(command, NativeChatKeyPurpose::ReviewDecision)
                .map_err(|_| refused())?;
            Ok((keys, review))
        })?;
        let native = keys.original.clone();
        let mut job = self.finish_prepared((cell, keys))?;
        job.inner.require_current().map_err(|_| Refusal::Stale)?;
        let mut ends = NativeChatKeyEnds::new();
        let body = job.inner.loan.body.as_mut().ok_or(Refusal::Unavailable)?;
        let founder = body
            .recovered_founder
            .as_mut()
            .ok_or(Refusal::Unavailable)?;
        let result = founder.finish_initial_review(
            request,
            (&native, &mut review_cut, &mut ends),
            caller,
            body.rng.as_mut().ok_or(Refusal::Unavailable)?,
        );
        job.inner.loan.recorded = true;
        // Keep the End through the physical operation, then suppress any
        // result addressed to a caller that locked or changed rows.
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        job.inner.require_current().map_err(|_| Refusal::Stale)?;
        result.map_err(|error| match error {
            AppError::ChatV2RegistryBusy | AppError::Io(_) => Refusal::Busy,
            _ => Refusal::Unavailable,
        })
    }
}
