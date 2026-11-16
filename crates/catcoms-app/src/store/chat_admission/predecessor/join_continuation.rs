//! Exact original RetryGrant reservation. Saved catalog labels never reconstruct its task.
//! The caller owns the sole sender; Runtime/candidate/outcome own only paid inert metadata.
use super::*;
use crate::store::critical_namespaces::pending_join::PendingCandidate;
use std::sync::atomic::{AtomicBool, Ordering};

/// Borrowed syntax/equality DATA from an exactly classified RetryGrant candidate. This grants
/// no task or disk proof; every mutation below additionally matches original runtime custody.
pub(in crate::store) struct JoinRetryCandidateBinding<'a> {
    pub(in crate::store) group: &'a [u8],
    pub(in crate::store) signer: &'a [u8; 32],
    pub(in crate::store) source: [u8; 32],
    pub(in crate::store) previous_revision: u64,
    pub(in crate::store) revision: u64,
    pub(in crate::store) expected_frame: [u8; 32],
    pub(in crate::store) frame: [u8; 32],
}

/// Nonserialized original completion cell. Only the actual candidate writer can mark SavedData;
/// only same-owner finalization under original admission can mark Active. Neither flag alone
/// supplies a permit, and DATA repair never owns a sender, transport, keys or session.
pub(in crate::store) struct JoinContinuationOutcome {
    old_identity: Arc<()>,
    identity: Arc<()>,
    source: [u8; 32],
    previous_revision: u64,
    revision: u64,
    expected_frame: [u8; 32],
    frame: [u8; 32],
    saved: AtomicBool,
    active: AtomicBool,
    _fee: CatalogReadReservation,
}
impl std::fmt::Debug for JoinContinuationOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalJoinContinuationOutcome")
            .finish_non_exhaustive()
    }
}
impl JoinContinuationOutcome {
    pub(in crate::store) fn is_saved_data(&self) -> bool {
        self.saved.load(Ordering::Acquire)
    }
    pub(in crate::store) fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    pub(in crate::store) fn mark_saved_data(&self, frame: [u8; 32]) -> Result<(), AppError> {
        if frame != self.frame {
            return Err(refused());
        }
        self.saved.store(true, Ordering::Release);
        Ok(())
    }
    pub(in crate::store) fn validate_candidate(
        &self,
        candidate: &PendingCandidate,
    ) -> Result<(), AppError> {
        let data = candidate.retry_grant_binding()?;
        if data.source != self.source
            || data.previous_revision != self.previous_revision
            || data.revision != self.revision
            || data.expected_frame != self.expected_frame
            || data.frame != self.frame
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl PreparedOriginalPredecessor {
    /// Negative lifetime only, BEFORE actual reservation; cannot independently activate/send.
    pub(in crate::store) fn close_prepared_join(&self) {
        self.task.close();
    }
    pub(in crate::store) fn prepared_join_identity(&self) -> Arc<()> {
        self.task.owner.identity()
    }
    pub(in crate::store) fn prepared_join_observer(&self) -> RequestCancellation {
        self.task.observer()
    }
    pub(in crate::store) fn prepare_join_continuation(
        &mut self,
        previous: &OriginalPredecessorTask,
        candidate: &PendingCandidate,
        pool: &CriticalCatalogReadPool,
    ) -> Result<Arc<JoinContinuationOutcome>, AppError> {
        let data = candidate.retry_grant_binding()?;
        let next = self.entry.as_mut().ok_or_else(refused)?;
        if next.kind != Kind::JoinCheckpoint
            || next.key.group() != data.group
            || next.key.signer() != data.signer
            || next.source != data.source
            || next.revision != data.revision
            || self.task.is_closed()
            || self.continuation_claim.is_some()
            || next.continuation.is_some()
        {
            return Err(refused());
        }
        let fee = pool.try_reserve(std::mem::size_of::<JoinContinuationOutcome>() + 1024)?;
        let outcome = Arc::new(JoinContinuationOutcome {
            old_identity: previous.owner.identity(),
            identity: self.task.owner.identity(),
            source: data.source,
            previous_revision: data.previous_revision,
            revision: data.revision,
            expected_frame: data.expected_frame,
            frame: data.frame,
            saved: AtomicBool::new(false),
            active: AtomicBool::new(false),
            _fee: fee,
        });
        next.continuation = Some(outcome.clone());
        self.continuation_claim = Some(outcome.clone());
        Ok(outcome)
    }
}

impl Registry {
    pub(in crate::store::chat_admission) fn validate_continuation_data(
        &self,
        capture: &ChatAdmissionCapture,
        outcome: &Arc<JoinContinuationOutcome>,
    ) -> Result<(), AppError> {
        if self.slots.iter().flatten().any(|entry| {
            entry.kind == Kind::JoinCheckpoint
                && entry.key == capture.key
                && entry.original == capture.original
                && Arc::ptr_eq(&entry.mount, &capture.mount_identity)
                && entry.source == outcome.source
                && entry.revision == outcome.revision
                && Arc::ptr_eq(&entry.identity, &outcome.identity)
                && entry
                    .continuation
                    .as_ref()
                    .is_some_and(|original| Arc::ptr_eq(original, outcome))
        }) {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

impl ServerStore {
    /// FIRST RetryGrant admission atomically reserves its exact new owner and Store output.
    /// The new sender remains in caller-owned prepared. No candidate IO occurs until both
    /// moves are complete, so a competing real coalesced source cannot steal saved revision2.
    pub(in crate::store) fn reserve_original_join_retry_candidate(
        &self,
        previous: &OriginalPredecessorTask,
        prepared: &mut PreparedOriginalPredecessor,
        candidate: &mut Option<PendingCandidate>,
    ) -> Result<(), AppError> {
        let original = candidate.as_ref().ok_or_else(refused)?.original.clone();
        let outcome = prepared
            .continuation_claim
            .as_ref()
            .ok_or_else(refused)?
            .clone();
        let retired;
        {
            let mut runtime = self.chat_mount.locked()?;
            let permit = JournalAdmissionPermit::for_original(self, &original.capture, &runtime);
            permit.validate_original()?;
            let mut mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_mut().ok_or_else(refused)?;
            let pending = candidate.as_ref().ok_or_else(refused)?;
            permit.validate_capture(&pending.original.capture)?;
            original.pin.validate_original(self, original.pin.group())?;
            if mounted.pending.is_some()
                || !pending.matches_expected_catalog(&mounted.current)
                || pending.execution_task.is_some()
                || pending.received_input.is_some()
                || !pending
                    .continuation_outcome
                    .as_ref()
                    .is_some_and(|same| Arc::ptr_eq(same, &outcome))
                || !previous.is_closed()
                || prepared.task.is_closed()
            {
                return Err(refused());
            }
            outcome.validate_candidate(pending)?;
            let next = prepared.entry.as_ref().ok_or_else(refused)?;
            let index = runtime
                .predecessors
                .slots
                .iter()
                .position(|entry| {
                    entry
                        .as_ref()
                        .is_some_and(|entry| entry.key == original.capture.key)
                })
                .ok_or_else(refused)?;
            let old = runtime.predecessors.slots[index]
                .as_ref()
                .ok_or_else(refused)?;
            if old.kind != Kind::JoinCheckpoint
                || next.kind != Kind::JoinCheckpoint
                || next.key != original.capture.key
                || old.original != original.capture.original
                || next.original != original.capture.original
                || !Arc::ptr_eq(&old.mount, &original.capture.mount_identity)
                || !Arc::ptr_eq(&next.mount, &old.mount)
                || old.source != outcome.source
                || next.source != old.source
                || old.revision != outcome.previous_revision
                || next.revision != outcome.revision
                || !Arc::ptr_eq(&old.identity, &previous.owner.identity())
                || !Arc::ptr_eq(&old.identity, &outcome.old_identity)
                || !Arc::ptr_eq(&next.identity, &outcome.identity)
                || !old.negative.is_cancelled()
                || old
                    .continuation
                    .as_ref()
                    .is_some_and(|prior| !prior.is_active())
            {
                return Err(refused());
            }
            permit.validate_original()?;
            if prepared.task.is_closed() {
                return Err(refused());
            }
            let next = prepared.entry.take().ok_or_else(refused)?;
            // Every check/allocation precedes these two no-fail ownership moves. Retired
            // metadata and its actual quota owner are kept alive until BOTH guards drain.
            retired = runtime.predecessors.slots[index].replace(next);
            mounted.pending = candidate.take();
        }
        drop(retired);
        self.retry_pending_join_catalog()
    }

    /// Finalize ONLY the exact original reserved owner after actual SavedData. The concrete
    /// caller crosses current complete file/root/source barriers and publishes its SAME Shared
    /// under this callback; decoded revision/status can never enter this path. No await.
    pub(in crate::store) fn finalize_original_join_retry(
        &self,
        capture: &ChatAdmissionCapture,
        prepared: &PreparedOriginalPredecessor,
        confirm: impl FnOnce(&JournalAdmissionPermit<'_>) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let runtime = self.chat_mount.locked()?;
        let permit = JournalAdmissionPermit::for_original(self, capture, &runtime);
        permit.validate_original()?;
        let outcome = prepared.continuation_claim.as_ref().ok_or_else(refused)?;
        runtime
            .predecessors
            .validate_continuation_data(capture, outcome)?;
        if prepared.entry.is_some()
            || prepared.task.is_closed()
            || !outcome.saved.load(Ordering::Acquire)
            || !Arc::ptr_eq(&prepared.task.owner.identity(), &outcome.identity)
        {
            return Err(refused());
        }
        confirm(&permit)?;
        permit.validate_original()?;
        if prepared.task.is_closed() {
            return Err(refused());
        }
        outcome.active.store(true, Ordering::Release);
        Ok(())
    }
}
