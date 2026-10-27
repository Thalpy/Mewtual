//! LOW Ready borrows only fixed history bytes. Taking an action row here would steal one
//! of the two original SDP positions after the ten worst-case confirmation rows.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::ready::admission::ReadyAdmission;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::{Body, Envelope, ReadyHeader, SignedObject, UnverifiedReady};

impl PlanBinding {
    fn matches_low_ready(&self, scope: Scope, issuer: DeviceId, h: &ReadyHeader) -> bool {
        self.low
            && self.value.live()
            && self.transition.epoch_is_live()
            && !self.successor_closed.load(Ordering::Acquire)
            && scope == self.scope
            && issuer == self.peer
            && h.epoch == self.epoch
            && h.target == self.local
            && h.source_join == self.peer_join
            && h.target_join == self.local_join
            && h.source_key_ref == self.peer_key
            && h.target_key_ref == self.local_key
            && h.handshake == self.handshake
            && self.transition.original_identity().is_some_and(|finish| {
                finish.message == h.finish_message && finish.hash == h.finish_object_hash
            })
    }
    fn matches_low_ready_peek(&self, p: &UnverifiedReady, mono: u64) -> bool {
        let h = &p.header;
        self.low
            && p.sequence == 1
            && self.live()
            && !self.successor_closed.load(Ordering::Acquire)
            && self.transition.epoch_is_live()
            && self
                .transition
                .original_until()
                .is_some_and(|until| mono >= until.saturating_sub(AWAIT_READY_MS) && mono < until)
            && p.scope == self.scope
            && p.issuer == self.peer
            && h.epoch == self.epoch
            && h.target == self.local
            && h.source_join == self.peer_join
            && h.target_join == self.local_join
            && h.source_key_ref == self.peer_key
            && h.target_key_ref == self.local_key
            && h.handshake == self.handshake
            && self.transition.original_identity().is_some_and(|finish| {
                finish.message == h.finish_message && finish.hash == h.finish_object_hash
            })
    }
}
impl Workflow {
    /// Header selection lends this ORIGINAL lane only. Forged Ready still pays normal
    /// requester/process work and must pass signatures, routing and final native admission.
    pub(crate) fn lends_low_ready_verification(
        &self,
        requester: DeviceId,
        frame: &[u8],
        mono: u64,
    ) -> bool {
        let Ok(p) = Envelope::peek_ready(frame) else {
            return false;
        };
        self.is_live()
            && self.binding.matches_low_ready_peek(&p, mono)
            && ((p.envelope_kind == 1 && requester == p.issuer)
                || (p.envelope_kind == 3 && p.header.helpers.contains(&Some(requester))))
    }
    pub(in crate::voice_v2) fn low_confirmation_sources_retired(&self) -> bool {
        self.binding.low && self.binding.low_ready.load(Ordering::Acquire)
    }
}
impl PreparedChain {
    fn low_ready_fee(&self) -> Result<Credit, VoiceError> {
        ByteBudget::Reserved(self.history.bytes.clone()).charge(
            self.binding.resources(),
            self.binding.scope.group,
            LOW_READY_RECEIPT_BYTES,
            ResourceKind::Retained,
        )
    }
    pub(in crate::voice_v2) fn low_ready_current_at(&self, mono: u64) -> bool {
        self.binding.low
            && self.binding.live()
            && !self.binding.successor_closed.load(Ordering::Acquire)
            && self.binding.transition.epoch_is_live()
            && self
                .binding
                .transition
                .original_until()
                .is_some_and(|until| mono >= until.saturating_sub(AWAIT_READY_MS) && mono < until)
    }
    pub(in crate::voice_v2) fn low_ready_accepted(&self) -> bool {
        self.binding.low && self.binding.low_ready.load(Ordering::Acquire)
    }
    /// Called only in the infallible exact native Ready publication tail. No lock,
    /// allocation or actual capacity owner is destroyed under Group/Policy.
    pub(in crate::voice_v2) fn publish_low_ready(&self) {
        debug_assert!(self.binding.low);
        self.binding.low_ready.store(true, Ordering::Release);
    }
    pub(in crate::voice_v2) fn prepare_low_ready_admission(
        &self,
        actor: &ActorOwner,
        object: &SignedObject,
        room: &RoomAuthority,
    ) -> Result<Arc<ReadyAdmission>, VoiceError> {
        let Body::PairReadyRound {
            sequence: 1,
            header,
            profile: crate::voice_v2::SdpProfile::CompleteSdpV1,
        } = object.body()
        else {
            return Err(VoiceError::Unauthorized);
        };
        let (wall, mono) = actor.preparation_time()?;
        if !self.low_ready_current_at(mono)
            || !self
                .binding
                .matches_low_ready(object.scope(), object.issuer(), header)
        {
            return Err(VoiceError::Unauthorized);
        }
        // A plain fee retains the original fixed reserve, never Workflow/phase custody.
        // No Slot/Key/row is issued: both original SDP action positions stay available.
        let fee = self.low_ready_fee()?;
        let effect = actor.effect(object.issuer(), self.binding.local, object.expires())?;
        ReadyAdmission::prepare(
            self.original_observation(),
            object,
            self.binding
                .transition
                .original_until()
                .ok_or(VoiceError::Unauthorized)?,
            wall,
            mono,
            room.clone(),
            effect,
            actor.preparation_clock(),
            fee,
        )
    }
    pub(in crate::voice_v2) fn ready_low_confirmation_binding(&self) -> Option<&Binding> {
        self.low_ready_accepted().then_some(&self.binding.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_ready_fee_with_ten_published_original_history_rows_preserves_both_sdp_slots() {
        use crate::voice_v2::chain::tests::{binding, prepare_chain};
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::with_limit(ROWS));
        let chain = prepare_chain(pool.clone(), binding(5), slots.clone()).unwrap();
        let workflow = chain.workflow().unwrap();
        // Capacity regression, not synthetic protocol readiness: publish ten ORIGINAL
        // history rows/credits/slots through their actual issue fence. Native confirmation
        // and causal Ready are independently exercised by the LocalJoin tests.
        let rows: Vec<_> = (0..10)
            .map(|row| {
                let PreparedHistory {
                    record,
                    slot,
                    issue,
                } = chain
                    .history
                    .prepare_row(row, Key::Decrypt(row as u64), &workflow)
                    .unwrap();
                issue.publish(|| ()).unwrap();
                (record, slot)
            })
            .collect();
        let control = workflow.confirmation_control();
        let prepared = control.prepare_sdp_capacity().unwrap();
        let fee = chain.low_ready_fee().unwrap();
        assert_eq!(chain.history.state.lock().unwrap().issued.count_ones(), 10);
        let group = chain.binding.scope.group;
        let crypto: Vec<_> = (0..3)
            .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
            .collect();
        let pressure = pool
            .charge(
                group,
                crate::voice_v2::resources::GROUP_BYTES - pool.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
        let ticket = control.transfer_sdp_capacity(prepared).unwrap();
        let exchange = crate::voice_v2::negotiation::Exchange::from_transferred(ticket, Vec::new());
        assert!(slots.charge(1).is_err());
        drop((
            exchange, rows, fee, control, workflow, chain, crypto, pressure,
        ));
        assert_eq!(pool.counts(), (0, 0));
        drop(slots.charge(ROWS).unwrap());
    }
}
