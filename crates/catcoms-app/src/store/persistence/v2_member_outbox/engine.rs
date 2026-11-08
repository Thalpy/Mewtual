//! The exclusive per-context owner: open/create, cold recovery, and bounded reads.
//!
//! Exclusion model. One [`MemberOutbox`] owner exists per context ID per registry, enforced by
//! [`MemberOutboxClaim`]. The owner is the ONLY path to the namespace's files; every read
//! borrows it and every transaction moves it into the admitted worker, so no reader can hold
//! an old root while a transaction replaces and retires nodes. The claim is released only when
//! the owner itself is dropped, never because a caller requested cancellation, and a worker
//! that still owns the owner keeps the claim through all of its IO and cleanup.
//!
//! Physical exclusion across Store instances is the separate `VaultPhysicalWorkLease`: every
//! operation that writes (open/recovery, append, checkpoint) holds one for its whole IO span,
//! so a replacement Store cannot even mount while that work drains. An idle owner holds no
//! lease. The lease is OS exclusion only, never authorization.
use super::data::{
    refused, signed_request_digest, MemberHeadId, MemberOutboxContextData, MemberOutboxContextId,
    MemberReceiptData, MemberRequestData, MemberRequestToken,
};
use super::frames::{
    classify_stage, decode_chunk_page, ControlKind, DispositionCode, EntryData, HeadData,
    ObjectRole, OutputData, RetiredData, SelectedHead, SettledData, StageShape, WalData, WalKind,
    CHUNK_PLAIN_CAP, ENTRY_PLAIN_CAP, HEAD_FILE_CAP, PAGE_FRAME_OVERHEAD, WAL_FILE_CAP,
};
use super::fs::{
    bounded_entries, create_namespace, observe, observe_directory, read_bounded, remove_listed,
    rename_fixed, HeldDirectory, Presence,
};
use super::transaction::{commit, retire, CleanupProof, PreparedTransition, Resume};
use super::trie::{
    insert_entry_rows, insert_settled_rows, lookup, NodeStore, PlanError, RowKey, RowValue,
    TokenRowData, TrieLimits, TriePlan, VerifySink, NODE_PLAIN_CAP, PRODUCTION_LIMITS,
};
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::AppError;
use catcoms_replication::chat::cold_actions::{
    assemble_action, ActionChunk, ActionPagesExpectation, AssembledActionOriginal,
};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::SignedAction;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::{open_private_chat_page, PrivateChatPageId};
use catcoms_storage::VaultPhysicalWorkLease;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

/// Logical invoices, each the caller's prepaid debit BEFORE the operation allocates. They are
/// derived from this module's hard caps (WAL <= 1 MiB, page <= 64 KiB, action <= 128 KiB,
/// at most 17 node reads per lookup), counted with generous overlap; they are not measured
/// allocator/RSS figures. Each owning value keeps its payment until it is destroyed.
pub(in crate::store) const MEMBER_OUTBOX_OWNER_FEE: usize = 16 * 1024;
/// Open/recovery: WAL file + decoded copy + plaintexts + replanned path nodes + head.
pub(in crate::store) const MEMBER_OUTBOX_RECOVERY_FEE: usize = 6 * 1024 * 1024;
/// Append preparation: chunk/entry/node frames, the WAL plaintext and file, path nodes.
pub(in crate::store) const MEMBER_OUTBOX_APPEND_FEE: usize = 6 * 1024 * 1024;
/// One token lookup: path nodes, entry, chunks and the assembled original (raw + typed).
pub(in crate::store) const MEMBER_OUTBOX_LOOKUP_FEE: usize = 2 * 1024 * 1024;
/// One debt batch of at most `MAX_SETTLED` assembled originals.
pub(in crate::store) const MEMBER_OUTBOX_DEBT_FEE: usize = 5 * 1024 * 1024;
/// One checkpoint: disposition rows, node frames and its WAL.
pub(in crate::store) const MEMBER_OUTBOX_CHECKPOINT_FEE: usize = 4 * 1024 * 1024;

const HEAD: &str = "head.bin";
const HEAD_STAGE: &str = "head.stage.v1";
const WAL: &str = "outbox.wal";
const WAL_STAGE: &str = "outbox.wal.stage.v1";

/// Persistent-usage policy. Exceeding any bound refuses BEFORE admission with
/// [`MemberOutboxRefusal::Pressure`]; nothing is ever evicted. Lowering a bound below current
/// usage only refuses new work: receipts, audit and existing debt are untouched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) struct MemberOutboxQuota {
    /// Unsent publication debt (`next_ordinal - front`), Held items included until settled.
    pub(in crate::store) max_pending: u64,
    /// Immutable object bytes, plus a transaction's WAL and head stage while it runs.
    pub(in crate::store) max_live_bytes: u64,
    /// Immutable objects, plus the WAL and head stage while a transaction runs.
    pub(in crate::store) max_live_objects: u64,
}
pub(in crate::store) const MEMBER_OUTBOX_DEFAULT_QUOTA: MemberOutboxQuota = MemberOutboxQuota {
    max_pending: 256,
    max_live_bytes: 256 * 1024 * 1024,
    max_live_objects: 1 << 20,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) enum MemberOutboxPressure {
    Pending,
    Bytes,
    Objects,
}

