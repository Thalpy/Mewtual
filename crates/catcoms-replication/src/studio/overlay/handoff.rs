//! Local transfer metadata. Decoding this record confers neither authority nor durability.
use super::*;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;

mod preparation;
pub use preparation::{StudioHandoffAuthority, StudioHandoffSigning};

use super::disposal::{get_provenance, put_provenance};

#[derive(Debug)]
pub enum StudioOverlaySave {
    Local(StudioLocalDraft),
    HandedOff(StudioHandoffOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioHandoffOutcome {
    pub basis: [u8; 32],
    pub epoch: u64,
    pub doc_id: u128,
    pub accepted: usize,
}

#[derive(Clone)]
struct Completed {
    target: StudioTarget,
    author: DeviceId,
    outcome: StudioHandoffOutcome,
    entries: Vec<Entry>,
}

#[derive(Clone)]
struct Prepared {
    epoch: u64,
    doc_id: u128,
    receipt: [u8; 32],
    before: [u8; 32],
    branch: [u8; 32],
    signed: Vec<[u8; 32]>,
}

/// Read-only local provenance. The mandatory target remains when the seed and ledger disappear.
#[derive(Clone)]
pub struct StudioOverlayState {
    target: StudioTarget,
    active: Option<StudioOverlay>,
    prepared: Option<Prepared>,
    completed: Option<Completed>,
    /// The retained terminal acknowledgement of a branch that was dropped rather than
    /// transferred. Deliberately independent of `active`: a vault can hold this alone, and can
    /// equally hold it beside a **new** branch started after the disposal, which is the state a
    /// later request classifies against.
    disposed: Option<StudioOverlayDisposal>,
    /// Monotonic per logical document, starting at 1. Incremented exactly once when a branch is
    /// first accepted where none existed, never reset, never reused, never decremented.
    ///
    /// This is the rollover defence. `minimum_new_basis_closed_epoch` is deliberately *not*
    /// advanced by a disposal, because a fresh Save on a still-eligible basis after a disposal is a
    /// legitimate new decision; what must not happen is an old request for a disposed branch being
    /// accepted as a new one. Binding the generation into the branch identity makes such a request
    /// name a namespace that no longer exists.
    branch_generation: u64,
    /// How this branch's base was obtained. A property of the mint, not of the basis blob, which is
    /// why it has to be carried: the nested basis alone cannot say whether it came from an installed
    /// Closing source or an unconfirmed preview.
    provenance: StudioOverlayProvenance,
    minimum_new_basis_closed_epoch: u64,
    legacy: bool,
}

/// Which terminal event, if any, an incoming request's `branch` names.
///
/// Exactly one arm can match, and the order of matching is part of the contract: live branch, then
/// transferred manifest, then disposed manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudioOverlayRequestClass {
    /// `branch` names the live branch; ordinary exact-retry or append applies.
    Active,
    /// `branch` names the retained transferred manifest, with a matching envelope.
    Transferred(StudioHandoffOutcome),
    /// `branch` names the retained disposal manifest, with a matching envelope.
    Disposed(Box<StudioOverlayDisposal>),
    /// None of the identities this record holds. **Not a verdict.** `branch_id` is a hash of a
    /// basis this stage deliberately does not mint, so it genuinely cannot tell a legitimate
    /// next-generation request from a stale one; `admit_new_branch` resolves it where a fresh basis
    /// exists. Reaching this arm does mean no terminal acknowledgement is owed.
    Unmatched,
}

/// The resolution of [`StudioOverlayRequestClass::Unmatched`], available only where a fresh basis
/// has just been minted under live authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOverlayAdmission {
    /// First acceptance of the derived next generation. The increment and the first accepted
    /// envelope become durable in the **same** sealed replacement, so there is no reserved but
    /// uncommitted generation and no second durable transition.
    New { generation: u64 },
    /// An older generation, a skipped generation, an unrelated basis, or an unknown identity.
    Stale,
}

/// `H("catcoms/studio-overlay-branch/v1", basis fingerprint, branch_generation)`.
///
/// An identifier, never authority. It cannot be inverted, which is exactly why the structural
/// classification stage cannot resolve `Unmatched` on its own.
fn branch_identity(basis: [u8; 32], generation: u64) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-overlay-branch/v1");
    hash.update(&basis);
    hash.update(&generation.to_be_bytes());
    *hash.finalize().as_bytes()
}

impl std::fmt::Debug for StudioOverlayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioOverlayState")
            .field("prepared", &self.prepared.is_some())
            .field("completed", &self.completed.is_some())
            .field("disposed", &self.disposed.is_some())
            .finish_non_exhaustive()
    }
}

