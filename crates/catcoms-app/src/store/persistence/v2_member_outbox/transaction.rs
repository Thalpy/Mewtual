//! Presealed transitions, the final save-admission edge, the finite admitted workers, and
//! the durable write protocol shared by warm work and cold roll-forward.
//!
//! Write protocol (one transaction; every step is idempotent under recovery):
//!
//! 1. Physical precondition: `head.bin` is byte-identical to the owner's selected head and
//!    `outbox.wal`, `outbox.wal.stage.v1`, `head.stage.v1` are all absent. A stale owner, a
//!    foreign leftover or unfinished cleanup is refused here, before anything is written.
//! 2. WAL stage: create/complete the exact WAL file, sync it, sync the directory.
//! 3. WAL MAIN: rename the stage onto `outbox.wal`, sync the directory. This is the commit
//!    point: from here recovery finishes the transaction; before it, recovery either promotes
//!    a complete authenticated stage or removes a torn one.
//! 4. Outputs: create or prefix-complete every exact immutable object, then one directory
//!    barrier.
//! 5. Selector CAS: create/complete `head.stage.v1`, sync, re-check the parent is still
//!    selected, rename onto `head.bin`, sync the directory. The outcome is cached here as
//!    SavedLocal BEFORE anything below can fail.
//! 6. Cleanup: delete each WAL-listed retiree (already-absent is fine), directory barrier,
//!    delete `outbox.wal`, directory barrier. Only an absent WAL lets a new transaction start
//!    (step 1), so cleanup is durable before another transaction can touch the control files,
//!    and a failed cleanup leaves the WAL in place for recovery to finish.
//!
//! No signer, SDK, projection, network, payment wait or await runs anywhere in this file.
use super::data::{
    refused, signed_request_digest, MemberHeadId, MemberOutboxContextId, MemberReceiptData,
    MemberRequestDigest, MemberRequestToken, MemberSignedOriginal,
};
use super::engine::{
    plan_refusal, receipt_of, require_payment, successor, verify_settled, DiskNodes,
    MemberDebtBatch, MemberOutbox, MemberOutboxError, MemberOutboxPressure, MemberOutboxRefusal,
    Namespace, OwnerState, MEMBER_OUTBOX_APPEND_FEE, MEMBER_OUTBOX_CHECKPOINT_FEE,
};
use super::frames::{
    encode_chunk_page, EntryData, HeadData, ObjectRole, OutputData, RetiredData, SelectedHead,
    SettledData, WalData, WalKind, HEAD_FILE_CAP, MAX_NODE_OUTPUTS, MAX_NODE_OUTPUT_BYTES,
    PAGE_FRAME_OVERHEAD, WAL_FILE_CAP,
};
use super::fs::{
    ensure_exact, observe, read_bounded, remove_listed, rename_fixed, Presence, Removal,
};
use super::trie::{
    insert_entry_rows, insert_settled_rows, lookup, retiree_unreachable, RowKey, SealSink,
    TriePlan, NODE_PLAIN_CAP,
};
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::AppError;
use catcoms_replication::chat::cold_actions::{OriginalActionDigest, PreparedActionPages};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::ActionId;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::{
    open_private_chat_page, PreparedPrivateChatPage, PrivateChatPageId,
};
use catcoms_storage::VaultPhysicalWorkLease;
use zeroize::Zeroizing;

/// Every write/barrier/selector boundary. Tests inject a crash or a pause at each one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Cut {
    NamespaceCreated,
    NamespaceParentSyncPending,
    NamespaceParentSynced,
    InitialStageWritten,
    WalStageWritten,
    WalMainNamed,
    OutputWritten(usize),
    OutputsSynced,
    HeadStageWritten,
    Selected,
    RetiredRemoved(usize),
    WalRetired,
    ScratchRemoved,
    StagePromoted,
}

#[cfg(test)]
pub(super) enum HookAction {
    /// Stop here as if the process died: the caller drops the owner and reopens cold.
    Crash,
    /// Run a callback (for example, block until released) and then continue.
    Call(Box<dyn FnOnce()>),
}

