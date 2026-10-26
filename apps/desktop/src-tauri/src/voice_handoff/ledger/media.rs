//! One original-PC media obligation, separate from negotiation/SDK/storage cleanup.
//! Capacity attachment is deliberately independent from current first-effect admission:
//! an irreversible partition must never be lost merely because native UI/Index is busy.
use super::super::requests::{
    media::{ExistingKind, OriginalKind},
    sdp_action::Begin,
    Action,
};
use super::transition::{action_matches, try_lock};
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::{self, NativeRowFunding, NativeRowPurpose};
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, MediaAdmission, SdpMedia};
use sha2::{Digest, Sha256};

/// Fixed pre-candidate record, boxed to keep the shared2304B Row unchanged. ORIGINAL PC
/// metadata pays its heap BEFORE the detached8KiB child exists. No producer backedge lives here.
pub(super) struct Claim {
    proof: [u8; 32],
    until: u64,
    worker: bool,
    closed: bool,
    released: bool,
    admitted: bool,
    retire_allowed: bool,
    pub(super) root: Option<Arc<SdpMedia>>,
    pub(super) migrated: bool,
}
impl Claim {
    fn new(identity: &Identity) -> Self {
        Self {
            proof: identity.proof,
            until: 0,
            worker: false,
            closed: false,
            released: false,
            admitted: false,
            retire_allowed: false,
            root: None,
            migrated: false,
        }
    }
    pub(super) fn whole_drained(&self) -> bool {
        // Never-admitted preparation has no new browser PC debt. Its worker and the
        // original SDK/storage obligations still prevent premature final/orphan GC.
        !self.worker && (self.migrated || !self.admitted() || self.released)
    }
    fn admitted(&self) -> bool {
        self.admitted || self.root.as_ref().is_some_and(|root| root.was_admitted())
    }
    fn close(&mut self) {
        self.closed = true;
        if let Some(root) = &self.root {
            self.admitted |= root.was_admitted();
            root.close();
        }
    }
    fn negotiation_may_close(&self) -> bool {
        !self.admitted()
            || self.closed
            || self.released
            || self.retire_allowed
            || self.root.as_ref().is_some_and(|root| !root.is_current())
    }
    fn observe_expiry(&mut self, mono: u64) -> Result<u32, VoiceError> {
        if self.admitted() && mono >= self.until {
            self.retire_allowed = true;
        }
        if self.negotiation_may_close() {
            Ok(0)
        } else {
            u32::try_from(self.until.saturating_sub(mono)).map_err(|_| VoiceError::Bounds)
        }
    }
}
/// Ordinary post-handoff negotiation cleanup must not cancel still-authorized Answer
/// retries. End/epoch/media Close is different: the original Source is already revoked.
pub(super) fn negotiation_may_close(storage: &super::sdp::Claim) -> bool {
    storage
        .pc
        .as_ref()
        .and_then(|pc| pc.media.as_ref())
        .is_none_or(|media| media.negotiation_may_close())
}

/// Preallocated OUTSIDE final native guards. An unused second candidate is separately
/// included in Task/Control construction payment. The original phase anchor is LAST so
/// even failed/unattached allocation custody survives destruction of the candidate Box.
pub(in crate::voice_handoff) struct Prepared {
    candidate: Option<Box<Claim>>,
    phase: Arc<sdp_handoff::Phase>,
}
impl Prepared {
    fn take(
        &mut self,
        proof: [u8; 32],
        phase: &Arc<sdp_handoff::Phase>,
    ) -> Result<Box<Claim>, VoiceError> {
        if !Arc::ptr_eq(&self.phase, phase)
            || self
                .candidate
                .as_ref()
                .is_none_or(|claim| claim.proof != proof)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(self.candidate.take().expect("one prepared media claim"))
    }
    pub(in crate::voice_handoff) const fn heap_bytes() -> usize {
        std::mem::size_of::<Claim>()
    }
}

/// Privately prepared from the exact original authenticated native record, never wire IDs
/// alone. Hashing/Arc preparation is outside final UI/Coordinator/row/Group/Policy guards.
pub(in crate::voice_handoff) struct Identity {
    parent: RowId,
    session: Arc<SessionIdentity>,
    action: Action,
    pc: [u8; 32],
    proof: [u8; 32],
}
impl Identity {
    fn matches(
        &self,
        parent: &ResolvedRow,
        session: &Arc<SessionIdentity>,
        request: &Begin<'_>,
    ) -> bool {
        self.parent == parent.id()
            && Arc::ptr_eq(&self.session, session)
            && self.pc == request.proof
            && (
                self.action.serial,
                self.action.version,
                self.action.input_tag,
            ) == (
                request.pc.base.action.serial,
                request.pc.base.action.version,
                request.pc.base.action.input_tag,
            )
    }
}

