//! One cancellable-owner-safe recovery wait. Network bytes own no mutable group state.
use super::*;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::Poll;

#[derive(Clone, Copy)]
enum Shape {
    Since,
    Full,
    Commits,
}

pub(super) struct PendingCatchup {
    task: CatchupTask,
    peer: PeerId,
    auth: RequestAuth,
    shape: Shape,
    doc_version: u64,
    /// A newer authenticated frontier arrived after this wait was prepared. Its older empty
    /// answer may still import valid bytes, but cannot discharge that later obligation.
    completeness_dirty: bool,
    member: Option<DeviceId>,
    cursor: Option<CatchupCursor>,
    pub(super) wait_started: Arc<std::sync::OnceLock<u64>>,
    pub(super) within_ms: u64,
    pub(super) kind: u8,
    pub(super) response: Pin<Box<dyn Future<Output = Result<Bytes, SyncError>> + Send>>,
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    fn forget_checked_source(&mut self, doc_type: DocType, doc_id: u128, peer: PeerId) {
        if let Some((_, checked)) = self.catchup_sources_checked.get_mut(&(doc_type, doc_id)) {
            checked.remove(&peer);
        }
    }

    /// A verified current member's unknown hashes justify one bounded reverse read, not trust
    /// in the claimed operations. The normal response signature, membership, ciphertext and
    /// signed-operation checks still decide what may be imported. No document or route is opened.
    pub(super) fn note_catchup_frontier_hint(
        &mut self,
        from: PeerId,
        requester: DeviceId,
        epoch: u64,
        doc_type: DocType,
        doc_id: u128,
        heads: &[[u8; 32]],
    ) {
        if epoch != self.group.epoch()
            || !self.group.is_active()
            || !self.group.contains_device(&self.device.device_id())
            || !self.group.contains_device(&requester)
            || !self.peer_is_connected(from)
            || self
                .docs
                .get(&(doc_type, doc_id))
                .is_none_or(|doc| doc.knows_changes(heads))
        {
            return;
        }
        let now = self.clock.monotonic_ms();
        let Some(next) = now.checked_add(CATCHUP_PEER_COOLDOWN_MS) else {
            return;
        };
        let key = (doc_type, doc_id, requester);
        if self
            .catchup_frontier_hints
            .get(&key)
            .is_some_and(|until| now < *until)
        {
            return;
        }
        if !self.catchup_frontier_hints.contains_key(&key)
            && self.catchup_frontier_hints.len() >= MAX_CATCHUP_STALL_ENTRIES
        {
            self.catchup_frontier_hints.retain(|_, until| *until > now);
            if self.catchup_frontier_hints.len() >= MAX_CATCHUP_STALL_ENTRIES {
                return;
            }
        }
        self.catchup_frontier_hints.insert(key, next);
        self.forget_checked_source(doc_type, doc_id, from);
        if let Some(wait) = self.pending_catchup.as_mut().filter(|wait| {
            wait.task == CatchupTask::Doc { doc_type, doc_id }
                && (wait.peer == from || wait.member == Some(requester))
        }) {
            // Kept on the existing owner, so expiry/pruning of the hint budget cannot erase it.
            wait.completeness_dirty = true;
        }
        self.enqueue_doc_catchup(doc_type, doc_id);
    }

    pub(super) fn start_queued_catchup(&mut self)
    where
        T: 'static,
    {
        if self.pending_catchup.as_ref().is_some_and(|pending| {
            pending.auth.epoch != self.group.epoch()
                || !self.group.is_active()
                || !self.group.contains_device(&self.device.device_id())
        }) {
            let stale = self.pending_catchup.take().expect("checked pending wait");
            self.requeue_catchup(stale.task);
            self.catchup_inflight = None;
        }
        if self.pending_catchup.is_some()
            || !self.group.is_active()
            || !self.group.contains_device(&self.device.device_id())
        {
            return;
        }
        let Some((task, peer)) = self.select_queued_catchup() else {
            return;
        };
        let shape = match task {
            CatchupTask::Commits { .. } => Shape::Commits,
            CatchupTask::Doc { .. } => Shape::Since,
        };
        if let Err(error) = self.begin_catchup_wait(task, peer, shape) {
            tracing::debug!(?task, ?peer, %error, "could not prepare queued catch-up");
            self.requeue_catchup(task);
            self.catchup_inflight = None;
        }
    }