#[cfg(test)]
thread_local! {
    pub(super) static HOOKS: std::cell::RefCell<Vec<(Cut, HookAction)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The write boundaries other modules' regressions may target (a subset of [`Cut`]). Hooks are
/// thread-local: they fire only for work run on the arming thread.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(in crate::store) enum MemberOutboxTestCut {
    /// After the complete WAL stage is written, BEFORE it becomes MAIN (pre-commit).
    WalStage,
    /// After the WAL is MAIN and the first immutable output exists (commit point passed).
    FirstOutput,
    /// After the head stage is complete, before the selector CAS.
    HeadStage,
}
#[cfg(test)]
impl MemberOutboxTestCut {
    fn cut(self) -> Cut {
        match self {
            Self::WalStage => Cut::WalStageWritten,
            Self::FirstOutput => Cut::OutputWritten(0),
            Self::HeadStage => Cut::HeadStageWritten,
        }
    }
}
/// Stop the next commit at `cut` as if the process died (an `Io` error is returned).
#[cfg(test)]
pub(in crate::store) fn member_outbox_crash_at_for_test(cut: MemberOutboxTestCut) {
    HOOKS.with(|hooks| hooks.borrow_mut().push((cut.cut(), HookAction::Crash)));
}
/// Run `call` when the next commit reaches `cut` (to pause, cancel or unwind), then continue.
#[cfg(test)]
pub(in crate::store) fn member_outbox_call_at_for_test(
    cut: MemberOutboxTestCut,
    call: Box<dyn FnOnce()>,
) {
    HOOKS.with(|hooks| hooks.borrow_mut().push((cut.cut(), HookAction::Call(call))));
}
#[cfg(test)]
pub(in crate::store) fn member_outbox_disarm_for_test() {
    HOOKS.with(|hooks| hooks.borrow_mut().clear());
}

/// Pass one boundary. Production: a no-op. Tests: fire the first matching hook once.
pub(super) fn reach(cut: Cut) -> Result<(), AppError> {
    #[cfg(test)]
    {
        let hit = HOOKS.with(|hooks| {
            let mut hooks = hooks.borrow_mut();
            let at = hooks.iter().position(|(c, _)| *c == cut)?;
            Some(hooks.remove(at).1)
        });
        match hit {
            Some(HookAction::Crash) => {
                return Err(AppError::Io("member outbox injected crash".into()));
            }
            Some(HookAction::Call(call)) => call(),
            None => {}
        }
    }
    let _ = cut;
    Ok(())
}

/// One complete presealed transition: exact WAL bytes, outputs, retirees and candidate head.
/// Built once; every retry and recovery reuses these exact bytes, never a reseal.
pub(super) struct PreparedTransition {
    pub(super) kind: WalKind,
    pub(super) parent: MemberHeadId,
    pub(super) candidate: SelectedHead,
    pub(super) wal_file: Zeroizing<Vec<u8>>,
    pub(super) outputs: Vec<OutputData>,
    pub(super) retired: Vec<RetiredData>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Resume {
    /// A new transaction: check the physical precondition and write the WAL first.
    Fresh,
    /// Cold roll-forward: the authenticated WAL MAIN already exists.
    FromMain,
}

/// Why each retiree is known to be unreachable from the new root.
#[derive(Clone, Copy)]
pub(super) enum CleanupProof<'a> {
    /// This process just planned (or re-planned and verified) the transition, so every
    /// retiree is a node it replaced on a copied path.
    Replanned,
    /// Cold cleanup of an already-selected transition: prove each still-present retiree
    /// unreachable by walking the new root down that retiree's own encoded position.
    WalkNewRoot(&'a [u8; 32]),
}

fn require_selected(ns: &Namespace, current: &SelectedHead) -> Result<(), AppError> {
    let now = read_bounded(&ns.head(), HEAD_FILE_CAP, false)?;
    if now.as_slice() != current.file.as_slice() {
        return Err(refused());
    }
    Ok(())
}

/// Steps 1-5 of the write protocol, through the selector CAS and its barrier.
pub(super) fn commit(
    ns: &Namespace,
    current: &SelectedHead,
    t: &PreparedTransition,
    resume: Resume,
) -> Result<(), AppError> {
    if t.parent != current.id {
        return Err(refused());
    }
    require_selected(ns, current)?;
    if resume == Resume::Fresh {
        for path in [ns.wal(), ns.wal_stage(), ns.head_stage()] {
            if observe(&path)? != Presence::Absent {
                return Err(refused());
            }
        }
        ensure_exact(&ns.wal_stage(), &t.wal_file)?;
        ns.dir.sync()?;
        reach(Cut::WalStageWritten)?;
        rename_fixed(&ns.wal_stage(), &ns.wal())?;
        ns.dir.sync()?;
        reach(Cut::WalMainNamed)?;
    }
    for (index, output) in t.outputs.iter().enumerate() {
        ensure_exact(&ns.object(output.role, output.id), &output.frame)?;
        reach(Cut::OutputWritten(index))?;
    }
    ns.dir.sync()?;
    reach(Cut::OutputsSynced)?;
    ensure_exact(&ns.head_stage(), &t.candidate.file)?;
    ns.dir.sync()?;
    reach(Cut::HeadStageWritten)?;
    require_selected(ns, current)?;
    rename_fixed(&ns.head_stage(), &ns.head())?;
    ns.dir.sync()
}

/// Step 6: delete retirees, then retire the WAL itself. Each barrier precedes the next step.
pub(super) fn retire(
    ns: &Namespace,
    context: MemberOutboxContextId,
    selected: &SelectedHead,
    retired: &[RetiredData],
    proof: CleanupProof<'_>,
) -> Result<(), AppError> {
    reach(Cut::Selected)?;
    for (index, item) in retired.iter().enumerate() {
        let path = ns.object(ObjectRole::Node, item.id);
        if let CleanupProof::WalkNewRoot(key) = proof {
            if observe(&path)? == Presence::Present {
                let frame = read_bounded(&path, NODE_PLAIN_CAP + PAGE_FRAME_OVERHEAD, false)?;
                let plain = open_private_chat_page(key, item.id, &frame)?;
                let mut nodes = DiskNodes { ns, key };
                if frame.len() != item.bytes as usize
                    || !retiree_unreachable(
                        context,
                        selected.data.root,
                        item.id,
                        &plain,
                        &mut nodes,
                    )?
                {
                    return Err(refused());
                }
            }
        }
        // WAL-listed retiree: an already-absent one was removed by an interrupted cleanup.
        remove_listed(&path)?;
        reach(Cut::RetiredRemoved(index))?;
    }
    ns.dir.sync()?;
    if remove_listed(&ns.wal())? != Removal::Removed {
        return Err(refused());
    }
    ns.dir.sync()?;
    reach(Cut::WalRetired)
}

fn output_of(role: ObjectRole, page: &PreparedPrivateChatPage) -> OutputData {
    OutputData {
        role,
        id: page.id(),
        frame: Zeroizing::new(page.as_bytes().to_vec()),
    }
}

struct TransitionParts {
    kind: WalKind,
    root: Option<PrivateChatPageId>,
    outputs: Vec<OutputData>,
    retired: Vec<RetiredData>,
    settled: Vec<SettledData>,
    next_ordinal: u64,
    front: u64,
}

impl MemberOutbox {
    /// Seal the candidate head and WAL once, then apply the physical quota (old and new
    /// nodes, WAL and head stage all coexist until cleanup finishes).
    fn seal_transition(
        &self,
        head: &SelectedHead,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
        parts: TransitionParts,
    ) -> Result<PreparedTransition, MemberOutboxError> {
        let TransitionParts {
            kind,
            root,
            outputs,
            retired,
            settled,
            next_ordinal,
            front,
        } = parts;
        let data = successor(head, root, &outputs, &retired, next_ordinal, front)?;
        if matches!(kind, WalKind::Append) {
            self.require_checkpoint_reserve(&data)?;
        }
        let candidate = SelectedHead::open(
            key,
            self.context.id,
            data.seal_file(key, self.context.id, rng)?,
        )?;
        if candidate.data != data {
            return Err(refused().into());
        }
        let wal = WalData {
            kind,
            parent: head.id,
            candidate: candidate.file.clone(),
            outputs,
            retired,
            settled,
        };
        let wal_file = wal.seal_file(key, self.context.id, rng)?;
        let WalData {
            outputs, retired, ..
        } = wal;
        self.check_quota(&head.data, &outputs, wal_file.len() + candidate.file.len())?;
        Ok(PreparedTransition {
            kind,
            parent: head.id,
            candidate,
            wal_file,
            outputs,
            retired,
        })
    }