/// Historical metadata only. `current` is true only on a separately current native admit/
/// status path, never merely because this original claim admitted in the past.
pub(in crate::voice_handoff) struct Receipt {
    pub proof: [u8; 32],
    pub admitted: bool,
    pub closed: bool,
    pub released: bool,
    pub current: bool,
    /// None on historical recovery, Some only from actual native-clock current admission.
    pub retire_in: Option<u32>,
}
fn receipt(claim: &Claim, current: bool) -> Receipt {
    Receipt {
        proof: claim.proof,
        admitted: claim.admitted(),
        closed: claim.closed,
        released: claim.released,
        current: current
            && !claim.closed
            && !claim.released
            && claim.admitted()
            && claim.root.as_ref().is_some_and(|root| root.is_current()),
        retire_in: None,
    }
}

fn original<'a>(
    parent: &ResolvedRow,
    state: &'a State,
    session: &Arc<SessionIdentity>,
    request: &Begin<'_>,
) -> Result<&'a super::pc::Claim, VoiceError> {
    if request.parent != parent.row.id || request.plaintext.is_some() {
        return Err(VoiceError::Unauthorized);
    }
    let storage = super::pc::storage(parent, state, session, &request.pc)?;
    let pc = storage.pc.as_ref().ok_or(VoiceError::Unauthorized)?;
    if !pc.matches(request.proof) {
        return Err(VoiceError::Unauthorized);
    }
    Ok(pc)
}
fn existing<'a>(
    parent: &ResolvedRow,
    state: &'a State,
    session: &Arc<SessionIdentity>,
    action: Action,
) -> Result<&'a super::pc::Claim, VoiceError> {
    if parent.row.key.parent.is_some()
        || !matches!(parent.row.key.purpose, Purpose::Derive(_))
        || !action_matches(state, action)
    {
        return Err(VoiceError::Unauthorized);
    }
    let transition = state
        .plan()
        .and_then(|p| p.transition.as_ref())
        .ok_or(VoiceError::Unauthorized)?;
    if !Arc::ptr_eq(&transition.session, session) {
        return Err(VoiceError::Unauthorized);
    }
    transition
        .key
        .as_ref()
        .and_then(|key| key.sdp.as_ref())
        .and_then(|sdp| sdp.pc.as_ref())
        .ok_or(VoiceError::Unauthorized)
}
/// Compute outside native final guards. This public metadata is not a capability: every
/// compact request additionally selects its stored exact claim under original Session/row.
fn proof(session: &SessionIdentity, parent: RowId, action: Action, pc: [u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"Mewtual native admitted media v1\0");
    hash.update(session.0);
    hash.update(parent.logical());
    hash.update(action.serial.to_be_bytes());
    hash.update(action.version.to_be_bytes());
    hash.update(action.input_tag);
    hash.update(pc);
    hash.finalize().into()
}
fn mutable(state: &mut State) -> &mut Claim {
    super::pc::mutable_storage(state)
        .pc
        .as_mut()
        .expect("original PC")
        .media
        .as_mut()
        .expect("original media claim")
}

/// Actual native work owner, retained through preparation AND publication/Actor return.
/// It retains the real paid parent, never the other way round. Field/payload destructors in
/// the enclosing producer must run before this final worker flag may become false.
pub(in crate::voice_handoff) struct Worker {
    parent: ResolvedRow,
}
impl Worker {
    /// Capacity-only attachment uses blocking Index -> Row OUTSIDE all UI/Coordinator/
    /// Route/Group/Policy guards. A transient try-lock refusal must not discard a partition.
    /// Close/Release can win before attachment; preserve their permanent negative state.
    pub(in crate::voice_handoff) fn attach(
        &self,
        candidate: Arc<SdpMedia>,
    ) -> Result<(), VoiceError> {
        let mut unattached = Some(candidate);
        let mut duplicate = false;
        let result;
        {
            let _index = self
                .parent
                .ledger
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut state = self
                .parent
                .row
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let pc = super::pc::mutable_storage(&mut state)
                .pc
                .as_mut()
                .expect("worker retains original PC");
            let intent = pc.intent().ok_or(VoiceError::Unauthorized)?;
            let valid = unattached
                .as_ref()
                .is_some_and(|candidate| candidate.uses_intent(intent));
            let media = pc
                .media
                .as_mut()
                .expect("worker retains original media claim");
            if !media.worker || !valid {
                result = Err(VoiceError::Unauthorized);
            } else if let Some(existing) = &media.root {
                if Arc::ptr_eq(existing, unattached.as_ref().unwrap()) {
                    // Reattaching the same paid result is recovery. Dropping our extra
                    // clone must never close the actual already retained preparation.
                    result = Ok(());
                    duplicate = true;
                } else {
                    result = Err(VoiceError::RetryMismatch);
                }
            } else {
                if media.closed || media.released {
                    unattached.as_ref().unwrap().close();
                }
                if !media.released {
                    media.root = unattached.take();
                }
                result = Ok(());
            }
        }
        if !duplicate {
            if let Some(candidate) = &unattached {
                candidate.close();
            }
        }
        drop(unattached); // Actual child/plan/weak allocation destruction is outside guards.
        result
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        {
            let _index = self
                .parent
                .ledger
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut state = self
                .parent
                .row
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            mutable(&mut state).worker = false;
        }
        self.parent.row.changed.notify_waiters();
    }
}

