//! Protected high-side Hello intake. All packet/action/receipt preparation precedes one short
//! Group -> Policy -> try-History publication. No engine or browser work runs in that fence.
use super::*;
use crate::voice_v2::chain::HelloAdmission;
use crate::voice_v2::crypto_actions::ActionQueue;
use crate::voice_v2::runtime::{ActorOwner, IngressTicket};

impl LocalJoin {
    /// Admit one authenticated Hello to its original native pair's reserved decrypt lane.
    /// The Root must first classify its shared public/encrypted message namespace, and supply
    /// a pure final room/call observer. This grants endpoint custody, never successful AEAD,
    /// a confirmed pair, permission to send Proof or live RTC readiness.
    pub(crate) fn admit_protected_hello(
        &mut self,
        actor: &ActorOwner,
        ticket: &IngressTicket,
        envelope: &Envelope,
        room: &crate::voice_v2::helper::RoomAuthority,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<Arc<HelloAdmission>, VoiceError> {
        if room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
            || actor.authorize_verified_route(ticket, envelope, authority.context)?
                != Disposition::Deliver
        {
            return Err(VoiceError::Unauthorized);
        }
        let object = envelope.object();
        let Body::Encrypted { header, .. } = object.body() else {
            return Err(VoiceError::Malformed);
        };
        if header.kind != EncryptedKind::ConfirmHello {
            return Err(VoiceError::Unauthorized);
        }
        self.pair_current(header.handshake, authority, clock.as_ref())?;
        let pair = &self.confirmations.pairs[&header.handshake];
        let chain = pair.chain.clone().ok_or(VoiceError::Unauthorized)?;
        if pair.low
            || pair.protected_epoch != Some(authority.context.epoch)
            || !chain.matches(
                &self.chain_binding(header.handshake, authority),
                &self.resources,
                &self.ledger.action_slots(),
                clock.monotonic_ms(),
            )
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = chain.find_hello(object)? {
            if !receipt.same_room(&room.lease) {
                return Err(VoiceError::Unauthorized);
            }
            // Abandoning the affine origin loan spends nothing. Cached admission is independent
            // of the consumed token and accepted-Hello floor, while current native ownership,
            // epoch and the original deadline remain mandatory.
            actor.with_endpoint_origin(
                ticket,
                envelope,
                authority.context,
                |wall, mono, _origin| {
                    final_owner(wall, mono)?;
                    if receipt.authorized_at(authority.context.epoch, wall, mono) {
                        Ok(())
                    } else {
                        Err(VoiceError::Unauthorized)
                    }
                },
            )?;
            return Ok(receipt);
        }
        if pair.established {
            return Err(VoiceError::Unauthorized);
        }
        let floor = pair.hello.candidate(header.sequence)?;
        let deadline = pair.deadline;
        let workflow = chain.workflow()?;
        let prepared = self
            .ledger
            .prepare_protected_hello(object.clone(), authority, clock.as_ref(), workflow)?
            .bind_confirmation(
                super::super::super::packets::PendingConfirmation::Hello(floor),
                deadline,
            )?;
        let (publication, event) =
            self.ledger
                .validate_prepared_inbound(prepared, authority, clock.as_ref())?;
        let effect = actor.effect(object.issuer(), authority.context.local, object.expires())?;
        let (history, receipt, workflow) = chain.prepare_hello(
            object,
            event.token,
            crate::voice_v2::chain::HelloConsent {
                room: room.lease.clone(),
                _room_custody: room.custody.clone(),
                effect,
                clock: clock.clone(),
            },
        )?;
        let (payload_bytes, engine_bytes) =
            crypto_actions::confirmation_bytes(event.aad().len(), event.ciphertext().len())?;
        let pending = publication
            .crypto_authority(clock)?
            .with_pending(chain.lease())?;
        let prepared_action = self.actions.prepare(
            Key::Decrypt(event.token),
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget: ByteBudget::Workflow(workflow),
                history: Some(history),
                payload_bytes,
                engine_bytes,
            },
        )?;
        let (mut issue, inactive) =
            ActionQueue::build_chain_decrypt(prepared_action, event, pending)?;
        // Owners stay OUTSIDE the FnOnce environment. A quota/owner refusal drops that closure
        // while Group/Policy are held; dropping a moved RowIssue there would reenter History.
        let mut publication = Some(publication);
        let mut inactive = Some(inactive);
        let mut published = None;
        actor.with_endpoint_origin(ticket, envelope, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            publication
                .as_ref()
                .expect("unpublished token")
                .admission()
                .check_at(wall, mono)?;
            issue.publish_hello(&receipt, origin, wall, mono, || {
                publication.take().expect("one token publication").publish();
                published = Some(
                    self.actions
                        .publish_inactive(inactive.take().expect("one action publication")),
                );
            })
        })?;
        // Control/engine retirement is deliberately outside Group/Policy/History. Close racing
        // this gap has already revoked the chain; registration completes unstarted retirement.
        chain.register_action(published.as_ref().expect("successful publication"));
        Ok(receipt)
    }
}