    fn check_quota(
        &self,
        head: &HeadData,
        outputs: &[OutputData],
        transient: usize,
    ) -> Result<(), MemberOutboxError> {
        let added: u64 = outputs.iter().map(|o| o.frame.len() as u64).sum();
        let bytes = head
            .live_bytes
            .checked_add(added)
            .and_then(|n| n.checked_add(transient as u64));
        if bytes.is_none_or(|n| n > self.quota.max_live_bytes) {
            return Err(MemberOutboxRefusal::Pressure(MemberOutboxPressure::Bytes).into());
        }
        let objects = head.live_objects.checked_add(outputs.len() as u64 + 2);
        if objects.is_none_or(|n| n > self.quota.max_live_objects) {
            return Err(MemberOutboxRefusal::Pressure(MemberOutboxPressure::Objects).into());
        }
        Ok(())
    }

    /// Do not admit debt that consumes the disk policy needed to settle it.
    /// One checkpoint adds at most B node bytes/N objects and settles k>=1
    /// entries. Thus L' + (P-k)*B <= L + B + (P-1)*B <= L + P*B.
    /// The same monotone bound holds for object counts. The fixed WAL/head
    /// overlap is additional. These are logical quota commitments, not physical
    /// preallocation or protection against external ENOSPC/trie exhaustion.
    /// Existing exact-operation peak checks remain necessary while old retirees
    /// and new nodes coexist. Lowering policy below an existing commitment does
    /// not erase receipts, but cannot retain this progress guarantee.
    fn require_checkpoint_reserve(&self, candidate: &HeadData) -> Result<(), MemberOutboxError> {
        let (bytes, objects) = checkpoint_commitment_data(candidate);
        if bytes.is_none_or(|bytes| bytes > self.quota.max_live_bytes) {
            return Err(MemberOutboxRefusal::Pressure(MemberOutboxPressure::Bytes).into());
        }
        if objects.is_none_or(|objects| objects > self.quota.max_live_objects) {
            return Err(MemberOutboxRefusal::Pressure(MemberOutboxPressure::Objects).into());
        }
        Ok(())
    }

