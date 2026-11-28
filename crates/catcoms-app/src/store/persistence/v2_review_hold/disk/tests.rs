use super::*;
use crate::store::persistence::v2_member_outbox::fs::ensure_exact_held;
use crate::store::persistence::v2_review_hold::tests::{fixture, scope, KEY};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn held_store_completes_only_the_exact_selected_prefix_and_preserves_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
    let pool = CriticalCatalogReadPool::new(2 * WORK).unwrap();
    let path = directory.path().join("held");
    std::fs::create_dir(&path).unwrap();
    let open = || {
        HoldDisk::from_held(
            HeldDirectory::open(path.clone()).unwrap(),
            scope(),
            &KEY,
            pool.clone(),
        )
        .unwrap()
    };
    let _lease = session.retain_for_physical_work();
    let disk = open();
    let fee = pool.try_reserve(WORK).unwrap();
    disk.require(&KEY, &fee).unwrap();
    assert!(disk.require(&[91; 32], &fee).is_err());
    assert!(disk.require(&KEY, &pool.try_reserve(1).unwrap()).is_err());

    let (plain, id) = fixture(scope(), 10, 11);
    let (_, sealed) = disk
        .scope
        .seal(&KEY, &plain, &mut ChaCha20Rng::seed_from_u64(12))
        .unwrap();
    let label = hex::encode(disk.scope.file_label(&KEY, id.id));
    let stage = path.join(format!("{label}{STAGE_SUFFIX}"));
    let selected = path.join(format!("{label}{HOLD_SUFFIX}"));
    ensure_exact_held(&stage, &sealed).unwrap();
    let prefix = &sealed[..sealed.len() / 2];
    std::fs::write(&selected, prefix).unwrap();
    // A process reopened the same held directory and recovered the exact
    // authenticated input. Reproof from Core belongs to the real caller.
    drop(disk);
    let disk = open();
    let first = disk.read_named(&KEY, &stage, id.id).unwrap().unwrap();
    assert_eq!(first.data.identity, id);
    let reopen_stage = || open_exact_write(&stage, WriteMode::Existing).unwrap();
    let partial = disk.read_frame(&selected).unwrap().unwrap();
    assert!(disk
        .cover_selected(
            &KEY,
            (&selected, Some(&partial)),
            (&stage, reopen_stage()),
            id,
            &first.frame.bytes,
            |_| Err(refused())
        )
        .is_err());
    assert_eq!(std::fs::read(&selected).unwrap(), prefix);
    assert_eq!(
        disk.cover_selected(
            &KEY,
            (&selected, Some(&partial)),
            (&stage, reopen_stage()),
            id,
            &first.frame.bytes,
            |case| {
                assert_eq!(case, id);
                Ok(())
            }
        )
        .unwrap(),
        id
    );
    assert_eq!(std::fs::read(&selected).unwrap(), *sealed);
    assert_eq!(
        disk.read_named(&KEY, &selected, id.id)
            .unwrap()
            .unwrap()
            .data
            .identity,
        id
    );

    let (_, second_ciphertext) = disk
        .scope
        .seal(&KEY, &plain, &mut ChaCha20Rng::seed_from_u64(13))
        .unwrap();
    assert_ne!(second_ciphertext, sealed);
    let other = path.join(format!("{}{STAGE_SUFFIX}", "0".repeat(64)));
    let other_stage = ensure_exact_held(&other, &second_ciphertext).unwrap();
    let resolved = disk.read_frame(&selected).unwrap().unwrap();
    assert!(disk
        .cover_selected(
            &KEY,
            (&selected, Some(&resolved)),
            (&other, other_stage),
            id,
            &second_ciphertext,
            |_| Ok(())
        )
        .is_err());
    assert!(disk.finish_selected(&selected, None, &sealed).is_err());
    assert_eq!(std::fs::read(&selected).unwrap(), *sealed);
    assert_eq!(std::fs::read(&stage).unwrap(), *sealed);

    let parked = selected.with_extension("parked");
    std::fs::rename(&selected, &parked).unwrap();
    std::fs::write(&selected, &*sealed).unwrap();
    assert!(disk
        .finish_selected(&selected, Some(&resolved), &sealed)
        .is_err());
    assert_eq!(std::fs::read(&selected).unwrap(), *sealed);
}

