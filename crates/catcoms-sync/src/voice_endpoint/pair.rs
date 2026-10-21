//! Trusted production pair selection. Renderer IDs are requests, never watch or pair authority.
use super::*;
use voice_v2::packets::Authority;
use voice_v2::presence::{PairReceipt, PairSelector, ProtectedPair};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Admit/recover a protected native pair selected by immutable remote join/key and epoch.
    /// New evidence comes only from this exact participant's implicit watcher. A lost reply
    /// retries the original selector, including when discovery expired or a newer join arrived.
    /// The fixed receipt grants no key, engine, packet, source, media or RTC authority.
    pub fn prepare_voice_pair(
        &mut self,
        grant: &Arc<CallGrant>,
        selector: PairSelector,
    ) -> Result<PairReceipt, VoiceError> {
        self.validate_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        if selector.epoch != context.epoch {
            return Err(VoiceError::Unauthorized);
        }
        let room = self.voice_room_authority(context.scope.room)?;
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let participant = voice
            .endpoints
            .participant
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let observer = voice
            .endpoints
            .observers
            .get(&participant.watch.id)
            .ok_or(VoiceError::Unauthorized)?;
        if !observer.lease.is_same(&participant.watch.lease) {
            return Err(VoiceError::Unauthorized);
        }
        // Do not call a caller-supplied watcher, or require renewed discovery before cached
        // pair recovery. Both the exact watcher allocation and selected record are rechecked.
        let fresh = observer.watcher.peer_for_pair(selector.peer);
        let call = participant.lease.clone();
        let watch = participant.watch.lease.clone();
        let authority = Authority {
            context: &context,
            lease: &call,
            member: &|id| members.contains(&id),
        };
        let (receipt, published) = participant.join.admit_protected_pair(
            ProtectedPair {
                actor,
                room: &room,
                watch: &watch,
                selector,
            },
            fresh.as_ref(),
            &authority,
            clock.as_ref(),
            |_, _| {
                if call.is_live() && watch.is_live() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )?;
        // A revision-driven caller may ensure the same pair repeatedly. Historical recovery
        // must not generate a new wake that sends that driver around its own retry loop.
        if published {
            voice.endpoints.changed();
        }
        Ok(receipt)
    }
}