    /// Prepare one presealed append of a signed original under `token`. Lookup happens first:
    /// an existing token is `AlreadySaved` (same request) or `TokenConflict`; an existing
    /// ActionId is `DuplicateAction`. Pressure and index exhaustion refuse here, before any
    /// admission. On refusal the original custody is returned untouched.
    pub(in crate::store) fn prepare_append(
        &self,
        key: &[u8; 32],
        token: MemberRequestToken,
        original: MemberSignedOriginal,
        rng: &mut dyn CryptoRngCore,
        payment: CatalogReadReservation,
    ) -> Result<PreparedMemberAppend, Box<ReturnedMemberOriginal>> {
        if let Err(error) = require_payment(&payment, &self.pool, MEMBER_OUTBOX_APPEND_FEE) {
            return Err(Box::new(ReturnedMemberOriginal { original, error }));
        }
        match self.prepare_append_inner(key, token, &original, rng) {
            Ok((transition, receipt)) => Ok(PreparedMemberAppend {
                transition,
                receipt,
                original,
                payment,
            }),
            Err(error) => Err(Box::new(ReturnedMemberOriginal { original, error })),
        }
    }

    fn prepare_append_inner(
        &self,
        key: &[u8; 32],
        token: MemberRequestToken,
        original: &MemberSignedOriginal,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(PreparedTransition, MemberReceiptData), MemberOutboxError> {
        let head = self.clean()?;
        let context = &self.context;
        let action = original.action();
        if action.signer() != &context.device
            || action.spec().realm.group() != context.group.as_slice()
        {
            return Err(MemberOutboxRefusal::Foreign.into());
        }
        let request = signed_request_digest(context.id, token, action.spec())?;
        if let Some((row, entry)) = self.token_row(key, head, token)? {
            let receipt = Box::new(receipt_of(context, token, &row, &entry));
            return Err(if row.request == request {
                MemberOutboxRefusal::AlreadySaved(receipt)
            } else {
                MemberOutboxRefusal::TokenConflict(receipt)
            }
            .into());
        }
        let mut nodes = DiskNodes { ns: &self.ns, key };
        if lookup(
            context.id,
            head.data.root,
            RowKey::action(action.id()),
            &mut nodes,
        )?
        .is_some()
        {
            return Err(MemberOutboxRefusal::DuplicateAction.into());
        }
        // Capacity applies only to new debt, after BOTH token and ActionId lookup.
        // An existing action under another token may already have been submitted: it
        // must retain DuplicateAction semantics even when the queue is full. Pressure
        // instead means this new action was not saved, which is a different promise.
        if head.data.pending() >= self.quota.max_pending {
            return Err(MemberOutboxRefusal::Pressure(MemberOutboxPressure::Pending).into());
        }
        // Author every immutable output exactly once, under the RNG seam.
        let pages = PreparedActionPages::prepare(context.vault, context.profile, action)
            .map_err(|_| refused())?;
        let ordinal = head.data.next_ordinal;
        let mut outputs = Vec::with_capacity(1 + pages.chunk_count());
        let mut chunks = Vec::with_capacity(pages.chunk_count());
        for index in 0..pages.chunk_count() {
            let chunk = pages.encode_chunk(index).map_err(|_| refused())?;
            let plain = encode_chunk_page(context.id, ordinal, index, &chunk)?;
            let page =
                PreparedPrivateChatPage::prepare(key, &plain, rng).map_err(AppError::from)?;
            chunks.push(page.id());
            outputs.push(output_of(ObjectRole::Chunk, &page));
        }
        let entry = EntryData {
            ordinal,
            token,
            request,
            admission: original.admission(),
            action: pages.action_id(),
            original: pages.original_digest(),
            original_len: u32::try_from(action.encode().len()).map_err(|_| refused())?,
            chunks,
        };
        let entry_page = PreparedPrivateChatPage::prepare(key, &entry.encode(context.id)?, rng)
            .map_err(AppError::from)?;
        outputs.push(output_of(ObjectRole::Entry, &entry_page));
        let mut plan = TriePlan::new(context.id, self.limits, head.data.root, &mut nodes);
        insert_entry_rows(&mut plan, &entry, entry_page.id()).map_err(plan_refusal)?;
        let mut sink = SealSink {
            key,
            rng: &mut *rng,
            outputs: Vec::new(),
            bytes: 0,
        };
        let (root, retired) = plan.finish(&mut sink).map_err(plan_refusal)?;
        outputs.append(&mut sink.outputs);
        let transition = self.seal_transition(
            head,
            key,
            rng,
            TransitionParts {
                kind: WalKind::Append,
                root,
                outputs,
                retired,
                settled: Vec::new(),
                next_ordinal: ordinal + 1,
                front: head.data.front,
            },
        )?;
        let receipt = MemberReceiptData {
            context: context.id,
            token,
            ordinal,
            entry: entry_page.id(),
            request,
            action: entry.action,
            original: entry.original,
            admission: entry.admission,
        };
        Ok((transition, receipt))
    }

    /// Prepare a publication checkpoint over the batch's contiguous settled prefix. The batch
    /// must be this owner's, start at the current front, and name the exact entries the
    /// selected index holds; a stale or foreign batch is refused and returned.
    pub(in crate::store) fn prepare_checkpoint(
        &self,
        key: &[u8; 32],
        batch: MemberDebtBatch,
        rng: &mut dyn CryptoRngCore,
        payment: CatalogReadReservation,
    ) -> Result<PreparedMemberCheckpoint, Box<ReturnedMemberDebtBatch>> {
        if let Err(error) = require_payment(&payment, &self.pool, MEMBER_OUTBOX_CHECKPOINT_FEE) {
            return Err(Box::new(ReturnedMemberDebtBatch { batch, error }));
        }
        match self.prepare_checkpoint_inner(key, &batch, rng) {
            Ok((transition, settled)) => Ok(PreparedMemberCheckpoint {
                context: self.context.id,
                transition,
                settled,
                payment,
            }),
            Err(error) => Err(Box::new(ReturnedMemberDebtBatch { batch, error })),
        }
    }

    fn prepare_checkpoint_inner(
        &self,
        key: &[u8; 32],
        batch: &MemberDebtBatch,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(PreparedTransition, usize), MemberOutboxError> {
        let head = self.clean()?;
        if batch.context != self.context.id {
            return Err(MemberOutboxRefusal::Foreign.into());
        }
        if batch.front != head.data.front {
            return Err(MemberOutboxRefusal::Stale.into());
        }
        let settled = batch.settled_prefix();
        if settled.is_empty() {
            return Err(MemberOutboxRefusal::NotPublishable.into());
        }
        verify_settled(&self.ns, &self.context, key, head, &settled)?;
        let mut nodes = DiskNodes { ns: &self.ns, key };
        let mut plan = TriePlan::new(self.context.id, self.limits, head.data.root, &mut nodes);
        insert_settled_rows(&mut plan, &settled, head.data.generation + 1).map_err(plan_refusal)?;
        let mut sink = SealSink {
            key,
            rng: &mut *rng,
            outputs: Vec::new(),
            bytes: 0,
        };
        let (root, retired) = plan.finish(&mut sink).map_err(plan_refusal)?;
        let outputs = std::mem::take(&mut sink.outputs);
        let count = settled.len();
        let transition = self.seal_transition(
            head,
            key,
            rng,
            TransitionParts {
                kind: WalKind::Checkpoint,
                root,
                outputs,
                retired,
                settled,
                next_ordinal: head.data.next_ordinal,
                front: head.data.front + count as u64,
            },
        )?;
        Ok((transition, count))
    }
}

/// Pure conservative quota arithmetic. Original selected counters are supplied
/// by the owning engine; this DATA calculation issues no admission or recovery.
fn checkpoint_commitment_data(candidate: &HeadData) -> (Option<u64>, Option<u64>) {
    let bytes = candidate
        .pending()
        .checked_mul(MAX_NODE_OUTPUT_BYTES as u64)
        .and_then(|reserve| candidate.live_bytes.checked_add(reserve))
        .and_then(|bytes| bytes.checked_add((WAL_FILE_CAP + HEAD_FILE_CAP) as u64));
    let objects = candidate
        .pending()
        .checked_mul(MAX_NODE_OUTPUTS as u64)
        .and_then(|reserve| candidate.live_objects.checked_add(reserve))
        .and_then(|objects| objects.checked_add(2));
    (bytes, objects)
}

/// A refused preparation hands the signed original back unchanged.
#[derive(Debug)]
pub(in crate::store) struct ReturnedMemberOriginal {
    pub(in crate::store) original: MemberSignedOriginal,
    pub(in crate::store) error: MemberOutboxError,
}

/// A refused checkpoint preparation hands the debt batch back unchanged.
#[derive(Debug)]
pub(in crate::store) struct ReturnedMemberDebtBatch {
    pub(in crate::store) batch: MemberDebtBatch,
    pub(in crate::store) error: MemberOutboxError,
}

/// The exact facts the primary's final save-admission guard checks, at the final edge, with
/// the actual member-source generation and native/UI-session generation: the original signed
/// request (token, request digest, ActionId, original bytes, original Admission) and the
/// selected parent head this transition starts from. DATA only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) struct MemberAppendAdmissionFacts {
    pub(in crate::store) context: MemberOutboxContextId,
    pub(in crate::store) parent: MemberHeadId,
    pub(in crate::store) ordinal: u64,
    pub(in crate::store) token: MemberRequestToken,
    pub(in crate::store) request: MemberRequestDigest,
    pub(in crate::store) action: ActionId,
    pub(in crate::store) original: OriginalActionDigest,
    pub(in crate::store) admission: OwnerFactId,
}

/// Final save-edge admission for ONE prepared append, plus the physical lease its worker
/// holds through every write, barrier and cleanup.
///
/// The one production issuer is the operational host's member-save final edge
/// (`operational::host::member_save`). Before calling [`Self::at_member_save_final_edge`] it
/// has, with no IO in between: issued the member lane's OWN save-source end for exactly these
/// facts (live source negative, the lane's own context, the token Lent with this ActionId, the
/// Plan's historical Admission), re-checked that the Store's member-outbox mount is still the
/// live one, and then observed the native session's complete cancellation authority live
/// (the linearization point). That source end and the native custody travel in the SAME job
/// as the admitted worker and are released only after its last File and cleanup. The
/// constructor is visible to `persistence` only because that host is a sibling subtree; the
/// facts alone are labels and confer nothing without that edge. The cfg(test) constructor
/// grants durability for storage tests only and exercises no authorization path.
pub(in crate::store) struct MemberOutboxSaveAdmission {
    facts: MemberAppendAdmissionFacts,
    lease: VaultPhysicalWorkLease,
}
impl std::fmt::Debug for MemberOutboxSaveAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberOutboxSaveAdmission")
            .finish_non_exhaustive()
    }
}
impl MemberOutboxSaveAdmission {
    /// See the type documentation: only the host's member-save final edge calls this, after
    /// its source, mount and native checks. `lease` is the Store's own physical-work loan.
    pub(in crate::store::persistence) fn at_member_save_final_edge(
        facts: MemberAppendAdmissionFacts,
        lease: VaultPhysicalWorkLease,
    ) -> Self {
        Self { facts, lease }
    }

