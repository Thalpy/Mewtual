//! Draft archive against real PIX-bearing branches.
//!
//! The codec's own tests live in `catcoms-replication`, but that crate's settlement fixture
//! authors header edits only, which carry no blob reference, so it can establish the base half
//! of `blob_cids` and nothing more. A comparison written there against both halves passed with
//! operation collection deleted, because both sides were the base set. This module pays that
//! debt where real frames and real published pixels exist.
use super::*;
use catcoms_replication::studio::{
    operation_blob_cid, StudioDraftArchive, StudioOverlayProvenance,
};

/// A branch whose accepted operations carry genuine published pixels, plus the CIDs they name.
/// Frames anchor to `[1; 16]`, which this fixture's base already holds.
fn frame_branch(
    f: &Fixture,
    store: &mut ServerStore,
    close: &CloseRecord,
    basis: &StudioClosingOverlayBasis,
) -> Vec<[u8; 32]> {
    let (insert_cid, insert_bytes) = published_pix(store, f, 0x51);
    let (replace_cid, replace_bytes) = published_pix(store, f, 0x52);
    let bodies = [
        FlipnoteOp::InsertFrame {
            frame: [2; 16],
            after: Some([1; 16]),
            cid: insert_cid,
            bytes: insert_bytes,
        },
        FlipnoteOp::ReplaceFrame {
            frame: [2; 16],
            cid: replace_cid,
            bytes: replace_bytes,
        },
    ];
    for (i, body) in bodies.into_iter().enumerate() {
        // `domain`'s second argument fills the whole nonce, so each operation needs a distinct
        // one: equal nonces give equal ids and the second acceptance would collide rather than
        // append. The range avoids the values this fixture's own base operations use.
        save(
            f,
            store,
            close,
            basis.fingerprint(),
            f.domain(body.encode().unwrap(), 0x71 + i as u8),
            300 + i as u64,
        );
    }
    vec![insert_cid, replace_cid]
}

