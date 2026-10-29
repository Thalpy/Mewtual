use super::super::runtime::Runtime;
use super::*;
use crate::ChannelSync;
use catcoms_rt::ManualClock;
use std::future::Future;
use std::task::{Context as TaskContext, Poll, Wake, Waker};

// Local aliases keep the tests on real issuer/Root paths, not caller-built grants.
type Node = ChannelSync<catcoms_rt::mem::MemNetwork, rand_chacha::ChaCha20Rng>;
async fn fixture() -> (Node, Arc<Runtime>) {
    let (node, runtime, _) = fixture_clock().await;
    (node, runtime)
}
async fn fixture_clock() -> (Node, Arc<Runtime>, Arc<ManualClock>) {
    let (_, mut nodes, _) = crate::tests::build_members(1).await;
    let mut node = nodes.remove(0);
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    node.prepare_voice_actor(
        &runtime,
        &ticket,
        991,
        |_| Ok(std::collections::BTreeSet::from([1, 2])),
        |_| Ok(std::collections::BTreeSet::new()),
    )
    .unwrap()
    .register()
    .unwrap();
    drop(node.voice_endpoint_changes().unwrap()); // finish actual initial view reconciliation
    (node, runtime, clock)
}

#[tokio::test]
async fn original_resources_identity_refuses_foreign_pool_without_charging_or_authorizing() {
    let (mut node, runtime) = fixture().await;
    let prepared = node.prepare_admitted_voice_call(1).unwrap();
    let original = prepared.observer();
    let own = runtime.resources();
    let alias = own.clone();
    let foreign = Arc::new(Resources::default());
    let before = (own.counts(), foreign.counts());
    for _ in 0..12 {
        assert!(original.uses_resources(&own));
        assert!(original.uses_resources(&alias));
        assert!(!original.uses_resources(&foreign));
    }
    assert_eq!((own.counts(), foreign.counts()), before);
    assert!(
        !original.is_active(),
        "funding identity never selects or activates a call"
    );
    original.close();
    assert!(
        original.uses_resources(&own),
        "immutable history remains comparable after End"
    );
    assert!(!original.uses_resources(&foreign));
    assert!(!original.is_active());
    assert_eq!((own.counts(), foreign.counts()), before);
}

#[tokio::test]
async fn other_room_canceled_prepare_is_reclaimed_before_successor_funding_under_pressure() {
    for keep_external in [false, true] {
        let (mut node, runtime, _) = fixture_clock().await;
        let pair = node.prepare_admitted_voice_call(1).unwrap();
        let external = keep_external.then(|| pair.observer());
        drop(pair);
        let resources = runtime.resources();
        let pressure = resources
            .charge(
                node.voice_v2_context(2).unwrap().scope.group,
                super::super::resources::GROUP_BYTES - resources.counts().0 - 256,
                ResourceKind::Retained,
            )
            .unwrap();
        let prepared = node.prepare_admitted_voice_call(2);
        if let Some(external) = external {
            assert!(
                matches!(prepared, Err(VoiceError::Capacity)),
                "real external owner still retains its original payment"
            );
            drop(external);
            let next = node.prepare_admitted_voice_call(2).unwrap();
            assert_eq!(next.grant().scope().room, 2);
        } else {
            assert_eq!(prepared.unwrap().grant().scope().room, 2);
        }
        drop(pressure);
    }
}

