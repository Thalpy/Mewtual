//! Current pair provenance for a scoped native input read. Actual Engine and Root view
//! admission happens later, after the trusted native callback has entered its own fence.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::sdp::{InitialSdpIntent, InputCurrent};

impl LocalJoin {
    #[allow(clippy::too_many_arguments)] // Independent original Root, room, action and native-call provenance.
    pub(crate) fn prepare_sdp_input_current<'a>(
        &'a self,
        actor: &'a ActorOwner,
        room: &'a RoomAuthority,
        action: &Arc<CryptoAction>,
        intent: &'a Arc<InitialSdpIntent>,
        context: &'a Context,
        call: &'a CallLease,
        watch: &'a crate::voice_v2::WatchLease,
    ) -> Result<InputCurrent<'a>, VoiceError> {
        if room.scope != self.scope
            || context.scope != self.scope
            || !actor.uses_resources(&self.resources)
            || !action.belongs_to(self.scope, &self.lease)
            || !call.is_same(&self.lease)
            || !action.uses_sdp_intent(intent)
            || !matches!(
                self.actions.key_of(action),
                Some(Key::SdpSeal(..) | Key::SdpOpen(_))
            )
        {
            return Err(VoiceError::Unauthorized);
        }
        let binding = action.sdp_binding().ok_or(VoiceError::Unauthorized)?;
        let pair = self
            .confirmations
            .pairs
            .get(&binding.handshake())
            .filter(|pair| {
                pair.sdp_intent
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, intent))
            })
            .ok_or(VoiceError::Unauthorized)?;
        Ok(InputCurrent {
            origin: actor.prepare_local_origin(pair.binding.peer, context)?,
            room,
            intent,
            call,
            watch,
            pair: &pair.pin,
        })
    }
}
