//! Original SDP action rows, distinct from confirmation Workflow/Phase loans. Payment is
//! prepared outside native guards; uniqueness and installation share the existing32-slot
//! Index. Recovering a fee cannot allocate a second candidate Row for that same purpose.
use super::transition::try_lock;
use super::*;
use crate::voice_handoff::requests::sdp_action::Begin as Request;
use crate::voice_handoff::transition::Admission;
use catcoms_sync::voice_v2::chain::sdp_handoff::{self, NativeRowFunding, NativeRowPurpose};
use catcoms_sync::voice_v2::sdp::publication::{History, Outcome, Prepared, Writer};
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, SdpDelivery};

pub(super) struct Owners {
    selected: Arc<SelectedHandoffOwner>,
    phase: Arc<sdp_handoff::Phase>,
    intent: Arc<InitialSdpIntent>,
    session: Arc<SessionIdentity>,
    outcome: Arc<Outcome>,
    funding: Arc<NativeRowFunding>,
}
pub(super) struct Pending {
    owners: Owners,
    pub(super) published: Option<Arc<CryptoAction>>,
    pub(super) observer: bool,
    body_matches: bool,
    body_tag: Option<[u8; 32]>,
    pub(super) worker: bool,
    handed_off: bool,
    closed: bool,
}
pub(super) struct Live {
    owners: Owners,
    pub(super) action: Arc<CryptoAction>,
    delivery: Option<Arc<SdpDelivery>>,
    body_tag: Option<[u8; 32]>,
    pub(super) worker: bool,
    pub(super) observer: bool,
    closed: bool,
}
/// Fixed direct recovery fields: no direct Phase, input, Copy, publication or SDK work.
/// Row.key.parent intentionally retains the original parent graph until safe GC; parent
/// storage closure must retire its unused reserve/Copies even while this history survives.
pub(super) struct Drained {
    pub(super) action: Option<Arc<CryptoAction>>,
    delivery: Option<Arc<SdpDelivery>>,
    body_tag: Option<[u8; 32]>,
    session: Arc<SessionIdentity>,
    outcome: Arc<Outcome>,
    funding: Arc<NativeRowFunding>,
}
/// Borrow only stable canonical completed-row evidence under the actual registered State.
/// This creates no observer/lease or Clock callback. A media publisher must also repeat
/// exact original parent/Session/row/action/phase identity under its final native fence.
pub(super) struct MediaRow<'a> {
    pub(super) action: &'a Arc<CryptoAction>,
    pub(super) funding: &'a Arc<NativeRowFunding>,
}
pub(super) fn media_row(
    state: &State,
    purpose: NativeRowPurpose,
) -> Result<MediaRow<'_>, VoiceError> {
    let Body::SdpDrained(value) = &state.body else {
        return Err(VoiceError::Capacity);
    };
    let action = value.action.as_ref().ok_or(VoiceError::Unauthorized)?;
    if !state.status.drained
        || !state.status.engine.settled
        || !state.status.acknowledged
        || !value.funding.is_collected()
        || value.funding.purpose() != purpose
        || state.status.action != Some((action.serial(), action.version(), action.input_tag()))
        || !state
            .status
            .engine
            .receipt
            .as_ref()
            .is_some_and(|r| match purpose {
                NativeRowPurpose::Seal => matches!(
                    r.outcome,
                    catcoms_sync::voice_v2::crypto_actions::CryptoOutcome::Sealed { .. }
                ),
                NativeRowPurpose::Open => {
                    r.outcome == catcoms_sync::voice_v2::crypto_actions::CryptoOutcome::Opened
                }
            })
    {
        return Err(VoiceError::Unauthorized);
    }
    Ok(MediaRow {
        action,
        funding: &value.funding,
    })
}
pub(in crate::voice_handoff) enum Begin {
    Factory(Registration),
    Existing(ResolvedRow),
}
/// An actual native worker retains the counted Pending through queue/result collection.
/// Losing its HTTP waiter cannot delete or renumber the original publication history.
pub(in crate::voice_handoff) struct Registration {
    ledger: Arc<Ledger>,
    row: Arc<Row>,
    // Fixture-only observation of actual worker retirement, before its real State lock.
    // Production has no callback/channel and no means to install this barrier.
    #[cfg(test)]
    before_retire: Option<std::sync::mpsc::SyncSender<()>>,
}
fn position(purpose: NativeRowPurpose) -> usize {
    match purpose {
        NativeRowPurpose::Seal => 0,
        NativeRowPurpose::Open => 1,
    }
}
/// Borrowed original publication only. No observer, input loan, copy, or factory is
/// manufactured. The caller must keep the actual State guard through these borrows.
pub(super) struct InputOwners<'a> {
    pub(super) selected: &'a Arc<SelectedHandoffOwner>,
    pub(super) phase: &'a Arc<sdp_handoff::Phase>,
    pub(super) intent: &'a Arc<InitialSdpIntent>,
    pub(super) session: &'a Arc<SessionIdentity>,
    pub(super) funding: &'a Arc<NativeRowFunding>,
    pub(super) action: &'a Arc<CryptoAction>,
    pub(super) closed: bool,
}
pub(super) fn input_owners(state: &State) -> Result<InputOwners<'_>, VoiceError> {
    let (owners, action, closed) = match &state.body {
        Body::SdpPending(value) if value.body_matches => (
            &value.owners,
            value.published.as_ref().ok_or(VoiceError::Capacity)?,
            value.closed,
        ),
        Body::SdpLive(value) => (&value.owners, &value.action, value.closed),
        // Drained is canonical history, never a reason to recreate a Phase/input.
        _ => return Err(VoiceError::Unauthorized),
    };
    Ok(InputOwners {
        selected: &owners.selected,
        phase: &owners.phase,
        intent: &owners.intent,
        session: &owners.session,
        funding: &owners.funding,
        action,
        closed,
    })
}
/// Exact paid input is borrowed through installation, so a producer cannot swap a
/// different request underneath a precomputed commitment. Hashing and fee preparation
/// finish before the native UI/Index/State admission guards are entered.
pub(in crate::voice_handoff) struct Candidate<'a> {
    request: &'a Request<'a>,
    funding: Arc<NativeRowFunding>,
    body_tag: Option<[u8; 32]>,
    outcome: Prepared,
    intent: Arc<InitialSdpIntent>,
}
impl Candidate<'_> {
    pub(in crate::voice_handoff) fn funding(&self) -> &Arc<NativeRowFunding> {
        &self.funding
    }
    pub(in crate::voice_handoff) fn intent(&self) -> &Arc<InitialSdpIntent> {
        &self.intent
    }
}
fn body_tag(state: &State) -> Option<Option<[u8; 32]>> {
    match &state.body {
        Body::SdpPending(p) => Some(p.body_tag),
        Body::SdpLive(p) => Some(p.body_tag),
        Body::SdpDrained(p) => Some(p.body_tag),
        _ => None,
    }
}
/// Original encrypted obligations are historical lookup bindings. Closed SDK/storage
/// still match them; a fresh effect additionally checks currency at its final admission.
fn original<'a>(
    parent: &ResolvedRow,
    state: &'a State,
    session: &Arc<SessionIdentity>,
    request: &Request<'_>,
) -> Result<(&'a super::sdp::Claim, &'a Arc<InitialSdpIntent>), VoiceError> {
    if parent.row.id != request.parent
        || !matches!(parent.row.key.purpose, Purpose::Derive(_))
        || parent.row.key.parent.is_some()
    {
        return Err(VoiceError::Unauthorized);
    }
    let storage = super::pc::storage(parent, state, session, &request.pc)?;
    let pc = storage.pc.as_ref().ok_or(VoiceError::Unauthorized)?;
    if !pc.matches(request.proof) {
        return Err(VoiceError::Unauthorized);
    }
    Ok((storage, pc.intent().ok_or(VoiceError::Unauthorized)?))
}
pub(super) fn session(state: &State) -> Option<&Arc<SessionIdentity>> {
    match &state.body {
        Body::SdpPending(p) => Some(&p.owners.session),
        Body::SdpLive(p) => Some(&p.owners.session),
        Body::SdpDrained(p) => Some(&p.session),
        _ => None,
    }
}
/// Unknown new body variants must never inherit plan(None)'s obligation-free default.
/// For SDP only this explicit terminal state proves action-local collection, never whole PC.
pub(super) fn locally_drained(state: &State) -> Option<bool> {
    match &state.body {
        Body::SdpPending(_) | Body::SdpLive(_) => Some(false),
        Body::SdpDrained(p) => Some(
            state.status.drained
                && p.funding.is_collected()
                && (p.action.is_some() || p.outcome.is_proven_unpublished())
                && (p.action.is_none()
                    || (state.status.engine.settled
                        && (state.status.engine.receipt.is_none() || state.status.acknowledged))),
        ),
        _ => None,
    }
}
/// Actual publication identity capture. Root observers used here are pure fixed metadata,
/// never input/engine/Clock calls. A mismatch still retains provenance-matching publication.
fn capture_locked(
    row: &Row,
    state: &mut State,
    action: &Arc<CryptoAction>,
    worker: bool,
) -> Result<(bool, Option<sdp_handoff::NativeIoClosure>), VoiceError> {
    let Body::SdpPending(p) = &mut state.body else {
        return Err(VoiceError::Unauthorized);
    };
    let expected = match row.key.purpose {
        Purpose::Sdp {
            purpose: NativeRowPurpose::Seal,
            ..
        } => CryptoOperation::SealSdp,
        Purpose::Sdp {
            purpose: NativeRowPurpose::Open,
            ..
        } => CryptoOperation::OpenSdp,
        _ => return Err(VoiceError::Unauthorized),
    };
    if p.worker != worker
        || !p.handed_off
        || !p.owners.outcome.owns_publication(action)
        || action.operation() != expected
        || !action.uses_sdp_intent(&p.owners.intent)
        || !action.belongs_to(p.owners.selected.binding().scope, p.owners.selected.lease())
    {
        return Err(VoiceError::Unauthorized);
    }
    if let Some(original) = &p.published {
        if !Arc::ptr_eq(original, action) {
            return Err(VoiceError::RetryMismatch);
        }
    } else {
        p.published = Some(action.clone());
    }
    let matches = match p.body_tag {
        Some(tag) => action.matches_sdp_plaintext_commitment(&tag),
        None => action.operation() == CryptoOperation::OpenSdp,
    };
    p.body_matches = matches;
    let closure = if matches {
        None
    } else {
        // A producer mixup must neither expose B as A's SDK input nor lose the
        // actually published B. Keep its SAME Arc for negative-only cleanup.
        p.closed = true;
        Some(p.owners.funding.close_io())
    };
    state.status.action = Some((action.serial(), action.version(), action.input_tag()));
    Ok((matches, closure))
}

