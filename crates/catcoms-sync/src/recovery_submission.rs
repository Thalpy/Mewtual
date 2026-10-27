//! Immutable automatic recovery submissions. The sole group owner plans and signs; these
//! finite futures hold transport waits without borrowing it. Driver admission repeats authority.
use super::*;
use catcoms_discovery::EndpointBudgetWindow;
use catcoms_rt::{BoxedDialPermit, ConnectionLease, DialPermit, RequestAuthority};
use futures::{future::BoxFuture, stream::FuturesUnordered};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock, Weak,
};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

const LOCAL_SUBMISSIONS: usize = 32;
const PROCESS_SUBMISSIONS: usize = 64;
const SUBMISSION_MS: u64 = 10_000;

struct Capacity {
    slots: Arc<Semaphore>,
    changed: watch::Sender<u64>,
}
fn capacity() -> Arc<Capacity> {
    static CAPACITY: OnceLock<Arc<Capacity>> = OnceLock::new();
    CAPACITY
        .get_or_init(|| {
            Arc::new(Capacity {
                slots: Arc::new(Semaphore::new(PROCESS_SUBMISSIONS)),
                changed: watch::channel(0).0,
            })
        })
        .clone()
}
#[derive(Debug)]
struct Flight {
    local: Option<OwnedSemaphorePermit>,
    process: Option<OwnedSemaphorePermit>,
    changed: watch::Sender<u64>,
}
impl Drop for Flight {
    fn drop(&mut self) {
        self.local.take();
        self.process.take();
        self.changed
            .send_modify(|value| *value = value.wrapping_add(1));
    }
}
#[derive(PartialEq, Eq)]
struct View {
    epoch: u64,
    policy: Option<[u8; 32]>,
    revision: u64,
    active_policy: bool,
    active_member: bool,
}
struct Gate {
    view: View,
    refs: Vec<DescriptorRef>,
    connected: Option<(PeerId, Option<ConnectionLease>)>,
    dial: Option<PeerId>,
    live: watch::Sender<bool>,
    clock: Arc<dyn Clock + Send>,
    deadline: u64,
    expires: u64,
}
impl fmt::Debug for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoverySubmissionAuthority { .. }")
    }
}
impl Gate {
    fn revoke(&self) {
        self.live.send_replace(false);
    }
}
#[async_trait::async_trait]
impl RequestAuthority for Gate {
    fn authorized(&self) -> bool {
        *self.live.borrow()
            && self.clock.monotonic_ms() < self.deadline
            && self.clock.now_ms() <= self.expires
            && self
                .connected
                .as_ref()
                .is_none_or(|(_, lease)| lease.as_ref().is_none_or(ConnectionLease::is_live))
    }
    async fn revoked(&self) {
        let mut live = self.live.subscribe();
        loop {
            if !self.authorized() {
                return;
            }
            let remaining = self.deadline.saturating_sub(self.clock.monotonic_ms()).min(
                self.expires
                    .saturating_sub(self.clock.now_ms())
                    .saturating_add(1),
            );
            tokio::select! {
                result = live.changed() => if result.is_err() { return; },
                _ = self.clock.sleep(std::time::Duration::from_millis(remaining)) => {},
                _ = async {
                    if let Some((_, Some(lease))) = &self.connected { lease.revoked().await; }
                    else { std::future::pending::<()>().await; }
                } => return,
            }
        }
    }
}
/// Capacity here bounds queued submissions/acknowledgements, not established sockets. Dials
/// retain their existing driver socket gates; notifications retain Flight through lower RR end.
struct GuardedPermit {
    inner: BoxedDialPermit,
    gate: Arc<Gate>,
    spent: Arc<AtomicBool>,
    _flight: Arc<Flight>,
}
impl fmt::Debug for GuardedPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryDialPermit { .. }")
    }
}
impl DialPermit for GuardedPermit {
    fn address(&self) -> &str {
        self.inner.address()
    }
    fn commit_if_current(self: Box<Self>) -> Option<String> {
        if !self.gate.authorized() {
            return None;
        }
        let address = self.inner.commit_if_current()?;
        self.spent.store(true, Ordering::Release);
        Some(address)
    }
}
pub(super) struct Completion {
    gate: Arc<Gate>,
    window: Option<EndpointBudgetWindow>,
    tally: DialOutcomeTally,
    epochs: Vec<(FreshnessPrincipal, u64)>,
    _flight: Arc<Flight>,
}
/// Capacity pressure preserves an outbox item; stale authority must discard it so one
/// retired helper cannot block unrelated current forwarding at the head of the queue.
pub(super) enum Admission {
    Busy,
    Stale,
}
pub(super) struct Submissions {
    active: Vec<Weak<Gate>>,
    local: Arc<Semaphore>,
    capacity: Arc<Capacity>,
    pub(super) jobs: FuturesUnordered<BoxFuture<'static, Completion>>,
    pub(super) changed: watch::Receiver<u64>,
}
impl Default for Submissions {
    fn default() -> Self {
        Self::with_capacity(capacity())
    }
}
impl Submissions {
    fn with_capacity(capacity: Arc<Capacity>) -> Self {
        Self {
            active: Vec::new(),
            local: Arc::new(Semaphore::new(LOCAL_SUBMISSIONS)),
            changed: capacity.changed.subscribe(),
            capacity,
            jobs: FuturesUnordered::new(),
        }
    }
}
impl Drop for Submissions {
    fn drop(&mut self) {
        for gate in self.active.iter().filter_map(Weak::upgrade) {
            gate.revoke();
        }
    }
}
impl Submissions {
    fn reserve(&self) -> Option<Arc<Flight>> {
        let local = self.local.clone().try_acquire_owned().ok()?;
        let process = self.capacity.slots.clone().try_acquire_owned().ok()?;
        Some(Arc::new(Flight {
            local: Some(local),
            process: Some(process),
            changed: self.capacity.changed.clone(),
        }))
    }
}