    fn begin_catchup_wait(
        &mut self,
        task: CatchupTask,
        peer: PeerId,
        shape: Shape,
    ) -> Result<(), SyncError>
    where
        T: 'static,
    {
        let (kind, body, doc_version, cursor, within_ms) = match task {
            CatchupTask::Commits { from_epoch, .. } => (
                KIND_COMMIT_CATCHUP,
                encode_commit_catchup_req(from_epoch),
                0,
                None,
                CATCHUP_REQUEST_MS,
            ),
            CatchupTask::Doc { doc_type, doc_id } => {
                let cursor = self.catchup_cursors.get(&(doc_type, doc_id, peer)).copied();
                let version = self.doc_version(doc_type, doc_id);
                if matches!(shape, Shape::Full) {
                    (
                        KIND_CATCHUP,
                        encode_catchup_req(doc_type, doc_id),
                        version,
                        cursor,
                        FULL_CATCHUP_REQUEST_MS,
                    )
                } else {
                    let heads = self
                        .docs
                        .get_mut(&(doc_type, doc_id))
                        .map(|doc| doc.sync_frontier(MAX_CATCHUP_SINCE_HEADS))
                        .unwrap_or_default();
                    (
                        KIND_CATCHUP_SINCE,
                        encode_catchup_since_req(doc_type, doc_id, &heads, cursor.as_ref()),
                        version,
                        cursor,
                        CATCHUP_REQUEST_MS,
                    )
                }
            }
        };
        let (request, auth) = self.build_authed_request(kind, &body)?;
        match task {
            CatchupTask::Commits { .. } => self.stats.commit_catchups_requested += 1,
            CatchupTask::Doc { .. } => self.stats.doc_catchups_requested += 1,
        }
        let transport = Arc::clone(&self.transport);
        let clock = Arc::clone(&self.clock);
        let wait_started = Arc::new(std::sync::OnceLock::new());
        let observed_start = wait_started.clone();
        self.pending_catchup = Some(PendingCatchup {
            task,
            peer,
            auth,
            shape,
            doc_version,
            completeness_dirty: false,
            member: self
                .member_peers
                .iter()
                .find(|proof| proof.peer == peer && proof.bound)
                .map(|proof| proof.device),
            cursor,
            wait_started,
            within_ms,
            kind,
            response: Box::pin(async move {
                // Diagnostic first-poll time only: neither projection reads nor cancelled outer
                // owner polls may renew this retained wait. It is not a wire handoff receipt.
                let _ = observed_start.set(clock.monotonic_ms());
                Self::request_within_deadline(
                    &transport,
                    &*clock,
                    peer,
                    request,
                    "queued catch-up",
                    within_ms,
                )
                .await
            }),
        });
        Ok(())
    }

