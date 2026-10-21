//! Actual native selected-key/actor fixtures; no Tauri serialization or live RTC is claimed.
use super::*;
use crate::voice_activation_tests::Fixture;
use catcoms_rt::rng::RngError;
use catcoms_rt::{CryptoRng, RngCore};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const POINT: &str = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
async fn select(f: &Fixture, point: [u8; 65]) -> (String, Arc<SelectedHandoffOwner>) {
    let owner = f.candidate().await;
    crate::activate_voice_owner_inner(&f.state, owner.clone())
        .await
        .unwrap();
    let executor = hex::encode([9; 32]);
    let admission = crate::voice_join_key::admit_inner(&f.state, &owner, &executor)
        .await
        .unwrap();
    crate::voice_join_key::publish_inner(&f.state, &owner, &executor, &admission.permit, point)
        .await
        .unwrap();
    crate::voice_join_key::settle_inner(&f.state, &owner, [9; 32])
        .await
        .unwrap();
    let selected = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .selected_handoff_owner(&owner, [9; 32])
        .unwrap();
    (owner, selected)
}
fn point() -> [u8; 65] {
    hex::decode(POINT).unwrap().try_into().unwrap()
}
struct Rng<'a> {
    rng: ChaCha20Rng,
    draws: usize,
    hook: Option<Box<dyn FnOnce() + 'a>>,
}
impl Rng<'_> {
    fn new() -> Self {
        Self {
            rng: ChaCha20Rng::seed_from_u64(29),
            draws: 0,
            hook: None,
        }
    }
    fn touch(&mut self) {
        self.draws += 1;
        if let Some(hook) = self.hook.take() {
            hook();
        }
    }
}
impl RngCore for Rng<'_> {
    fn next_u32(&mut self) -> u32 {
        self.touch();
        self.rng.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.touch();
        self.rng.next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.touch();
        self.rng.fill_bytes(bytes);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), RngError> {
        self.touch();
        self.rng.try_fill_bytes(bytes)
    }
}
impl CryptoRng for Rng<'_> {}
fn ready(status: Status) -> Arc<SetupOutput> {
    match status {
        Status::Ready(public) => public,
        _ => panic!("expected immutable setup"),
    }
}

#[test]
fn fixed_index_and_row_graph_fit_their_separate_real_payments() {
    let arc_words = 2 * std::mem::size_of::<usize>();
    let index = std::mem::size_of::<Registry>() + std::mem::size_of::<Index>() + arc_words;
    assert!(index <= INDEX_BYTES, "index requires {index}");
    // Include retained row, its Arc, preparation, Session result/projection and a second
    // returned descriptor copy. SelectedHandoffOwner has its separate original paid owner.
    let row = std::mem::size_of::<Row>()
        + arc_words
        + std::mem::size_of::<Ticket>()
        + std::mem::size_of::<SetupOutput>()
        + std::mem::size_of::<Credit>()
        + arc_words * 2
        + std::mem::size_of::<Arc<Session>>()
        + std::mem::size_of::<Begin>()
        + std::mem::size_of::<Status>();
    assert!(row <= ROW_BYTES, "row/output peak requires {row}");
}

#[tokio::test]
async fn lost_setup_reply_and_duplicate_ensure_recover_original_session_without_rng_or_crypto() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let mut rng = Rng::new();
    let first = ready(ensure(&f.state, &owner, [9; 32], &mut rng).await.unwrap());
    assert!(rng.draws > 0);
    let before = selected.resources().counts();
    let group = selected.binding().scope.group;
    let held = selected
        .resources()
        .charge(
            group,
            256 * 1024 - before.0 + INDEX_BYTES,
            ResourceKind::Retained,
        )
        .unwrap();
    let mut unused = Rng::new();
    let recovered = ready(
        ensure(&f.state, &owner, [9; 32], &mut unused)
            .await
            .unwrap(),
    );
    assert!(Arc::ptr_eq(&first, &recovered));
    assert_eq!(first.public.session, recovered.public.session);
    assert_eq!(first.public.transcript(), recovered.public.transcript());
    assert_eq!(unused.draws, 0);
    assert_eq!(
        selected.resources().counts().1,
        2,
        "original setup engine and independent control lane survive reply loss"
    );
    drop(held);
    f.close().await;
}

