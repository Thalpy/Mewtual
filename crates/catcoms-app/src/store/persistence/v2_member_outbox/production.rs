//! Production owners of the member outboxes of ONE physical Store mount.
//!
//! [`MemberOutboxMount`] is installed at most once per physical Store mount, lazily, from the
//! namespace5 layout that this Store's concrete configured handler declared and mounted
//! (`critical_namespaces::member_outbox_layout`). It owns:
//!
//! * the mount's [`MemberOutboxRegistry`] (exclusive per-context owner claims);
//! * the production [`MemberOutboxParentDirectory`], held by the mount's original identity;
//! * a bounded table of per-context [`MemberContextCell`]s, each paid from the independent
//!   member-outbox pool.
//!
//! A cell lives with the Store, not with an operational host. A host dropped or replaced while
//! a save job runs therefore never loses that job's outcome or the exact signed original it was
//! saving: the original sits in the cell for the whole run (recovered-DATA custody), and the job
//! deposits its outcome there before it releases anything.
//!
//! What a cell may hold, and what it must never hold:
//! * the idle exclusive [`MemberOutbox`] owner, or nothing while a running job owns it;
//! * at most ONE request's custody DATA ([`MemberOriginalCustody`]);
//! * NEVER a save-source end, a physical lease or a native commit guard. Those move with the
//!   job and are released by it; parking one here would let dormant DATA hold a close open.
//!
//! Lock order: the Store's mount slot, then a mount's cell table, then one cell. No cell lock
//! is held across another cell's lock, and a job's completion takes only its own cell lock.
//! The final save edge only ever `try_lock`s (Busy instead of waiting).
//!
//! Authority boundary: nothing here grants membership, signing, save or publication rights.
//! Holding a cell or an owner is custody; a save still needs the host's final edge.
use super::data::{
    MemberOutboxContextData, MemberOutboxContextId, MemberReceiptData, MemberRequestToken,
    MemberSignedOriginal,
};
use super::engine::{
    MemberOutbox, MemberOutboxError, MemberOutboxOpen, MemberOutboxParentDirectory,
    MemberOutboxQuota, MemberOutboxRefusal, MemberOutboxRegistry, MEMBER_OUTBOX_OWNER_FEE,
    MEMBER_OUTBOX_RECOVERY_FEE,
};
use super::transaction::PreparedMemberAppend;
use crate::store::critical_namespaces::member_outbox_layout::{
    MAX_BASE_PATH, MAX_MEMBER_OUTBOX_CONTEXTS, MEMBER_OUTBOX_SLOT_FEE,
};
use crate::store::critical_namespaces::member_outbox_namespace_codec::MemberOutboxCapabilityData;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::critical_namespaces::refused;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_rt::{CryptoRngCore, RequestCancellation};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use zeroize::Zeroizing;

/// The key the member-outbox engine is given: this vault's database key, exactly as the
/// documented format specifies (`CHAT-V2-MEMBER-OUTBOX.md`, Formats). Control files derive
/// their own member-outbox keys from it inside the engine; immutable objects use the shared
/// private chat page crypto, separated from other page families by their role tag and context
/// ID. PERSISTENCE FORMAT: passing any other key makes existing outboxes unreadable (refused
/// as `Incomplete`, never treated as empty).
pub(in crate::store) fn member_outbox_key(
    store: &ServerStore,
) -> Result<Zeroizing<[u8; 32]>, AppError> {
    Ok(Zeroizing::new(store.keys.db_key()?))
}

/// Retained per-mount metadata on the member pool: the mount itself, its bounded cell table
/// and the held parent path. Each context's cell additionally pays
/// [`MEMBER_OUTBOX_SLOT_FEE`]. A logical inventory, not an allocator/RSS figure.
pub(in crate::store) const MEMBER_OUTBOX_MOUNT_FEE: usize =
    2 * std::mem::size_of::<MemberOutboxMount>()
        + MAX_MEMBER_OUTBOX_CONTEXTS * std::mem::size_of::<Arc<MemberContextCell>>()
        + MAX_BASE_PATH
        + 4 * 1024;