impl ResolvedRow {
    #[cfg(test)]
    pub(in crate::voice_handoff) fn sdp_test_outcome(&self) -> Arc<Outcome> {
        let state = self.row.state.lock().unwrap();
        match &state.body {
            Body::SdpPending(p) => p.owners.outcome.clone(),
            Body::SdpLive(p) => p.owners.outcome.clone(),
            Body::SdpDrained(p) => p.outcome.clone(),
            _ => panic!("actual SDP row"),
        }
    }
    /// Recover ONLY the actual positive publication from this row's original slot. Read
    /// its historical Arc outside native guards; no current grant, Pair lookup, clock or
    /// new Copy is needed. Entered uncertainty is deliberately not converted to absence.
    pub(super) fn recover_sdp_publication(&self) -> Result<(), VoiceError> {
        let outcome = {
            let state = try_lock(&self.row.state)?;
            match &state.body {
                Body::SdpPending(p) if !p.worker && p.handed_off && p.published.is_none() => {
                    p.owners.outcome.clone()
                }
                _ => return Ok(()),
            }
        };
        let History::Published(action) = outcome.history() else {
            return Ok(());
        };
        let closure = {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            let Body::SdpPending(p) = &state.body else {
                return Ok(());
            };
            if p.worker || !Arc::ptr_eq(&outcome, &p.owners.outcome) {
                return Err(VoiceError::Unauthorized);
            }
            let (_, closure) = capture_locked(&self.row, &mut state, &action, false)?;
            closure
        };
        drop(closure);
        self.row.changed.notify_waiters();
        Ok(())
    }
    /// Prepay/recover the original fixed fee outside Index/Row/UI guards. This issues no
    /// Row or worker. Historical callers first use lookup_sdp_action without prepayment.
    pub(in crate::voice_handoff) fn sdp_action_candidate<'a>(
        &self,
        session: &Arc<SessionIdentity>,
        request: &'a Request<'a>,
    ) -> Result<Candidate<'a>, VoiceError> {
        let body_tag = request
            .plaintext
            .map(catcoms_sync::voice_v2::sdp::plaintext_commitment)
            .transpose()?;
        let purpose = if body_tag.is_some() {
            NativeRowPurpose::Seal
        } else {
            NativeRowPurpose::Open
        };
        let (phase, intent) = {
            let _index = self.transition_index()?;
            let state = try_lock(&self.row.state)?;
            let (storage, intent) = original(self, &state, session, request)?;
            if storage.closed
                || storage.actions_closed[position(purpose)]
                || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
            {
                return Err(VoiceError::Unauthorized);
            }
            let phase = storage.phase.as_ref().ok_or(VoiceError::Unauthorized)?;
            if !intent.uses_phase(phase) {
                return Err(VoiceError::Unauthorized);
            }
            (phase.clone(), intent.clone())
        };
        let funding = phase.native_history(purpose)?;
        let outcome = Prepared::new(intent.clone(), funding.clone(), body_tag)?;
        Ok(Candidate {
            request,
            funding,
            body_tag,
            outcome,
            intent,
        })
    }
    /// Existing Begin checks its full original binding and canonical body without a
    /// Candidate, fee, Copy, Clock or factory. Pending is NOT retry evidence: only the
    /// same installed outcome may make the returned planning hint true. Final resume
    /// repeats every current/worker/observer/outcome check under its admission fence.
    pub(in crate::voice_handoff) fn check_sdp_begin(
        &self,
        parent: &ResolvedRow,
        session: &Arc<SessionIdentity>,
        request: &Request<'_>,
    ) -> Result<bool, VoiceError> {
        let tag = request
            .plaintext
            .map(catcoms_sync::voice_v2::sdp::plaintext_commitment)
            .transpose()?;
        let purpose = if tag.is_some() {
            NativeRowPurpose::Seal
        } else {
            NativeRowPurpose::Open
        };
        if !Arc::ptr_eq(&self.ledger, &parent.ledger)
            || self.row.key.purpose
                != (Purpose::Sdp {
                    parent: parent.id(),
                    purpose,
                })
            || !self
                .row
                .key
                .parent
                .as_ref()
                .is_some_and(|row| Arc::ptr_eq(row, &parent.row))
        {
            return Err(VoiceError::Unauthorized);
        }
        let _index = self.transition_index()?;
        let parent_state = try_lock(&parent.row.state)?;
        original(parent, &parent_state, session, request)?;
        let state = try_lock(&self.row.state)?;
        if !self::session(&state).is_some_and(|value| Arc::ptr_eq(value, session)) {
            return Err(VoiceError::Unauthorized);
        }
        if body_tag(&state) != Some(tag) {
            return Err(VoiceError::RetryMismatch);
        }
        Ok(
            matches!(&state.body, Body::SdpPending(value) if !value.worker && !value.observer
            && !value.closed && value.published.is_none() && value.owners.outcome.permits_local_retry()),
        )
    }
    /// Pure current check for a new-work prerequisite (notably waiting for HIGH's
    /// accepted/applied Open). It installs nothing, spends no attempt and grants no effect.
    pub(in crate::voice_handoff) fn require_sdp_publication_current(
        &self,
        admission: &Admission<'_>,
        candidate: &Candidate<'_>,
        wall: u64,
        mono: u64,
    ) -> Result<(), VoiceError> {
        if !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        let (storage, intent) = original(self, &state, admission.session(), candidate.request)?;
        require_current(
            self, &state, storage, intent, candidate, admission, wall, mono,
        )
    }
    /// Reclaim ONLY this original Pending after a proven local, unstarted or explicit
    /// prepublication Capacity refusal. Do not mint another row/outcome/deadline, reset
    /// handed_off, or use a stale outside negative snapshot. Publication recovery wins.
    /// A worker which may still enter Root is excluded by its SAME outcome's affine Writer.
    pub(in crate::voice_handoff) fn resume_sdp_action(
        &self,
        parent: &ResolvedRow,
        admission: &Admission<'_>,
        candidate: &Candidate<'_>,
        wall: u64,
        mono: u64,
    ) -> Result<Option<Registration>, VoiceError> {
        if !Arc::ptr_eq(&self.ledger, &parent.ledger)
            || !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
            || !self
                .row
                .key
                .parent
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &parent.row))
        {
            return Err(VoiceError::Unauthorized);
        }
        let index = try_lock(&self.ledger.index)?;
        for row in [&parent.row, &self.row] {
            if !index.rows[row.id.slot]
                .as_ref()
                .is_some_and(|r| Arc::ptr_eq(r, row))
            {
                return Err(VoiceError::Unauthorized);
            }
        }
        let parent_state = try_lock(&parent.row.state)?;
        let (storage, intent) = original(
            parent,
            &parent_state,
            admission.session(),
            candidate.request,
        )?;
        let phase = storage.phase.as_ref().ok_or(VoiceError::Unauthorized)?;
        let Purpose::Sdp {
            parent: original_parent,
            purpose,
        } = self.row.key.purpose
        else {
            return Err(VoiceError::Unauthorized);
        };
        if original_parent != parent.id()
            || !candidate
                .outcome
                .matches(intent, &candidate.funding, candidate.body_tag)
            || candidate.funding.purpose() != purpose
            || !candidate.funding.uses_phase(phase)
            || storage.closed
            || storage.actions_closed[position(purpose)]
            || storage.worker()
            || storage.sdk_preparing
            || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
            || !self.row.key.lease.is_live()
            || parent_state.final_phase.is_some()
            || parent_state.final_delivery().is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        let mut state = try_lock(&self.row.state)?;
        if self::body_tag(&state) != Some(candidate.body_tag) {
            return Err(VoiceError::RetryMismatch);
        }
        let Body::SdpPending(p) = &mut state.body else {
            return Ok(None); // Live/terminal recovery must never re-enqueue its factory.
        };
        if !Arc::ptr_eq(&p.owners.session, admission.session())
            || !Arc::ptr_eq(&p.owners.intent, intent)
            || !Arc::ptr_eq(&p.owners.phase, phase)
            || !Arc::ptr_eq(&p.owners.funding, &candidate.funding)
            || p.closed
        {
            return Err(VoiceError::Unauthorized);
        }
        if p.worker
            || p.observer
            || p.published.is_some()
            || !p.owners.outcome.permits_local_retry()
        {
            return Ok(None);
        }
        intent.remaining_at(wall, mono)?;
        admission.current()?;
        // Final infallible tail. A retained Writer would have made the original slot
        // InFlight above; no arbitrary callback or new token is manufactured here.
        p.worker = true;
        Ok(Some(Registration {
            ledger: self.ledger.clone(),
            row: self.row.clone(),
            #[cfg(test)]
            before_retire: None,
        }))
    }
    /// Adopt an actual recovered Action after its original worker has disappeared and IO
    /// admission closed. No Copy, body, current grant or Clock is needed. A trusted Actor
    /// recovery path must supply the real Arc; renderer labels cannot construct this value.
    /// This closes an uncertainty only with positive publication evidence, never absence.
    pub(in crate::voice_handoff) fn capture_closed_sdp_publication(
        &self,
        session: &Arc<SessionIdentity>,
        action: &Arc<CryptoAction>,
    ) -> Result<(), VoiceError> {
        let (matches, closure) = {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            let Body::SdpPending(p) = &state.body else {
                return Err(VoiceError::Unauthorized);
            };
            if p.worker || !p.owners.funding.is_closed() || !Arc::ptr_eq(session, &p.owners.session)
            {
                return Err(VoiceError::Unauthorized);
            }
            capture_locked(&self.row, &mut state, action, false)?
        };
        drop(closure);
        self.row.changed.notify_waiters();
        if matches {
            Ok(())
        } else {
            Err(VoiceError::RetryMismatch)
        }
    }
    /// Return is action-local. Close admission under Index/State, then notify OUTSIDE all
    /// guards. Existing real workers/Copy counters and engine/ACK remain separate gates.
    pub(in crate::voice_handoff) fn close_sdp_action(
        &self,
        session: &Arc<SessionIdentity>,
    ) -> Result<(), VoiceError> {
        let closure = {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            if self.row.key.parent.is_none()
                || !self::session(&state).is_some_and(|original| Arc::ptr_eq(original, session))
            {
                return Err(VoiceError::Unauthorized);
            }
            let funding = match &mut state.body {
                Body::SdpPending(p) => {
                    p.closed = true;
                    &p.owners.funding
                }
                Body::SdpLive(p) => {
                    p.closed = true;
                    &p.owners.funding
                }
                Body::SdpDrained(p) => &p.funding,
                _ => return Err(VoiceError::Unauthorized),
            };
            funding.close_io()
        };
        drop(closure);
        self.row.changed.notify_waiters();
        Ok(())
    }
    /// Closure may retire an ACTUALLY unstarted captured Action. A permit/terminal
    /// stays intact; the Root gate, not a native boolean, decides whether retirement is safe.
    fn retire_closed_unstarted_sdp(&self) -> Result<(), VoiceError> {
        let ready = {
            let state = try_lock(&self.row.state)?;
            match &state.body {
                Body::SdpPending(p) => {
                    (p.closed
                        || !p.owners.phase.effect().is_live()
                        || !self.row.key.lease.is_live())
                        && !p.worker
                        && p.published.is_some()
                        && p.owners.funding.is_collected()
                }
                // Successful worker attachment does not imply the SDK ever started. Root
                // alone decides whether this exact live action is genuinely unstarted;
                // real permits/terminal receipts retain their actual settlement/ACK duties.
                Body::SdpLive(p) => {
                    (p.closed
                        || !p.owners.phase.effect().is_live()
                        || !self.row.key.lease.is_live())
                        && !p.worker
                        && p.owners.funding.is_collected()
                }
                _ => false,
            }
        };
        if !ready {
            return Ok(());
        }
        if let Some(observer) = Ledger::observe(&self.row, true)? {
            observer.action().retire_if_unstarted();
            let observed = observer.action().handoff_status();
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            merge_observed(&mut state.status, observed)?;
            drop(state);
            drop(observer);
        }
        Ok(())
    }
    /// Cache genuine Root engine/ACK history before entering the final compaction fence.
    /// Input/SDK/Actor owners MUST all have bound the original concrete counter first.
    pub(in crate::voice_handoff) fn try_compact_sdp(&self) -> Result<bool, VoiceError> {
        self.recover_sdp_publication()?;
        self.reconcile()?;
        self.retire_closed_unstarted_sdp()?;
        let retired = {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            if let Body::SdpDrained(_) = state.body {
                return Ok(locally_drained(&state) == Some(true));
            }
            let engine_done = state.status.engine.settled
                && (state.status.engine.receipt.is_none() || state.status.acknowledged);
            let ready = match &state.body {
                Body::SdpPending(p) => {
                    (p.closed
                        || !p.owners.phase.effect().is_live()
                        || !self.row.key.lease.is_live())
                        && !p.worker
                        && !p.observer
                        && p.owners.funding.is_collected()
                        && (if p.published.is_some() {
                            engine_done
                        } else {
                            // This must be a FRESH check on the SAME installed slot after
                            // CLOSED+zero, under this final Index/State fence. Worker loss
                            // or an outside negative snapshot never authorizes compaction.
                            p.owners.outcome.is_proven_unpublished()
                        })
                }
                Body::SdpLive(p) => {
                    !p.worker
                        && !p.observer
                        && p.owners.funding.is_collected()
                        && (p.closed || !p.owners.phase.effect().is_live())
                        && engine_done
                }
                _ => return Err(VoiceError::Unauthorized),
            };
            if !ready {
                return Ok(false);
            }
            // Clone fixed fields only after all terminal gates. No speculative Action
            // observer or variable payload owner is hidden in an unsuccessful attempt.
            let (action, delivery, body_tag, session, outcome, funding, handed_off) =
                match &state.body {
                    Body::SdpPending(p) => (
                        p.published.clone(),
                        None,
                        p.body_tag,
                        p.owners.session.clone(),
                        p.owners.outcome.clone(),
                        p.owners.funding.clone(),
                        p.handed_off,
                    ),
                    Body::SdpLive(p) => (
                        Some(p.action.clone()),
                        p.delivery.clone(),
                        p.body_tag,
                        p.owners.session.clone(),
                        p.owners.outcome.clone(),
                        p.owners.funding.clone(),
                        true,
                    ),
                    _ => unreachable!("checked SDP state"),
                };
            // These infallible moves retain SAME fixed history; the retired variable graph
            // drops only after every native guard has returned. No shared Phase close.
            let retired = std::mem::replace(
                &mut state.body,
                Body::SdpDrained(Drained {
                    action,
                    delivery,
                    body_tag,
                    session,
                    outcome,
                    funding,
                }),
            );
            state.status.drained = true;
            if let Some(receipt) = &mut state.status.engine.receipt {
                receipt.retired = true;
            }
            if matches!(retired, Body::SdpPending(_)) {
                state.status.pending = PendingStatus::Closed {
                    actor_handoff: handed_off,
                };
            }
            retired
        };
        drop(retired);
        self.row.changed.notify_waiters();
        Ok(true)
    }
}
/// Shared pure first-publication admission. An HTTP prerequisite wait and actual Row
/// installation must never disagree about a dead original SDK/PC or renew its deadline.
#[allow(clippy::too_many_arguments)]
fn require_current(
    parent: &ResolvedRow,
    state: &State,
    storage: &super::sdp::Claim,
    intent: &Arc<InitialSdpIntent>,
    candidate: &Candidate<'_>,
    admission: &Admission<'_>,
    wall: u64,
    mono: u64,
) -> Result<(), VoiceError> {
    let phase = storage.phase.as_ref().ok_or(VoiceError::Unauthorized)?;
    let funding = &candidate.funding;
    if !funding.uses_phase(phase)
        || !intent.uses_phase(phase)
        || !candidate
            .outcome
            .matches(intent, funding, candidate.body_tag)
        || storage.closed
        || storage.actions_closed[position(funding.purpose())]
        || funding.is_closed()
        || storage.worker()
        || storage.sdk_preparing
        || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
        || !parent.row.key.lease.is_live()
        || state.final_delivery().is_some()
        || state.final_phase.is_some()
    {
        return Err(VoiceError::Unauthorized);
    }
    intent.remaining_at(wall, mono)?;
    admission.current()
}
impl Ledger {
    /// Lost Begin may conceal both Action and child RowId. Resolve only exact original
    /// parent/Session/PC/purpose, with no current grant, fee/copy charge or factory call.
    pub(in crate::voice_handoff) fn lookup_sdp_action(
        self: &Arc<Self>,
        parent: &ResolvedRow,
        selected: &SelectedHandoffOwner,
        session: &Arc<SessionIdentity>,
        request: &Request<'_>,
        purpose: NativeRowPurpose,
    ) -> Result<Option<ResolvedRow>, VoiceError> {
        if !Arc::ptr_eq(self, &parent.ledger)
            || !parent.row.key.matches(selected, parent.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let index = try_lock(&self.index)?;
        if !index.rows[parent.id().slot]
            .as_ref()
            .is_some_and(|row| Arc::ptr_eq(row, &parent.row))
        {
            return Err(VoiceError::Unauthorized);
        }
        let state = try_lock(&parent.row.state)?;
        original(parent, &state, session, request)?;
        let purpose = Purpose::Sdp {
            parent: parent.id(),
            purpose,
        };
        Ok(index
            .rows
            .iter()
            .flatten()
            .find(|row| {
                row.key.matches(selected, purpose)
                    && row
                        .key
                        .parent
                        .as_ref()
                        .is_some_and(|p| Arc::ptr_eq(p, &parent.row))
            })
            .map(|row| ResolvedRow {
                ledger: self.clone(),
                row: row.clone(),
            }))
    }
    /// Latch a purpose close BEFORE a Row exists. A later/parked Begin rechecks this bit
    /// under the SAME Index+parent fence. Absence reports no IO/SDK/Root drainage proof.
    pub(in crate::voice_handoff) fn close_sdp_original(
        self: &Arc<Self>,
        parent: &ResolvedRow,
        selected: &SelectedHandoffOwner,
        session: &Arc<SessionIdentity>,
        request: &Request<'_>,
        purpose: NativeRowPurpose,
    ) -> Result<Option<ResolvedRow>, VoiceError> {
        if !Arc::ptr_eq(self, &parent.ledger)
            || !parent.row.key.matches(selected, parent.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let row = {
            let index = try_lock(&self.index)?;
            if !index.rows[parent.id().slot]
                .as_ref()
                .is_some_and(|row| Arc::ptr_eq(row, &parent.row))
            {
                return Err(VoiceError::Unauthorized);
            }
            let mut state = try_lock(&parent.row.state)?;
            original(parent, &state, session, request)?;
            super::pc::mutable_storage(&mut state).actions_closed[position(purpose)] = true;
            index
                .rows
                .iter()
                .flatten()
                .find(|row| {
                    row.key.matches(
                        selected,
                        Purpose::Sdp {
                            parent: parent.id(),
                            purpose,
                        },
                    ) && row
                        .key
                        .parent
                        .as_ref()
                        .is_some_and(|p| Arc::ptr_eq(p, &parent.row))
                })
                .cloned()
        };
        let resolved = row.map(|row| ResolvedRow {
            ledger: self.clone(),
            row,
        });
        if let Some(row) = &resolved {
            row.close_sdp_action(session)?;
        }
        Ok(resolved)
    }
    /// Unique original Row installation. Fee admission has already finished outside every
    /// native guard. No candidate Arc is allocated until uniqueness has been established
    /// under Index; construction/install then contain no fallible callback or Root work.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::voice_handoff) fn begin_sdp_action(
        self: &Arc<Self>,
        parent: &ResolvedRow,
        selected: &Arc<SelectedHandoffOwner>,
        admission: &Admission<'_>,
        candidate: &Candidate<'_>,
        wall: u64,
        mono: u64,
    ) -> Result<Begin, VoiceError> {
        if !Arc::ptr_eq(self, &parent.ledger)
            || !Arc::ptr_eq(self, admission.ledger())
            || !parent.row.key.matches(selected, parent.row.key.purpose)
            || !parent
                .row
                .key
                .matches(admission.selected(), parent.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let request = candidate.request;
        let funding = &candidate.funding;
        let purpose = funding.purpose();
        let body_tag = candidate.body_tag;
        let key = Purpose::Sdp {
            parent: parent.id(),
            purpose,
        };
        let mut index = try_lock(&self.index)?;
        if !index.rows[parent.id().slot]
            .as_ref()
            .is_some_and(|row| Arc::ptr_eq(row, &parent.row))
        {
            return Err(VoiceError::Unauthorized);
        }
        let state = try_lock(&parent.row.state)?;
        let (storage, intent) = original(parent, &state, admission.session(), request)?;
        let phase = storage.phase.as_ref().ok_or(VoiceError::Unauthorized)?;
        if funding.purpose() != purpose
            || !funding.uses_phase(phase)
            || !intent.uses_phase(phase)
            || !candidate.outcome.matches(intent, funding, body_tag)
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(row) = index
            .rows
            .iter()
            .flatten()
            .find(|row| row.key.matches(selected, key))
        {
            if !row
                .key
                .parent
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &parent.row))
            {
                return Err(VoiceError::Unauthorized);
            }
            let child = try_lock(&row.state)?;
            if self::body_tag(&child) != Some(body_tag) {
                return Err(VoiceError::RetryMismatch);
            }
            return Ok(Begin::Existing(ResolvedRow {
                ledger: self.clone(),
                row: row.clone(),
            }));
        }
        require_current(
            parent, &state, storage, intent, candidate, admission, wall, mono,
        )?;
        let slot = index.vacant().ok_or(VoiceError::Capacity)?;
        let serial = index.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        // Exact uniqueness and all fallible/current gates precede BOTH allocations.
        // Competing candidates must reuse this slot; no speculative Outcome is installed.
        let outcome = candidate.outcome.install();
        let row = Arc::new(Row {
            id: RowId { slot, serial },
            key: Key {
                grant: selected.grant_identity(),
                executor: selected.executor(),
                lease: selected.lease().clone(),
                purpose: key,
                parent: Some(parent.row.clone()),
            },
            state: Mutex::new(State {
                body: Body::SdpPending(Pending {
                    owners: Owners {
                        selected: selected.clone(),
                        phase: phase.clone(),
                        intent: intent.clone(),
                        session: admission.session().clone(),
                        outcome,
                        funding: funding.clone(),
                    },
                    published: None,
                    observer: false,
                    body_matches: false,
                    body_tag,
                    worker: true,
                    handed_off: false,
                    closed: false,
                }),
                status: Status {
                    pending: PendingStatus::Open,
                    action: None,
                    engine: CryptoResultStatus {
                        settled: false,
                        receipt: None,
                    },
                    acknowledged: false,
                    drained: false,
                },
                final_phase: None,
                session_collected: false,
                child: None,
            }),
            changed: Notify::new(),
            cleanup_waiter: AtomicBool::new(false),
            _metadata: RowPayment::Sdp(funding.clone()),
        });
        index.serial = serial;
        index.rows[slot] = Some(row.clone());
        Ok(Begin::Factory(Registration {
            ledger: self.clone(),
            row,
            #[cfg(test)]
            before_retire: None,
        }))
    }
}
/// Original fixed provenance for a separately paid producer. These clones grant no
/// fresh effect or worker claim; Registration remains affine and mark_handoff repeats
/// current Session/PC/SDK/intent checks. Original fixed-row funding covers its initial
/// prepayment snapshot; the later task also inventories its synchronous constructor use.
pub(in crate::voice_handoff) struct SubmissionOwners {
    pub phase: Arc<sdp_handoff::Phase>,
    pub intent: Arc<InitialSdpIntent>,
    pub funding: Arc<NativeRowFunding>,
    pub handshake: [u8; 32],
}
/// One paid immutable lookup target, not a second worker/Writer or current permission.
/// The actual affine Registration remains in the detached worker until sender retirement.
pub(in crate::voice_handoff) struct PublicationTarget {
    row: ResolvedRow,
    selected: Arc<SelectedHandoffOwner>,
    session: Arc<SessionIdentity>,
    phase: Arc<sdp_handoff::Phase>,
    intent: Arc<InitialSdpIntent>,
    outcome: Arc<Outcome>,
    funding: Arc<NativeRowFunding>,
    body: Option<[u8; 32]>,
}
impl PublicationTarget {
    /// Final UI/Coordinator/route fences are already borrowed. Hold original Index,
    /// parent and row through the Root tail; every owner stays outside these guards.
    /// A queued earlier handoff is uncertainty only and cannot authorize this effect.
    pub(in crate::voice_handoff) fn admit(
        &self,
        admission: &Admission<'_>,
        step: catcoms_sync::voice_v2::sdp::SdpPublicationAdmission<'_>,
    ) -> Result<(), VoiceError> {
        if !Arc::ptr_eq(&self.row.ledger, admission.ledger())
            || !Arc::ptr_eq(&self.session, admission.session())
            || !std::ptr::eq(self.selected.as_ref(), admission.selected())
            || !self
                .row
                .row
                .key
                .matches(admission.selected(), self.row.row.key.purpose)
            || !step.belongs_to_outcome(&self.outcome)
        {
            return Err(VoiceError::Unauthorized);
        }
        let index = self.row.transition_index()?;
        let parent = self
            .row
            .row
            .key
            .parent
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !index.rows[parent.id.slot]
            .as_ref()
            .is_some_and(|row| Arc::ptr_eq(row, parent))
        {
            return Err(VoiceError::Unauthorized);
        }
        let parent_state = try_lock(&parent.state)?;
        let claim = parent_state
            .plan()
            .and_then(|p| p.transition.as_ref())
            .ok_or(VoiceError::Unauthorized)?;
        let key = claim.key.as_ref().ok_or(VoiceError::Unauthorized)?;
        let storage = key.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
        let state = try_lock(&self.row.row.state)?;
        let Body::SdpPending(p) = &state.body else {
            return Err(VoiceError::Unauthorized);
        };
        let Purpose::Sdp { purpose, .. } = self.row.row.key.purpose else {
            return Err(VoiceError::Unauthorized);
        };
        if !p.worker
            || !p.handed_off
            || p.closed
            || p.published.is_some()
            // body_matches is populated by capture AFTER publication. Requiring it
            // here would reject every new action. The entered original Outcome binds
            // Root's actual payload before this scoped callback is lent.
            || p.body_tag != self.body
            || !Arc::ptr_eq(&p.owners.selected, &self.selected)
            || !Arc::ptr_eq(&p.owners.session, &self.session)
            || !Arc::ptr_eq(&claim.session, &self.session)
            || !Arc::ptr_eq(&p.owners.phase, &self.phase)
            || !Arc::ptr_eq(&p.owners.intent, &self.intent)
            || !Arc::ptr_eq(&p.owners.outcome, &self.outcome)
            || !Arc::ptr_eq(&p.owners.funding, &self.funding)
            || !self.funding.uses_phase(&self.phase)
            || self.funding.purpose() != purpose
            || self.funding.is_closed()
            || claim.released
            || key.released
            || !key.custody.is_current()
            || storage.closed
            || storage.actions_closed[position(purpose)]
            || storage.worker()
            || storage.sdk_preparing
            || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
            || storage
                .phase
                .as_ref()
                .is_none_or(|phase| !Arc::ptr_eq(phase, &self.phase))
            || storage
                .pc
                .as_ref()
                .and_then(|pc| pc.intent())
                .is_none_or(|intent| !Arc::ptr_eq(intent, &self.intent))
            || !self.row.row.key.lease.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let (wall, mono) = step.observed_time();
        self.intent.remaining_at(wall, mono)?;
        admission.current()?;
        step.commit()
    }
}
impl Registration {
    /// Construct only after the original native Task has paid this complete target.
    /// It grants no queue admission and cannot outlive its worker as effect authority.
    pub(in crate::voice_handoff) fn publication_target(
        &self,
        session: &Arc<SessionIdentity>,
    ) -> Result<PublicationTarget, VoiceError> {
        let resolved = self.resolved();
        let _index = resolved.transition_index()?;
        let state = try_lock(&self.row.state)?;
        let Body::SdpPending(p) = &state.body else {
            return Err(VoiceError::Unauthorized);
        };
        if !p.worker || !p.handed_off || p.closed || !Arc::ptr_eq(session, &p.owners.session) {
            return Err(VoiceError::Unauthorized);
        }
        Ok(PublicationTarget {
            row: self.resolved(),
            selected: p.owners.selected.clone(),
            session: session.clone(),
            phase: p.owners.phase.clone(),
            intent: p.owners.intent.clone(),
            outcome: p.owners.outcome.clone(),
            funding: p.owners.funding.clone(),
            body: p.body_tag,
        })
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_retirement_signal(
        mut self,
        signal: std::sync::mpsc::SyncSender<()>,
    ) -> Self {
        self.before_retire = Some(signal);
        self
    }
    pub(in crate::voice_handoff) fn submission_owners(
        &self,
        session: &Arc<SessionIdentity>,
    ) -> Result<SubmissionOwners, VoiceError> {
        let resolved = self.resolved();
        let _index = resolved.transition_index()?;
        let parent = self
            .row
            .key
            .parent
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let parent_state = try_lock(&parent.state)?;
        let state = try_lock(&self.row.state)?;
        let Body::SdpPending(p) = &state.body else {
            return Err(VoiceError::Unauthorized);
        };
        if !p.worker
            || p.closed
            || p.published.is_some()
            || !Arc::ptr_eq(session, &p.owners.session)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(SubmissionOwners {
            phase: p.owners.phase.clone(),
            intent: p.owners.intent.clone(),
            funding: p.owners.funding.clone(),
            handshake: parent_state
                .plan()
                .ok_or(VoiceError::Unauthorized)?
                .identity
                .handshake(),
        })
    }
    /// Obtain an affine token only from this canonical row while its actual worker still
    /// owns Registration. Allocation identity repeats at final native handoff admission.
    pub(in crate::voice_handoff) fn publication_writer(&self) -> Result<Writer, VoiceError> {
        let outcome = {
            let state = try_lock(&self.row.state)?;
            let Body::SdpPending(p) = &state.body else {
                return Err(VoiceError::Unauthorized);
            };
            if !p.worker || p.closed || p.published.is_some() {
                return Err(VoiceError::Unauthorized);
            }
            p.owners.outcome.clone()
        };
        outcome.try_writer()
    }
    pub(in crate::voice_handoff) fn id(&self) -> RowId {
        self.row.id
    }
    pub(in crate::voice_handoff) fn resolved(&self) -> ResolvedRow {
        ResolvedRow {
            ledger: self.ledger.clone(),
            row: self.row.clone(),
        }
    }
    /// Mark before awaiting mailbox publication, under the actual current native fence.
    /// Any uncertain handoff remains counted after this ticket's submitting waiter is lost.
    pub(in crate::voice_handoff) fn mark_handoff(
        &self,
        admission: &Admission<'_>,
        writer: &Writer,
        wall: u64,
        mono: u64,
    ) -> Result<(), VoiceError> {
        if !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let index = try_lock(&self.ledger.index)?;
        let parent = self
            .row
            .key
            .parent
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        for row in [parent, &self.row] {
            if !index.rows[row.id.slot]
                .as_ref()
                .is_some_and(|indexed| Arc::ptr_eq(indexed, row))
            {
                return Err(VoiceError::Unauthorized);
            }
        }
        let parent_state = try_lock(&parent.state)?;
        let storage = parent_state
            .plan()
            .and_then(|p| p.transition.as_ref())
            .and_then(|t| t.key.as_ref())
            .and_then(|key| key.sdp.as_ref())
            .ok_or(VoiceError::Unauthorized)?;
        let Purpose::Sdp { purpose, .. } = self.row.key.purpose else {
            return Err(VoiceError::Unauthorized);
        };
        let mut state = try_lock(&self.row.state)?;
        let Body::SdpPending(p) = &mut state.body else {
            return Err(VoiceError::Unauthorized);
        };
        if !p.worker
            || !writer.belongs_to(&p.owners.outcome)
            || p.closed
            || storage.closed
            || storage.actions_closed[position(purpose)]
            || p.owners.funding.is_closed()
            || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
            || storage
                .pc
                .as_ref()
                .and_then(|pc| pc.intent())
                .is_none_or(|intent| !Arc::ptr_eq(intent, &p.owners.intent))
            || !Arc::ptr_eq(&p.owners.session, admission.session())
            || !self.row.key.lease.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        p.owners.intent.remaining_at(wall, mono)?;
        admission.current()?;
        p.handed_off = true;
        Ok(())
    }
    /// Capture MUST precede every current/attachment/status observation. This pure path
    /// deliberately accepts the SAME real published Action after End. It calls no Clock,
    /// allocates no loan, and cannot discard a publication because attachment is refused.
    pub(in crate::voice_handoff) fn capture(
        &self,
        action: &Arc<CryptoAction>,
    ) -> Result<(), VoiceError> {
        let (matches, closure) = {
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            capture_locked(&self.row, &mut state, action, true)?
        };
        drop(closure);
        self.row.changed.notify_waiters();
        if matches {
            Ok(())
        } else {
            Err(VoiceError::RetryMismatch)
        }
    }
    /// Metadata attachment has no first-effect meaning and needs no live UI/PC. Retain
    /// original publication even after closure; subsequent fresh operations recheck currency.
    pub(in crate::voice_handoff) fn attach_published(&self) -> Result<(), VoiceError> {
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let Body::SdpPending(p) = &state.body else {
            return Err(VoiceError::Unauthorized);
        };
        if !p.worker || p.published.is_none() || !p.body_matches || p.observer {
            return Err(VoiceError::Unauthorized);
        }
        let Body::SdpPending(mut p) = std::mem::replace(&mut state.body, Body::ClosedPending)
        else {
            unreachable!("guarded original Pending");
        };
        let closed =
            p.closed || !p.owners.phase.effect().is_live() || !self.row.key.lease.is_live();
        let action = p.published.take().expect("original publication");
        state.body = Body::SdpLive(Live {
            owners: p.owners,
            action,
            delivery: None,
            body_tag: p.body_tag,
            worker: p.worker,
            observer: false,
            closed,
        });
        state.status.pending = PendingStatus::NotPending;
        Ok(())
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(signal) = self.before_retire.take() {
            let _ = signal.send(());
        }
        {
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            match &mut state.body {
                Body::SdpPending(p) => p.worker = false,
                Body::SdpLive(p) => p.worker = false,
                _ => {}
            }
        }
        self.row.changed.notify_waiters();
    }
}

#[cfg(test)]
impl ResolvedRow {
    pub(in crate::voice_handoff) fn sdp_test_hold_state<T>(&self, apply: impl FnOnce() -> T) -> T {
        let _state = self.row.state.lock().unwrap();
        apply()
    }
    pub(in crate::voice_handoff) fn sdp_test_notification(
        &self,
    ) -> tokio::sync::futures::Notified<'_> {
        self.row.changed.notified()
    }
}
#[cfg(test)]
impl Ledger {
    pub(in crate::voice_handoff) fn sdp_test_children_closed(&self, parent: &ResolvedRow) -> bool {
        let index = self.index.lock().unwrap();
        Self::children_locally_closed(&index, &parent.row)
    }
    pub(in crate::voice_handoff) fn sdp_test_rows(&self, parent: RowId) -> usize {
        self.index
            .lock()
            .unwrap()
            .rows
            .iter()
            .flatten()
            .filter(|row| matches!(row.key.purpose,Purpose::Sdp {parent:id,..} if id==parent))
            .count()
    }
}

