//! Closed complete-SDP results. Signatures/input/result payloads are prepared outside
//! Engine/Group/Policy, and the final path uses original observations without callbacks.
//! Publishing a seal does not pump Source: the real engine/RPC must settle its original
//! Crypto lane before Source can borrow that lane. Capacity retains the signed cache.
use super::*;
use crate::voice_v2::presence::ProtectedSealedResult;

impl LocalJoin {
    #[cfg(test)]
    pub(crate) fn submit_sdp_sealed_result(
        &mut self,
        request: ProtectedSealedResult<'_>,
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        self.submit_sdp_sealed_result_scoped(
            request,
            signer,
            authority,
            clock,
            final_owner,
            |step| step.commit(),
        )
    }

    pub(crate) fn submit_sdp_sealed_result_scoped(
        &mut self,
        request: ProtectedSealedResult<'_>,
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        publish: impl FnOnce(crate::voice_v2::sdp::SdpResultAdmission<'_>) -> Result<(), VoiceError>,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        let ProtectedSealedResult {
            actor,
            room,
            action,
            ciphertext,
            claimed_tag,
        } = request;
        if room.scope != self.scope
            || !actor.uses_resources(&self.resources)
            || !action.belongs_to(self.scope, &self.lease)
            || action.operation() != CryptoOperation::SealSdp
            || !matches!(self.actions.key_of(action), Some(Key::SdpSeal(..)))
        {
            return Err(VoiceError::Unauthorized);
        }
        // Exact terminal recovery precedes consumed packet/input/settled-engine checks.
        // The new observation helper never samples Clock beneath Engine on either branch.
        if let Some(receipt) = action
            .observe_sdp_result(ciphertext, claimed_tag)?
            .recover()?
        {
            return Ok(receipt);
        }
        let input = action.input()?;
        let CryptoInput::SealSdp(flight) = input.as_input() else {
            return Err(VoiceError::Unauthorized);
        };
        let pc = action.sdp_binding().ok_or(VoiceError::Unauthorized)?;
        let pair = self
            .confirmations
            .pairs
            .get(&flight.handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if !pair
            .sdp_seal
            .as_ref()
            .is_some_and(|original| Arc::ptr_eq(original, action))
            || !pair
                .sdp_intent
                .as_ref()
                .is_some_and(|intent| pc.belongs_to_intent(intent))
            || !pc.matches_plaintext(&flight.plaintext)
            || (pair.low && pc.offer().message != flight.reservation.id)
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = pair.binding.peer;
        let pair_pin = pair.pin.clone();
        let _preparation = action.begin_result_preparation()?;
        if crate::voice_v2::sdp::sdp_result_preparation_bytes(
            flight.reservation.aad.len(),
            ciphertext.len(),
        )
        .is_none_or(|bytes| bytes > action.engine_bytes())
        {
            return Err(VoiceError::Capacity);
        }
        self.current(signer, authority)?;
        self.pair_current(flight.handshake, authority, clock)?;
        let mut publication = Some(self.ledger.prepare_sealed_publication(
            flight.reservation.id,
            ciphertext,
            signer,
            authority,
            clock,
        )?);
        let signed = publication.as_ref().expect("prepared signature").signed();
        let Body::Encrypted { header, .. } = signed.body() else {
            return Err(VoiceError::Unauthorized);
        };
        if header.offer != Some(pc.offer()) {
            return Err(VoiceError::Unauthorized);
        }
        let outcome = CryptoOutcome::Sealed {
            message: signed.id(),
            hash: signed.hash(),
        };
        let result_binding = ResultBinding {
            pair: pair_pin,
            attempt: None,
            deadline: pc.deadline(),
            expires: flight.reservation.expires,
        };
        let mut terminal = Some(TerminalResult {
            binding: result_binding,
            outcome,
        });
        let mut origin = Some(actor.prepare_local_origin(peer, authority.context)?);
        let (wall, mono) = origin
            .as_ref()
            .expect("original Actor samples")
            .observed_time();
        // Actor preparation invokes injected Clock. Refresh the Action's original
        // domain LAST so expiry during that preparation cannot authorize a Result.
        let mut observation = Some(action.observe_sdp_result(ciphertext, claimed_tag)?);
        let mut retired_signed = None;
        let mut result = None;
        let mut final_owner = Some(final_owner);
        {
            let mut commit = || {
                let committed = observation.take().ok_or(VoiceError::Unauthorized)?.commit(
                    |own_mono, own_wall| {
                        origin.take().expect("one original origin").with_origin(
                            |wall, mono, origin| {
                                final_owner.take().expect("one original owner check")(wall, mono)?;
                                if !room.lease.is_live() || !pc.current_at(wall, mono) {
                                    return Err(VoiceError::Unauthorized);
                                }
                                action.check_pending_at(own_mono, own_wall)?;
                                publication.as_ref().expect("one prepared seal").check_at(
                                    origin.epoch(),
                                    wall,
                                    mono,
                                )?;
                                origin.publish(|| {
                                    retired_signed = Some(
                                        publication.take().expect("one seal publication").publish(),
                                    );
                                })
                            },
                        )?;
                        Ok(terminal.take().expect("one immutable terminal"))
                    },
                );
                let status = committed.as_ref().map(|_| ()).map_err(|error| *error);
                result = Some(committed);
                status
            };
            publish(crate::voice_v2::sdp::SdpResultAdmission::new(
                action,
                crate::voice_v2::sdp::SdpResultIdentity::Bytes(claimed_tag),
                wall,
                mono,
                &mut commit,
            ))?;
        }
        // Signed clone, packet/slot/Phase and native result owners never drop under guards.
        drop((retired_signed, publication, terminal));
        result.ok_or(VoiceError::Unauthorized)?
    }
}
