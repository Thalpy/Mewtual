//! Actual configured Store arms + ONE owned MemNetwork + real saved owner reply. These
//! tests are authored UNRUN under the user compilation/test/build pause. No SDK acceptance,
//! member Current or physical/network test is inferred from retaining signed INPUT DATA.
mod continuation;
use super::*;
use crate::store::{
    CheckpointJoinExecutionPolicy, CheckpointJoinNetwork, CheckpointJoinSubmissionStatus as Status,
};
use std::time::Duration;

fn execution_budget() -> OwnerEvidenceBudget {
    OwnerEvidenceBudget::checkpoint(
        2,
        256 * 1024 * 1024,
        256 * 1024 * 1024,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap()
}
fn execution_store(
    path: &Path,
    rng: &mut ChaCha20Rng,
    pool: &CriticalCatalogReadPool,
    limit: u32,
) -> ServerStore {
    let policy = CheckpointJoinExecutionPolicy::new(
        pool.clone(),
        2 * codec::MAX_CATALOG_FRAMED,
        limit,
        10_000,
    )
    .unwrap();
    let store =
        ServerStore::open_with_checkpoint_join_execution(path, SECRET, rng, policy).unwrap();
    store
        .configure_owner_evidence_budget(execution_budget())
        .unwrap();
    store
}
fn fixture(limit: u32) -> Fixture {
    let source_dir = tempfile::tempdir().unwrap();
    let member_dir = tempfile::tempdir().unwrap();
    let mut source_rng = ChaCha20Rng::seed_from_u64(0xe001);
    let mut member_rng = ChaCha20Rng::seed_from_u64(0xe002);
    let source = ServerStore::open(source_dir.path(), SECRET, &mut source_rng)
        .unwrap()
        .into_critical_namespace_store(SECRET, &mut source_rng)
        .unwrap();
    source
        .configure_owner_evidence_budget(execution_budget())
        .unwrap();
    let hub = Hub::new();
    let owner = founder(&source, &hub, 0x8101, &mut source_rng);
    drop(plain_store(member_dir.path(), &mut member_rng));
    let pool = CriticalCatalogReadPool::new(512 * 1024 * 1024).unwrap();
    let member = Arc::new(execution_store(
        member_dir.path(),
        &mut member_rng,
        &pool,
        limit,
    ));
    Fixture {
        owner,
        source,
        member: Some(member),
        pool,
        member_rng,
        hub,
        _source_dir: source_dir,
        member_dir,
    }
}
fn start(f: &mut Fixture, invite: InviteToken, peer: u64) -> CheckpointJoinNetwork<MemNetwork> {
    let mut prepared = f.begin(invite);
    prepared
        .save_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(peer))
        .unwrap();
    prepared
        .activate_original_transport(
            f.member(),
            f.hub.join(PeerId::from_u64(peer)),
            PeerId::from_u64(0x8101),
        )
        .unwrap()
        .into_network_execution(f.member())
        .unwrap()
}
fn row(store: &ServerStore) -> codec::Metadata {
    store
        .pending_join_summary()
        .unwrap()
        .rows
        .into_iter()
        .flatten()
        .next()
        .unwrap()
}
fn arm(network: &mut CheckpointJoinNetwork<MemNetwork>, store: &ServerStore) -> Status {
    network
        .ensure_submission_armed(
            store,
            &mut ChaCha20Rng::seed_from_u64(0xe003),
            Arc::new(ManualClock::new(1_000)),
        )
        .unwrap()
}
async fn receive(f: &mut Fixture, network: &mut CheckpointJoinNetwork<MemNetwork>) {
    let mut source_rng = ChaCha20Rng::seed_from_u64(0xe004);
    let exchange = network.exchange_armed();
    let drive = async {
        for _ in 0..64 {
            f.owner
                .run_owner_checkpoint_once(&f.source, &mut source_rng)
                .await
                .unwrap();
            // Harness fairness only: even an accidentally always-ready driver must return to
            // the awaiting exchange/timeout rather than spin past either test bound.
            tokio::task::yield_now().await;
        }
        panic!("owner event bound exhausted before the exact original response");
    };
    tokio::pin!(exchange, drive);
    let status=tokio::time::timeout(Duration::from_secs(10),async {
        tokio::select! { result=&mut exchange=>result.unwrap(), _=&mut drive=>unreachable!("bounded driver runs until exchange wins") }
    }).await.expect("real owner/network exchange must complete within harness bound");
    assert_eq!(status, Status::InputHeld);
}