    #[cfg(test)]
    pub(in crate::store) fn durability_only_for_test(
        facts: MemberAppendAdmissionFacts,
        lease: VaultPhysicalWorkLease,
    ) -> Self {
        Self { facts, lease }
    }
}

/// A presealed, NOT admitted append. Not Clone. Dropping it before admission writes nothing.
pub(in crate::store) struct PreparedMemberAppend {
    transition: PreparedTransition,
    receipt: MemberReceiptData,
    original: MemberSignedOriginal,
    // LAST: the buffers above drop before their payment is refunded.
    payment: CatalogReadReservation,
}
impl std::fmt::Debug for PreparedMemberAppend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedMemberAppend")
            .finish_non_exhaustive()
    }
}
impl PreparedMemberAppend {
    pub(in crate::store) fn admission_facts(&self) -> MemberAppendAdmissionFacts {
        MemberAppendAdmissionFacts {
            context: self.receipt.context,
            parent: self.transition.parent,
            ordinal: self.receipt.ordinal,
            token: self.receipt.token,
            request: self.receipt.request,
            action: self.receipt.action,
            original: self.receipt.original,
            admission: self.receipt.admission,
        }
    }
    pub(in crate::store) fn signed_original(&self) -> &MemberSignedOriginal {
        &self.original
    }
    #[cfg(test)]
    pub(super) fn outputs_for_test(&self) -> &[OutputData] {
        &self.transition.outputs
    }
    #[cfg(test)]
    pub(super) fn initial_peak_for_test(&self) -> (u64, u64) {
        (
            self.transition
                .outputs
                .iter()
                .map(|output| output.frame.len() as u64)
                .sum::<u64>()
                + self.transition.wal_file.len() as u64
                + self.transition.candidate.file.len() as u64,
            self.transition.outputs.len() as u64 + 2,
        )
    }
    #[cfg(test)]
    pub(super) fn retired_for_test(&self) -> &[RetiredData] {
        &self.transition.retired
    }
    /// Abandon before admission: nothing was written; the original custody comes back.
    pub(in crate::store) fn abandon(self) -> MemberSignedOriginal {
        self.original
    }
    /// Non-consuming DATA check of exactly what [`Self::admit`] requires of the owner: this
    /// context, a clean state and the prepared parent still selected. The final edge runs it
    /// BEFORE the native linearization point so nothing fallible follows that point.
    pub(in crate::store) fn admissible_by(&self, owner: &MemberOutbox) -> bool {
        let facts = self.admission_facts();
        owner.context.id == facts.context
            && matches!(&owner.state, OwnerState::Clean(head) if head.id == facts.parent)
    }
    /// Bind the final admission and the owner. Refuses (returning everything) unless the
    /// admission names exactly these facts and the owner still selects the prepared parent.
    pub(in crate::store) fn admit(
        self,
        owner: MemberOutbox,
        admission: MemberOutboxSaveAdmission,
    ) -> Result<AdmittedMemberAppend, Box<RefusedMemberAppendAdmission>> {
        let facts = self.admission_facts();
        let current = matches!(&owner.state, OwnerState::Clean(head) if head.id == facts.parent);
        if admission.facts != facts || owner.context.id != facts.context || !current {
            return Err(Box::new(RefusedMemberAppendAdmission {
                owner,
                prepared: self,
                admission,
                refusal: MemberOutboxRefusal::Stale,
            }));
        }
        let MemberOutboxSaveAdmission { lease, .. } = admission;
        Ok(AdmittedMemberAppend {
            transition: self.transition,
            receipt: self.receipt,
            original: self.original,
            payment: self.payment,
            owner,
            lease,
        })
    }
}

