//! One endpoint admission namespace for public controls and protected encrypted confirmation.
//! Live kind28 dispatch remains disabled while SDP transfer and production IPC are incomplete.
use super::*;
use voice_v2::chain::ConfirmationAdmission;
use voice_v2::negotiation::admission::SdpAdmission;
use voice_v2::ReadyAdmission;

#[cfg(test)]
mod layout_tests {
    use super::*;
    #[test]
    fn protected_identity_shared_node_allowance_includes_keys_values_and_internal_edges() {
        use std::mem::size_of;
        // Rust's B=6 BTree nodes hold11 keys/values and up to12 child edges. The32-byte
        // header allowance exceeds parent/index/length metadata and alignment. This is a
        // structural growth guard, excluding allocator overhead; per-record fees jointly
        // cover shared nodes. Never describe a512B per-record allowance as a whole node.
        let bytes =
            32 + 11 * (size_of::<Key>() + size_of::<EncryptedReceipt>()) + 12 * size_of::<usize>();
        assert!(
            bytes <= 1024,
            "shared protected identity node grew to {bytes}"
        );
    }
}

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
#[derive(Clone)]
pub(super) enum EncryptedReceipt {
    Confirmation(ConfirmationReceipt),
    FirstOffer(Arc<SdpAdmission>),
    Answer(Arc<SdpAdmission>),
    Ready(Arc<ReadyAdmission>),
}
impl EncryptedReceipt {
    pub(super) fn retain(&self) -> bool {
        match self {
            Self::Confirmation(receipt) => receipt.retain(),
            Self::FirstOffer(receipt) | Self::Answer(receipt) => {
                receipt.retains_identity() || Arc::strong_count(receipt) > 1
            }
            Self::Ready(receipt) => receipt.retains_identity() || Arc::strong_count(receipt) > 1,
        }
    }
    pub(super) fn object_hash(&self) -> [u8; 32] {
        match self {
            Self::Confirmation(receipt) => receipt.object_hash(),
            Self::FirstOffer(receipt) | Self::Answer(receipt) => receipt.hash,
            Self::Ready(receipt) => receipt.hash,
        }
    }
    fn same_call(&self, call: &CallLease) -> bool {
        match self {
            Self::Confirmation(receipt) => receipt.0.admission.same_call(call),
            Self::FirstOffer(receipt) | Self::Answer(receipt) => receipt.same_call(call),
            Self::Ready(receipt) => receipt.same_call(call),
        }
    }
}
pub(crate) enum EndpointReceipt {
    Public(PublicReceipt),
    Confirmation(ConfirmationReceipt),
    FirstOffer(Arc<SdpAdmission>),
    Answer(Arc<SdpAdmission>),
    Ready(Arc<ReadyAdmission>),
}
impl EndpointReceipt {
    pub(crate) fn authorized(&self) -> bool {
        match self {
            Self::Public(receipt) => receipt.authorized(),
            Self::Confirmation(receipt) => receipt.0.admission.authorized(),
            Self::FirstOffer(receipt) | Self::Answer(receipt) => receipt.authorized(),
            Self::Ready(receipt) => receipt.authorized(),
        }
    }
    pub(crate) fn object_hash(&self) -> [u8; 32] {
        match self {
            Self::Public(receipt) => receipt.object_hash(),
            Self::Confirmation(receipt) => receipt.object_hash(),
            Self::FirstOffer(receipt) | Self::Answer(receipt) => receipt.hash,
            Self::Ready(receipt) => receipt.hash,
        }
    }
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        match self {
            Self::Public(receipt) => receipt.authorized_at(wall, mono),
            Self::FirstOffer(receipt) | Self::Answer(receipt) => receipt.authorized_at(wall, mono),
            Self::Ready(receipt) => receipt.authorized_at(wall, mono),
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
            Self::FirstOffer(receipt) | Self::Answer(receipt) => receipt.revoked().await,
            Self::Ready(receipt) => receipt.revoked().await,
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
        // Ready shares fixed identity history with encrypted input, but is never generic
        // public queue admission: its original native LOW pair must accept causally.
        if !matches!(
            envelope.object().body(),
            Body::Encrypted { .. } | Body::PairReady { .. } | Body::PairReadyRound { .. }
        ) {
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
        let participant = voice
            .endpoints
            .participant
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        // Only the exact original native Ready round can spend its prepaid namespace
        // position. A delayed legacy/wrong-round offer gets no full-table exemption.
        let reserved_offer = participant
            .join
            .has_ready_offer_position(envelope.object(), voice.endpoints.public.namespace());
        let reserved_answer = participant
            .join
            .has_answer_position(envelope.object(), voice.endpoints.public.namespace());
        if existing.is_none()
            && voice.endpoints.public.full()
            && !reserved_offer
            && !reserved_answer
        {
            return Err(VoiceError::Capacity);
        }
        if existing
            .as_ref()
            .is_some_and(|receipt| !receipt.same_call(&participant.lease))
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
        let final_owner = |_, _| {
            if parent.strong_count() > 0
                && call.is_live()
                && watch.is_live()
                && room.lease.is_live()
            {
                Ok(())
            } else {
                Err(VoiceError::Unauthorized)
            }
        };
        if matches!(
            envelope.object().body(),
            Body::PairReady { .. } | Body::PairReadyRound { .. }
        ) {
            let admission = participant.join.admit_protected_ready(
                actor,
                &ticket,
                &envelope,
                &room,
                &authority,
                final_owner,
            )?;
            if let Some(existing) = existing {
                let EncryptedReceipt::Ready(existing) = existing else {
                    return Err(VoiceError::RetryMismatch);
                };
                if !Arc::ptr_eq(&existing, &admission) {
                    return Err(VoiceError::RetryMismatch);
                }
                return Ok(EndpointReceipt::Ready(existing));
            }
            assert!(voice
                .endpoints
                .public
                .encrypted
                .insert(key, EncryptedReceipt::Ready(admission.clone()))
                .is_none());
            voice.endpoints.changed();
            return Ok(EndpointReceipt::Ready(admission));
        }
        if matches!(envelope.object().body(), Body::Encrypted { header, .. } if header.kind.is_offer())
        {
            let admission = participant.join.admit_protected_first_offer(
                actor,
                &ticket,
                &envelope,
                &room,
                &authority,
                clock,
                final_owner,
            )?;
            if let Some(existing) = existing {
                let EncryptedReceipt::FirstOffer(existing) = existing else {
                    return Err(VoiceError::RetryMismatch);
                };
                if !Arc::ptr_eq(&existing, &admission) {
                    return Err(VoiceError::RetryMismatch);
                }
                return Ok(EndpointReceipt::FirstOffer(existing));
            }
            assert!(voice
                .endpoints
                .public
                .encrypted
                .insert(key, EncryptedReceipt::FirstOffer(admission.clone()))
                .is_none());
            voice.endpoints.changed();
            return Ok(EndpointReceipt::FirstOffer(admission));
        }
        if matches!(envelope.object().body(), Body::Encrypted { header, .. } if matches!(header.kind, voice_v2::EncryptedKind::AnswerRound(_)))
        {
            let admission = participant.join.admit_protected_answer(
                actor,
                &ticket,
                &envelope,
                &room,
                voice.endpoints.public.namespace(),
                &authority,
                clock,
                final_owner,
            )?;
            if let Some(existing) = existing {
                let EncryptedReceipt::Answer(existing) = existing else {
                    return Err(VoiceError::RetryMismatch);
                };
                if !Arc::ptr_eq(&existing, &admission) {
                    return Err(VoiceError::RetryMismatch);
                }
                return Ok(EndpointReceipt::Answer(existing));
            }
            assert!(voice
                .endpoints
                .public
                .encrypted
                .insert(key, EncryptedReceipt::Answer(admission.clone()))
                .is_none());
            voice.endpoints.changed();
            return Ok(EndpointReceipt::Answer(admission));
        }
        let existing = match existing {
            Some(EncryptedReceipt::Confirmation(receipt)) => Some(receipt),
            None => None,
            _ => return Err(VoiceError::RetryMismatch),
        };
        let admission = participant.join.admit_protected_confirmation(
            actor,
            &ticket,
            &envelope,
            &room,
            &authority,
            clock,
            final_owner,
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
                .insert(key, EncryptedReceipt::Confirmation(receipt.clone()))
                .is_none(),
            "native endpoint identity uniqueness"
        );
        voice.endpoints.changed();
        Ok(EndpointReceipt::Confirmation(receipt))
    }
}
