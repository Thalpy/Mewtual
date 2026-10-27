//! A signature may be computed before native admission, but it is not a source-visible packet.
//! Keep that inert exact artifact in its original reserved row so a local final-fence Capacity
//! refusal cannot trigger another signature or accept changed ciphertext on retry.
use super::*;

// One body storage slot, with an explicit publication bit; adding a second optional signed
// object would enlarge every fixed outgoing row even when only one stage can be occupied.
pub(super) enum SealedState {
    Unsigned,
    Prepared(SignedObject),
    Published(SignedObject),
}
impl SealedState {
    fn signed(&self) -> Option<&SignedObject> {
        match self {
            Self::Unsigned => None,
            Self::Prepared(signed) | Self::Published(signed) => Some(signed),
        }
    }
    pub(super) fn published(&self) -> Option<&SignedObject> {
        match self {
            Self::Published(signed) => Some(signed),
            Self::Unsigned | Self::Prepared(_) => None,
        }
    }
}

/// Exclusive synchronous ledger loan. Its return copy is already allocated; publication only
/// changes an existing stage and revokes pending crypto. It cannot cross an await or permit a
/// second operation to mutate the row. On refusal leave this owner OUTSIDE native guards, then
/// drop it after those guards; its original row keeps the inert signature for exact retry.
pub(in crate::voice_v2) struct PreparedSealedPublication<'a> {
    ledger: &'a mut PacketLedger,
    id: MessageId,
    signed: SignedObject,
    epoch: u64,
    local: CallLease,
    pair: CallLease,
}
impl PreparedSealedPublication<'_> {
    /// A caller may prepare its native pair's separate retained copy before the final fence.
    /// This is private preparation, never a grant to enqueue the artifact as a source packet.
    pub(in crate::voice_v2) fn signed(&self) -> &SignedObject {
        &self.signed
    }
    /// Pure ownership/time check using final native-origin samples. No Clock/member/signer
    /// callback or additional ledger operation may intervene between this check and publish.
    pub(in crate::voice_v2) fn check_at(
        &self,
        epoch: u64,
        wall: u64,
        mono: u64,
    ) -> Result<(), VoiceError> {
        let outgoing = self
            .ledger
            .outgoing
            .get(&self.id)
            .expect("exclusive prepared row");
        if epoch != self.epoch
            || !self.local.is_live()
            || !self.pair.is_live()
            || !outgoing.live.is_live()
            || outgoing
                .exchange
                .as_ref()
                .is_some_and(|exchange| !exchange.is_live())
            || outgoing
                .sdp_binding
                .as_ref()
                .is_some_and(|binding| !binding.current_at(wall, mono))
        {
            return Err(VoiceError::Unauthorized);
        }
        self.signed.check_freshness(wall)?;
        if outgoing.sealed.published().is_none() {
            // Existing published-byte retries are historical compatibility data. Their later
            // source_authority check still rejects a retired chain; do not turn this pending
            // first-publication gate into permission to send or a cached-history restriction.
            if !outgoing.crypto.0.is_live()
                || outgoing
                    .chain
                    .as_ref()
                    .is_some_and(|chain| !chain.is_live())
            {
                return Err(VoiceError::Unauthorized);
            }
            // A late reservation may have construction time left after its original pair or
            // SDP exchange has expired. First publication can promote confirmation state, so
            // source-side refusal alone is too late. Use all original bounds at this fence.
            let deadline = outgoing
                .construction
                .min(outgoing.source_deadline)
                .min(self.ledger.sdp_deadline(outgoing.exchange.as_ref())?);
            if mono >= deadline {
                return Err(VoiceError::Reservation);
            }
        }
        Ok(())
    }
    /// Infallible apply after check_at under the caller's final fence. Trusted short lease/watch
    /// notification is allowed; no application callback, allocation or owner destruction occurs.
    pub(in crate::voice_v2) fn publish(self) -> SignedObject {
        self.publish_with(|_, signed| signed)
    }
    /// Publish another already-prepared part of the SAME synchronous native transaction after
    /// releasing the exclusive row loan. The callback obeys the caller's infallible final fence;
    /// it may only move prepaid state, revoke short gates and defer retired owners to outer scope.
    pub(in crate::voice_v2) fn publish_with<T>(
        self,
        apply: impl FnOnce(&mut PacketLedger, SignedObject) -> T,
    ) -> T {
        let outgoing = self
            .ledger
            .outgoing
            .get_mut(&self.id)
            .expect("exclusive prepared row");
        if outgoing.sealed.published().is_none() {
            let SealedState::Prepared(signed) =
                std::mem::replace(&mut outgoing.sealed, SealedState::Unsigned)
            else {
                unreachable!("prepared signed stage")
            };
            outgoing.sealed = SealedState::Published(signed);
            outgoing.crypto.0.revoke();
        }
        apply(self.ledger, self.signed)
    }
}