#[test]
fn saved_arm_lost_return_retries_exact_invocation_without_another_unit_or_ciphertext() {
    let mut f = fixture(2);
    let invite = f.invite(70);
    let mut network = start(&mut f, invite, 0xe010);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let saved = f.current_bytes();
    let original = row(f.member());
    let state = original.execution.unwrap();
    assert_eq!(
        (state.grant, state.limit, state.spent, state.ordinal),
        (1, 2, 1, 1)
    );
    for _ in 0..3 {
        assert_eq!(arm(&mut network, f.member()), Status::Armed);
        assert_eq!(f.current_bytes(), saved);
    }
    assert!(row(f.member()) == original);
    assert!(
        network.next_submission().is_err(),
        "an Armed ticket cannot be replayed/replaced"
    );
}

#[test]
fn exact_admitted_arm_repairs_after_caller_task_drop_but_never_becomes_a_new_ticket() {
    for fault in [3, 5, 4, 1, 2] {
        let mut f = fixture(1);
        let invite = f.invite(71);
        let mut network = start(&mut f, invite, 0xe011);
        network
            .ensure_initial_grant(f.member(), &mut ChaCha20Rng::seed_from_u64(11))
            .unwrap();
        // Interrupt the actual debit after migration/grant already crossed their barriers.
        f.member()
            .critical_namespace_fault
            .store(fault, Ordering::SeqCst);
        assert!(network
            .ensure_submission_armed(
                f.member(),
                &mut ChaCha20Rng::seed_from_u64(1),
                Arc::new(ManualClock::new(1_000))
            )
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
        network.close();
        drop(network);
        f.member()
            .critical_namespace_fault
            .store(0, Ordering::SeqCst);
        f.member().retry_pending_join_catalog().unwrap();
        assert_eq!(f.current_bytes(), candidate);
        assert!(f
            .member()
            .pending_join_catalog
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .pending
            .is_none());
    }
}

#[tokio::test]
async fn real_signed_reply_is_durable_input_once_and_never_a_member_current_or_sdk_result() {
    let mut f = fixture(2);
    let invite = f.invite(72);
    let mut network = start(&mut f, invite, 0xe012);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let before = row(f.member());
    receive(&mut f, &mut network).await;
    assert!(
        row(f.member()) == before,
        "network receipt alone does not change the journal"
    );
    network
        .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(2))
        .unwrap();
    let saved = f.current_bytes();
    let after = row(f.member());
    let mut expected = before.clone();
    expected.execution.as_mut().unwrap().input = after.execution.unwrap().input;
    assert!(after == expected);
    assert!(after.execution.unwrap().input.is_some());
    network.close();
    network
        .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(3))
        .unwrap();
    assert_eq!(
        f.current_bytes(),
        saved,
        "same input cache cannot reseal on a lost return"
    );
    assert_eq!(network.status(), Status::InputHeld);
    assert!(network.next_submission().is_err());
    let capture = f
        .member()
        .capture_chat_admission(
            crate::store::ChatAdmissionKey::new(after.group.as_slice(), after.signer).unwrap(),
        )
        .unwrap();
    assert!(
        JournalBasis::from_capture(&capture) == after.basis,
        "signed DATA never advances admission"
    );
}