#[test]
fn draft_archive_collects_the_same_references_a_live_branch_protects() {
    // The preservation guarantee is exactly this: whatever the branch kept alive, the archive
    // keeps alive. Both halves, base and operations, against the live branch's own sets.
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let operation_cids = frame_branch(&f, &mut store, &close, &basis);

    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let overlay = state.overlay().unwrap();
    let base = overlay.base_blob_cids().unwrap();

    // The live branch's complete conservative set, assembled the way the inventory arm does.
    let mut expected = base.clone();
    for (_, intent) in state.pending() {
        if let Some(cid) = operation_blob_cid(&intent.operation).unwrap() {
            expected.insert(cid);
        }
    }

    // Guard the guard. Without this the comparison below would hold even with operation
    // collection deleted, which is precisely how the replication-crate version of this test
    // passed while proving nothing.
    for cid in &operation_cids {
        assert!(
            expected.contains(cid),
            "the fixture must reference {cid:?} through an accepted operation"
        );
        assert!(
            !base.contains(cid),
            "operation references must not also come from the base, or the halves are not \
             separable and this test cannot distinguish them"
        );
    }

    let archive = StudioDraftArchive::from_branch(
        overlay,
        &state.ledger,
        StudioOverlayProvenance::Closing,
        true,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap();
    let read = StudioDraftArchive::decode(&archive.encode().unwrap()).unwrap();
    assert_eq!(
        read.blob_cids().unwrap(),
        expected,
        "the archive's reference set must equal the live branch's"
    );
}

/// N19. The preservation guarantee, end to end through the real scanner: after a preserving
/// disposal the archive is the only thing keeping the branch's pixels alive, and a complete
/// reference scan must still protect them.
///
/// Two ways this could pass while proving nothing, both designed against:
///
/// 1. **Another holder.** If the live branch, the source or recovery still named the CIDs, the
///    assertion would hold with the archive collector deleted. So the intent record and the
///    source are removed first, leaving the archive as the sole durable namer.
/// 2. **Fail-closed protection.** A scan that refuses installs no known set, and deletions are
///    then refused wholesale. Membership in the returned pin set is a positive assertion, so an
///    empty or unknown protection cannot satisfy it; and the control below shows the same vault,
///    minus only the archive, does NOT pin them.
///
/// The control is what makes the result attributable: the CIDs are absent before the archive
/// exists and present after, in one vault, with nothing else changed.
#[test]
fn draft_archive_references_survive_a_complete_scan_as_the_sole_holder() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let operation_cids = frame_branch(&f, &mut store, &close, &basis);

    // The payload, taken while the live branch still exists.
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let payload = StudioDraftArchive::from_branch(
        state.overlay().unwrap(),
        &state.ledger,
        StudioOverlayProvenance::Closing,
        true,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap()
    .encode()
    .unwrap();
    let base_cids = state.overlay().unwrap().base_blob_cids().unwrap();
    drop(state);

    // Remove every other durable namer of those pixels: the accepted branch and the source.
    // The bare `scope_bytes` in scope here is the Studio family's; the intent record has its
    // own, and addressing it with the wrong one silently names a file that does not exist.
    let intent_scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    fs::remove_file(store.epoch_intent_path(&intent_scope)).unwrap();
    fs::remove_file(f.path(&store)).unwrap();
    drop(store);

    // Control: with no archive, nothing pins them.
    let mut store = open(root.path());
    let pins = store.creative_pinned_cids().unwrap();
    let pinned = |pins: &crate::store::CreativeReferences, cid: &[u8; 32]| {
        pins.for_group(&f.group.group_id())
            .any(|held| *held == catcoms_storage::Cid::from_bytes(*cid))
    };
    for cid in operation_cids.iter().chain(base_cids.iter()) {
        assert!(
            !pinned(&pins, cid),
            "control is broken: {cid:?} is still held by something other than the archive, so \
             the assertion below would pass with the collector deleted"
        );
    }
    drop(pins);

    // Now the archive is the sole holder.
    crate::store::epoch_draft_archive::write_draft_archive_for_test(
        &store,
        SERVER,
        &f.logical,
        &payload,
        &mut rng(),
    )
    .unwrap();
    drop(store);
    let mut store = open(root.path());
    let pins = store.creative_pinned_cids().unwrap();
    for cid in operation_cids.iter().chain(base_cids.iter()) {
        assert!(
            pinned(&pins, cid),
            "archived pixel {cid:?} became reclaimable: the archive is the only thing naming it"
        );
    }
}

/// I-5's other half: the arm is narrowed, not removed. An archive whose payload will not decode
/// must still fail the scan closed, exactly as a corrupt record of any other family does, rather
/// than completing with a known set that silently omits whatever it was protecting.
#[test]
fn an_undecodable_draft_archive_still_fails_a_reference_scan_closed() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    // A well-framed record whose body is not a draft archive. Everything the seam validates
    // still holds: canonical scope, filename agreement, sealing, framing and the bound.
    crate::store::epoch_draft_archive::write_draft_archive_for_test(
        &store,
        SERVER,
        &f.logical,
        b"not a draft archive payload",
        &mut rng(),
    )
    .unwrap();
    drop(store);
    let mut store = open(root.path());

    let refused = store.creative_pinned_cids();
    assert!(
        refused.is_err(),
        "an archive whose payload cannot be decoded must fail the scan closed, not be skipped"
    );
}

#[test]
fn draft_archive_of_a_frame_branch_round_trips_through_real_storage() {
    // The replication round trip uses header edits. Frame operations carry a CID and a declared
    // byte count, and a branch mixing inserts with a replace of the same frame is the shape a
    // real disposal is most likely to meet, so round trip that one too.
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let overlay = state.overlay().unwrap();
    let bytes = StudioDraftArchive::from_branch(
        overlay,
        &state.ledger,
        StudioOverlayProvenance::Closing,
        true,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap()
    .encode()
    .unwrap();

    let read = StudioDraftArchive::decode(&bytes).unwrap();
    assert_eq!(read.accepted(), 2);
    assert_eq!(read.target(), f.target);
    assert_eq!(read.author(), f.device.device_id());
    assert_eq!(read.basis(), overlay.basis());
    assert_eq!(read.document(), &f.logical);
    assert_eq!(read.encode().unwrap(), bytes);
}
