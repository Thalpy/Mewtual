//! Retained canonical evidence after this local admission has ended.
//!
//! This initial capsule deliberately offers no Server, transport, MLS signer, subscribe/run
//! method or history-completeness claim. In particular, ordinary positive MLS restore rejects
//! a snapshot whose actual own leaf was removed. The separately reviewed bounded read-only
//! ledger decoder will consume the retained canonical source without reviving that MLS actor.
use crate::store::owner_evidence::canonical::ValidatedOwnerSnapshot;

/// Original canonical snapshot/archive custody remains retained even when no authoring instance
/// can be restored. No decoder or caller-provided retirement bool constructs a runnable owner.
pub(crate) struct OfflineRetired {
    source: ValidatedOwnerSnapshot,
}
impl std::fmt::Debug for OfflineRetired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineRetired").finish_non_exhaustive()
    }
}
impl OfflineRetired {
    // Only the Store-backed canonical classifier may construct this negative-only result.
    pub(super) fn from_validated_source(source: ValidatedOwnerSnapshot) -> Self {
        Self { source }
    }

    /// Retained evidence only. The canonical source's exact old scope stays fenced; this borrow
    /// cannot select a later mount/admission or provide positive restore/publication permission.
    pub(crate) fn source(&self) -> &ValidatedOwnerSnapshot {
        &self.source
    }
}
