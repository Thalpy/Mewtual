//! Flow R: the scheduled resolution of an interrupted Prepared handoff (design 6.4.2).
//!
//! A Prepared record is what an interrupted H5 leaves behind. The synchronous resolver,
//! `resolve_studio_handoff_with_io(None)`, settles it in one custody visit, and pays a full source
//! restore there, twice when the source is cold: once inside the inventory scan's inline
//! validation, once in `checked_studio_source`. Flow R splits it the way H1 and H2 split the
//! forward path:
//!
//! ```text
//! R1 capture   custody    membership, two bounded reads, the Hold early exit, the stamp
//! R2 resolve   detached   structural decode, restore, evidence, next state, inventory warmth
//! R3 commit    custody    warm the inventory, stamp equality, then the resolver's own writes
//! ```
//!
//! **Equivalence is the contract.** R3 writes exactly what the synchronous resolver would write at
//! R3's moment, or refuses with nothing durable changed. Everything R2 carries is a pure function
//! of the stamped bytes and the stamped public context, so R3's stamp equality is what lets it use
//! them; and R3 re-derives everything else itself: its own decode of the intent record, the
//! source's accounting check, the flush, the reference check, and a shape check on the carried
//! next state.
//!
//! **No live capability crosses the boundary.** The worker gets authenticated plaintext and public
//! context (group id, target, actor, owner), never a `ServerGroup`, `MlsDevice`, store or writer,
//! exactly as H2 does.
//!
//! The fences (rotation, adoption, repair) keep the synchronous resolver as the backstop, and so
//! does `start_studio_handoff_with_io` for its synchronous callers. Only the scheduled probe
//! routes a Prepared branch here.
use super::super::epoch_intents::{self, EpochIntentState};
use super::super::epoch_recovery::inventory::{studio_inventory_warmth, StudioInventoryWarmth};
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_replication::studio::{StudioHandoffEvidence, StudioOverlayState};

/// What R1 captured: record identity and the public context the restore depends on.
///
/// **Deliberately no tenure and no MLS epoch.** Resolution signs nothing and mints no authority,
/// and none of the restore, `evidence`, `return_to_active`, `complete`, the flush-only save, the
/// reference check or the intents write reads either (design 6.4.3, question 1). The owner is
/// kept because the restore takes it.
struct StudioResolveStamp {
    mount: Arc<()>,
    server: u64,
    document: LogicalDocument,
    target: StudioTarget,
    actor: DeviceId,
    actor_key: Vec<u8>,
    owner: DeviceId,
    /// The intent record's (plaintext blake3, physical size).
    intent: (blake3::Hash, u64),
    /// The source record's, read with the family bound.
    source: (blake3::Hash, u64),
}

/// R1's result: authenticated plaintext plus the stamp. Nothing durable was touched.
pub(crate) struct StudioResolveCapture {
    stamp: StudioResolveStamp,
    group: Vec<u8>,
    intent_bytes: Zeroizing<Vec<u8>>,
    source_bytes: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for StudioResolveCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioResolveCapture { .. }")
    }
}

/// What R1 concluded.
pub(crate) enum StudioResolveStart {
    /// Captured for R2.
    Captured(Box<StudioResolveCapture>),
    /// The record is not (or no longer) Prepared: nothing for Flow R, and H1's ordinary path
    /// applies. Not a failure, so the caller does not back the target off.
    NotPrepared,
    /// The framing-only evidence is Hold, which means the resolver refuses unconditionally. No
    /// worker and no restore: the caller backs the target off, as H1's refusal does today.
    Hold,
}

/// The decision R2 reached. `Hold` is not here: R2 refuses it as an error.
enum ResolveOutcome {
    /// The source holds none of the branch: return it to Active.
    Absent { next: StudioOverlayState },
    /// The source holds the whole branch: flush it unchanged, then complete.
    Complete {
        next: StudioOverlayState,
        unit: StudioEpoch,
        before: Zeroizing<Vec<u8>>,
    },
}

/// R2's result, carried to R3. Valid only behind the stamp.
pub(crate) struct StudioResolvePlan {
    stamp: StudioResolveStamp,
    /// The restored source's protocol bytes, the one accounting fact the stamp does not carry.
    /// R3 checks it with `verify_record` for both outcomes (design 6.4.3, M-2).
    protocol_bytes: usize,
    outcome: ResolveOutcome,
    /// The source's inventory validation, taken by [`ServerStore::warm_studio_resolution`].
    warmth: Option<StudioInventoryWarmth>,
}