/// Typed refusals. None of them means a write happened; none means "absent" except `Absent`.
#[derive(Debug)]
pub(in crate::store) enum MemberOutboxRefusal {
    /// The namespace does not exist and creation was not requested.
    Absent,
    /// Another owner for this context is alive in this registry.
    Busy,
    /// A persistent quota would be exceeded. Nothing was evicted.
    Pressure(MemberOutboxPressure),
    /// The trie's depth/collision/output bounds would be exceeded. No row was dropped.
    IndexExhausted,
    /// The token already holds this exact request. Look it up instead of saving again.
    AlreadySaved(Box<MemberReceiptData>),
    /// The token already holds a DIFFERENT request.
    TokenConflict(Box<MemberReceiptData>),
    /// The same semantic ActionId is already stored (whatever its original bytes).
    DuplicateAction,
    /// The prepared work, batch or admission does not match this owner's current state.
    Stale,
    /// A previous operation's outcome is uncertain; run recovery first.
    NeedsRecovery,
    /// Stored state is incomplete, unknown, foreign or inconsistent. It was preserved.
    Incomplete,
    /// The supplied payment is not from this owner's pool or is too small.
    Payment,
    /// The signed original does not belong to this context (group or device).
    Foreign,
    /// The debt item cannot be marked submitted (it is Held, or out of range).
    NotPublishable,
}

#[derive(Debug)]
pub(in crate::store) enum MemberOutboxError {
    Refused(MemberOutboxRefusal),
    /// IO or other failure. Never a statement of absence.
    Failed(Box<AppError>),
}
impl From<AppError> for MemberOutboxError {
    fn from(error: AppError) -> Self {
        Self::Failed(Box::new(error))
    }
}
impl From<MemberOutboxRefusal> for MemberOutboxError {
    fn from(refusal: MemberOutboxRefusal) -> Self {
        Self::Refused(refusal)
    }
}
impl MemberOutboxError {
    pub(in crate::store) fn refusal(&self) -> Option<&MemberOutboxRefusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::Failed(_) => None,
        }
    }
}

/// Map a stored-state failure: IO stays a failure (retryable, says nothing), every
/// authentication/decode/consistency failure becomes a preserved `Incomplete` refusal.
pub(super) fn preserved(error: AppError) -> MemberOutboxError {
    match error {
        AppError::Io(_) => MemberOutboxError::from(error),
        _ => MemberOutboxError::Refused(MemberOutboxRefusal::Incomplete),
    }
}

pub(super) fn plan_refusal(error: PlanError) -> MemberOutboxError {
    match error {
        PlanError::Duplicate => MemberOutboxRefusal::DuplicateAction.into(),
        PlanError::Exhausted => MemberOutboxRefusal::IndexExhausted.into(),
        PlanError::Failed(error) => MemberOutboxError::Failed(error),
    }
}

fn plan_preserved(error: PlanError) -> MemberOutboxError {
    match error {
        PlanError::Failed(error) => preserved(*error),
        _ => MemberOutboxRefusal::Incomplete.into(),
    }
}

/// Validate a prepaid debit against the owner's pool BEFORE any allocation it covers.
pub(super) fn require_payment(
    payment: &CatalogReadReservation,
    pool: &CriticalCatalogReadPool,
    fee: usize,
) -> Result<(), MemberOutboxError> {
    payment
        .validate(pool, fee)
        .map_err(|_| MemberOutboxRefusal::Payment.into())
}

/// Per-registry exclusive context claims. The primary embeds ONE registry beside its Store's
/// session guard; clones share claims. A claim outlives its Store if work still holds it.
#[derive(Clone, Default)]
pub(in crate::store) struct MemberOutboxRegistry {
    claimed: Arc<Mutex<BTreeSet<[u8; 32]>>>,
}
impl std::fmt::Debug for MemberOutboxRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberOutboxRegistry")
            .finish_non_exhaustive()
    }
}
impl MemberOutboxRegistry {
    pub(in crate::store) fn new() -> Self {
        Self::default()
    }
    fn claim(
        &self,
        context: MemberOutboxContextId,
    ) -> Result<MemberOutboxClaim, MemberOutboxError> {
        let accepted = match self.claimed.lock() {
            Ok(mut claimed) => claimed.insert(*context.as_bytes()),
            // A poisoned registry cannot prove exclusivity; refuse rather than guess.
            Err(_) => false,
        };
        if !accepted {
            return Err(MemberOutboxRefusal::Busy.into());
        }
        Ok(MemberOutboxClaim {
            claimed: self.claimed.clone(),
            context: *context.as_bytes(),
        })
    }
    /// Whether a context is currently claimed. Diagnostic DATA only, never a permission.
    pub(in crate::store) fn is_claimed(&self, context: MemberOutboxContextId) -> bool {
        match self.claimed.lock() {
            Ok(claimed) => claimed.contains(context.as_bytes()),
            Err(_) => true,
        }
    }
}

/// Exclusive context claim. Released only on drop of its owner (or worker holding the owner).
pub(super) struct MemberOutboxClaim {
    claimed: Arc<Mutex<BTreeSet<[u8; 32]>>>,
    context: [u8; 32],
}
impl Drop for MemberOutboxClaim {
    fn drop(&mut self) {
        // Release even through poison: the owner is gone, so the claim must not leak forever.
        let mut claimed = match self.claimed.lock() {
            Ok(claimed) => claimed,
            Err(poisoned) => poisoned.into_inner(),
        };
        claimed.remove(&self.context);
    }
}

/// The separately held, validated parent directory that contains member namespaces.
///
/// Production construction is the primary's (critical-namespace provisioning); it must not be
/// a closed founder/group inventory, and a caller-provided path alone is not provenance. Only
/// the cfg(test) temporary-vault fixture exists here.
pub(in crate::store) struct MemberOutboxParentDirectory {
    held: HeldDirectory,
}
impl std::fmt::Debug for MemberOutboxParentDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberOutboxParentDirectory")
            .finish_non_exhaustive()
    }
}
impl MemberOutboxParentDirectory {
    #[cfg(test)]
    pub(in crate::store) fn temporary_fixture_for_test(
        path: &std::path::Path,
    ) -> Result<Self, AppError> {
        Ok(Self {
            held: HeldDirectory::open(path.to_path_buf())?,
        })
    }
}

