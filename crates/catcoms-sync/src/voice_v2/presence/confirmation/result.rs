//! First protected plaintext effects use the original Root actor, not only its child call lease.
//! Session revocation flips an atomic before it can acquire Policy and revoke those children.
//! Prepare first, then hold Engine and borrow the short nonblocking Group/Policy publication fence.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;
mod sealed;
pub(crate) use sealed::ProtectedSealedResult;
pub(super) use sealed::SealedResult;

pub(crate) struct ProtectedOpenedResult<'a> {
    pub actor: &'a ActorOwner,
    pub room: &'a RoomAuthority,
    pub action: &'a Arc<CryptoAction>,
    pub plaintext: &'a [u8],
    pub claimed_tag: [u8; 32],
}

impl LocalJoin {
    /// Recognize the actual retained action allocation before recovering history or admitting a
    /// first effect. History never restores a token, engine, pair or RTC capability. Native IPC
    /// recovery after the participant disappears must use its separately registered action.
    pub(crate) fn commit_protected_confirmation_result(
        &mut self,
        request: ProtectedOpenedResult<'_>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        let ProtectedOpenedResult {
            actor,
            room,
            action,
            plaintext,
            claimed_tag,
        } = request;
        if room.scope != self.scope
            || !actor.uses_resources(&self.resources)
            || !action.belongs_to(self.scope, &self.lease)
            || action.operation() != CryptoOperation::DecryptConfirmation
            || !matches!(self.actions.key_of(action), Some(Key::Decrypt(_)))
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = action.recover_result(plaintext, claimed_tag)? {
            return Ok(receipt);
        }
        let input = match action.input() {
            Ok(input) => input,
            Err(_) => {
                return action
                    .recover_result(plaintext, claimed_tag)?
                    .ok_or(VoiceError::Unauthorized)
            }
        };
        let CryptoInput::DecryptConfirmation(packet) = input.as_input() else {
            return Err(VoiceError::Unauthorized);
        };
        let peer = packet.issuer;
        let _preparation = action.begin_result_preparation()?;
        action.check_chain_epoch(authority.context.epoch)?;
        // These owners deliberately outlive BOTH the result closure and its Engine guard.
        // A refused origin must not drop the prepared replacement attempt while Policy is held;
        // successful application also retains the consumed token/slot and old promotion rows.
        // Pruning/member retirement may destroy sibling packet owners. Perform all such
        // housekeeping before Engine as well as before Group/Policy. If settlement wins while
        // preparing, with_result refuses without applying this private plan.
        let mut prepared = Some(self.prepare_confirmation_terminal(
            OpenedFlight {
                token: packet.token,
                aad: packet.aad(),
                sequence: packet.header.sequence,
                ciphertext: packet.ciphertext(),
                plaintext,
            },
            authority,
            clock,
            rng,
        )?);
        let finish = if matches!(
            prepared.as_ref().expect("prepared transition").change,
            PreparedConfirmation::Finish
        ) {
            let original = &prepared.as_ref().expect("prepared Finish").prepared;
            let chain = self.confirmations.pairs[&original.header.handshake]
                .chain
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?;
            Some((
                chain.prepare_finish_window(actor, authority.context)?,
                crate::voice_v2::chain::FinishIdentity {
                    message: original.message,
                    hash: original.hash,
                },
            ))
        } else {
            None
        };
        let mut retirement = ConfirmationRetirement::default();
        let result = action.with_result(plaintext, claimed_tag, || {
            action.check_chain_epoch(authority.context.epoch)?;
            // This sample uses the action's original clock. Actor time is sampled LAST, outside
            // Group/Policy; repeat all original pure gates after that last injected callback.
            let (own_mono, own_wall) = action.pending_observation();
            let mut terminal = None;
            actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
                final_owner(wall, mono)?;
                if !room.lease.is_live() {
                    return Err(VoiceError::Unauthorized);
                }
                action.check_pending_at(own_mono, own_wall)?;
                prepared
                    .as_ref()
                    .expect("prepared plaintext transition")
                    .prepared
                    .check_at(origin.epoch(), wall, mono)?;
                // Publish repeats the actual original session/actor atomics. A window CAS
                // refusal is captured BEFORE any token/result/promotion effect; origin has no
                // quota debit. No Clock, signer, RNG or Engine acquisition follows that CAS.
                origin.publish(|| {
                    terminal = Some((|| {
                        if let Some((window, identity)) = &finish {
                            window.publish_at(mono, *identity)?;
                        }
                        Ok(self.apply_confirmation_terminal(
                            prepared.take().expect("one plaintext publication"),
                            &mut retirement,
                        ))
                    })());
                })
            })?;
            terminal.expect("successful original origin observation")
        });
        retirement.finish(self);
        drop(prepared);
        result
    }
}