#[tokio::test]
async fn held_reply_survives_pause_and_newer_schedule_without_changing_newer_counters() {
    let mut f = fixture(2);
    let invite = f.invite(73);
    let mut network = start(&mut f, invite.clone(), 0xe013);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    receive(&mut f, &mut network).await;
    let mut control = f.begin(invite);
    control
        .pause_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(4))
        .unwrap();
    let before = row(f.member());
    assert_eq!(before.revision, 2);
    network
        .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(5))
        .unwrap();
    let after = row(f.member());
    let mut expected = before;
    expected.execution.as_mut().unwrap().input = after.execution.unwrap().input;
    assert!(after == expected);
    assert!(after.execution.unwrap().input.is_some());
}

#[tokio::test]
async fn input_store_retains_exact_candidate_across_every_warm_write_fault_and_owner_drop() {
    for fault in [3, 5, 4, 1, 2] {
        let mut f = fixture(2);
        let invite = f.invite(74);
        let mut network = start(&mut f, invite, 0xe014);
        assert_eq!(arm(&mut network, f.member()), Status::Armed);
        receive(&mut f, &mut network).await;
        f.member()
            .critical_namespace_fault
            .store(fault, Ordering::SeqCst);
        assert!(network
            .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(6))
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
        assert!(row(f.member()).execution.unwrap().input.is_some());
    }
}

#[tokio::test]
async fn cancelled_submission_stays_spent_and_exact_source_can_retry_without_second_owner_add() {
    let mut f = fixture(2);
    let invite = f.invite(75);
    let mut network = start(&mut f, invite.clone(), 0xe015);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    {
        let mut flight = Box::pin(network.exchange_armed());
        assert!(futures::poll!(&mut flight).is_pending());
    }
    assert_eq!(network.status(), Status::Spent);
    let first = row(f.member());
    assert_eq!(first.execution.unwrap().spent, 1);
    network.next_submission().unwrap();
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let second = row(f.member());
    assert_eq!(second.execution.unwrap().spent, 2);
    assert_eq!(second.package, first.package);
    assert_eq!(second.request, first.request);
    receive(&mut f, &mut network).await;
    network
        .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(7))
        .unwrap();
    // Real owner exact original request cache remains available. Merely retaining this input
    // cannot consume another membership operation or introduce a different package.
    assert!(row(f.member()).execution.unwrap().input.is_some());
}

#[test]
fn cold_reopen_preserves_spent_arm_and_exhaustion_does_not_get_a_fresh_grant() {
    let mut f = fixture(1);
    let invite = f.invite(76);
    let mut network = start(&mut f, invite.clone(), 0xe016);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let before = row(f.member());
    drop(network);
    drop(f.member.take());
    f.member = Some(Arc::new(execution_store(
        f.member_dir.path(),
        &mut f.member_rng,
        &f.pool,
        64,
    )));
    let mut next = start(&mut f, invite, 0xe017);
    assert_eq!(arm(&mut next, f.member()), Status::Exhausted);
    assert!(
        row(f.member()) == before,
        "new configured limit cannot rewrite the saved original grant"
    );
}

#[tokio::test]
async fn default_and_pending_readers_preserve_execution_sources_and_reject_before_old_cleaners() {
    let mut f = fixture(1);
    let invite = f.invite(77);
    let mut network = start(&mut f, invite, 0xe018);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    receive(&mut f, &mut network).await;
    network
        .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(8))
        .unwrap();
    let original = f.current_bytes();
    drop(network);
    drop(f.member.take());
    assert!(ServerStore::open(f.member_dir.path(), SECRET, &mut f.member_rng).is_err());
    let policy =
        PendingJoinCatalogPolicy::new(f.pool.clone(), 2 * codec::MAX_CATALOG_FRAMED).unwrap();
    assert!(ServerStore::open_with_pending_join_catalog(
        f.member_dir.path(),
        SECRET,
        &mut f.member_rng,
        policy
    )
    .is_err());
    assert_eq!(
        fs::read(f.member_dir.path().join(Slot::Current.name())).unwrap(),
        original
    );
    let reopened = execution_store(f.member_dir.path(), &mut f.member_rng, &f.pool, 1);
    assert!(row(&reopened).execution.unwrap().input.is_some());
}

