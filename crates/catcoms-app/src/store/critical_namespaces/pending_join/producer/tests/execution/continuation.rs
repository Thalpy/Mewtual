//! Real original owner/cache + actual reserved retry revision and same owned transport.
//! Authored UNRUN under the compiler/test/build pause. No SDK/Current claim.
use super::*;

fn independent_control(store: &ServerStore) {
    let capture = store
        .capture_chat_admission(crate::store::ChatAdmissionKey::new(&[0xc1], [0xc1; 32]).unwrap())
        .unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xf1ff);
    let prepared = store
        .prepare_legacy_chat_admission(
            &capture,
            b"independent namespace control while retry reserved",
            &mut rng,
        )
        .unwrap();
    let current = store
        .finish_chat_admission(&prepared, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    assert!(store
        .with_chat_admission_storage(&current, |_| Ok(()))
        .is_ok());
}

async fn save_owner_result_without_polling_reply(
    f: &mut Fixture,
    network: &mut CheckpointJoinNetwork<MemNetwork>,
) {
    let mut flight = Box::pin(network.exchange_armed());
    assert!(futures::poll!(&mut flight).is_pending());
    let mut rng = ChaCha20Rng::seed_from_u64(0xf101);
    tokio::time::timeout(Duration::from_secs(10), async {
        for _ in 0..64 {
            f.owner
                .run_owner_checkpoint_once(&f.source, &mut rng)
                .await
                .unwrap();
            if f.owner
                .mint_owner_checkpoint_invite(&f.source, [0xff; 16], 100_000, vec![])
                .is_err()
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("bounded original owner never covered the first Add");
    })
    .await
    .expect("owner result must cross its actual save barrier within harness bound");
    drop(flight); // The response may exist below; it NEVER entered the actual client cell.
    assert_eq!(network.status(), Status::Spent);
}

#[tokio::test]
async fn exact_retry_grant_keeps_same_kp_request_and_old_observer_closed_after_lost_owner_reply() {
    let mut f = fixture(1);
    let invite = f.invite(100);
    let mut network = start(&mut f, invite, 0xf102);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let first = row(f.member());
    let old_negative = network.cancellation();
    save_owner_result_without_polling_reply(&mut f, &mut network).await;
    assert_eq!(
        network
            .ensure_retry_grant(f.member(), 3, &mut ChaCha20Rng::seed_from_u64(0xf103))
            .unwrap(),
        Status::Spent
    );
    assert!(old_negative.is_cancelled());
    assert!(!network.cancellation().is_cancelled());
    let granted = row(f.member());
    let state = granted.execution.unwrap();
    assert_eq!(
        (state.grant, state.limit, state.spent, state.ordinal),
        (2, 3, 0, 1)
    );
    assert_eq!(granted.revision, 2);
    assert!(state.arm == first.execution.unwrap().arm);
    assert_eq!(granted.package, first.package);
    assert_eq!(granted.request, first.request);
    assert_eq!(granted.ciphertext, first.ciphertext);
    let saved = f.current_bytes();
    for _ in 0..3 {
        assert_eq!(
            network
                .ensure_retry_grant(f.member(), 3, &mut ChaCha20Rng::seed_from_u64(0xf104))
                .unwrap(),
            Status::Spent
        );
        assert_eq!(f.current_bytes(), saved);
    }
    assert!(network
        .ensure_retry_grant(f.member(), 4, &mut ChaCha20Rng::seed_from_u64(0xf105))
        .is_err());
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    receive(&mut f, &mut network).await; // Source is already generation2: only exact cached result may answer.
    network
        .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(0xf106))
        .unwrap();
    let input = row(f.member()).execution.unwrap().input.unwrap();
    assert_eq!(input.arm.ordinal, 2);
    assert_eq!(input.arm.grant, 2);
}

#[test]
fn original_reserved_retry_revision_cannot_be_stolen_by_genuine_coalesced_caller_after_repair() {
    let mut f = fixture(2);
    let invite = f.invite(101);
    let mut network = start(&mut f, invite.clone(), 0xf107);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    f.member()
        .critical_namespace_fault
        .store(1, Ordering::SeqCst);
    assert!(network
        .ensure_retry_grant(f.member(), 4, &mut ChaCha20Rng::seed_from_u64(0xf108))
        .is_err());
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    let contender = f.begin(invite); // Actual source opener repairs r2 file/root before returning.
    let reserved = row(f.member());
    assert_eq!(reserved.revision, 2);
    assert_eq!(reserved.execution.unwrap().grant, 2);
    independent_control(f.member());
    let returned = contender
        .activate_original_transport(
            f.member(),
            f.hub.join(PeerId::from_u64(0xf109)),
            PeerId::from_u64(0x8101),
        )
        .unwrap_err();
    let (same_source, same_transport, _) = returned.into_parts();
    assert_eq!(same_source.metadata.package, reserved.package);
    assert_eq!(
        catcoms_rt::MeshTransport::local_peer(&same_transport),
        PeerId::from_u64(0xf109)
    );
    drop(same_source);
    drop(same_transport);
    assert_eq!(
        network
            .ensure_retry_grant(f.member(), 4, &mut ChaCha20Rng::seed_from_u64(0xf10a))
            .unwrap(),
        Status::Spent
    );
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
}

