//! Actual Store/canonical/factory boundaries. No save-success booleans or fake Current source.
use super::*;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"canonical owner actual store fixture";
type Attempt = OwnerFounderInitialization<catcoms_rt::MemNetwork, ChaCha20Rng>;

#[test]
fn canonical_initialization_prepays_independent_backing_and_payload_capacity() {
    let output = 1024 * 1024usize;
    let mls = 512 * 1024usize;
    let backing = 512 * 1024usize;
    let expected = 4 * (output + WRAPPER_FEE + SEALED_FEE + 1) + 2 * (backing + 2 * mls);
    let budget =
        OwnerEvidenceBudget::checkpoint(1, 4 * 1024 * 1024, expected, output, mls, backing)
            .unwrap();
    assert_eq!(budget.buffer_fee().unwrap(), expected);
    assert!(OwnerEvidenceBudget::checkpoint(
        1,
        4 * 1024 * 1024,
        expected - 1,
        output,
        mls,
        backing
    )
    .is_err());
    assert!(OwnerEvidenceBudget::checkpoint(1, 4 * 1024 * 1024, expected, output, mls, 0).is_err());
    assert!(OwnerEvidenceBudget::checkpoint(
        1,
        4 * 1024 * 1024,
        expected,
        output,
        mls,
        MAX_MLS_BACKING_BYTES + 1
    )
    .is_err());
}

fn store() -> (tempfile::TempDir, ServerStore, ChaCha20Rng) {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc001);
    let store = ServerStore::open(directory.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut rng)
        .unwrap();
    store
        .configure_owner_evidence_budget(
            OwnerEvidenceBudget::checkpoint(
                4,
                16 * 1024 * 1024,
                8 * 1024 * 1024,
                1024 * 1024,
                512 * 1024,
                512 * 1024,
            )
            .unwrap(),
        )
        .unwrap();
    (directory, store, rng)
}
fn begin(store: &ServerStore, peer: u64) -> Attempt {
    store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(peer)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(peer),
            Box::new(ManualClock::new(1_000)),
            "actual canonical owner fixture",
            9,
        )
        .unwrap()
}

#[test]
fn same_initializer_repairs_current_scope_barrier_before_rechecking_its_immutable_pin() {
    use std::sync::atomic::Ordering;
    let (directory, store, mut rng) = store();
    let mut attempt = begin(&store, 0xc071);
    let scope = attempt.private.reserve_scope(&store, &mut rng).unwrap();
    // Initializing is already saved. This real fault is consumed only by the later Current
    // admission flush, after actual pin and canonical head saves, not a mocked covering result.
    store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
    assert!(attempt.retry(&store, &mut rng).is_err());
    assert!(attempt.saved.is_some());
    assert_eq!(
        attempt.private.preparation().unwrap().nonce(),
        scope.nonce()
    );
    let canonical_path = canonical_path(directory.path(), &attempt);
    let canonical = fs::read(&canonical_path).unwrap();
    let pins = fs::read(directory.path().join("chat-owner-pins.bin")).unwrap();
    let owned = attempt.retry(&store, &mut rng).unwrap();
    assert!(owned.is_live());
    assert!(owned.server.sync.durable_send_context().is_ok());
    assert_eq!(
        fs::read(directory.path().join("chat-owner-pins.bin")).unwrap(),
        pins
    );
    assert_eq!(
        fs::read(canonical_path).unwrap(),
        canonical,
        "exact saved head is not re-encoded during scope retry"
    );
}

#[test]
fn revoked_original_initializer_cannot_flush_dirty_scope_as_a_pin_retry() {
    use rand_core::RngCore;
    use std::sync::atomic::Ordering;
    let (directory, store, mut rng) = store();
    let mut attempt = begin(&store, 0xc072);
    let scope = attempt.private.reserve_scope(&store, &mut rng).unwrap();
    store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
    assert!(attempt.retry(&store, &mut rng).is_err());
    assert!(attempt.saved.is_some());
    let admissions = fs::read(directory.path().join("chat-admissions.bin")).unwrap();
    let pins = fs::read(directory.path().join("chat-owner-pins.bin")).unwrap();
    scope.revoker().revoke();
    let mut unchanged = rng.clone();
    assert!(store
        .lock_old_owner_pin_admission(&scope, &mut rng)
        .is_err());
    assert_eq!(
        rng.next_u64(),
        unchanged.next_u64(),
        "revoked original must refuse before repair encryption"
    );
    assert_eq!(
        fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
        admissions
    );
    assert_eq!(
        fs::read(directory.path().join("chat-owner-pins.bin")).unwrap(),
        pins
    );
}
fn canonical_path(directory: &std::path::Path, attempt: &Attempt) -> std::path::PathBuf {
    let prepared = attempt.prepared.as_ref().unwrap();
    directory.join("owner-state").join(format!(
        "{}.bin",
        hex::encode(key(prepared.group(), prepared.founder()))
    ))
}
fn contained_snapshot<'a>(plain: &'a [u8]) -> &'a [u8] {
    let mut decoder = Decoder::new(plain);
    assert_eq!(
        decoder.get_str().unwrap(),
        "catcoms/owner-canonical-snapshot/v1"
    );
    for _ in 0..5 {
        decoder.get_bytes().unwrap();
    } // group/signer/profile/domain/nonce
    decoder.get_u64().unwrap();
    decoder.get_u64().unwrap();
    decoder.get_bytes().unwrap(); // parent
    decoder.get_bytes().unwrap(); // original initialization debt label; never fresh provenance
    let snapshot = decoder.get_bytes().unwrap();
    decoder.finish().unwrap();
    snapshot
}