/// One namespace's closed file layout. No caller-provided or scanned name is ever used.
pub(super) struct Namespace {
    pub(super) dir: HeldDirectory,
}
impl Namespace {
    pub(super) fn head(&self) -> PathBuf {
        self.dir.child(HEAD)
    }
    pub(super) fn head_stage(&self) -> PathBuf {
        self.dir.child(HEAD_STAGE)
    }
    pub(super) fn wal(&self) -> PathBuf {
        self.dir.child(WAL)
    }
    pub(super) fn wal_stage(&self) -> PathBuf {
        self.dir.child(WAL_STAGE)
    }
    pub(super) fn object(&self, role: ObjectRole, id: PrivateChatPageId) -> PathBuf {
        let mut name = String::with_capacity(70);
        name.push_str(role.prefix());
        super::data::push_hex(&mut name, id.as_bytes());
        name.push_str(".bin");
        self.dir.child(&name)
    }
}

/// Node reads straight from the namespace. Every page is opened against its exact ID.
pub(super) struct DiskNodes<'a> {
    pub(super) ns: &'a Namespace,
    pub(super) key: &'a [u8; 32],
}
impl NodeStore for DiskNodes<'_> {
    fn read_node(
        &mut self,
        id: PrivateChatPageId,
    ) -> Result<(Zeroizing<Vec<u8>>, usize), AppError> {
        let frame = read_bounded(
            &self.ns.object(ObjectRole::Node, id),
            NODE_PLAIN_CAP + PAGE_FRAME_OVERHEAD,
            false,
        )?;
        let plain = open_private_chat_page(self.key, id, &frame)?;
        Ok((plain, frame.len()))
    }
}

pub(super) fn read_entry(
    ns: &Namespace,
    key: &[u8; 32],
    context: MemberOutboxContextId,
    id: PrivateChatPageId,
) -> Result<EntryData, AppError> {
    let frame = read_bounded(
        &ns.object(ObjectRole::Entry, id),
        ENTRY_PLAIN_CAP + PAGE_FRAME_OVERHEAD,
        false,
    )?;
    EntryData::decode(context, &open_private_chat_page(key, id, &frame)?)
}

/// Assemble and fully authenticate an entry's original from its opened chunk plaintexts:
/// chunk framing/context/order, original digest, signature, group, device, length and the
/// entry's request digest recomputed from the signed spec itself.
pub(super) fn assemble_entry(
    context: &MemberOutboxContextData,
    entry: &EntryData,
    plains: &[Zeroizing<Vec<u8>>],
) -> Result<AssembledActionOriginal, AppError> {
    if plains.len() != entry.chunks.len() {
        return Err(refused());
    }
    let mut chunks = Vec::with_capacity(plains.len());
    for (index, plain) in plains.iter().enumerate() {
        let inner = decode_chunk_page(context.id, entry.ordinal, index, plain)?;
        chunks.push(ActionChunk::decode(inner).map_err(|_| refused())?);
    }
    let assembled = assemble_action(
        &ActionPagesExpectation {
            vault: &context.vault,
            group: &context.group,
            profile: context.profile,
            action: entry.action,
            original: entry.original,
        },
        &chunks,
    )
    .map_err(|_| refused())?;
    let action: &SignedAction = assembled.action();
    if action.signer() != &context.device
        || assembled.original_bytes().len() != entry.original_len as usize
        || signed_request_digest(context.id, entry.token, action.spec())? != entry.request
    {
        return Err(refused());
    }
    Ok(assembled)
}

fn read_original(
    ns: &Namespace,
    key: &[u8; 32],
    context: &MemberOutboxContextData,
    entry: &EntryData,
) -> Result<AssembledActionOriginal, AppError> {
    let mut plains = Vec::with_capacity(entry.chunks.len());
    for id in &entry.chunks {
        let frame = read_bounded(
            &ns.object(ObjectRole::Chunk, *id),
            CHUNK_PLAIN_CAP + PAGE_FRAME_OVERHEAD,
            false,
        )?;
        plains.push(open_private_chat_page(key, *id, &frame)?);
    }
    assemble_entry(context, entry, &plains)
}

/// The next head after a transition: exact persistent counters, never estimates.
pub(super) fn successor(
    parent: &SelectedHead,
    root: Option<PrivateChatPageId>,
    outputs: &[OutputData],
    retired: &[RetiredData],
    next_ordinal: u64,
    front: u64,
) -> Result<HeadData, AppError> {
    let added: u64 = outputs.iter().map(|o| o.frame.len() as u64).sum();
    let removed: u64 = retired.iter().map(|r| u64::from(r.bytes)).sum();
    Ok(HeadData {
        generation: parent.data.generation.checked_add(1).ok_or_else(refused)?,
        parent: *parent.id.as_bytes(),
        root,
        next_ordinal,
        front,
        live_bytes: parent
            .data
            .live_bytes
            .checked_add(added)
            .and_then(|n| n.checked_sub(removed))
            .ok_or_else(refused)?,
        live_objects: parent
            .data
            .live_objects
            .checked_add(outputs.len() as u64)
            .and_then(|n| n.checked_sub(retired.len() as u64))
            .ok_or_else(refused)?,
    })
}

pub(super) enum OwnerState {
    Clean(SelectedHead),
    /// The last operation's outcome is uncertain or its cleanup is unfinished. Every read and
    /// transaction refuses until [`MemberOutbox::recover`] re-derives state from the files.
    NeedsRecovery,
}

