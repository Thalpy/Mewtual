//! Native Open consumes the original authenticated token and publishes exact terminal history
//! plus its fixed delivery observation together. No Root plaintext cache or RTC call exists.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::sdp::{SdpDelivery, SdpOpenOutput, SdpOpenResult};

impl LocalJoin {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn submit_sdp_open_result(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        action: &Arc<CryptoAction>,
        output: SdpOpenOutput<'_>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<SdpOpenResult, VoiceError> {
        if room.scope != self.scope
            || !actor.uses_resources(&self.resources)
            || !action.belongs_to(self.scope, &self.lease)
            || action.operation() != CryptoOperation::OpenSdp
            || !matches!(self.actions.key_of(action), Some(Key::SdpOpen(_)))
        {
            return Err(VoiceError::Unauthorized);
        }
        // The same registered row can recover after input compaction and token consumption.
        // Native authenticates it independently after its Root participant has gone away.
        if let Some(result) = action.recover_sdp_open_result(output)? {
            return Ok(result);
        }
        let input = action.input()?;
        let CryptoInput::OpenSdp(packet) = input.as_input() else {
            return Err(VoiceError::Unauthorized);
        };
        let pc = action.sdp_binding().ok_or(VoiceError::Unauthorized)?;
        let pair = self
            .confirmations
            .pairs
            .get(&packet.header.handshake)
            .ok_or(VoiceError::Unauthorized)?;
        let admission = if pair.low {
            &pair.answer_receipt
        } else {
            &pair.first_offer
        }
        .as_ref()
        .ok_or(VoiceError::Unauthorized)?
        .clone();
        if self.actions.key_of(action) != Some(Key::SdpOpen(packet.token))
            || !pair
                .sdp_intent
                .as_ref()
                .is_some_and(|intent| pc.belongs_to_intent(intent))
            || !pc.matches_packet(packet)
            || !admission.same_room(&room.lease)
            || !admission.same_call(authority.lease)
            || admission.epoch() != authority.context.epoch
        {
            return Err(VoiceError::Unauthorized);
        }
        let pair_pin = pair.pin.clone();
        let _preparation = action.begin_result_preparation()?;
        self.pair_current(packet.header.handshake, authority, clock)?;
        let mut prepared = Some(self.ledger.validate_inbound_commit(
            packet.token,
            packet.aad(),
            packet.header.sequence,
            packet.ciphertext(),
            authority,
            clock,
        )?);
        let outcome = match output {
            SdpOpenOutput::Plaintext { bytes, .. } if std::str::from_utf8(bytes).is_ok() => {
                CryptoOutcome::Opened
            }
            SdpOpenOutput::Plaintext { .. } | SdpOpenOutput::AuthenticationFailed => {
                CryptoOutcome::Rejected(VoiceError::Malformed)
            }
        };
        // Exact byte hash/length is already checked by the observation. The fixed candidate
        // is fully allocated before any token/result effect and has no payload backedge.
        let mut delivery = if outcome == CryptoOutcome::Opened {
            Some(SdpDelivery::prepare(
                action,
                admission.clone(),
                packet,
                match output {
                    SdpOpenOutput::Plaintext { claimed_tag, .. } => claimed_tag,
                    SdpOpenOutput::AuthenticationFailed => unreachable!(),
                },
            )?)
        } else {
            None
        };
        let ready = prepared.as_ref().expect("original token plan");
        let mut terminal = Some(TerminalResult {
            binding: ResultBinding {
                pair: pair_pin,
                attempt: None,
                deadline: ready.retire.min(pc.deadline()),
                expires: ready.expires,
            },
            outcome,
        });
        self.ledger
            .validate_inbound_owner(ready, authority, clock)?;
        // All callback-bearing samples precede Engine. The final origin repeats original
        // current owners using only try-locks and pure observations; no await is allowed.
        let origin = actor.prepare_local_origin(packet.issuer, authority.context)?;
        let observed = action.observe_sdp_open_output(output)?;
        let mut retired_packet = None;
        let result = observed.commit_with_delivery(|own_mono, own_wall| {
            origin.with_origin(|wall, mono, origin| {
                final_owner(wall, mono)?;
                if !room.lease.is_live()
                    || !pc.current_at(wall, mono)
                    || !admission.authorized_at(wall, mono)
                {
                    return Err(VoiceError::Unauthorized);
                }
                action.check_pending_at(own_mono, own_wall)?;
                prepared.as_ref().expect("one original token").check_at(
                    origin.epoch(),
                    wall,
                    mono,
                )?;
                origin.publish(|| {
                    retired_packet = Some(self.ledger.consume_validated_inbound_deferred(
                        prepared.take().expect("one original token consumption"),
                    ));
                })
            })?;
            Ok((
                terminal.take().expect("one exact Open terminal"),
                delivery.take(),
            ))
        });
        // Destruction can return slots, notify revocation and retire PhaseLoans. Keep ALL
        // these actual owners outside Engine/Group/Policy even on refusal and exact retry.
        drop((retired_packet, prepared, terminal, delivery));
        result?;
        action
            .recover_sdp_open_result(output)?
            .ok_or(VoiceError::Unauthorized)
    }

