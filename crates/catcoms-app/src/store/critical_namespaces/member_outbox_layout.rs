//! Concrete configured handler for critical namespace5: the ordinary-member outbox layout.
//!
//! This module owns the trusted configuration of the member-outbox namespace (the independent
//! logical pool every member-outbox operation pays from, persistent quotas, and the number of
//! member contexts one Store mount may own at once) and the physical namespace5 declaration:
//!
//! ```text
//! <vault>/chat-critical-namespaces.member-outbox.seed  declaration seed (transient)
//! <vault>/chat-owner-v2/member-outbox-layout.bin       sealed marker: the exact descriptor
//! <vault>/chat-owner-v2/member-outbox-v1/              parent of every `mo-<context>` namespace
//! ```
//!
//! The configuration is installed in `V2CurrentLayoutPolicy` before physical opening; it is
//! never decoded from storage. Every opener WITHOUT it refuses an ID5 catalog entry and every
//! fixed artifact above before any old cleaner ([`refuse_unsupported_storage`]). Binaries that
//! predate namespace5 are fenced the same way governance4 fences them: an owned EMPTY legacy
//! catalog stage is created and synced before any CONTENT-BEARING member output (seed bytes,
//! marker, parent, candidate), and they already reject that framing; once promoted, their
//! closed grammars reject ID5 itself. They are not fenced from a zero-byte seed claim, nor from
//! an abandoned unauthenticated seed while its disposal runs: neither carries anything.
//!
//! Declaration order (warm, one owner, every step idempotent under cold recovery):
//! zero-byte seed claim -> empty stage fence -> seed bytes -> marker -> parent directory ->
//! complete candidate stage -> rename stage onto current (commit point) -> verify -> remove
//! seed. Cold recovery with a complete authenticated seed finishes exactly that sequence.
//!
//! A FIRST seed that never authenticated (a crash before it was complete) is abandoned
//! instead: the cold inspector's first-torn role unlinks the zero-byte fence, then the seed,
//! and the warm retry declares afresh. It applies only when nothing a complete seed precedes
//! exists (no marker, no parent, no ID5 in the authenticated current catalog). An authenticated
//! ordinary JOIN successor from the claim window is left to JOIN settlement. A zero-byte
//! predecessor stage in that window is indistinguishable from the fence and is disposable;
//! other partial/foreign stages, markers and directories are preserved and refused.
//! Design and residuals: `docs/communication-recovery/CHAT-V2-NAMESPACE5-RECOVERY.md`.
//!
//! Authority boundary. Declaring or mounting this namespace grants no membership, signing,
//! save or publication right. A member save still needs the actual current member source and
//! the final native/UI admission (`persistence::v2_member_outbox`'s production issuer).
use super::member_outbox_namespace_codec::MemberOutboxCapabilityData;
use super::read_pool::CatalogReadReservation;
use super::*;
use crate::store::persistence::v2_member_outbox::{
    MemberOutbox, MemberOutboxQuota, MEMBER_OUTBOX_APPEND_FEE, MEMBER_OUTBOX_CHECKPOINT_FEE,
    MEMBER_OUTBOX_DEBT_FEE, MEMBER_OUTBOX_LOOKUP_FEE, MEMBER_OUTBOX_MOUNT_FEE,
    MEMBER_OUTBOX_OWNER_FEE, MEMBER_OUTBOX_RECOVERY_FEE,
};

pub(in crate::store) mod codec;
mod declaration;

pub(in crate::store) const MEMBER_OUTBOX_SEED: &str = "chat-critical-namespaces.member-outbox.seed";
pub(in crate::store) const MEMBER_OUTBOX_MARKER: &str = "member-outbox-layout.bin";
pub(in crate::store) const MEMBER_OUTBOX_PARENT: &str = "member-outbox-v1";
pub(in crate::store) const MAX_BASE_PATH: usize = current_layout::MAX_OPEN_BASE_PATH;

