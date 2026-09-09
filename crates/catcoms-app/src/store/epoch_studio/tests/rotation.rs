//! Real typed signed closures, with a large (but accepted) Automerge message to reach the
//! production lower bound cheaply. Failure seams wrap the actual vault writer, never the gate.
use super::super::rotation::{RotationSync, RotationWrite};
use super::super::source::SourceVersion;
use super::*;
use catcoms_replication::studio::StudioRecovery;
use catcoms_rt::ManualClock;

fn eligible(f: &Fixture, store: &mut ServerStore) {
    let mut b = budget(store, f);
    let (_, state) = f.edit(store, &mut b, f.insert());
    let observed = state.source.as_ref().map(SourceVersion::record);
    let mut unit = state.unit;
    let before = unit.snapshot().unwrap();
    for n in 10..20 {
        let mut op = f.title();
        op.nonce = [n; 16];
        let mut copy = StudioEpoch::restore(
            &unit.snapshot().unwrap(),
            &f.group,
            f.target,
            f.device.device_id(),
        )
        .unwrap();
        let packet = copy
            .edit_or_reseal(&f.device, &f.group, &mut rng(), &op, 100)
            .unwrap();
        let mut expanded = automerge::Change::from_bytes(f.signed(&packet).delta)
            .unwrap()
            .decode();
        expanded.message = Some("x".repeat(220_000));
        let change = automerge::Change::from(expanded);
        let signed = SignedOp::sign_domain(
            &f.device,
            f.logical.doc_type,
            f.id,
            change.raw_bytes().to_vec(),
            &op,
        )
        .unwrap();
        let packet = SealedOp::seal(&signed, &f.group, &f.device, &mut rng()).unwrap();
        assert_eq!(
            unit.ingest(&packet, &f.group, &f.device).unwrap(),
            Admission::Accepted
        );
    }
    let state = store
        .save_studio_source(
            SERVER,
            unit,
            observed,
            &before,
            WritePurpose::Ordinary,
            &mut rng(),
            &mut b.storage,
            atomic_write,
            sync_studio,
        )
        .unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
}
fn warm(f: &Fixture, store: &mut ServerStore) {
    // Loading a cold source is an explicit test action, analogous to production detached prep.
    // Re-save unchanged only for a real physical stamp, never invent one from a normalized unit.
    let mut b = budget(store, f);
    let (unit, observed, before) = store
        .checked_studio_source(SERVER, &f.group, f.target, &f.device, false, &mut b.storage)
        .unwrap();
    let scope = scope_bytes(SERVER, &f.logical).unwrap();
    let actual = store.read_studio_record(&scope).unwrap().unwrap();
    let version = store
        .studio_source_version(SERVER, &unit, &actual.plain, actual.physical_bytes)
        .unwrap();
    let state = store
        .save_studio_source_reusing(
            SERVER,
            unit,
            observed,
            &before,
            WritePurpose::Settlement,
            &mut rng(),
            &mut b.storage,
            atomic_write,
            sync_studio,
            Some(version),
        )
        .unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
}
fn rotate(f: &Fixture, store: &mut ServerStore) -> (StudioRotationOutcome, EpochStudioState) {
    let mut b = budget(store, f);
    store
        .rotate_studio_owner(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            0,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
        )
        .unwrap()
}
fn sync(step: RotationSync, p: &Path, bytes: u64) -> Result<(), AppError> {
    match step {
        RotationSync::Intents => crate::store::epoch_intents::sync_intent(p, bytes),
        _ => sync_studio(p, bytes),
    }
}