#[tokio::test]
async fn quiet_collection_after_lost_prepare_reply_needs_no_new_rpc_credit() {
    let (mut node, runtime, clock) = fixture_clock().await;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let prepared = node.prepare_admitted_voice_call(1).unwrap();
    drop(receiver);
    drop(sender.send(prepared)); // actual owned prepared pair dies at a lost-reply boundary
    let resources = runtime.resources();
    let scope = node.voice_v2_context(2).unwrap().scope;
    let pressure = resources
        .charge(
            scope.group,
            super::super::resources::GROUP_BYTES - resources.counts().0 - 256,
            ResourceKind::Retained,
        )
        .unwrap();
    assert!(matches!(
        ActivationRpcPermit::reserve(&resources, scope),
        Err(VoiceError::Capacity)
    ));
    // This is the existing Actor housekeeping path, not a newly funded cleanup RPC.
    clock.advance_ms(1001);
    node.collect_expired_voice();
    drop(ActivationRpcPermit::reserve(&resources, scope).unwrap());
    assert_eq!(
        node.prepare_admitted_voice_call(2)
            .unwrap()
            .grant()
            .scope()
            .room,
        2
    );
    drop(pressure);
}
fn key() -> [u8; 65] {
    hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5")
        .unwrap().try_into().unwrap()
}
struct ProbeWake {
    locks_free: Arc<dyn Fn() -> bool + Send + Sync>,
    called: Arc<std::sync::atomic::AtomicUsize>,
    panic: bool,
}
impl Wake for ProbeWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        assert!(
            (self.locks_free)(),
            "wake probe rejected current guard/authority state"
        );
        let previous = self.called.fetch_add(1, Ordering::AcqRel);
        assert!(
            !(self.panic && previous == 0),
            "controlled one panicking waker"
        );
    }
}

#[tokio::test]
async fn prepared_profile_is_original_inert_and_old_lower_factories_do_not_spend_allocator() {
    let (mut node, _runtime) = fixture().await;
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    let original = pair.observer();
    assert!(!original.is_active() && !original.is_closed());
    assert!(!node.has_live_v2_participant());
    let wrapped = Arc::new(CallGrant::from_lease(
        pair.grant().lease(),
        pair.grant().scope(),
    ));
    assert!(wrapped.lease().is_admitted());
    assert!(
        !original.matches_grant(&wrapped),
        "equal lease cannot replace original grant Arc"
    );
    assert!(
        super::super::WatchLease::new(CallLease::default())
            .with_parent(pair.grant().lease())
            .is_err(),
        "an admitted child cannot be relabelled as an Existing watcher parent"
    );
    let context = node.voice_v2_context(1).unwrap();
    let resources = original.core.resources.clone();
    assert!(super::super::presence::LocalJoin::new(
        &context,
        super::super::JoinKey::new([1; 32], key()).unwrap(),
        pair.grant().lease(),
        resources.clone()
    )
    .is_err());
    assert!(super::super::packets::PacketLedger::new(
        &context,
        [1; 32],
        [2; 32],
        pair.grant().lease(),
        resources
    )
    .is_err());
    assert!(!pair
        .grant()
        .lease()
        .state()
        .ledger_claimed
        .load(Ordering::Acquire));
}

#[tokio::test]
async fn actual_root_selection_is_inert_and_blocks_older_directory_recognized_child() {
    let (mut node, _runtime) = fixture().await;
    let old = Arc::new(node.voice_call_grant(1).unwrap());
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    let original = pair.observer();
    let selected = node
        .select_admitted_voice_call(pair.grant(), &original)
        .unwrap();
    assert!(!original.is_active());
    assert!(
        node.has_live_v2_participant(),
        "legacy exclusion precedes any LocalJoin"
    );
    assert!(
        old.lease().is_live(),
        "test keeps a genuinely issued older child"
    );
    assert!(node
        .start_voice_endpoint(old.clone(), [4; 32], key())
        .is_err());
    assert!(node
        .start_voice_endpoint(pair.grant().clone(), [5; 32], key())
        .is_err());
    selected.commit_native(pair.grant()).unwrap();
    assert!(original.is_active());
    assert!(node.validate_voice_call_grant(&old).is_err());
    // Even a correctly activated admitted call does not expose an old endpoint producer.
    assert!(node
        .start_voice_endpoint(pair.grant().clone(), [5; 32], key())
        .is_err());
    node.clear_admitted_voice_call(&original).unwrap();
    assert!(original.root_returned().unwrap().matches(&original));
    node.start_voice_endpoint(old, [6; 32], key()).unwrap();
}

#[tokio::test]
async fn healthy_existing_participant_refuses_upgrade_and_ordinary_existing_retry_still_works() {
    let (mut node, _runtime) = fixture().await;
    let old = Arc::new(node.voice_call_grant(1).unwrap());
    node.start_voice_endpoint(old.clone(), [3; 32], key())
        .unwrap();
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    let original = pair.observer();
    assert!(node
        .select_admitted_voice_call(pair.grant(), &original)
        .is_err());
    node.start_voice_endpoint(old.clone(), [3; 32], key())
        .unwrap();
    assert!(old.lease().is_live());
    let zero = original.seal_unclaimed_residence().unwrap();
    assert!(zero.matches(&original));
    assert!(
        original.root_returned().is_none(),
        "zero is NOT an actual returned residence"
    );
}

