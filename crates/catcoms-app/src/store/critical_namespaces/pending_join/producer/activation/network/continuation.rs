//! Exact caller-owned RetryGrant continuation. Store sees only weak reservation + DATA.
use super::*;
use crate::store::chat_admission::predecessor::{
    JoinContinuationOutcome, PreparedOriginalPredecessor,
};
use catcoms_sync::PreparedOriginalJoinContinuation;

/// All optional owners stay HERE on every error. In particular a saved r2 cannot lose its
/// unique new sender and then attempt same-revision renewal from a coalesced source.
pub(super) struct RetryContinuation {
    limit: u32,
    target: codec::Metadata,
    candidate: Option<PendingCandidate>,
    prepared: Option<PreparedOriginalPredecessor>,
    sync: Option<PreparedOriginalJoinContinuation>,
    next: Option<Arc<Shared>>,
    outcome: Option<Arc<JoinContinuationOutcome>>,
    registered: bool,
    complete: bool,
    fee: Arc<CatalogReadReservation>,
}

impl RetryContinuation {
    pub(super) fn close(&self) {
        if let Some(prepared) = &self.prepared {
            prepared.close_prepared_join();
        }
        if let Some(next) = &self.next {
            next.cell.close();
        }
    }
}

impl<T: MeshTransport> CheckpointJoinNetwork<T> {
    /// Explicit local retry grant only. It never runs automatically after timeout/restart.
    /// Repeated/lost returns recover ONE retained target/result; a changed limit refuses.
    /// Exhaustion/cancellation does not refresh the old task: saved revision+new identity are
    /// reserved with the exact candidate BEFORE r2 can be observed by another genuine caller.
    pub fn ensure_retry_grant(
        &mut self,
        store: &ServerStore,
        limit: u32,
        rng: &mut impl CryptoRngCore,
    ) -> Result<CheckpointJoinSubmissionStatus, AppError> {
        let settings = store
            .pending_join_policy
            .as_ref()
            .and_then(|policy| policy.execution)
            .ok_or_else(refused)?;
        super::super::super::super::execution_policy::Settings::new(limit, settings.timeout_ms)?;
        if let Some(original) = &self.retry {
            if original.limit != limit {
                return Err(refused());
            }
            if original.complete {
                return Ok(self.status());
            }
        } else {
            self.prepare_retry_request(store, limit)?;
        }
        let mut retry = self.retry.take().ok_or_else(refused)?;
        // Do NOT use ? between taking this owner and putting it back. A fallible registration/
        // attachment must retain the SAME new sender/cell/claim, never drop and recreate it.
        let result = self.advance_retry_grant(store, &mut retry, rng);
        self.retry = Some(retry);
        result
    }

    /// Start a DIFFERENT intentional retry action only after the prior one completed. This
    /// releases its cached response, not saved units/source/current task or a bind claim.
    pub fn next_retry_grant(&mut self) -> Result<(), AppError> {
        if !self.retry.as_ref().is_some_and(|retry| retry.complete) {
            return Err(refused());
        }
        let retired = self.retry.take();
        drop(retired); // No admission/catalog/cell writer is held.
        Ok(())
    }