// The cell's own layout (owner, custody, lock) must fit the slot invoice it pays.
const _: () = assert!(std::mem::size_of::<MemberContextCell>() <= MEMBER_OUTBOX_SLOT_FEE);

/// The one request whose signed original this Store holds for a context.
pub(in crate::store) enum MemberOriginalCustody {
    /// Presealed and NOT admitted. Admission needs a fresh final edge; dropping it writes
    /// nothing. Its exact original is inside.
    Prepared(Box<PreparedMemberAppend>),
    /// A job owns the exclusive owner and runs (or is about to run) the commit. The exact
    /// original stays HERE so an unwind or an unrun drop cannot lose it.
    Running {
        token: MemberRequestToken,
        original: MemberSignedOriginal,
    },
    /// The outcome is not known: an uncertain run, a lost (unwound) job, or a de-admission
    /// whose original did not match its commit. Recover the owner, then look the token up with
    /// the original's OWN signed request: Hit settles it; Absent re-saves THIS original. Never
    /// sign again for it. `lost` records that the job unwound (diagnostic DATA only).
    Unresolved {
        token: MemberRequestToken,
        original: MemberSignedOriginal,
        lost: bool,
    },
    /// SavedLocal, cached by the job before it released anything. The member lane is told on
    /// the next phase-1 call; the durable entry is authoritative either way.
    Saved(MemberReceiptData),
}
impl std::fmt::Debug for MemberOriginalCustody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The kind only: originals, receipts and tokens are private correlation DATA.
        match self {
            Self::Prepared(_) => f.write_str("Prepared"),
            Self::Running { .. } => f.write_str("Running"),
            Self::Unresolved { lost, .. } => write!(f, "Unresolved {{ lost: {lost} }}"),
            Self::Saved(_) => f.write_str("Saved"),
        }
    }
}
impl MemberOriginalCustody {
    pub(in crate::store) fn token(&self) -> MemberRequestToken {
        match self {
            Self::Prepared(prepared) => prepared.admission_facts().token,
            Self::Running { token, .. } | Self::Unresolved { token, .. } => *token,
            Self::Saved(receipt) => receipt.token,
        }
    }
}

/// One context's state on this mount. Field order is Drop order: the owner (and its claim)
/// before the custody DATA.
#[derive(Default)]
pub(in crate::store) struct MemberContextState {
    /// The idle exclusive owner. `None`: not opened yet on this mount, moved into a running
    /// job, or dropped by an unwound job (the next phase-1 call reopens and recovers).
    pub(in crate::store) owner: Option<MemberOutbox>,
    pub(in crate::store) custody: Option<MemberOriginalCustody>,
}

/// A context's Store-scoped cell. Shared (by `Arc`) only with the job that is running for it;
/// no Store, host or SDK backlink is reachable from here.
pub(in crate::store) struct MemberContextCell {
    context: MemberOutboxContextId,
    state: Mutex<MemberContextState>,
    // LAST: the slot payment outlives every value the state above holds.
    _slot: CatalogReadReservation,
}
impl std::fmt::Debug for MemberContextCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberContextCell").finish_non_exhaustive()
    }
}
impl MemberContextCell {
    /// Strict read: a poisoned cell refuses. Diagnostics only.
    pub(in crate::store) fn lock(&self) -> Result<MutexGuard<'_, MemberContextState>, AppError> {
        self.state.lock().map_err(|_| refused())
    }
    /// Phase 1 (which may block): recover a poisoned cell instead of disabling the context for
    /// the Store's life. Every write to the state is a single assignment and phase 1 keeps
    /// custody in place across lane and IO calls, so a panic that poisoned the cell left it
    /// either intact or with one in-hand original gone (crash-equivalent). The lane then still
    /// holds that token Lent with no custody, and the next call retires the token after an
    /// exact Absent lookup instead of signing again.
    pub(in crate::store) fn lock_recovering(&self) -> MutexGuard<'_, MemberContextState> {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.state.clear_poison();
        guard
    }
    /// The final edge never waits: `Ok(None)` means another holder (Busy).
    pub(in crate::store) fn try_lock(
        &self,
    ) -> Result<Option<MutexGuard<'_, MemberContextState>>, AppError> {
        match self.state.try_lock() {
            Ok(guard) => Ok(Some(guard)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Poisoned(_)) => Err(refused()),
        }
    }
    /// Completion and de-admission ONLY: depositing a job's exact DATA is never refused, even
    /// into a poisoned cell. Never used to admit: the final edge's [`Self::try_lock`] refuses a
    /// poisoned cell, and the next phase-1 call ([`Self::lock_recovering`]) reads the outcome.
    pub(in crate::store) fn lock_for_completion(&self) -> MutexGuard<'_, MemberContextState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The member-outbox production owners of one physical Store mount. See the module docs.