/// Inputs to open (or create) one context's namespace.
pub(in crate::store) struct MemberOutboxOpen<'a> {
    pub(in crate::store) registry: &'a MemberOutboxRegistry,
    pub(in crate::store) parent: &'a MemberOutboxParentDirectory,
    pub(in crate::store) context: MemberOutboxContextData,
    pub(in crate::store) quota: MemberOutboxQuota,
    /// Create the namespace when it does not exist. Without it, absence is `Absent`.
    pub(in crate::store) create: bool,
    pub(in crate::store) pool: &'a CriticalCatalogReadPool,
    /// Retained by the owner for its whole life ([`MEMBER_OUTBOX_OWNER_FEE`]).
    pub(in crate::store) metadata: CatalogReadReservation,
    /// Covers open/recovery only ([`MEMBER_OUTBOX_RECOVERY_FEE`]); released after.
    pub(in crate::store) work: CatalogReadReservation,
    /// Held through every open/recovery write and barrier; released after.
    pub(in crate::store) lease: VaultPhysicalWorkLease,
}

/// The exclusive owner of one member namespace. See the module docs for exclusion.
pub(in crate::store) struct MemberOutbox {
    pub(super) ns: Namespace,
    pub(super) context: MemberOutboxContextData,
    pub(super) quota: MemberOutboxQuota,
    pub(super) limits: TrieLimits,
    pub(super) pool: CriticalCatalogReadPool,
    pub(super) state: OwnerState,
    // Drop order is declaration order: the metadata payment, then (LAST) the claim.
    _metadata: CatalogReadReservation,
    _claim: MemberOutboxClaim,
}
impl std::fmt::Debug for MemberOutbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberOutbox").finish_non_exhaustive()
    }
}

/// What a token lookup found.
#[derive(Debug)]
pub(in crate::store) enum MemberLookup {
    /// No entry holds this token. Only returned after a successful authenticated read.
    Absent,
    /// The token holds a DIFFERENT request; its receipt says which.
    Conflict(Box<MemberReceiptData>),
    /// The exact original request: its receipt and the authenticated original bytes.
    Hit(Box<MemberSavedOriginal>),
}

/// Local publication status of one entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) enum MemberEntryStatus {
    /// Still in publication debt (or not yet settled by a checkpoint).
    Pending,
    /// A checkpoint recorded this disposition. Audit DATA; submission is not peer receipt.
    Settled(DispositionCode),
}

/// One recovered exact original. Keeps its read payment until dropped.
pub(in crate::store) struct MemberSavedOriginal {
    receipt: MemberReceiptData,
    status: MemberEntryStatus,
    original: AssembledActionOriginal,
    _payment: CatalogReadReservation,
}
impl std::fmt::Debug for MemberSavedOriginal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberSavedOriginal")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
impl MemberSavedOriginal {
    pub(in crate::store) fn receipt(&self) -> &MemberReceiptData {
        &self.receipt
    }
    pub(in crate::store) fn status(&self) -> MemberEntryStatus {
        self.status
    }
    pub(in crate::store) fn action(&self) -> &SignedAction {
        self.original.action()
    }
    pub(in crate::store) fn original_bytes(&self) -> &[u8] {
        self.original.original_bytes()
    }
}

impl MemberOutbox {
    /// Open (or create) the namespace, completing or preserving any interrupted work.
    pub(in crate::store) fn open(
        request: MemberOutboxOpen<'_>,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, MemberOutboxError> {
        Self::open_with_limits(request, key, rng, PRODUCTION_LIMITS)
    }

    /// Test-only smaller trie limits, to force splits and exhaustion with few entries.
    #[cfg(test)]
    pub(super) fn open_with_limits_for_test(
        request: MemberOutboxOpen<'_>,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
        limits: TrieLimits,
    ) -> Result<Self, MemberOutboxError> {
        Self::open_with_limits(request, key, rng, limits)
    }

    fn open_with_limits(
        request: MemberOutboxOpen<'_>,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
        limits: TrieLimits,
    ) -> Result<Self, MemberOutboxError> {
        let MemberOutboxOpen {
            registry,
            parent,
            context,
            quota,
            create,
            pool,
            metadata,
            work,
            lease,
        } = request;
        require_payment(&metadata, pool, MEMBER_OUTBOX_OWNER_FEE)?;
        require_payment(&work, pool, MEMBER_OUTBOX_RECOVERY_FEE)?;
        let claim = registry.claim(context.id())?;
        parent.held.require()?;
        let path = parent.held.child(&context.namespace_leaf());
        let created = match observe_directory(&path)? {
            Presence::Present => false,
            Presence::Absent if create => {
                create_namespace(&path)?;
                parent.held.sync()?;
                true
            }
            Presence::Absent => return Err(MemberOutboxRefusal::Absent.into()),
        };
        let ns = Namespace {
            dir: HeldDirectory::open(path)?,
        };
        let head = if created {
            initialize(&ns, &context, key, rng)?
        } else {
            recover(&ns, &context, key, limits, rng)?
        };
        let owner = Self {
            ns,
            context,
            quota,
            limits,
            pool: pool.clone(),
            state: OwnerState::Clean(head),
            _metadata: metadata,
            _claim: claim,
        };
        // Every File of open/recovery has closed; only now release the work payment and lease.
        drop(work);
        drop(lease);
        Ok(owner)
    }

    pub(in crate::store) fn context_id(&self) -> MemberOutboxContextId {
        self.context.id
    }
    pub(in crate::store) fn needs_recovery(&self) -> bool {
        matches!(self.state, OwnerState::NeedsRecovery)
    }
    pub(super) fn clean(&self) -> Result<&SelectedHead, MemberOutboxError> {
        match &self.state {
            OwnerState::Clean(head) => Ok(head),
            OwnerState::NeedsRecovery => Err(MemberOutboxRefusal::NeedsRecovery.into()),
        }
    }
    /// The selected head's identity. DATA for binding prepared work and diagnostics.
    pub(in crate::store) fn selected_head(&self) -> Result<MemberHeadId, MemberOutboxError> {
        Ok(self.clean()?.id)
    }
    /// Unsent publication debt count.
    pub(in crate::store) fn pending(&self) -> Result<u64, MemberOutboxError> {
        Ok(self.clean()?.data.pending())
    }

