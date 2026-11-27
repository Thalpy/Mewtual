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
    let result = disk.cover_selected((&final_path, None), (&stage, original), id, &sealed, |_| {
        std::fs::rename(&stage, &moved).unwrap();
        std::fs::write(&stage, &*sealed).unwrap();
        Ok(())
    });
    assert!(result.is_err());
    assert!(!final_path.exists());
    assert_eq!(std::fs::read(stage).unwrap(), *sealed);
    assert_eq!(std::fs::read(moved).unwrap(), *sealed);
}