#[cfg(test)]
mod tests;
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    fn recovery_submission_view(&self) -> View {
        View {
            epoch: self.group.epoch(),
            policy: self.group_policy_digest(),
            revision: self.group_policy_revision,
            active_policy: self.group_policy_active,
            active_member: self.group.is_active()
                && self.group.contains_device(&self.device.device_id()),
        }
    }
    /// Called synchronously by the existing MLS/policy/descriptor/proof observation seam before
    /// an await. A removal and readmission permanently revokes the old job, even if refs match.
    pub(super) fn reconcile_recovery_submissions(&mut self) {
        if self.recovery_submissions.active.is_empty() {
            return;
        }
        let view = self.recovery_submission_view();
        for gate in self
            .recovery_submissions
            .active
            .iter()
            .filter_map(Weak::upgrade)
        {
            if gate.view != view
                || !view.active_member
                || !gate
                    .refs
                    .iter()
                    .all(|reference| self.descriptor_ref_is_current(reference))
                || gate.connected.as_ref().is_some_and(|(peer, _)| {
                    !gate.refs.iter().any(|reference| {
                        reference.peer == *peer && self.proven_member_path(reference.device, *peer)
                    })
                })
            {
                gate.revoke();
            }
        }
        self.recovery_submissions
            .active
            .retain(|gate| gate.strong_count() > 0);
    }
    fn recovery_submission_gate(
        &mut self,
        refs: Vec<DescriptorRef>,
        connected: Option<PeerId>,
        dial: Option<PeerId>,
        expires: u64,
    ) -> Result<(Arc<Gate>, Arc<Flight>), Admission> {
        self.reconcile_recovery_submissions();
        let view = self.recovery_submission_view();
        if !view.active_member
            || refs.is_empty()
            || refs.len() > 3
            || !refs
                .iter()
                .all(|reference| self.descriptor_ref_is_current(reference))
        {
            return Err(Admission::Stale);
        }
        if self.recovery_submissions.active.len() >= LOCAL_SUBMISSIONS
            || dial.is_some_and(|peer| {
                self.recovery_submissions
                    .active
                    .iter()
                    .filter_map(Weak::upgrade)
                    .any(|gate| gate.dial == Some(peer) && gate.authorized())
            })
        {
            return Err(Admission::Busy);
        }
        let connection = if let Some(peer) = connected {
            if !refs.iter().any(|reference| {
                reference.peer == peer && self.proven_member_path(reference.device, peer)
            }) {
                return Err(Admission::Stale);
            }
            let lease = self.transport.connection_lease(peer);
            if self.transport.supports_connection_leases()
                && lease.as_ref().is_none_or(|lease| !lease.is_live())
            {
                return Err(Admission::Stale);
            }
            Some((peer, lease))
        } else {
            None
        };
        let flight = self.recovery_submissions.reserve().ok_or(Admission::Busy)?;
        let gate = Arc::new(Gate {
            view,
            refs,
            connected: connection,
            dial,
            live: watch::channel(true).0,
            clock: self.clock.clone(),
            deadline: self.clock.monotonic_ms().saturating_add(SUBMISSION_MS),
            expires,
        });
        if !gate.authorized() {
            return Err(Admission::Stale);
        }
        self.recovery_submissions.active.push(Arc::downgrade(&gate));
        Ok((gate, flight))
    }
    pub(super) fn complete_recovery_submission(&mut self, completed: Completion) {
        self.reconcile_recovery_submissions();
        if let Some(window) = completed.window {
            self.discovery
                .refund_endpoint_budget_in(window, completed.tally.refundable);
        }
        // A stale completion cannot cool a peer or manufacture present-time route evidence.
        if completed.gate.authorized() && completed.tally.submitted > 0 {
            if let Some(peer) = completed.gate.dial {
                self.note_dial_attempt(peer.as_bytes().to_vec(), completed.epochs);
            }
        }
    }
}
impl<T: MeshTransport + 'static, R: CryptoRngCore> ChannelSync<T, R> {
    pub(super) fn queue_recovery_dials(
        &mut self,
        peer: PeerId,
        refs: Vec<DescriptorRef>,
        permits: Vec<BoxedDialPermit>,
        epochs: Vec<(FreshnessPrincipal, u64)>,
        batch: bool,
        expires: u64,
    ) -> bool {
        let count = permits.len();
        let Ok((gate, flight)) = self.recovery_submission_gate(refs, None, Some(peer), expires)
        else {
            self.discovery.refund_endpoint_budget(count);
            return false;
        };
        let window = self.discovery.endpoint_budget_window();
        let mut spends = Vec::new();
        let permits = permits
            .into_iter()
            .map(|inner| {
                let spent = Arc::new(AtomicBool::new(false));
                spends.push(spent.clone());
                Box::new(GuardedPermit {
                    inner,
                    gate: gate.clone(),
                    spent,
                    _flight: flight.clone(),
                }) as BoxedDialPermit
            })
            .collect::<Vec<_>>();
        let transport = self.transport.clone();
        self.recovery_submissions.jobs.push(Box::pin(async move {
            let mut tally = DialOutcomeTally::default();
            if batch {
                let outcomes = transport.dial_peer_permits(peer, permits).await;
                match outcomes {
                    Ok(outcomes) if outcomes.len() == spends.len() => {
                        for outcome in outcomes {
                            tally.observe(Ok(outcome));
                        }
                    }
                    _ => {
                        for spent in spends {
                            tally.observe(if spent.load(Ordering::Acquire) {
                                Ok(catcoms_rt::DialSubmission::FailedAfterCommit)
                            } else {
                                Err(TransportError::Closed)
                            });
                        }
                    }
                }
            } else {
                for (permit, spent) in permits.into_iter().zip(spends) {
                    let outcome = transport.dial_permit(permit).await;
                    tally.observe(match outcome {
                        Err(_) if spent.load(Ordering::Acquire) => {
                            Ok(catcoms_rt::DialSubmission::FailedAfterCommit)
                        }
                        outcome => outcome,
                    });
                }
            }
            Completion {
                gate,
                window: Some(window),
                tally,
                epochs,
                _flight: flight,
            }
        }));
        true
    }
    pub(super) fn queue_recovery_notification(
        &mut self,
        item: RepairOutboxItem,
    ) -> Result<(), Admission> {
        let (gate, flight) = self.recovery_submission_gate(
            item.required_refs,
            Some(item.target),
            None,
            item.expires_at_ms,
        )?;
        let transport = self.transport.clone();
        self.recovery_submissions.jobs.push(Box::pin(async move {
            let (publisher, receiver) = watch::channel(false);
            let cancellation = RequestCancellation::new(receiver, Some(flight.clone())).with_authority(gate.clone());
            // The actual driver retains Flight in pending_req through response/failure. Deadline
            // retirement stops waiting only; it never recycles an outstanding lower stream.
            tokio::select! {
                _ = transport.notify_connected_cancellable(item.target, ProtocolId(RR_PROTOCOL), Bytes::from(item.payload), cancellation) => {},
                _ = gate.revoked() => {},
            }
            drop(publisher);
            Completion { gate, window: None, tally: DialOutcomeTally::default(), epochs: Vec::new(), _flight: flight }
        }));
        Ok(())
    }
}
