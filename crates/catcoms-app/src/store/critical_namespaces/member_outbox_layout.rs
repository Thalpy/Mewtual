//! Concrete configured handler for critical namespace5: the ordinary-member outbox layout.
//!
//! This module owns the trusted configuration of the member-outbox namespace: the independent
//! logical pool every member-outbox operation pays from, the persistent quotas, and the number
//! of member contexts one Store mount may own at once. The configuration is installed in
//! `V2CurrentLayoutPolicy` before physical opening; it is never decoded from storage, and an
//! opener without it refuses ID5 and every member-outbox fixed artifact before old cleaners.
//!
//! Authority boundary. Configuring this handler grants no membership, signing, save or
//! publication right. A member save still needs the actual current member source and the
//! final native/UI admission (`persistence::v2_member_outbox`'s production issuer).
use super::read_pool::CriticalCatalogReadPool;
use super::refused;
use crate::store::persistence::v2_member_outbox::{
    MemberOutbox, MemberOutboxQuota, MEMBER_OUTBOX_APPEND_FEE, MEMBER_OUTBOX_CHECKPOINT_FEE,
    MEMBER_OUTBOX_DEBT_FEE, MEMBER_OUTBOX_LOOKUP_FEE, MEMBER_OUTBOX_OWNER_FEE,
    MEMBER_OUTBOX_RECOVERY_FEE,
};
use crate::AppError;

/// Per-context resident slot invoice beside the owner's own retained metadata fee: the owner
/// value itself (moved between the Store's slot table and a running job) plus fixed slot,
/// map-node and completion-cell overhead. A logical inventory, not an allocator/RSS figure.
pub(in crate::store) const MEMBER_OUTBOX_SLOT_FEE: usize =
    2 * std::mem::size_of::<MemberOutbox>() + 8 * 1024;

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
/// value that can be simultaneously retained for it. Open/recovery and append preparation
/// never coexist for one owner (both need the exclusive owner and recovery consumes no
/// prepared work), so the larger of the two counts once; a lookup hit, a debt batch held
/// during publication and a prepared checkpoint can all coexist with prepared append work.
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
    /// work and never evicts receipts or debt.
    pub fn new(
        pool: CriticalCatalogReadPool,
        max_pending: u64,
        max_live_bytes: u64,
        max_live_objects: u64,
        max_contexts: usize,
    ) -> Result<Self, AppError> {
        let policy = Self {
            pool,
            quota: MemberOutboxQuota {
                max_pending,
                max_live_bytes,
                max_live_objects,
            },
            max_contexts,
        };
        policy.require_capacity()?;
        Ok(policy)
    }

    /// Exact checked logical requirement for `max_contexts` contexts.
    pub fn required_capacity(max_contexts: usize) -> Option<usize> {
        per_context_requirement().checked_mul(max_contexts)
    }

    /// Repeated at configuration and at mount: nonzero quotas, a bounded context count and a
    /// pool whose configured capacity covers every context's simultaneous invoices.
    pub(in crate::store) fn require_capacity(&self) -> Result<(), AppError> {
        if self.quota.max_pending == 0
            || self.quota.max_live_bytes == 0
            || self.quota.max_live_objects == 0
            || self.max_contexts == 0
            || self.max_contexts > MAX_MEMBER_OUTBOX_CONTEXTS
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