    /// Returns `None` after local recovery work, and `Some(None)` only on transport closure.
    /// An actor command can drop this *poll*, but cannot drop the request/nonce/response owner.
    pub(super) async fn next_recovery_or_event(&mut self) -> Option<Option<TransportEvent>>
    where
        T: 'static,
    {
        enum Ready {
            Response(Result<Bytes, SyncError>),
            Submission(super::recovery_submission::Completion),
            FileSource(super::file_service::service::Completion),
            Event(Option<TransportEvent>),
            Retry,
            Capacity,
            Reconciliation,
        }
        if self.bound_source_reconciliation_owed {
            // Authentication can complete after the connection edge that declined recovery.
            // Drain one bounded rotating pass without waiting for the periodic fallback. The
            // obligation and queue survive outer-poll cancellation; current sources are checked
            // again by scheduling and by the request selector.
            self.schedule_reconciliation();
            self.start_queued_catchup();
        }
        // A pending request has its own finite deadline. An unrelated expired cooldown must
        // not busy-loop while that single slot is occupied.
        let catchup_retry = if self.pending_catchup.is_some() || !self.group.is_active() {
            None
        } else {
            self.next_catchup_retry_delay()
        };
        let retry_delay = [catchup_retry, self.next_durable_chat_retry_delay()]
            .into_iter()
            .flatten()
            .min();
        let reconciliation_delay = self.next_reconciliation_delay();
        let transport = Arc::clone(&self.transport);
        let clock = Arc::clone(&self.clock);
        let event = transport.next_event();
        let retry = async {
            match retry_delay {
                Some(ms) => clock.sleep(std::time::Duration::from_millis(ms)).await,
                None => std::future::pending().await,
            }
        };
        let reconciliation = async {
            match reconciliation_delay {
                Some(ms) => clock.sleep(std::time::Duration::from_millis(ms)).await,
                None => std::future::pending().await,
            }
        };
        let mut changed = self.recovery_submissions.changed.clone();
        let waiting_submission =
            !self.repair_outbox.is_empty() || !self.delivery_receipt_outbox.is_empty();
        let capacity = async {
            if waiting_submission {
                let _ = changed.changed().await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        futures::pin_mut!(event, retry, capacity, reconciliation);
        let ready = poll_fn(|cx| {
            // One bounded local turn per interval precedes ready I/O, so sustained incoming
            // traffic cannot starve quiet anti-entropy. Advance the shared deadline before
            // reporting readiness; cancelled caller polls retain that exact schedule.
            if reconciliation.as_mut().poll(cx).is_ready() {
                self.schedule_reconciliation();
                return Poll::Ready(Ready::Reconciliation);
            }
            // Rotate catch-up, inbound events, detached submissions and file completions. This poll
            // may be cancelled by an actor command; the jobs/permits remain on the group owner.
            for offset in 0..4 {
                let source = (self.recovery_turn + offset) % 4;
                if source == 0 {
                    if let Some(pending) = self.pending_catchup.as_mut() {
                        if let Poll::Ready(result) = pending.response.as_mut().poll(cx) {
                            self.recovery_turn = 1;
                            return Poll::Ready(Ready::Response(result));
                        }
                    }
                } else if source == 1 {
                    if let Poll::Ready(value) = event.as_mut().poll(cx) {
                        self.recovery_turn = 2;
                        return Poll::Ready(Ready::Event(value));
                    }
                } else if source == 2 && !self.recovery_submissions.jobs.is_empty() {
                    use futures::Stream;
                    if let Poll::Ready(Some(completed)) =
                        Pin::new(&mut self.recovery_submissions.jobs).poll_next(cx)
                    {
                        self.recovery_turn = 3;
                        return Poll::Ready(Ready::Submission(completed));
                    }
                } else if source == 3 && !self.file_service.jobs.is_empty() {
                    use futures::Stream;
                    if let Poll::Ready(Some(completed)) =
                        Pin::new(&mut self.file_service.jobs).poll_next(cx)
                    {
                        self.recovery_turn = 0;
                        return Poll::Ready(Ready::FileSource(completed));
                    }
                }
            }
            if capacity.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Ready::Capacity);
            }
            retry.as_mut().poll(cx).map(|()| Ready::Retry)
        })
        .await;
        match ready {
            Ready::Response(response) => {
                self.complete_queued_catchup(response);
                None
            }
            Ready::Event(event) => Some(event),
            Ready::Submission(completed) => {
                self.complete_recovery_submission(completed);
                None
            }
            Ready::FileSource(completed) => {
                self.complete_file_service(completed);
                None
            }
            Ready::Capacity => {
                self.recovery_submissions.changed.borrow_and_update();
                None
            }
            Ready::Retry => None,
            Ready::Reconciliation => None,
        }
    }

