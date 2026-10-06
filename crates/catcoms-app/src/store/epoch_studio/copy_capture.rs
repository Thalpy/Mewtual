//! The composite capture copy needs (design 5.2), stage C1.
//!
//! Revision 1 of the design claimed that copy needed nothing beyond the accepted inspection capture
//! and that only the rebuild function changed. That was withdrawn: the inspection capture holds one
//! record's plaintext and stamp and **nothing about a destination**, so a planner that has to check
//! capacity, tombstones and conflicts against where the work is going has no bytes to check them
//! against.
//!
//! So C1 takes a second bounded capture in the **same custody visit under the same preparation
//! permit**: the destination's authenticated Studio source record, and its recovery record if it
//! has one, with the digest and physical size of each. No projection is materialised here. Both
//! projections are built on the detached worker, which is what keeps custody spent on evidence
//! rather than on decoding.
//!
//! Nothing in this module is authority. The captured bytes are a *proposal's* inputs; C3 and C4
//! re-read both records and compare before anything durable happens, because a plan derived from
//! superseded bytes is a stale proposal however well formed it is.
use super::super::epoch_recovery;
use super::super::{StudioInspectionCapture, StudioInspectionStamp};
use super::*;
use crate::studio::restore::{
    PlanScope, StudioRecoveryDisposition, StudioRecoveryItem, StudioRecoveryMode,
};
use catcoms_crypto::DeviceId;

/// Record identity captured under custody. No key, store handle, `Server` or budget, so it is safe
/// to hold across a detach.
pub(crate) struct StudioDestinationStamp {
    mount: Arc<()>,
    pub(in crate::store) server: u64,
    pub(in crate::store) document: LogicalDocument,
    pub(in crate::store) target: StudioTarget,
    /// The device the capture was taken for. A copy is authored by whoever asked for it, and a
    /// capture taken for one device must not be finished by another.
    actor: DeviceId,
    /// Public context the detached decode needs, and **deliberately not a currency field**.
    ///
    /// `StudioEpoch::prepare_vault_source` takes the designated owner because a vault snapshot is
    /// scoped to one, so the planner cannot decode the destination without it. A change of owner
    /// does not change the destination's *content*, which is what a copy proposal is about, and the
    /// two record digests already catch every change that does. Making it a currency field would
    /// invalidate plans for a reason unrelated to what they propose.
    owner: DeviceId,
    /// `(blake3 of the authenticated plaintext, physical bytes)`, the same currency contract
    /// `studio_inspection_is_current` uses. Two values rather than one because a record that
    /// changed size without changing its plaintext digest would be a sealing anomaly worth
    /// refusing, not worth tolerating.
    source: (blake3::Hash, u64),
    /// `None` when the destination has no recovery record. Absence is part of the stamp: a
    /// destination that acquires one between the plan and the apply has changed, because a
    /// tombstone in a newly retained version can block a resurrection the plan thought was free.
    recovery: Option<(blake3::Hash, u64)>,
}

impl std::fmt::Debug for StudioDestinationStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioDestinationStamp { .. }")
    }
}

/// Authenticated destination bytes and their currency stamp.
pub(crate) struct StudioDestinationCapture {
    pub(in crate::store) stamp: StudioDestinationStamp,
    pub(in crate::store) source: Zeroizing<Vec<u8>>,
    pub(in crate::store) recovery: Option<Zeroizing<Vec<u8>>>,
}

impl std::fmt::Debug for StudioDestinationCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioDestinationCapture { .. }")
    }
}

/// What a renderer asked to copy, and where.
#[derive(Clone, Copy, Debug)]
pub struct StudioOverlayCopyChoice {
    pub destination: StudioTarget,
    pub item: StudioRecoveryItem,
    pub mode: StudioRecoveryMode,
}