    /// Re-derive state from the files after an uncertain operation. Not a fresh signing or
    /// publication admission: it can only finish or preserve work that was already admitted.
    pub(in crate::store) fn recover(
        &mut self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
        lease: VaultPhysicalWorkLease,
        work: CatalogReadReservation,
    ) -> Result<(), MemberOutboxError> {
        require_payment(&work, &self.pool, MEMBER_OUTBOX_RECOVERY_FEE)?;
        self.state = OwnerState::NeedsRecovery;
        let head = recover(&self.ns, &self.context, key, self.limits, rng)?;
        self.state = OwnerState::Clean(head);
        drop(work);
        drop(lease);
        Ok(())
    }

    /// Exact request lookup BEFORE any new authoring. An IO or decode error is an error,
    /// never `Absent`. A historical receipt stays readable after removal; reading it grants
    /// no OPEN, signing or publication right.
    pub(in crate::store) fn lookup(
        &self,
        key: &[u8; 32],
        request: &MemberRequestData<'_>,
        payment: CatalogReadReservation,
    ) -> Result<MemberLookup, MemberOutboxError> {
        require_payment(&payment, &self.pool, MEMBER_OUTBOX_LOOKUP_FEE)?;
        let head = self.clean()?;
        let digest = request.digest(self.context.id)?;
        let Some((row, entry)) = self.token_row(key, head, request.token)? else {
            return Ok(MemberLookup::Absent);
        };
        let receipt = receipt_of(&self.context, request.token, &row, &entry);
        if row.request != digest {
            return Ok(MemberLookup::Conflict(Box::new(receipt)));
        }
        let original = read_original(&self.ns, key, &self.context, &entry)?;
        let status = self.status_of(key, head, entry.ordinal)?;
        Ok(MemberLookup::Hit(Box::new(MemberSavedOriginal {
            receipt,
            status,
            original,
            _payment: payment,
        })))
    }

    /// The token's row and its entry, cross-checked against each other.
    pub(super) fn token_row(
        &self,
        key: &[u8; 32],
        head: &SelectedHead,
        token: MemberRequestToken,
    ) -> Result<Option<(TokenRowData, EntryData)>, AppError> {
        let mut nodes = DiskNodes { ns: &self.ns, key };
        let row = match lookup(
            self.context.id,
            head.data.root,
            RowKey::token(token),
            &mut nodes,
        )? {
            None => return Ok(None),
            Some(RowValue::Token(row)) => row,
            Some(_) => return Err(refused()),
        };
        let entry = read_entry(&self.ns, key, self.context.id, row.entry)?;
        if entry.token != token
            || entry.ordinal != row.ordinal
            || entry.request != row.request
            || entry.action != row.action
            || entry.original != row.original
            || entry.ordinal >= head.data.next_ordinal
        {
            return Err(refused());
        }
        Ok(Some((row, entry)))
    }

    fn status_of(
        &self,
        key: &[u8; 32],
        head: &SelectedHead,
        ordinal: u64,
    ) -> Result<MemberEntryStatus, AppError> {
        let mut nodes = DiskNodes { ns: &self.ns, key };
        match lookup(
            self.context.id,
            head.data.root,
            RowKey::disposition(ordinal),
            &mut nodes,
        )? {
            Some(RowValue::Disposition(row)) if ordinal < head.data.front => {
                Ok(MemberEntryStatus::Settled(row.code))
            }
            None if ordinal >= head.data.front => Ok(MemberEntryStatus::Pending),
            // Behind the front without a disposition, or a disposition ahead of it: corrupt.
            _ => Err(refused()),
        }
    }

    /// Read the entry an ordinal row names, checking the row/entry pair.
    pub(super) fn debt_entry(
        &self,
        key: &[u8; 32],
        head: &SelectedHead,
        ordinal: u64,
    ) -> Result<(PrivateChatPageId, EntryData), AppError> {
        let mut nodes = DiskNodes { ns: &self.ns, key };
        let row = match lookup(
            self.context.id,
            head.data.root,
            RowKey::ordinal(ordinal),
            &mut nodes,
        )? {
            Some(RowValue::Ordinal(row)) => row,
            _ => return Err(refused()),
        };
        let entry = read_entry(&self.ns, key, self.context.id, row.entry)?;
        if entry.ordinal != ordinal || entry.action != row.action {
            return Err(refused());
        }
        Ok((row.entry, entry))
    }

    /// Read the next bounded page of publication debt in ordinal order, classifying each item
    /// against the Admission the caller's real publication guard currently holds. The batch
    /// carries the exact original bytes; resealing them under the current envelope is the
    /// caller's. Neither this read nor the class is a publication permission.
    pub(in crate::store) fn debt_batch(
        &self,
        key: &[u8; 32],
        current_admission: OwnerFactId,
        maximum: usize,
        payment: CatalogReadReservation,
    ) -> Result<MemberDebtBatch, MemberOutboxError> {
        require_payment(&payment, &self.pool, MEMBER_OUTBOX_DEBT_FEE)?;
        let head = self.clean()?;
        let count = head
            .data
            .pending()
            .min(maximum.min(super::frames::MAX_SETTLED) as u64);
        let mut items = Vec::with_capacity(count as usize);
        for ordinal in head.data.front..head.data.front + count {
            let (entry_id, entry) = self.debt_entry(key, head, ordinal)?;
            let original = read_original(&self.ns, key, &self.context, &entry)?;
            items.push(MemberDebtItem {
                ordinal,
                entry: entry_id,
                admission: entry.admission,
                class: if entry.admission == current_admission {
                    MemberDebtClass::Publishable
                } else {
                    MemberDebtClass::Held
                },
                original,
                submitted: false,
            });
        }
        Ok(MemberDebtBatch {
            context: self.context.id,
            front: head.data.front,
            current_admission,
            items,
            _payment: payment,
        })
    }
}

