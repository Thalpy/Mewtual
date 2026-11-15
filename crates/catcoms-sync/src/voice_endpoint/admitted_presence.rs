//! Root-current admitted discovery. Native chooses and freezes the canonical join
//! and on-curve key before calling this trusted API; raw point bytes here are not
//! an independent SDK/curve-validation proof or renderer signing permission.
use super::*;
use voice_v2::admitted_presence::{AdmittedPresence, AdmittedPresenceReceipt, PresenceStorage};
use voice_v2::call_activation::ActivationObserver;
use voice_v2::presence::FreshPeer;
use voice_v2::source::AdmittedDiscoveryActivity;

/// Rebuild authority on EVERY positive Root turn. Equal public labels, retained
/// receipt metadata or a live lease alone cannot substitute for the exact winner.
pub(super) fn current(
    voice: &crate::voice_runtime::VoiceRuntime,
    instance: &Arc<()>,
    original: &ActivationObserver,
) -> Result<voice_v2::Context, VoiceError> {
    let held = voice
        .endpoints
        .activation
        .as_ref()
        .ok_or(VoiceError::Unauthorized)?;
    if !held.original().is_same(original)
        || !original.is_active()
        || !original.belongs_to(&voice.resources, instance)
    {
        return Err(VoiceError::Unauthorized);
    }
    let grant = original.grant().ok_or(VoiceError::Unauthorized)?;
    if !original.matches_grant(&grant) {
        return Err(VoiceError::Unauthorized);
    }
    let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
    let context = actor.source_context(original.scope())?;
    voice.directory.validate_call_grant(actor, &grant)?;
    actor.try_with_current_members(&context, |_| Ok(()))?;
    if !original.is_active() {
        return Err(VoiceError::Unauthorized);
    }
    Ok(context)
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Publish the exact selected NativeActive original's first immutable key.
    /// Exact retries return historical installation evidence without allocating,
    /// consuming RNG or changing deadlines. Different join/key retries refuse.
    /// This starts shared discovery only, with no packet ledger/round/RTC producer.
    pub fn start_admitted_voice_presence(
        &mut self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        join: voice_v2::Nonce,
        point: [u8; 65],
    ) -> Result<AdmittedPresenceReceipt, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        if !original.matches_grant(&grant) {
            return Err(VoiceError::Unauthorized);
        }
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        let context = current(voice, &self.blob_fetch_instance, &original)?;
        let key = JoinKey::new(join, point)?;
        let held = voice
            .endpoints
            .activation
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if let Some(presence) = &held.presence {
            return if presence.get().key() == &key {
                Ok(AdmittedPresenceReceipt::installed(&original, &key))
            } else {
                Err(VoiceError::RetryMismatch)
            };
        }
        let id = voice
            .endpoints
            .serial
            .checked_add(1)
            .ok_or(VoiceError::Reservation)?;
        // All funding precedes Box/Watcher allocation and issuer registration.
        let funding = PresenceStorage::reserve(&original)?;
        let local = AdmittedPresence::new(
            held,
            &context,
            id,
            key.clone(),
            &funding,
            voice.resources.clone(),
            &mut self.rng,
        )?;
        // RNG and issuer implementations are callbacks. Revalidate the SAME
        // residence after them, before installing or issuing accepted history.
        current(voice, &self.blob_fetch_instance, &original)?;
        let storage = PresenceStorage::new(local, funding);
        voice
            .endpoints
            .activation
            .as_mut()
            .expect("same Root residence")
            .presence = Some(storage);
        voice.endpoints.serial = id;
        voice.endpoints.changed();
        Ok(AdmittedPresenceReceipt::installed(&original, &key))
    }

    /// Shared challenge-backed room liveness from the ORIGINAL implicit watcher.
    /// These observations intentionally carry no admitted-support/profile proof.
    pub(super) fn admitted_voice_presence_peers(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
    ) -> Result<Vec<FreshPeer>, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        if !original.matches_grant(grant) {
            return Err(VoiceError::Unauthorized);
        }
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        let context = current(voice, &self.blob_fetch_instance, original)?;
        let peers = voice
            .endpoints
            .activation
            .as_mut()
            .and_then(|held| held.presence.as_mut())
            .ok_or(VoiceError::Unauthorized)?
            .get_mut()
            .peers(&context, &|id| members.contains(&id), clock.as_ref())?;
        current(voice, &self.blob_fetch_instance, original)?;
        Ok(peers)
    }

    /// Paid native observation: the original permit follows the returned vector,
    /// command and waiter through actual output return. It proves liveness only.
    pub fn admitted_voice_presence_peer_snapshot(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        permit: voice_v2::presence::PeerSnapshotPermit,
    ) -> voice_v2::presence::PeerSnapshot {
        let result = (|| {
            let context = self.voice_v2_context(grant.scope().room)?;
            if permit.scope() != grant.scope()
                || context.scope != permit.scope()
                || !self
                    .voice_runtime
                    .as_ref()
                    .and_then(|voice| voice.current.as_ref())
                    .is_some_and(|actor| actor.uses_resources(permit.resources()))
            {
                return Err(VoiceError::Unauthorized);
            }
            self.admitted_voice_presence_peers(grant, original)
                .map(|peers| (context.epoch, peers))
        })();
        permit.complete(result)
    }

    pub(super) fn queue_admitted_voice_discovery(
        &mut self,
        object: voice_v2::SignedObject,
        activity: AdmittedDiscoveryActivity,
    ) -> Result<bool, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let held = voice
            .endpoints
            .activation
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        current(voice, &self.blob_fetch_instance, held.original())?;
        if !activity.is_original(held.original()) {
            return Err(VoiceError::Unauthorized);
        }
        let room = self.voice_room_authority(object.scope().room)?;
        self.voice_transport_actor()?
            .source()?
            .admit_admitted_discovery(self.voice_transport_actor()?, object, activity, room)
    }
}
