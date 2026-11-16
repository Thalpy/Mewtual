//! Actual saved Pending source into negative task/ONE T only. NO network API is available.
//! Authored UNRUN under the compiler/test/build pause.
use super::*;
use catcoms_rt::MeshTransport;

fn save(fixture: &mut Fixture, prepared: &mut JournalledCheckpointJoinPreparation) {
    prepared
        .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0xc101))
        .unwrap();
}

#[test]
fn actual_saved_source_registers_once_and_duplicate_returns_same_transport_and_source() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(41);
    let mut original = fixture.begin(invite.clone());
    let package = original.metadata.package;
    save(&mut fixture, &mut original);
    let first = original
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc102)),
            PeerId::from_u64(0x8101),
        )
        .unwrap();
    assert!(first.can_submit_original());
    let duplicate = fixture.begin(invite);
    assert_eq!(duplicate.metadata.package, package);
    let failed = duplicate
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc103)),
            PeerId::from_u64(0x8101),
        )
        .unwrap_err();
    let (source, transport, _) = failed.into_parts();
    assert_eq!(transport.local_peer(), PeerId::from_u64(0xc103));
    assert_eq!(source.metadata.package, package);
    assert!(!first.cancellation().is_cancelled());
    drop(first);
    assert!(
        source
            .activate_original_transport(fixture.member(), transport, PeerId::from_u64(0x8101))
            .is_err(),
        "dropping the owner cannot renew the same scheduling revision"
    );
}

#[test]
fn unsaved_and_unsupported_activation_return_originals_without_new_registry_or_package() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(42);
    let original = fixture.begin(invite);
    let package = original.metadata.package;
    let source = original.metadata.source;
    let failed = original
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc104)),
            PeerId::from_u64(0x8101),
        )
        .unwrap_err();
    let (mut original, transport, _) = failed.into_parts();
    assert_eq!(transport.local_peer(), PeerId::from_u64(0xc104));
    assert_eq!(original.metadata.package, package);
    assert_eq!(original.metadata.source, source);
    save(&mut fixture, &mut original);
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (original, _, _) = original
        .activate_original_transport(
            fixture.member(),
            NoRequestTransport(requests.clone()),
            PeerId::from_u64(0x8101),
        )
        .unwrap_err()
        .into_parts();
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(original.metadata.package, package);
    let active = original
        .activate_original_transport(fixture.member(), transport, PeerId::from_u64(0x8101))
        .unwrap();
    assert!(active.can_submit_original());
}

#[test]
fn pause_closes_before_uncertain_write_and_only_saved_new_schedule_registers() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(43);
    let mut original = fixture.begin(invite.clone());
    save(&mut fixture, &mut original);
    let old = original
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc105)),
            PeerId::from_u64(0x8101),
        )
        .unwrap();
    let old_negative = old.cancellation();
    let mut source = fixture.begin(invite);
    let package = source.metadata.package;
    fixture
        .member()
        .critical_namespace_fault
        .store(3, Ordering::SeqCst);
    assert!(source
        .pause_pending(fixture.member(), &mut fixture.member_rng.clone())
        .is_err());
    assert!(
        old_negative.is_cancelled(),
        "negative closure precedes the refused file write"
    );
    assert!(!old.can_submit_original());
    fixture
        .member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    source
        .pause_pending(fixture.member(), &mut fixture.member_rng.clone())
        .unwrap();
    source
        .resume_pending(fixture.member(), &mut fixture.member_rng.clone())
        .unwrap();
    assert_eq!(source.metadata.package, package);
    let new = source
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc106)),
            PeerId::from_u64(0x8101),
        )
        .unwrap();
    assert!(new.can_submit_original());
    old.close();
    assert!(
        !new.cancellation().is_cancelled(),
        "stale close has no lookup by logical key"
    );
}

#[test]
fn protected_absent_basis_refuses_generic_issuance_but_unrelated_saved_control_progresses() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(44);
    let mut source = fixture.begin(invite);
    let key = crate::store::ChatAdmissionKey::new(
        source.metadata.group.as_slice(),
        source.metadata.signer,
    )
    .unwrap();
    save(&mut fixture, &mut source);
    let active = source
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc107)),
            PeerId::from_u64(0x8101),
        )
        .unwrap();
    let capture = fixture.member().capture_chat_admission(key).unwrap();
    assert!(fixture
        .member()
        .prepare_legacy_chat_admission(
            &capture,
            b"must not replace retained source",
            &mut fixture.member_rng.clone()
        )
        .is_err());
    let other = founder(
        fixture.member(),
        &fixture.hub,
        0xc108,
        &mut ChaCha20Rng::seed_from_u64(0xc108),
    );
    assert!(other
        .mint_owner_checkpoint_invite(fixture.member(), [45; 16], 100_000, vec![])
        .is_ok());
    assert!(!active.cancellation().is_cancelled());
    active.close();
    assert!(fixture
        .member()
        .prepare_legacy_chat_admission(
            &capture,
            b"weak expiry is not source absence",
            &mut fixture.member_rng.clone()
        )
        .is_err());
}