#[test]
fn actual_founder_canonical_fault_retry_retains_same_frozen_bytes_and_paid_quota() {
    for after_rename in [false, true] {
        let (directory, store, mut rng) = store();
        let mut attempt = begin(&store, 0xc002);
        let path = canonical_path(directory.path(), &attempt);
        store
            .owner_evidence
            .canonical_fault
            .store(if after_rename { 2 } else { 1 }, Ordering::SeqCst);
        let failure = attempt.retry(&store, &mut rng).unwrap_err();
        assert_eq!(
            matches!(failure, AppError::CommittedButNotDurable(_)),
            after_rename
        );
        assert_eq!(
            path.exists(),
            after_rename,
            "fault must exercise actual rename outcome"
        );
        let frozen = attempt.private.frozen_snapshot().unwrap();
        let pointer = frozen.bytes().as_ptr();
        let digest = frozen.digest();
        let bytes = frozen.bytes().to_vec();
        let scope = attempt.private.preparation().unwrap().clone();
        let owner = scope.owner_identity();
        let hot = store.owner_evidence.canonical.lock().unwrap().hot_bytes;
        assert!(
            hot >= bytes.len() * 4,
            "full reserved fee stays with retained frozen candidate"
        );
        assert!(attempt
            .private
            .server_mut_for_owner_initialization()
            .is_err());
        assert!(attempt
            .private
            .server()
            .unwrap()
            .sync
            .durable_send_context()
            .is_err());
        // A second real failure must neither re-encode nor resample source/nonce/facts/head.
        store
            .owner_evidence
            .canonical_fault
            .store(1, Ordering::SeqCst);
        assert!(attempt.retry(&store, &mut rng).is_err());
        assert_eq!(
            attempt.private.frozen_snapshot().unwrap().bytes().as_ptr(),
            pointer
        );
        assert_eq!(attempt.private.frozen_snapshot().unwrap().digest(), digest);
        assert_eq!(attempt.private.frozen_snapshot().unwrap().bytes(), bytes);
        assert_eq!(
            attempt.private.preparation().unwrap().nonce(),
            scope.nonce()
        );
        assert!(Arc::ptr_eq(
            &attempt.private.preparation().unwrap().owner_identity(),
            &owner
        ));
        assert_eq!(
            store.owner_evidence.canonical.lock().unwrap().hot_bytes,
            hot
        );
        let owned = attempt.retry(&store, &mut rng).unwrap();
        assert!(owned.is_live());
        assert!(owned.server.sync.durable_send_context().is_ok());
        assert!(attempt.private.frozen_snapshot().is_err());
        assert_eq!(store.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
        let raw = fs::read(&path).unwrap();
        let sealed = crate::store::unframe(&raw).unwrap();
        let plain = Zeroizing::new(unseal(&store.keys.db_key().unwrap(), &sealed).unwrap());
        assert_eq!(contained_snapshot(&plain), bytes);
        assert!(
            attempt.retry(&store, &mut rng).is_err(),
            "moved owner is one-shot"
        );
    }
}

#[test]
fn actual_saved_owner_tail_restore_cannot_borrow_plain_current_scope_or_numeric_alias() {
    let (directory, store, mut rng) = store();
    let mut attempt = begin(&store, 0xc003);
    // Fixture copies are never registered/running. They model captured numeric mirror bytes;
    // the real private factory exposes no public snapshot/Server escape before the save.
    let old_no_tail = attempt
        .private
        .server_mut_for_owner_initialization()
        .unwrap()
        .snapshot()
        .unwrap();
    store.save_server(1, &old_no_tail, &mut rng).unwrap();
    store.save_server(2, &old_no_tail, &mut rng).unwrap();
    let mirror_one = fs::read(directory.path().join("servers/1.bin")).unwrap();
    let path = canonical_path(directory.path(), &attempt);
    let owned = attempt.retry(&store, &mut rng).unwrap();
    let raw = fs::read(&path).unwrap();
    let plain = Zeroizing::new(
        unseal(
            &store.keys.db_key().unwrap(),
            &crate::store::unframe(&raw).unwrap(),
        )
        .unwrap(),
    );
    let mut restored = Server::restore(
        contained_snapshot(&plain),
        Hub::new().join(PeerId::from_u64(0xc004)),
        ChaCha20Rng::seed_from_u64(0xc004),
        Box::new(ManualClock::new(1_000)),
        "raw owner tail",
    )
    .unwrap();
    let label = restored.sync.chat_scope_label().unwrap();
    let identity = crate::store::ChatAdmissionKey::new(&owned.group(), owned.founder).unwrap();
    let crate::store::ChatAdmissionResolution::Current(binding) = store
        .resolve_chat_admission(&identity, label.nonce())
        .unwrap()
    else {
        panic!("actual initialization crossed Current inventory barrier");
    };
    restored.sync.bind_managed_chat_scope(&binding).unwrap();
    assert!(
        restored.sync.durable_send_context().is_err(),
        "plain Current cannot mint canonical owner authority"
    );
    assert!(store.load_server(1).is_err());
    assert!(store.load_server(2).is_err());
    assert!(store.save_server(1, &old_no_tail, &mut rng).is_err());
    assert_eq!(
        fs::read(directory.path().join("servers/1.bin")).unwrap(),
        mirror_one
    );
    let saved = fs::read(&path).unwrap();
    store.remove_server(1).unwrap();
    store.remove_server(2).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        saved,
        "Leave of a numeric mirror never deletes canonical history/pin"
    );
    assert_eq!(
        store
            .capture_mounted_owner_profile(&owned.group())
            .unwrap()
            .profile(),
        owned.profile()
    );
}