pub(super) fn receipt_of(
    context: &MemberOutboxContextData,
    token: MemberRequestToken,
    row: &TokenRowData,
    entry: &EntryData,
) -> MemberReceiptData {
    MemberReceiptData {
        context: context.id,
        token,
        ordinal: row.ordinal,
        entry: row.entry,
        request: row.request,
        action: row.action,
        original: row.original,
        admission: entry.admission,
    }
}

/// Publication class of one debt item for the presented current Admission.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) enum MemberDebtClass {
    /// Signed under the presented current Admission: may be submitted, then marked.
    Publishable,
    /// Signed under a DIFFERENT (earlier) Admission. Never submitted by this path; a
    /// checkpoint settles it as Held and its original stays in the audit.
    Held,
}

/// One unsent original in a debt batch.
pub(in crate::store) struct MemberDebtItem {
    ordinal: u64,
    entry: PrivateChatPageId,
    admission: OwnerFactId,
    class: MemberDebtClass,
    original: AssembledActionOriginal,
    submitted: bool,
}
impl std::fmt::Debug for MemberDebtItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberDebtItem")
            .field("ordinal", &self.ordinal)
            .field("class", &self.class)
            .finish_non_exhaustive()
    }
}
impl MemberDebtItem {
    pub(in crate::store) fn ordinal(&self) -> u64 {
        self.ordinal
    }
    pub(in crate::store) fn class(&self) -> MemberDebtClass {
        self.class
    }
    pub(in crate::store) fn admission(&self) -> OwnerFactId {
        self.admission
    }
    /// The exact original envelope bytes, identical across every retry and transport epoch.
    pub(in crate::store) fn original_bytes(&self) -> &[u8] {
        self.original.original_bytes()
    }
    pub(in crate::store) fn action(&self) -> &SignedAction {
        self.original.action()
    }
}

/// A bounded, owner-bound page of publication debt. Not Clone; keeps its payment.
pub(in crate::store) struct MemberDebtBatch {
    pub(super) context: MemberOutboxContextId,
    pub(super) front: u64,
    pub(super) current_admission: OwnerFactId,
    pub(super) items: Vec<MemberDebtItem>,
    _payment: CatalogReadReservation,
}
impl std::fmt::Debug for MemberDebtBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberDebtBatch")
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}
impl MemberDebtBatch {
    pub(in crate::store) fn items(&self) -> &[MemberDebtItem] {
        &self.items
    }
    /// Record that transport accepted this item's exact original for local submission. Only
    /// Publishable items can be marked; marks may arrive in any order, but a checkpoint
    /// settles only the contiguous prefix, so a failed earlier item is never skipped.
    pub(in crate::store) fn record_submitted(
        &mut self,
        index: usize,
    ) -> Result<(), MemberOutboxError> {
        match self.items.get_mut(index) {
            Some(item) if item.class == MemberDebtClass::Publishable => {
                item.submitted = true;
                Ok(())
            }
            _ => Err(MemberOutboxRefusal::NotPublishable.into()),
        }
    }
    /// The contiguous settled prefix: Held items, or Publishable items marked submitted.
    pub(super) fn settled_prefix(&self) -> Vec<SettledData> {
        self.items
            .iter()
            .take_while(|item| item.class == MemberDebtClass::Held || item.submitted)
            .map(|item| SettledData {
                ordinal: item.ordinal,
                entry: item.entry,
                disposition: match item.class {
                    MemberDebtClass::Held => DispositionCode::Held,
                    MemberDebtClass::Publishable => DispositionCode::Submitted,
                },
                admission: self.current_admission,
            })
            .collect()
    }
}

/// Write the generation-0 head of a freshly created (empty) namespace: stage, then rename.
fn initialize(
    ns: &Namespace,
    context: &MemberOutboxContextData,
    key: &[u8; 32],
    rng: &mut dyn CryptoRngCore,
) -> Result<SelectedHead, MemberOutboxError> {
    let file = HeadData::initial().seal_file(key, context.id, rng)?;
    super::fs::ensure_exact(&ns.head_stage(), &file)?;
    ns.dir.sync()?;
    super::transaction::reach(super::transaction::Cut::InitialStageWritten)?;
    rename_fixed(&ns.head_stage(), &ns.head())?;
    ns.dir.sync()?;
    Ok(SelectedHead::open(key, context.id, file)?)
}

/// `head.bin` is missing in an existing namespace. Complete ONLY a provably interrupted
/// creation: the directory holds nothing, or nothing but the generation-0 creation stage.
/// Anything else (objects, WAL, other names) means a populated namespace lost its selector;
/// it is preserved and refused, never silently treated as a fresh empty outbox.
fn interrupted_creation(
    ns: &Namespace,
    context: &MemberOutboxContextData,
    key: &[u8; 32],
    rng: &mut dyn CryptoRngCore,
) -> Result<SelectedHead, MemberOutboxError> {
    let entries = bounded_entries(&ns.dir, 2)?;
    if entries.is_empty() {
        return initialize(ns, context, key, rng);
    }
    if entries.len() != 1 || entries[0] != HEAD_STAGE {
        return Err(MemberOutboxRefusal::Incomplete.into());
    }
    let raw = read_bounded(&ns.head_stage(), HEAD_FILE_CAP, true).map_err(preserved)?;
    match classify_stage(ControlKind::Head, context.id, &[0; 32], &raw) {
        StageShape::Complete => {
            let head = HeadData::open_file(key, context.id, &raw).map_err(preserved)?;
            if head != HeadData::initial() {
                return Err(MemberOutboxRefusal::Incomplete.into());
            }
            rename_fixed(&ns.head_stage(), &ns.head())?;
            ns.dir.sync()?;
            Ok(SelectedHead::open(key, context.id, raw)?)
        }
        StageShape::PreambleTorn | StageShape::BodyTorn => {
            // The only file in the namespace is a torn creation stage of THIS context's
            // generation-0 preamble: nothing was ever selected, so nothing can be lost.
            remove_listed(&ns.head_stage())?;
            ns.dir.sync()?;
            initialize(ns, context, key, rng)
        }
        StageShape::Unknown => Err(MemberOutboxRefusal::Incomplete.into()),
    }
}