/// Admission refused: every input is handed back unchanged.
pub(in crate::store) struct RefusedMemberAppendAdmission {
    pub(in crate::store) owner: MemberOutbox,
    pub(in crate::store) prepared: PreparedMemberAppend,
    pub(in crate::store) admission: MemberOutboxSaveAdmission,
    pub(in crate::store) refusal: MemberOutboxRefusal,
}
impl std::fmt::Debug for RefusedMemberAppendAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefusedMemberAppendAdmission")
            .field("refusal", &self.refusal)
            .finish_non_exhaustive()
    }
}

/// The finite admitted worker. It owns the exclusive owner (and so its context claim), the
/// presealed bytes, their payment and the physical lease. Running it cannot be cancelled: it
/// completes or stops at a crash-equivalent failure, and only then releases anything.
pub(in crate::store) struct AdmittedMemberAppend {
    transition: PreparedTransition,
    receipt: MemberReceiptData,
    original: MemberSignedOriginal,
    payment: CatalogReadReservation,
    owner: MemberOutbox,
    // LAST: physical exclusion outlives every buffer, File and the owner's own cleanup.
    lease: VaultPhysicalWorkLease,
}
impl std::fmt::Debug for AdmittedMemberAppend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedMemberAppend")
            .finish_non_exhaustive()
    }
}