#[test]
fn original_mount_drop_fences_initializer_without_retaining_vault_session() {
    let (directory, store, mut rng) = store();
    let mut attempt = begin(&store, 0xc005);
    store
        .owner_evidence
        .canonical_fault
        .store(2, Ordering::SeqCst);
    assert!(matches!(
        attempt.retry(&store, &mut rng),
        Err(AppError::CommittedButNotDurable(_))
    ));
    let bytes = attempt.private.frozen_snapshot().unwrap().bytes().to_vec();
    drop(store);
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    assert!(
        attempt.retry(&reopened, &mut rng).is_err(),
        "old original capture cannot adopt fresh mount"
    );
    assert_eq!(attempt.private.frozen_snapshot().unwrap().bytes(), bytes);
    // The OLD initializer cannot adopt the reopened mount. The DISTINCT canonical restore
    // path must authenticate its original source and cross file/child-parent/root barriers;
    // visible configuration cannot mint fresh founder provenance or a completed save result.
    assert!(reopened
        .capture_mounted_owner_profile(attempt.prepared.as_ref().unwrap().group())
        .is_ok());
}

#[test]
fn visible_owner_directory_retries_original_root_barrier_before_any_child_work() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc006);
    let store = ServerStore::open(directory.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut rng)
        .unwrap();
    let path = directory.path().join("owner-state");
    assert!(!path.exists());
    store
        .owner_evidence
        .directory_sync_failure
        .store(true, Ordering::SeqCst);
    assert!(store.owner_canonical_directory().is_err());
    assert!(
        path.is_dir(),
        "real mkdir preceded failed actual parent-sync callback"
    );
    assert!(
        store.owner_canonical_directory().is_err(),
        "visible existing directory must retry barrier"
    );
    assert!(fs::read_dir(&path).unwrap().next().is_none());
    store
        .owner_evidence
        .directory_sync_failure
        .store(false, Ordering::SeqCst);
    assert_eq!(store.owner_canonical_directory().unwrap(), path);
}