/// Cold (or warm-uncertain) recovery. Classifies the fixed control files against the
/// authenticated selected head and either finishes already-admitted work, removes a
/// demonstrably owned torn pre-commit stage, or preserves and refuses everything else.
pub(super) fn recover(
    ns: &Namespace,
    context: &MemberOutboxContextData,
    key: &[u8; 32],
    limits: TrieLimits,
    rng: &mut dyn CryptoRngCore,
) -> Result<SelectedHead, MemberOutboxError> {
    if observe(&ns.head())? == Presence::Absent {
        return interrupted_creation(ns, context, key, rng);
    }
    let current = SelectedHead::open(
        key,
        context.id,
        read_bounded(&ns.head(), HEAD_FILE_CAP, false)?,
    )
    .map_err(preserved)?;
    let head_stage = observe(&ns.head_stage())?;
    let wal_stage = observe(&ns.wal_stage())?;
    let mut wal = observe(&ns.wal())?;
    if wal == Presence::Absent {
        if head_stage == Presence::Present {
            // A head stage is only ever written after WAL MAIN. Without one it is not ours.
            return Err(MemberOutboxRefusal::Incomplete.into());
        }
        if wal_stage == Presence::Absent {
            return Ok(current);
        }
        let raw = read_bounded(&ns.wal_stage(), WAL_FILE_CAP, true).map_err(preserved)?;
        match classify_stage(ControlKind::Wal, context.id, current.id.as_bytes(), &raw) {
            StageShape::PreambleTorn | StageShape::BodyTorn => {
                // Demonstrably owned pre-commit scratch: the namespace's authenticated head is
                // this context's, no WAL MAIN or head stage exists, and the stage is a strict
                // prefix of the exact preamble bound to the CURRENT selected head. It was never
                // MAIN, so no save it carried was ever reported. Remove only this fixed name.
                remove_listed(&ns.wal_stage())?;
                ns.dir.sync()?;
                super::transaction::reach(super::transaction::Cut::ScratchRemoved)?;
                return Ok(current);
            }
            StageShape::Complete => {
                // Promote only a complete stage that authenticates and fully parses.
                WalData::open_file(key, context.id, &raw).map_err(preserved)?;
                rename_fixed(&ns.wal_stage(), &ns.wal())?;
                ns.dir.sync()?;
                super::transaction::reach(super::transaction::Cut::StagePromoted)?;
                wal = Presence::Present;
            }
            StageShape::Unknown => return Err(MemberOutboxRefusal::Incomplete.into()),
        }
    } else if wal_stage == Presence::Present {
        return Err(MemberOutboxRefusal::Incomplete.into());
    }
    debug_assert_eq!(wal, Presence::Present);
    let wal_file = read_bounded(&ns.wal(), WAL_FILE_CAP, false)?;
    let parsed = WalData::open_file(key, context.id, &wal_file).map_err(preserved)?;
    let candidate =
        SelectedHead::open(key, context.id, parsed.candidate.clone()).map_err(preserved)?;
    if parsed.parent == current.id {
        // Roll forward an already-admitted transition after re-deriving all of it.
        let transition = verify_transition(
            ns, context, key, limits, &current, parsed, candidate, wal_file,
        )?;
        commit(ns, &current, &transition, Resume::FromMain).map_err(preserved)?;
        retire(
            ns,
            context.id,
            &transition.candidate,
            &transition.retired,
            CleanupProof::Replanned,
        )
        .map_err(preserved)?;
        Ok(transition.candidate)
    } else if candidate.id == current.id {
        // Already selected: finish only the cleanup, proving each retiree unreachable.
        if observe(&ns.head_stage())? == Presence::Present {
            return Err(MemberOutboxRefusal::Incomplete.into());
        }
        retire(
            ns,
            context.id,
            &current,
            &parsed.retired,
            CleanupProof::WalkNewRoot(key),
        )
        .map_err(preserved)?;
        Ok(current)
    } else {
        Err(MemberOutboxRefusal::Incomplete.into())
    }
}

