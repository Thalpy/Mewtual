//! High-side inert SDP custody. This is not causal Ready or permission to expose SDP/RTC.
//! LOW extraction must wait for authenticated Ready: closing LOW's old chain here would
//! revoke its original Finish Source before HIGH could receive/open it and become ready.
use super::*;
use crate::voice_v2::chain::PreparedSdpCapacity;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::negotiation::{Exchange, SdpSuccessor};
use crate::voice_v2::runtime::ActorOwner;

pub(super) enum Preparation {
    Reserved {
        capacity: PreparedSdpCapacity,
        helpers: Vec<DeviceId>,
    },
    Transferred {
        owner: Arc<Exchange>,
        // Held outside Producer/Exchange to avoid a credited-cache self-custody cycle.
        ready: Option<Box<crate::voice_v2::ready::producer::CacheCapacity>>,
    },
}
impl Preparation {
    pub(super) fn close(&self) {
        if let Self::Transferred { owner, .. } = self {
            // An exported observation may still retain the allocation. Pair destruction
            // must close its unused lanes now rather than wait for that historical handle.
            owner.close();
        }
    }
}
pub(super) fn collect(preparation: &mut Option<Preparation>, mono: u64) {
    match preparation.as_ref() {
        Some(Preparation::Reserved { capacity, .. }) if !capacity.current_at(mono) => {
            // Outside native Group/Policy/Engine guards: actual capacity destruction may
            // reenter Pool and the unique old Workflow preparation latch.
            drop(preparation.take());
        }
        Some(Preparation::Transferred { owner, .. }) if owner.collect_pending_successor(mono) => {
            // Release native closed custody too: keeping this Arc forever would occupy the
            // sole registry through the join. External observations/actual owners still
            // delay real last-drop. Original Finish/chain stamps prevent reconstruction.
            drop(preparation.take());
        }
        _ => {}
    }
}
impl Pair {
    pub(in crate::voice_v2::presence) fn protected_mode(&self) -> bool {
        self.protected_epoch.is_some()
    }
}

/// Paired endpoint plus an optional immutable helper choice. Some([None, None]) is an
/// already frozen direct choice; None alone permits first-time connected selection.
pub(crate) type HighReadyHelpers = (DeviceId, Option<[Option<DeviceId>; 2]>);