#[tokio::test]
async fn close_then_seal_prevents_any_late_root_acquisition() {
    let (mut node, _runtime) = fixture().await;
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    let original = pair.observer();
    let zero = original.seal_unclaimed_residence().unwrap();
    assert!(zero.matches(&original));
    assert!(node
        .select_admitted_voice_call(pair.grant(), &original)
        .is_err());
    assert!(original.select().is_err());
    assert!(original.root_returned().is_none());
}

#[tokio::test]
async fn held_ticket_requires_actual_marker_even_when_close_precedes_installation() {
    let (mut node, _runtime) = fixture().await;
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    let original = pair.observer();
    let (residence, selected) = original.select().unwrap();
    original.close();
    assert!(original.seal_unclaimed_residence().is_none());
    assert!(selected.commit_native(pair.grant()).is_err());
    assert!(original.root_returned().is_none());
    drop(residence);
    assert!(original.root_returned().unwrap().matches(&original));
}

#[tokio::test]
async fn actual_root_alias_destruction_precedes_return_certificate_and_plain_payment() {
    let (mut node, runtime) = fixture().await;
    let baseline = runtime.resources().counts();
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    let original = pair.observer();
    let (residence, selected) = original.select().unwrap();
    drop(selected);
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    *original.core.returned.before_return.lock().unwrap() = Some((entered.clone(), resume.clone()));
    drop(pair); // strong Grant/Room custody is unnecessary for negative recovery
    assert!(original.core.grant.upgrade().is_none());
    let dropped = std::thread::spawn(move || drop(residence));
    entered.wait();
    assert!(original.root_returned().is_none());
    assert!(runtime.resources().counts().0 >= baseline.0 + ACTIVATION_BYTES);
    resume.wait();
    dropped.join().unwrap();
    let returned = original.root_returned().unwrap();
    node.clear_admitted_voice_call(&original).unwrap(); // release real issuer aliases too
    drop(original);
    assert!(
        runtime.resources().counts().0 >= baseline.0 + ACTIVATION_BYTES,
        "plain receipt keeps last payer after Core/Weak allocation destruction"
    );
    drop(returned);
    assert_eq!(runtime.resources().counts(), baseline);
}

#[tokio::test]
async fn original_issuer_aliases_keep_same_payment_after_grant_core_and_receipts_are_gone() {
    let (mut node, runtime) = fixture().await;
    let baseline = runtime.resources().counts();
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    drop(pair); // no Root selection; only actual Policy/Room child aliases remain
    assert_eq!(
        runtime.resources().counts().0,
        baseline.0 + ACTIVATION_BYTES
    );
    // Successor creation prunes the old real aliases BEFORE asking the pool for new fees.
    let existing = node.voice_call_grant(1).unwrap();
    assert_eq!(runtime.resources().counts().0, baseline.0 + 256);
    drop(existing);
}

#[test]
fn final_paid_lease_wrapper_drops_allocation_before_its_last_plain_payer() {
    let resources = Arc::new(Resources::default());
    let scope = Scope::for_group(b"paid original admitted lease", None, 1);
    let funding = Funding::reserve(&resources, scope, 0).unwrap();
    let first = CallLease::admitted(funding.clone());
    let mut last = first.clone();
    drop(funding);
    drop(first);
    assert_eq!(resources.counts().0, ACTIVATION_BYTES);
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let after_entered = entered.clone();
    let after_resume = resume.clone();
    // Exercise the actual private Drop helper with a barrier AFTER Arc::drop returns.
    // No Weak lease exists; Sender, LeaseState and its header have all been destroyed.
    let state = last.0.take().unwrap();
    drop(last);
    let worker = std::thread::spawn(move || {
        CallLease::destroy_state(state, || {
            after_entered.wait();
            after_resume.wait();
        })
    });
    entered.wait();
    assert_eq!(resources.counts().0, ACTIVATION_BYTES);
    resume.wait();
    worker.join().unwrap();
    assert_eq!(resources.counts().0, 0);
}

