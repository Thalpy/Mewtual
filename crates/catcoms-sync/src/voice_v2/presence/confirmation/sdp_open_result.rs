//! Native Open consumes the original authenticated token and publishes exact terminal history
//! plus its fixed delivery observation together. No Root plaintext cache or RTC call exists.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::sdp::{SdpDelivery, SdpOpenOutput, SdpOpenResult};

impl LocalJoin {
    #[cfg(test)]
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
        self.submit_sdp_open_result_scoped(
            actor,
            room,
            action,
            output,
            authority,
            clock,
            final_owner,
            |step| step.commit(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn submit_sdp_open_result_scoped(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        action: &Arc<CryptoAction>,
        output: SdpOpenOutput<'_>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        publish: impl FnOnce(crate::voice_v2::sdp::SdpResultAdmission<'_>) -> Result<(), VoiceError>,
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
        if crate::voice_v2::sdp::sdp_result_preparation_bytes(
            packet.aad().len(),
            packet.ciphertext().len(),
        )
        .is_none_or(|bytes| bytes > action.engine_bytes())
        {
            return Err(VoiceError::Capacity);
        }
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
        let mut origin = Some(actor.prepare_local_origin(packet.issuer, authority.context)?);
        let (wall, mono) = origin
            .as_ref()
            .expect("original Actor samples")
            .observed_time();
        let mut observed = Some(action.observe_sdp_open_output(output)?);
        let mut retired_packet = None;
        let mut result = None;
        let mut final_owner = Some(final_owner);
        {
            let mut commit = || {
                let committed = observed
                    .take()
                    .ok_or(VoiceError::Unauthorized)?
                    .commit_with_delivery(|own_mono, own_wall| {
                        origin.take().expect("one original origin").with_origin(
                            |wall, mono, origin| {
                                final_owner.take().expect("one original owner check")(wall, mono)?;
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
                                    retired_packet = Some(
                                        self.ledger.consume_validated_inbound_deferred(
                                            prepared
                                                .take()
                                                .expect("one original token consumption"),
                                        ),
                                    );
                                })
                            },
                        )?;
                        Ok((
                            terminal.take().expect("one exact Open terminal"),
                            delivery.take(),
                        ))
                    });
                let status = committed.as_ref().map(|_| ()).map_err(|error| *error);
                result = Some(committed);
                status
            };
            let identity = match output {
                SdpOpenOutput::Plaintext { claimed_tag, .. } => {
                    crate::voice_v2::sdp::SdpResultIdentity::Bytes(claimed_tag)
                }
                SdpOpenOutput::AuthenticationFailed => {
                    crate::voice_v2::sdp::SdpResultIdentity::AuthenticationFailed
                }
            };
            publish(crate::voice_v2::sdp::SdpResultAdmission::new(
                action,
                identity,
                wall,
                mono,
                &mut commit,
            ))?;
        }
        // Destruction can return slots, notify revocation and retire PhaseLoans. Keep ALL
        // these actual owners outside Engine/Group/Policy even on refusal and exact retry.
        drop((retired_packet, prepared, terminal, delivery));
        result.ok_or(VoiceError::Unauthorized)??;
        action
            .recover_sdp_open_result(output)?
            .ok_or(VoiceError::Unauthorized)
    }

    /// Fresh native RTC admission for the exact original accepted Open. It creates no new
    /// grant on retry; the adapter/browser must retain one SDK-started latch on this record.
    /// A historical receipt or a previously admitted record cannot bypass current ownership.
    #[cfg(test)]
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
        self.with_sdp_delivery_admission(
            actor,
            room,
            action,
            delivery,
            authority,
            false,
            final_owner,
            |step| step.commit(),
        )
    }

    /// Trusted native completion after the original private SDK has actually applied this
    /// remote description. Root records causal order only; browser/native actual promises,
    /// strings and IO remain independently paid until their real last-owner destruction.
    #[cfg(test)]
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
        self.with_sdp_delivery_admission(
            actor,
            room,
            action,
            delivery,
            authority,
            true,
            final_owner,
            |step| step.commit(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_sdp_delivery_admission(
        &self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        action: &Arc<CryptoAction>,
        delivery: &Arc<SdpDelivery>,
        authority: &Authority<'_>,
        applied: bool,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        publish: impl FnOnce(crate::voice_v2::sdp::SdpDeliveryAdmission<'_>) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        if room.scope != self.scope
            || !actor.uses_resources(&self.resources)
            || !action.belongs_to(self.scope, &self.lease)
            || !action.owns_sdp_delivery(delivery)?
        {
            return Err(VoiceError::Unauthorized);
        }
        let Some(Key::SdpOpen(_)) = self.actions.key_of(action) else {
            return Err(VoiceError::Unauthorized);
        };
        // An exact already-recorded Applied observation is recoverable after End. This
        // branch grants no new RTC admission and never invokes the unused native hook.
        if applied && delivery.was_applied() {
            return Ok(());
        }
        if !room.lease.is_live() || (applied && !delivery.was_admitted()) {
            return Err(VoiceError::Unauthorized);
        }
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
        let mut origin = Some(actor.prepare_local_origin(pair.binding.peer, authority.context)?);
        let (wall, mono) = origin
            .as_ref()
            .expect("original delivery origin")
            .observed_time();
        let mut final_owner = Some(final_owner);
        let mut committed = None;
        let result;
        {
            let mut commit = || {
                let result = origin.take().ok_or(VoiceError::Unauthorized)?.with_origin(
                    |wall, mono, origin| {
                        final_owner.take().expect("one original delivery owner")(wall, mono)?;
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
                    },
                );
                committed = Some(result);
                result
            };
            // Native enters UI/Coordinator/route/row BEFORE commit enters Group/Policy.
            // Prepared origin and all owned caller graphs remain in this outer frame.
            result = publish(crate::voice_v2::sdp::SdpDeliveryAdmission::new(
                action,
                delivery,
                applied,
                wall,
                mono,
                &mut commit,
            ));
        }
        match committed {
            // A caller error after actual publication cannot contradict that transition.
            Some(result) => result,
            None => {
                result?;
                Err(VoiceError::Unauthorized) // A no-op hook cannot report admission.
            }
        }
    }
}