#[tokio::test]
async fn pending_is_visible_before_rng_and_duplicate_begin_allocates_no_second_work() {
    let f = Fixture::new().await;
    let (_, selected) = select(&f, point()).await;
    let Begin::Work(mut ticket) = f
        .state
        .voice_channels
        .begin(selected.clone(), [8; 32])
        .unwrap()
    else {
        panic!("first work")
    };
    let before = selected.resources().counts();
    let mut rng = Rng::new();
    rng.hook = Some(Box::new(|| {
        assert!(matches!(
            f.state
                .voice_channels
                .begin(selected.clone(), [8; 32])
                .unwrap(),
            Begin::Existing(Status::Pending)
        ));
        assert_eq!(selected.resources().counts(), before);
    }));
    let (session, _, setup) = ticket.compute(&mut rng).unwrap();
    drop((session, setup, ticket));
    drop(rng);
    assert!(matches!(
        f.state
            .voice_channels
            .begin(selected.clone(), [8; 32])
            .unwrap(),
        Begin::Existing(Status::Closed {
            native_setup_drained: true
        })
    ));
    f.close().await;
}

#[tokio::test]
async fn close_first_latches_before_any_setup_payment_or_draw_and_unknown_retired_allocates_none() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    assert!(matches!(
        f.state
            .voice_channels
            .close(&owner, [9; 32], Some(selected.clone()))
            .unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    let before = selected.resources().counts();
    let mut rng = Rng::new();
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut rng).await.unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    assert_eq!(rng.draws, 0);
    assert_eq!(selected.resources().counts(), before);
    assert!(f
        .state
        .voice_channels
        .close("retired-unknown", [9; 32], None)
        .is_err());
    assert_eq!(selected.resources().counts(), before);
    f.close().await;
}

#[tokio::test]
async fn close_during_rng_preserves_pending_custody_then_permanently_refuses_installation() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let mut rng = Rng::new();
    rng.hook = Some(Box::new(|| {
        assert!(matches!(
            f.state.voice_channels.close(&owner, [9; 32], None).unwrap(),
            Status::Closed {
                native_setup_drained: false
            }
        ));
        assert_eq!(
            selected.resources().counts().1,
            2,
            "actual arithmetic and independent control lane remain charged"
        );
    }));
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut rng).await,
        Err(Refused::Binding)
    ));
    drop(rng);
    assert_eq!(selected.resources().counts().1, 0);
    let mut unused = Rng::new();
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut unused)
            .await
            .unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    assert_eq!(unused.draws, 0);
    f.close().await;
}

#[tokio::test]
async fn end_and_atomic_lock_registration_during_rng_each_refuse_final_publication() {
    for lock in [false, true] {
        let f = Fixture::new().await;
        let (owner, selected) = select(&f, point()).await;
        let mut rng = Rng::new();
        rng.hook = Some(Box::new(|| {
            if lock {
                crate::register_ui_lock_request(&f.state);
            } else {
                f.state.voice_authority.lock().unwrap().end(&owner);
            }
        }));
        assert!(matches!(
            ensure(&f.state, &owner, [9; 32], &mut rng).await,
            Err(Refused::Binding)
        ));
        drop(rng);
        assert!(matches!(
            f.state.voice_channels.close(&owner, [9; 32], None).unwrap(),
            Status::Closed {
                native_setup_drained: true
            }
        ));
        assert_eq!(selected.resources().counts().1, 0);
        f.close().await;
    }
}

#[tokio::test]
async fn malformed_curve_point_fails_before_rng_and_leaves_closed_counted_fence() {
    let f = Fixture::new().await;
    let mut invalid = [0; 65];
    invalid[0] = 4;
    let (owner, selected) = select(&f, invalid).await;
    let mut rng = Rng::new();
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut rng).await,
        Err(Refused::Binding)
    ));
    assert_eq!(rng.draws, 0);
    assert_eq!(selected.resources().counts().1, 0);
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut rng).await.unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    f.close().await;
}

