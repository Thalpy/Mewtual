//! The native founder audit borrows the original runtime only for local work.
use super::*;
use crate::store::{NativeTextHeldReviewContinuation, NativeTextHeldReviewPage};

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberPreparation<T, R> {
    pub fn audit_founder_held_posts(
        self,
        channel: u128,
        after: Option<&NativeTextHeldReviewContinuation>,
        limit: usize,
        caller: &RequestCancellation,
    ) -> Result<NativeTextHeldReviewPage, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let (cell, mut keys) = self.prepare_keys()?;
        let selected = keys.member_entry().1.observe_current();
        let caller = connected::constrain_founder_transport(
            self.scope.clone(),
            selected,
            caller.clone(),
            None,
        );
        let mut job = self.finish_prepared((cell, keys))?;
        job.inner.require_current().map_err(|_| Refusal::Stale)?;
        let founder = job
            .inner
            .loan
            .body
            .as_mut()
            .and_then(|body| body.recovered_founder.as_mut())
            .ok_or(Refusal::Unavailable)?;
        let result = founder
            .selected_held_review_window(channel, after, limit, &caller)
            .map_err(|_| Refusal::Unavailable);
        job.inner.loan.recorded = true;
        job.inner.require_current().map_err(|_| Refusal::Stale)?;
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        result
    }
}