#[test]
fn pending_creation_is_prepaid_before_founder_profile_and_transfers_fee_once() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc007);
    let store = ServerStore::open(directory.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut rng)
        .unwrap();
    let budget = OwnerEvidenceBudget::checkpoint(
        1,
        4 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap();
    store.configure_owner_evidence_budget(budget).unwrap();
    let first = begin(&store, 0xc007);
    let fee = store.owner_evidence.canonical.lock().unwrap().hot_bytes;
    assert!(fee > 0);
    assert_eq!(
        store
            .owner_evidence
            .canonical
            .lock()
            .unwrap()
            .pending_creations,
        1
    );
    let refuse = store.prepare_owner_evidence_founder(
        Hub::new().join(PeerId::from_u64(0xc008)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0xc008),
        Box::new(ManualClock::new(1_000)),
        "over paid slot",
        9,
    );
    assert!(refuse.is_err());
    assert!(store
        .owner_evidence
        .pins
        .lock()
        .unwrap()
        .table
        .as_ref()
        .unwrap()
        .is_empty());
    drop(first);
    assert_eq!(store.owner_evidence.canonical.lock().unwrap().hot_bytes, 0);
    let mut next = begin(&store, 0xc009);
    store
        .owner_evidence
        .canonical_fault
        .store(1, Ordering::SeqCst);
    assert!(next.retry(&store, &mut rng).is_err());
    assert_eq!(
        store.owner_evidence.canonical.lock().unwrap().hot_bytes,
        fee
    );
    assert_eq!(
        store
            .owner_evidence
            .canonical
            .lock()
            .unwrap()
            .pending_creations,
        0
    );
    assert!(next.reservation.is_some());
}

#[test]
fn concurrent_groups_merge_whole_uncertain_pin_table_before_original_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc010);
    let store = ServerStore::open(directory.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut rng)
        .unwrap();
    store
        .configure_owner_evidence_budget(
            OwnerEvidenceBudget::checkpoint(
                4,
                16 * 1024 * 1024,
                16 * 1024 * 1024,
                1024 * 1024,
                512 * 1024,
                512 * 1024,
            )
            .unwrap(),
        )
        .unwrap();
    // Construct both fallible private creations BEFORE spawning/barrier; an earlier regression
    // must fail the test, not strand the other worker forever waiting for a panicked peer.
    let first_attempt = begin(&store, 0xc011);
    let second_attempt = begin(&store, 0xc012);
    let (first_start, first_gate) = std::sync::mpsc::channel();
    let (second_start, second_gate) = std::sync::mpsc::channel();
    let shared = &store;
    let run = |mut attempt: Attempt, peer| {
        let mut rng = ChaCha20Rng::seed_from_u64(peer);
        let outcome = attempt.retry(shared, &mut rng);
        (attempt, outcome)
    };
    store.owner_evidence.pin_fault.store(2, Ordering::SeqCst);
    let ((mut first, first_result), (mut second, second_result)) = std::thread::scope(|scope| {
        // These bounded coordination waits affect test scheduling only, never protocol time.
        // If creating either OS thread panics, the other exits instead of stranding scope's
        // mandatory join forever at a Barrier. Both real private attempts already exist.
        let first = scope.spawn(move || {
            first_gate
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("first writer start");
            run(first_attempt, 0xc011)
        });
        let second = scope.spawn(move || {
            second_gate
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("second writer start");
            run(second_attempt, 0xc012)
        });
        first_start.send(()).unwrap();
        second_start.send(()).unwrap();
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(
        first_result.is_ok(),
        second_result.is_ok(),
        "one real root parent fault"
    );
    let first_group = first
        .prepared
        .as_ref()
        .map(|p| p.group().to_vec())
        .unwrap_or_else(|| first_result.as_ref().unwrap().group());
    let second_group = second
        .prepared
        .as_ref()
        .map(|p| p.group().to_vec())
        .unwrap_or_else(|| second_result.as_ref().unwrap().group());
    assert_ne!(first_group, second_group);
    {
        let roots = store.owner_evidence.pins.lock().unwrap();
        assert!(
            !roots.dirty,
            "the second writer synced the WHOLE possibly installed table"
        );
        assert_eq!(roots.table.as_ref().unwrap().len(), 2);
        assert!(roots.table.as_ref().unwrap().contains_key(&first_group));
        assert!(roots.table.as_ref().unwrap().contains_key(&second_group));
    }
    let mut completed = Vec::new();
    for (attempt, result) in [(&mut first, first_result), (&mut second, second_result)] {
        match result {
            Ok(owned) => completed.push(owned),
            Err(AppError::CommittedButNotDurable(_)) => {
                let original = attempt.private.preparation().unwrap().nonce();
                let owned = attempt.retry(&store, &mut rng).unwrap();
                assert_eq!(owned.binding.scope.nonce(), original);
                completed.push(owned);
            }
            Err(error) => panic!("unexpected real root fault outcome: {error}"),
        }
    }
    let records: Vec<_> = completed
        .iter()
        .map(|owner| (owner.group(), owner.profile()))
        .collect();
    drop(completed);
    drop(first);
    drop(second);
    drop(store);
    let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    for (group, profile) in records {
        assert_eq!(
            reopened
                .capture_mounted_owner_profile(&group)
                .unwrap()
                .profile(),
            profile
        );
    }
}