#[tokio::test]
async fn old_mount_cannot_persist_held_input_into_a_new_mount_or_send_from_its_old_task() {
    let mut f = fixture(2);
    let invite = f.invite(78);
    let mut network = start(&mut f, invite, 0xe019);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    receive(&mut f, &mut network).await;
    drop(f.member.take());
    f.member = Some(Arc::new(execution_store(
        f.member_dir.path(),
        &mut f.member_rng,
        &f.pool,
        2,
    )));
    let before = f.current_bytes();
    assert!(network
        .persist_received_input(f.member(), &mut ChaCha20Rng::seed_from_u64(9))
        .is_err());
    assert!(
        network
            .ensure_submission_armed(
                f.member(),
                &mut ChaCha20Rng::seed_from_u64(10),
                Arc::new(ManualClock::new(1_000))
            )
            .unwrap()
            == Status::InputHeld
    );
    assert_eq!(f.current_bytes(), before);
}

#[test]
fn negative_closure_after_actual_admission_before_catalog_wait_rejects_first_arm_assignment() {
    let mut f = fixture(2);
    let invite = f.invite(79);
    let mut network = start(&mut f, invite, 0xe020);
    network
        .ensure_initial_grant(f.member(), &mut ChaCha20Rng::seed_from_u64(12))
        .unwrap();
    let before = f.current_bytes();
    let negative = f.member().chat_mount_revoker();
    *f.member()
        .pending_join_before_catalog_writer
        .lock()
        .unwrap() = Some(Box::new(move || negative.revoke()));
    assert!(network
        .ensure_submission_armed(
            f.member(),
            &mut ChaCha20Rng::seed_from_u64(13),
            Arc::new(ManualClock::new(1_000))
        )
        .is_err());
    assert_eq!(f.current_bytes(), before);
    assert!(f
        .member()
        .pending_join_catalog
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .pending
        .is_none());
}

