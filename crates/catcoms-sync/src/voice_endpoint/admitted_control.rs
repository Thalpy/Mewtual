//! Trusted Root protocol-support APIs. Endpoint evidence is selected from the
//! actual original Watcher; the distinct direct-helper API uses a real route.
use super::*;
use voice_v2::admitted_control::{
    self as control, Binding, DirectHelperSupportObservation, DirectHelperSupportProbe,
    EndpointSupportObservation, EndpointSupportQuery, SupportQueryIdentity,
};
use voice_v2::call_activation::ActivationObserver;

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Recover/create this original's current endpoint protocol query. Source
    /// capacity does not remint it: quiet scheduling retries the SAME bytes.
    pub fn ensure_admitted_endpoint_support(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        peer: DeviceId,
    ) -> Result<EndpointSupportQuery, VoiceError> {
        self.admitted_support_query(grant, original, peer, false, None)
            .map(control::Queries::endpoint)
    }
    /// Explicit q2 replacement. q1 is negatively closed before q2 can publish,
    /// even when source publication subsequently waits for bounded capacity.
    pub fn refresh_admitted_endpoint_support(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        peer: DeviceId,
        previous: &SupportQueryIdentity,
    ) -> Result<EndpointSupportQuery, VoiceError> {
        self.admitted_support_query(grant, original, peer, false, Some(previous))
            .map(control::Queries::endpoint)
    }
    /// Direct connected helper probe; neither argument nor output substitutes for
    /// endpoint FreshPeer evidence. The remote helper need not join any room.
    pub fn ensure_admitted_direct_helper_support(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        helper: DeviceId,
    ) -> Result<DirectHelperSupportProbe, VoiceError> {
        self.admitted_support_query(grant, original, helper, true, None)
            .map(control::Queries::helper)
    }
    pub fn refresh_admitted_direct_helper_support(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        helper: DeviceId,
        previous: &SupportQueryIdentity,
    ) -> Result<DirectHelperSupportProbe, VoiceError> {
        self.admitted_support_query(grant, original, helper, true, Some(previous))
            .map(control::Queries::helper)
    }
    #[allow(clippy::too_many_arguments)]
    fn admitted_support_query(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        peer: DeviceId,
        helper: bool,
        previous: Option<&SupportQueryIdentity>,
    ) -> Result<control::Owner, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        self.prune_voice_routes();
        if !original.matches_grant(grant) {
            return Err(VoiceError::Unauthorized);
        }
        let context = admitted_presence::current(
            self.voice_runtime
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?,
            &self.blob_fetch_instance,
            original,
        )?;
        if peer == context.local {
            return Err(VoiceError::Unauthorized);
        }
        if previous.is_none() {
            if let Some(queries) = self
                .voice_runtime
                .as_ref()
                .and_then(|voice| voice.endpoints.activation.as_ref())
                .and_then(|held| held.presence.as_ref())
                .and_then(|presence| presence.get().queries())
            {
                if let Some(existing) = queries.existing(peer, helper)? {
                    return Ok(existing);
                }
            }
        }
        let room = self.voice_room_authority(context.scope.room)?;
        // Capture actual transport callbacks before entering any native fences.
        let binding = if helper {
            let route = self
                .admitted_bound_device_route(peer, &context)?
                .ok_or(VoiceError::Unauthorized)?;
            let resources = self
                .voice_runtime
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?
                .resources
                .clone();
            let payment = resources.charge(
                context.scope.group,
                voice_v2::ROUTE_METADATA_BYTES,
                ResourceKind::Retained,
            )?;
            Binding::DirectHelper(self.capture_voice_reply_route(route, Arc::new(payment))?)
        } else {
            let presence = self
                .voice_runtime
                .as_ref()
                .and_then(|voice| voice.endpoints.activation.as_ref())
                .and_then(|held| held.presence.as_ref())
                .ok_or(VoiceError::Unauthorized)?
                .get();
            Binding::Endpoint(presence.query_peer(peer).ok_or(VoiceError::Unauthorized)?)
        };
        let clock = self.voice_v2_clock();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let presence = voice
            .endpoints
            .activation
            .as_mut()
            .and_then(|held| held.presence.as_mut())
            .ok_or(VoiceError::Unauthorized)?
            .get_mut();
        let key = presence.key().clone();
        let queries = presence.prepare_queries()?;
        let ordinal = queries.next_ordinal(peer, previous)?;
        actor.prepare_control_members(context.local, peer, &context)?;
        let prepared = queries.prepare(
            actor,
            original,
            &key,
            &context,
            room,
            binding,
            ordinal,
            &self.device,
            &mut self.rng,
            &voice.resources,
            clock.clone(),
            peer,
        )?;
        // Signer/RNG/Clock are callbacks. Repeat the exact current Root/grant/
        // directory binding after them, then use a final no-callback transaction.
        admitted_presence::current(voice, &self.blob_fetch_instance, original)?;
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        let presence = voice
            .endpoints
            .activation
            .as_ref()
            .and_then(|held| held.presence.as_ref())
            .ok_or(VoiceError::Unauthorized)?
            .get();
        let queries = presence.queries().ok_or(VoiceError::Unauthorized)?;
        let old = actor.try_with_current_members(&context, |members| {
            if !original.is_active() || !members(peer) || presence.key() != &key {
                return Err(VoiceError::Unauthorized);
            }
            queries.install_at(&prepared, previous, wall, mono)
        })?;
        // q1 is already negative. Wake outside table/Group/Policy before any q2
        // Source publication. A lost reply can recover q2 through ensure.
        if let Some(old) = old {
            old.notify();
            drop(old);
        }
        let ledger = actor.control()?;
        match ledger.queue_query(actor, &prepared, &context) {
            Ok(()) | Err(VoiceError::Capacity | VoiceError::RateLimited) => {}
            Err(error) => return Err(error),
        }
        Ok(prepared)
    }
    pub fn admitted_endpoint_support_observation(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        query: &EndpointSupportQuery,
    ) -> Result<EndpointSupportObservation, VoiceError> {
        self.admitted_support_observation(grant, original, query.identity(), false)
            .map(|features| {
                features.map_or(EndpointSupportObservation::Unknown, |features| {
                    EndpointSupportObservation::Observed { features }
                })
            })
    }
    pub fn admitted_direct_helper_support_observation(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        query: &DirectHelperSupportProbe,
    ) -> Result<DirectHelperSupportObservation, VoiceError> {
        self.admitted_support_observation(grant, original, query.identity(), true)
            .map(|features| {
                features.map_or(DirectHelperSupportObservation::Unknown, |features| {
                    DirectHelperSupportObservation::Observed { features }
                })
            })
    }
    fn admitted_support_observation(
        &mut self,
        grant: &Arc<CallGrant>,
        original: &ActivationObserver,
        query: &SupportQueryIdentity,
        helper: bool,
    ) -> Result<Option<u16>, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        if !original.matches_grant(grant) {
            return Err(VoiceError::Unauthorized);
        }
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let context = admitted_presence::current(voice, &self.blob_fetch_instance, original)?;
        let queries = voice
            .endpoints
            .activation
            .as_ref()
            .and_then(|held| held.presence.as_ref())
            .and_then(|presence| presence.get().queries())
            .ok_or(VoiceError::Unauthorized)?;
        let owner = queries.current(query)?;
        let key = voice
            .endpoints
            .activation
            .as_ref()
            .and_then(|held| held.presence.as_ref())
            .ok_or(VoiceError::Unauthorized)?
            .get()
            .key();
        if !owner.matches_original(original, key) {
            return Err(VoiceError::Unauthorized);
        }
        let same = queries
            .existing(owner.object().metadata().target(), helper)?
            .ok_or(VoiceError::Unauthorized)?;
        if !same.same(&owner) {
            return Err(VoiceError::RetryMismatch);
        }
        let (wall, mono) = owner.samples();
        voice
            .current
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .try_with_current_members(&context, |members| {
                if !original.is_active() || !members(owner.object().metadata().target()) {
                    return Err(VoiceError::Unauthorized);
                }
                Ok(owner.features_at(wall, mono))
            })
    }
    pub(super) fn pump_admitted_support_queries(&mut self) -> Result<(), VoiceError> {
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let Some(held) = voice.endpoints.activation.as_ref() else {
            return Ok(());
        };
        let context =
            admitted_presence::current(voice, &self.blob_fetch_instance, held.original())?;
        let Some(queries) = held
            .presence
            .as_ref()
            .and_then(|presence| presence.get().queries())
        else {
            return Ok(());
        };
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let ledger = actor.control()?;
        for owner in queries.pending() {
            if let Err(error) = ledger.queue_query(actor, &owner, &context) {
                if !matches!(
                    error,
                    VoiceError::Capacity | VoiceError::RateLimited | VoiceError::Unauthorized
                ) {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    pub(super) fn admitted_control_helpers(
        &self,
        scope: Scope,
        wall: u64,
        mono: u64,
        skip: DeviceId,
        after: Option<DeviceId>,
    ) -> [Option<control::Owner>; 2] {
        let Some(voice) = self.voice_runtime.as_ref() else {
            return [None, None];
        };
        let Some(held) = voice.endpoints.activation.as_ref() else {
            return [None, None];
        };
        if held.original().scope() != scope
            || admitted_presence::current(voice, &self.blob_fetch_instance, held.original())
                .is_err()
        {
            return [None, None];
        }
        held.presence
            .as_ref()
            .and_then(|presence| presence.get().queries())
            .map_or([None, None], |queries| {
                queries.compatible_helpers(wall, mono, skip, after)
            })
    }
    pub(super) fn admit_admitted_control(
        &mut self,
        ticket: &voice_v2::runtime::IngressTicket,
        envelope: &control::wire::Envelope<'_>,
        context: &voice_v2::Context,
        room: voice_v2::helper::RoomAuthority,
    ) -> Result<control::Receipt, VoiceError> {
        use control::{Object, ProtocolOperation};
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let ledger = actor.control()?;
        let m = envelope.object().metadata();
        actor.prepare_control_members(m.issuer(), m.target(), context)?;
        match m.operation() {
            ProtocolOperation::Query => {
                let cached = ledger.cached(m, envelope.object().hash(), false)?;
                let (owner, cached) = match cached {
                    Some(owner) => (owner, true),
                    None => (
                        ledger.prepare_response(
                            actor,
                            envelope.object(),
                            context,
                            room,
                            &self.device,
                            &mut self.rng,
                        )?,
                        false,
                    ),
                };
                ledger.admit(actor, ticket, envelope, context, &owner, cached)
            }
            ProtocolOperation::Support { .. } => {
                let held = voice
                    .endpoints
                    .activation
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?;
                admitted_presence::current(voice, &self.blob_fetch_instance, held.original())?;
                let owner = held
                    .presence
                    .as_ref()
                    .and_then(|presence| presence.get().queries())
                    .and_then(|queries| queries.response_target(m))
                    .ok_or(VoiceError::RetryMismatch)?;
                // The query's original fee already pays one bounded Support. A
                // missing/late query never allocates an owner or signs a reply.
                let mut support = Some(Object::copy(envelope.object()));
                actor.with_control_origin(ticket, envelope, context, |wall, mono, origin| {
                    owner.accept_at(&mut support, wall, mono, |apply| origin.publish(apply))
                })?;
                // Wake source cancellation only after all original native/state
                // guards have gone. The accepted observation remains current.
                owner.notify();
                Ok(control::Receipt::new(owner))
            }
            ProtocolOperation::Other => Err(VoiceError::Unauthorized),
        }
    }
}
