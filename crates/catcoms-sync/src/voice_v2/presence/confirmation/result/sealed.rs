//! Sign/copy before Engine and native admission, retaining an inert signature across local
//! refusal. The source-visible packet, native pair state and terminal result publish once.
use super::*;

pub(crate) struct ProtectedSealedResult<'a> {
    pub actor: &'a ActorOwner,
    pub room: &'a RoomAuthority,
    pub action: &'a Arc<CryptoAction>,
    pub ciphertext: &'a [u8],
    pub claimed_tag: [u8; 32],
}

pub(in crate::voice_v2::presence::confirmation) struct SealedResult<'a> {
    pub action: &'a CryptoAction,
    pub ciphertext: &'a [u8],
    pub claimed_tag: [u8; 32],
    pub signer: &'a dyn VoiceSigner,
}
pub(in crate::voice_v2::presence::confirmation) struct ResultOrigin<'a> {
    pub actor: &'a ActorOwner,
    pub room: &'a RoomAuthority,
}

struct PreparedSeal {
    handshake: Nonce,
    kind: EncryptedKind,
    pair_signed: Option<SignedObject>,
    returned: Option<SignedObject>,
    window: Option<crate::voice_v2::chain::PreparedFinishWindow>,
    identity: crate::voice_v2::chain::FinishIdentity,
}
impl PreparedSeal {
    fn publish(
        &mut self,
        publication: &mut Option<crate::voice_v2::packets::PreparedSealedPublication<'_>>,
        confirmations: &mut Confirmations,
        retirement: &mut ConfirmationRetirement,
        mono: u64,
    ) -> Result<(), VoiceError> {
        if let Some(window) = &self.window {
            window.publish_at(mono, self.identity)?;
        }
        publication
            .take()
            .expect("one signature publication")
            .publish_with(|ledger, signed| {
                self.returned = Some(signed);
                apply_confirmation_seal(
                    confirmations,
                    ledger,
                    self.handshake,
                    self.kind,
                    self.pair_signed.take().expect("prepared pair signature"),
                    retirement,
                );
            });
        Ok(())
    }
}

impl LocalJoin {
    pub(crate) fn submit_protected_confirmation_result(
        &mut self,
        request: ProtectedSealedResult<'_>,
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        if request.room.scope != self.scope
            || !request.actor.uses_resources(&self.resources)
            || !matches!(self.actions.key_of(request.action), Some(Key::Encrypt(..)))
        {
            return Err(VoiceError::Unauthorized);
        }
        self.seal_confirmation_result(
            SealedResult {
                action: request.action,
                ciphertext: request.ciphertext,
                claimed_tag: request.claimed_tag,
                signer,
            },
            authority,
            clock,
            Some(ResultOrigin {
                actor: request.actor,
                room: request.room,
            }),
            final_owner,
        )
    }

    pub(in crate::voice_v2::presence::confirmation) fn seal_confirmation_result(
        &mut self,
        request: SealedResult<'_>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        origin: Option<ResultOrigin<'_>>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        let SealedResult {
            action,
            ciphertext,
            claimed_tag,
            signer,
        } = request;
        if !action.belongs_to(self.scope, &self.lease)
            || action.operation() != CryptoOperation::EncryptConfirmation
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = action.recover_result(ciphertext, claimed_tag)? {
            return Ok(receipt);
        }
        let input = match action.input() {
            Ok(input) => input,
            Err(_) => {
                return action
                    .recover_result(ciphertext, claimed_tag)?
                    .ok_or(VoiceError::Unauthorized)
            }
        };
        let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
            return Err(VoiceError::Unauthorized);
        };
        let _preparation = action.begin_result_preparation()?;
        action.check_chain_epoch(authority.context.epoch)?;
        self.current(signer, authority)?;
        self.pair_current(flight.handshake, authority, clock)?;
        let pair = &self.confirmations.pairs[&flight.handshake];
        let peer = pair.binding.peer;
        let pending = pair.action.as_ref().ok_or(VoiceError::Reservation)?;
        if pending.serial != flight.action
            || pending
                .draft
                .as_ref()
                .is_none_or(|draft| draft.id != flight.reservation.id)
        {
            return Err(VoiceError::Reservation);
        }
        let kind = pending.kind;
        if kind == EncryptedKind::ConfirmFinish {
            self.promotion_current(flight.handshake)?;
            self.ledger
                .validate_establish(&flight.handshake, authority)?;
        }
        let binding = self.result_binding(flight.handshake, u64::MAX, flight.reservation.expires);
        let mut publication = Some(self.ledger.prepare_sealed_publication(
            flight.reservation.id,
            ciphertext,
            signer,
            authority,
            clock,
        )?);
        // All signature copies and the hash encoding are allocated before either final fence.
        let signed = publication.as_ref().expect("prepared signature").signed();
        let outcome = CryptoOutcome::Sealed {
            message: signed.id(),
            hash: signed.hash(),
        };
        // Compatibility result recovery and ordinary confirmation do not arm this native
        // transition. Only the application-bound original origin may preserve transfer custody.
        let window = if kind == EncryptedKind::ConfirmFinish {
            origin
                .as_ref()
                .map(|origin| {
                    pair.chain
                        .as_ref()
                        .ok_or(VoiceError::Unauthorized)?
                        .prepare_finish_window(origin.actor, authority.context)
                })
                .transpose()?
        } else {
            None
        };
        let mut prepared = PreparedSeal {
            handshake: flight.handshake,
            kind,
            pair_signed: Some(signed.clone()),
            returned: None,
            window,
            identity: crate::voice_v2::chain::FinishIdentity {
                message: signed.id(),
                hash: signed.hash(),
            },
        };
        let mut terminal = Some(TerminalResult { binding, outcome });
        let mut retirement = ConfirmationRetirement::default();
        let confirmations = &mut self.confirmations;
        let result = action.with_result(ciphertext, claimed_tag, || {
            action.check_chain_epoch(authority.context.epoch)?;
            let (own_mono, own_wall) = action.pending_observation();
            let mut publication_result = None;
            if let Some(origin) = origin {
                origin
                    .actor
                    .with_local_origin(peer, authority.context, |wall, mono, permit| {
                        final_owner(wall, mono)?;
                        if !origin.room.lease.is_live() {
                            return Err(VoiceError::Unauthorized);
                        }
                        action.check_pending_at(own_mono, own_wall)?;
                        publication.as_ref().expect("prepared signature").check_at(
                            permit.epoch(),
                            wall,
                            mono,
                        )?;
                        permit.publish(|| {
                            publication_result = Some(prepared.publish(
                                &mut publication,
                                confirmations,
                                &mut retirement,
                                mono,
                            ));
                        })
                    })?;
            } else {
                let wall = clock.now_ms();
                let mono = clock.monotonic_ms();
                action.check_pending_at(own_mono, own_wall)?;
                publication.as_ref().expect("prepared signature").check_at(
                    authority.context.epoch,
                    wall,
                    mono,
                )?;
                final_owner(wall, mono)?;
                publication_result =
                    Some(prepared.publish(&mut publication, confirmations, &mut retirement, mono));
            }
            publication_result.expect("successful original origin observation")?;
            Ok(terminal.take().expect("one terminal publication"))
        });
        drop(publication);
        retirement.finish(self);
        drop((prepared, terminal));
        result
    }
}