#[test]
fn drop_of_reserved_retry_owner_preserves_exact_data_but_never_renews_its_same_revision() {
    for fault in [3, 5, 4, 1, 2] {
        let mut f = fixture(2);
        let invite = f.invite(102);
        let mut network = start(&mut f, invite.clone(), 0xf10b);
        assert_eq!(arm(&mut network, f.member()), Status::Armed);
        f.member()
            .critical_namespace_fault
            .store(fault, Ordering::SeqCst);
        assert!(network
            .ensure_retry_grant(f.member(), 3, &mut ChaCha20Rng::seed_from_u64(0xf10c))
            .is_err());
        let candidate = {
            let mounted = f.member().pending_join_catalog.lock().unwrap();
            mounted
                .as_ref()
                .unwrap()
                .pending
                .as_ref()
                .unwrap()
                .bytes
                .to_vec()
        };
        drop(network);
        f.member()
            .critical_namespace_fault
            .store(0, Ordering::SeqCst);
        f.member().retry_pending_join_catalog().unwrap();
        assert_eq!(f.current_bytes(), candidate);
        let mut source = f.begin(invite.clone());
        let failed = source
            .activate_original_transport(
                f.member(),
                f.hub.join(PeerId::from_u64(0xf10d)),
                PeerId::from_u64(0x8101),
            )
            .unwrap_err();
        source = failed.into_parts().0;
        source
            .pause_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(0xf10e))
            .unwrap();
        source
            .resume_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(0xf10f))
            .unwrap();
        assert_eq!(source.metadata.revision, 4);
        let mut next = source
            .activate_original_transport(
                f.member(),
                f.hub.join(PeerId::from_u64(0xf110)),
                PeerId::from_u64(0x8101),
            )
            .unwrap()
            .into_network_execution(f.member())
            .unwrap();
        assert_eq!(arm(&mut next, f.member()), Status::Armed);
        let state = row(f.member()).execution.unwrap();
        assert_eq!((state.grant, state.spent, state.ordinal), (2, 1, 2));
    }
}

#[test]
fn two_explicit_zero_submit_regrants_preserve_any_earlier_latest_arm_and_lost_return_is_idempotent()
{
    let mut f = fixture(2);
    let invite = f.invite(103);
    let mut network = start(&mut f, invite, 0xf111);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let original = row(f.member()).execution.unwrap().arm;
    network
        .ensure_retry_grant(f.member(), 3, &mut ChaCha20Rng::seed_from_u64(0xf112))
        .unwrap();
    network.next_retry_grant().unwrap();
    network
        .ensure_retry_grant(f.member(), 4, &mut ChaCha20Rng::seed_from_u64(0xf113))
        .unwrap();
    let state = row(f.member()).execution.unwrap();
    assert_eq!((state.grant, state.spent, state.ordinal), (3, 0, 1));
    assert!(state.arm == original);
    let saved = f.current_bytes();
    network
        .ensure_retry_grant(f.member(), 4, &mut ChaCha20Rng::seed_from_u64(0xf114))
        .unwrap();
    assert_eq!(f.current_bytes(), saved);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let next = row(f.member()).execution.unwrap();
    assert_eq!((next.grant, next.spent, next.ordinal), (3, 1, 2));
}

#[tokio::test]
async fn received_input_refuses_retry_grant_without_dropping_bytes_or_changing_units() {
    let mut f = fixture(2);
    let invite = f.invite(104);
    let mut network = start(&mut f, invite, 0xf115);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    receive(&mut f, &mut network).await;
    network
        .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(0xf116))
        .unwrap();
    let before = f.current_bytes();
    assert!(network
        .ensure_retry_grant(f.member(), 4, &mut ChaCha20Rng::seed_from_u64(0xf117))
        .is_err());
    assert_eq!(f.current_bytes(), before);
    assert_eq!(network.status(), Status::InputHeld);
}

#[test]
fn close_of_pending_retry_cancels_prospective_owner_and_data_repair_cannot_activate_it() {
    let mut f = fixture(2);
    let invite = f.invite(105);
    let mut network = start(&mut f, invite.clone(), 0xf118);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    f.member()
        .critical_namespace_fault
        .store(1, Ordering::SeqCst);
    assert!(network
        .ensure_retry_grant(f.member(), 4, &mut ChaCha20Rng::seed_from_u64(0xf119))
        .is_err());
    network.close();
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    f.member().retry_pending_join_catalog().unwrap();
    assert!(network
        .ensure_retry_grant(f.member(), 4, &mut ChaCha20Rng::seed_from_u64(0xf11a))
        .is_err());
    assert_eq!(network.status(), Status::Closed);
    let source = f.begin(invite);
    assert!(source
        .activate_original_transport(
            f.member(),
            f.hub.join(PeerId::from_u64(0xf11b)),
            PeerId::from_u64(0x8101)
        )
        .is_err());
    assert_eq!(
        row(f.member()).execution.unwrap().grant,
        2,
        "repair preserves data, never renews sender"
    );
}

struct NeverRetryRng;
impl rand_core::RngCore for NeverRetryRng {
    fn next_u32(&mut self) -> u32 {
        panic!("paused source reached retry RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("paused source reached retry RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("paused source reached retry RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("paused source reached retry RNG")
    }
}
impl rand_core::CryptoRng for NeverRetryRng {}
#[test]
fn actual_paused_source_refuses_executable_retry_without_rng_units_or_byte_changes() {
    let mut f = fixture(2);
    let invite = f.invite(106);
    let mut network = start(&mut f, invite.clone(), 0xf11c);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let mut control = f.begin(invite);
    control
        .pause_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(0xf11d))
        .unwrap();
    let before = f.current_bytes();
    let state = row(f.member()).execution.unwrap();
    assert!(network
        .ensure_retry_grant(f.member(), 4, &mut NeverRetryRng)
        .is_err());
    assert_eq!(f.current_bytes(), before);
    assert!(row(f.member()).execution == Some(state));
    assert_eq!(
        network.status(),
        Status::Closed,
        "old context cannot adopt a newer paused schedule"
    );
}