#[test]
fn held_store_treats_a_partial_stage_as_uncertain_only_for_its_case() {
    let directory = tempfile::tempdir().unwrap();
    let pool = CriticalCatalogReadPool::new(WORK).unwrap();
    let disk = HoldDisk::from_held(
        HeldDirectory::open(directory.path().to_path_buf()).unwrap(),
        scope(),
        &KEY,
        pool,
    )
    .unwrap();
    let (_, id) = fixture(scope(), 14, 15);
    let unrelated = ReviewCaseId::from_bytes([91; 32]);
    let path = |case| {
        let label = hex::encode(disk.scope.file_label(&KEY, case));
        disk.directory.child(&format!("{label}{STAGE_SUFFIX}"))
    };
    std::fs::write(path(id.id), b"incomplete").unwrap();
    assert!(disk.read_named(&KEY, &path(id.id), id.id).is_err());
    assert!(disk
        .read_named(&KEY, &path(unrelated), unrelated)
        .unwrap()
        .is_none());
    disk.require_room().unwrap();
    for index in 0..MAX_CASES - 1 {
        std::fs::write(
            disk.directory.child(&format!("{index:064x}{STAGE_SUFFIX}")),
            [],
        )
        .unwrap();
    }
    assert!(disk.require_room().is_err());
    assert_eq!(std::fs::read(path(id.id)).unwrap(), b"incomplete");
}

#[test]
fn held_store_refuses_same_byte_stage_replacement_during_history_check() {
    let root = tempfile::tempdir().unwrap();
    let pool = CriticalCatalogReadPool::new(WORK).unwrap();
    let disk = HoldDisk::from_held(
        HeldDirectory::open(root.path().to_path_buf()).unwrap(),
        scope(),
        &KEY,
        pool,
    )
    .unwrap();
    let (plain, id) = fixture(scope(), 10, 11);
    let (_, sealed) = disk
        .scope
        .seal(&KEY, &plain, &mut ChaCha20Rng::seed_from_u64(12))
        .unwrap();
    let stage = disk.directory.child("fixture.stage");
    let final_path = disk.directory.child("fixture.held");
    let original = ensure_exact_held(&stage, &sealed).unwrap();
    let moved = disk.directory.child("fixture-original");
    let result = disk.cover_selected(
        &KEY,
        (&final_path, None),
        (&stage, original),
        id,
        &sealed,
        |_| {
            std::fs::rename(&stage, &moved).unwrap();
            std::fs::write(&stage, &*sealed).unwrap();
            Ok(())
        },
    );
    assert!(result.is_err());
    assert!(!final_path.exists());
    assert_eq!(std::fs::read(stage).unwrap(), *sealed);
    assert_eq!(std::fs::read(moved).unwrap(), *sealed);
}