pub(in crate::store) struct MemberOutboxMount {
    registry: MemberOutboxRegistry,
    parent: MemberOutboxParentDirectory,
    capability: MemberOutboxCapabilityData,
    pool: CriticalCatalogReadPool,
    quota: MemberOutboxQuota,
    max_contexts: usize,
    // The physical mount this table was installed on, and that mount's liveness negative.
    // Compared by identity; neither is a permission.
    store_mount: Arc<()>,
    negative: RequestCancellation,
    cells: Mutex<Vec<Arc<MemberContextCell>>>,
    // LAST: the table's metadata payment outlives every cell reference above.
    _fee: CatalogReadReservation,
}
impl std::fmt::Debug for MemberOutboxMount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberOutboxMount").finish_non_exhaustive()
    }
}

impl MemberOutboxMount {
    /// The mounted namespace5 capability (vault, Current3 root, descriptor). DATA only.
    pub(in crate::store) fn capability(&self) -> MemberOutboxCapabilityData {
        self.capability
    }

    /// Reserve and validate one invoice on the independent member-outbox pool.
    pub(in crate::store) fn pay(&self, fee: usize) -> Result<CatalogReadReservation, AppError> {
        let payment = self.pool.try_reserve(fee)?;
        payment.validate(&self.pool, fee)?;
        Ok(payment)
    }

    /// Cached negatives only: no IO, no lock, no wait. The table must belong to THIS physical
    /// mount and that mount must still be live.
    pub(in crate::store) fn require_live(&self, store: &ServerStore) -> Result<(), AppError> {
        if self.negative.is_cancelled() || !Arc::ptr_eq(&self.store_mount, &store.registry_mount())
        {
            return Err(refused());
        }
        Ok(())
    }

    /// The cell of `context`, creating it (bounded by the policy, paid first) if this mount
    /// has none yet. Cells are never removed during the mount's life.
    pub(in crate::store) fn cell(
        &self,
        context: MemberOutboxContextId,
    ) -> Result<Arc<MemberContextCell>, AppError> {
        let mut cells = self.cells.lock().map_err(|_| refused())?;
        if let Some(cell) = cells.iter().find(|cell| cell.context == context) {
            return Ok(cell.clone());
        }
        if cells.len() >= self.max_contexts {
            return Err(refused());
        }
        let slot = self.pay(MEMBER_OUTBOX_SLOT_FEE)?;
        let cell = Arc::new(MemberContextCell {
            context,
            state: Mutex::new(MemberContextState::default()),
            _slot: slot,
        });
        cells.push(cell.clone());
        Ok(cell)
    }