/// A separate signed source plus its exact local manifest, never produced by a decoder.
#[derive(Debug)]
pub struct StudioHandoffCandidate {
    source: StudioEpoch,
    metadata: StudioOverlayState,
}
impl StudioHandoffCandidate {
    pub fn into_parts(self) -> (StudioEpoch, StudioOverlayState) {
        (self.source, self.metadata)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioHandoffEvidence {
    /// No branch id exists in the same physical destination. Full local work must remain.
    Absent,
    /// Every full signed change, not merely its domain envelope, is retained.
    Complete,
    /// Partial, conflicting or different physical source. No completion or prefix replay.
    Hold,
}

impl StudioOverlayState {
    /// The first branch of a logical document: generation 1, `Closing` provenance.
    ///
    /// Kept as the plain constructor so that every existing caller and every existing vault is
    /// unchanged, and so a state built this way still encodes as v2 byte-for-byte. A branch admitted
    /// after a disposal uses [`Self::new_admitted`] instead.
    pub fn new(basis: &StudioClosingOverlayBasis) -> Self {
        Self {
            target: basis.0.target,
            active: Some(StudioOverlay::new(basis)),
            prepared: None,
            completed: None,
            disposed: None,
            branch_generation: 1,
            provenance: StudioOverlayProvenance::Closing,
            minimum_new_basis_closed_epoch: 0,
            legacy: false,
        }
    }

    /// A branch accepted where none existed, carrying forward what this vault already holds.
    ///
    /// The generation comes from [`StudioOverlayAdmission::New`], which is the only thing that
    /// derives it, so the increment cannot happen anywhere else. Any retained terminal manifest is
    /// preserved: a new branch does not erase the acknowledgement owed for the previous one.
    pub fn new_admitted(
        &self,
        basis: &StudioClosingOverlayBasis,
        admission: StudioOverlayAdmission,
        provenance: StudioOverlayProvenance,
    ) -> Result<Self, ReplError> {
        let StudioOverlayAdmission::New { generation } = admission else {
            return Err(ReplError::IntentConflict);
        };
        if self.active.is_some() {
            return Err(ReplError::IntentConflict);
        }
        Ok(Self {
            target: self.target,
            active: Some(StudioOverlay::new(basis)),
            prepared: None,
            completed: self.completed.clone(),
            disposed: self.disposed.clone(),
            branch_generation: generation,
            provenance,
            minimum_new_basis_closed_epoch: self.minimum_new_basis_closed_epoch,
            legacy: false,
        })
    }

    pub fn provenance(&self) -> StudioOverlayProvenance {
        self.provenance
    }

    /// Whether this state is fully described by the v2 layout, and so must encode as v2.
    ///
    /// One predicate, consulted by both the encoder and the version choice, so "what v2 can express"
    /// cannot drift from "what we emit tag 2 for". Two copies of that judgement is how a state ends
    /// up with two valid encodings.
    fn is_v2_expressible(&self) -> bool {
        self.branch_generation == 1
            && matches!(self.provenance, StudioOverlayProvenance::Closing)
            && self.disposed.is_none()
    }

    pub fn branch_generation(&self) -> u64 {
        self.branch_generation
    }

    /// The identity every request must carry, or `None` when there is no live branch to name.
    pub fn branch_id(&self) -> Option<[u8; 32]> {
        self.active
            .as_ref()
            .map(|active| branch_identity(active.basis(), self.branch_generation))
    }

    /// Structural classification of an incoming request, before any basis mint, tenure read, source
    /// lookup or media admission.
    ///
    /// Matching order is live branch, then transferred, then disposed, and exactly one arm can
    /// match because the three identities are distinct by construction: a live branch's id is
    /// derived from the current generation, and a retained manifest's is the one recorded when it
    /// became terminal.
    ///
    /// The `intent` is required for the terminal arms and not merely for symmetry: an
    /// acknowledgement is owed only for the exact operation the terminal branch recorded, so a
    /// request naming the right branch with a body that manifest never held is `Unmatched` rather
    /// than acknowledged.
    pub fn classify_request(
        &self,
        target: StudioTarget,
        branch: [u8; 32],
        intent: &LocalIntent,
    ) -> Result<StudioOverlayRequestClass, ReplError> {
        self.check_target(target)?;
        if self.branch_id() == Some(branch) {
            return Ok(StudioOverlayRequestClass::Active);
        }
        let id = intent.operation.id(&intent.author);
        // The transferred manifest stores no branch identity of its own, and it must not start
        // storing one: the `completed` block is part of the v2 layout, so adding a field would
        // rewrite existing records and break the byte-identity every current vault depends on.
        //
        // It is derivable instead. A transfer does not change `branch_generation` - only admitting a
        // new branch does - so while no newer branch has been admitted, the transferred branch's id
        // is exactly `branch_identity(outcome.basis, branch_generation)`. Once a new branch is
        // admitted the generation moves on and that id becomes unrecoverable, so an old transferred
        // request degrades to `Unmatched` and is refused. That is the same degradation design 6.6
        // accepts for forgotten disposals, and in the same safe direction: refusal, never acceptance.
        if let Some(c) = &self.completed {
            if branch_identity(c.outcome.basis, self.branch_generation) == branch {
                return Ok(match c.entries.iter().find(|e| e.id == id) {
                    Some(entry)
                        if c.author == intent.author && entry.envelope == envelope(intent)? =>
                    {
                        StudioOverlayRequestClass::Transferred(c.outcome.clone())
                    }
                    // The right branch, a body it never held. An acknowledgement is owed for the
                    // exact operation the terminal branch recorded and for nothing else.
                    _ => StudioOverlayRequestClass::Unmatched,
                });
            }
        }
        if let Some(d) = &self.disposed {
            if d.branch == branch {
                return Ok(match d.entries.iter().find(|e| e.id == id) {
                    Some(entry)
                        if d.author == intent.author && entry.envelope == envelope(intent)? =>
                    {
                        StudioOverlayRequestClass::Disposed(Box::new(d.clone()))
                    }
                    _ => StudioOverlayRequestClass::Unmatched,
                });
            }
        }
        Ok(StudioOverlayRequestClass::Unmatched)
    }

    /// Resolve `Unmatched` where a fresh basis has just been minted under live authority.
    ///
    /// `New` exactly when there is no active branch **and** `branch` is the identity derived from
    /// this fresh basis at `branch_generation + 1`. Everything else is `Stale`: an older generation,
    /// a skipped one, an unrelated basis, an unknown identity.
    ///
    /// The generation is derived here and never reserved, so two concurrent visits cannot both hold
    /// a claim on the same number - whichever commits first makes the other's derived id stop
    /// matching. Exhaustion refuses rather than wrapping, because a wrapped generation would let an
    /// ancient request name a live namespace again, which is the one thing this defence exists to
    /// prevent.
    pub fn admit_new_branch(
        &self,
        target: StudioTarget,
        branch: [u8; 32],
        fresh: &StudioClosingOverlayBasis,
    ) -> Result<StudioOverlayAdmission, ReplError> {
        self.check_target(target)?;
        if self.active.is_some() {
            return Ok(StudioOverlayAdmission::Stale);
        }
        let Some(generation) = self.branch_generation.checked_add(1) else {
            return Err(ReplError::EpochBound);
        };
        if branch_identity(fresh.fingerprint(), generation) == branch {
            return Ok(StudioOverlayAdmission::New { generation });
        }
        Ok(StudioOverlayAdmission::Stale)
    }
    /// The retained terminal disposal, if this vault has one.
    pub fn disposed(&self) -> Option<&StudioOverlayDisposal> {
        self.disposed.as_ref()
    }

    /// The live branch's content identity, the value a disposal request must carry back.
    ///
    /// Exposed because the request has to be built from something: the inspection a user saw
    /// reports this, and `dispose` refuses anything else. Without an accessor a caller would have
    /// to re-derive the hash, which is precisely the second representation that lets a request
    /// name work the user never saw.
    pub fn branch_content(&self, ledger: &IntentLedger) -> Result<[u8; 32], ReplError> {
        branch_hash(self.active.as_ref().ok_or(ReplError::Malformed)?, ledger)
    }

    /// Drop the active branch without transferring it.
    ///
    /// Returns the rebuilt state and **the exact ids the caller must retire from the ledger**,
    /// read once from the branch rather than recomputed by the caller. Two derivations of "which
    /// ids went away" is how an entry ends up charged to a branch that no longer exists.
    ///
    /// This proves nothing about authorization. The caller has already established membership,
    /// authorship, the absence of a transfer hold, the branch identity, and - for `Preserve` - that
    /// a durable archive for this exact branch exists. All of that needs records this layer cannot
    /// read. What happens here is the state rebuild, its validation, and the two checks this layer
    /// *can* make: the transfer hold below, and `content` against the branch's own hash. A
    /// `StudioDisposalDecision` is required rather than a bare mode so that a destructive disposal
    /// cannot be constructed through the preserving path.
    ///
    /// **`branch`, `generation` and `provenance` are now DERIVED from this state, not supplied.**
    /// An earlier version took all three from the caller and recorded them verbatim, which let a
    /// manifest misdescribe its own branch - a Closing branch could be labelled `Unconfirmed` - with
    /// nothing at this layer able to notice. Now that the state carries the generation and the
    /// provenance there is nothing left for a caller to get wrong, and the manifest cannot disagree
    /// with the branch it describes. `content` remains a parameter precisely because it is the value
    /// the *user* saw: checking the caller's copy against the branch's own hash is the whole point.
    ///
    /// A branch that survives structural validation but cannot be replayed is still disposable.
    /// Refusing here would leave exactly the drafts most in need of disposal undisposable.
    pub fn dispose(
        &self,
        ledger: &IntentLedger,
        decision: StudioDisposalDecision,
        content: [u8; 32],
        sequence: u64,
        at: u64,
    ) -> Result<(Self, BTreeSet<[u8; 32]>), ReplError> {
        let active = self.active.as_ref().ok_or(ReplError::Malformed)?;
        let branch = self.branch_id().ok_or(ReplError::Malformed)?;
        let generation = self.branch_generation;
        let provenance = self.provenance;
        // A Prepared branch refuses outright: a transfer hold is live evidence that someone else
        // may be about to accept this work, and dropping it here would race that acceptance.
        if self.prepared.is_some() {
            return Err(ReplError::IntentConflict);
        }
        if content != branch_hash(active, ledger)? {
            return Err(ReplError::IntentConflict);
        }
        let manifest = StudioOverlayDisposal::from_branch(
            active, ledger, provenance, branch, content, generation, &decision, sequence, at,
        )?;
        let removed = manifest.removed_ids();
        let next = Self {
            target: self.target,
            // Cleared: the whole point of the transition. The manifest is self-contained, so
            // nothing that survives refers to the branch that is gone.
            active: None,
            prepared: None,
            completed: self.completed.clone(),
            disposed: Some(manifest),
            // Carried, not reset. The namespace is monotonic per logical document, so a disposal
            // must not hand the next branch a number this one already used.
            branch_generation: self.branch_generation,
            provenance: self.provenance,
            minimum_new_basis_closed_epoch: self.minimum_new_basis_closed_epoch,
            legacy: false,
        };
        // Validate and encode the rebuilt state, as `append`, `complete` and `set_prepared` all do.
        // Returning it unvalidated would make the store the first thing to discover any
        // inconsistency, at the point where it is already committed to a write. This is also what
        // enforces the cross-manifest rule when `completed` is retained.
        next.encode_vault(ledger)?;
        Ok((next, removed))
    }

    pub fn target(&self) -> StudioTarget {
        self.target
    }
    pub fn overlay(&self) -> Option<&StudioOverlay> {
        self.active.as_ref()
    }
    pub fn is_prepared(&self) -> bool {
        self.prepared.is_some()
    }
    pub fn has_completed(&self) -> bool {
        self.completed.is_some()
    }
    pub fn minimum_new_basis_closed_epoch(&self) -> u64 {
        self.minimum_new_basis_closed_epoch
    }
    pub fn check_target(&self, target: StudioTarget) -> Result<(), ReplError> {
        if target != self.target {
            return Err(ReplError::EpochScope);
        }
        Ok(())
    }
    /// This equality must precede any acknowledgement, source lookup or sync reservation.
    pub fn completed_retry(
        &self,
        target: StudioTarget,
        basis: [u8; 32],
        intent: &LocalIntent,
    ) -> Result<Option<StudioHandoffOutcome>, ReplError> {
        self.check_target(target)?;
        let Some(done) = &self.completed else {
            return Ok(None);
        };
        let id = intent.operation.id(&intent.author);
        let Some(entry) = done.entries.iter().find(|e| e.id == id) else {
            return Ok(None);
        };
        if done.author != intent.author
            || done.outcome.basis != basis
            || entry.envelope != envelope(intent)?
        {
            return Err(ReplError::IntentConflict);
        }
        Ok(Some(done.outcome.clone()))
    }
    pub fn completed_branch(
        &self,
        target: StudioTarget,
        author: DeviceId,
        basis: [u8; 32],
    ) -> Result<Option<StudioHandoffOutcome>, ReplError> {
        self.check_target(target)?;
        Ok(self
            .completed
            .as_ref()
            .filter(|c| c.author == author && c.outcome.basis == basis)
            .map(|c| c.outcome.clone()))
    }
    pub fn append(
        &mut self,
        basis: &StudioClosingOverlayBasis,
        ledger: &IntentLedger,
        id: [u8; 32],
        ts: u64,
    ) -> Result<StudioLocalDraft, ReplError> {
        self.check_target(basis.0.target)?;
        if self.prepared.is_some() {
            return Err(ReplError::EpochClosed);
        }
        check_basis_floor(
            basis.0.receipt.closed_epoch,
            self.minimum_new_basis_closed_epoch,
        )?;
        let mut active = self
            .active
            .clone()
            .unwrap_or_else(|| StudioOverlay::new(basis));
        let view = active.append(basis, ledger, id, ts)?;
        let mut next = self.clone();
        next.active = Some(active);
        next.legacy = false;
        next.encode_vault(ledger)?;
        *self = next;
        Ok(view)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_handoff(
        &self,
        source: &mut StudioEpoch,
        ledger: &IntentLedger,
        device: &MlsDevice,
        group: &ServerGroup,
        tenure: u64,
        _rng: &mut impl CryptoRngCore,
    ) -> Result<StudioHandoffCandidate, ReplError> {
        let authority = self.handoff_authority(device, group, tenure)?;
        // Compatibility batch for the accepted synchronous store transaction. Runtime callers
        // must schedule the detached stages and each signing turn separately under fresh stamps.
        let candidate = source.copy_handoff_source(group)?;
        let mut signing =
            self.clone()
                .prepare_handoff_detached(candidate, ledger.clone(), authority)?;
        while signing.sign_next(device, group, tenure)? {}
        signing.finish()
    }

    fn prepared_manifest(
        mut self,
        candidate: StudioEpoch,
        ledger: &IntentLedger,
        before: [u8; 32],
    ) -> Result<StudioHandoffCandidate, ReplError> {
        self.validate(ledger)?;
        if self.prepared.is_some() {
            return Err(ReplError::EpochClosed);
        }
        let overlay = self.active.as_ref().ok_or(ReplError::EpochScope)?;
        let signed = overlay
            .checked_entries(ledger)?
            .into_iter()
            .map(|(_, i)| {
                candidate
                    .overlay_signed_hash(i)?
                    .ok_or(ReplError::Malformed)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.set_prepared(
            candidate.epoch(),
            candidate.doc_id(),
            before,
            signed,
            ledger,
        )?;
        Ok(StudioHandoffCandidate {
            source: candidate,
            metadata: self,
        })
    }

    fn set_prepared(
        &mut self,
        epoch: u64,
        doc_id: u128,
        before: [u8; 32],
        signed: Vec<[u8; 32]>,
        ledger: &IntentLedger,
    ) -> Result<(), ReplError> {
        let overlay = self.active.as_ref().ok_or(ReplError::EpochScope)?;
        self.prepared = Some(Prepared {
            epoch,
            doc_id,
            receipt: overlay.receipt().hash(),
            before,
            branch: branch_hash(overlay, ledger)?,
            signed,
        });
        self.legacy = false;
        self.encode_vault(ledger)?;
        Ok(())
    }
    /// Read-only evidence classification; never clears the local hold or implies a flush.
    pub fn evidence(
        &self,
        source: &StudioEpoch,
        ledger: &IntentLedger,
    ) -> Result<StudioHandoffEvidence, ReplError> {
        self.validate(ledger)?;
        let p = self.prepared.as_ref().ok_or(ReplError::EpochScope)?;
        if source.target() != self.target
            || source.doc_id() != p.doc_id
            || source.epoch() != p.epoch
        {
            return Ok(StudioHandoffEvidence::Hold);
        }
        let overlay = self.active.as_ref().ok_or(ReplError::Malformed)?;
        let mut count = 0;
        for ((_, intent), expected) in overlay.checked_entries(ledger)?.into_iter().zip(&p.signed) {
            match source.overlay_signed_hash(intent) {
                Ok(Some(actual)) if &actual == expected => count += 1,
                Ok(None) => {}
                _ => return Ok(StudioHandoffEvidence::Hold),
            }
        }
        Ok(if count == p.signed.len() {
            StudioHandoffEvidence::Complete
        } else if count == 0 {
            StudioHandoffEvidence::Absent
        } else {
            StudioHandoffEvidence::Hold
        })
    }
    pub fn matches_source_before(&self, source: &mut StudioEpoch) -> Result<bool, ReplError> {
        let hash = source_hash(source)?;
        Ok(self.prepared.as_ref().is_some_and(|p| p.before == hash))
    }
    /// Caller must persist this through its accounted writer. The full branch remains intact.
    pub fn return_to_active(
        &self,
        source: &StudioEpoch,
        ledger: &IntentLedger,
    ) -> Result<Self, ReplError> {
        if self.evidence(source, ledger)? != StudioHandoffEvidence::Absent {
            return Err(ReplError::IntentConflict);
        }
        let mut next = self.clone();
        next.prepared = None;
        Ok(next)
    }
    /// Only a checked complete signed log can release the base. Store must flush it FIRST.
    pub fn complete(&self, source: &StudioEpoch, ledger: &IntentLedger) -> Result<Self, ReplError> {
        if self.evidence(source, ledger)? != StudioHandoffEvidence::Complete {
            return Err(ReplError::IntentConflict);
        }
        let p = self.prepared.as_ref().ok_or(ReplError::Malformed)?;
        let active = self.active.as_ref().ok_or(ReplError::Malformed)?;
        let mut next = self.clone();
        next.completed = Some(Completed {
            target: self.target,
            author: active.author(),
            outcome: StudioHandoffOutcome {
                basis: active.basis(),
                epoch: p.epoch,
                doc_id: p.doc_id,
                accepted: active.entries.len(),
            },
            entries: active.entries.clone(),
        });
        next.minimum_new_basis_closed_epoch = next.minimum_new_basis_closed_epoch.max(p.epoch);
        next.active = None;
        next.prepared = None;
        next.legacy = false;
        next.encode_vault(ledger)?;
        Ok(next)
    }
    fn validate(&self, ledger: &IntentLedger) -> Result<(), ReplError> {
        if self.target.document(&ledger.document().server_id)? != *ledger.document() {
            return Err(ReplError::EpochScope);
        }
        // The generation namespace. `>= 1` because a zero generation would make the first branch's
        // identity collide with the "no branch yet" case, and every branch is at least the first.
        if self.branch_generation < 1 {
            return Err(ReplError::Malformed);
        }
        if let Some(disposal) = &self.disposed {
            // A manifest from the future would mean the namespace went backwards, which is the one
            // thing monotonicity buys.
            if disposal.generation > self.branch_generation {
                return Err(ReplError::Malformed);
            }
            // A live branch beside a retained disposal must be a strictly later generation. This is
            // implied by `branch_id` construction, since admitting a branch increments, but it is
            // asserted rather than assumed: if it ever failed, the two would share an identity and
            // `classify_request` could answer `Active` and `Disposed` for the same request.
            if self.active.is_some() && self.branch_generation <= disposal.generation {
                return Err(ReplError::Malformed);
            }
        }
        // `Unconfirmed` has no installed source, so it can neither be handed off nor carry a source
        // identity. The nested basis blob is untouched for it, which means those fields must be
        // canonically zero rather than merely ignored.
        if let StudioOverlayProvenance::Unconfirmed { .. } = self.provenance {
            if self.prepared.is_some() {
                return Err(ReplError::EpochAuthority);
            }
            if let Some(active) = &self.active {
                if !active.has_zero_source_identity() {
                    return Err(ReplError::Malformed);
                }
            }
        }
        if let Some(active) = &self.active {
            if active.target() != self.target {
                return Err(ReplError::EpochScope);
            }
            check_basis_floor(
                active.receipt().closed_epoch,
                self.minimum_new_basis_closed_epoch,
            )?;
            active.checked_entries(ledger)?;
        }
        if let Some(p) = &self.prepared {
            let active = self.active.as_ref().ok_or(ReplError::Malformed)?;
            if p.epoch
                != active
                    .receipt()
                    .closed_epoch
                    .checked_add(1)
                    .ok_or(ReplError::EpochBound)?
                || p.receipt != active.receipt().hash()
                || p.doc_id
                    != crate::epoch_id(
                        ledger.document().doc_type,
                        &ledger.document().logical_key,
                        p.epoch,
                        &active.receipt().close_record_hash,
                    )
                || p.signed.len() != active.entries.len()
                || p.branch != branch_hash(active, ledger)?
            {
                return Err(ReplError::IntentConflict);
            }
        }
        if let Some(c) = &self.completed {
            if c.target != self.target
                || c.outcome.epoch == 0
                || c.outcome.epoch > self.minimum_new_basis_closed_epoch
                || c.outcome.accepted != c.entries.len()
            {
                return Err(ReplError::EpochScope);
            }
            validate_manifest(&c.entries)?;
            let pending: BTreeMap<_, _> = ledger.pending().collect();
            for entry in &c.entries {
                if self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.contains(&entry.id))
                {
                    return Err(ReplError::IntentConflict);
                }
                if let Some(intent) = pending.get(&entry.id) {
                    if intent.author != c.author || envelope(intent)? != entry.envelope {
                        return Err(ReplError::IntentConflict);
                    }
                }
            }
        }
        if let Some(disposal) = &self.disposed {
            if disposal.target != self.target {
                return Err(ReplError::EpochScope);
            }
            disposal.validate()?;
            // Two rules shared with the transferred manifest, and a third that only exists because
            // two terminal manifests can now be retained at once.
            //
            // The shared pair: a retained manifest must not claim an id the *current* live branch
            // holds, or a request naming that id would classify against the wrong event; and where
            // an id is somehow still in the ledger, its author and envelope must be the ones this
            // manifest recorded, so a retained acknowledgement cannot be made to describe a
            // different body.
            //
            // The third rule is **no id may appear in both terminal manifests**. `classify_request`
            // answers "which terminal event is this request about" and exactly one arm may match;
            // an id in both would make `Transferred` and `Disposed` simultaneously true for one
            // request while saying opposite things about where the work went. This slice is where
            // the rule becomes necessary, because `dispose` retains `completed` while adding
            // `disposed`.
            let pending: BTreeMap<_, _> = ledger.pending().collect();
            for entry in &disposal.entries {
                if self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.contains(&entry.id))
                {
                    return Err(ReplError::IntentConflict);
                }
                if self
                    .completed
                    .as_ref()
                    .is_some_and(|c| c.entries.iter().any(|e| e.id == entry.id))
                {
                    return Err(ReplError::IntentConflict);
                }
                if let Some(intent) = pending.get(&entry.id) {
                    if intent.author != disposal.author || envelope(intent)? != entry.envelope {
                        return Err(ReplError::IntentConflict);
                    }
                }
            }
        }
        Ok(())
    }
    pub fn encode_vault(&self, ledger: &IntentLedger) -> Result<Vec<u8>, ReplError> {
        self.validate(ledger)?;
        // `is_v2_expressible()` joins the legacy conditions, which covers the disposal, the
        // generation and the provenance in one predicate. The v1 arm writes the single-branch format,
        // which has nowhere to put any of the three: without this a state carrying one would have it
        // silently dropped on encode, and silently losing a terminal record or a generation is the
        // one failure this family must never have.
        if self.legacy
            && self.prepared.is_none()
            && self.completed.is_none()
            && self.is_v2_expressible()
            && self.minimum_new_basis_closed_epoch == 0
        {
            return self
                .active
                .as_ref()
                .ok_or(ReplError::Malformed)?
                .encode_vault(ledger);
        }
        let mut e = Encoder::new();
        // Tag 3 only when the state cannot be expressed as v2: a non-Closing provenance, a
        // generation other than 1, or a retained disposal. Everything else still encodes as v2
        // byte-for-byte, so no existing record is rewritten and the canonical re-encode equality
        // every current vault relies on is untouched. A v1 or v2 record decodes with generation 1
        // and Closing provenance, which is exactly what those records always meant.
        //
        // Old readers fail closed on tag 3, which is the intended direction: a reader with no notion
        // of a generation namespace must not silently read a later generation as the first.
        e.put_u8(if self.is_v2_expressible() { 2 } else { 3 });
        put_target(&mut e, self.target)?;
        e.put_u64(self.minimum_new_basis_closed_epoch);
        match &self.active {
            None => {
                e.put_u8(0);
            }
            Some(active) => {
                e.put_u8(if self.prepared.is_some() { 2 } else { 1 });
                put(&mut e, &active.encode_vault(ledger)?)?;
                if let Some(p) = &self.prepared {
                    e.put_u64(p.epoch);
                    put(&mut e, &p.doc_id.to_be_bytes())?;
                    for hash in [p.receipt, p.before, p.branch] {
                        put(&mut e, &hash)?;
                    }
                    e.put_u32(p.signed.len() as u32);
                    for hash in &p.signed {
                        put(&mut e, hash)?;
                    }
                }
            }
        }
        match &self.completed {
            None => {
                e.put_u8(0);
            }
            Some(c) => {
                e.put_u8(1);
                put_target(&mut e, c.target)?;
                put(&mut e, c.author.as_bytes())?;
                put(&mut e, &c.outcome.basis)?;
                e.put_u64(c.outcome.epoch);
                put(&mut e, &c.outcome.doc_id.to_be_bytes())?;
                e.put_u32(c.entries.len() as u32);
                for entry in &c.entries {
                    put_entry(&mut e, entry)?;
                }
            }
        }
        if !self.is_v2_expressible() {
            e.put_u64(self.branch_generation);
            put_provenance(&mut e, &self.provenance)?;
            match &self.disposed {
                None => {
                    e.put_u8(0);
                }
                Some(d) => {
                    e.put_u8(1);
                    d.put(&mut e)?;
                }
            }
        }
        let bytes = e.finish();
        let seed_bytes = self.active.as_ref().map_or(0, |a| a.base.seed.len());
        if bytes.len().saturating_sub(seed_bytes) > MAX_METADATA || bytes.len() > MAX_EXTENSION {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }
    /// Full validation, including complete ordered reconstruction of any retained branch.
    pub fn decode_vault(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError> {
        Self::decode_vault_inner(bytes, ledger, true)
    }

    /// Identity, bounds, scope, entry and canonical-encoding validation without replaying the
    /// branch. See `StudioOverlay::decode_vault_structural`: this mints no authority, and a
    /// decoded Prepared or Completed flag remains evidence of a local record, never a capability.
    pub fn decode_vault_structural(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError> {
        Self::decode_vault_inner(bytes, ledger, false)
    }

    fn decode_vault_inner(
        bytes: &[u8],
        ledger: &IntentLedger,
        replay: bool,
    ) -> Result<Self, ReplError> {
        if bytes.len() > MAX_EXTENSION {
            return Err(ReplError::EpochBound);
        }
        if bytes.first() == Some(&1) {
            let active = StudioOverlay::decode_vault_inner(bytes, ledger, replay)?;
            return Ok(Self {
                target: active.target(),
                active: Some(active),
                prepared: None,
                completed: None,
                disposed: None,
                // A v1 record predates the namespace, so it is the first generation on a Closing
                // basis. That is what such a record has always meant; reading it any other way
                // would invent history it does not contain.
                branch_generation: 1,
                provenance: StudioOverlayProvenance::Closing,
                minimum_new_basis_closed_epoch: 0,
                legacy: true,
            });
        }
        let mut d = Decoder::new(bytes);
        let version = byte(&mut d)?;
        if version != 2 && version != 3 {
            return Err(ReplError::Malformed);
        }
        let target = get_target(&mut d)?;
        let minimum_new_basis_closed_epoch = number(&mut d)?;
        let tag = byte(&mut d)?;
        let (active, prepared) = match tag {
            0 => (None, None),
            1 | 2 => {
                let raw = d.get_bytes().map_err(|_| ReplError::Malformed)?;
                // Count the rest of the v2 record before allocating/reconstructing the seed.
                // The nested v1 decoder independently bounds its own seed and metadata.
                if bytes.len().saturating_sub(raw.len()) > MAX_METADATA {
                    return Err(ReplError::EpochBound);
                }
                let seed_bytes = nested_seed_len(raw)?;
                if bytes.len().saturating_sub(seed_bytes) > MAX_METADATA {
                    return Err(ReplError::EpochBound);
                }
                let a = StudioOverlay::decode_vault_inner(raw, ledger, replay)?;
                let p = if tag == 2 {
                    let epoch = number(&mut d)?;
                    let doc_id = u128::from_be_bytes(fixed(&mut d)?);
                    let receipt = fixed(&mut d)?;
                    let before = fixed(&mut d)?;
                    let branch = fixed(&mut d)?;
                    let n = count(&mut d)?;
                    let signed = (0..n)
                        .map(|_| fixed(&mut d))
                        .collect::<Result<Vec<_>, _>>()?;
                    Some(Prepared {
                        epoch,
                        doc_id,
                        receipt,
                        before,
                        branch,
                        signed,
                    })
                } else {
                    None
                };
                (Some(a), p)
            }
            _ => return Err(ReplError::Malformed),
        };
        let completed = match byte(&mut d)? {
            0 => None,
            1 => {
                let target = get_target(&mut d)?;
                let author = DeviceId::from_bytes(fixed(&mut d)?);
                let basis = fixed(&mut d)?;
                let epoch = number(&mut d)?;
                let doc_id = u128::from_be_bytes(fixed(&mut d)?);
                let n = count(&mut d)?;
                let entries = (0..n)
                    .map(|_| get_entry(&mut d))
                    .collect::<Result<Vec<_>, _>>()?;
                Some(Completed {
                    target,
                    author,
                    outcome: StudioHandoffOutcome {
                        basis,
                        epoch,
                        doc_id,
                        accepted: n,
                    },
                    entries,
                })
            }
            _ => return Err(ReplError::Malformed),
        };
        // v3 carries the generation, the provenance and an optional disposal block. v2 carries none
        // of them, and means generation 1, Closing provenance and no disposal - exactly what such a
        // record has always meant.
        //
        // A v3 record that is nevertheless v2-expressible is refused, not silently accepted: the
        // canonical re-encode check below would produce tag 2 for it and the comparison fails. That
        // is what keeps one state from having two valid encodings, which would otherwise break
        // Agent 1's digest fence.
        let (branch_generation, provenance, disposed) = if version == 3 {
            let generation = number(&mut d)?;
            let provenance = get_provenance(&mut d)?;
            let disposed = match byte(&mut d)? {
                0 => None,
                1 => Some(StudioOverlayDisposal::get(&mut d)?),
                _ => return Err(ReplError::Malformed),
            };
            (generation, provenance, disposed)
        } else {
            (1, StudioOverlayProvenance::Closing, None)
        };
        d.finish().map_err(|_| ReplError::Malformed)?;
        let out = Self {
            target,
            active,
            prepared,
            completed,
            disposed,
            branch_generation,
            provenance,
            minimum_new_basis_closed_epoch,
            legacy: false,
        };
        if out.encode_vault(ledger)?.as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(out)
    }
}

fn source_hash(source: &mut StudioEpoch) -> Result<[u8; 32], ReplError> {
    Ok(blake3::derive_key(
        "catcoms/studio-overlay-source-before/v1",
        &source.snapshot()?,
    ))
}
fn check_basis_floor(closed: u64, floor: u64) -> Result<(), ReplError> {
    if closed < floor {
        return Err(ReplError::EpochScope);
    }
    Ok(())
}
// Borrow the nested v1 fields to charge combined metadata before any seed allocation/replay.
fn nested_seed_len(raw: &[u8]) -> Result<usize, ReplError> {
    let mut d = Decoder::new(raw);
    if byte(&mut d)? != 1 {
        return Err(ReplError::Malformed);
    }
    byte(&mut d)?;
    for _ in 0..5 {
        d.get_bytes().map_err(|_| ReplError::Malformed)?;
    }
    let seed = d.get_bytes().map_err(|_| ReplError::Malformed)?;
    if seed.len() > MAX_CHECKPOINT_BYTES {
        return Err(ReplError::EpochBound);
    }
    Ok(seed.len())
}
fn branch_hash(active: &StudioOverlay, ledger: &IntentLedger) -> Result<[u8; 32], ReplError> {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-overlay-branch-ledger/v1");
    for bytes in [active.encode_vault(ledger)?, ledger.encode()?] {
        hash.update(&(bytes.len() as u64).to_be_bytes());
        hash.update(&bytes);
    }
    Ok(*hash.finalize().as_bytes())
}
pub(super) fn validate_manifest(entries: &[Entry]) -> Result<(), ReplError> {
    if entries.is_empty() || entries.len() > MAX_STUDIO_OVERLAY_OPS {
        return Err(ReplError::EpochBound);
    }
    let mut seen = BTreeSet::new();
    for (i, e) in entries.iter().enumerate() {
        if e.sequence != i as u64 + 1 || !seen.insert(e.id) {
            return Err(ReplError::IntentConflict);
        }
        integer_bound(e.ts)?;
    }
    Ok(())
}
pub(super) fn put(e: &mut Encoder, bytes: &[u8]) -> Result<(), ReplError> {
    e.put_bytes(bytes)
        .map(|_| ())
        .map_err(|_| ReplError::EpochBound)
}
pub(super) fn byte(d: &mut Decoder<'_>) -> Result<u8, ReplError> {
    d.get_u8().map_err(|_| ReplError::Malformed)
}
pub(super) fn number(d: &mut Decoder<'_>) -> Result<u64, ReplError> {
    d.get_u64().map_err(|_| ReplError::Malformed)
}
pub(super) fn count(d: &mut Decoder<'_>) -> Result<usize, ReplError> {
    let n = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
    if n == 0 || n > MAX_STUDIO_OVERLAY_OPS {
        return Err(ReplError::EpochBound);
    }
    Ok(n)
}
pub(super) fn put_target(e: &mut Encoder, target: StudioTarget) -> Result<(), ReplError> {
    e.put_u8(match target {
        StudioTarget::Index { .. } => 0,
        StudioTarget::Flipnote { .. } => 1,
    });
    put(e, &target.channel())?;
    if let StudioTarget::Flipnote { object, .. } = target {
        put(e, &object)?;
    }
    Ok(())
}
pub(super) fn get_target(d: &mut Decoder<'_>) -> Result<StudioTarget, ReplError> {
    let tag = byte(d)?;
    let channel = fixed(d)?;
    match tag {
        0 => Ok(StudioTarget::Index { channel }),
        1 => Ok(StudioTarget::Flipnote {
            channel,
            object: fixed(d)?,
        }),
        _ => Err(ReplError::Malformed),
    }
}
pub(super) fn put_entry(e: &mut Encoder, entry: &Entry) -> Result<(), ReplError> {
    put(e, &entry.id)?;
    put(e, &entry.envelope)?;
    e.put_u64(entry.sequence);
    e.put_u64(entry.ts);
    Ok(())
}
pub(super) fn get_entry(d: &mut Decoder<'_>) -> Result<Entry, ReplError> {
    Ok(Entry {
        id: fixed(d)?,
        envelope: fixed(d)?,
        sequence: number(d)?,
        ts: number(d)?,
    })
}