impl std::fmt::Debug for StudioResolvePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioResolvePlan { .. }")
    }
}

/// What R3 did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StudioResolved {
    /// The branch is Active again (Absent evidence).
    Returned,
    /// The transfer completed durably (Complete evidence).
    Completed,
    /// The intent record had changed and is no longer Prepared: someone else resolved it, so
    /// there was nothing to write, and nothing to back off.
    Superseded,
}

impl StudioResolveCapture {
    /// R2, on a blocking worker and off custody. Signs nothing and writes nothing.
    pub(crate) fn resolve(self) -> Result<StudioResolvePlan, AppError> {
        let stamp = self.stamp;
        // The resolver's own decode: structural, not the full `EpochIntentState::decode`. The full
        // decode replays the branch, which costs a reconstruction per attempt and refuses a record
        // that decodes structurally but not fully, which the resolver would resolve (6.4.3, M-1).
        let scope = epoch_intents::scope_bytes(stamp.server, &stamp.document)?;
        let state =
            EpochIntentState::decode_structural(&self.intent_bytes, &scope, &stamp.document)?;
        let metadata = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay metadata missing"))?;
        metadata.check_target(stamp.target).map_err(invalid)?;
        if !metadata.is_prepared() {
            return Err(invalid("handoff is no longer Prepared"));
        }
        let source_scope = scope_bytes(stamp.server, &stamp.document)?;
        let (target, snapshot) = decode_record(&self.source_bytes, &source_scope, &stamp.document)?;
        if target != stamp.target {
            return Err(invalid("prepared source channel changed"));
        }
        // The scan's own validation of these bytes, keyed as the scan keys it, so R3's budget
        // finds the source warm and restores nothing inline (6.4.3, HIGH-2).
        let warmth = studio_inventory_warmth(&self.source_bytes, stamp.source.1)?;
        // The resolver's restore with public context only: `restore` and `prepare_vault_source`
        // are the same `restore_scoped`, with the same owner normalisation.
        let mut unit = StudioEpoch::prepare_vault_source(
            snapshot,
            &self.group,
            target,
            stamp.actor,
            stamp.owner,
        )
        .map_err(invalid)?;
        // `checked_studio_source`'s order: the protocol bytes, then the normalized snapshot.
        let protocol_bytes = unit.storage_protocol_bytes().map_err(invalid)?;
        let before = Zeroizing::new(unit.snapshot().map_err(invalid)?);
        let outcome = match metadata.evidence(&unit, &state.ledger).map_err(invalid)? {
            StudioHandoffEvidence::Absent => ResolveOutcome::Absent {
                next: metadata
                    .return_to_active(&unit, &state.ledger)
                    .map_err(invalid)?,
            },
            StudioHandoffEvidence::Complete => ResolveOutcome::Complete {
                next: metadata.complete(&unit, &state.ledger).map_err(invalid)?,
                unit,
                before,
            },
            // The resolver's own refusal and message. R1's framing-only exit makes this rare,
            // but a difference only a restore can see still lands here.
            StudioHandoffEvidence::Hold => {
                return Err(invalid(
                    "prepared handoff retains conflicting or incomplete signed evidence",
                ))
            }
        };
        Ok(StudioResolvePlan {
            stamp,
            protocol_bytes,
            outcome,
            warmth: Some(warmth),
        })
    }
}

/// What R3's re-read found, against the stamp.
enum StampedRecords {
    Unchanged,
    /// The intent record differs: another write resolved or changed it.
    IntentChanged,
    /// Only the source differs: a history-preserving write landed on the Prepared destination.
    SourceChanged,
}