#[tokio::test]
async fn received_input_data_permit_survives_actual_original_row_retirement_but_cannot_admit_grants(
) {
    let mut f = fixture(2);
    let invite = f.invite(83);
    let device = MlsDevice::generate().unwrap();
    let signer: [u8; 32] = device.public_key_bytes().try_into().unwrap();
    let capture = f
        .member()
        .capture_chat_admission(
            crate::store::ChatAdmissionKey::new(&invite.group_id, signer).unwrap(),
        )
        .unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(14);
    let reservation = f
        .member()
        .prepare_legacy_chat_admission(
            &capture,
            b"actual preexisting row for negative-only boundary",
            &mut rng,
        )
        .unwrap();
    let binding = f
        .member()
        .finish_chat_admission(&reservation, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    let mut preparation = f
        .member
        .as_deref()
        .unwrap()
        .prepare_journalled_owner_checkpoint_join(
            &f.source,
            &f.owner,
            device,
            invite,
            &mut f.member_rng,
        )
        .unwrap();
    preparation.save_pending(f.member(), &mut rng).unwrap();
    let mut network = preparation
        .activate_original_transport(
            f.member(),
            f.hub.join(PeerId::from_u64(0xe021)),
            PeerId::from_u64(0x8101),
        )
        .unwrap()
        .into_network_execution(f.member())
        .unwrap();
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    receive(&mut f, &mut network).await;
    f.member()
        .retire_chat_admission(&binding, &mut rng)
        .unwrap();
    let before = row(f.member());
    f.member()
        .critical_namespace_fault
        .store(3, Ordering::SeqCst);
    assert!(network
        .persist_received_input(f.member(), &mut rng)
        .is_err());
    {
        let mut mounted = f.member().pending_join_catalog.lock().unwrap();
        let candidate = mounted.as_mut().unwrap().pending.as_mut().unwrap();
        assert!(candidate.validate_received_input_change().is_ok());
        let next = candidate.next.clone();
        let changed = candidate
            .next
            .rows
            .iter_mut()
            .flatten()
            .find(|entry| entry.source == before.source)
            .unwrap();
        changed.execution.as_mut().unwrap().grant += 1;
        assert!(
            candidate.validate_received_input_change().is_err(),
            "DATA-only permission cannot smuggle a regrant"
        );
        candidate.next = next;
    }
    drop(network);
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    f.member().retry_pending_join_catalog().unwrap();
    let after = row(f.member());
    let mut expected = before;
    expected.execution.as_mut().unwrap().input = after.execution.unwrap().input;
    assert!(after == expected);
    assert!(after.execution.unwrap().input.is_some());
    assert!(f
        .member()
        .with_chat_admission_storage_permit(&binding, |_| Ok(()))
        .is_err());
}

#[test]
fn combined_archive_declaration_preserves_version_two_source_and_first_grant_without_reset() {
    use crate::store::{ArchiveBootstrapCatalogPolicy, PreparedArchiveBootstrapStore};
    let mut f = fixture(2);
    let invite = f.invite(84);
    let mut network = start(&mut f, invite.clone(), 0xe022);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    let before = row(f.member());
    drop(network);
    drop(f.member.take());
    let policy = ArchiveBootstrapCatalogPolicy::new(
        f.pool.clone(),
        4 * super::super::super::super::archive_bootstrap::MAX_SEED_FRAMED,
    )
    .unwrap()
    .with_checkpoint_join_execution(64, 10_000)
    .unwrap();
    let mut declaration =
        PreparedArchiveBootstrapStore::open(f.member_dir.path(), SECRET, &mut f.member_rng, policy)
            .unwrap();
    declaration.retry(&mut f.member_rng).unwrap();
    let store = declaration.into_store().unwrap();
    store
        .configure_owner_evidence_budget(execution_budget())
        .unwrap();
    f.member = Some(Arc::new(store));
    assert!(row(f.member()) == before);
    let bytes = f.current_bytes();
    let key = f.member().keys.db_key().unwrap();
    let plain = Zeroizing::new(unseal(&key, &unframe(&bytes).unwrap()).unwrap());
    let parsed = f.member().parse_configured_join_catalog(&plain).unwrap();
    let archive = parsed.archive_payload.unwrap().to_vec();
    let mut next = start(&mut f, invite, 0xe023);
    assert_eq!(arm(&mut next, f.member()), Status::Armed);
    let after = row(f.member());
    assert_eq!(after.execution.unwrap().limit, 2);
    assert_eq!(after.execution.unwrap().spent, 2);
    assert_eq!(after.package, before.package);
    assert_eq!(after.request, before.request);
    assert_eq!(after.ciphertext, before.ciphertext);
    let updated = f.current_bytes();
    let updated_plain = Zeroizing::new(unseal(&key, &unframe(&updated).unwrap()).unwrap());
    assert_eq!(
        f.member()
            .parse_configured_join_catalog(&updated_plain)
            .unwrap()
            .archive_payload
            .unwrap(),
        archive
    );
}

#[tokio::test]
async fn dropping_network_owner_after_verified_reply_cannot_drop_only_input_or_prevent_store_save()
{
    let mut f = fixture(2);
    let invite = f.invite(85);
    let mut network = start(&mut f, invite, 0xe024);
    assert_eq!(arm(&mut network, f.member()), Status::Armed);
    receive(&mut f, &mut network).await;
    let original = row(f.member());
    assert!(original.execution.unwrap().input.is_none());
    drop(network);
    assert_eq!(
        f.member()
            .persist_retained_checkpoint_join_inputs(&mut ChaCha20Rng::seed_from_u64(15))
            .unwrap(),
        1
    );
    assert!(row(f.member()).execution.unwrap().input.is_some());
    let bytes = f.current_bytes();
    assert_eq!(
        f.member()
            .persist_retained_checkpoint_join_inputs(&mut ChaCha20Rng::seed_from_u64(16))
            .unwrap(),
        1
    );
    assert_eq!(
        f.current_bytes(),
        bytes,
        "Store recovery never repeats the request or re-seals an already saved input"
    );
}

#[test]
fn actual_row_retirement_after_counter_admission_repairs_only_same_data_without_a_ticket() {
    let mut f = fixture(2);
    let invite = f.invite(86);
    let device = MlsDevice::generate().unwrap();
    let signer = device.public_key_bytes().try_into().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(17);
    let capture = f
        .member()
        .capture_chat_admission(
            crate::store::ChatAdmissionKey::new(&invite.group_id, signer).unwrap(),
        )
        .unwrap();
    let old = f
        .member()
        .prepare_legacy_chat_admission(&capture, b"real row before journal source", &mut rng)
        .unwrap();
    let binding = f
        .member()
        .finish_chat_admission(&old, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    let mut source = f
        .member
        .as_deref()
        .unwrap()
        .prepare_journalled_owner_checkpoint_join(
            &f.source,
            &f.owner,
            device,
            invite,
            &mut f.member_rng,
        )
        .unwrap();
    source.save_pending(f.member(), &mut rng).unwrap();
    let mut network = source
        .activate_original_transport(
            f.member(),
            f.hub.join(PeerId::from_u64(0xe025)),
            PeerId::from_u64(0x8101),
        )
        .unwrap()
        .into_network_execution(f.member())
        .unwrap();
    network.ensure_initial_grant(f.member(), &mut rng).unwrap();
    f.member()
        .critical_namespace_fault
        .store(1, Ordering::SeqCst);
    assert!(network
        .ensure_submission_armed(f.member(), &mut rng, Arc::new(ManualClock::new(1_000)))
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
    f.member()
        .retire_chat_admission(&binding, &mut rng)
        .unwrap();
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    f.member().retry_pending_join_catalog().unwrap();
    assert_eq!(f.current_bytes(), candidate);
    assert_eq!(row(f.member()).execution.unwrap().spent, 1);
    assert_eq!(network.status(), Status::Closed);
    assert_eq!(arm(&mut network, f.member()), Status::Closed);
    assert!(network.next_submission().is_err());
    // Repair did not revive the old row or invoke any positive old storage callback.
    assert!(f
        .member()
        .with_chat_admission_storage(&binding, |_| -> Result<(), AppError> {
            panic!("repair cannot renew Current")
        })
        .is_err());
    let other = f
        .member()
        .capture_chat_admission(crate::store::ChatAdmissionKey::new(&[0xee], [0xee; 32]).unwrap())
        .unwrap();
    let preparation = f
        .member()
        .prepare_legacy_chat_admission(&other, b"independent key progresses after repair", &mut rng)
        .unwrap();
    assert!(f
        .member()
        .finish_chat_admission(&preparation, &mut rng, |_, _| Ok(()))
        .is_ok());
}

#[test]
fn retired_repair_identity_cannot_repair_a_later_actual_pending_object_for_the_same_source() {
    use crate::store::chat_admission::journal_basis::JoinCatalogWritePermit;
    let mut f = fixture(2);
    let invite = f.invite(87);
    let mut source = f.begin(invite);
    f.member()
        .critical_namespace_fault
        .store(3, Ordering::SeqCst);
    assert!(source
        .save_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(18))
        .is_err());
    let (original, identity) = {
        let mounted = f.member().pending_join_catalog.lock().unwrap();
        let pending = mounted.as_ref().unwrap().pending.as_ref().unwrap();
        (pending.original.clone(), pending.retained_identity())
    };
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    f.member().retry_pending_join_catalog().unwrap();
    f.member()
        .critical_namespace_fault
        .store(3, Ordering::SeqCst);
    assert!(source
        .pause_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(19))
        .is_err());
    let bytes = f.current_bytes();
    assert!(f
        .member()
        .with_retained_join_catalog_repair(&original.capture, &identity, |permit| f
            .member()
            .settle_pending_join_candidate(JoinCatalogWritePermit::Retained(permit), &original))
        .is_err());
    assert_eq!(f.current_bytes(), bytes);
    assert!(f
        .member()
        .pending_join_catalog
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .pending
        .is_some());
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    f.member().retry_pending_join_catalog().unwrap();
}

#[test]
fn actual_retained_repair_refuses_foreign_stage_and_old_physical_mount_without_touching_bytes() {
    let mut f = fixture(2);
    let invite = f.invite(88);
    let mut source = f.begin(invite);
    f.member()
        .critical_namespace_fault
        .store(3, Ordering::SeqCst);
    assert!(source
        .save_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(20))
        .is_err());
    let (original, identity) = {
        let mounted = f.member().pending_join_catalog.lock().unwrap();
        let pending = mounted.as_ref().unwrap().pending.as_ref().unwrap();
        (pending.original.clone(), pending.retained_identity())
    };
    let old = f.current_bytes();
    let stage = f.member_dir.path().join(Slot::Stage.name());
    fs::write(&stage, b"foreign complete or partial stage").unwrap();
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    assert!(f.member().retry_pending_join_catalog().is_err());
    assert_eq!(f.current_bytes(), old);
    assert_eq!(
        fs::read(&stage).unwrap(),
        b"foreign complete or partial stage"
    );
    fs::remove_file(&stage).unwrap();
    f.member().retry_pending_join_catalog().unwrap();
    drop(source);
    drop(f.member.take());
    f.member = Some(Arc::new(execution_store(
        f.member_dir.path(),
        &mut f.member_rng,
        &f.pool,
        2,
    )));
    let current = f.current_bytes();
    assert!(f
        .member()
        .with_retained_join_catalog_repair(
            &original.capture,
            &identity,
            |_| -> Result<(), AppError> { panic!("old-mount repair must not enter callback") }
        )
        .is_err());
    assert_eq!(f.current_bytes(), current);
}