    fn prepare_retry_request(&mut self, store: &ServerStore, limit: u32) -> Result<(), AppError> {
        match self.shared.cell.try_status() {
            Some(OriginalJoinNetworkStatus::InputHeld | OriginalJoinNetworkStatus::Submitting)
            | None => return Err(refused()),
            _ => {}
        }
        store.retry_pending_join_catalog()?;
        let current = store.pending_join_summary()?;
        let mut invocation = self.shared.invocation.try_lock().map_err(|_| refused())?;
        let mut preparation = self
            .shared
            .control
            .preparation
            .lock()
            .map_err(|_| refused())?;
        let row = current
            .rows
            .iter()
            .flatten()
            .find(|row| row.source == self.shared.source)
            .ok_or_else(refused)?;
        if row != &preparation.metadata {
            if !preparation
                .transition
                .as_ref()
                .is_some_and(|target| target == row)
                || invocation
                    .as_ref()
                    .is_some_and(|original| &original.target != row)
            {
                return Err(refused());
            }
            // Only this SAME genuinely proposed result may repair a lost local assignment.
            // Matching parsed labels alone never create an arm/ticket or new capture.
            preparation.metadata = row.clone();
            preparation.transition = None;
            preparation.expected = current.clone();
        } else if preparation.transition.is_some() {
            // Store has no pending candidate and current is STILL the original metadata:
            // this particular local proposal never reached first admission. No unit refund.
            preparation.transition = None;
        }
        preparation.confirm_existing(store)?;
        let mut state = preparation.metadata.execution.ok_or_else(refused)?;
        if state.grant == 0
            || state.input.is_some()
            || preparation.metadata.scheduling != codec::Scheduling::Active
        {
            return Err(refused());
        }
        let mut target = preparation.metadata.clone();
        state.grant = state.grant.checked_add(1).ok_or_else(refused)?;
        state.limit = limit;
        state.spent = 0;
        target.revision = target.revision.checked_add(1).ok_or_else(refused)?;
        target.execution = Some(state);
        let pool = &store.pending_join_policy.as_ref().ok_or_else(refused)?.pool;
        // Named fixed container/layout invoice. Large candidate bytes keep WRITE_FEE and the
        // immutable source/input buffers retain their SAME original strong resource owners.
        let fee = Arc::new(pool.try_reserve(
            std::mem::size_of::<RetryContinuation>()
                + std::mem::size_of::<Control>()
                + std::mem::size_of::<Shared>()
                + std::mem::size_of::<RetainedOriginalJoinNetwork>()
                + std::mem::size_of::<PreparedOriginalJoinContinuation>()
                + 4096,
        )?);
        self.task.close();
        self.shared.cell.close();
        let retired = invocation.take();
        drop(preparation);
        drop(invocation);
        drop(retired);
        // Reconcile old upper tickets outside all writers; actual lower driver clones retain
        // their original debit independently, until completion. Persistent spent stays spent.
        self.shared.retire_closed_unstarted();
        let _ = self.shared.cell.try_status();
        self.retry = Some(Box::new(RetryContinuation {
            limit,
            target,
            candidate: None,
            prepared: None,
            sync: None,
            next: None,
            outcome: None,
            registered: false,
            complete: false,
            fee,
        }));
        Ok(())
    }