impl ServerStore {
    /// R1. Runs under custody and takes **no inventory**: it writes nothing, and R3 repeats every
    /// check that matters. A synchronous inventory here would validate a cold source inline, which
    /// is a full restore (design 6.4.3, HIGH-2).
    pub(crate) fn capture_studio_resolution(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
    ) -> Result<StudioResolveStart, AppError> {
        current_member(group, device)?;
        let owner = group
            .designated_committer()
            .ok_or_else(|| invalid("no current owner"))?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let intent_scope = epoch_intents::scope_bytes(server, &document)?;
        let Some(intent) = self.read_scoped_intent_plain(&intent_scope)? else {
            return Ok(StudioResolveStart::NotPrepared);
        };
        let state = EpochIntentState::decode_structural(&intent.plain, &intent_scope, &document)?;
        let Some(metadata) = state.handoff_metadata() else {
            return Ok(StudioResolveStart::NotPrepared);
        };
        metadata.check_target(target).map_err(invalid)?;
        if !metadata.is_prepared() {
            return Ok(StudioResolveStart::NotPrepared);
        }
        // The family bound, as `checked_studio_source` reads it, never H1's 8 MiB retained bound:
        // after an interrupted H5 this may be the successor, which can be larger.
        let source_scope = scope_bytes(server, &document)?;
        let source = self
            .read_studio_record(&source_scope)?
            .ok_or_else(|| invalid("prepared handoff source missing"))?;
        // The scan's read discipline: a cached validation these bytes contradict can never hit
        // again, and left in place it would refuse the warm install R3 makes, so R3's scan would
        // restore this source inline after all (the re-review of 6.4.2, M-1). That is exactly the
        // case after an H5 that refused once its Source write had landed, with no restart.
        self.evict_stale_studio_inventory(
            &source_scope,
            source.physical_bytes,
            blake3::hash(&source.plain),
        );
        self.check_studio_intent_link(server, &document, &source_scope, &source.plain)?;
        // The Hold early exit (6.4.3, question 4). Framing only, as the eligibility view already
        // runs it under custody, and a Hold here means the resolver refuses whatever a restore
        // would show. Absent and Complete are never acted on from this reading.
        let (stored, snapshot) = decode_record(&source.plain, &source_scope, &document)?;
        if stored != target {
            return Err(invalid("wrong object channel"));
        }
        if metadata
            .evidence_in_vault(snapshot, &state.ledger)
            .map_err(invalid)?
            == StudioHandoffEvidence::Hold
        {
            return Ok(StudioResolveStart::Hold);
        }
        Ok(StudioResolveStart::Captured(Box::new(
            StudioResolveCapture {
                stamp: StudioResolveStamp {
                    mount: self.registry_mount(),
                    server,
                    document,
                    target,
                    actor: device.device_id(),
                    actor_key: device.public_key_bytes(),
                    owner,
                    intent: (blake3::hash(&intent.plain), intent.physical_bytes),
                    source: (blake3::hash(&source.plain), source.physical_bytes),
                },
                group: group.group_id(),
                intent_bytes: intent.plain,
                source_bytes: source.plain,
            },
        )))
    }

    /// R3's first half, before the budget is built: memoize R2's validation of the source.
    ///
    /// Sound with no other check, because the cache is content-addressed: a hit needs the exact
    /// key, physical size and plaintext digest, so the entry only ever serves the bytes it was
    /// computed from, and `IfVacant` never displaces an entry another read has put. The mount
    /// check is hygiene. Returns whether the cache was written.
    pub(crate) fn warm_studio_resolution(&mut self, plan: &mut StudioResolvePlan) -> bool {
        if !Arc::ptr_eq(&plan.stamp.mount, &self.registry_mount()) {
            return false;
        }
        plan.warmth
            .take()
            .is_some_and(|warmth| self.warm_studio_inventory(warmth))
    }