/// Synchronous local compaction inventory, additional to its HTTP caller. Recovery,
/// reconciliation and retirement finish before the final Body replacement. The retired
/// enum value is still actual stack storage even though its fixed Arcs have original fees.
#[cfg(test)]
pub(in crate::voice_handoff) fn compaction_layout_bytes() -> usize {
    let replacement = std::mem::size_of::<Body>()
        + std::mem::size_of::<Drained>()
        + std::mem::size_of::<Option<Arc<CryptoAction>>>()
        + std::mem::size_of::<Option<Arc<SdpDelivery>>>()
        + std::mem::size_of::<Option<[u8; 32]>>()
        + 3 * std::mem::size_of::<Arc<()>>()
        + 3 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 128;
    let recovery = std::mem::size_of::<History>()
        + std::mem::size_of::<Arc<Outcome>>()
        + std::mem::size_of::<Option<sdp_handoff::NativeIoClosure>>()
        + 3 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 128;
    // Includes the actual Observation/status merge used by unstarted retirement too.
    replacement
        .max(recovery)
        .max(super::reconciliation_layout_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_native_sdp_row_and_original_fee_graph_fit_the_prepaid_history() {
        #[repr(C)]
        struct ArcRow {
            strong: std::sync::atomic::AtomicUsize,
            weak: std::sync::atomic::AtomicUsize,
            row: Row,
        }
        // Row includes the actual largest shared State/Body and its payment enum. Root
        // Action/delivery payloads have their own original fixed funding, not duplicate
        // native allocations. Variable SDK/IO/Actor graphs need separate original Copies.
        let bytes = std::mem::size_of::<ArcRow>()
            + NativeRowFunding::allocation_bytes()
            + Outcome::allocation_bytes()
            + std::mem::size_of::<Writer>()
            + 2 * std::mem::size_of::<History>()
            + std::mem::size_of::<SubmissionOwners>()
            + std::mem::size_of::<Registration>()
            + std::mem::size_of::<Begin>()
            + std::mem::size_of::<Candidate<'_>>()
            + 2 * std::mem::size_of::<ResolvedRow>()
            + std::mem::size_of::<Observation>()
            + 2 * std::mem::size_of::<Status>()
            + std::mem::size_of::<CryptoHandoffStatus>()
            + std::mem::size_of::<sdp_handoff::NativeIoClosure>()
            + 128;
        assert!(
            bytes <= sdp_handoff::NATIVE_ROW_BYTES,
            "native SDP fixed graph needs {bytes}"
        );
    }
}
