//! Root derivation prepares immutable native input, not key bytes. The browser keeps its P-256
//! private key; an accepted historical Derived marker cannot recreate a missing local key pair.
use super::*;
use crate::voice_v2::crypto_actions::ActionQueue;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;

pub(crate) struct ProtectedDerivation<'a> {
    pub actor: &'a ActorOwner,
    pub room: &'a RoomAuthority,
    pub handshake: Nonce,
}
pub(crate) struct ProtectedDerivationResult<'a> {
    pub actor: &'a ActorOwner,
    pub room: &'a RoomAuthority,
    pub action: &'a Arc<CryptoAction>,
    pub claimed_tag: [u8; 32],
}

impl LocalJoin {
    /// Publish one protected row-zero action for an actual proven native pair. Cached factory
    /// recovery returns the original allocation even after ACK/compaction; it admits no engine.
    pub(crate) fn prepare_protected_derivation(
        &mut self,
        request: ProtectedDerivation<'_>,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        let ProtectedDerivation {
            actor,
            room,
            handshake,
        } = request;
        if room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        let chain = self.prepare_crypto_chain(handshake, authority, clock.as_ref())?;
        let peer = self.confirmations.pairs[&handshake].binding.peer;
        if let Some(action) = self.actions.get(Key::Derive(handshake)) {
            if !action.uses_chain(&chain.lease()) {
                return Err(VoiceError::Unauthorized);
            }
            let binding = self.chain_binding(handshake, authority);
            let slots = self.ledger.action_slots();
            actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
                final_owner(wall, mono)?;
                if !room.lease.is_live() || !chain.matches(&binding, &self.resources, &slots, mono)
                {
                    return Err(VoiceError::Unauthorized);
                }
                origin.publish(|| ())
            })?;
            return Ok(action);
        }
        let (payload_bytes, engine_bytes) =
            crypto_actions::derive_bytes(self.confirmations.pairs[&handshake].transcript.len())?;
        let (history, budget) = chain.prepare_derive()?;
        let prepared = self.actions.prepare(
            Key::Derive(handshake),
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget,
                history: Some(history),
                payload_bytes,
                engine_bytes,
            },
        )?;
        let pending = self
            .pair_derivation_authority(handshake, authority, clock.clone())?
            .with_pending(chain.lease())?;
        let setup = self.pair_setup(handshake, authority, clock.as_ref())?;
        let (mut issue, inactive) = ActionQueue::build_chain_derive(prepared, setup, pending)?;
        let mut inactive = Some(inactive);
        let mut published = None;
        actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live() {
                return Err(VoiceError::Unauthorized);
            }
            issue.publish_derivation(origin, mono, || {
                published = Some(
                    self.actions
                        .publish_inactive(inactive.take().expect("one derivation publication")),
                );
            })
        })?;
        let action = published.expect("successful native derivation publication");
        chain.register_action(&action);
        Ok(action)
    }

    /// First derivation readiness is admitted by the original actor/session, not a still-live
    /// child alone. History remains metadata and is recovered before consumed pending owners.
    pub(crate) fn commit_protected_derivation_result(
        &mut self,
        request: ProtectedDerivationResult<'_>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        let ProtectedDerivationResult {
            actor,
            room,
            action,
            claimed_tag,
        } = request;
        let Some(Key::Derive(handshake)) = self.actions.key_of(action) else {
            return Err(VoiceError::Unauthorized);
        };
        let chain = self
            .confirmations
            .pairs
            .get(&handshake)
            .and_then(|pair| pair.chain.clone())
            .ok_or(VoiceError::Unauthorized)?;
        if room.scope != self.scope
            || !actor.uses_resources(&self.resources)
            || !action.belongs_to(self.scope, &self.lease)
            || action.operation() != CryptoOperation::Derive
            || !action.uses_chain(&chain.lease())
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = action.recover_result(&[], claimed_tag)? {
            return Ok(receipt);
        }
        let _preparation = action.begin_result_preparation()?;
        action.check_chain_epoch(authority.context.epoch)?;
        self.pair_current(handshake, authority, clock)?;
        let peer = self.confirmations.pairs[&handshake].binding.peer;
        let chain_binding = self.chain_binding(handshake, authority);
        let slots = self.ledger.action_slots();
        let mut binding = self.result_binding(handshake, u64::MAX, u64::MAX);
        binding.attempt = None;
        let mut terminal = Some(TerminalResult {
            binding,
            outcome: CryptoOutcome::Derived,
        });
        let result = action.with_result(&[], claimed_tag, || {
            let (own_mono, own_wall) = action.pending_observation();
            actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
                final_owner(wall, mono)?;
                // Root supplies this original Runtime clock to the pair and action. Its final
                // native observation must also fence deadline expiry after preparation's own
                // samples; a clock callback can advance time without revoking a lease.
                if !room.lease.is_live()
                    || !chain.matches(&chain_binding, &self.resources, &slots, mono)
                {
                    return Err(VoiceError::Unauthorized);
                }
                action.check_chain_epoch(origin.epoch())?;
                action.check_pending_at(own_mono, own_wall)?;
                origin.publish(|| ())
            })?;
            // Native admission is the linearization point. Engine remains held while this
            // infallible tail moves only prebuilt historical metadata, never key/RTC authority.
            Ok(terminal.take().expect("one derivation result publication"))
        });
        drop(terminal);
        result
    }
}