impl LocalJoin {
    /// Fixed identity lookup for native membership snapshots. This is metadata only;
    /// every preparation still validates role, epoch, Finish and the actual final owners.
    pub(crate) fn protected_ready_peer(&self, handshake: Nonce) -> Result<DeviceId, VoiceError> {
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if pair.protected_epoch.is_none() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(pair.binding.peer)
    }
    /// Metadata-only native scheduling observation. Recover an existing immutable helper
    /// choice before looking at today's routes: churn cannot rewrite an admitted intent.
    /// The actual preparation still rechecks finite Finish, original owners and final gates.
    pub(crate) fn high_ready_helpers(
        &self,
        handshake: Nonce,
        authority: &Authority<'_>,
    ) -> Result<HighReadyHelpers, VoiceError> {
        self.owned(authority)?;
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if pair.low
            || !pair.established
            || pair.retired
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
        {
            return Err(VoiceError::Unauthorized);
        }
        let frozen = pair.sdp_successor.as_ref().map(|preparation| {
            let helpers = match preparation {
                Preparation::Reserved { helpers, .. } => helpers.as_slice(),
                Preparation::Transferred { owner, .. } => owner.helpers(),
            };
            [helpers.first().copied(), helpers.get(1).copied()]
        });
        Ok((pair.binding.peer, frozen))
    }
    /// Borrow one original Ready plus its private Source provenance. The actual Root
    /// namespace is mandatory; a standalone signed-cache preparation cannot export work.
    pub(crate) fn ready_source_activity(
        &self,
        handshake: Nonce,
        namespace: &crate::voice_v2::negotiation::namespace::ReceiptNamespace,
        authority: &Authority<'_>,
    ) -> Result<(&SignedObject, crate::voice_v2::source::SourceActivity), VoiceError> {
        let activity = self.source_activity(authority)?;
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if pair.low
            || pair.retired
            || !pair.established
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
        {
            return Err(VoiceError::Unauthorized);
        }
        let Some(Preparation::Transferred {
            ready: Some(ready), ..
        }) = &pair.sdp_successor
        else {
            return Err(VoiceError::Unauthorized);
        };
        let (object, ready) = ready.source_authority(namespace)?;
        Ok((object, activity.with_ready(ready)))
    }
    pub(crate) fn has_first_offer(&self, handshake: Nonce) -> bool {
        self.confirmations
            .pairs
            .get(&handshake)
            .is_some_and(|pair| pair.first_offer.is_some())
    }
    /// Capacity-only query used while installing the native receiver, before a packet exists.
    pub(crate) fn has_ready_position(
        &self,
        handshake: Nonce,
        namespace: &crate::voice_v2::negotiation::namespace::ReceiptNamespace,
    ) -> bool {
        match self
            .confirmations
            .pairs
            .get(&handshake)
            .and_then(|pair| pair.sdp_successor.as_ref())
        {
            Some(Preparation::Transferred {
                ready: Some(ready), ..
            }) => ready.has_position(namespace),
            _ => false,
        }
    }
    pub(crate) fn has_ready_offer_position(
        &self,
        object: &SignedObject,
        namespace: &crate::voice_v2::negotiation::namespace::ReceiptNamespace,
    ) -> bool {
        let Body::Encrypted { header, .. } = object.body() else {
            return false;
        };
        match self
            .confirmations
            .pairs
            .get(&header.handshake)
            .and_then(|pair| pair.sdp_successor.as_ref())
        {
            Some(Preparation::Transferred {
                owner,
                ready: Some(ready),
            }) => {
                owner.matches_first_offer(object.scope(), object.issuer(), header)
                    && ready.has_position(namespace)
            }
            _ => false,
        }
    }
    /// Test-only observation of the actual installed native publication, never a factory.
    #[cfg(test)]
    pub(crate) fn sdp_round_for_test(&self, handshake: Nonce) -> Option<crate::voice_v2::SdpRound> {
        match self
            .confirmations
            .pairs
            .get(&handshake)?
            .sdp_successor
            .as_ref()?
        {
            Preparation::Transferred { owner, .. } => owner.sdp_round(),
            _ => None,
        }
    }
    pub(crate) fn attach_ready_position(
        &mut self,
        handshake: Nonce,
        position: crate::voice_v2::negotiation::namespace::ReceiptPosition,
    ) -> Result<(), VoiceError> {
        match self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .and_then(|pair| pair.sdp_successor.as_mut())
        {
            Some(Preparation::Transferred {
                ready: Some(ready), ..
            }) => ready.attach_position(position),
            _ => Err(VoiceError::Unauthorized),
        }
    }
    /// Install the original HIGH bounded first-Offer inbox and commit the exact immutable
    /// Ready marker. The native KeyClaim worker schedules this before Source publication;
    /// LOW causal admission and recoverable Offer-token publication still precede activation.
    /// No packet is exported here and generic SDP/crypto/RTC lenders remain inactive.
    ///
    /// All byte/table positions are acquired before the final native fence. A refusal keeps
    /// the SAME inbox, stamp and signature. The marker has no separate origin debit; eventual
    /// Source admission owns the one original packet debit. `final_owner` is pure as below.
    #[allow(clippy::too_many_arguments)]
    pub fn install_high_ready_receive(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        helpers: &[DeviceId],
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        self.prepare_high_ready_artifact(
            actor,
            room,
            handshake,
            helpers,
            signer,
            authority,
            clock,
            rng,
            |_, _| Ok(()),
        )?;
        let binding = self.chain_binding(handshake, authority);
        let Some(Preparation::Transferred {
            owner,
            ready: Some(ready),
        }) = &mut self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("original pair")
            .sdp_successor
        else {
            return Err(VoiceError::Unauthorized);
        };
        let (wall, mono) = actor.preparation_time()?;
        ready.install_receive(self.ledger.packet_slots(), mono)?;
        let publication = ready.prepare_publication(wall, mono)?;
        // The original Pair retains both inbox and Exchange across all error/drop paths.
        // No credited last owner is moved into a closure destroyed beneath native guards.
        let mut publication = Some(publication);
        actor.with_local_origin(binding.peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live()
                || !owner.successor_current_at(&binding, &self.resources, mono)
                || !origin.contains_members(owner.helpers())
            {
                return Err(VoiceError::Unauthorized);
            }
            ready.prepared_at(owner, wall, mono)?;
            publication
                .as_ref()
                .expect("unpublished marker")
                .check_at(wall, mono)?;
            origin.publish(|| {
                publication
                    .take()
                    .expect("one marker publication")
                    .publish()
            })
        })
    }

    /// Prepare one immutable HIGH Ready signature in native Pair custody. This deliberately
    /// exposes no SignedObject or Source activity: the native KeyClaim worker must also
    /// install receive custody, commit the marker and queue that exact original Source.
    /// Never expose its signer as unrestricted renderer IPC.
    ///
    /// Refusal retains the same paid producer, first ID/timestamps and prepared signature.
    /// Final UI/session admission runs after signer callbacks under the actual native view.
    /// `final_owner` runs under Group -> Policy and must perform only pure owner checks:
    /// no Clock, blocking locks, Engine/runtime reentry or actual owner destruction.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_high_ready_artifact(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        helpers: &[DeviceId],
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        let successor = self.prepare_high_sdp_successor(
            actor,
            room,
            handshake,
            helpers,
            authority,
            clock,
            |_, _| Ok(()),
        )?;
        if !successor.confirmation_drained() {
            return Err(VoiceError::Capacity);
        }
        let public_key = *self
            .announcement
            .as_ref()
            .ok_or(VoiceError::Reservation)?
            .public_key();
        let binding = self.chain_binding(handshake, authority);
        let Some(Preparation::Transferred {
            owner,
            ready: Some(ready),
        }) = &mut self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("original pair")
            .sdp_successor
        else {
            return Err(VoiceError::Unauthorized);
        };
        ready.prepare_artifact(signer, public_key, actor, rng)?;
        actor.with_local_origin(binding.peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live()
                || !owner.successor_current_at(&binding, &self.resources, mono)
                || !origin.contains_members(owner.helpers())
            {
                return Err(VoiceError::Unauthorized);
            }
            ready.prepared_at(owner, wall, mono)?;
            // Preparation acknowledgement only. No original packet quota or Ready marker
            // is published, and no receiver/crypto/SDK/RTC permission is manufactured.
            origin.publish(|| ())
        })
    }

    /// Retain the HIGH endpoint's original inert SDP successor after actual native Finish-open.
    /// Exact retries keep the same reservation/lane/slots even across final-fence or registry
    /// Capacity. No Ready, new deadline, source attempt, crypto/SDP or RTC authority is created.
    ///
    /// This capacity-only substep is used by the native KeyClaim worker's HIGH preparation.
    /// The higher-level factory also installs the actual receive promise and original marker;
    /// the worker separately queues its paid Ready Source. LOW prepays and transfers through
    /// its own native Ready-admission path. None of these steps grants browser SDP/RTC use.
    ///
    /// `final_owner` runs under the ACTUAL Group -> Policy guards. It must only check the
    /// original native grant/UI generation, without Clock sampling, runtime/ledger/Engine
    /// reentry, blocking, or arbitrary callbacks. It grants no successor activation authority.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_high_sdp_successor(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        helpers: &[DeviceId],
        authority: &Authority<'_>,
        clock: &dyn Clock,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<SdpSuccessor, VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        let pair = &self.confirmations.pairs[&handshake];
        // Orientation and actual native Finish-open precede every successor payment. A
        // low local-seal marker is intentionally insufficient, even if it says Established.
        if pair.low
            || !pair.established
            || pair.protected_epoch != Some(authority.context.epoch)
            || !room.lease.is_live()
            || room.scope != self.scope
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        let chain = pair.chain.clone().ok_or(VoiceError::Unauthorized)?;
        let binding = self.chain_binding(handshake, authority);
        if !chain.high_successor_current(&binding, &self.resources, clock.monotonic_ms())
            || !chain.successor_actor_matches(actor, authority.context)?
        {
            return Err(VoiceError::Unauthorized);
        }
        if helpers.len() > 2 {
            return Err(VoiceError::Bounds);
        }
        let mut helpers = helpers.to_vec();
        helpers.sort();
        if helpers.windows(2).any(|pair| pair[0] == pair[1])
            || helpers
                .iter()
                .any(|helper| *helper == self.local || *helper == binding.peer)
        {
            return Err(VoiceError::Malformed);
        }
        if (!helpers.is_empty() && (!authority.context.indirect || self.scope.policy.is_none()))
            || helpers.iter().any(|helper| !(authority.member)(*helper))
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(preparation) = &self.confirmations.pairs[&handshake].sdp_successor {
            let same = match preparation {
                Preparation::Reserved {
                    helpers: original, ..
                } => *original == helpers,
                Preparation::Transferred { owner, .. } => owner.same_helpers(&helpers),
            };
            if !same {
                return Err(VoiceError::Scope);
            }
        }
        // Classify immutable helper identity before touching any new ordinary origin rows.
        actor.prepare_ready_members(binding.peer, &helpers, authority.context)?;
        if self.confirmations.pairs[&handshake].sdp_successor.is_none() {
            let workflow = chain.workflow()?;
            let capacity = workflow.confirmation_control().prepare_sdp_capacity()?;
            self.confirmations
                .pairs
                .get_mut(&handshake)
                .expect("current pair")
                .sdp_successor = Some(Preparation::Reserved { capacity, helpers });
        }
        let control = if matches!(
            self.confirmations.pairs[&handshake].sdp_successor,
            Some(Preparation::Reserved { .. })
        ) {
            Some(chain.workflow()?.confirmation_control())
        } else {
            None
        };
        let preparation = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("current pair")
            .sdp_successor
            .take()
            .expect("retained preparation");
        let preparation = match preparation {
            Preparation::Reserved { capacity, helpers } => {
                match control
                    .expect("original open workflow")
                    .transfer_sdp_capacity(capacity)
                {
                    Ok(ticket) => Preparation::Transferred {
                        owner: Exchange::from_transferred(ticket, helpers),
                        ready: None,
                    },
                    Err((error, capacity)) => {
                        self.confirmations
                            .pairs
                            .get_mut(&handshake)
                            .expect("current pair")
                            .sdp_successor = Some(Preparation::Reserved { capacity, helpers });
                        return Err(error);
                    }
                }
            }
            preparation @ Preparation::Transferred { .. } => preparation,
        };
        // The native holder is populated BEFORE either fallible final admission or registry
        // publication. A lost/refused reply cannot abandon the already extracted lane.
        self.confirmations
            .pairs
            .get_mut(&handshake)
            .expect("current pair")
            .sdp_successor = Some(preparation);
        // No old cache may pin the retired Workflow until signed expiry. Actual native
        // result/SDK/Source/Lower owners remain independent and still delay real drainage.
        self.ledger.retire_high_confirmation_cache(&chain)?;
        let Some(Preparation::Transferred { owner, ready }) = &mut self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("current pair")
            .sdp_successor
        else {
            unreachable!()
        };
        let owner = owner.clone();
        if ready.is_none() {
            // Retain immediately: later final-fence/registry refusals must not lose the
            // sole issued Producer or let a retry create another independent allowance.
            *ready = Some(owner.prepare_ready_cache()?);
        }
        let ready = ready
            .as_ref()
            .expect("original retained Ready cache capacity");
        // All three old Proof attempts may have occupied the original outgoing table.
        // Cache retirement above returns them to these SAME moved parents before splitting.
        owner.prepare_packet_claims()?;
        actor.with_local_origin(binding.peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live()
                || !owner.successor_current_at(&binding, &self.resources, mono)
                || !ready.current_for(&owner)
                || !origin.contains_members(owner.helpers())
            {
                return Err(VoiceError::Unauthorized);
            }
            origin.publish(|| ())
        })?;
        self.resources.register_exchange(self.scope.group, &owner)?;
        Ok(SdpSuccessor { owner })
    }
}