#[test]
fn studio_rotation_store_installs_retires_covered_and_preserves_new_edits_on_retry() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        eligible(&f, &mut store);
        assert_eq!(f.intents(&store), 1);
        let (outcome, state) = rotate(&f, &mut store);
        assert_eq!(
            outcome,
            StudioRotationOutcome::Installed {
                publication_pending: true
            }
        );
        assert_eq!((state.epoch(), state.op_count()), (1, 0));
        assert_eq!(f.intents(&store), 0);
        let saved = store.load_epoch_owner_receipts(SERVER, &f.logical).unwrap();
        let receipt = saved.pending().unwrap().clone();
        assert!(saved.close_for(&receipt).is_some());
        let mut b = budget(&mut store, &f);
        let (_, new) = store
            .edit_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                state.doc_id(),
                &f.device,
                f.title(),
                100,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        let expected = new.projection().unwrap();
        drop(store);
        let mut store = open(root.path());
        warm(&f, &mut store);
        let (outcome, state) = rotate(&f, &mut store);
        assert_eq!(
            outcome,
            StudioRotationOutcome::AlreadyInstalled {
                publication_pending: true
            }
        );
        assert_eq!(state.projection().unwrap(), expected);
        assert_eq!(f.intents(&store), 1);
        if art {
            assert_eq!(
                store
                    .load_epoch_recovery(SERVER, &f.logical)
                    .unwrap()
                    .retained()
                    .len(),
                1
            );
        }
    }
}

#[test]
fn studio_rotation_store_every_write_crash_resumes_exact_decision_and_preserves_content() {
    for art in [false, true] {
        // Reuse the same real signed source across independent vaults; rebuilding its large
        // eligibility messages for every I/O failure would test no additional behavior.
        let fixture_root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut fixture_store = open(fixture_root.path());
        eligible(&f, &mut fixture_store);
        let snapshot = f.load(&fixture_store).unwrap().unit.snapshot().unwrap();
        for boundary in [
            RotationWrite::Journal,
            RotationWrite::Source,
            RotationWrite::Recovery,
            RotationWrite::Intents,
            RotationWrite::Successor,
        ] {
            // Index with no excluded/deleted state needs no recovery. Introduce an excluded
            // edit by persisting the decision first below, so every boundary is actually hit.
            for after in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let mut store = open(root.path());
                let mut initial = budget(&mut store, &f);
                store
                    .prepare_epoch_intent(
                        SERVER,
                        &f.logical,
                        f.insert(),
                        &f.device,
                        &f.group,
                        &mut rng(),
                        &mut initial.storage,
                        &mut initial.intents,
                    )
                    .unwrap();
                let unit =
                    StudioEpoch::restore(&snapshot, &f.group, f.target, f.device.device_id())
                        .unwrap();
                let state = store
                    .save_studio_source(
                        SERVER,
                        unit,
                        None,
                        &[],
                        WritePurpose::Ordinary,
                        &mut rng(),
                        &mut initial.storage,
                        atomic_write,
                        sync_studio,
                    )
                    .unwrap();
                store.retain_studio_source(&f.group, &f.device, state);
                let mut b = budget(&mut store, &f);
                let mut state = f.load(&store).unwrap();
                let decision = state
                    .unit
                    .new_owner_decision(&f.group, &f.device, 0, None)
                    .unwrap();
                store
                    .prepare_studio_owner_decision_with_writer(
                        SERVER,
                        &decision,
                        &f.group,
                        0,
                        &mut rng(),
                        &mut b.storage,
                        atomic_write,
                    )
                    .unwrap();
                f.edit(&mut store, &mut b, f.title());
                warm(&f, &mut store);
                let mut b = budget(&mut store, &f);
                let mut hit = false;
                let result = store.rotate_studio_owner_with_io(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    0,
                    &ManualClock::new(1000),
                    &mut rng(),
                    &mut b,
                    &mut |step, p, bytes| {
                        if step == boundary {
                            hit = true;
                            if after {
                                atomic_write(p, bytes)?;
                            }
                            return Err(AppError::Io("injected rotation write failure".into()));
                        }
                        atomic_write(p, bytes)
                    },
                    &mut sync,
                );
                assert!(result.is_err());
                assert!(hit, "boundary {boundary:?}");
                assert!(b.requires_reconciliation());
                drop(store);
                let mut store = open(root.path());
                let held = store.load_epoch_owner_receipts(SERVER, &f.logical).unwrap();
                assert_eq!(held.pending(), Some(decision.receipt()));
                warm(&f, &mut store);
                let (_, installed) = rotate(&f, &mut store);
                assert_eq!(installed.epoch(), 1);
                let intents = store.load_epoch_intents(SERVER, &f.logical).unwrap();
                assert_eq!(intents.pending().len(), 1);
                assert_eq!(intents.pending().next().unwrap().1.operation, f.title());
                let recovery = store.load_epoch_recovery(SERVER, &f.logical).unwrap();
                let r = StudioRecovery::from_snapshot(
                    recovery.retained().next().unwrap(),
                    &f.logical,
                    f.target.channel(),
                )
                .unwrap();
                assert!(r.operations().values().any(|op| op.operation == f.title()));
                if art {
                    assert!(store
                        .creative_pinned_cids()
                        .unwrap()
                        .for_group(&f.group.group_id())
                        .any(|cid| *cid == catcoms_storage::Cid::from_bytes([3; 32])));
                }
            }
        }
    }
}

