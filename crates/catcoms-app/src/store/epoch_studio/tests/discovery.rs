use super::*;

#[test]
fn studio_installed_availability_refuses_uninstalled_faulted_and_mismatched_choices() {
    for art in [false, true] {
        for case in [
            "missing",
            "epoch_zero",
            "closing",
            "fault",
            "journal",
            "tenure",
            "installed",
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let f = Fixture::new(art);
            let mut receipt = None;
            if case != "missing" {
                if case == "epoch_zero" {
                    let mut b = budget(&mut store, &f);
                    let (_, state) = f.edit(&mut store, &mut b, f.insert());
                    store.retain_studio_source(&f.group, &f.device, state);
                } else {
                    receipt = Some(prepared(&f, &mut store));
                    if matches!(case, "closing" | "fault") {
                        let state = f.load(&store).unwrap();
                        let next = if case == "fault" {
                            let held = receipt.as_ref().unwrap();
                            Receipt::sign(
                                f.logical.clone(),
                                held.closed_epoch,
                                [8; 32],
                                [9; 32],
                                0,
                                InheritedCheckpoint::EpochZero,
                                &f.device,
                            )
                            .unwrap()
                        } else {
                            f.receipt(&state, 8)
                        };
                        let mut b = budget(&mut store, &f);
                        let (_, state) = store
                            .seal_studio_epoch(
                                SERVER,
                                &f.group,
                                f.target,
                                &f.device,
                                next,
                                0,
                                &mut rng(),
                                &mut b,
                            )
                            .unwrap();
                        store.retain_studio_source(&f.group, &f.device, state);
                    } else if case == "journal" {
                        // A newer irrevocable choice cannot be completed using the old installed seed.
                        let mut b = budget(&mut store, &f);
                        store
                            .complete_studio_head(
                                SERVER,
                                receipt.as_ref().unwrap(),
                                &mut rng(),
                                &mut b,
                            )
                            .unwrap();
                        let next = f.receipt(&f.load(&store).unwrap(), 8);
                        let mut b = budget(&mut store, &f);
                        store
                            .prepare_epoch_owner_receipt(
                                SERVER,
                                next,
                                &f.group,
                                0,
                                &mut rng(),
                                &mut b.storage,
                            )
                            .unwrap();
                    }
                }
            }
            let before = store.load_epoch_owner_receipts(SERVER, &f.logical).unwrap();
            let mut b = budget(&mut store, &f);
            let result = store.complete_studio_installed_head(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                u64::from(case == "tenure"),
                &mut rng(),
                &mut b,
            );
            assert_eq!(result.is_ok(), case == "installed", "{case}: {result:?}");
            let after = store.load_epoch_owner_receipts(SERVER, &f.logical).unwrap();
            if case == "installed" {
                assert!(after.pending().is_none());
                assert_eq!(after.published(), receipt.as_ref());
            } else {
                assert_eq!(after.pending(), before.pending());
                assert_eq!(after.published(), before.published());
            }
        }
    }
}

pub(super) fn prepared(f: &Fixture, store: &mut ServerStore) -> Receipt {
    let mut b = budget(store, f);
    let (_, source) = f.edit(store, &mut b, f.insert());
    let receipt = f.receipt(&source, 7);
    let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
    store
        .prepare_epoch_owner_receipt(
            SERVER,
            receipt.clone(),
            &f.group,
            0,
            &mut rng(),
            &mut b.storage,
        )
        .unwrap();
    let mut b = budget(store, f);
    let (_, saved) = store
        .adopt_studio_checkpoint(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &receipt,
            Some(seed.bytes()),
            0,
            &catcoms_rt::ManualClock::new(1000),
            &mut rng(),
            &mut b,
        )
        .unwrap();
    store.retain_studio_source(&f.group, &f.device, saved);
    receipt
}
fn head(
    f: &Fixture,
    store: &mut ServerStore,
    b: &mut EpochStudioBudget,
) -> Result<catcoms_sync::receipt_head::ReceiptHeadSelection, AppError> {
    store.prepare_studio_head(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        Some(0),
        &mut rng(),
        b,
    )
}

#[test]
fn studio_discovery_store_absence_is_inventory_checked_and_cold_service_never_rebuilds() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let f = Fixture::new(art);
        let mut b = budget(&mut store, &f);
        assert!(head(&f, &mut store, &mut b).unwrap().receipt.is_none());
        assert!(!f.path(&store).exists());
        let receipt = prepared(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let answer = head(&f, &mut store, &mut b).unwrap();
        assert!(answer.prove);
        assert_eq!(answer.receipt, Some(receipt));
        // Dropping only work reuse is not corruption. The authenticated file still needs the
        // separate preparation lane; this service path cannot pay its restore cost per request.
        store.studio_source = None;
        let restores = super::super::source::studio_full_restores_for_test();
        let mut b = budget(&mut store, &f);
        assert!(head(&f, &mut store, &mut b).is_err());
        assert_eq!(
            super::super::source::studio_full_restores_for_test(),
            restores
        );
    }
}

#[test]
fn studio_discovery_store_changed_missing_or_corrupt_source_never_becomes_hint_or_seed() {
    for art in [false, true] {
        for damage in ["missing", "corrupt", "stale_inventory"] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let f = Fixture::new(art);
            prepared(&f, &mut store);
            let mut b = budget(&mut store, &f);
            match damage {
                "missing" => std::fs::remove_file(f.path(&store)).unwrap(),
                "corrupt" => atomic_write(&f.path(&store), b"not a sealed source").unwrap(),
                _ => {
                    let _newer = budget(&mut store, &f);
                }
            }
            assert!(head(&f, &mut store, &mut b).is_err());
            assert!(b.requires_reconciliation());
        }
    }
}

#[test]
fn studio_discovery_store_source_flush_and_journal_write_fail_before_proof_and_retry_exactly() {
    for art in [false, true] {
        for failure in ["source", "journal_before", "journal_after"] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let f = Fixture::new(art);
            let receipt = prepared(&f, &mut store);
            let mut b = budget(&mut store, &f);
            let result = store.prepare_studio_head_with_io(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                Some(0),
                &mut rng(),
                &mut b,
                |path, bytes| {
                    if failure == "source" {
                        Err(invalid("injected source flush"))
                    } else {
                        sync_studio(path, bytes)
                    }
                },
                |path, bytes| {
                    assert_ne!(
                        failure, "source",
                        "journal must not run after failed source barrier"
                    );
                    if failure == "journal_after" {
                        atomic_write(path, bytes)?;
                    }
                    Err(invalid("injected journal write"))
                },
            );
            assert!(result.is_err());
            assert!(b.requires_reconciliation());
            let journal = store.load_epoch_owner_receipts(SERVER, &f.logical).unwrap();
            assert_eq!(journal.pending(), Some(&receipt));
            assert!(journal.published().is_none());
            let mut b = budget(&mut store, &f);
            let answer = head(&f, &mut store, &mut b).unwrap();
            assert!(answer.prove);
            assert_eq!(answer.receipt, Some(receipt));
        }
    }
}
