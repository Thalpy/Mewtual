//! Scheduling history on the original KeyClaim. These flags own no Exchange/Workflow,
//! grant no SDP effect, and never extend the original finite Finish window.
use super::transition::try_lock;
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::voice_handoff) enum ReadyProgress {
    Pending,
    CoreDrained,
    HighQueued,
    Expired,
}
impl ReadyProgress {
    pub(in crate::voice_handoff) fn core_drained(self) -> bool {
        matches!(self, Self::CoreDrained | Self::HighQueued)
    }
    fn needs_work(self, low: bool) -> bool {
        self == Self::Pending || (!low && self == Self::CoreDrained)
    }
}
/// The real indexed parent and original immutable release proof survive the Actor wait.
/// A public RowId or matching peer labels cannot construct a replacement continuation.
pub(in crate::voice_handoff) struct ReadyContinuation {
    pub parent: ResolvedRow,
    pub handshake: [u8; 32],
    pub low: bool,
    proof: [u8; 32],
}
impl Ledger {
    pub(in crate::voice_handoff) fn has_ready_work(
        &self,
        selected: &SelectedHandoffOwner,
        session: &Arc<SessionIdentity>,
    ) -> bool {
        let Ok(index) = try_lock(&self.index) else {
            return false;
        };
        index.rows.iter().flatten().any(|row| {
            if row.key.parent.is_some() || !row.key.matches(selected, row.key.purpose) {
                return false;
            }
            let Ok(state) = try_lock(&row.state) else {
                return false;
            };
            ready_candidate(&state, session).is_some()
        })
    }
    pub(in crate::voice_handoff) fn ready_continuation(
        self: &Arc<Self>,
        selected: &SelectedHandoffOwner,
        session: &Arc<SessionIdentity>,
        cursor: &mut u8,
        mono: u64,
    ) -> Result<ReadyContinuation, VoiceError> {
        let index = try_lock(&self.index)?;
        for offset in 0..ROWS {
            let slot = (usize::from(*cursor) + offset) % ROWS;
            let Some(row) = &index.rows[slot] else {
                continue;
            };
            if row.key.parent.is_some() || !row.key.matches(selected, row.key.purpose) {
                continue;
            }
            let mut state = try_lock(&row.state)?;
            let Some((low, handshake, proof)) = ready_candidate(&state, session) else {
                continue;
            };
            let claim = state.plan_mut().unwrap().transition.as_mut().unwrap();
            // Current key retention deliberately outlives this first finite Ready window.
            // Stop this scheduling hint on expiry; never turn it into a fresh round grant.
            if claim.custody.remaining_at(mono).is_err() {
                claim.key.as_mut().unwrap().ready = ReadyProgress::Expired;
                continue;
            }
            *cursor = ((slot + 1) % ROWS) as u8;
            return Ok(ReadyContinuation {
                parent: ResolvedRow {
                    ledger: self.clone(),
                    row: row.clone(),
                },
                handshake,
                low,
                proof,
            });
        }
        Err(VoiceError::Capacity)
    }
}
fn ready_candidate(
    state: &State,
    session: &Arc<SessionIdentity>,
) -> Option<(bool, [u8; 32], [u8; 32])> {
    if state.final_phase.is_some() || state.final_delivery().is_some() {
        return None;
    }
    let plan = state.plan()?;
    let claim = plan.transition.as_ref()?;
    let key = claim.key.as_ref()?;
    (!key.released
        && key.custody.is_current()
        && Arc::ptr_eq(&claim.session, session)
        && key.ready.needs_work(plan.local_is_low))
    .then(|| (plan.local_is_low, plan.identity.handshake(), key.receipt))
}
impl ReadyContinuation {
    /// Repeat the exact original UI/Coordinator/Route/Index/parent admission after every
    /// Actor await. KeyRelease can run without UI locking, so its Root atomic fence is
    /// independently required at every eventual Root publication as well.
    pub(in crate::voice_handoff) fn progress(
        &self,
        admission: &super::super::transition::Admission<'_>,
        mono: u64,
        update: Option<ReadyProgress>,
    ) -> Result<ReadyProgress, VoiceError> {
        if !Arc::ptr_eq(&self.parent.ledger, admission.ledger())
            || !self
                .parent
                .row
                .key
                .matches(admission.selected(), self.parent.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let _index = self.parent.transition_index()?;
        let mut state = try_lock(&self.parent.row.state)?;
        if state.final_phase.is_some() || state.final_delivery().is_some() {
            return Err(VoiceError::Unauthorized);
        }
        let plan = state.plan_mut().ok_or(VoiceError::Unauthorized)?;
        if plan.local_is_low != self.low || plan.identity.handshake() != self.handshake {
            return Err(VoiceError::Unauthorized);
        }
        let claim = plan.transition.as_mut().ok_or(VoiceError::Unauthorized)?;
        let key = claim.key.as_mut().ok_or(VoiceError::Unauthorized)?;
        if !Arc::ptr_eq(&claim.session, admission.session())
            || key.receipt != self.proof
            || key.released
            || !key.custody.is_current()
        {
            return Err(VoiceError::Unauthorized);
        }
        claim.custody.remaining_at(mono)?;
        admission.current()?;
        if let Some(update) = update {
            // Never downgrade a queued HIGH receipt or revive an expired first intent.
            if key.ready == ReadyProgress::Pending && update == ReadyProgress::CoreDrained
                || !self.low
                    && key.ready == ReadyProgress::CoreDrained
                    && update == ReadyProgress::HighQueued
            {
                key.ready = update;
            } else if key.ready != update {
                return Err(VoiceError::Unauthorized);
            }
        }
        Ok(key.ready)
    }
}

#[cfg(test)]
impl ResolvedRow {
    /// Independent native publication oracle; tests cannot set Ready/CoreDrained from a
    /// browser boolean or injected protocol marker. The real paid worker is the sole writer.
    pub(crate) fn ready_test_observation(&self) -> (bool, u8, bool, bool) {
        let _index = self.indexed().unwrap();
        let state = self.row.state.lock().unwrap();
        let plan = state.plan().unwrap();
        let key = plan.transition.as_ref().unwrap().key.as_ref().unwrap();
        (
            plan.local_is_low,
            key.ready as u8,
            key.released,
            key.custody.is_current(),
        )
    }
    pub(crate) fn ready_test_complete(&self) -> bool {
        let _index = self.indexed().unwrap();
        let state = self.row.state.lock().unwrap();
        let Some(plan) = state.plan() else {
            return false;
        };
        plan.transition
            .as_ref()
            .and_then(|claim| claim.key.as_ref())
            .is_some_and(|key| {
                !key.released
                    && key.custody.is_current()
                    && if plan.local_is_low {
                        key.ready == ReadyProgress::CoreDrained
                    } else {
                        key.ready == ReadyProgress::HighQueued
                    }
            })
    }
}
