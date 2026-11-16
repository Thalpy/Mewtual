//! Scoped first-media publication after canonical original SDP completion. The trusted
//! native caller must independently bind its actual Session, drained rows and live key;
//! no renderer media status, equal labels or retained history can replace that admission.
use super::*;
use voice_v2::crypto_actions::CryptoAction;
use voice_v2::packets::Authority;
use voice_v2::sdp::{InitialSdpIntent, MediaAdmission, SdpMedia};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Borrow one first-media final opportunity while the sole Actor is exclusively held.
    /// Native's synchronous callback acquires UI/Coordinator/route/original-row gates,
    /// consumes this step, then installs its preallocated exact media claim without an
    /// intervening callback/fallible allocation. The step cannot escape or be reused.
    /// Current admission is distinct from exact authenticated native history recovery.
    ///
    /// ```compile_fail
    /// use catcoms_rt::{CryptoRngCore, MeshTransport};
    /// use catcoms_sync::{ChannelSync, voice_v2::{CallGrant, crypto_actions::CryptoAction,
    ///     sdp::{InitialSdpIntent, SdpMedia}}};
    /// use std::sync::Arc;
    /// fn escape<T: MeshTransport, R: CryptoRngCore>(sync: &mut ChannelSync<T, R>,
    ///     grant: &Arc<CallGrant>, intent: &Arc<InitialSdpIntent>, media: &Arc<SdpMedia>,
    ///     seal: &Arc<CryptoAction>, open: &Arc<CryptoAction>) {
    ///     let mut escaped = None;
    ///     let _ = sync.with_voice_initial_media_admission(grant, intent, media, seal, open,
    ///         |step| { escaped = Some(step); Ok(()) });
    /// }
    /// ```
    pub fn with_voice_initial_media_admission(
        &mut self,
        grant: &Arc<CallGrant>,
        intent: &Arc<InitialSdpIntent>,
        media: &Arc<SdpMedia>,
        seal: &Arc<CryptoAction>,
        open: &Arc<CryptoAction>,
        publish: impl FnOnce(MediaAdmission<'_>) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let room = voice.directory.authority(context.scope)?;
        let participant = voice
            .endpoints
            .participant
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
            || seal.sdp_local() != Some(self.device.device_id())
            || open.sdp_local() != Some(self.device.device_id())
            || !seal.uses_resources(&voice.resources)
            || !open.uses_resources(&voice.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = seal.sdp_peer().ok_or(VoiceError::Unauthorized)?;
        let members = super::ready::ready_member_snapshot(actor, &context, peer, &[])?;
        participant.join.with_initial_media_admission(
            actor,
            &room,
            intent,
            media,
            seal,
            open,
            &Authority {
                context: &context,
                lease: &participant.lease,
                member: &|id| members.contains(&Some(id)),
            },
            |_, _| {
                if participant.lease.is_live() && participant.watch.is_live() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
            publish,
        )
    }
}
