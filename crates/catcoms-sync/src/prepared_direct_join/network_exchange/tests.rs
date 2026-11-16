//! Trusted-host test issuers exercise the actual Sync state machine and Mem lower owner.
//! These are not authenticated journal/file witnesses. All tests authored UNRUN.
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork, Responder};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

#[test]
fn arm_data_debug_omits_invocation_and_execution_metadata() {
    // Inert labels only; formatting this does not model a saved arm/ticket or paid work.
    let data = OriginalJoinArmData {
        grant: 1,
        ordinal: 2,
        revision: 3,
        activation: [4; 32],
        invocation: [5; 32],
        deadline_ms: 6,
    };
    assert_eq!(format!("{data:?}"), "OriginalJoinArmData { .. }");
}

struct Context {
    id: Arc<()>,
    request: DirectJoinRequestIdentity,
    cancel: RequestCancellation,
    floor: u64,
}
impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TestOriginalContext")
    }
}
impl OriginalJoinNetworkContext for Context {
    fn identity(&self) -> Arc<()> {
        self.id.clone()
    }
    fn binding(&self) -> [u8; 32] {
        [7; 32]
    }
    fn request(&self) -> DirectJoinRequestIdentity {
        self.request
    }
    fn spent_ordinal_floor(&self) -> u64 {
        self.floor
    }
    fn cancellation(&self) -> RequestCancellation {
        self.cancel.clone()
    }
    fn check_original(&self) -> Result<(), SyncError> {
        if self.cancel.is_cancelled() {
            Err(SyncError::JoinRejected)
        } else {
            Ok(())
        }
    }
}
struct Saved {
    ctx: Arc<Context>,
    data: OriginalJoinArmData,
}
impl fmt::Debug for Saved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TestSavedData")
    }
}
impl OriginalJoinSavedArm for Saved {
    fn context_identity(&self) -> Arc<()> {
        self.ctx.identity()
    }
    fn binding(&self) -> [u8; 32] {
        self.ctx.binding()
    }
    fn request(&self) -> DirectJoinRequestIdentity {
        self.ctx.request
    }
    fn data(&self) -> OriginalJoinArmData {
        self.data
    }
    fn check_original_data(&self) -> Result<(), SyncError> {
        Ok(())
    } // test mount stays live after task close
}
#[derive(Debug)]
struct Debit(Arc<AtomicUsize>);
impl Debit {
    fn reserve(count: &Arc<AtomicUsize>) -> Option<Arc<Self>> {
        count
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        Some(Arc::new(Self(count.clone())))
    }
}
impl Drop for Debit {
    fn drop(&mut self) {
        assert_eq!(self.0.fetch_sub(1, Ordering::SeqCst), 1)
    }
}
#[derive(Debug)]
struct Ticket {
    saved: Arc<Saved>,
    cancel: RequestCancellation,
    target: PeerId,
}
impl OriginalJoinNetworkSubmission for Ticket {
    fn saved(&self) -> Arc<dyn OriginalJoinSavedArm> {
        self.saved.clone()
    }
    fn target(&self) -> PeerId {
        self.target
    }
    fn cancellation(&self) -> RequestCancellation {
        self.cancel.clone()
    }
}
struct Fixture {
    bound: BoundOriginalJoinNetwork<MemNetwork>,
    cell: Arc<RetainedOriginalJoinNetwork>,
    owner: ChannelSync<MemNetwork, ChaCha20Rng>,
    task: NegativeOwner,
    ctx: Arc<Context>,
    ready: Vec<u8>,
    count: Arc<AtomicUsize>,
    hub: Arc<Hub>,
}
fn fixture() -> Fixture {
    let hub = Hub::new();
    let target = PeerId::from_u64(0xd001);
    let member = PeerId::from_u64(0xd002);
    let mut owner = ChannelSync::found(
        hub.join(target),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0xd003),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let invite = owner.mint_invite([0xd0; 16], 10_000, vec![]).unwrap();
    let mut source = PreparedDirectJoin::prepare(
        MlsDevice::generate().unwrap(),
        invite,
        DirectJoinLimits::new(512 * 1024, 512 * 1024, 1024 * 1024).unwrap(),
    )
    .unwrap();
    source
        .seal_source(&[8; 32], [7; 32], &mut ChaCha20Rng::seed_from_u64(0xd004))
        .unwrap();
    let ready = owner.serve_join(member, &source.request[1..]).unwrap();
    let task = NegativeOwner::new();
    let ctx = Arc::new(Context {
        id: Arc::new(()),
        request: source.request_identity().unwrap(),
        cancel: task.observer(),
        floor: 0,
    });
    let cell = RetainedOriginalJoinNetwork::from_original_local(source, ctx.clone()).unwrap();
    let bound = cell
        .clone()
        .bind_original_transport(hub.join(member), target)
        .unwrap();
    Fixture {
        bound,
        cell,
        owner,
        task,
        ctx,
        ready,
        count: Arc::new(AtomicUsize::new(0)),
        hub,
    }
}
fn ticket(f: &Fixture, ordinal: u64) -> Box<dyn OriginalJoinNetworkSubmission> {
    let debit = Debit::reserve(&f.count).expect("one genuine lower slot available");
    Box::new(Ticket {
        saved: Arc::new(Saved {
            ctx: f.ctx.clone(),
            data: OriginalJoinArmData {
                grant: 1,
                ordinal,
                revision: 1,
                activation: [1; 32],
                invocation: [ordinal as u8; 32],
                deadline_ms: 10_000,
            },
        }),
        cancel: f.task.observer().retain_custody(debit),
        target: PeerId::from_u64(0xd001),
    })
}
async fn queued_request_data(transport: &MemNetwork) -> (Bytes, Responder) {
    for _ in 0..16 {
        let mut event = Box::pin(transport.next_event());
        match futures::poll!(&mut event) {
            std::task::Poll::Ready(Some(TransportEvent::Request {
                data, responder, ..
            })) => return (data, responder),
            std::task::Poll::Ready(Some(_)) => {}
            _ => panic!("polled original Mem request must already be queued"),
        }
    }
    panic!("bounded lifecycle prefix exhausted");
}