#[tokio::test]
async fn same_original_recovers_selection_and_old_clear_never_clears_replacement() {
    let (mut node, _runtime) = fixture().await;
    let first = node.prepare_admitted_voice_call(1).unwrap();
    let a = first.observer();
    drop(node.select_admitted_voice_call(first.grant(), &a).unwrap()); // lost reply
    node.select_admitted_voice_call(first.grant(), &a)
        .unwrap()
        .commit_native(first.grant())
        .unwrap();
    node.clear_admitted_voice_call(&a).unwrap();
    let second = node.prepare_admitted_voice_call(1).unwrap();
    let b = second.observer();
    node.select_admitted_voice_call(second.grant(), &b)
        .unwrap()
        .commit_native(second.grant())
        .unwrap();
    node.clear_admitted_voice_call(&a).unwrap();
    assert!(b.is_active());
    assert!(a.root_returned().unwrap().matches(&a));
    assert!(b.root_returned().is_none());
    node.clear_admitted_voice_call(&b).unwrap();
}

#[tokio::test]
async fn epoch_revoke_wakes_original_prepared_lease_without_changing_existing_child_behavior() {
    let (mut node, _runtime) = fixture().await;
    let existing = node.voice_call_grant(1).unwrap();
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    let original = pair.observer();
    let context = node.voice_v2_context(1).unwrap();
    let voice = node.voice_runtime.as_mut().unwrap();
    voice
        .current
        .as_mut()
        .unwrap()
        .sync_view(super::super::runtime::View {
            epoch: context.epoch + 1,
            members: std::collections::BTreeSet::from([context.local]),
            policy: context.scope.policy,
            eligible: true,
            indirect: false,
        })
        .unwrap();
    original.wait_closed().await;
    assert!(original.is_closed());
    assert!(existing.lease().is_live());
}

#[tokio::test]
async fn epoch_closes_commit_and_serializes_old_prepared_issuance_before_publication() {
    let (mut node, runtime) = fixture().await;
    let old_existing = node.voice_call_grant(1).unwrap();
    let pair = node.prepare_admitted_voice_call(1).unwrap();
    let original = pair.observer();
    let selected = node
        .select_admitted_voice_call(pair.grant(), &original)
        .unwrap();
    let context = node.voice_v2_context(1).unwrap();
    let voice = node.voice_runtime.as_mut().unwrap();
    let actor = voice.current.as_mut().unwrap();
    let issuer = actor.admitted_issuer_for_test();
    let old_funding = Funding::reserve(&runtime.resources(), context.scope, context.epoch).unwrap();
    let (start, started) = std::sync::mpsc::channel();
    let (finished, result) = std::sync::mpsc::channel();
    let grant = pair.grant().clone();
    let mut worker = None;
    actor
        .sync_view_with_epoch_hook(
            super::super::runtime::View {
                epoch: context.epoch + 1,
                members: std::collections::BTreeSet::from([context.local]),
                policy: context.scope.policy,
                eligible: true,
                indirect: false,
            },
            || {
                // Epoch value is still old here, but the SAME original lease gate is closed.
                assert!(selected.commit_native(&grant).is_err());
                worker = Some(std::thread::spawn(move || {
                    start.send(()).unwrap();
                    finished.send(issuer(old_funding)).unwrap();
                }));
                started.recv().unwrap();
                assert!(
                    matches!(result.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)),
                    "actual issuer cannot pass the held Policy publication fence"
                );
            },
        )
        .unwrap();
    assert!(
        matches!(result.recv().unwrap(), Err(VoiceError::Freshness)),
        "old funding cannot issue after epoch publication even before a Core exists"
    );
    worker.take().unwrap().join().unwrap();
    assert!(!original.is_active() && original.is_closed());
    assert!(
        old_existing.lease().is_live(),
        "Existing epoch behavior stays unchanged"
    );
}