#[test]
fn studio_rotation_store_missing_ineligible_or_bad_tenure_never_journals() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        for present in [false, true] {
            let mut b = budget(&mut store, &f);
            if present {
                f.edit(&mut store, &mut b, f.insert());
            }
            assert!(store
                .rotate_studio_owner(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    0,
                    &ManualClock::new(0),
                    &mut rng(),
                    &mut b
                )
                .is_err());
            assert!(store
                .load_epoch_owner_receipts(SERVER, &f.logical)
                .unwrap()
                .pending()
                .is_none());
            assert_eq!(f.intents(&store), usize::from(present));
        }
        let mut b = budget(&mut store, &f);
        assert!(store
            .rotate_studio_owner(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                1,
                &ManualClock::new(0),
                &mut rng(),
                &mut b
            )
            .is_err());
    }
}

#[test]
fn studio_rotation_store_source_flush_failure_precedes_any_journal_or_retirement() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    eligible(&f, &mut store);
    let before = fs::read(f.path(&store)).unwrap();
    let mut b = budget(&mut store, &f);
    assert!(store
        .rotate_studio_owner_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            0,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
            &mut |_, _, _| panic!("no write before source flush"),
            &mut |step, _, _| {
                assert_eq!(step, RotationSync::Source);
                Err(AppError::Io("flush".into()))
            }
        )
        .is_err());
    assert!(b.requires_reconciliation());
    assert_eq!(fs::read(f.path(&store)).unwrap(), before);
    assert!(store
        .load_epoch_owner_receipts(SERVER, &f.logical)
        .unwrap()
        .pending()
        .is_none());
    assert_eq!(f.intents(&store), 1);
}