async fn queued_request(transport: &MemNetwork) -> Responder {
    queued_request_data(transport).await.1
}

#[tokio::test]
async fn cancellation_spends_ticket_but_actual_lower_responder_keeps_only_slot_until_completion() {
    let mut f = fixture();
    f.cell.install_original_submission(ticket(&f, 1)).unwrap();
    let mut exchange = Box::pin(f.bound.exchange_armed());
    assert!(futures::poll!(&mut exchange).is_pending());
    let responder = queued_request(&f.owner.transport).await;
    drop(exchange);
    assert_eq!(f.cell.try_status(), Some(OriginalJoinNetworkStatus::Spent));
    assert_eq!(
        f.count.load(Ordering::SeqCst),
        1,
        "actual lower responder retains completion custody"
    );
    assert!(
        Debit::reserve(&f.count).is_none(),
        "upper cancellation cannot refund actual driver work"
    );
    drop(responder);
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
    f.cell.install_original_submission(ticket(&f, 2)).unwrap();
    let mut exchange = Box::pin(f.bound.exchange_armed());
    assert!(futures::poll!(&mut exchange).is_pending());
    let responder = queued_request(&f.owner.transport).await;
    responder.respond(Bytes::from(f.ready.clone()));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), exchange)
            .await
            .unwrap()
            .unwrap(),
        OriginalJoinNetworkStatus::InputHeld
    );
    assert_eq!(
        f.count.load(Ordering::SeqCst),
        0,
        "held signed DATA retains no consumed lower ticket"
    );
    f.task.close();
    let a = f
        .cell
        .seal_received_input(&[8; 32], &mut ChaCha20Rng::seed_from_u64(0xd005))
        .unwrap();
    let b = f
        .cell
        .seal_received_input(&[8; 32], &mut ChaCha20Rng::seed_from_u64(0xd006))
        .unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(a.arm_data().ordinal, 2);
    assert_eq!(
        f.cell.try_status(),
        Some(OriginalJoinNetworkStatus::InputHeld)
    );
}

