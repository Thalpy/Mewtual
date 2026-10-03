use super::*;
use catcoms_replication::studio::catchup::{
    StudioPageOutcome, StudioPageProvider, StudioPageRequest,
};

pub(super) fn interrupt(f: &Fixture, store: &mut ServerStore, basis: [u8; 32], step: WriteTag) {
    let mut b = budget(store, f);
    let mut hit = false;
    let error = store
        .handoff_studio_overlay_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::Hooked {
                before: Some(&mut |at: WriteTag, _: &Path, _: &[u8]| {
                    if at == step {
                        hit = true;
                        return Intercept::Fail(invalid("interrupted transfer"));
                    }
                    Intercept::Continue
                }),
                before_sync: None,
                before_unlink: None,
                after: None,
            },
        )
        .unwrap_err();
    assert!(hit && error.to_string().contains("interrupted transfer"));
}

#[test]
fn studio_overlay_handoff_publication_and_shared_replacement_fences_survive_restart() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(true);
    let mut store = open(root.path());
    let requester = MlsDevice::generate().unwrap();
    f.group
        .add_member(&f.device, requester.key_package().unwrap())
        .unwrap();
    let (_, basis, _) = prepare(&f, &mut store);
    interrupt(&f, &mut store, basis, WriteTag::Completed);
    drop(store);
    let mut store = open(root.path());
    warm(&f, &mut store);
    let source = f.load(&store).unwrap();
    let epoch_id = source.doc_id();
    let seed = source
        .unit
        .receipt_head()
        .unwrap()
        .unwrap()
        .seed_change_hash;
    assert_eq!(source.op_count(), 1);
    let mut pager = StudioPageProvider::new(
        f.device.device_id(),
        Arc::new(ManualClock::new(1000)),
        &mut rng(),
    );
    let req = || StudioPageRequest {
        requester: requester.device_id(),
        doc_id: epoch_id,
        heads: &[],
        seed: Some(seed),
        cursor: None,
    };
    let error = store
        .serve_studio_page(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &mut pager,
            req(),
            &mut rng(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("prepared overlay handoff is not publishable"),
        "{error}"
    );
    let original = canonical(&store);
    let mut b = budget(&mut store, &f);
    let observed = source.source.as_ref().map(SourceVersion::record);
    let mut unit = source.unit;
    let before = unit.snapshot().unwrap();
    let replacement = StudioEpoch::new(&f.group, f.target, f.device.device_id()).unwrap();
    let mut wrote = false;
    let result = store.save_studio_source(
        SERVER,
        replacement,
        observed,
        &before,
        WritePurpose::Ordinary,
        &mut rng(),
        &mut b.storage,
        WriteStep::new(WriteTag::Source),
        // Records whether a replacement was even attempted; the refusal under test happens
        // before this point, so it must not fire.
        &mut WriteHooks::Hooked {
            before: Some(&mut |_: WriteTag, _: &Path, _: &[u8]| {
                wrote = true;
                Intercept::Continue
            }),
            before_sync: None,
            before_unlink: None,
            after: None,
        },
    );
    assert!(
        matches!(result,Err(AppError::Invalid(ref s)) if s.contains("prepared handoff blocks source replacement")),
        "Prepared source was replaced: {result:?}"
    );
    assert!(!wrote);
    assert_eq!(canonical(&store), original);
    let mut b = budget(&mut store, &f);
    let result = store.edit_studio_epoch(
        SERVER,
        &f.group,
        f.target,
        epoch_id,
        &f.device,
        f.title(),
        999,
        &mut rng(),
        &mut b,
    );
    assert!(
        matches!(result,Err(AppError::Invalid(ref s)) if s.contains("handoff must resolve before ordinary Apply"))
    );
    transfer(&f, &mut store, basis);
    warm(&f, &mut store);
    assert!(matches!(
        store
            .serve_studio_page(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &mut pager,
                req(),
                &mut rng()
            )
            .unwrap(),
        StudioPageOutcome::Page(_)
    ));
}

#[test]
fn studio_overlay_handoff_missing_metadata_cannot_become_publishable_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    interrupt(&f, &mut store, basis, WriteTag::Completed);
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let path = store.epoch_intent_path(&scope);
    let saved = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    drop(store);
    let mut store = open(root.path());
    let result = store.load_studio_epoch(SERVER, &f.group, f.target, &f.device);
    assert!(
        matches!(result,Err(AppError::Invalid(ref s)) if s.contains("required handoff metadata missing")),
        "missing Prepared record exposed its source: {result:?}"
    );
    fs::write(path, saved).unwrap();
    transfer(&f, &mut store, basis);
    grow(&f, &mut store);
    let (_, next) = rotate(&f, &mut store);
    assert_eq!(next.epoch(), 2);
    let path = store.epoch_intent_path(&scope);
    fs::remove_file(path).unwrap();
    drop(store);
    let store = open(root.path());
    assert!(
        store
            .load_studio_epoch(SERVER, &f.group, f.target, &f.device)
            .is_err(),
        "rotation dropped the required intent link"
    );
}

