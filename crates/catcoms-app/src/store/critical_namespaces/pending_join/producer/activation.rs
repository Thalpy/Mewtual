//! Actual saved-source to original typed transport/task custody.
//! Explicit execution adds saved arms and signed INPUT DATA; SDK/Intent/ready remain next.
use super::*;
use crate::store::chat_admission::predecessor::{
    OriginalPredecessorTask, PreparedOriginalPredecessor,
};
use catcoms_rt::{MeshTransport, PeerId, RequestCancellation};
use catcoms_sync::PreOwnerConnectionHandoff;

// Fixed logical metadata allowance for the task, weak registry slot, two bounded key copies,
// Arc/wakeup bookkeeping and caller wrapper. This is deliberately NOT a physical heap/RSS
// measurement; provider/source buffers retain their separate original paid owners below.
const ACTIVATION_METADATA: usize = 4096;
pub(super) mod network;
pub use network::{CheckpointJoinNetwork, CheckpointJoinSubmissionStatus};

/// One actual saved finite-checkpoint source, original T/target and negative task lifetime.
/// Its only public operations are negative close/cancellation and local route eligibility.
/// It has no direct request/finish/old-initializer/ready/source-extraction API. Explicit
/// into_network_execution preserves SAME T and adds only saved arms plus signed input DATA.
pub struct ActivatedCheckpointJoin<T> {
    // First field: negative sender drops BEFORE source/transport, without a custom Drop which
    // would prevent an exact consuming handoff of the same owned fields to the network owner.
    task: OriginalPredecessorTask,
    preparation: JournalledCheckpointJoinPreparation,
    transport: T,
    target: PeerId,
    // Initialized once, outside any eventual request future. Never seed from a snapshot or
    // move an old prefix to a different T. Network is intentionally still unavailable here.
    _prefix: PreOwnerConnectionHandoff,
    _activation_fee: CatalogReadReservation,
}
impl<T> std::fmt::Debug for ActivatedCheckpointJoin<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActivatedCheckpointJoin")
            .finish_non_exhaustive()
    }
}
impl<T> ActivatedCheckpointJoin<T> {
    /// Stop only this original task. No allowance/source/counter is deleted or refunded.
    pub fn close(&self) {
        self.task.close();
    }
    /// Negative-only original observation; retaining it cannot keep its publisher alive.
    pub fn cancellation(&self) -> RequestCancellation {
        self.task.observer()
    }
}
impl<T: MeshTransport> ActivatedCheckpointJoin<T> {
    /// Local route DATA only. It grants no submission, and a later execution consumer must
    /// capture/recheck the actual connection incarnation at its single-use admission edge.
    pub fn can_submit_original(&self) -> bool {
        !self.task.is_closed()
            && !self.preparation.original.pin.is_revoked()
            && self.transport.supports_connection_leases()
            && self.transport.supports_connected_cancellable_requests()
            && self
                .transport
                .connection_lease(self.target)
                .is_some_and(|lease| lease.is_live())
    }
}

/// An unsuccessful pre-activation call returns the SAME source and actual T. It cannot consume
/// an invite, mint another package, create a runnable source or refresh the captured root.
pub struct ReturnedCheckpointJoinActivation<T> {
    preparation: JournalledCheckpointJoinPreparation,
    transport: T,
    error: AppError,
}
impl<T> std::fmt::Debug for ReturnedCheckpointJoinActivation<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReturnedCheckpointJoinActivation")
            .finish_non_exhaustive()
    }
}
impl<T> ReturnedCheckpointJoinActivation<T> {
    pub fn into_parts(self) -> (JournalledCheckpointJoinPreparation, T, AppError) {
        (self.preparation, self.transport, self.error)
    }
}

impl JournalledCheckpointJoinPreparation {
    /// Bind a saved Active source to ONE original transport and trusted finite-checkpoint
    /// endpoint. The host chooses that endpoint: a live peer is not proof of the invite signer,
    /// and this is not production authenticated/indirect routing. No network is performed.
    /// Exact saved source/file/root barriers and original admission→catalog CAS precede task
    /// registration. Ordinary Pending DATA opening never creates a worker automatically.
    // Preserve the original paid preparation and caller transport inline on every refusal;
    // allocation is not required to return an owner after failed admission.
    #[allow(clippy::result_large_err)]
    pub fn activate_original_transport<T: MeshTransport>(
        self,
        store: &ServerStore,
        transport: T,
        target: PeerId,
    ) -> Result<ActivatedCheckpointJoin<T>, ReturnedCheckpointJoinActivation<T>> {
        let prepared = (|| {
            if !transport.supports_connection_leases()
                || !transport.supports_connected_cancellable_requests()
                || self.metadata.scheduling != codec::Scheduling::Active
                || self.transition.is_some()
            {
                return Err(refused());
            }
            self.original
                .pin
                .validate_original(store, self.metadata.group.as_slice())?;
            self.creation.snapshot_limits(store)?;
            // An unsaved Local source does not become runnable/registered from labels. This
            // crosses real complete source barriers and refuses unknown pending candidates.
            self.confirm_existing(store)?;
            let policy = store.pending_join_policy.as_ref().ok_or_else(refused)?;
            let fee = policy.pool.try_reserve(
                ACTIVATION_METADATA + std::mem::size_of::<ActivatedCheckpointJoin<T>>(),
            )?;
            let mut predecessor = PreparedOriginalPredecessor::join_checkpoint(
                &self.original.capture,
                self.metadata.source,
                self.metadata.revision,
                &policy.pool,
            )?;
            let opened = store.inspect_pending_join_catalog()?;
            store.register_original_predecessor(
                &self.original.capture,
                &mut predecessor,
                |permit| {
                    #[cfg(test)]
                    {
                        // SAME bounded scheduling seam as source writes, after admission and
                        // immediately before the actual catalog wait. Release its own mutex.
                        let hook = store
                            .pending_join_before_catalog_writer
                            .lock()
                            .map_err(|_| refused())?
                            .take();
                        if let Some(hook) = hook {
                            hook();
                        }
                    }
                    let mounted = store.pending_join_catalog.lock().map_err(|_| refused())?;
                    let mounted = mounted.as_ref().ok_or_else(refused)?;
                    permit.validate_original()?;
                    self.original
                        .pin
                        .validate_original(store, self.metadata.group.as_slice())?;
                    if mounted.pending.is_some()
                        || mounted.current.digest != opened.selected.digest
                        || !mounted
                            .current
                            .rows
                            .iter()
                            .flatten()
                            .any(|row| row == &self.metadata)
                    {
                        return Err(refused());
                    }
                    permit.validate_original()?;
                    store.settle_pending_join_catalog(&opened)
                },
            )?;
            // No fallible operation/await after registration. The original sender moves once
            // to the typed task, never into Store metadata or a retained DATA/candidate owner.
            Ok((predecessor.into_task(), fee))
        })();
        match prepared {
            Ok((task, fee)) => Ok(ActivatedCheckpointJoin {
                preparation: self,
                transport,
                target,
                task,
                _prefix: PreOwnerConnectionHandoff::default(),
                _activation_fee: fee,
            }),
            Err(error) => Err(ReturnedCheckpointJoinActivation {
                preparation: self,
                transport,
                error,
            }),
        }
    }
}
