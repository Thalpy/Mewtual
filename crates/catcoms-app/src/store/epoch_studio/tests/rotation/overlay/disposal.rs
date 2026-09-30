//! The disposal transaction, D1 to D6, against real branches in a real vault.
//!
//! Every test here drives `dispose_studio_overlay_with_io`, the production transaction, and the
//! branches come from the ordinary Save path. Nothing constructs a branch or a request by hand.
use super::archive::frame_branch;
use super::*;
use crate::store::epoch_intents::disposal::{
    StudioDisposalRequestMode, StudioOverlayDisposalRequest,
};
use catcoms_replication::studio::{
    StudioDiscardConfirmation, StudioDisposalMode, StudioDraftArchive, StudioOverlayProvenance,
};

fn confirmation() -> StudioDiscardConfirmation {
    StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap()
}

/// The request a correctly behaved caller would build from the current inspection.
fn honest_request(
    f: &Fixture,
    store: &mut ServerStore,
    mode: StudioDisposalRequestMode,
) -> StudioOverlayDisposalRequest {
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let metadata = state.handoff_metadata().unwrap();
    StudioOverlayDisposalRequest {
        branch: metadata.branch_id().unwrap(),
        content: metadata.branch_content(&state.ledger).unwrap(),
        accepted: state.overlay().unwrap().accepted(),
        mode,
    }
}

fn dispose(
    f: &Fixture,
    store: &mut ServerStore,
    request: &StudioOverlayDisposalRequest,
) -> Result<catcoms_replication::studio::StudioOverlayDisposal, AppError> {
    let mut b = budget(store, f);
    store.dispose_studio_overlay_with_io(
        SERVER,
        &f.logical,
        f.target,
        &f.group,
        &f.device,
        request,
        4242,
        &mut rng(),
        &mut b.storage,
        &mut b.intents,
        &mut WriteHooks::None,
    )
}

/// Build and persist the archive for the current branch, which is what a preserving disposal needs
/// to already exist.
fn preserve_archive(f: &Fixture, store: &mut ServerStore) -> StudioDraftArchive {
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let metadata = state.handoff_metadata().unwrap();
    let archive = StudioDraftArchive::from_branch(
        state.overlay().unwrap(),
        &state.ledger,
        StudioOverlayProvenance::Closing,
        true,
        metadata.branch_id().unwrap(),
        metadata.branch_content(&state.ledger).unwrap(),
        metadata.branch_generation(),
    )
    .unwrap();
    drop(state);
    let mut b = budget(store, f);
    store
        .write_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            &archive,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            &mut WriteHooks::None,
        )
        .expect("the archive must persist");
    archive
}

/// The whole transaction, discarding: the branch goes, its entries go, the manifest stays, and all
/// three land in one replacement.
#[test]
fn a_discarding_disposal_removes_the_branch_and_its_entries_in_one_replacement() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    // Split the ledger into the branch's entries and everything else, using the branch's own
    // membership rather than assuming the ledger holds nothing but the branch. This vault does hold
    // other pending intents, and a disposal that retired those too would be destroying work it was
    // never asked about - so the split is the point of the test, not incidental to it.
    let before = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let overlay = before.overlay().unwrap();
    let (branch_ids, other_ids): (Vec<[u8; 32]>, Vec<[u8; 32]>) = before
        .pending()
        .map(|(id, _)| *id)
        .partition(|id| overlay.contains(id));
    assert!(!branch_ids.is_empty(), "the branch must hold entries");
    assert!(
        !other_ids.is_empty(),
        "this fixture must hold a pending intent outside the branch, or the test cannot show that \
         disposal leaves those alone"
    );
    drop(before);

    let request = honest_request(
        &f,
        &mut store,
        StudioDisposalRequestMode::Discard(confirmation()),
    );
    let manifest = dispose(&f, &mut store, &request).expect("a confirmed discard must succeed");
    assert_eq!(manifest.mode, StudioDisposalMode::Discarded);
    assert_eq!(manifest.accepted, branch_ids.len());

    // Reopen: the durable record is the one that matters.
    drop(store);
    let store = open(root.path());
    let after = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(
        after.overlay().is_none(),
        "the branch must be gone from the durable record"
    );
    assert_eq!(
        after.handoff_metadata().unwrap().disposed().unwrap().mode,
        StudioDisposalMode::Discarded,
        "the terminal manifest must be durable"
    );
    for id in &branch_ids {
        assert!(
            !after.pending().any(|(pending, _)| pending == id),
            "entry {id:?} was acknowledged as disposed but is still in the ledger"
        );
    }
    for id in &other_ids {
        assert!(
            after.pending().any(|(pending, _)| pending == id),
            "intent {id:?} was never part of the branch and must survive its disposal"
        );
    }
}

/// D4: a preserving disposal requires a durable archive for this exact branch, and the manifest names
/// it so a reader can find the bodies.
#[test]
fn a_preserving_disposal_requires_a_durable_archive_and_names_it() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    // Without an archive, refused, and the branch survives.
    let request = honest_request(&f, &mut store, StudioDisposalRequestMode::Preserve);
    assert!(
        dispose(&f, &mut store, &request).is_err(),
        "a preserving disposal with no archive must be refused"
    );
    assert!(
        store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .overlay()
            .is_some(),
        "a refused disposal must leave the branch intact"
    );

    // With one, it succeeds and the manifest names that exact archive.
    let archive = preserve_archive(&f, &mut store);
    let request = honest_request(&f, &mut store, StudioDisposalRequestMode::Preserve);
    let manifest = dispose(&f, &mut store, &request).expect("a preserving disposal must succeed");
    assert_eq!(
        manifest.mode,
        StudioDisposalMode::Preserved {
            archive: archive.archive_id().unwrap()
        },
        "the manifest must name the archive that holds the bodies"
    );
}