#[test]
fn active_task_never_retains_old_store_session_and_reopen_cancels_only_old_owner() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(46);
    let mut source = fixture.begin(invite.clone());
    save(&mut fixture, &mut source);
    let old = source
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc109)),
            PeerId::from_u64(0x8101),
        )
        .unwrap();
    let old_negative = old.cancellation();
    drop(fixture.member.take());
    assert!(old_negative.is_cancelled());
    let member = configured(
        fixture.member_dir.path(),
        &mut fixture.member_rng,
        &fixture.pool,
    );
    fixture.member = Some(Arc::new(member));
    let recovered = fixture.begin(invite);
    let new = recovered
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc110)),
            PeerId::from_u64(0x8101),
        )
        .unwrap();
    assert!(new.can_submit_original());
    old.close();
    assert!(new.can_submit_original());
}

#[test]
fn task_drop_releases_task_source_fees_but_retained_registry_metadata_stays_charged() {
    let mut fixture = Fixture::new();
    let baseline = fixture.pool.charged_for_test();
    let invite = fixture.invite(47);
    let mut source = fixture.begin(invite);
    save(&mut fixture, &mut source);
    let active = source
        .activate_original_transport(
            fixture.member(),
            fixture.hub.join(PeerId::from_u64(0xc111)),
            PeerId::from_u64(0x8101),
        )
        .unwrap();
    let during = fixture.pool.charged_for_test();
    drop(active);
    let retained = fixture.pool.charged_for_test();
    assert!(
        retained > baseline,
        "weak key/identity exclusion retains its ORIGINAL real metadata fee"
    );
    assert!(
        retained < during,
        "source/task buffers do not remain in the registry"
    );
    drop(fixture.member.take());
    assert_eq!(fixture.pool.charged_for_test(), 0);
}

#[test]
fn final_negative_after_admission_before_catalog_wait_refuses_registration_with_originals() {
    use std::sync::mpsc;
    use std::time::Duration;
    struct ReleaseOnDrop(Option<mpsc::Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let mut fixture = Fixture::new();
    let invite = fixture.invite(48);
    let mut source = fixture.begin(invite);
    save(&mut fixture, &mut source);
    let member = fixture.member.as_ref().unwrap().clone();
    let negative = member.chat_mount_revoker();
    let weak = Arc::downgrade(&member);
    let (reached_tx, reached_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (contended_tx, contended_rx) = mpsc::channel();
    let release = ReleaseOnDrop(Some(release_tx));
    *member.pending_join_before_catalog_writer.lock().unwrap() = Some(Box::new(move || {
        reached_tx.send(()).unwrap();
        release_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("bounded harness release");
        let store = weak.upgrade().unwrap();
        contended_tx
            .send(matches!(
                store.pending_join_catalog.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ))
            .unwrap();
    }));
    let transport = fixture.hub.join(PeerId::from_u64(0xc112));
    let (done_tx, done_rx) = mpsc::channel();
    let worker_store = member.clone();
    let worker = std::thread::spawn(move || {
        let result =
            source.activate_original_transport(&worker_store, transport, PeerId::from_u64(0x8101));
        let rejected_with_same_t = match result {
            Err(returned) => {
                let (source, transport, _) = returned.into_parts();
                let same = transport.local_peer() == PeerId::from_u64(0xc112);
                drop(source);
                drop(transport);
                same
            }
            Ok(_) => false,
        };
        drop(worker_store);
        let _ = done_tx.send(rejected_with_same_t);
    });
    reached_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("actual admission-held seam");
    let catalog = member.pending_join_catalog.lock().unwrap();
    negative.revoke();
    release.0.as_ref().unwrap().send(()).unwrap();
    assert!(contended_rx.recv_timeout(Duration::from_secs(10)).unwrap());
    drop(catalog);
    assert!(done_rx.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap(); // Bounded completion FIRST; detached on unwind rather than join trap.
}