    pub(super) fn complete_queued_catchup(&mut self, response: Result<Bytes, SyncError>)
    where
        T: 'static,
    {
        let pending = self
            .pending_catchup
            .take()
            .expect("a response has its owner");
        let PendingCatchup {
            task,
            peer,
            auth,
            shape,
            doc_version,
            completeness_dirty,
            cursor,
            ..
        } = pending;
        // Commands and inbound commits may change the world while bytes are in flight. A
        // stale result grants no proof, cursor or completion; retain its recovery obligation.
        let stale = auth.epoch != self.group.epoch()
            || !self.group.is_active()
            || !self.group.contains_device(&self.device.device_id())
            || matches!(task, CatchupTask::Doc { doc_type, doc_id }
                if self.catchup_cursors.get(&(doc_type, doc_id, peer)).copied() != cursor);
        if stale {
            self.requeue_catchup(task);
            self.catchup_inflight = None;
            return;
        }
        match task {
            CatchupTask::Commits {
                from_epoch, gap_at, ..
            } => {
                let before = self.group.epoch();
                let outcome = response
                    .and_then(|bytes| self.apply_commit_catchup_response(peer, auth, &bytes))
                    .unwrap_or(CommitCatchupOutcome::Unanswered);
                self.finish_queued_commits(peer, from_epoch, gap_at, before, outcome);
            }
            CatchupTask::Doc { doc_type, doc_id } => {
                let moved = self.doc_version(doc_type, doc_id) != doc_version;
                let result = response.and_then(|bytes| match shape {
                    Shape::Since => {
                        self.apply_catchup_since_response(peer, doc_type, doc_id, auth, &bytes)
                    }
                    Shape::Full => self
                        .apply_full_catchup_response(peer, doc_type, doc_id, &bytes)
                        .map(Some),
                    Shape::Commits => unreachable!(),
                });
                match result {
                    Ok(None) => {
                        // Compatibility keeps its existing larger finite deadline, but no
                        // longer monopolizes or loses its wait on the mutable actor owner.
                        if self.begin_catchup_wait(task, peer, Shape::Full).is_ok() {
                            self.pending_catchup
                                .as_mut()
                                .expect("new full wait")
                                .completeness_dirty = completeness_dirty;
                            return;
                        }
                        self.requeue_catchup(task);
                    }
                    Err(error) => {
                        tracing::debug!(?task, ?peer, %error, "queued catch-up failed");
                        self.cool_off_catchup_peer(peer, doc_type, doc_id);
                        self.requeue_catchup(task);
                    }
                    Ok(Some(_)) => {}
                }
                if completeness_dirty {
                    // The source's old reply cannot overwrite a later authenticated hint. Only
                    // this exact source's answer is invalidated; unrelated completeness, failure
                    // cooldowns, original cursors and continuation ownership remain unchanged.
                    self.forget_checked_source(doc_type, doc_id, peer);
                    self.requeue_catchup(task);
                }
                if moved {
                    // A local edit during the wait is not evidence that this source answered
                    // about that new frontier. Accept valid history, then reopen the sweep.
                    self.clear_sources_checked(doc_type, doc_id);
                    self.requeue_catchup(task);
                }
            }
        }
        self.catchup_inflight = None;
    }

    fn finish_queued_commits(
        &mut self,
        peer: PeerId,
        from_epoch: u64,
        gap_at: Option<u64>,
        before: u64,
        outcome: CommitCatchupOutcome,
    ) {
        let here = self.group.epoch();
        let progressed = here > before;
        let closed = outcome.answered()
            && !matches!(outcome, CommitCatchupOutcome::Stranded { .. })
            && self.pending_commits.is_empty()
            && gap_at.is_none_or(|gap| here >= gap);
        tracing::debug!(
            ?peer,
            ?outcome,
            from_epoch,
            gap_at,
            epoch_before = before,
            epoch_after = here,
            closed,
            "commit catch-up finished"
        );
        if progressed {
            self.failed_catchup_peers.clear();
        }
        if !closed {
            if !progressed {
                self.note_failed_catchup_peer(peer);
            }
            self.enqueue_commit_catchup_for(here, gap_at, None);
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod frontier_tests;