#[tokio::test]
async fn once_bound_transport_cannot_be_replaced_and_same_consumed_ordinal_cannot_replay() {
    let mut f = fixture();
    let replacement = f.hub.join(PeerId::from_u64(0xd010));
    let returned = f
        .cell
        .clone()
        .bind_original_transport(replacement, PeerId::from_u64(0xd001))
        .unwrap_err();
    let (replacement, cell, _) = returned.into_parts();
    assert_eq!(replacement.local_peer(), PeerId::from_u64(0xd010));
    assert!(Arc::ptr_eq(&cell, &f.cell));
    f.cell.install_original_submission(ticket(&f, 1)).unwrap();
    let mut exchange = Box::pin(f.bound.exchange_armed());
    assert!(futures::poll!(&mut exchange).is_pending());
    let responder = queued_request(&f.owner.transport).await;
    drop(exchange);
    let _ = f.cell.try_status();
    drop(responder);
    let rejected = f
        .cell
        .install_original_submission(ticket(&f, 1))
        .unwrap_err();
    drop(rejected);
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
    assert_eq!(f.cell.try_status(), Some(OriginalJoinNetworkStatus::Spent));
    f.cell.install_original_submission(ticket(&f, 2)).unwrap();
}

#[tokio::test]
async fn bad_signature_is_spent_data_is_not_retained_and_private_source_still_allows_next_arm() {
    let mut f = fixture();
    f.cell.install_original_submission(ticket(&f, 1)).unwrap();
    let mut exchange = Box::pin(f.bound.exchange_armed());
    assert!(futures::poll!(&mut exchange).is_pending());
    let responder = queued_request(&f.owner.transport).await;
    let mut bad = f.ready.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    responder.respond(Bytes::from(bad));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), exchange)
            .await
            .unwrap()
            .unwrap(),
        OriginalJoinNetworkStatus::Spent
    );
    assert!(f
        .cell
        .seal_received_input(&[8; 32], &mut ChaCha20Rng::seed_from_u64(0xd007))
        .is_err());
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
    f.cell.install_original_submission(ticket(&f, 2)).unwrap();
}

#[tokio::test]
async fn opaque_continuation_preserves_same_transport_prefix_and_request_and_cannot_attach_elsewhere(
) {
    let mut f = fixture();
    f.cell.install_original_submission(ticket(&f, 1)).unwrap();
    let mut flight = Box::pin(f.bound.exchange_armed());
    assert!(futures::poll!(&mut flight).is_pending());
    let (original_request, responder) = queued_request_data(&f.owner.transport).await;
    drop(flight);
    let _ = f.cell.try_status();
    f.task.close();
    // Synthetic local lifecycle INPUT only: the opaque continuation must preserve this exact
    // already-consumed prefix, never replace it with another transport's current snapshot.
    let prior = PeerId::from_u64(0xd077);
    assert!(f
        .bound
        .prefix
        .observe(&TransportEvent::PeerConnected(prior)));
    let next_task = NegativeOwner::new();
    let too_old = Arc::new(Context {
        id: Arc::new(()),
        request: f.ctx.request,
        cancel: next_task.observer(),
        floor: 0,
    });
    assert!(
        f.bound.prepare_original_continuation(too_old).is_err(),
        "saved consumed ordinal cannot go backwards"
    );
    let next_ctx = Arc::new(Context {
        id: Arc::new(()),
        request: f.ctx.request,
        cancel: next_task.observer(),
        floor: 1,
    });
    let prepared = f
        .bound
        .prepare_original_continuation(next_ctx.clone())
        .unwrap();
    let next = prepared.retained_cell_data();
    assert!(
        next.clone()
            .bind_original_transport(
                f.hub.join(PeerId::from_u64(0xd099)),
                PeerId::from_u64(0xd001)
            )
            .is_err(),
        "new bind claim was reserved before publication"
    );
    let mut foreign = fixture();
    let returned = foreign
        .bound
        .attach_original_continuation(prepared)
        .unwrap_err();
    let (same, _) = returned.into_parts();
    f.bound.attach_original_continuation(same).unwrap();
    assert!(f.bound.prefix.peers.contains_key(&prior));
    assert_eq!(f.cell.try_status(), Some(OriginalJoinNetworkStatus::Closed));
    assert_eq!(
        f.count.load(Ordering::SeqCst),
        1,
        "continuing source/context cannot refund actual old driver work"
    );
    assert!(Debit::reserve(&f.count).is_none());
    drop(responder);
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
    let debit = Debit::reserve(&f.count).unwrap();
    let ticket = Box::new(Ticket {
        saved: Arc::new(Saved {
            ctx: next_ctx,
            data: OriginalJoinArmData {
                grant: 2,
                ordinal: 2,
                revision: 2,
                activation: [2; 32],
                invocation: [2; 32],
                deadline_ms: 20_000,
            },
        }),
        cancel: next_task.observer().retain_custody(debit),
        target: PeerId::from_u64(0xd001),
    });
    next.install_original_submission(ticket).unwrap();
    let mut flight = Box::pin(f.bound.exchange_armed());
    assert!(futures::poll!(&mut flight).is_pending());
    let (request, responder) = queued_request_data(&f.owner.transport).await;
    assert_eq!(request, original_request);
    responder.respond(Bytes::from(f.ready.clone()));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), flight)
            .await
            .unwrap()
            .unwrap(),
        OriginalJoinNetworkStatus::InputHeld
    );
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
}