#[tokio::test]
async fn admitted_wakers_reenter_after_group_and_policy_on_epoch_policy_and_error_paths() {
    for branch in 0..5 {
        let (mut node, runtime) = fixture().await;
        let pair = node.prepare_admitted_voice_call(1).unwrap();
        let original = pair.observer();
        let context = node.voice_v2_context(1).unwrap();
        let actor = node
            .voice_runtime
            .as_mut()
            .unwrap()
            .current
            .as_mut()
            .unwrap();
        let called = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(ProbeWake {
            locks_free: actor.lock_probe_for_test(),
            called: called.clone(),
            panic: false,
        }));
        let mut waiting = Box::pin(original.wait_closed());
        assert!(matches!(
            waiting.as_mut().poll(&mut TaskContext::from_waker(&waker)),
            Poll::Pending
        ));
        let fresh = super::super::DeviceKeypair::from_seed(&[99; 32]).device_id();
        let members = if branch == 2 || branch == 3 {
            std::collections::BTreeSet::from([context.local, fresh])
        } else {
            std::collections::BTreeSet::from([context.local])
        };
        if branch == 4 {
            actor.supersede_revision_for_test();
        }
        let pressure = (branch == 2).then(|| {
            runtime
                .resources()
                .charge(
                    context.scope.group,
                    super::super::resources::GROUP_BYTES - runtime.resources().counts().0,
                    ResourceKind::Retained,
                )
                .unwrap()
        });
        let result = actor.sync_view(super::super::runtime::View {
            epoch: context.epoch + u64::from(branch == 0 || branch == 2),
            members,
            policy: if branch == 1 {
                Some([7; 32])
            } else {
                context.scope.policy
            },
            eligible: true,
            indirect: false,
        });
        if branch == 2 {
            assert_eq!(result, Err(VoiceError::Capacity));
        } else if branch == 3 {
            assert_eq!(result, Err(VoiceError::Freshness));
        } else if branch == 4 {
            assert_eq!(result, Err(VoiceError::Unauthorized));
        } else {
            result.unwrap();
        }
        assert!(called.load(Ordering::Acquire) > 0);
        assert!(matches!(
            waiting.as_mut().poll(&mut TaskContext::from_waker(&waker)),
            Poll::Ready(())
        ));
        drop(pressure);
    }
}

#[tokio::test]
async fn existing_waker_panic_observes_negative_actor_policy_and_all_admitted_children() {
    let (mut node, _runtime) = fixture().await;
    let earlier_existing = node.voice_call_grant(1).unwrap();
    let first = node.prepare_admitted_voice_call(1).unwrap();
    let second = node.prepare_admitted_voice_call(2).unwrap();
    let a = first.observer();
    let b = second.observer();
    let context = node.voice_v2_context(1).unwrap();
    let actor = node
        .voice_runtime
        .as_mut()
        .unwrap()
        .current
        .as_mut()
        .unwrap();
    let negative = actor.negative_view_probe_for_test();
    let checked_a = a.clone();
    let checked_b = b.clone();
    let existing_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let existing_waker = Waker::from(Arc::new(ProbeWake {
        locks_free: Arc::new(move || negative() && checked_a.is_closed() && checked_b.is_closed()),
        called: existing_count.clone(),
        panic: true,
    }));
    let old_lease = earlier_existing.lease();
    let mut existing_wait = Box::pin(old_lease.wait_revoked());
    assert!(matches!(
        existing_wait
            .as_mut()
            .poll(&mut TaskContext::from_waker(&existing_waker)),
        Poll::Pending
    ));
    let count_a = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count_b = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let wake_a = Waker::from(Arc::new(ProbeWake {
        locks_free: actor.lock_probe_for_test(),
        called: count_a.clone(),
        panic: false,
    }));
    let wake_b = Waker::from(Arc::new(ProbeWake {
        locks_free: actor.lock_probe_for_test(),
        called: count_b.clone(),
        panic: false,
    }));
    let mut wait_a = Box::pin(a.wait_closed());
    let mut wait_b = Box::pin(b.wait_closed());
    assert!(matches!(
        wait_a.as_mut().poll(&mut TaskContext::from_waker(&wake_a)),
        Poll::Pending
    ));
    assert!(matches!(
        wait_b.as_mut().poll(&mut TaskContext::from_waker(&wake_b)),
        Poll::Pending
    ));
    // A contradictory same-epoch roster follows the actual whole-Actor negative
    // branch. The earlier Existing callback runs under inherited locks and panics;
    // all new admitted phases must already be closed and wake after those guards.
    let fresh = super::super::DeviceKeypair::from_seed(&[98; 32]).device_id();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        actor.sync_view(super::super::runtime::View {
            epoch: context.epoch,
            members: std::collections::BTreeSet::from([context.local, fresh]),
            policy: context.scope.policy,
            eligible: true,
            indirect: false,
        })
    }));
    assert!(result.is_err());
    assert!(existing_count.load(Ordering::Acquire) > 0);
    assert!(a.is_closed() && b.is_closed());
    assert!(
        count_a.load(Ordering::Acquire) > 0 && count_b.load(Ordering::Acquire) > 0,
        "deferred admitted notification survives an earlier inherited callback panic"
    );
}