/// The outcome of one admitted append.
#[derive(Debug)]
pub(in crate::store) enum MemberAppendOutcome {
    /// SavedLocal: durably selected. Cached at the CAS, before cleanup could fail; a cleanup
    /// failure only leaves the returned owner needing recovery.
    Saved(MemberReceiptData),
    /// Stopped before the outcome was known. The save may still complete during recovery
    /// (a complete WAL stage or MAIN is finished, never discarded). Recover, then look the
    /// token up; re-save only on `Absent`, with this exact returned original.
    Uncertain {
        original: MemberSignedOriginal,
        error: Box<AppError>,
    },
}

pub(in crate::store) struct MemberAppendRun {
    pub(in crate::store) owner: MemberOutbox,
    pub(in crate::store) outcome: MemberAppendOutcome,
}

impl AdmittedMemberAppend {
    /// Storage-level worker: commit, then hand the original back on an uncertain outcome.
    /// Shares [`AdmittedMemberCommit::run`] with the production job, so both paths run the
    /// exact same write protocol.
    pub(in crate::store) fn run(self) -> MemberAppendRun {
        let (original, commit) = self.into_commit();
        let MemberCommitRun { owner, outcome } = commit.run();
        let outcome = match outcome {
            Ok(receipt) => MemberAppendOutcome::Saved(receipt),
            Err(error) => MemberAppendOutcome::Uncertain { original, error },
        };
        MemberAppendRun { owner, outcome }
    }

    /// Separate the exact signed original from the presealed commit. The production save job
    /// keeps the original in the Store's recovered-DATA custody while the worker does IO, so
    /// an unwind or an unrun drop can never lose it; the commit needs only presealed bytes.
    pub(in crate::store) fn into_commit(self) -> (MemberSignedOriginal, AdmittedMemberCommit) {
        let Self {
            transition,
            receipt,
            original,
            payment,
            owner,
            lease,
        } = self;
        (
            original,
            AdmittedMemberCommit {
                transition,
                receipt,
                payment,
                owner,
                lease,
            },
        )
    }
}

/// The admitted worker without its signed original (see [`AdmittedMemberAppend::into_commit`]).
/// It owns the exclusive owner, the presealed bytes, their payment and the physical lease.
/// Running it cannot be cancelled; dropping it unrun performs no IO.
pub(in crate::store) struct AdmittedMemberCommit {
    transition: PreparedTransition,
    receipt: MemberReceiptData,
    payment: CatalogReadReservation,
    owner: MemberOutbox,
    // LAST: physical exclusion outlives every buffer, File and the owner's own cleanup.
    lease: VaultPhysicalWorkLease,
}
impl std::fmt::Debug for AdmittedMemberCommit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedMemberCommit")
            .finish_non_exhaustive()
    }
}

/// The outcome of one admitted commit. `Ok` is SavedLocal: durably selected, cached at the CAS
/// before cleanup could fail (a cleanup failure only leaves the owner needing recovery). `Err`
/// is uncertain: recovery may still finish it; recover, then look the token up.
pub(in crate::store) struct MemberCommitRun {
    pub(in crate::store) owner: MemberOutbox,
    pub(in crate::store) outcome: Result<MemberReceiptData, Box<AppError>>,
}

