//! First native PC intent on the actual protected Pair. This performs no SDP crypto,
//! packet allocation, PC construction or activation. Those operations require this same
//! intent plus their own native final admission; an RPC receipt cannot reconstruct it.
use super::*;
use crate::voice_v2::chain::sdp_handoff::EffectObserver;
use crate::voice_v2::chain::EstablishedKeyCustody;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::sdp::InitialSdpIntent;

impl LocalJoin {
    /// Only the native original-KeyClaim adapter may call this with its actual funded
    /// phase and admitted SDK obligation. Root independently derives the original Ready,
    /// role and finite time from its Pair. Different generation/phase retries never replace
    /// a possibly admitted intent. Exact recovery may return CLOSED history, not effects.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_initial_sdp_intent(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        key: &EstablishedKeyCustody,
        effect: EffectObserver,
        generation: u64,
        authority: &Authority<'_>,
    ) -> Result<Arc<InitialSdpIntent>, VoiceError> {
        // No new native work is created by historical exact recovery. The adapter still
        // authenticates the original Session/key/storage/SDK identities independently.
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if let Some(intent) = &pair.sdp_intent {
            return if intent.same_preparation(key, &effect, generation) {
                Ok(intent.clone())
            } else {
                Err(VoiceError::RetryMismatch)
            };
        }
        self.owned(authority)?;
        if pair.retired
            || !pair.established
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
            || room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        let Some(successor::Preparation::Transferred { owner, .. }) = &pair.sdp_successor else {
            return Err(VoiceError::Capacity);
        };
        let (wall, mono) = actor.preparation_time()?;
        let metadata = owner.initial_sdp_metadata(wall, mono)?;
        let binding = metadata.original.binding();
        if binding.scope != self.scope
            || binding.epoch != authority.context.epoch
            || binding.local != authority.context.local
            || !binding.call.is_same(authority.lease)
            || binding.handshake != handshake
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = binding.peer;
        // Allocation and every fallible preparation precede Group/Policy. The outer
        // candidate remains alive on refusal/unwind; no last owner drops beneath guards.
        let candidate = InitialSdpIntent::prepare(metadata, effect, generation, key, wall, mono)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("same exclusive Pair");
        actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
            if !room.lease.is_live() || !candidate.current_at(wall, mono) {
                return Err(VoiceError::Unauthorized);
            }
            origin.publish(|| {
                debug_assert!(pair.sdp_intent.is_none());
                pair.sdp_intent = Some(candidate.clone());
            })
        })?;
        Ok(candidate)
    }
}
