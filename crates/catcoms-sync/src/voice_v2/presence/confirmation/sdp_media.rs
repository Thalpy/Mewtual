//! One first-media transition after the original complete SDP round. This does not claim
//! Source/Lower retirement or audible media; native additionally binds original drained
//! rows, UI Session, key/SDK and exact preparation before consuming this borrowed tail.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::sdp::{InitialSdpIntent, MediaAdmission, SdpMedia};

impl LocalJoin {
    #[allow(clippy::too_many_arguments)] // Original independently authenticated owners, not labels.
    pub(crate) fn with_initial_media_admission(
        &self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        intent: &Arc<InitialSdpIntent>,
        media: &Arc<SdpMedia>,
        seal: &Arc<CryptoAction>,
        open: &Arc<CryptoAction>,
        authority: &Authority<'_>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        publish: impl FnOnce(MediaAdmission<'_>) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        if room.scope != self.scope
            || media.scope() != self.scope
            || authority.context.scope != self.scope
            || !self.lease.is_same(authority.lease)
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
            || !media.matches_actions(intent, seal, open)
            || !seal.belongs_to(self.scope, &self.lease)
            || !open.belongs_to(self.scope, &self.lease)
            || self.actions.key_of(seal).is_none()
            || self.actions.key_of(open).is_none()
        {
            return Err(VoiceError::Unauthorized);
        }
        let pair = self
            .confirmations
            .pairs
            .get(&intent.handshake())
            .filter(|pair| {
                pair.sdp_intent
                    .as_ref()
                    .is_some_and(|stored| Arc::ptr_eq(stored, intent))
            })
            .ok_or(VoiceError::Unauthorized)?;
        if pair.binding.peer != media.peer() {
            return Err(VoiceError::Unauthorized);
        }
        let mut origin = Some(actor.prepare_local_origin(pair.binding.peer, authority.context)?);
        let (wall, mono) = origin
            .as_ref()
            .expect("original media origin")
            .observed_time();
        let mut final_owner = Some(final_owner);
        let mut committed = None;
        let result;
        {
            let mut commit = || {
                let result = origin.take().ok_or(VoiceError::Unauthorized)?.with_origin(
                    |wall, mono, origin| {
                        final_owner.take().expect("one original media owner")(wall, mono)?;
                        if !room.lease.is_live() || !media.first_current_at(intent, wall, mono) {
                            return Err(VoiceError::Unauthorized);
                        }
                        let mut admitted = Err(VoiceError::Unauthorized);
                        origin.publish(|| admitted = media.admit())?;
                        admitted
                    },
                );
                committed = Some(result);
                result
            };
            // Native UI/Coordinator/route/row precede Root try Group/Policy. No retained
            // preparation, Clock callback, destructor or async frame moves below them.
            result = publish(MediaAdmission::new(media, intent, wall, mono, &mut commit));
        }
        match committed {
            Some(result) => result, // A later hook error cannot contradict actual admission.
            None => {
                result?;
                Err(VoiceError::Unauthorized)
            }
        }
    }
}