/// Re-derive an authenticated WAL's complete transition from the selected parent: original
/// signature, context, request digest, index rows, every node plaintext, retirees and the
/// exact candidate counters. Labels in the WAL are never trusted on their own.
#[allow(clippy::too_many_arguments)]
fn verify_transition(
    ns: &Namespace,
    context: &MemberOutboxContextData,
    key: &[u8; 32],
    limits: TrieLimits,
    parent: &SelectedHead,
    wal: WalData,
    candidate: SelectedHead,
    wal_file: Zeroizing<Vec<u8>>,
) -> Result<PreparedTransition, MemberOutboxError> {
    let mut nodes = DiskNodes { ns, key };
    let (node_outputs, next_ordinal, front, rows): (&[OutputData], u64, u64, RowsToVerify) =
        match wal.kind {
            WalKind::Append => {
                if !wal.settled.is_empty() {
                    return Err(MemberOutboxRefusal::Incomplete.into());
                }
                let entry_at = wal
                    .outputs
                    .iter()
                    .position(|o| o.role == ObjectRole::Entry)
                    .ok_or(MemberOutboxRefusal::Incomplete)?;
                let (chunks, rest) = wal.outputs.split_at(entry_at);
                let (entry_output, node_outputs) =
                    rest.split_first().ok_or(MemberOutboxRefusal::Incomplete)?;
                if chunks.iter().any(|o| o.role != ObjectRole::Chunk) {
                    return Err(MemberOutboxRefusal::Incomplete.into());
                }
                let entry = EntryData::decode(
                    context.id,
                    &open_private_chat_page(key, entry_output.id, &entry_output.frame)
                        .map_err(|e| preserved(e.into()))?,
                )
                .map_err(preserved)?;
                if entry.ordinal != parent.data.next_ordinal
                    || entry.chunks.len() != chunks.len()
                    || entry.chunks.iter().zip(chunks).any(|(id, o)| *id != o.id)
                {
                    return Err(MemberOutboxRefusal::Incomplete.into());
                }
                let mut plains = Vec::with_capacity(chunks.len());
                for chunk in chunks {
                    plains.push(
                        open_private_chat_page(key, chunk.id, &chunk.frame)
                            .map_err(|e| preserved(e.into()))?,
                    );
                }
                assemble_entry(context, &entry, &plains).map_err(preserved)?;
                (
                    node_outputs,
                    parent.data.next_ordinal + 1,
                    parent.data.front,
                    RowsToVerify::Entry(Box::new((entry, entry_output.id))),
                )
            }
            WalKind::Checkpoint => {
                if wal.settled.is_empty() || wal.outputs.iter().any(|o| o.role != ObjectRole::Node)
                {
                    return Err(MemberOutboxRefusal::Incomplete.into());
                }
                verify_settled(ns, context, key, parent, &wal.settled)?;
                (
                    &wal.outputs[..],
                    parent.data.next_ordinal,
                    parent.data.front + wal.settled.len() as u64,
                    RowsToVerify::Settled,
                )
            }
        };
    let mut plan = TriePlan::new(context.id, limits, parent.data.root, &mut nodes);
    let planned = match &rows {
        RowsToVerify::Entry(entry) => insert_entry_rows(&mut plan, &entry.0, entry.1),
        RowsToVerify::Settled => {
            insert_settled_rows(&mut plan, &wal.settled, parent.data.generation + 1)
        }
    };
    planned.map_err(plan_preserved)?;
    let mut sink = VerifySink {
        key,
        expected: node_outputs.iter(),
    };
    let (root, retired) = plan.finish(&mut sink).map_err(plan_preserved)?;
    if sink.expected.next().is_some() || retired != wal.retired {
        return Err(MemberOutboxRefusal::Incomplete.into());
    }
    let expected = successor(parent, root, &wal.outputs, &retired, next_ordinal, front)?;
    if candidate.data != expected {
        return Err(MemberOutboxRefusal::Incomplete.into());
    }
    Ok(PreparedTransition {
        kind: wal.kind,
        parent: parent.id,
        candidate,
        wal_file,
        outputs: wal.outputs,
        retired,
    })
}

enum RowsToVerify {
    Entry(Box<(EntryData, PrivateChatPageId)>),
    Settled,
}

/// A checkpoint's settled items must be the contiguous prefix at the parent front, each the
/// exact entry its ordinal row names, with a disposition consistent with its own Admission.
pub(super) fn verify_settled(
    ns: &Namespace,
    context: &MemberOutboxContextData,
    key: &[u8; 32],
    parent: &SelectedHead,
    settled: &[SettledData],
) -> Result<(), MemberOutboxError> {
    if settled.len() as u64 > parent.data.pending() {
        return Err(MemberOutboxRefusal::Stale.into());
    }
    let mut nodes = DiskNodes { ns, key };
    for (offset, item) in settled.iter().enumerate() {
        let ordinal = parent.data.front + offset as u64;
        if item.ordinal != ordinal {
            return Err(MemberOutboxRefusal::Stale.into());
        }
        let row = match lookup(
            context.id,
            parent.data.root,
            RowKey::ordinal(ordinal),
            &mut nodes,
        )
        .map_err(preserved)?
        {
            Some(RowValue::Ordinal(row)) => row,
            _ => return Err(MemberOutboxRefusal::Incomplete.into()),
        };
        if row.entry != item.entry {
            return Err(MemberOutboxRefusal::Stale.into());
        }
        let entry = read_entry(ns, key, context.id, row.entry).map_err(preserved)?;
        let consistent = match item.disposition {
            DispositionCode::Submitted => entry.admission == item.admission,
            DispositionCode::Held => entry.admission != item.admission,
        };
        if entry.ordinal != ordinal || !consistent {
            return Err(MemberOutboxRefusal::Stale.into());
        }
    }
    Ok(())
}

#[cfg(test)]
impl MemberOutbox {
    pub(super) fn namespace_path_for_test(&self) -> PathBuf {
        self.ns.dir.path().to_path_buf()
    }
    pub(super) fn object_path_for_test(&self, role: ObjectRole, id: PrivateChatPageId) -> PathBuf {
        self.ns.object(role, id)
    }
    pub(super) fn reachable_nodes_for_test(&self, key: &[u8; 32]) -> Vec<PrivateChatPageId> {
        let head = self.clean().expect("clean owner");
        let mut nodes = DiskNodes { ns: &self.ns, key };
        super::trie::reachable_for_test(self.context.id, head.data.root, &mut nodes)
            .expect("walk the selected tree")
    }
    pub(super) fn head_data_for_test(&self) -> HeadData {
        self.clean().expect("clean owner").data.clone()
    }
}