struct RowOwner {
    row: ResolvedRow,
    action: Arc<CryptoAction>,
    funding: Arc<NativeRowFunding>,
}
/// Immutable original registered rows, prepared after callback-bearing canonical history
/// OUTSIDE native guards. Fields stay private: wire metadata cannot construct this binding.
pub(in crate::voice_handoff) struct Rows {
    seal: RowOwner,
    open: RowOwner,
}
impl Rows {
    pub(in crate::voice_handoff) fn seal(&self) -> &Arc<CryptoAction> {
        &self.seal.action
    }
    pub(in crate::voice_handoff) fn open(&self) -> &Arc<CryptoAction> {
        &self.open.action
    }
}
fn row_binding(
    owner: &RowOwner,
    parent: &ResolvedRow,
    selected: &SelectedHandoffOwner,
    session: &Arc<SessionIdentity>,
    phase: &Arc<sdp_handoff::Phase>,
    state: &State,
    purpose: NativeRowPurpose,
) -> Result<(), VoiceError> {
    if !Arc::ptr_eq(&owner.row.ledger, &parent.ledger)
        || owner.row.row.key.purpose
            != (Purpose::Sdp {
                parent: parent.id(),
                purpose,
            })
        || !owner
            .row
            .row
            .key
            .matches(selected, owner.row.row.key.purpose)
        || !owner
            .row
            .row
            .key
            .parent
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(p, &parent.row))
        || !super::sdp_rows::session(state).is_some_and(|s| Arc::ptr_eq(s, session))
    {
        return Err(VoiceError::Unauthorized);
    }
    let actual = super::sdp_rows::media_row(state, purpose)?;
    if !Arc::ptr_eq(actual.action, &owner.action)
        || !Arc::ptr_eq(actual.funding, &owner.funding)
        || !actual.funding.uses_phase(phase)
    {
        return Err(VoiceError::Unauthorized);
    }
    Ok(())
}
impl Ledger {
    /// Observe exact original completed rows without entering native UI/Coordinator/Route.
    /// Open Applied comes from the SAME actual Root Action's canonical history. Its native
    /// Live/Drained delivery cache is not an authority source and may legitimately be empty.
    pub(in crate::voice_handoff) fn media_rows(
        self: &Arc<Self>,
        parent: &ResolvedRow,
        selected: &Arc<SelectedHandoffOwner>,
        session: &Arc<SessionIdentity>,
        phase: &Arc<sdp_handoff::Phase>,
    ) -> Result<Rows, VoiceError> {
        let observe = |purpose| -> Result<RowOwner, VoiceError> {
            let row = self
                .lookup_original(
                    selected,
                    Purpose::Sdp {
                        parent: parent.id(),
                        purpose,
                    },
                )
                .ok_or(VoiceError::Capacity)?;
            let history = row.sdp_history(session)?; // Root Clock/Engine callbacks run outside all native gates.
            if purpose == NativeRowPurpose::Open
                && !history.delivery().is_some_and(|d| d.was_applied())
            {
                return Err(VoiceError::Unauthorized);
            }
            let (serial, version, input_tag) =
                history.status().action.ok_or(VoiceError::Capacity)?;
            let action = row.sdp_result_action(
                session,
                Action {
                    serial,
                    version,
                    input_tag,
                },
            )?;
            let owner;
            {
                let _index = row.transition_index()?;
                let state = try_lock(&row.row.state)?;
                let actual = super::sdp_rows::media_row(&state, purpose)?;
                if !Arc::ptr_eq(actual.action, &action)
                    || !actual.funding.uses_phase(phase)
                    || !super::sdp_rows::session(&state).is_some_and(|s| Arc::ptr_eq(s, session))
                    || !row
                        .row
                        .key
                        .parent
                        .as_ref()
                        .is_some_and(|p| Arc::ptr_eq(p, &parent.row))
                {
                    return Err(VoiceError::Unauthorized);
                }
                owner = RowOwner {
                    row: ResolvedRow {
                        ledger: row.ledger.clone(),
                        row: row.row.clone(),
                    },
                    action: action.clone(),
                    funding: actual.funding.clone(),
                };
            }
            Ok(owner)
        };
        let seal = observe(NativeRowPurpose::Seal)?;
        let open = observe(NativeRowPurpose::Open)?;
        if seal.row.id() == open.row.id() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Rows { seal, open })
    }
}