#[cfg(test)]
thread_local! {
    static EMULATE_PRE_NAMESPACE5: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static COLD_FAULT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static COLD_FAULT_FIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static NAME_REPORTED_PRESENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Regression-only cold cut inside the first-torn disposal (226, 227, 228; see the persistence
/// module). The Store, and so its fault atomic, is created inside the very `open` that runs
/// the disposal, so this seam is thread-local and armed before `open`. It is one-shot: it
/// disarms itself when it fires, and tests assert it fired, so a dead seam cannot make a
/// "completes on reopen" assertion vacuous.
#[cfg(test)]
pub(in crate::store) fn arm_member_cold_fault_for_test(point: u8) {
    COLD_FAULT.with(|cell| cell.set(point));
    COLD_FAULT_FIRED.with(|cell| cell.set(false));
}
/// Whether the armed cut fired since it was armed or last taken; resets the observation, so a
/// later call reports only a NEW firing.
#[cfg(test)]
pub(in crate::store) fn take_member_cold_fault_fired_for_test() -> bool {
    COLD_FAULT_FIRED.with(|cell| cell.replace(false))
}
/// Regression-only, one-shot: the next post-unlink NotFound proof reports the name as still
/// present (the Windows delete-pending case a real runner does not reproduce).
#[cfg(test)]
pub(in crate::store) fn report_name_present_once_for_test() {
    NAME_REPORTED_PRESENT.with(|cell| cell.set(true));
}
#[cfg(test)]
pub(in crate::store) fn take_name_reported_present_for_test() -> bool {
    NAME_REPORTED_PRESENT.with(|cell| cell.replace(false))
}
/// The first-torn disposal's cold cut points. Always `Ok` outside regressions.
pub(in crate::store) fn member_cold_fault(point: u8) -> Result<(), AppError> {
    #[cfg(test)]
    if COLD_FAULT.with(std::cell::Cell::get) == point {
        COLD_FAULT.with(|cell| cell.set(0));
        COLD_FAULT_FIRED.with(|cell| cell.set(true));
        return Err(AppError::Io(
            "injected member-outbox first-torn disposal cut".into(),
        ));
    }
    let _ = point;
    Ok(())
}
/// Regression-only emulation of a binary that predates namespace5: such a binary does not
/// know the member fixed names, so the name fence and governance's member-seed prior-debt
/// check are skipped. Grammar and every other gate are unchanged. Always off in production.
#[cfg(test)]
pub(in crate::store) fn emulate_pre_namespace5_for_test(on: bool) {
    EMULATE_PRE_NAMESPACE5.with(|cell| cell.set(on));
}
#[cfg(test)]
pub(in crate::store) fn pre_namespace5_emulated() -> bool {
    EMULATE_PRE_NAMESPACE5.with(std::cell::Cell::get)
}
#[cfg(not(test))]
pub(in crate::store) const fn pre_namespace5_emulated() -> bool {
    false
}

/// Negative fixed-name fence for every opener WITHOUT the concrete member-outbox handler.
/// Presence of any kind (file, directory, dangling link, unreadable entry) refuses: none can
/// be discarded, cleaned or interpreted as an empty namespace by an older gate.
pub(in crate::store) fn refuse_unsupported_storage(dir: &Path) -> Result<(), AppError> {
    if pre_namespace5_emulated() {
        return Ok(());
    }
    let root = dir.join(ARCHIVE_BOOTSTRAP_ROOT);
    for path in [
        dir.join(MEMBER_OUTBOX_SEED),
        root.join(MEMBER_OUTBOX_MARKER),
        root.join(MEMBER_OUTBOX_PARENT),
    ] {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    Ok(())
}

/// Per-context resident slot invoice beside the owner's own retained metadata fee: the owner
/// value itself (moved between the Store's slot table and a running job), fixed slot,
/// map-node and completion-cell overhead, and the heap of ONE retained signed original. The
/// cell keeps a Running or Unresolved original after its append payment has drained, so that
/// copy is paid here. A logical inventory, not an allocator/RSS figure.
pub(in crate::store) const MEMBER_OUTBOX_SLOT_FEE: usize = 2 * std::mem::size_of::<MemberOutbox>()
    + 2 * catcoms_replication::chat::MAX_ACTION_BYTES
    + 8 * 1024;

/// Hard ceiling on member contexts per Store mount. The founder-as-member vertical needs one;
/// joined-member sources add one per joined group. Bounded so the slot table and its pool
/// requirement stay finite.
pub(in crate::store) const MAX_MEMBER_OUTBOX_CONTEXTS: usize = 64;

/// Trusted member-outbox configuration. Clones share the SAME pool account.
#[derive(Clone)]
pub struct V2MemberOutboxPolicy {
    /// Independent of `archive.pending.pool`: member-outbox work can neither starve nor be
    /// starved by catalog/JOIN/founder work. Every engine invoice is paid from here.
    pub(in crate::store) pool: CriticalCatalogReadPool,
    pub(in crate::store) quota: MemberOutboxQuota,
    pub(in crate::store) max_contexts: usize,
    /// Fixed-file capacity for the namespace5 declaration artifacts (see
    /// [`codec::PERSISTENT_MINIMUM`]); not free-space evidence.
    persistent_capacity: usize,
}
impl std::fmt::Debug for V2MemberOutboxPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2MemberOutboxPolicy")
            .field("quota", &self.quota)
            .field("max_contexts", &self.max_contexts)
            .finish_non_exhaustive()
    }
}

/// Logical requirement for ONE context: its retained owner metadata and slot, plus every
/// value that the supported save path can simultaneously retain for it. `refresh_prepared`
/// can recover an owner while its cell still holds an old prepared append (6+6 MiB today);
/// it drops lookup payment before recovery and old append payment before replacement.
/// The existing work allowance below (17 MiB today) covers that real overlap. It also
/// inventories publication/checkpoint work, which is NOT yet connected to these cells.
/// Before connecting it, recalculate all overlaps: recovery and prepared append are not
/// mutually exclusive, and this formula is not proof that future combined work fits.
const fn per_context_requirement() -> usize {
    let open_or_append = if MEMBER_OUTBOX_RECOVERY_FEE > MEMBER_OUTBOX_APPEND_FEE {
        MEMBER_OUTBOX_RECOVERY_FEE
    } else {
        MEMBER_OUTBOX_APPEND_FEE
    };
    MEMBER_OUTBOX_OWNER_FEE
        + MEMBER_OUTBOX_SLOT_FEE
        + open_or_append
        + MEMBER_OUTBOX_LOOKUP_FEE
        + MEMBER_OUTBOX_DEBT_FEE
        + MEMBER_OUTBOX_CHECKPOINT_FEE
}

impl V2MemberOutboxPolicy {
    /// Configure the member-outbox handler. `pool` must be an account dedicated to member
    /// outboxes with capacity for `max_contexts` simultaneous contexts (see
    /// [`Self::required_capacity`]). Quotas bound persistent usage: pressure refuses new
    /// work and never evicts receipts or debt. `persistent_capacity` must cover the
    /// declaration's fixed files.
    pub fn new(
        pool: CriticalCatalogReadPool,
        max_pending: u64,
        max_live_bytes: u64,
        max_live_objects: u64,
        max_contexts: usize,
        persistent_capacity: usize,
    ) -> Result<Self, AppError> {
        let policy = Self {
            pool,
            quota: MemberOutboxQuota {
                max_pending,
                max_live_bytes,
                max_live_objects,
            },
            max_contexts,
            persistent_capacity,
        };
        policy.require_capacity()?;
        Ok(policy)
    }

    /// Exact checked logical pool requirement for `max_contexts` contexts, plus the Store
    /// mount's own retained table metadata (`MEMBER_OUTBOX_MOUNT_FEE`).
    pub fn required_capacity(max_contexts: usize) -> Option<usize> {
        per_context_requirement()
            .checked_mul(max_contexts)?
            .checked_add(MEMBER_OUTBOX_MOUNT_FEE)
    }

    /// Installation check against the opener's catalog/JOIN pool: the member pool must be a
    /// DIFFERENT account (independent credits), and the catalog pool must be able to fund the
    /// namespace5 declaration phase and its retained marker at once (both are held together
    /// while the cold inspector captures the marker).
    pub(in crate::store) fn require_independent_of(
        &self,
        catalog: &CriticalCatalogReadPool,
    ) -> Result<(), AppError> {
        let declaration = codec::DECLARATION_FEE
            .checked_add(MOUNTED_FEE)
            .ok_or_else(refused)?;
        if self.pool.same_account(catalog) || catalog.capacity() < declaration {
            return Err(refused());
        }
        Ok(())
    }

    /// Repeated at configuration, before declaration and at mount: nonzero quotas, a bounded
    /// context count, a pool whose configured capacity covers every context's simultaneous
    /// invoices, and the declaration's fixed-file capacity.
    pub(in crate::store) fn require_capacity(&self) -> Result<(), AppError> {
        if self.quota.max_pending == 0
            || self.quota.max_live_bytes == 0
            || self.quota.max_live_objects == 0
            || self.max_contexts == 0
            || self.max_contexts > MAX_MEMBER_OUTBOX_CONTEXTS
            || self.persistent_capacity < codec::PERSISTENT_MINIMUM
        {
            return Err(refused());
        }
        let required = Self::required_capacity(self.max_contexts).ok_or_else(refused)?;
        if self.pool.capacity() < required {
            return Err(refused());
        }
        Ok(())
    }
}

/// One held real directory (the `chat-owner-v2` root, or the member parent): its open handle
/// and OS identity, captured before any leaf below it is opened. Never caller-supplied.
pub(in crate::store) struct MemberOutboxDirectoryCapture {
    pub(in crate::store) file: File,
    pub(in crate::store) identity: catcoms_storage::kept::SessionId,
}

/// The complete retained declaration, warm or cold. BOTH whole catalog frames stay available
/// after promotion; nothing is synthesized from a Summary. Field order is Drop order: every
/// File drains before the caller's separately held declaration fee.
pub(in crate::store) struct Declaration {
    pub(in crate::store) capability: MemberOutboxCapabilityData,
    pub(in crate::store) original: Zeroizing<Vec<u8>>,
    pub(in crate::store) before: pending_join_codec::Summary,
    pub(in crate::store) candidate: Zeroizing<Vec<u8>>,
    pub(in crate::store) next: pending_join_codec::Summary,
    pub(in crate::store) marker: Zeroizing<Vec<u8>>,
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
    pub(in crate::store) root: MemberOutboxDirectoryCapture,
    pub(in crate::store) initial_current: pending_join::CreatedStage,
    pub(in crate::store) initial_current_frame: [u8; 32],
    pub(in crate::store) seed_file: Option<pending_join::CreatedStage>,
    // A successful unlink is cached BEFORE closing handles/fallible barriers. On Windows
    // our own open File would otherwise keep the name delete-pending across warm retries.
    // Retained seed bytes still bind the same declaration; true only permits proving absence.
    pub(in crate::store) seed_unlinked: bool,
    pub(in crate::store) stage_file: Option<pending_join::CreatedStage>,
    // Cold AlreadyPromoted only: an independently captured exact duplicate stage, retained
    // across every wait and re-compared before unlink. A warm owner never adopts one.
    pub(in crate::store) captured_duplicate_stage: Option<pending_join::CreatedStage>,
    pub(in crate::store) marker_file: Option<pending_join::CreatedStage>,
    // The parent directory, once created (warm) or proven (cold) under the held root.
    pub(in crate::store) parent: Option<MemberOutboxDirectoryCapture>,
}

/// What the cold inspector classified at the catalog stage slot for a first-torn disposal
/// (`ServerStore::dispose_first_torn_member_outbox_seed`). DATA plus at most one held File;
/// it grants no write beyond that one classified unlink.
pub(in crate::store) enum FirstTornStage {
    /// No stage: nothing to remove there.
    Absent,
    /// A real zero-byte file beside the seed claim: this declaration's legacy fence. (In the
    /// documented downgrade residual it may be another writer's empty stage, which holds no
    /// bytes either.) Unlinked first.
    Fence((File, Zeroizing<Vec<u8>>)),
    /// A stage that is never this declaration's: a complete authenticated ordinary successor
    /// without ID5, or any stage beside a governance4 seed. Left to its owner's inspector.
    Untouched,
}

/// Original installed marker plus the held parent directory. No outbox owner, save or
/// governance capability is represented. Fee LAST: Files/metadata drain first.
pub(in crate::store) struct MountedMemberOutboxLayout {
    pub(in crate::store) capability: MemberOutboxCapabilityData,
    pub(in crate::store) marker: File,
    pub(in crate::store) marker_id: catcoms_storage::kept::SessionId,
    pub(in crate::store) marker_digest: [u8; 32],
    pub(in crate::store) root: MemberOutboxDirectoryCapture,
    pub(in crate::store) parent: MemberOutboxDirectoryCapture,
    pub(in crate::store) mount: Arc<()>,
    pub(in crate::store) negative: RequestCancellation,
    pub(in crate::store) _fee: CatalogReadReservation,
}

/// The exact first-probed marker and parent, kept across older inspector/account waits so a
/// byte-equal replacement cannot be adopted by the later installation.
pub(in crate::store) struct MemberOutboxMarkerCapture {
    pub(in crate::store) capability: MemberOutboxCapabilityData,
    pub(in crate::store) marker: File,
    pub(in crate::store) marker_id: catcoms_storage::kept::SessionId,
    pub(in crate::store) raw: Zeroizing<Vec<u8>>,
    pub(in crate::store) root: MemberOutboxDirectoryCapture,
    pub(in crate::store) parent: MemberOutboxDirectoryCapture,
    pub(in crate::store) fee: CatalogReadReservation,
}

/// Retained marker/parent metadata: its own debit on the catalog/JOIN pool, separate from the
/// transient declaration fee, held for the private Store lifetime. Named buffers and handles
/// only; not allocator/RSS evidence.
pub(in crate::store) const MOUNTED_FEE: usize =
    2 * std::mem::size_of::<MountedMemberOutboxLayout>()
        + 2 * std::mem::size_of::<MemberOutboxMarkerCapture>()
        + 8 * std::mem::size_of::<File>()
        + 4 * std::mem::size_of::<std::fs::Metadata>()
        + 2 * (codec::MAX_MARKER_FRAME + 1)
        + 2 * codec::MAX_MARKER_PLAIN
        + 7 * (MAX_BASE_PATH + 256)
        + 2 * (32 * 1024 + 256)
        + 4 * 64;
const _: () = assert!(
    2 * std::mem::size_of::<Declaration>()
        + 4 * std::mem::size_of::<pending_join_codec::Parsed<'static>>()
        + 4 * std::mem::size_of::<pending_join_codec::Summary>()
        + std::mem::size_of::<MountedMemberOutboxLayout>()
        + 8 * std::mem::size_of::<File>()
        + 4 * std::mem::size_of::<std::fs::Metadata>()
        + 7 * (MAX_BASE_PATH + 256)
        + 2 * (32 * 1024 + 256)
        + 32 * 1024
        <= 512 * 1024
);
// Physical regressions live beside the openers that own the fixtures:
// `current_layout/tests/member_outbox.rs` and `governance_layout/tests/member_outbox.rs`.
