//! The native founder audit borrows the original runtime only for local work.
use super::*;
use crate::store::persistence::v2_member_transport::{
    FounderReceiveOperation, FounderReceiveOutcome,
};
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
        let job = self.finish_prepared((cell, keys))?;
        match job.run_founder_member_receive(
            channel,
            FounderReceiveOperation::HeldAudit(after, limit),
            &caller,
        ) {
            Ok(FounderReceiveOutcome::HeldAudit(page)) => Ok(*page),
            Ok(_) => Err(Refusal::Failed),
            Err(Refusal::Failed) => Err(Refusal::Unavailable),
            Err(error) => Err(error),
        }
    }
}