    /// Fresh native RTC admission for the exact original accepted Open. It creates no new
    /// grant on retry; the adapter/browser must retain one SDK-started latch on this record.
    /// A historical receipt or a previously admitted record cannot bypass current ownership.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_sdp_delivery(
        &self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        action: &Arc<CryptoAction>,
        delivery: &Arc<SdpDelivery>,
        authority: &Authority<'_>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        self.transition_sdp_delivery(actor, room, action, delivery, authority, false, final_owner)
    }

    /// Trusted native completion after the original private SDK has actually applied this
    /// remote description. Root records causal order only; browser/native actual promises,
    /// strings and IO remain independently paid until their real last-owner destruction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complete_sdp_delivery(
        &self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        action: &Arc<CryptoAction>,
        delivery: &Arc<SdpDelivery>,
        authority: &Authority<'_>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        self.transition_sdp_delivery(actor, room, action, delivery, authority, true, final_owner)
    }

    #[allow(clippy::too_many_arguments)]
    fn transition_sdp_delivery(
        &self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        action: &Arc<CryptoAction>,
        delivery: &Arc<SdpDelivery>,
        authority: &Authority<'_>,
        applied: bool,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        if room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
            || !action.belongs_to(self.scope, &self.lease)
            || !action.owns_sdp_delivery(delivery)?
            || (applied && !delivery.was_admitted())
        {
            return Err(VoiceError::Unauthorized);
        }
        let Some(Key::SdpOpen(_)) = self.actions.key_of(action) else {
            return Err(VoiceError::Unauthorized);
        };
        let pc = action.sdp_binding().ok_or(VoiceError::Unauthorized)?;
        let pair = self
            .confirmations
            .pairs
            .get(&pc.handshake())
            .filter(|pair| {
                pair.sdp_intent
                    .as_ref()
                    .is_some_and(|intent| pc.belongs_to_intent(intent))
            })
            .ok_or(VoiceError::Unauthorized)?;
        let admission = if pair.low {
            &pair.answer_receipt
        } else {
            &pair.first_offer
        }
        .as_ref()
        .ok_or(VoiceError::Unauthorized)?;
        if !delivery.same_admission(admission)
            || !admission.same_call(authority.lease)
            || !admission.same_room(&room.lease)
            || admission.epoch() != authority.context.epoch
        {
            return Err(VoiceError::Unauthorized);
        }
        let origin = actor.prepare_local_origin(pair.binding.peer, authority.context)?;
        origin.with_origin(|wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live() || !delivery.current_at(wall, mono) {
                return Err(VoiceError::Unauthorized);
            }
            origin.publish(|| {
                if applied {
                    delivery.applied()
                } else {
                    delivery.admit()
                }
            })
        })
    }
}