    /// An EXISTING cell only; the final edge never creates one and never waits (`Ok(None)` is
    /// "absent or busy").
    pub(in crate::store) fn existing_cell(
        &self,
        context: MemberOutboxContextId,
    ) -> Result<Option<Arc<MemberContextCell>>, AppError> {
        match self.cells.try_lock() {
            Ok(cells) => Ok(cells.iter().find(|cell| cell.context == context).cloned()),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Poisoned(_)) => Err(refused()),
        }
    }

    /// Open (creating it if needed) the namespace of `context`, completing or preserving any
    /// interrupted work, under this mount's registry and production parent. Pays the owner's
    /// retained metadata and the open/recovery work invoice first; holds the Store's physical
    /// lease through every open/recovery write.
    pub(in crate::store) fn open_owner(
        &self,
        store: &ServerStore,
        context: MemberOutboxContextData,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<MemberOutbox, MemberOutboxError> {
        // A logical payment shortfall is a retryable Payment refusal, never an IO failure.
        let metadata = self
            .pay(MEMBER_OUTBOX_OWNER_FEE)
            .map_err(|_| MemberOutboxRefusal::Payment)?;
        let work = self
            .pay(MEMBER_OUTBOX_RECOVERY_FEE)
            .map_err(|_| MemberOutboxRefusal::Payment)?;
        let lease = store._session.retain_for_physical_work();
        MemberOutbox::open(
            MemberOutboxOpen {
                registry: &self.registry,
                parent: &self.parent,
                context,
                quota: self.quota,
                create: true,
                pool: &self.pool,
                metadata,
                work,
                lease,
            },
            key,
            rng,
        )
    }

    /// Re-derive an owner's state from its files after an uncertain operation.
    pub(in crate::store) fn recover_owner(
        &self,
        store: &ServerStore,
        owner: &mut MemberOutbox,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), MemberOutboxError> {
        let work = self
            .pay(MEMBER_OUTBOX_RECOVERY_FEE)
            .map_err(|_| MemberOutboxRefusal::Payment)?;
        owner.recover(key, rng, store._session.retain_for_physical_work(), work)
    }
}

impl ServerStore {
    /// Install (once per physical mount) or return this Store's member-outbox mount. Requires
    /// the concrete configured namespace5 handler and its validated mounted layout, which the
    /// install re-proves (`MemberOutboxParentDirectory::from_mount`). Later calls check only
    /// cached negatives. Refuses on an opener without the handler.
    pub(in crate::store) fn member_outbox_mount(&self) -> Result<Arc<MemberOutboxMount>, AppError> {
        let policy = self.member_outbox_policy().ok_or_else(refused)?;
        let mut slot = self.member_outbox_mount.lock().map_err(|_| refused())?;
        if let Some(mount) = slot.as_ref() {
            mount.require_live(self)?;
            return Ok(mount.clone());
        }
        policy.require_capacity()?;
        let fee = policy.pool.try_reserve(MEMBER_OUTBOX_MOUNT_FEE)?;
        fee.validate(&policy.pool, MEMBER_OUTBOX_MOUNT_FEE)?;
        let capability = self
            .member_outbox_layout
            .lock()
            .map_err(|_| refused())?
            .as_ref()
            .ok_or_else(refused)?
            .capability;
        let parent = MemberOutboxParentDirectory::from_mount(self)?;
        let mount = Arc::new(MemberOutboxMount {
            registry: MemberOutboxRegistry::new(),
            parent,
            capability,
            pool: policy.pool.clone(),
            quota: policy.quota,
            max_contexts: policy.max_contexts,
            store_mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            cells: Mutex::new(Vec::with_capacity(policy.max_contexts)),
            _fee: fee,
        });
        mount.require_live(self)?;
        *slot = Some(mount.clone());
        Ok(mount)
    }

    /// The ALREADY installed mount for the final edge: never installs, never does IO and never
    /// waits (`Ok(None)` is "not installed or busy").
    pub(in crate::store) fn installed_member_outbox_mount(
        &self,
    ) -> Result<Option<Arc<MemberOutboxMount>>, AppError> {
        match self.member_outbox_mount.try_lock() {
            Ok(slot) => match slot.as_ref() {
                Some(mount) => {
                    mount.require_live(self)?;
                    Ok(Some(mount.clone()))
                }
                None => Ok(None),
            },
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Poisoned(_)) => Err(refused()),
        }
    }
}

// The mount is reachable from the Store (shared across threads) and a cell moves with a job
// onto the blocking pool: both must stay thread-safe as their contents evolve.
const _: fn() = || {
    fn shared<T: Send + Sync + 'static>() {}
    shared::<MemberOutboxMount>();
    shared::<MemberContextCell>();
};