    fn advance_retry_grant(
        &mut self,
        store: &ServerStore,
        retry: &mut RetryContinuation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<CheckpointJoinSubmissionStatus, AppError> {
        if retry.complete {
            return Ok(self.status());
        }
        let policy = store.pending_join_policy.as_ref().ok_or_else(refused)?;
        if retry.prepared.is_none() {
            let preparation = self
                .shared
                .control
                .preparation
                .lock()
                .map_err(|_| refused())?;
            let prepared = PreparedOriginalPredecessor::join_checkpoint(
                &preparation.original.capture,
                self.shared.source,
                retry.target.revision,
                &policy.pool,
            )?;
            retry.prepared = Some(prepared);
        }
        if retry.candidate.is_none() && retry.outcome.is_none() {
            let preparation = self
                .shared
                .control
                .preparation
                .lock()
                .map_err(|_| refused())?;
            retry.candidate =
                Some(preparation.prepare_retry_grant_candidate(store, &retry.target, rng)?);
        }
        if retry.outcome.is_none() {
            let candidate = retry.candidate.as_mut().ok_or_else(refused)?;
            let prepared = retry.prepared.as_mut().ok_or_else(refused)?;
            let outcome =
                prepared.prepare_join_continuation(&self.task, candidate, &policy.pool)?;
            candidate.continuation_outcome = Some(outcome.clone());
            retry.outcome = Some(outcome);
        }
        if retry.next.is_none() {
            let prepared = retry.prepared.as_ref().ok_or_else(refused)?;
            let control = Arc::new(Control {
                preparation: self.shared.control.preparation.clone(),
                identity: prepared.prepared_join_identity(),
                binding: self.shared.control.binding,
                request: self.shared.control.request,
                negative: prepared.prepared_join_observer(),
                data_negative: self.shared.control.data_negative.clone(),
                floor: retry.target.execution.ok_or_else(refused)?.ordinal,
                target: self.shared.control.target,
                _input_fee: self.shared.control._input_fee.clone(),
                _continuation_fee: Some(retry.fee.clone()),
            });
            // This moves no T/prefix and reserves the new cell's bind claim to this Bound.
            // Refusal preserves its old source; success stays retained before any writer IO.
            let sync = self.bound.prepare_original_continuation(control.clone())?;
            let cell = sync.retained_cell_data();
            retry.next = Some(Arc::new(Shared {
                source: self.shared.source,
                revision: retry.target.revision,
                control,
                cell,
                invocation: Mutex::new(None),
            }));
            retry.sync = Some(sync);
        }
        let outcome = retry.outcome.as_ref().ok_or_else(refused)?.clone();
        if retry.candidate.is_some() {
            store.reserve_original_join_retry_candidate(
                &self.task,
                retry.prepared.as_mut().ok_or_else(refused)?,
                &mut retry.candidate,
            )?;
        } else if !outcome.is_saved_data() {
            // Candidate already belongs to Store; repair never remints the new identity or
            // makes it Active. Original source/task errors leave this same owner for retry.
            store.retry_pending_join_catalog()?;
        }
        if !outcome.is_saved_data() {
            return Err(refused());
        }
        let next = retry.next.as_ref().ok_or_else(refused)?.clone();
        if !retry.registered {
            let mut preparation = self
                .shared
                .control
                .preparation
                .lock()
                .map_err(|_| refused())?;
            preparation.metadata = retry.target.clone();
            preparation.transition = None;
            preparation.expected = store.pending_join_summary()?;
            let opened = store.inspect_pending_join_catalog()?;
            let mut retired = None;
            store.finalize_original_join_retry(
                &preparation.original.capture,
                retry.prepared.as_ref().ok_or_else(refused)?,
                |permit| {
                    let mut mounted = store.pending_join_catalog.lock().map_err(|_| refused())?;
                    let mounted = mounted.as_mut().ok_or_else(refused)?;
                    permit.validate_original()?;
                    preparation
                        .original
                        .pin
                        .validate_original(store, preparation.metadata.group.as_slice())?;
                    if mounted.pending.is_some()
                        || mounted.current.digest != opened.selected.digest
                        || !mounted
                            .current
                            .rows
                            .iter()
                            .flatten()
                            .any(|row| row == &retry.target)
                        || next.control.negative.is_cancelled()
                    {
                        return Err(refused());
                    }
                    let index = mounted
                        .networks
                        .iter()
                        .position(|entry| {
                            entry.as_ref().is_some_and(|old| old.source == next.source)
                        })
                        .ok_or_else(refused)?;
                    let old = mounted.networks[index].as_ref().ok_or_else(refused)?;
                    if !Arc::ptr_eq(old, &self.shared) && !Arc::ptr_eq(old, &next) {
                        return Err(refused());
                    }
                    // Complete actual file/root/policy checks BEFORE publishing this same Shared.
                    store.settle_pending_join_catalog(&opened)?;
                    permit.validate_original()?;
                    if next.control.negative.is_cancelled() {
                        return Err(refused());
                    }
                    if !Arc::ptr_eq(old, &next) {
                        retired = mounted.networks[index].replace(next.clone());
                    }
                    Ok(())
                },
            )?;
            drop(preparation);
            drop(retired);
            // The registered SAME prepared owner is cached BEFORE fallible Sync attachment.
            retry.registered = true;
        }
        let sync = retry.sync.take().ok_or_else(refused)?;
        let prepared = match retry.prepared.take() {
            Some(original) => original,
            None => {
                retry.sync = Some(sync);
                return Err(refused());
            }
        };
        if let Err(returned) = self.bound.attach_original_continuation(sync) {
            let (original, _) = returned.into_parts();
            retry.sync = Some(original);
            retry.prepared = Some(prepared);
            return Err(refused());
        }
        // No await/fallible check follows attachment. Same sender moves once, old owners
        // retire outside every writer. A refused attachment restored BOTH original owners.
        let task = prepared.into_task();
        let old_task = std::mem::replace(&mut self.task, task);
        let old_shared = std::mem::replace(&mut self.shared, next);
        retry.complete = true;
        drop(old_task);
        drop(old_shared);
        Ok(self.status())
    }
}
