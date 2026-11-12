//! Readonly history inventory, kept separate from change reconstruction and transactions.
//!
//! These observers expose existing storage only. They do not verify authorship, establish a
//! complete historical archive, commit pending operations, or provide a snapshot across mutations.

use std::sync::Arc;

use crate::{AutoCommit, Automerge, Change, ChangeHash};

// Opt-in dev instrumentation only. Resolver2 keeps the replication dev-dependency's
// feature out of normal application builds. No operation contents or global cross-test
// state are recorded; observers can prove that a read did not enter reconstruction.
#[cfg(feature = "history-recovery-test-counters")]
thread_local! {
    static RECOVERY_WORK: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}
#[cfg(feature = "history-recovery-test-counters")]
/// Test-only cumulative reconstruction/value visits on this thread; contains no history.
pub fn history_recovery_work_on_this_thread() -> (usize, usize) {
    RECOVERY_WORK.with(std::cell::Cell::get)
}
#[cfg(feature = "history-recovery-test-counters")]
pub(crate) fn note_history_reconstruction() {
    RECOVERY_WORK.with(|work| {
        let (reconstructed, values) = work.get();
        work.set((reconstructed.saturating_add(1), values));
    });
}
#[cfg(feature = "history-recovery-test-counters")]
pub(crate) fn note_history_value() {
    RECOVERY_WORK.with(|work| {
        let (reconstructed, values) = work.get();
        work.set((reconstructed, values.saturating_add(1)));
    });
}

/// An opaque, process-local document-instance identity for rejecting replaced-source cursors.
///
/// Repeated reads and token clones compare equal. A newly constructed, loaded, cloned or forked
/// document has a distinct identity, even when its applied and queued change counts are identical.
/// The token keeps its allocation alive, preventing allocator-address reuse from creating an ABA.
/// This is neither a mutation revision nor an authority token; it is intentionally not serialized.
#[derive(Clone, Debug)]
pub struct HistoryInstanceId(Arc<()>);

impl PartialEq for HistoryInstanceId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for HistoryInstanceId {}

/// Unlike a copied public token, a copied document always starts a new observation instance.
/// Keeping this behavior on the private field leaves upstream's derived document Clone intact.
#[derive(Debug, Default)]
pub(crate) struct HistoryInstance(Arc<()>);

impl Clone for HistoryInstance {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Automerge {
    /// Borrow every applied change hash in internal graph insertion order, in constant time.
    ///
    /// This is not chronological, causal-authority or canonical page order. No metadata vector,
    /// parent list, operation body or reconstructed Change is created. Queued changes are separate
    /// in [`Self::queued_changes`]; uncommitted local operations are not changes in this inventory.
    /// Keep the borrow only for this readonly observation. A later mutation/replacement requires
    /// fresh source validation; this API supplies neither MVCC nor an immutable history snapshot.
    pub fn applied_change_hashes(&self) -> &[ChangeHash] {
        self.change_graph.all_hashes()
    }

    /// Borrow unapplied changes waiting for dependencies, in constant time.
    ///
    /// The queue owns these actual changes, so [`Change::raw_bytes`] can be borrowed/chunked
    /// without reconstructing an applied change. Their presence is not proof of an original signed
    /// envelope or of historical membership. Queue positions/order may change on any mutation,
    /// particularly when a missing parent arrives. Callers must reacquire and validate the source
    /// before resuming work after a mutation rather than retaining queue offsets as stable IDs.
    pub fn queued_changes(&self) -> &[Change] {
        self.queue.as_slice()
    }

    /// Test applied-graph membership through its existing hash index (expected constant time).
    ///
    /// A queued change returns false even when its exact raw bytes are retained. This test does
    /// not reconstruct or decode a Change and does not prove signature or projection admission.
    pub fn has_applied_change(&self, hash: &ChangeHash) -> bool {
        self.change_graph.has_change(hash)
    }

    /// Return the current nonserialized observation-instance identity without copying history.
    ///
    /// Ordinary in-place edits keep the identity. A replacement/load/clone gets a fresh identity;
    /// callers additionally need their own mutation revision or an uninterrupted readonly borrow.
    pub fn history_instance_id(&self) -> HistoryInstanceId {
        HistoryInstanceId(Arc::clone(&self.history_instance.0))
    }
}

impl AutoCommit {
    /// Borrow applied hashes without closing this document's pending transaction.
    ///
    /// Unlike mutating history helpers, this forwards directly through `&self`. Pending local
    /// operations are deliberately omitted. A reader claiming a complete current committed view
    /// must first require [`crate::transaction::Transactable::pending_ops`] to be zero, without
    /// using `document()`, `get_heads()` or a save to commit an interrupted edit as a read effect.
    /// The ordering and mutation limitations of [`Automerge::applied_change_hashes`] apply.
    pub fn applied_change_hashes(&self) -> &[ChangeHash] {
        self.doc.applied_change_hashes()
    }

    /// Borrow dependency-blocked changes without closing a pending local transaction.
    ///
    /// See [`Automerge::queued_changes`] for exact raw-byte and unstable queue-position semantics.
    pub fn queued_changes(&self) -> &[Change] {
        self.doc.queued_changes()
    }

    /// Test applied-graph membership without closing a pending local transaction.
    pub fn has_applied_change(&self, hash: &ChangeHash) -> bool {
        self.doc.has_applied_change(hash)
    }

    /// Read the underlying document-instance identity without closing a pending transaction.
    ///
    /// This identity does not change on ordinary edits and is not a mutation revision.
    pub fn history_instance_id(&self) -> HistoryInstanceId {
        self.doc.history_instance_id()
    }
}

#[cfg(test)]
mod tests;