impl ResolvedRow {
    /// Funding/identity preparation only; negative commands also work before pc.intent
    /// exists. The original64KiB metadata is already prepaid, independent of Root media.
    fn prepare_media_claim(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Begin<'_>,
        identity: &Identity,
    ) -> Result<Prepared, VoiceError> {
        let phase = {
            let _index = self.transition_index()?;
            let state = try_lock(&self.row.state)?;
            original(self, &state, session, request)?;
            if !identity.matches(self, session, request) {
                return Err(VoiceError::Unauthorized);
            }
            super::pc::storage(self, &state, session, &request.pc)?
                .phase
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?
                .clone()
        };
        Ok(Prepared {
            candidate: Some(Box::new(Claim::new(identity))),
            phase,
        })
    }
    pub(in crate::voice_handoff) fn media_prepare_claim(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Begin<'_>,
        identity: &Identity,
    ) -> Result<Prepared, VoiceError> {
        self.prepare_media_claim(session, request, identity)
    }
    /// Fresh native currentness for an EXISTING admitted media claim. Old negotiation/key/
    /// Ready closure is intentionally not consulted. Actual UI/selection plus original
    /// Root call/pair/actor/session/policy/epoch and own media Close still revoke it.
    pub(in crate::voice_handoff) fn media_current(
        &self,
        admission: &super::super::transition::Admission<'_>,
        action: Action,
        claimed: [u8; 32],
        mono: u64,
    ) -> Result<Receipt, VoiceError> {
        if !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let _index = self.transition_index()?;
        let mut state = try_lock(&self.row.state)?;
        let pc = existing(self, &state, admission.session(), action)?;
        let _media = pc
            .media
            .as_ref()
            .filter(|media| media.proof == claimed)
            .ok_or(VoiceError::Unauthorized)?;
        admission.current()?;
        let media = mutable(&mut state);
        let remaining = media.observe_expiry(mono)?;
        let mut output = receipt(media, true);
        output.retire_in = Some(remaining);
        Ok(output)
    }
    /// Consume the original Root tail while actual UI/Coordinator/Route and ordered native
    /// Index -> parent -> Seal -> Open states remain held. No history/Clock callback or
    /// owned graph drops below these guards. Root adds its own try Group/Policy gates.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::voice_handoff) fn commit_media(
        &self,
        admission: &super::super::transition::Admission<'_>,
        request: &Begin<'_>,
        identity: &Identity,
        rows: &Rows,
        phase: &Arc<sdp_handoff::Phase>,
        intent: &Arc<InitialSdpIntent>,
        media: &Arc<SdpMedia>,
        step: MediaAdmission<'_>,
    ) -> Result<Receipt, VoiceError> {
        if !identity.matches(self, admission.session(), request)
            || !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
            || !step.matches(media, intent)
        {
            return Err(VoiceError::Unauthorized);
        }
        let output;
        {
            let index = self.transition_index()?;
            for owner in [&rows.seal, &rows.open] {
                if !index.rows[owner.row.id().slot]
                    .as_ref()
                    .is_some_and(|actual| Arc::ptr_eq(actual, &owner.row.row))
                {
                    return Err(VoiceError::Unauthorized);
                }
            }
            let mut state = try_lock(&self.row.state)?;
            let seal = try_lock(&rows.seal.row.row.state)?;
            let open = try_lock(&rows.open.row.row.state)?;
            let pc = original(self, &state, admission.session(), request)?;
            let key = super::sdp::claim(self, &state, admission.session(), &request.pc.base)?;
            let storage = super::pc::storage(self, &state, admission.session(), &request.pc)?;
            if key.released
                || !key.custody.is_current()
                || storage.closed
                || storage.worker()
                || storage.sdk_preparing
                || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
                || state.final_phase.is_some()
                || state.final_delivery().is_some()
                || pc.worker()
                || !pc
                    .intent()
                    .is_some_and(|stored| Arc::ptr_eq(stored, intent))
                || !storage
                    .phase
                    .as_ref()
                    .is_some_and(|stored| Arc::ptr_eq(stored, phase))
            {
                return Err(VoiceError::Unauthorized);
            }
            let claim = pc.media.as_ref().ok_or(VoiceError::Unauthorized)?;
            if !claim.worker
                || claim.closed
                || claim.released
                || claim.admitted()
                || claim.proof != identity.proof
                || !claim
                    .root
                    .as_ref()
                    .is_some_and(|root| Arc::ptr_eq(root, media))
            {
                return Err(VoiceError::Unauthorized);
            }
            row_binding(
                &rows.seal,
                self,
                admission.selected(),
                admission.session(),
                phase,
                &seal,
                NativeRowPurpose::Seal,
            )?;
            row_binding(
                &rows.open,
                self,
                admission.selected(),
                admission.session(),
                phase,
                &open,
                NativeRowPurpose::Open,
            )?;
            let (wall, mono) = step.observed_time();
            let remaining =
                u32::try_from(intent.remaining_at(wall, mono)?).map_err(|_| VoiceError::Bounds)?;
            let until = mono
                .checked_add(u64::from(remaining))
                .ok_or(VoiceError::Bounds)?;
            admission.current()?;
            // Stamp immutable capacity metadata before the effect. An exceptional tail
            // after Root admission must never leave a zero deadline that Current could
            // mistake for already expired Source authority. No grant is created here.
            let claim = mutable(&mut state);
            if claim.until != 0 && claim.until != until {
                return Err(VoiceError::Unauthorized);
            }
            claim.until = until;
            step.commit()?;
            // No fallible allocation/callback after actual Root publication. A lost reply
            // recovers THIS canonical claim; it never needs another partition/admission.
            let claim = mutable(&mut state);
            claim.admitted = true;
            let mut snapshot = receipt(claim, true);
            snapshot.retire_in = Some(remaining);
            output = snapshot;
        }
        Ok(output)
    }
    pub(in crate::voice_handoff) fn media_identity(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Begin<'_>,
    ) -> Result<Identity, VoiceError> {
        let pc = {
            let _index = self.transition_index()?;
            let state = try_lock(&self.row.state)?;
            original(self, &state, session, request)?.proof()
        };
        Ok(Identity {
            parent: self.id(),
            session: session.clone(),
            action: request.pc.base.action,
            pc,
            proof: proof(session, self.id(), request.pc.base.action, pc),
        })
    }
    /// Original capacity/provenance only. The producer pays its measured task BEFORE
    /// observing large history or constructing a callback. First effect admission remains
    /// separate and always rechecks the same native UI/Session/key/PC and canonical rows.
    pub(in crate::voice_handoff) fn media_original_parts(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Begin<'_>,
    ) -> Result<(Arc<sdp_handoff::Phase>, Arc<InitialSdpIntent>), VoiceError> {
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        let pc = original(self, &state, session, request)?;
        let storage = super::pc::storage(self, &state, session, &request.pc)?;
        Ok((
            storage
                .phase
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?
                .clone(),
            pc.intent().ok_or(VoiceError::Capacity)?.clone(),
        ))
    }
    /// Publish the finite worker marker BEFORE the irreversible capacity partition.
    /// A refused Actor submission retains its candidate in this same claim for retry.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::voice_handoff) fn begin_media_worker(
        &self,
        admission: &super::super::transition::Admission<'_>,
        request: &Begin<'_>,
        physical: &super::super::channel::PhysicalHold,
        session: &Arc<super::super::channel::Session>,
        identity: &Identity,
        prepared: &mut Prepared,
        wall: u64,
        mono: u64,
    ) -> Result<(Worker, Option<Arc<SdpMedia>>), VoiceError> {
        if !identity.matches(self, session.identity(), request)
            || !physical.belongs_to(session)
            || !Arc::ptr_eq(session.identity(), admission.session())
            || !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let output;
        {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            let pc = original(self, &state, admission.session(), request)?;
            let key = super::sdp::claim(self, &state, admission.session(), &request.pc.base)?;
            let storage = super::pc::storage(self, &state, admission.session(), &request.pc)?;
            if key.released
                || !key.custody.is_current()
                || storage.closed
                || storage.worker()
                || storage.sdk_preparing
                || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
                || state.final_phase.is_some()
                || state.final_delivery().is_some()
                || pc.worker()
                || pc
                    .media
                    .as_ref()
                    .is_some_and(|m| m.closed || m.released || m.admitted())
            {
                return Err(VoiceError::Unauthorized);
            }
            if pc.media.as_ref().is_some_and(|m| m.worker) {
                return Err(VoiceError::Capacity);
            }
            pc.intent()
                .ok_or(VoiceError::Capacity)?
                .remaining_at(wall, mono)?;
            admission.current()?;
            let pc = super::pc::mutable_storage(&mut state).pc.as_mut().unwrap();
            if pc.media.is_none() {
                let phase = super::pc::storage(self, &state, admission.session(), &request.pc)?
                    .phase
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?;
                let candidate = prepared.take(identity.proof, phase)?;
                super::pc::mutable_storage(&mut state)
                    .pc
                    .as_mut()
                    .unwrap()
                    .media = Some(candidate);
            }
            let media = super::pc::mutable_storage(&mut state)
                .pc
                .as_mut()
                .unwrap()
                .media
                .as_mut()
                .unwrap();
            if media.proof != identity.proof {
                return Err(VoiceError::Unauthorized);
            }
            media.worker = true;
            output = (
                Worker {
                    parent: ResolvedRow {
                        ledger: self.ledger.clone(),
                        row: self.row.clone(),
                    },
                },
                media.root.clone(),
            );
        }
        Ok(output)
    }
    /// Authenticated full-selector recovery/negative commands, also after End. Resolve of
    /// absent state creates nothing; Close/Release permanently fence late preparation.
    pub(in crate::voice_handoff) fn media_original_history(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Begin<'_>,
        kind: OriginalKind,
    ) -> Result<Option<Receipt>, VoiceError> {
        if kind == OriginalKind::Admit {
            return Err(VoiceError::Unauthorized);
        }
        let identity = self.media_identity(session, request)?;
        // Resolve creates nothing. Negative commands prepare outside all final guards,
        // then preserve whichever permanent claim won before they acquired the row.
        let mut prepared = if matches!(kind, OriginalKind::Close | OriginalKind::Release) {
            Some(self.prepare_media_claim(session, request, &identity)?)
        } else {
            None
        };
        let retired;
        let output;
        {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            original(self, &state, session, request)?;
            let pc = super::pc::mutable_storage(&mut state).pc.as_mut().unwrap();
            if pc.media.is_none() && kind == OriginalKind::Resolve {
                return Ok(None);
            }
            if pc.media.is_none() {
                let phase = super::pc::storage(self, &state, session, &request.pc)?
                    .phase
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?;
                let candidate = prepared
                    .as_mut()
                    .ok_or(VoiceError::Unauthorized)?
                    .take(identity.proof, phase)?;
                super::pc::mutable_storage(&mut state)
                    .pc
                    .as_mut()
                    .unwrap()
                    .media = Some(candidate);
            }
            let media = super::pc::mutable_storage(&mut state)
                .pc
                .as_mut()
                .unwrap()
                .media
                .as_mut()
                .unwrap();
            if media.proof != identity.proof {
                return Err(VoiceError::Unauthorized);
            }
            if matches!(kind, OriginalKind::Close | OriginalKind::Release) {
                media.close();
            }
            retired = if kind == OriginalKind::Release {
                media.released = true;
                media.root.take()
            } else {
                None
            };
            output = receipt(media, false);
        }
        drop(retired);
        self.row.changed.notify_waiters();
        Ok(Some(output))
    }
    /// Compact cleanup requires a previously created exact claim. A recomputed proof or
    /// same-byte foreign Session cannot bootstrap an obligation or fresh media admission.
    pub(in crate::voice_handoff) fn media_existing_history(
        &self,
        session: &Arc<SessionIdentity>,
        action: Action,
        claimed: [u8; 32],
        kind: ExistingKind,
    ) -> Result<Receipt, VoiceError> {
        if matches!(
            kind,
            ExistingKind::Current | ExistingKind::Detach | ExistingKind::RetireHistory
        ) {
            return Err(VoiceError::Unauthorized);
        }
        let retired;
        let output;
        {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            let pc = existing(self, &state, session, action)?;
            if pc.media.as_ref().is_none_or(|media| media.proof != claimed) {
                return Err(VoiceError::Unauthorized);
            }
            let media = mutable(&mut state);
            media.close();
            retired = if kind == ExistingKind::Release {
                media.released = true;
                media.root.take()
            } else {
                None
            };
            output = receipt(media, false);
        }
        drop(retired);
        self.row.changed.notify_waiters();
        Ok(output)
    }
}