#[test]
fn witness_requires_a_covered_body_and_completes_only_its_exact_prefix() {
    let root = tempfile::tempdir().unwrap();
    let session = catcoms_storage::acquire_vault_session(root.path()).unwrap();
    let holds = root.path().join("held");
    let receipts = root.path().join("receipts");
    std::fs::create_dir(&holds).unwrap();
    std::fs::create_dir(&receipts).unwrap();
    let pool = CriticalCatalogReadPool::new(2 * WORK).unwrap();
    let disk = HoldDisk::from_held(
        HeldDirectory::open(holds.clone()).unwrap(),
        scope(),
        &KEY,
        pool,
    )
    .unwrap()
    .with_selection_witnesses(HeldDirectory::open(receipts.clone()).unwrap(), [46; 32]);

    let (plain, case) = fixture(scope(), 10, 11);
    let (_, frame) = disk
        .scope
        .seal(&KEY, &plain, &mut ChaCha20Rng::seed_from_u64(12))
        .unwrap();
    let label = hex::encode(disk.scope.file_label(&KEY, case.id));
    let stage = holds.join(format!("{label}{STAGE_SUFFIX}"));
    let held = holds.join(format!("{label}{HOLD_SUFFIX}"));
    let receipt = receipts.join(format!("{label}.receipt"));
    ensure_exact_held(&stage, &frame).unwrap();

    // A fully written body can survive a refused result. The witness must
    // still be absent if the writer did not reach its final current check.
    let mut checks = 0;
    let cut = disk.cover_selected(
        &KEY,
        (&held, None),
        (
            &stage,
            open_exact_write(&stage, WriteMode::Existing).unwrap(),
        ),
        case,
        &frame,
        |_| {
            checks += 1;
            if checks == 2 {
                Err(refused())
            } else {
                Ok(())
            }
        },
    );
    assert!(cut.is_err());
    assert_eq!(std::fs::read(&held).unwrap(), *frame);
    assert_eq!(observe(&receipt).unwrap(), Presence::Absent);
    let read = |after| {
        disk.selected_review_window(
            &KEY,
            HeldReviewQuery { after, limit: 1 },
            &session,
            || Ok(()),
            |record| {
                catcoms_replication::chat::SignedAction::decode(record.original_bytes_data())
                    .map_err(|_| refused())
            },
        )
        .unwrap()
    };
    let unwitnessed = read(None);
    assert!(unwitnessed.records.is_empty());
    assert_eq!(unwitnessed.unavailable, 1);
    assert_eq!(observe(&receipt).unwrap(), Presence::Absent);
    drop(unwitnessed);

    let retry = || {
        let original = disk.read_frame(&held).unwrap().unwrap();
        disk.cover_selected(
            &KEY,
            (&held, Some(&original)),
            (
                &stage,
                open_exact_write(&stage, WriteMode::Existing).unwrap(),
            ),
            case,
            &frame,
            |_| Ok(()),
        )
    };
    assert_eq!(retry().unwrap(), case);
    let witness = std::fs::read(&receipt).unwrap();
    assert_eq!(witness.len(), 32);
    assert_ne!(label, "0".repeat(64));
    std::fs::write(
        holds.join(format!("{}{HOLD_SUFFIX}", "0".repeat(64))),
        b"incomplete",
    )
    .unwrap();
    let first = read(None);
    assert!(first.records.is_empty());
    assert_eq!(first.unavailable, 1);
    assert_eq!(first.next, Some([0; 32]));
    drop(first);
    let next = read(Some([0; 32]));
    assert_eq!(next.records.len(), 1);
    assert_eq!(next.records[0].case, case.id);
    assert_eq!(next.records[0].signed.id(), case.action);
    assert_eq!(next.unavailable, 0);
    assert!(next.next.is_none());
    drop(next);
    std::fs::write(&receipt, &witness[..12]).unwrap();
    let short = read(Some([0; 32]));
    assert!(short.records.is_empty());
    assert_eq!(short.unavailable, 1);
    assert_eq!(std::fs::read(&receipt).unwrap(), &witness[..12]);
    drop(short);
    assert_eq!(retry().unwrap(), case);
    assert_eq!(std::fs::read(&receipt).unwrap(), witness);
    let mut conflict = witness.clone();
    conflict[0] ^= 1;
    std::fs::write(&receipt, &conflict).unwrap();
    assert_eq!(retry().unwrap(), case);
    assert_eq!(std::fs::read(&receipt).unwrap(), conflict);
    assert_eq!(std::fs::read(&held).unwrap(), *frame);
    let wrong_witness = read(Some([0; 32]));
    assert!(wrong_witness.records.is_empty());
    assert_eq!(wrong_witness.unavailable, 1);
    drop(wrong_witness);
    std::fs::write(&receipt, &witness).unwrap();

    // A local replacement while proving a later row cannot leave an
    // earlier detached record in the same published window.
    let (plain, second_case) = fixture(scope(), 16, 17);
    let (_, other_frame) = disk
        .scope
        .seal(&KEY, &plain, &mut ChaCha20Rng::seed_from_u64(15))
        .unwrap();
    let second_label = hex::encode(disk.scope.file_label(&KEY, second_case.id));
    let second_stage = holds.join(format!("{second_label}{STAGE_SUFFIX}"));
    let second_held = holds.join(format!("{second_label}{HOLD_SUFFIX}"));
    let staged = ensure_exact_held(&second_stage, &other_frame).unwrap();
    disk.cover_selected(
        &KEY,
        (&second_held, None),
        (&second_stage, staged),
        second_case,
        &other_frame,
        |_| Ok(()),
    )
    .unwrap();
    let (replaced, remaining) = if second_label < label {
        (second_held, case)
    } else {
        (held, second_case)
    };
    let mut calls = 0;
    let page = disk
        .selected_review_window(
            &KEY,
            HeldReviewQuery {
                after: Some([0; 32]),
                limit: 2,
            },
            &session,
            || Ok(()),
            |record| {
                calls += 1;
                if calls == 2 {
                    let bytes = std::fs::read(&replaced).unwrap();
                    std::fs::rename(&replaced, replaced.with_extension("parked")).unwrap();
                    std::fs::write(&replaced, bytes).unwrap();
                }
                catcoms_replication::chat::SignedAction::decode(record.original_bytes_data())
                    .map_err(|_| refused())
            },
        )
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].case, remaining.id);
    assert_eq!(page.unavailable, 1);
}
