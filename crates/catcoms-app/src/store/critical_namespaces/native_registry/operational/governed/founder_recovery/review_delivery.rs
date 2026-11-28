//! Serve one independently selected public decision on the original
//! founder's connected dependency lane.
use super::*;
use crate::store::persistence::v2_member_transport::{
    prepare_founder_review_delivery_reply, FounderDependencyReply, FOUNDER_DEPENDENCY_REPLY_FEE,
};
use catcoms_net::NativeChatMemberDependencyIncoming;
use catcoms_sync::owner_v2::REVIEW_STREAM_CARRIER;

impl NativeTextColdFounderSource {
    pub(super) fn prepare_review_delivery_reply(
        &mut self,
        incoming: &NativeChatMemberDependencyIncoming,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<FounderDependencyReply, AppError> {
        incoming.current_data().ok_or_else(refused)?;
        let (mount, fee) = self
            .work
            .recovered_founder_resources(FOUNDER_DEPENDENCY_REPLY_FEE)?;
        let (work, selection, source) = (&self.work, &self.selection, &self.source);
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let pin = review_decision::original_review_anchor(source);
        let reply = work.with_recovered_founder_store(selection, source, |store| {
            governed.with_checked_review_parts(store, source, |verifier, _, frozen, physical| {
                let provider = sdk.select_member_dependency_access(
                    verifier,
                    REVIEW_STREAM_CARRIER,
                    self.local.0,
                    mount,
                )?;
                prepare_founder_review_delivery_reply(
                    store,
                    (&pin, verifier, frozen),
                    provider,
                    (incoming, caller),
                    fee,
                    |anchor| {
                        work.require_recovered_founder_source(selection, source)?;
                        physical(anchor)
                    },
                    rng,
                )
            })
        })?;
        self.restore_first_add(caller)?;
        Ok(reply)
    }
}
