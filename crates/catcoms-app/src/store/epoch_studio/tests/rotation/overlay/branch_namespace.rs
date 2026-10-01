//! The branch-generation namespace on the real Flow S path.
//!
//! Agent 2's core tests prove the classifier and the admission in isolation. These prove the Save
//! path actually consults them: every request goes through `save_studio_closing_overlay`, every
//! branch is disposed through the production disposal transaction, and nothing here constructs a
//! branch, an admission or a request by hand.
use super::*;
use crate::store::epoch_intents::disposal::{
    StudioDisposalRequestMode, StudioOverlayDisposalRequest,
};
use catcoms_replication::studio::{
    StudioDiscardConfirmation, StudioOverlayDisposal, StudioOverlaySave,
};

/// A distinct title operation per nonce, so the two branches' operations are disjoint.
fn op(f: &Fixture, nonce: u8) -> DomainOp {
    let mut op = f.title();
    op.nonce = [nonce; 16];
    op
}

/// One Save through the real entry point, naming `branch` exactly as the request carried it.
fn send(
    f: &Fixture,
    store: &mut ServerStore,
    close: &CloseRecord,
    basis: [u8; 32],
    branch: [u8; 32],
    op: DomainOp,
) -> Result<StudioOverlaySave, AppError> {
    let mut b = budget(store, f);
    store.save_studio_closing_overlay(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        close,
        StudioOwnerTenure::Known(0),
        basis,
        branch,
        op,
        500,
        &mut rng(),
        &mut b,
    )
}

/// Discard the live branch through the production transaction, with the request a correctly
/// behaved caller builds from the inspection it was shown.
fn discard_live(f: &Fixture, store: &mut ServerStore) -> StudioOverlayDisposal {
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let metadata = state.handoff_metadata().unwrap();
    let request = StudioOverlayDisposalRequest {
        branch: metadata.branch_id().unwrap(),
        content: metadata.branch_content(&state.ledger).unwrap(),
        accepted: state.overlay().unwrap().accepted(),
        mode: StudioDisposalRequestMode::Discard(
            StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap(),
        ),
    };
    drop(state);
    let mut b = budget(store, f);
    store
        .dispose_studio_overlay_with_io(
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
        .unwrap()
}

fn intents_record(f: &Fixture, store: &ServerStore) -> Vec<u8> {
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    fs::read(store.epoch_intent_path(&scope)).unwrap()
}

/// Design N17b, Agent 2's acceptance sequence: accept G1, dispose it; accept a disjoint G2 on the
/// same basis, dispose it; restart; then deliver delayed requests from both. G2's is acknowledged
/// from the retained manifest, and **G1's is refused as stale and opens nothing**.
///
/// **The control is the same request, earlier.** Before G2 is disposed, G1's manifest is still the
/// retained one, so the identical delayed G1 request is answered with a terminal `Disposed`
/// acknowledgement - Agent 2's note that the shorter dispose-G1 / admit-G2 / retry-G1 sequence
/// does not reproduce the hazard, because the manifest catches it. Running both in one test is
/// what shows the long sequence, and specifically G2's disposal replacing G1's manifest, is what
/// moves the answer from acknowledgement to `Stale`. Without the namespace, that moment is where a
/// delayed G1 request would have been accepted into a new branch.
///
/// Both targets, because Flipnote and Index classify against differently shaped records.
#[test]
fn a_delayed_request_for_a_branch_whose_manifest_was_replaced_is_stale_on_the_save_path() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (close, basis) = closing(&f, &mut store);
        let basis = basis.fingerprint();
        let x = op(&f, 101);
        let y = op(&f, 201);

        // G1, accepted then discarded.
        let g1 = request_branch(&f, &mut store, &close);
        assert!(matches!(
            send(&f, &mut store, &close, basis, g1, x.clone()).unwrap(),
            StudioOverlaySave::Local(_)
        ));
        discard_live(&f, &mut store);

        // G2 on the very same basis: a new generation, so a different identity.
        let g2 = request_branch(&f, &mut store, &close);
        assert_ne!(
            g2, g1,
            "the second branch reused the first branch's identity"
        );
        assert!(matches!(
            send(&f, &mut store, &close, basis, g2, y.clone()).unwrap(),
            StudioOverlaySave::Local(_)
        ));

        // The control: G1's manifest is still the retained one, so the delayed G1 request is
        // owed - and given - its terminal acknowledgement, and nothing is written.
        let before = intents_record(&f, &store);
        match send(&f, &mut store, &close, basis, g1, x.clone()).unwrap() {
            StudioOverlaySave::Disposed(manifest) => assert_eq!(
                manifest.branch, g1,
                "acknowledged against the wrong manifest"
            ),
            other => panic!("the delayed G1 request was not acknowledged as disposed: {other:?}"),
        }
        assert_eq!(intents_record(&f, &store), before);

        // G2 disposed: its manifest replaces G1's. Restart, so nothing in memory stands in for the
        // durable record.
        discard_live(&f, &mut store);
        drop(store);
        let mut store = open(root.path());
        let before = intents_record(&f, &store);
        let generation = store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .handoff_metadata()
            .unwrap()
            .branch_generation();
        assert_eq!(
            generation, 2,
            "the generation is not monotonic across both disposals"
        );

        // G2's delayed request: acknowledged from the retained manifest.
        match send(&f, &mut store, &close, basis, g2, y).unwrap() {
            StudioOverlaySave::Disposed(manifest) => assert_eq!(manifest.branch, g2),
            other => panic!("the delayed G2 request was not acknowledged as disposed: {other:?}"),
        }

        // G1's delayed request: stale, and it opens nothing.
        let refused = send(&f, &mut store, &close, basis, g1, x.clone());
        assert!(
            matches!(refused, Err(AppError::Invalid(ref s)) if s.contains("stale branch")),
            "a delayed request for a branch whose manifest was replaced was not refused as \
             stale: {refused:?}"
        );
        assert_eq!(
            intents_record(&f, &store),
            before,
            "the refused request changed the intent record"
        );
        let after = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert!(
            after.overlay().is_none(),
            "the refused request opened a branch"
        );
        assert!(
            !after
                .pending()
                .any(|(id, _)| *id == x.id(&f.device.device_id())),
            "the refused request left a ledger entry"
        );
        assert_eq!(
            after.handoff_metadata().unwrap().branch_generation(),
            generation,
            "the refused request moved the generation"
        );
    }
}

/// The one way a branch is opened is admission at S1b, so a request naming an identity that no
/// admission would mint is refused there even on a document with no record at all - the
/// generation-1 case `admit_first_branch` exists for.
///
/// The positive control is the same operation with the branch the ticket actually names, which
/// is accepted: the refusal is the identity, not the operation or the fixture.
#[test]
fn a_first_save_naming_an_identity_no_admission_would_mint_is_stale() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let basis = basis.fingerprint();
    let named = request_branch(&f, &mut store, &close);
    let mut forged = named;
    forged[0] ^= 1;
    let before = store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .handoff_metadata()
        .is_none();
    assert!(before, "the fixture must start with no overlay record");

    let refused = send(&f, &mut store, &close, basis, forged, op(&f, 7));
    assert!(
        matches!(refused, Err(AppError::Invalid(ref s)) if s.contains("stale branch")),
        "a first Save naming an unminted identity was not refused as stale: {refused:?}"
    );
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .handoff_metadata()
        .is_none());

    assert!(matches!(
        send(&f, &mut store, &close, basis, named, op(&f, 7)).unwrap(),
        StudioOverlaySave::Local(_)
    ));
}