#[test]
fn preparing_one_continuation_moves_source_once_and_never_releases_its_bind_claim_on_drop() {
    let mut f = fixture();
    f.task.close();
    let next_task = NegativeOwner::new();
    let context = Arc::new(Context {
        id: Arc::new(()),
        request: f.ctx.request,
        cancel: next_task.observer(),
        floor: 0,
    });
    let prepared = f.bound.prepare_original_continuation(context).unwrap();
    let retained = prepared.retained_cell_data();
    drop(prepared);
    let another = Arc::new(Context {
        id: Arc::new(()),
        request: f.ctx.request,
        cancel: next_task.observer(),
        floor: 0,
    });
    assert!(
        f.bound.prepare_original_continuation(another).is_err(),
        "old cell no longer owns a second private source"
    );
    assert!(retained
        .bind_original_transport(
            f.hub.join(PeerId::from_u64(0xd09a)),
            PeerId::from_u64(0xd001)
        )
        .is_err());
}

struct DropCheckedContext {
    original: Context,
    old: std::sync::Weak<RetainedOriginalJoinNetwork>,
    observed: Arc<AtomicBool>,
}
impl fmt::Debug for DropCheckedContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DropCheckedContext")
    }
}
impl Drop for DropCheckedContext {
    fn drop(&mut self) {
        let free = self.old.upgrade().is_some_and(|cell| {
            let free = cell.state.try_lock().is_ok();
            free
        });
        self.observed.store(free, Ordering::SeqCst);
    }
}
impl OriginalJoinNetworkContext for DropCheckedContext {
    fn identity(&self) -> Arc<()> {
        self.original.identity()
    }
    fn binding(&self) -> [u8; 32] {
        self.original.binding()
    }
    fn request(&self) -> DirectJoinRequestIdentity {
        self.original.request()
    }
    fn spent_ordinal_floor(&self) -> u64 {
        self.original.spent_ordinal_floor()
    }
    fn cancellation(&self) -> RequestCancellation {
        self.original.cancellation()
    }
    fn check_original(&self) -> Result<(), SyncError> {
        self.original.check_original()
    }
}
#[tokio::test]
async fn refused_continuation_drops_last_context_custody_only_after_old_cell_guard_drains() {
    let mut f = fixture();
    f.cell.install_original_submission(ticket(&f, 1)).unwrap();
    let mut flight = Box::pin(f.bound.exchange_armed());
    assert!(futures::poll!(&mut flight).is_pending());
    let responder = queued_request(&f.owner.transport).await;
    responder.respond(Bytes::from(f.ready.clone()));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), flight)
            .await
            .unwrap()
            .unwrap(),
        OriginalJoinNetworkStatus::InputHeld
    );
    f.task.close();
    let next_task = NegativeOwner::new();
    let observed = Arc::new(AtomicBool::new(false));
    // Move the SOLE context Arc in: its final Drop may own a real quota/destructor in the host.
    let context = Arc::new(DropCheckedContext {
        original: Context {
            id: Arc::new(()),
            request: f.ctx.request,
            cancel: next_task.observer(),
            floor: 1,
        },
        old: Arc::downgrade(&f.cell),
        observed: observed.clone(),
    });
    assert!(
        f.bound.prepare_original_continuation(context).is_err(),
        "held INPUT cannot be regranted"
    );
    assert!(
        observed.load(Ordering::SeqCst),
        "host metadata custody never drops under old cell guard"
    );
    assert_eq!(
        f.cell.try_status(),
        Some(OriginalJoinNetworkStatus::InputHeld)
    );
}