#[cfg(test)]
impl ResolvedRow {
    /// Observe only the original public bindings after a real browser/native round.
    /// This does not install a claim, accept a result, or manufacture admission evidence.
    pub(in crate::voice_handoff) fn media_test_original(
        &self,
        session: &Arc<SessionIdentity>,
    ) -> Begin<'static> {
        let _index = self.transition_index().unwrap();
        let state = self.row.state.lock().unwrap();
        let (serial, version, input_tag) = state.status.action.unwrap();
        let plan = state.plan().unwrap();
        let transition = plan.transition.as_ref().unwrap();
        assert!(Arc::ptr_eq(&transition.session, session));
        let key = transition.key.as_ref().unwrap();
        let storage = key.sdp.as_ref().unwrap();
        let (sdk_generation, sdk) = storage.sdk.as_ref().unwrap().identity();
        Begin {
            parent: self.id(),
            pc: super::super::requests::pc::Request {
                base: super::super::requests::sdp::Request {
                    action: Action {
                        serial,
                        version,
                        input_tag,
                    },
                    handshake: plan.identity.handshake(),
                    key: key.receipt,
                    sequence: storage.sequence,
                    kind: super::super::requests::sdp::Kind::Prepare,
                },
                storage: storage.proof,
                sdk_generation,
                sdk,
                kind: super::super::requests::pc::Kind::Admit,
            },
            proof: storage.pc.as_ref().unwrap().proof(),
            plaintext: None,
        }
    }
    /// The SAME retained child is required across local retryable refusals.
    pub(in crate::voice_handoff) fn media_test_root(&self) -> Option<Arc<SdpMedia>> {
        let _index = self.transition_index().unwrap();
        let state = self.row.state.lock().unwrap();
        state
            .plan()
            .unwrap()
            .transition
            .as_ref()
            .unwrap()
            .key
            .as_ref()
            .unwrap()
            .sdp
            .as_ref()
            .unwrap()
            .pc
            .as_ref()
            .unwrap()
            .media
            .as_ref()
            .and_then(|media| media.root.clone())
    }
    pub(crate) fn media_test_until(&self) -> u64 {
        let state = self.row.state.lock().unwrap();
        state
            .plan()
            .and_then(|plan| plan.transition.as_ref())
            .and_then(|claim| claim.key.as_ref())
            .and_then(|key| key.sdp.as_ref())
            .and_then(|sdp| sdp.pc.as_ref())
            .and_then(|pc| pc.media.as_ref())
            .unwrap()
            .until
    }
    pub(crate) fn media_test_final_progress(&self) -> (bool, bool, bool) {
        let state = self.row.state.lock().unwrap();
        state
            .final_delivery()
            .map_or((false, false, false), |delivery| {
                (
                    delivery.kind == FinalKind::Plan,
                    delivery.issued.is_some(),
                    delivery.acknowledged,
                )
            })
    }
    pub(crate) fn media_test_migrated(&self) -> bool {
        let state = self.row.state.lock().unwrap();
        state
            .plan()
            .and_then(|plan| plan.transition.as_ref())
            .and_then(|claim| claim.key.as_ref())
            .and_then(|key| key.sdp.as_ref())
            .and_then(|sdp| sdp.pc.as_ref())
            .and_then(|pc| pc.media.as_ref())
            .is_some_and(|media| media.migrated && media.root.is_none())
    }
    pub(in crate::voice_handoff) fn media_test_transition_proof(&self) -> [u8; 32] {
        let _index = self.transition_index().unwrap();
        self.row
            .state
            .lock()
            .unwrap()
            .plan()
            .unwrap()
            .transition
            .as_ref()
            .unwrap()
            .receipt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inert() -> Claim {
        let mut logical = [0; 16];
        logical[15] = 1;
        Claim::new(&Identity {
            parent: RowId::from_logical(logical).unwrap(),
            session: Arc::new(SessionIdentity([1; 32])),
            action: Action {
                serial: 1,
                version: 1,
                input_tag: [2; 32],
            },
            pc: [3; 32],
            proof: [4; 32],
        })
    }
    #[test]
    fn never_admitted_refusal_or_close_has_no_new_sdk_release_debt() {
        let mut claim = inert();
        assert!(
            claim.whole_drained(),
            "a refused inert preparation has no media PC"
        );
        claim.close();
        assert!(claim.closed);
        assert!(!claim.admitted());
        assert!(
            claim.whole_drained(),
            "old SDK/key drain must not be stranded by a negative marker"
        );
    }
    #[test]
    fn actual_native_preparation_must_retire_even_for_never_admitted_or_released_state() {
        let mut claim = inert();
        claim.worker = true;
        assert!(!claim.whole_drained());
        claim.close();
        claim.released = true;
        assert!(
            !claim.whole_drained(),
            "a release reply is not actual worker completion"
        );
        claim.worker = false;
        assert!(claim.whole_drained());
    }
    #[test]
    fn admitted_media_requires_separate_release_even_after_close_and_negotiation_retirement() {
        let mut claim = inert();
        claim.admitted = true;
        claim.until = 9000;
        assert!(!claim.whole_drained());
        claim.close();
        claim.retire_allowed = true;
        assert!(
            !claim.whole_drained(),
            "Close/old Source expiry cannot claim the browser peer drained"
        );
        claim.released = true;
        assert!(claim.whole_drained());
        assert!(
            receipt(&claim, false).admitted,
            "release keeps original admission history"
        );
    }
    #[test]
    fn source_retirement_uses_original_deadline_and_survives_a_lost_current_reply() {
        let mut claim = inert();
        claim.admitted = true;
        claim.until = 9000;
        assert_eq!(claim.observe_expiry(8999), Ok(1));
        assert!(!claim.negotiation_may_close());
        assert_eq!(claim.observe_expiry(9000), Ok(0));
        assert!(claim.negotiation_may_close());
        // Discard the projection: the actual old deadline/latch survives a lost reply.
        {
            let _lost = receipt(&claim, false);
        }
        assert_eq!(claim.observe_expiry(9500), Ok(0));
        assert_eq!(claim.until, 9000);
        assert_eq!(
            claim.observe_expiry(1),
            Ok(0),
            "a different/rolled-back sample cannot rearm it"
        );
    }
}

