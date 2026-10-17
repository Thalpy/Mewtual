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
    cursor: Option<CatchupCursor>,
    pub(super) response: Pin<Box<dyn Future<Output = Result<Bytes, SyncError>> + Send>>,
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
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
        self.pending_catchup = Some(PendingCatchup {
            task,
            peer,
            auth,
            shape,
            doc_version,
            cursor,
            response: Box::pin(async move {
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
            Event(Option<TransportEvent>),
            Retry,
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
        let transport = Arc::clone(&self.transport);
        let clock = Arc::clone(&self.clock);
        let event = transport.next_event();
        let retry = async {
            match retry_delay {
                Some(ms) => clock.sleep(std::time::Duration::from_millis(ms)).await,
                None => std::future::pending().await,
            }
        };
        futures::pin_mut!(event, retry);
        let ready = poll_fn(|cx| {
            // Alternate ready completions and inbound events. A peer flooding either side
            // cannot indefinitely suppress the other; a pending branch registers its waker.
            for response_first in [self.prefer_catchup_response, !self.prefer_catchup_response] {
                if response_first {
                    if let Some(pending) = self.pending_catchup.as_mut() {
                        if let Poll::Ready(result) = pending.response.as_mut().poll(cx) {
                            self.prefer_catchup_response = false;
                            return Poll::Ready(Ready::Response(result));
                        }
                    }
                } else if let Poll::Ready(value) = event.as_mut().poll(cx) {
                    self.prefer_catchup_response = true;
                    return Poll::Ready(Ready::Event(value));
                }
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
            Ready::Retry => None,
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