#[test]
fn valid_bytes_on_an_unadmitted_candidate_identity_cannot_enter_the_repair_callback() {
    let mut f = fixture(2);
    let invite = f.invite(89);
    let mut source = f.begin(invite);
    f.member()
        .critical_namespace_fault
        .store(3, Ordering::SeqCst);
    assert!(source
        .save_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(21))
        .is_err());
    let fee = f.pool.try_reserve(WRITE_FEE).unwrap();
    let counterfeit = {
        let mounted = f.member().pending_join_catalog.lock().unwrap();
        let original = mounted.as_ref().unwrap().pending.as_ref().unwrap();
        // Same genuine byte commitments/root/capture, but this fresh runtime identity was
        // NEVER installed. Only the original Store-owned object may issue a repair callback.
        PendingCandidate {
            identity: Arc::new(()),
            bytes: Zeroizing::new(original.bytes.to_vec()),
            next: original.next.clone(),
            expected: original.expected.clone(),
            original: original.original.clone(),
            created: None,
            execution_task: None,
            received_input: None,
            continuation_outcome: None,
            _fee: fee,
        }
    };
    let identity = counterfeit.retained_identity();
    let before = f.current_bytes();
    assert!(f
        .member()
        .with_retained_join_catalog_repair(
            &counterfeit.original.capture,
            &identity,
            |_| -> Result<(), AppError> {
                panic!("unadmitted runtime identity cannot enter repair callback")
            }
        )
        .is_err());
    assert_eq!(f.current_bytes(), before);
    assert!(f
        .member()
        .pending_join_catalog
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .pending
        .is_some());
    drop(counterfeit);
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    f.member().retry_pending_join_catalog().unwrap();
}
