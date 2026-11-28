//! Serve one independently selected public decision on the original
//! founder's connected dependency lane.
use super::*;
use crate::store::persistence::governance_layout::joined::review_decision::ReviewDelivery;
use crate::store::persistence::v2_member_transport::{
    FounderDependencyReply, FOUNDER_DEPENDENCY_REPLY_FEE,
};
use catcoms_net::NativeChatMemberDependencyIncoming;
use catcoms_replication::chat::owner_bodies::ReviewStreamQuery;
use catcoms_sync::owner_v2::REVIEW_STREAM_CARRIER;
use std::cell::RefCell;
use zeroize::Zeroizing;

impl NativeTextColdFounderSource {
    pub(super) fn prepare_review_delivery_reply(
        &mut self,
        incoming: &NativeChatMemberDependencyIncoming,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<FounderDependencyReply, AppError> {
        let (requester, wire) = incoming.current_data().ok_or_else(refused)?;
        let (mount, fee) = self
            .work
            .recovered_founder_resources(FOUNDER_DEPENDENCY_REPLY_FEE)?;
        let (work, selection, source) = (&self.work, &self.selection, &self.source);
        let governed = self.governed.as_mut().ok_or_else(refused)?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let pin = review_decision::original_review_anchor(source);
        let current = || {
            if caller.is_cancelled() || incoming.current_data().is_none() {
                return Err(refused());
            }
            work.require_recovered_founder_source(selection, source)
        };
        let reply = work.with_recovered_founder_store(selection, source, |store| {
            governed.with_checked_review_parts(store, source, |verifier, _, frozen, physical| {
                let provider = sdk.select_member_dependency_access(
                    verifier,
                    REVIEW_STREAM_CARRIER,
                    self.local.0,
                    mount,
                )?;
                let request =
                    provider.open_review_delivery_request(verifier, &pin, requester, wire)?;
                let physical = RefCell::new(physical);
                let require = |anchor| {
                    current()?;
                    if !request.requester_data().is_live_cached() {
                        return Err(refused());
                    }
                    physical.borrow_mut()(anchor)
                };
                let query = request.query_data();
                let response = store.read_public_review(
                    &pin,
                    query,
                    |position| {
                        require(position.map(|position| {
                            (position.membership_header, position.membership_epoch)
                        }))
                    },
                    |record| {
                        let ReviewStreamQuery::Approval { channel, .. } = query else {
                            return Err(refused());
                        };
                        let held = record.held_data();
                        let target = held.target_data().spec();
                        if target.channel != channel {
                            return Ok(false);
                        }
                        let exact = record.decision_data().original_data();
                        require(Some((exact.remove_header, exact.remove_epoch)))?;
                        let author = review::frozen_author(frozen, target.context.admission)?;
                        // The writer already proved its reply parents
                        // before signing and journal selection. Serving
                        // rechecks the exact frozen original, without
                        // taking the ordinary archive gate inside ID11.
                        // The recipient must prove its own dependencies.
                        verifier
                            .verify_removed_create_record_for_audit(held, author)
                            .map_err(|_| refused())?;
                        Ok(true)
                    },
                )?;
                let frame = response.map(|reply| match reply {
                    ReviewDelivery::Header(entry) => Zeroizing::new(entry.encode()),
                    ReviewDelivery::Approval(frame) => frame,
                });
                require(None)?;
                let wire = provider.seal_review_delivery_reply(
                    &pin,
                    requester,
                    &request,
                    frame.as_deref().map(Vec::as_slice),
                    rng,
                )?;
                require(None)?;
                Ok(FounderDependencyReply {
                    wire: bytes::Bytes::from(wire),
                    observation: provider.into_observation(),
                    requester: Some(request.into_requester()),
                    fee,
                })
            })
        })?;
        self.restore_first_add(caller)?;
        Ok(reply)
    }
}
