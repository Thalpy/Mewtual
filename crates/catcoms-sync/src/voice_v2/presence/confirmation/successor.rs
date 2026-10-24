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
    Transferred(Arc<Exchange>),
}
impl Preparation {
    pub(super) fn close(&self) {
        if let Self::Transferred(owner) = self {
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
        Some(Preparation::Transferred(owner)) if owner.collect_pending_successor(mono) => {
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

impl LocalJoin {
    /// Retain the HIGH endpoint's original inert SDP successor after actual native Finish-open.
    /// Exact retries keep the same reservation/lane/slots even across final-fence or registry
    /// Capacity. No Ready, new deadline, source attempt, crypto/SDP or RTC authority is created.
    ///
    /// This native-only factory is deliberately not scheduled by application maintenance yet.
    /// Causal Ready publication/admission and explicit browser key transfer must be implemented
    /// before scheduling it; the low-side equivalent does not exist. The separate32KiB Ready
    /// byte envelope is prepaid, but helper origin rows and the independently owned/quotaed
    /// Ready Source must still be installed before any readiness can be promised.
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
                Preparation::Transferred(owner) => owner.same_helpers(&helpers),
            };
            if !same {
                return Err(VoiceError::Scope);
            }
        } else {
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
                    Ok(ticket) => {
                        Preparation::Transferred(Exchange::from_transferred(ticket, helpers))
                    }
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
            preparation @ Preparation::Transferred(_) => preparation,
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
        let Some(Preparation::Transferred(owner)) =
            &self.confirmations.pairs[&handshake].sdp_successor
        else {
            unreachable!()
        };
        let owner = owner.clone();
        actor.with_local_origin(binding.peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !room.lease.is_live()
                || !owner.successor_current_at(&binding, &self.resources, mono)
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
