//! Inert admission capacity in the real native ledger. A claim has no selector,
//! Pending row, Action or signing authority. It reserves one original serial and
//! the two existing payments that a future private parent publication must MOVE,
//! never refund and reacquire. No operational consumption API is supplied here.
use super::*;
use std::num::NonZeroU64;

mod admitted;
pub(in crate::voice_handoff) use admitted::AdmittedParentClaim;
#[cfg(test)]
pub(in crate::voice_handoff) use admitted::{
    fixture_existing_allocators_are_excluded, fixture_existing_claim,
};

/// Only bootstrap's original Slot/finalizer may retain this affine value. The
/// ledger index stores a scalar stamp, never a back-reference to the claim or
/// bootstrap Row. Ledger's existing action/media rows retain no bootstrap Route,
/// so this strong Ledger owner cannot form Slot -> Ledger -> Slot custody cycles.
///
/// Field order is a return barrier: preparation and its actual credits are
/// destroyed before Returned can make the physical position reusable, including
/// unwinding destruction. The original bootstrap/task inventory covers this
/// small inline wrapper and its fixed final-return frame after those fees drop.
pub(in crate::voice_handoff) struct ParentClaim {
    owners: Owners,
    returned: Returned,
}
struct Owners {
    selected: Arc<SelectedHandoffOwner>,
    session: Arc<SessionIdentity>,
    generation: u64,
    // These are the SAME future parent contributions. A later consume API must
    // move them with the reserved serial; constructing a fresh PreparedPhase or
    // charging another Row would break the promise under ordinary-pool pressure.
    _prepared: PreparedPhase,
    _metadata: Credit,
}
struct Returned {
    // Future consumption must atomically transfer this SAME stamp into the real
    // Row under Index before disarming this marker. Dropping the claim and then
    // starting an ordinary Begin would lose both reserved capacity and provenance.
    ledger: Arc<Ledger>,
    id: RowId,
    #[cfg(test)]
    before_release: Option<(
        std::sync::mpsc::SyncSender<()>,
        std::sync::mpsc::Receiver<()>,
    )>,
}
impl Drop for Returned {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some((entered, release)) = self.before_release.take() {
            // Deterministic actual-destruction boundary; time is only a failure
            // watchdog. Never install a callback or ambient timer in production.
            let _ = entered.send(());
            let _ = release.recv_timeout(std::time::Duration::from_secs(5));
        }
        let mut index = self.ledger.index.lock().unwrap_or_else(|e| e.into_inner());
        if index.reserved[self.id.slot].map(NonZeroU64::get) == Some(self.id.serial)
            && index.rows[self.id.slot].is_none()
        {
            index.reserved[self.id.slot] = None;
        }
        // An impossible mismatch fails closed. A destructor must never clear a
        // successor's stamp, invent a new serial, or panic during another unwind.
    }
}
impl ParentClaim {
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_id(&self) -> RowId {
        self.returned.id
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_hold_return(
        &mut self,
        entered: std::sync::mpsc::SyncSender<()>,
        release: std::sync::mpsc::Receiver<()>,
    ) {
        self.returned.before_release = Some((entered, release));
    }
    /// The permanent native worker also funds this synchronous prepayment frame.
    /// The future Row/phase fees remain independent of its inline custody graph.
    pub(in crate::voice_handoff) fn preparation_layout_bytes() -> usize {
        std::mem::size_of::<Owners>()
            + std::mem::size_of::<Returned>()
            + std::mem::size_of::<Credit>()
            + std::mem::size_of::<PreparedPhase>()
            + std::mem::size_of::<std::sync::MutexGuard<'_, Index>>()
    }
    /// Pure exact provenance, suitable for bootstrap's final short fence. The
    /// affine owner itself keeps its stamp occupied; taking Index here would
    /// introduce a new lock nesting just to re-read our private invariant.
    pub(in crate::voice_handoff) fn matches(
        &self,
        selected: &Arc<SelectedHandoffOwner>,
        session: &Arc<SessionIdentity>,
        generation: u64,
    ) -> bool {
        generation != 0
            && self.owners.generation == generation
            && Arc::ptr_eq(&self.owners.selected, selected)
            && Arc::ptr_eq(&self.owners.session, session)
            && selected.lease().is_live()
    }
    pub(in crate::voice_handoff) fn belongs_to(&self, ledger: &Arc<Ledger>) -> bool {
        Arc::ptr_eq(&self.returned.ledger, ledger)
    }
}
impl Ledger {
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_parent_reserved(&self, id: RowId) -> bool {
        self.index.lock().unwrap().reserved[id.slot].map(NonZeroU64::get) == Some(id.serial)
    }
    /// Prepay one future native parent without selecting a peer or invoking any
    /// Root factory. All fees and the prepared phase come from the exact native
    /// selection's pool/group. Caller must repeat original UI/Session/generation
    /// authorization before attaching it to Slot; this value alone authorizes no
    /// signed readiness, crypto, protocol attempt, browser work or RTC effect.
    pub(in crate::voice_handoff) fn reserve_parent(
        self: &Arc<Self>,
        selected: Arc<SelectedHandoffOwner>,
        session: Arc<SessionIdentity>,
        generation: u64,
    ) -> Result<ParentClaim, VoiceError> {
        if generation == 0 || !selected.lease().is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let resources = selected.resources();
        let scope = selected.binding().scope;
        let metadata = resources.charge(scope.group, ROW_BYTES, ResourceKind::Retained)?;
        let prepared = PreparedPhase::new(resources, scope, selected.lease().clone())?;
        // Declare owners BEFORE Index so error/unwind releases Index first. Root
        // and prepared/resource destructors are never run under native guards.
        let owners = Owners {
            selected,
            session,
            generation,
            _prepared: prepared,
            _metadata: metadata,
        };
        let mut index = transition::try_lock(&self.index)?;
        if !owners.selected.lease().is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let slot = index.vacant().ok_or(VoiceError::Capacity)?;
        let serial = index.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        let stamp = NonZeroU64::new(serial).ok_or(VoiceError::Capacity)?;
        index.serial = serial;
        index.reserved[slot] = Some(stamp);
        drop(index);
        Ok(ParentClaim {
            owners,
            returned: Returned {
                ledger: self.clone(),
                id: RowId { slot, serial },
                #[cfg(test)]
                before_release: None,
            },
        })
    }
}

#[cfg(test)]
mod tests;