    /// R3: commit a resolution under stamp equality.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_studio_resolution(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        plan: StudioResolvePlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioResolved, AppError> {
        self.commit_studio_resolution_with_io(
            server,
            group,
            target,
            device,
            plan,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_studio_resolution_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        plan: StudioResolvePlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioResolved, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let StudioResolvePlan {
            stamp,
            protocol_bytes,
            outcome,
            warmth: _,
        } = plan;
        if stamp.server != server
            || stamp.target != target
            || stamp.document.server_id != group.group_id()
        {
            return Err(invalid("resolution plan is for another target"));
        }
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount())
            || stamp.actor != device.device_id()
            || stamp.actor_key != device.public_key_bytes()
        {
            return Err(invalid(
                "resolution plan was captured for another mount or device",
            ));
        }
        let records = self.stamped_resolution_records(&stamp)?;
        if let StampedRecords::IntentChanged = records {
            // Someone else wrote the intent record. If it is no longer Prepared it was resolved
            // (a fence, typically), and that resolution stands: nothing to do, nothing to pace.
            let state = self.load_epoch_intents_structural(server, &stamp.document)?;
            return if state.handoff_prepared() {
                Err(invalid("prepared intent record changed since the capture"))
            } else {
                Ok(StudioResolved::Superseded)
            };
        }
        // An owner change alters a restore input; the live resolver uses the live owner. Like a
        // changed source it falls back rather than refusing, so the record is not left Prepared.
        let owner_moved = group.designated_committer() != Some(stamp.owner);
        if matches!(records, StampedRecords::SourceChanged) || owner_moved {
            // HIGH-1 (design 6.4.3). A Prepared destination legitimately takes history-preserving
            // writes, such as a peer's received operations, so under steady inbound a stamp might
            // never match, and a Prepared record blocks the user's edits, disposal and page
            // service. Resolve now, synchronously: exactly today's cost, and only when contended.
            self.resolve_studio_handoff_with_io(
                server, group, target, device, None, rng, budget, hooks,
            )?;
            return self.resolved_outcome(server, &stamp.document);
        }
        // R3's own decode, which also checks the intent record against this visit's inventory.
        // The digest equality above makes it byte-identical to what R2 decoded, and it is this
        // state, never a carried one, that is written.
        let document = stamp.document.clone();
        let mut state = self.checked_epoch_replay_state(
            server,
            &document,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        let metadata = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay metadata missing"))?;
        metadata.check_target(target).map_err(invalid)?;
        if !metadata.is_prepared() {
            return Err(invalid("handoff is no longer Prepared"));
        }
        // The source record against the budget, for both outcomes, as the resolver does before it
        // classifies (6.4.3, M-2). R2's protocol figure is only a claim until this passes.
        let source_scope = scope_bytes(server, &document)?;
        let observed = storage_record(
            server,
            &document,
            &source_scope,
            stamp.source.1,
            protocol_bytes,
        )?;
        budget
            .storage
            .verify_record(
                &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                *blake3::hash(&source_scope).as_bytes(),
                Some(observed),
            )
            .map_err(invalid)?;
        match outcome {
            ResolveOutcome::Absent { next } => {
                if !returned_shape(metadata, &next) {
                    return Err(invalid(
                        "resolution plan does not return this branch to active",
                    ));
                }
                state.overlay = Some(next);
                self.persist_handoff_intents(
                    server,
                    &document,
                    state,
                    false,
                    WriteTag::Active,
                    rng,
                    budget,
                    hooks,
                )?;
                Ok(StudioResolved::Returned)
            }
            ResolveOutcome::Complete { next, unit, before } => {
                if !completed_shape(metadata, &next, target, &unit)? {
                    return Err(invalid("resolution plan does not complete this branch"));
                }
                // The resolver's own sequence. The flush-only step re-snapshots the unit and
                // checks it against `before` and against disk, and refuses any difference.
                let source = self.save_studio_source(
                    server,
                    unit,
                    Some(observed),
                    &before,
                    WritePurpose::Ordinary,
                    rng,
                    &mut budget.storage,
                    WriteStep::flush_only(
                        WriteTag::Source,
                        "handoff resolution requires unchanged source",
                    ),
                    hooks,
                )?;
                self.check_handoff_references(metadata, &source.unit, &state)?;
                state.overlay = Some(next);
                self.persist_handoff_intents(
                    server,
                    &document,
                    state,
                    false,
                    WriteTag::Completed,
                    rng,
                    budget,
                    hooks,
                )?;
                Ok(StudioResolved::Completed)
            }
        }
    }

    /// Re-read both stamped records, each with its own reader and bound.
    fn stamped_resolution_records(
        &self,
        stamp: &StudioResolveStamp,
    ) -> Result<StampedRecords, AppError> {
        let intent_scope = epoch_intents::scope_bytes(stamp.server, &stamp.document)?;
        let intent = self.read_scoped_intent_plain(&intent_scope)?;
        if intent.map(|i| (blake3::hash(&i.plain), i.physical_bytes)) != Some(stamp.intent) {
            return Ok(StampedRecords::IntentChanged);
        }
        let source_scope = scope_bytes(stamp.server, &stamp.document)?;
        let source = self.read_studio_record(&source_scope)?;
        if source.map(|s| (blake3::hash(&s.plain), s.physical_bytes)) != Some(stamp.source) {
            return Ok(StampedRecords::SourceChanged);
        }
        Ok(StampedRecords::Unchanged)
    }