#[test]
fn studio_overlay_handoff_owner_rotation_resolves_committed_batch_before_retirement() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, _) = prepare(&f, &mut store);
        interrupt(&f, &mut store, basis, WriteTag::Completed);
        grow(&f, &mut store);
        drop(store);
        let mut store = open(root.path());
        warm(&f, &mut store);
        assert!(store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .handoff_prepared());
        let (_, next) = rotate(&f, &mut store);
        assert_eq!(next.epoch(), 2);
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert!(state.overlay().is_none() && state.pending().len() == 0);
        assert!(state
            .handoff_metadata()
            .unwrap()
            .completed_branch(f.target, f.device.device_id(), basis)
            .unwrap()
            .is_some());
    }
}

#[test]
fn studio_overlay_handoff_absent_manifest_returns_to_active_after_unrelated_ingest() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, expected) = prepare(&f, &mut store);
    interrupt(&f, &mut store, basis, WriteTag::Source);
    let mut source = f.load(&store).unwrap().unit;
    let mut unrelated = f.title();
    unrelated.nonce = [77; 16];
    let packet = source
        .edit_or_reseal(&f.device, &f.group, &mut rng(), &unrelated, 555)
        .unwrap();
    let mut b = budget(&mut store, &f);
    store
        .ingest_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &packet,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    drop(store);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    store
        .resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap();
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(!state.handoff_prepared());
    assert_eq!(
        state.local_draft().unwrap().unwrap().projection(),
        &expected
    );
    assert_eq!(f.load(&store).unwrap().op_count(), 1);
    let original = canonical(&store);
    let mut b = budget(&mut store, &f);
    assert!(store
        .handoff_studio_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b
        )
        .is_err());
    assert_eq!(
        canonical(&store),
        original,
        "non-pristine successor was silently rebased"
    );
}

#[test]
fn studio_overlay_handoff_adoption_resolves_before_pruning_signed_history() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    interrupt(&f, &mut store, basis, WriteTag::Completed);
    drop(store);
    let mut store = open(root.path());
    warm(&f, &mut store);
    let mut projection = f.load(&store).unwrap().projection().unwrap();
    let StudioProjection::Flipnote(ref mut p) = projection else {
        unreachable!()
    };
    p.epoch = 3;
    let seed = projection.checkpoint([99; 32]).unwrap();
    let receipt = Receipt::sign(
        f.logical.clone(),
        3,
        [99; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &f.device,
    )
    .unwrap();
    let mut b = budget(&mut store, &f);
    let (outcome, state) = store
        .adopt_studio_checkpoint(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &receipt,
            Some(seed.bytes()),
            0,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(outcome, crate::store::StudioAdoptionOutcome::Installed);
    assert_eq!(state.epoch(), 4);
    let intents = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(intents.overlay().is_none());
    assert_eq!(intents.pending().len(), 1);
    assert!(intents
        .handoff_metadata()
        .unwrap()
        .completed_branch(f.target, f.device.device_id(), basis)
        .unwrap()
        .is_some());
    assert!(store
        .load_epoch_recovery(SERVER, &f.logical)
        .unwrap()
        .retained()
        .any(|record| {
            StudioRecovery::from_snapshot(record, &f.logical, f.target.channel())
                .unwrap()
                .operations()
                .contains_key(&f.title().id(&f.device.device_id()))
        }));
}

#[test]
fn studio_overlay_handoff_frozen_owner_takeover_preserves_completed_local_work() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    interrupt(&f, &mut store, basis, WriteTag::Completed);
    grow(&f, &mut store);
    let mut old_source = f.load(&store).unwrap();
    let previous = old_source.unit.receipt_head().unwrap().cloned().unwrap();
    let decision = old_source
        .unit
        .new_owner_decision(&f.group, &f.device, 0, Some(&previous))
        .unwrap();
    let mut b = budget(&mut store, &f);
    store
        .prepare_studio_owner_decision_with_writer(
            SERVER,
            &decision,
            &f.group,
            0,
            &mut rng(),
            &mut b.storage,
            &mut WriteHooks::None,
        )
        .unwrap();
    let (_, sealed) = store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            decision.receipt().clone(),
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(sealed.phase(), EpochPhase::Closing);
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .handoff_prepared());
    let author = f.device.device_id();
    let next = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.device, next.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut group = ServerGroup::join(&next, &welcome).unwrap();
    group.remove_member(&next, &author).unwrap();
    f.group = group;
    f.device = next;
    drop(store);
    let mut store = open(root.path());
    warm(&f, &mut store);
    let mut b = budget(&mut store, &f);
    let (_, state) = store
        .rotate_studio_owner(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            f.group.epoch(),
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(state.epoch(), 2);
    let intents = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(intents.overlay().is_none());
    assert!(intents
        .handoff_metadata()
        .unwrap()
        .completed_branch(f.target, author, basis)
        .unwrap()
        .is_some());
}

#[test]
fn studio_overlay_handoff_all_exact_resolution_in_fault_does_not_clear_fault() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    interrupt(&f, &mut store, basis, WriteTag::Completed);
    let conflict = Receipt::sign(
        f.logical.clone(),
        0,
        [99; 32],
        [88; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &f.device,
    )
    .unwrap();
    let mut b = budget(&mut store, &f);
    let (outcome, state) = store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            conflict,
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(outcome, ReceiptIngest::Fault);
    assert_eq!(state.phase(), EpochPhase::Fault);
    drop(store);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    store
        .resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap();
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    let intents = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(intents.overlay().is_none());
    assert!(intents
        .handoff_metadata()
        .unwrap()
        .completed_branch(f.target, f.device.device_id(), basis)
        .unwrap()
        .is_some());
}