/// D4's full-envelope half. An archive for a **different** branch must not authorise this disposal,
/// even though it is a perfectly valid archive for this document.
#[test]
fn a_preserving_disposal_refuses_an_archive_for_another_branch() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    // An archive taken now, then the branch grows. The archive is still valid evidence - of a
    // smaller branch. Destroying the larger one on its strength would lose the extra work.
    preserve_archive(&f, &mut store);
    save(
        &f,
        &mut store,
        &close,
        basis.fingerprint(),
        f.domain(
            FlipnoteOp::SetHeader(FlipnoteHeader::Title("work after the archive".into()))
                .encode()
                .unwrap(),
            0x7f,
        ),
        400,
    );

    let request = honest_request(&f, &mut store, StudioDisposalRequestMode::Preserve);
    assert!(
        dispose(&f, &mut store, &request).is_err(),
        "an archive of a smaller branch must not authorise disposing of the larger one"
    );
    assert!(
        store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .overlay()
            .is_some(),
        "the branch must survive"
    );
}

/// D3, all three halves. Each is a distinct check with a distinct meaning, and each must refuse on
/// its own without the others masking it.
#[test]
fn d3_refuses_a_wrong_branch_a_wrong_content_and_a_wrong_count_separately() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    for (label, mutate) in [
        (
            "branch",
            (|r: &mut StudioOverlayDisposalRequest| r.branch[0] ^= 0xff)
                as fn(&mut StudioOverlayDisposalRequest),
        ),
        ("content", |r| r.content[0] ^= 0xff),
        ("accepted", |r| r.accepted += 1),
    ] {
        let mut request = honest_request(
            &f,
            &mut store,
            StudioDisposalRequestMode::Discard(confirmation()),
        );
        mutate(&mut request);
        assert!(
            dispose(&f, &mut store, &request).is_err(),
            "a wrong {label} must refuse"
        );
        assert!(
            store
                .load_epoch_intents(SERVER, &f.logical)
                .unwrap()
                .overlay()
                .is_some(),
            "a disposal refused on {label} must leave the branch intact"
        );
    }

    // The honest request still works, so each refusal above was about its own field.
    let request = honest_request(
        &f,
        &mut store,
        StudioDisposalRequestMode::Discard(confirmation()),
    );
    assert!(dispose(&f, &mut store, &request).is_ok());
}

/// D1's authorship half. Group membership is not standing over another device's local draft.
#[test]
fn only_the_branchs_own_author_may_dispose_of_it() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let request = honest_request(
        &f,
        &mut store,
        StudioDisposalRequestMode::Discard(confirmation()),
    );

    // A second device that is NOT a member: refused at membership.
    let stranger = MlsDevice::generate().unwrap();
    let mut b = budget(&mut store, &f);
    assert!(
        store
            .dispose_studio_overlay_with_io(
                SERVER,
                &f.logical,
                f.target,
                &f.group,
                &stranger,
                &request,
                4242,
                &mut rng(),
                &mut b.storage,
                &mut b.intents,
                &mut WriteHooks::None,
            )
            .is_err(),
        "a non-member must not dispose of a branch"
    );
    drop(b);
    assert!(
        store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .overlay()
            .is_some(),
        "the branch must survive"
    );
    // The author still can, so the refusal was about the device.
    assert!(dispose(&f, &mut store, &request).is_ok());
}

/// The transaction is atomic in the direction that matters: a failure anywhere leaves the branch,
/// its entries and any archive exactly as they were.
#[test]
fn a_disposal_that_fails_at_the_write_leaves_the_branch_and_its_entries_intact() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    let before = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let ids: Vec<[u8; 32]> = before.pending().map(|(id, _)| *id).collect();
    assert!(!ids.is_empty());
    drop(before);

    let request = honest_request(
        &f,
        &mut store,
        StudioDisposalRequestMode::Discard(confirmation()),
    );
    {
        let mut refuse = |_t: WriteTag, _p: &std::path::Path, _b: &[u8]| {
            Intercept::Fail(AppError::Io(
                "injected refusal before the disposal write".into(),
            ))
        };
        let mut hooks = WriteHooks::Hooked {
            before: Some(&mut refuse),
            before_sync: None,
            before_unlink: None,
            after: None,
        };
        let mut b = budget(&mut store, &f);
        assert!(
            store
                .dispose_studio_overlay_with_io(
                    SERVER,
                    &f.logical,
                    f.target,
                    &f.group,
                    &f.device,
                    &request,
                    4242,
                    &mut rng(),
                    &mut b.storage,
                    &mut b.intents,
                    &mut hooks,
                )
                .is_err(),
            "a refusal before the write must fail the call"
        );
    }

    drop(store);
    let store = open(root.path());
    let after = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(
        after.overlay().is_some(),
        "a disposal refused before its write must leave the branch"
    );
    assert!(
        after.handoff_metadata().unwrap().disposed().is_none(),
        "and must leave no terminal manifest"
    );
    for id in &ids {
        assert!(
            after.pending().any(|(pending, _)| pending == id),
            "entry {id:?} was retired by a disposal that never wrote"
        );
    }
}