/// Snapshot the same original claim before allocating its independently paid index entry.
pub(super) fn migration_parts(
    parent: &ResolvedRow,
    selected: &SelectedHandoffOwner,
    session: &Arc<SessionIdentity>,
    action: Action,
    claimed: [u8; 32],
    mono: u64,
) -> Result<(Begin<'static>, Arc<SdpMedia>, u64), VoiceError> {
    let _index = parent.transition_index()?;
    let state = try_lock(&parent.row.state)?;
    if !parent.row.key.matches(selected, parent.row.key.purpose) {
        return Err(VoiceError::Unauthorized);
    }
    let pc = existing(parent, &state, session, action)?;
    let media = pc.media.as_ref().ok_or(VoiceError::Unauthorized)?;
    if media.proof != claimed
        || media.worker
        || media.migrated
        || media.closed
        || media.released
        || !media.admitted()
    {
        return Err(VoiceError::Unauthorized);
    }
    if media.until == 0 {
        return Err(VoiceError::Unauthorized);
    }
    if mono < media.until {
        return Err(VoiceError::Capacity);
    }
    let root = media.root.as_ref().ok_or(VoiceError::Unauthorized)?;
    if !root.is_current() {
        return Err(VoiceError::Unauthorized);
    }
    let plan = state.plan().ok_or(VoiceError::Unauthorized)?;
    let key = plan
        .transition
        .as_ref()
        .and_then(|claim| claim.key.as_ref())
        .ok_or(VoiceError::Unauthorized)?;
    let storage = key.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
    let (sdk_generation, sdk) = storage
        .sdk
        .as_ref()
        .ok_or(VoiceError::Unauthorized)?
        .identity();
    let original = Begin {
        parent: parent.id(),
        pc: super::super::requests::pc::Request {
            base: super::super::requests::sdp::Request {
                action,
                handshake: plan.identity.handshake(),
                key: key.receipt,
                sequence: storage.sequence,
                kind: super::super::requests::sdp::Kind::Prepare,
            },
            storage: storage.proof,
            sdk_generation,
            sdk,
            kind: super::super::requests::pc::Kind::Admit,
        },
        proof: pc.proof(),
        plaintext: None,
    };
    Ok((original, root.clone(), media.until))
}
/// Borrow only through the original final UI/Index/Row tail. A failed index publication
/// leaves this owner in place; observing actual source expiry is independently legitimate.
// Keep each original authority/immutable preparation binding explicit at this final gate.
#[allow(clippy::too_many_arguments)]
pub(super) fn migration_claim<'a>(
    parent: &ResolvedRow,
    state: &'a mut State,
    session: &Arc<SessionIdentity>,
    action: Action,
    claimed: [u8; 32],
    root: &Arc<SdpMedia>,
    until: u64,
    mono: u64,
) -> Result<&'a mut Claim, VoiceError> {
    let pc = existing(parent, state, session, action)?;
    let media = pc.media.as_ref().ok_or(VoiceError::Unauthorized)?;
    if media.proof != claimed
        || media.worker
        || media.migrated
        || media.closed
        || media.released
        || !media.admitted()
        || !media
            .root
            .as_ref()
            .is_some_and(|stored| Arc::ptr_eq(stored, root))
        || !root.is_current()
    {
        return Err(VoiceError::Unauthorized);
    }
    if media.until != until || media.until == 0 {
        return Err(VoiceError::Unauthorized);
    }
    if mono < media.until {
        return Err(VoiceError::Capacity);
    }
    let media = mutable(state);
    media.observe_expiry(mono)?;
    Ok(media)
}
