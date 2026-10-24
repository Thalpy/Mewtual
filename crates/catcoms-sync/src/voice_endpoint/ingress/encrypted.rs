//! One endpoint admission namespace for public controls and protected encrypted confirmation.
//! Live kind28 dispatch remains disabled while SDP transfer and production IPC are incomplete.
use super::*;
use voice_v2::chain::ConfirmationAdmission;

struct ProtectedPacket {
    admission: Arc<ConfirmationAdmission>,
}
#[derive(Clone)]
pub(crate) struct ConfirmationReceipt(Arc<ProtectedPacket>);
impl ConfirmationReceipt {
    pub(super) fn retain(&self) -> bool {
        self.0.admission.retains_identity() || Arc::strong_count(&self.0) > 1
    }
    pub(super) fn object_hash(&self) -> [u8; 32] {
        self.0.admission.hash
    }
}
pub(crate) enum EndpointReceipt {
    Public(PublicReceipt),
    Confirmation(ConfirmationReceipt),
}
impl EndpointReceipt {
    pub(crate) fn authorized(&self) -> bool {
        match self {
            Self::Public(receipt) => receipt.authorized(),
            Self::Confirmation(receipt) => receipt.0.admission.authorized(),
        }
    }
    pub(crate) fn object_hash(&self) -> [u8; 32] {
        match self {
            Self::Public(receipt) => receipt.object_hash(),
            Self::Confirmation(receipt) => receipt.object_hash(),
        }
    }
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        match self {
            Self::Public(receipt) => receipt.authorized_at(wall, mono),
            // Epoch currency is ALSO required from the original Root reply route. This fixed
            // metadata cannot inspect or recreate Root; direct recovery repeats current Context.
            Self::Confirmation(receipt) => {
                receipt
                    .0
                    .admission
                    .authorized_at(receipt.0.admission.epoch(), wall, mono)
            }
        }
    }
    pub(crate) async fn revoked(&self) {
        match self {
            Self::Public(receipt) => receipt.revoked().await,
            Self::Confirmation(receipt) => receipt.0.admission.revoked().await,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub(crate) fn admit_voice_endpoint(
        &mut self,
        ticket: IngressTicket,
        envelope: Envelope,
        context: &Context,
        room: RoomAuthority,
    ) -> Result<EndpointReceipt, VoiceError> {
        // A signed Ready is NOT a queued public notification. Until its dedicated exact
        // protected native admission exists, refuse it here: a generic PublicReceipt would
        // falsely acknowledge readiness before validating/advancing the original pair.
        if matches!(envelope.object().body(), Body::PairReady { .. }) {
            return Err(VoiceError::Unauthorized);
        }
        if !matches!(envelope.object().body(), Body::Encrypted { .. }) {
            return self
                .admit_voice_endpoint_public(ticket, envelope, context, room)
                .map(EndpointReceipt::Public);
        }
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        voice.endpoints.public.prune(clock.as_ref());
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        if actor.authorize_verified_route(&ticket, &envelope, context)?
            != voice_v2::Disposition::Deliver
        {
            return Err(VoiceError::Unauthorized);
        }
        // Shared immutable IDs are classified before the native ledger can propose a replay
        // bit, token, candidate or action. Separate maps are only work representations.
        voice.endpoints.public.check_identity(envelope.object())?;
        let key = (envelope.object().issuer(), envelope.object().id());
        let existing = voice.endpoints.public.encrypted.get(&key).cloned();
        if existing.is_none() && voice.endpoints.public.full() {
            return Err(VoiceError::Capacity);
        }
        let participant = voice
            .endpoints
            .participant
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        if existing
            .as_ref()
            .is_some_and(|receipt| !receipt.0.admission.same_call(&participant.lease))
        {
            return Err(VoiceError::Unauthorized);
        }
        if room.scope != context.scope
            || participant.watch.scope != context.scope
            || !participant
                .grant
                .upgrade()
                .is_some_and(|grant| grant.scope() == context.scope)
        {
            return Err(VoiceError::Unauthorized);
        }
        // Retain no strong call grant while observing final consent. Lost native ownership
        // synchronously revokes the original call; a replacement cannot authorize this receipt.
        let parent = participant.grant.clone();
        let call = participant.lease.clone();
        let watch = participant.watch.clone();
        let authority = Authority {
            context,
            lease: &participant.lease,
            member: &|id| members.contains(&id),
        };
        let admission = participant.join.admit_protected_confirmation(
            actor,
            &ticket,
            &envelope,
            &room,
            &authority,
            clock,
            |_, _| {
                if parent.strong_count() > 0
                    && call.is_live()
                    && watch.is_live()
                    && room.lease.is_live()
                {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )?;
        if let Some(existing) = existing {
            if !Arc::ptr_eq(&existing.0.admission, &admission) {
                return Err(VoiceError::RetryMismatch);
            }
            return Ok(EndpointReceipt::Confirmation(existing));
        }
        // The Hello row's fixed receipt portion pays this small wrapper/Arc and map entry.
        // No recoverable allocation/queue operation follows the origin transaction.
        let receipt = ConfirmationReceipt(Arc::new(ProtectedPacket { admission }));
        assert!(
            voice
                .endpoints
                .public
                .encrypted
                .insert(key, receipt.clone())
                .is_none(),
            "native endpoint identity uniqueness"
        );
        voice.endpoints.changed();
        Ok(EndpointReceipt::Confirmation(receipt))
    }
}