#[test]
fn studio_rotation_store_pending_recovery_survives_restart_until_exact_ack() {
    use catcoms_replication::{RecoveryReason, RecoveryTransition};
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    eligible(&f, &mut store);
    let projection = f.load(&store).unwrap().projection().unwrap();
    let clock = ManualClock::new(1000);
    let mut b = budget(&mut store, &f);
    for salt in [41, 42] {
        let snapshot = StudioRecovery::snapshot(
            &projection,
            None,
            RecoveryReason::Excluded,
            [salt; 32],
            &Default::default(),
        )
        .unwrap();
        store
            .update_epoch_recovery_accounted(
                SERVER,
                &f.logical,
                EpochRecoveryAction::Stage(snapshot),
                &clock,
                &mut rng(),
                &mut b.storage,
            )
            .unwrap();
    }
    assert_eq!(
        rotate(&f, &mut store).0,
        StudioRotationOutcome::RecoveryPending
    );
    let warning = store
        .load_epoch_recovery(SERVER, &f.logical)
        .unwrap()
        .eviction_pending()
        .unwrap()
        .unwrap();
    assert_eq!(f.intents(&store), 1);
    drop(store);
    let mut store = open(root.path());
    warm(&f, &mut store);
    assert_eq!(
        rotate(&f, &mut store).0,
        StudioRotationOutcome::RecoveryPending
    );
    assert_eq!(
        store
            .load_epoch_recovery(SERVER, &f.logical)
            .unwrap()
            .eviction_pending()
            .unwrap(),
        Some(warning)
    );
    let RecoveryTransition::EvictionPending {
        oldest_snapshot,
        staged_snapshot,
        ..
    } = warning
    else {
        panic!("warning")
    };
    let mut b = budget(&mut store, &f);
    assert!(store
        .update_epoch_recovery_accounted(
            SERVER,
            &f.logical,
            EpochRecoveryAction::Acknowledge {
                oldest_snapshot: [0; 32],
                staged_snapshot
            },
            &clock,
            &mut rng(),
            &mut b.storage
        )
        .is_err());
    store
        .update_epoch_recovery_accounted(
            SERVER,
            &f.logical,
            EpochRecoveryAction::Acknowledge {
                oldest_snapshot,
                staged_snapshot,
            },
            &clock,
            &mut rng(),
            &mut b.storage,
        )
        .unwrap();
    warm(&f, &mut store);
    assert_eq!(rotate(&f, &mut store).1.epoch(), 1);
    assert_eq!(f.intents(&store), 0);
    assert_eq!(
        store
            .load_epoch_recovery(SERVER, &f.logical)
            .unwrap()
            .retained()
            .len(),
        2
    );
}

#[test]
fn studio_rotation_store_same_id_wrong_intent_body_cannot_be_retired() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        eligible(&f, &mut store);
        let mut b = budget(&mut store, &f);
        // Ballast operation 10 is accepted but has no local intent. Journal a DIFFERENT body
        // with its same nonce, proving the retirement barrier compares more than the id.
        let body = if art {
            FlipnoteOp::SetHeader(FlipnoteHeader::Title("not receipted".into()))
                .encode()
                .unwrap()
        } else {
            IndexOp::SetTitle {
                object: [1; 16],
                title: "not receipted".into(),
            }
            .encode()
            .unwrap()
        };
        store
            .prepare_epoch_intent(
                SERVER,
                &f.logical,
                f.domain(body, 10),
                &f.device,
                &f.group,
                &mut rng(),
                &mut b.storage,
                &mut b.intents,
            )
            .unwrap();
        let result = store.rotate_studio_owner(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            0,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("intent envelope conflicts"));
        assert_eq!(f.intents(&store), 2);
        let source = f.load(&store).unwrap();
        assert_eq!(
            (source.epoch(), source.phase(), source.op_count()),
            (0, EpochPhase::Closing, 11)
        );
    }
}

#[test]
fn studio_rotation_store_unwind_after_successor_write_poisoned_budget_reopens_safely() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    eligible(&f, &mut store);
    let mut b = budget(&mut store, &f);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.rotate_studio_owner_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            0,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
            &mut |step, p, bytes| {
                atomic_write(p, bytes)?;
                assert_ne!(step, RotationWrite::Successor, "injected post-write unwind");
                Ok(())
            },
            &mut sync,
        )
    }));
    assert!(result.is_err());
    assert!(b.requires_reconciliation());
    drop(store);
    let mut store = open(root.path());
    warm(&f, &mut store);
    assert_eq!(
        rotate(&f, &mut store).0,
        StudioRotationOutcome::AlreadyInstalled {
            publication_pending: true
        }
    );
    assert_eq!(f.intents(&store), 0);
    assert_eq!(
        store
            .load_epoch_recovery(SERVER, &f.logical)
            .unwrap()
            .retained()
            .len(),
        1
    );
}