#[tokio::test]
async fn one_panicking_waker_does_not_strand_later_invalidated_children() {
    let (mut node, _runtime) = fixture().await;
    let first = node.prepare_admitted_voice_call(1).unwrap();
    let second = node.prepare_admitted_voice_call(2).unwrap();
    let a = first.observer();
    let b = second.observer();
    let context = node.voice_v2_context(1).unwrap();
    let actor = node
        .voice_runtime
        .as_mut()
        .unwrap()
        .current
        .as_mut()
        .unwrap();
    let count_a = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count_b = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let wake_a = Waker::from(Arc::new(ProbeWake {
        locks_free: actor.lock_probe_for_test(),
        called: count_a.clone(),
        panic: true,
    }));
    let wake_b = Waker::from(Arc::new(ProbeWake {
        locks_free: actor.lock_probe_for_test(),
        called: count_b.clone(),
        panic: false,
    }));
    let mut wait_a = Box::pin(a.wait_closed());
    let mut wait_b = Box::pin(b.wait_closed());
    assert!(matches!(
        wait_a.as_mut().poll(&mut TaskContext::from_waker(&wake_a)),
        Poll::Pending
    ));
    assert!(matches!(
        wait_b.as_mut().poll(&mut TaskContext::from_waker(&wake_b)),
        Poll::Pending
    ));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        actor.sync_view(super::super::runtime::View {
            epoch: context.epoch + 1,
            members: std::collections::BTreeSet::from([context.local]),
            policy: context.scope.policy,
            eligible: true,
            indirect: false,
        })
    }));
    assert!(result.is_err());
    assert!(count_a.load(Ordering::Acquire) > 0);
    assert!(
        count_b.load(Ordering::Acquire) > 0,
        "array drop glue must wake later entries during unwind"
    );
    assert!(a.is_closed() && b.is_closed());
}

#[test]
fn complete_activation_heap_and_weak_tail_have_an_independent_growth_guard() {
    use std::mem::size_of;
    #[repr(C)]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    assert_eq!(
        size_of::<CallLease>(),
        size_of::<Arc<super::super::LeaseState>>(),
        "private paid-drop wrapper must preserve the existing handle footprint"
    );
    // Pinned Tokio1.53.3 watch::Shared: lock/value, state+counts, eight receive
    // Notify values and one sender Notify. Include optional cursor and alignment.
    // Receiver waiters are intrusive in the separately measured native task future.
    let watch = 64 + 4 * size_of::<usize>() + 9 * size_of::<tokio::sync::Notify>() + 64;
    assert!(size_of::<std::sync::RwLock<bool>>() <= 64);
    let bytes = size_of::<ArcAllocation<Core>>()
        + size_of::<ArcAllocation<ReturnState>>()
        + size_of::<ArcAllocation<Funding>>()
        + size_of::<ArcAllocation<CallGrant>>() // whole Weak-retained original allocation
        + size_of::<ArcAllocation<super::super::LeaseState>>() + watch
        + EpochObservation::retained_graph_bytes()
        + size_of::<ArcAllocation<()>>()
        + size_of::<RootResidence>() + size_of::<RootSelectedCall>()
        + size_of::<PreparedAdmittedCall>() + 128;
    eprintln!("activation complete retained/preparation {bytes}/{ACTIVATION_BYTES}");
    assert!(bytes <= ACTIVATION_BYTES);
}