impl AdmittedMemberCommit {
    pub(in crate::store) fn run(self) -> MemberCommitRun {
        let Self {
            transition,
            receipt,
            payment,
            mut owner,
            lease,
        } = self;
        let outcome = match std::mem::replace(&mut owner.state, OwnerState::NeedsRecovery) {
            OwnerState::Clean(current) => {
                match commit(&owner.ns, &current, &transition, Resume::Fresh) {
                    Err(error) => Err(Box::new(error)),
                    Ok(()) => {
                        owner.state = OwnerState::Clean(transition.candidate.clone());
                        let cleaned = retire(
                            &owner.ns,
                            owner.context.id,
                            &transition.candidate,
                            &transition.retired,
                            CleanupProof::Replanned,
                        );
                        if cleaned.is_err() {
                            owner.state = OwnerState::NeedsRecovery;
                        }
                        Ok(receipt)
                    }
                }
            }
            // Admission proved the owner clean; keep the refusal total anyway.
            OwnerState::NeedsRecovery => Err(Box::new(refused())),
        };
        // All Files have closed. Buffers and payment go first; the lease goes last.
        drop(transition);
        drop(payment);
        drop(lease);
        MemberCommitRun { owner, outcome }
    }

    /// De-admit an UNRUN commit: no IO happened, so the lease is released and the exact
    /// presealed append is rebuilt around the SAME original for a later, fresh final edge.
    /// Refuses (returning both) unless `original` is byte-for-byte the one this commit was
    /// admitted for: its ActionId, its Admission and its original-byte digest (which covers
    /// the signature an ActionId alone does not).
    pub(in crate::store) fn de_admit(
        self,
        original: MemberSignedOriginal,
    ) -> Result<(MemberOutbox, PreparedMemberAppend), Box<(MemberOutbox, MemberSignedOriginal)>>
    {
        let Self {
            transition,
            receipt,
            payment,
            owner,
            lease,
        } = self;
        let same = original.action().id() == receipt.action
            && original.admission() == receipt.admission
            && PreparedActionPages::prepare(
                owner.context.vault,
                owner.context.profile,
                original.action(),
            )
            .is_ok_and(|pages| pages.original_digest() == receipt.original);
        let result = if same {
            Ok((
                owner,
                PreparedMemberAppend {
                    transition,
                    receipt,
                    original,
                    payment,
                },
            ))
        } else {
            drop(transition);
            drop(payment);
            Err(Box::new((owner, original)))
        };
        drop(lease);
        result
    }
}

/// A presealed checkpoint. Not Clone; keeps its payment until run or dropped.
pub(in crate::store) struct PreparedMemberCheckpoint {
    context: MemberOutboxContextId,
    transition: PreparedTransition,
    settled: usize,
    payment: CatalogReadReservation,
}
impl std::fmt::Debug for PreparedMemberCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedMemberCheckpoint")
            .field("settled", &self.settled)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) struct MemberCheckpointSaved {
    pub(in crate::store) settled: usize,
    pub(in crate::store) front: u64,
}

pub(in crate::store) struct MemberCheckpointRun {
    pub(in crate::store) owner: MemberOutbox,
    pub(in crate::store) outcome: Result<MemberCheckpointSaved, MemberOutboxError>,
}

impl PreparedMemberCheckpoint {
    /// Run the checkpoint with its physical lease. A checkpoint records local scheduling
    /// only; it needs no member-source admission and grants no publication right.
    pub(in crate::store) fn run(
        self,
        mut owner: MemberOutbox,
        lease: VaultPhysicalWorkLease,
    ) -> MemberCheckpointRun {
        let Self {
            context,
            transition,
            settled,
            payment,
        } = self;
        let current =
            matches!(&owner.state, OwnerState::Clean(head) if head.id == transition.parent);
        if owner.context.id != context || !current {
            return MemberCheckpointRun {
                owner,
                outcome: Err(MemberOutboxRefusal::Stale.into()),
            };
        }
        let outcome = match std::mem::replace(&mut owner.state, OwnerState::NeedsRecovery) {
            OwnerState::Clean(current) => {
                match commit(&owner.ns, &current, &transition, Resume::Fresh) {
                    Err(error) => Err(MemberOutboxError::from(error)),
                    Ok(()) => {
                        owner.state = OwnerState::Clean(transition.candidate.clone());
                        let cleaned = retire(
                            &owner.ns,
                            owner.context.id,
                            &transition.candidate,
                            &transition.retired,
                            CleanupProof::Replanned,
                        );
                        if cleaned.is_err() {
                            owner.state = OwnerState::NeedsRecovery;
                        }
                        Ok(MemberCheckpointSaved {
                            settled,
                            front: transition.candidate.data.front,
                        })
                    }
                }
            }
            OwnerState::NeedsRecovery => Err(MemberOutboxRefusal::NeedsRecovery.into()),
        };
        drop(transition);
        drop(payment);
        drop(lease);
        MemberCheckpointRun { owner, outcome }
    }
}

#[cfg(test)]
mod quota_tests;