/// C2's output: a proposal plus everything C3 and C4 need to prove it still describes reality.
pub(crate) struct StudioOverlayCopyPlan {
    pub(in crate::store) source: StudioInspectionStamp,
    pub(in crate::store) destination: StudioDestinationStamp,
    /// What the plan was asked for.
    ///
    /// Carried because C3 and C4 have to re-examine the **item** under custody: an `Object` put
    /// needs an existence probe the detached worker cannot run, and a plan that did not remember
    /// what it resolved could not be re-examined at all.
    pub(in crate::store) choice: StudioOverlayCopyChoice,
    /// The destination's epoch identity at plan time. C3 and C4 require it unchanged and `Open`,
    /// because a proposal for one epoch is not a proposal for its successor.
    pub(in crate::store) epoch_id: u128,
    pub(in crate::store) phase: EpochPhase,
    /// `recovery_fingerprint()` of the destination projection the plan was built against, which is
    /// what an apply echoes back as `expected_projection`.
    pub(in crate::store) fingerprint: [u8; 32],
    pub(in crate::store) plan: crate::studio::restore::StudioRecoveryPlan,
}

impl std::fmt::Debug for StudioOverlayCopyPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioOverlayCopyPlan")
            .field("disposition", &self.plan.disposition)
            .finish_non_exhaustive()
    }
}

/// Accessors rather than public fields: the stamps and the plan's body are the store's business,
/// and a caller that could take a stamp out could compare it against something else.
impl StudioOverlayCopyPlan {
    pub(crate) fn destination_target(&self) -> StudioTarget {
        self.destination.target
    }
    pub(crate) fn epoch_id(&self) -> u128 {
        self.epoch_id
    }
    pub(crate) fn phase(&self) -> EpochPhase {
        self.phase
    }
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
    pub(crate) fn disposition(&self) -> StudioRecoveryDisposition {
        self.plan.disposition
    }
    pub(crate) fn body(&self) -> Option<&Vec<u8>> {
        self.plan.body.as_ref()
    }
    pub(crate) fn original_author(&self) -> Option<DeviceId> {
        self.plan.original_author
    }
    /// What the plan resolved, which is not the same as what it was asked for. A preview reports
    /// this rather than the request.
    pub(crate) fn source_ops(&self) -> &[[u8; 32]] {
        &self.plan.source_ops
    }
    pub(crate) fn choice(&self) -> StudioOverlayCopyChoice {
        self.choice
    }
    /// Downgrade a plan whose target turned out not to exist.
    ///
    /// Taken as a method rather than performed by the caller so the two fields that must move
    /// together do: a held plan carries no body and no `source_ops`, because reporting ids for a
    /// proposal that will not be offered would report work read on behalf of a refusal.
    pub(crate) fn hold(&mut self, disposition: StudioRecoveryDisposition) {
        self.plan.disposition = disposition;
        self.plan.body = None;
        self.plan.original_author = None;
        self.plan.source_ops.clear();
    }
}

/// The two captures C1 took, held together so C2 cannot be run against a mismatched pair.
pub(crate) struct StudioOverlayCopyCapture {
    pub(in crate::store) source: StudioInspectionCapture,
    pub(in crate::store) destination: StudioDestinationCapture,
}