    /// What a synchronous resolution left: Completed if the branch is gone, Returned if it is
    /// Active. The resolver either wrote one of the two or returned an error.
    fn resolved_outcome(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<StudioResolved, AppError> {
        let state = self.load_epoch_intents_structural(server, document)?;
        let metadata = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay metadata missing"))?;
        if metadata.is_prepared() {
            return Err(invalid("prepared handoff remained Prepared"));
        }
        Ok(if metadata.overlay().is_some() {
            StudioResolved::Returned
        } else {
            StudioResolved::Completed
        })
    }
}

/// Absent's shape (design 6.4.3, M-3): the same live branch, by identity, author, basis, entry
/// count, provenance and generation, no longer Prepared, with the completed record, the disposal
/// and the basis floor unchanged. A Completed-shaped state has no live branch and cannot pass, so
/// it can never reach the arm that skips the flush and the reference check. Only public
/// accessors: nothing here needs new replication-core API.
fn returned_shape(metadata: &StudioOverlayState, next: &StudioOverlayState) -> bool {
    let (Some(branch), Some(returned)) = (metadata.overlay(), next.overlay()) else {
        return false;
    };
    !next.is_prepared()
        && next.target() == metadata.target()
        && next.branch_id() == metadata.branch_id()
        && next.branch_generation() == metadata.branch_generation()
        && next.provenance() == metadata.provenance()
        && next.has_completed() == metadata.has_completed()
        && next.disposed().is_some() == metadata.disposed().is_some()
        && next.minimum_new_basis_closed_epoch() == metadata.minimum_new_basis_closed_epoch()
        && returned.author() == branch.author()
        && returned.basis() == branch.basis()
        && returned.accepted() == branch.accepted()
}

/// Complete's shape: no live branch, not Prepared, and the completed record of exactly this
/// branch, by author, basis and accepted count, for the epoch and document the carried unit is.
/// The flush's `preserves_vault_source` then ties that unit to the bytes on disk.
fn completed_shape(
    metadata: &StudioOverlayState,
    next: &StudioOverlayState,
    target: StudioTarget,
    unit: &StudioEpoch,
) -> Result<bool, AppError> {
    let Some(branch) = metadata.overlay() else {
        return Ok(false);
    };
    if next.overlay().is_some() || next.is_prepared() || next.target() != metadata.target() {
        return Ok(false);
    }
    Ok(next
        .completed_branch(target, branch.author(), branch.basis())
        .map_err(invalid)?
        .is_some_and(|outcome| {
            outcome.accepted == branch.accepted()
                && outcome.epoch == unit.epoch()
                && outcome.doc_id == unit.doc_id()
        }))
}

#[cfg(test)]
impl StudioResolvePlan {
    /// Whether R2 decided Complete, for tests that must know which arm they are driving.
    pub(crate) fn is_complete_for_test(&self) -> bool {
        matches!(self.outcome, ResolveOutcome::Complete { .. })
    }

    /// Replace the carried next state, to prove R3's shape check refuses a mismatched one.
    pub(crate) fn with_next_for_test(mut self, replacement: StudioOverlayState) -> Self {
        match &mut self.outcome {
            ResolveOutcome::Absent { next } | ResolveOutcome::Complete { next, .. } => {
                *next = replacement;
            }
        }
        self
    }

    /// Claim a different protocol-byte figure, to prove R3's `verify_record` checks the worker's
    /// accounting fact rather than trusting it (design 6.4.3, M-2).
    pub(crate) fn with_protocol_bytes_for_test(mut self, bytes: usize) -> Self {
        self.protocol_bytes = bytes;
        self
    }

    pub(crate) fn protocol_bytes_for_test(&self) -> usize {
        self.protocol_bytes
    }

    /// Take the carried next state, so a test can move it into another plan.
    pub(crate) fn next_for_test(&self) -> StudioOverlayState {
        match &self.outcome {
            ResolveOutcome::Absent { next } | ResolveOutcome::Complete { next, .. } => next.clone(),
        }
    }
}