impl PacketLedger {
    /// Prepare the exact first signature/copies without publishing or consuming crypto authority.
    /// Any existing prepared/published stage rejects changed ciphertext BEFORE another signer
    /// call. The same reserved row/byte charge covers its single stored signed body throughout.
    pub(in crate::voice_v2) fn prepare_sealed_publication(
        &mut self,
        id: MessageId,
        ciphertext: &[u8],
        signer: &(impl VoiceSigner + ?Sized),
        authority: &Authority<'_>,
        clock: &dyn Clock,
    ) -> Result<PreparedSealedPublication<'_>, VoiceError> {
        self.prune(clock);
        self.current(authority)?;
        let outgoing = self.outgoing.get(&id).ok_or(VoiceError::Reservation)?;
        if !outgoing.live.is_live()
            || outgoing
                .exchange
                .as_ref()
                .is_some_and(|exchange| !exchange.is_live())
        {
            return Err(VoiceError::Unauthorized);
        }
        freshness(
            outgoing.draft.issued,
            outgoing.draft.expires,
            clock.now_ms(),
        )?;
        let pair = self
            .pair(&outgoing.draft.header.handshake, authority)?
            .live
            .clone();
        if signer.voice_public_key()? != outgoing.draft.public_key {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(signed) = outgoing.sealed.signed() {
            let Body::Encrypted {
                ciphertext: original,
                ..
            } = signed.body()
            else {
                unreachable!()
            };
            if ciphertext != original {
                return Err(VoiceError::RetryMismatch);
            }
        } else {
            let deadline = outgoing
                .construction
                .min(outgoing.source_deadline)
                .min(self.sdp_deadline(outgoing.exchange.as_ref())?);
            if clock.monotonic_ms() >= deadline {
                return Err(VoiceError::Reservation);
            }
            if ciphertext.len() != outgoing.ciphertext_bytes {
                return Err(VoiceError::Bounds);
            }
            self.pair(&outgoing.draft.header.handshake, authority)?;
            if !outgoing.live.is_live()
                || !outgoing.crypto.0.is_live()
                || outgoing
                    .chain
                    .as_ref()
                    .is_some_and(|chain| !chain.is_live())
                || outgoing
                    .exchange
                    .as_ref()
                    .is_some_and(|exchange| !exchange.is_live())
            {
                return Err(VoiceError::Unauthorized);
            }
            let signed = outgoing.draft.seal(signer, ciphertext)?;
            self.outgoing.get_mut(&id).expect("checked row").sealed = SealedState::Prepared(signed);
        }
        // Clone before native Group/Policy, not during publication. This copy has exactly the
        // same lifetime/peak as the old submit return copy; future IPC copies need own custody.
        let signed = self.outgoing[&id]
            .sealed
            .signed()
            .expect("signed stage")
            .clone();
        Ok(PreparedSealedPublication {
            ledger: self,
            id,
            signed,
            epoch: authority.context.epoch,
            local: authority.lease.clone(),
            pair,
        })
    }
}