#[tokio::test]
async fn unwinding_rng_closes_original_ticket_and_retires_crypto_before_drained_status() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let Begin::Work(mut ticket) = f
        .state
        .voice_channels
        .begin(selected.clone(), [8; 32])
        .unwrap()
    else {
        panic!("work")
    };
    let mut rng = Rng::new();
    rng.hook = Some(Box::new(|| panic!("injected scalar RNG failure")));
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = ticket.compute(&mut rng);
    }));
    assert!(failure.is_err());
    drop(ticket);
    assert_eq!(selected.resources().counts().1, 0);
    assert!(matches!(
        f.state.voice_channels.close(&owner, [9; 32], None).unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    f.close().await;
}

#[tokio::test]
async fn full_group_or_crypto_pressure_refuses_before_rng_without_stranding_a_row() {
    for crypto in [false, true] {
        let f = Fixture::new().await;
        let (owner, selected) = select(&f, point()).await;
        let resources = selected.resources();
        let group = selected.binding().scope.group;
        let mut pressure = Vec::new();
        if crypto {
            for _ in 0..4 {
                pressure.push(resources.charge(group, 0, ResourceKind::Crypto).unwrap());
            }
        } else {
            pressure.push(
                resources
                    .charge(
                        group,
                        256 * 1024 - resources.counts().0,
                        ResourceKind::Retained,
                    )
                    .unwrap(),
            );
        }
        let mut rng = Rng::new();
        assert!(matches!(
            ensure(&f.state, &owner, [9; 32], &mut rng).await,
            Err(Refused::Capacity)
        ));
        assert_eq!(rng.draws, 0);
        drop(pressure);
        assert!(matches!(
            ensure(&f.state, &owner, [9; 32], &mut rng).await.unwrap(),
            Status::Ready(_)
        ));
        f.close().await;
    }
}

#[tokio::test]
async fn monotonic_identity_distinguishes_repeated_rng_and_exhaustion_never_wraps() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let first = ready(
        ensure(&f.state, &owner, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let registry = Registry::default();
    {
        let resources = selected.resources();
        let mut held = registry.index.lock().unwrap();
        Registry::index(&mut held, &resources).unwrap().serial = 1;
    }
    let device = *f
        .state
        .servers
        .lock()
        .await
        .get(&7)
        .unwrap()
        .device_id
        .as_bytes();
    let Begin::Work(mut ticket) = registry.begin(selected.clone(), device).unwrap() else {
        panic!("work")
    };
    let (_, second, _setup) = ticket.compute(&mut Rng::new()).unwrap();
    assert_ne!(first.public.session, second.session);
    assert_eq!(
        &first.public.transcript()[first.public.length - 65..],
        &second.transcript()[second.length - 65..],
        "repeated RNG gave the identical native point"
    );
    assert_ne!(first.public.hash, second.hash);
    // A third private registry permits deterministic counter exhaustion without issuing keys.
    let exhausted = Registry::default();
    {
        let resources = selected.resources();
        let mut held = exhausted.index.lock().unwrap();
        Registry::index(&mut held, &resources).unwrap().serial = u128::MAX;
    }
    let before = selected.resources().counts();
    assert!(matches!(
        exhausted.begin(selected, [8; 32]),
        Err(Refused::Exhausted)
    ));
    assert_eq!(
        before,
        f.state
            .voice_authority
            .lock()
            .unwrap()
            .runtime()
            .resources()
            .counts()
    );
    f.close().await;
}

#[tokio::test]
async fn exact_replies_share_one_paid_projection_until_last_output_after_registry_drop() {
    let mut f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let first = ready(
        ensure(&f.state, &owner, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let before = selected.resources().counts();
    let mut replies = Vec::new();
    for _ in 0..64 {
        let mut unused = Rng::new();
        let reply = ready(
            ensure(&f.state, &owner, [9; 32], &mut unused)
                .await
                .unwrap(),
        );
        assert!(Arc::ptr_eq(&first, &reply));
        assert_eq!(unused.draws, 0);
        replies.push(reply);
    }
    assert_eq!(selected.resources().counts(), before);
    let old = std::mem::take(&mut f.state.voice_channels);
    drop(old);
    assert_eq!(
        selected.resources().counts(),
        (
            before.0
                - INDEX_BYTES
                - super::super::channel::CONTROL_BYTES
                - super::super::channel::BOOTSTRAP_BYTES
                - super::super::channel::BROWSER_CONTROL_BYTES
                - super::super::channel::bootstrap::SETUP_WORK_BYTES,
            0
        )
    );
    drop(replies);
    let retained = selected.resources().counts();
    assert!(
        first.public().transcript().len() > 65,
        "borrowed output remains backed after registry removal"
    );
    drop(first);
    assert_eq!(selected.resources().counts(), (retained.0 - ROW_BYTES, 0));
    f.close().await;
}

#[tokio::test]
async fn exact_setup_settlement_after_reply_loss_and_lock_releases_only_setup_and_never_reopens() {
    for lock in [false, true] {
        let f = Fixture::new().await;
        let (owner, selected) = select(&f, point()).await;
        let _unobserved = ready(
            ensure(&f.state, &owner, [9; 32], &mut Rng::new())
                .await
                .unwrap(),
        );
        let before = selected.resources().counts();
        assert_eq!(before.1, 2);
        assert!(matches!(
            settle_setup(&f.state, &owner, [8; 32]).await,
            Err(Refused::Binding)
        ));
        assert_eq!(selected.resources().counts(), before);
        if lock {
            crate::register_ui_lock_request(&f.state);
        } else {
            f.state.voice_authority.lock().unwrap().end(&owner);
        }
        // End/lock separately retires the actual Root CallGrant. Measure setup retirement
        // AFTER that independent effect; it must release exactly its own original8KiB/slot.
        let before_cleanup = selected.resources().counts();
        assert_eq!(before_cleanup.1, 2);
        assert!(matches!(
            settle_setup(&f.state, &owner, [9; 32]).await.unwrap(),
            Status::Closed {
                native_setup_drained: true
            }
        ));
        let retired = selected.resources().counts();
        assert_eq!(
            retired,
            (
                before_cleanup.0 - super::super::channel::bootstrap::SETUP_WORK_BYTES,
                1
            )
        );
        assert!(matches!(
            settle_setup(&f.state, &owner, [9; 32]).await.unwrap(),
            Status::Closed { .. }
        ));
        assert_eq!(selected.resources().counts(), retired);
        if lock {
            let generation = f
                .state
                .ui_session_generation
                .load(std::sync::atomic::Ordering::Acquire);
            crate::finalize_unlock_session(&f.state, generation, false)
                .await
                .unwrap();
        }
        let mut unused = Rng::new();
        assert!(matches!(
            ensure(&f.state, &owner, [9; 32], &mut unused).await,
            Err(Refused::Binding)
        ));
        assert_eq!(unused.draws, 0);
        assert_eq!(selected.resources().counts(), retired);
        f.close().await;
    }
}

#[tokio::test]
async fn trusted_close_first_fences_delayed_setup_and_unknown_retired_cleanup_allocates_nothing() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    assert!(matches!(
        settle_setup(&f.state, &owner, [9; 32]).await.unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    let before = selected.resources().counts();
    assert_eq!(before.1, 0);
    let mut unused = Rng::new();
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut unused)
            .await
            .unwrap(),
        Status::Closed { .. }
    ));
    assert_eq!(unused.draws, 0);
    assert_eq!(selected.resources().counts(), before);
    assert!(matches!(
        settle_setup(&f.state, "retired-unknown", [9; 32])
            .await
            .unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    assert_eq!(selected.resources().counts(), before);
    f.close().await;
}

#[tokio::test]
async fn one_free_crypto_slot_rolls_back_preparation_without_rng_row_or_serial_changes() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let resources = selected.resources();
    f.state.voice_channels.bind_pool(&resources).unwrap();
    let lanes = (0..3)
        .map(|_| {
            resources
                .reserve_crypto_lane(selected.binding().scope.group)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let before = resources.counts();
    let mut rng = Rng::new();
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut rng).await,
        Err(Refused::Capacity)
    ));
    assert_eq!(rng.draws, 0);
    assert_eq!(resources.counts(), before);
    {
        let held = f.state.voice_channels.index.lock().unwrap();
        let index = held.as_ref().unwrap();
        assert_eq!(index.serial, 0);
        assert!(index.rows.iter().all(Option::is_none));
    }
    for lane in lanes {
        lane.close();
    }
    let reply = ready(ensure(&f.state, &owner, [9; 32], &mut rng).await.unwrap());
    assert_eq!(u128::from_be_bytes(reply.public().session), 1);
    assert!(rng.draws > 0);
    f.close().await;
}

#[tokio::test]
async fn lock_before_first_admission_has_authoritative_no_work_cleanup_without_allocating_a_row() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    crate::register_ui_lock_request(&f.state);
    // The atomic lock fence can precede Coordinator revocation. Neither rejected setup nor
    // the no-work cleanup may allocate/release channel capacity in this ordering.
    let before = selected.resources().counts();
    let mut rng = Rng::new();
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut rng).await,
        Err(Refused::Binding)
    ));
    assert_eq!(rng.draws, 0);
    assert!(matches!(
        settle_setup(&f.state, &owner, [9; 32]).await.unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    assert_eq!(selected.resources().counts(), before);
    assert!(f.state.voice_channels.index.lock().unwrap().is_none());
    // End does not turn the authoritative no-work result into an unresolvable Binding error.
    f.state.voice_authority.lock().unwrap().end(&owner);
    let after_end = selected.resources().counts();
    assert!(matches!(
        settle_setup(&f.state, &owner, [9; 32]).await.unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    assert_eq!(selected.resources().counts(), after_end);
    assert!(f.state.voice_channels.index.lock().unwrap().is_none());
    f.close().await;
}