impl StudioOverlayCopyCapture {
    /// C2, detached. **No store, no `ServerGroup`, no `MlsDevice`, no key and no writer.**
    ///
    /// Everything it needs is public context captured under custody: the group id from the
    /// destination document, the actor, and the designated owner that a vault snapshot is scoped
    /// to. The result is a proposal and nothing more. C3 revalidates every stamp. C4 compares no
    /// stamp: it re-plans from the records it reads then and requires the same epoch, destination
    /// projection, a `Ready` disposition and the same body before anything durable happens.
    pub(crate) fn plan(
        self,
        choice: StudioOverlayCopyChoice,
        scope: PlanScope,
    ) -> Result<StudioOverlayCopyPlan, AppError> {
        if choice.destination != self.destination.stamp.target {
            return Err(invalid("copy plan names a destination it did not capture"));
        }
        // The branch, rebuilt exactly as an inspection would rebuild it. A copy source that cannot
        // be reconstructed has no projection to resolve a value out of, so unlike archiving this
        // path genuinely does require the typed rebuild.
        let (source_stamp, inspected) = self.source.rebuild()?;
        let draft = inspected
            .draft
            .ok_or_else(|| invalid("no local draft to copy from"))?;
        let historical = draft.projection().clone();

        let stamp = self.destination.stamp;
        let document = &stamp.document;
        let scope_bytes = super::scope_bytes(stamp.server, document)?;
        let (stored, snapshot) =
            super::decode_record(&self.destination.source, &scope_bytes, document)?;
        if stored != stamp.target {
            return Err(invalid("copy destination record names another target"));
        }
        let unit = catcoms_replication::studio::StudioEpoch::prepare_vault_source(
            snapshot,
            &document.server_id,
            stamp.target,
            stamp.actor,
            stamp.owner,
        )
        .map_err(invalid)?;
        let current = unit.projection().map_err(invalid)?;

        // The destination's own retained versions. They exist here for one reason: a deletion in a
        // version the destination still retains blocks a resurrection even when it was compacted
        // out of the current checkpoint.
        let history = match &self.destination.recovery {
            None => Vec::new(),
            Some(plain) => {
                let recovery_scope = epoch_recovery::scope_bytes(stamp.server, document)?;
                let state =
                    super::super::EpochRecoveryState::decode(plain, &recovery_scope, document)?;
                state
                    .retained()
                    .chain(state.staged())
                    .map(|s| {
                        catcoms_replication::studio::StudioRecovery::from_snapshot(
                            s,
                            document,
                            stamp.target.channel(),
                        )
                        .map_err(invalid)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
        };

        let plan = crate::studio::restore::plan(
            &current,
            &historical,
            &history,
            choice.item,
            choice.mode,
            stamp.actor,
            scope,
        )?;
        Ok(StudioOverlayCopyPlan {
            fingerprint: current.recovery_fingerprint().map_err(invalid)?,
            epoch_id: unit.doc_id(),
            phase: unit.phase(),
            source: source_stamp,
            destination: stamp,
            choice,
            plan,
        })
        // No branch identity is carried, deliberately. At C3 the source stamp already requires the
        // branch's intent record to be byte-identical, which covers every way the branch could
        // move. A second branch check would be a second representation of the same fact, and the
        // kind that drifts. At C4 there is no stamp. A branch that moved is refused only if the
        // re-plan no longer resolves the named value or rebuilds a different body; otherwise the
        // copy carries the value the branch holds then (design C1'). Copy is also not a
        // preservation claim (C-P), so binding it to a branch generation would suggest an
        // accounting relationship it does not have.
    }
}

impl ServerStore {
    /// C1 in one call: the accepted source capture plus the destination's two records, one permit,
    /// one visit. Held together so C2 cannot be run against a mismatched pair.
    pub(crate) fn capture_studio_overlay_copy(
        &self,
        server: u64,
        group: &ServerGroup,
        source: StudioTarget,
        destination: StudioTarget,
        device: &MlsDevice,
    ) -> Result<StudioOverlayCopyCapture, AppError> {
        // Same-document copy - the primary case - addresses one target twice, and that is fine:
        // the source stamp describes the intent record and the destination stamp describes the
        // Studio and recovery records, so the two never overlap even when the target does.
        Ok(StudioOverlayCopyCapture {
            source: self.capture_studio_inspection(
                server,
                &group.group_id(),
                source,
                device.device_id(),
            )?,
            destination: self.capture_studio_destination(server, group, destination, device)?,
        })
    }

    /// Whether the copy destination's document is under a **transfer hold**: a staged (Prepared)
    /// handoff of its branch (design 6.3 C1').
    ///
    /// A copy is an ordinary Apply into the destination, and the publication path already refuses
    /// one while the destination is Prepared. Without this check the refusal came only at C4,
    /// after C3 had told the user the copy was Ready (the review's M4). Asked at C1 and again at
    /// C3, since a hold can be staged in between. Retryable: the hold ends when its handoff
    /// resolves.
    ///
    /// The destination's document is derived from its target, so another channel label for the
    /// same Flipnote names the same record and the same hold (C-0). A hold on the *source* is
    /// deliberately not asked about: copying out of a branch whose handoff is staged is permitted,
    /// and neither clears `Prepared` nor counts as evidence that the handoff completed.
    pub(crate) fn studio_copy_destination_held(
        &self,
        server: u64,
        group: &ServerGroup,
        destination: StudioTarget,
    ) -> Result<bool, AppError> {
        let document = destination.document(&group.group_id()).map_err(invalid)?;
        Ok(self
            .load_epoch_intents_structural(server, &document)?
            .handoff_prepared())
    }

    /// All three records a copy proposal rests on, rechecked together.
    ///
    /// One call rather than two at the call site, because "the source is current" and "the
    /// destination is current" are not separately meaningful: a proposal is stale if any of the
    /// three records moved, and a caller that could check one and forget the other would have a
    /// proposal it believed in for the wrong reason.
    pub(crate) fn studio_copy_is_current(
        &self,
        server: u64,
        group: &ServerGroup,
        device: &MlsDevice,
        plan: &StudioOverlayCopyPlan,
    ) -> Result<bool, AppError> {
        if !self.studio_inspection_is_current(
            server,
            &group.group_id(),
            plan.source.target(),
            device.device_id(),
            &plan.source,
        )? {
            return Ok(false);
        }
        self.studio_destination_is_current(
            server,
            group,
            plan.destination.target,
            device,
            &plan.destination,
        )
    }

    /// Both destination records, read once under the caller's existing custody and permit.
    ///
    /// The destination's `LogicalDocument` is derived from its target rather than accepted
    /// alongside it, so a request cannot name one channel's target and another channel's document
    /// (C-0: the destination's identity is its complete `LogicalDocument`).
    ///
    /// Decodes nothing. A malformed destination record is the detached worker's problem and
    /// refusing it here would spend custody on work that does not need it.
    pub(crate) fn capture_studio_destination(
        &self,
        server: u64,
        group: &ServerGroup,
        destination: StudioTarget,
        device: &MlsDevice,
    ) -> Result<StudioDestinationCapture, AppError> {
        current_member(group, device)?;
        let document = destination.document(&group.group_id()).map_err(invalid)?;
        let source = self
            .read_studio_record_bounded(
                &scope_bytes(server, &document)?,
                source::MAX_RETAINED_BYTES as usize,
            )?
            .ok_or_else(|| invalid("copy destination has no Studio source"))?;
        let recovery =
            self.read_scoped_recovery_plain(&epoch_recovery::scope_bytes(server, &document)?)?;
        Ok(StudioDestinationCapture {
            stamp: StudioDestinationStamp {
                mount: self.registry_mount(),
                server,
                document,
                target: destination,
                actor: device.device_id(),
                owner: group
                    .designated_committer()
                    .ok_or_else(|| invalid("no current owner"))?,
                source: (blake3::hash(&source.plain), source.physical_bytes),
                recovery: recovery
                    .as_ref()
                    .map(|r| (blake3::hash(&r.plain), r.physical_bytes)),
            },
            source: source.plain,
            recovery: recovery.map(|r| r.plain),
        })
    }

    /// Re-read **both** destination records and compare digest and physical size.
    ///
    /// Run at C3, alongside the source capture's own `studio_inspection_is_current`. Three records
    /// have to be unchanged for a copy proposal to still describe reality: the branch it came from,
    /// and the destination's two. C4 does not call it: it re-captures and re-plans, and a changed
    /// destination is refused there on its `epoch_id`, projection fingerprint, disposition or body.
    pub(crate) fn studio_destination_is_current(
        &self,
        server: u64,
        group: &ServerGroup,
        destination: StudioTarget,
        device: &MlsDevice,
        stamp: &StudioDestinationStamp,
    ) -> Result<bool, AppError> {
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount())
            || stamp.server != server
            || stamp.target != destination
            || stamp.actor != device.device_id()
            || stamp.document.server_id != group.group_id()
            || stamp.document != destination.document(&group.group_id()).map_err(invalid)?
        {
            return Ok(false);
        }
        let Some(source) = self.read_studio_record_bounded(
            &scope_bytes(server, &stamp.document)?,
            source::MAX_RETAINED_BYTES as usize,
        )?
        else {
            return Ok(false);
        };
        if (blake3::hash(&source.plain), source.physical_bytes) != stamp.source {
            return Ok(false);
        }
        // Compared as an `Option`, so acquiring a recovery record where there was none, and losing
        // one where there was, are both changes. Reading only "if we captured one" would let a
        // newly retained version's tombstones appear under a plan that never saw them.
        let recovery = self
            .read_scoped_recovery_plain(&epoch_recovery::scope_bytes(server, &stamp.document)?)?
            .map(|r| (blake3::hash(&r.plain), r.physical_bytes));
        Ok(recovery == stamp.recovery)
    }
}

/// Test seams for the Server-level copy tests, which cannot reach the store's write hooks or the
/// intent ledger (both deliberately private to the store).
#[cfg(test)]
impl ServerStore {
    /// Stage a durable transfer hold on `target`'s live branch: the state a handoff leaves when it
    /// is interrupted after its Prepared record and before its Source write. Driven through the
    /// real handoff with a refusal injected at the Source write, so nothing is synthesised.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stage_studio_transfer_hold_for_test(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        tenure: u64,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) {
        let document = target.document(&group.group_id()).unwrap();
        let basis = self
            .load_epoch_intents_structural(server, &document)
            .unwrap()
            .handoff_metadata()
            .and_then(|m| m.overlay())
            .expect("a live branch to stage a transfer hold on")
            .basis();
        let error = self
            .handoff_studio_overlay_with_io(
                server,
                group,
                target,
                device,
                basis,
                Some(tenure),
                rng,
                budget,
                &mut WriteHooks::Hooked {
                    before: Some(&mut |at: WriteTag, _: &std::path::Path, _: &[u8]| {
                        if at == WriteTag::Source {
                            return Intercept::Fail(invalid("staged transfer hold"));
                        }
                        Intercept::Continue
                    }),
                    before_sync: None,
                    before_unlink: None,
                    after: None,
                },
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("staged transfer hold"),
            "the handoff must stop at the injected Source refusal, got: {error}"
        );
        assert!(self
            .load_epoch_intents_structural(server, &document)
            .unwrap()
            .handoff_prepared());
    }

    /// The live branch of `document`, as everything that records it: its id, its content hash, its
    /// accepted count and the overlay metadata's own canonical bytes.
    pub(crate) fn studio_branch_identity_for_test(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> ([u8; 32], [u8; 32], usize, Vec<u8>) {
        let state = self.load_epoch_intents(server, document).unwrap();
        let metadata = state.handoff_metadata().expect("a live branch");
        (
            metadata.branch_id().expect("a live branch"),
            metadata.branch_content(&state.ledger).unwrap(),
            metadata.overlay().expect("a live branch").accepted(),
            metadata.encode_vault(&state.ledger).unwrap(),
        )
    }

    /// `document`'s whole intent ledger, encoded. The branch identity above covers the branch's own
    /// entries and handoff; this covers every other intent too, so a test can show that a copy
    /// retired or added nothing anywhere in the source's record.
    pub(crate) fn studio_intent_ledger_for_test(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Vec<u8> {
        self.load_epoch_intents(server, document)
            .unwrap()
            .ledger
            .encode()
            .unwrap()
    }
}
