//! Native Finish history and bounded custody between confirmation and later SDP preparation.
//! This does not reserve SDP, issue Ready, renew an old action or grant RTC authority. The
//! original variable Workflow may survive its first deadline ONLY for affine phase transfer.
use super::*;
use crate::voice_v2::runtime::{ActorOwner, EpochObservation};
use crate::voice_v2::{Context, MessageId};
use std::sync::atomic::AtomicU64;
use std::sync::OnceLock;

const OPEN: u64 = 0;
const CLOSED: u64 = u64::MAX;
// A bounded best-effort drain/preparation interval, separate from every old 15-second
// confirmation operation. Two actual Lower requests and their retained completions may
// outlive that first interval. Permanent pressure can still prevent later SDP readiness.
pub(in crate::voice_v2) const AWAIT_READY_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_v2) struct FinishIdentity {
    pub message: MessageId,
    pub hash: [u8; 32],
}
struct FinishRecord {
    identity: FinishIdentity,
    until: u64,
}

/// Read-only native history, never Ready, a current grant, a transfer ticket or RTC authority.
/// It survives closure so a lost reply cannot invent another Finish or extend its first window.
pub struct FinishEvidence {
    message: MessageId,
    hash: [u8; 32],
    until: u64,
}
redacted_debug!(FinishEvidence);
impl FinishEvidence {
    pub fn message(&self) -> MessageId {
        self.message
    }
    pub fn object_hash(&self) -> [u8; 32] {
        self.hash
    }
    /// Original monotonic transfer-custody expiry, not the signed Finish/action deadline.
    pub fn transition_until(&self) -> u64 {
        self.until
    }
}
impl ConfirmationControl {
    /// Observe only this original plan's successfully published native Finish. No current
    /// authority is implied, including when this retained evidence survives End or expiry.
    pub fn confirmed_finish(&self) -> Option<FinishEvidence> {
        self.binding
            .transition
            .finish
            .get()
            .map(|record| FinishEvidence {
                message: record.identity.message,
                hash: record.identity.hash,
                until: record.until,
            })
    }
}

#[derive(Default)]
pub(super) struct FinishWindow {
    // Authoritative Open / Await(until) / Closed state. Load/store of a deadline alone
    // cannot serialize a stale collector decision against an accepted Finish publication.
    phase: AtomicU64,
    finish: OnceLock<FinishRecord>,
    epoch: OnceLock<EpochObservation>,
}
impl FinishWindow {
    pub(super) fn is_live(&self) -> bool {
        self.phase.load(Ordering::Acquire) != CLOSED
            && self.epoch.get().is_none_or(EpochObservation::is_live)
    }
    pub(super) fn close(&self) {
        self.phase.store(CLOSED, Ordering::Release);
    }

    /// Claim expiry before cleanup; losing an old Open decision to Finish must reload.
    /// This is the SAME operation for the pair and process Workflow collectors.
    fn claim_expired(&self, original_deadline: u64, mono: u64) -> bool {
        self.claim_expired_after_observation(original_deadline, mono, || {})
    }
    // The production caller supplies no callback. Tests park this SAME decision immediately
    // before its first CAS, so a successful Finish can force the actual reload branch.
    fn claim_expired_after_observation(
        &self,
        original_deadline: u64,
        mono: u64,
        before_claim: impl FnOnce(),
    ) -> bool {
        if self.epoch.get().is_some_and(|epoch| !epoch.is_live()) {
            self.close();
            return true;
        }
        let mut observed = self.phase.load(Ordering::Acquire);
        let mut before_claim = Some(before_claim);
        loop {
            if observed == CLOSED {
                return true;
            }
            let deadline = if observed == OPEN {
                original_deadline
            } else {
                observed
            };
            if mono < deadline {
                return false;
            }
            if let Some(before_claim) = before_claim.take() {
                before_claim();
            }
            match self
                .phase
                .compare_exchange(observed, CLOSED, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(current) => observed = current,
            }
        }
    }
}

/// Preparation has no authority or allocation of its own. It refers only to the original
/// paid fixed binding. A refused result drops it outside Engine/Group/Policy, without taking
/// the window, consuming a token or creating any successor reservation.
pub(in crate::voice_v2) struct PreparedFinishWindow {
    binding: Arc<PlanBinding>,
}
impl PreparedFinishWindow {
    /// Called INSIDE the final LocalOrigin::publish closure, before any irreversible Finish
    /// effect. All fallible checks precede CAS; success has an allocation/callback-free tail.
    pub(in crate::voice_v2) fn publish_at(
        &self,
        mono: u64,
        identity: FinishIdentity,
    ) -> Result<(), VoiceError> {
        let until = mono
            .checked_add(AWAIT_READY_MS)
            .filter(|until| *until != CLOSED)
            .ok_or(VoiceError::Freshness)?;
        if mono >= self.binding.deadline
            || !self.binding.live()
            || !self
                .binding
                .transition
                .epoch
                .get()
                .is_some_and(EpochObservation::is_live)
        {
            return Err(VoiceError::Unauthorized);
        }
        self.binding
            .transition
            .phase
            .compare_exchange(OPEN, until, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| VoiceError::Unauthorized)?;
        // Only the winning arm can install this exact original native signed identity. No
        // transport/public/crypto output fingerprint may stand in for this evidence.
        self.binding
            .transition
            .finish
            .set(FinishRecord { identity, until })
            .unwrap_or_else(|_| panic!("one successful native Finish arm"));
        Ok(())
    }
}

impl PreparedChain {
    #[cfg(test)]
    pub(in crate::voice_v2) fn claim_window_expiry_for_test(&self, mono: u64) -> bool {
        self.binding
            .transition
            .claim_expired(self.binding.deadline, mono)
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn finish_window_for_test(&self) -> Option<(FinishIdentity, u64)> {
        self.binding
            .transition
            .finish
            .get()
            .map(|record| (record.identity, record.until))
    }
    pub(in crate::voice_v2) fn prepare_finish_window(
        &self,
        actor: &ActorOwner,
        context: &Context,
    ) -> Result<PreparedFinishWindow, VoiceError> {
        if !actor.uses_resources(self.binding.resources())
            || context.scope != self.binding.scope
            || context.local != self.binding.local
            || context.epoch != self.binding.epoch
            || !self.binding.live()
            || !self.live.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let epoch = actor.observe_epoch(context)?;
        // Store only one original observer, including across a refused result. A replacement
        // actor/policy cannot adopt this phase merely by supplying equal public fields.
        if let Err(epoch) = self.binding.transition.epoch.set(epoch) {
            if !self
                .binding
                .transition
                .epoch
                .get()
                .expect("original epoch observer")
                .same(&epoch)
            {
                return Err(VoiceError::Unauthorized);
            }
        }
        if self.binding.transition.phase.load(Ordering::Acquire) != OPEN {
            return Err(VoiceError::Unauthorized);
        }
        Ok(PreparedFinishWindow {
            binding: self.binding.clone(),
        })
    }

    pub(in crate::voice_v2) fn collect_expired(&self, mono: u64) {
        if !self.binding.live()
            || !self.live.is_live()
            || self
                .binding
                .transition
                .claim_expired(self.binding.deadline, mono)
        {
            self.close();
        }
    }
}

#[cfg(test)]
mod tests;
