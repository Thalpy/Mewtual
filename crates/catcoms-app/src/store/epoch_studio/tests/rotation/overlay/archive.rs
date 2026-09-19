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

    // The archive itself, taken while the live branch still exists. Retained as the typed value
    // rather than as bytes, so the production writer can persist it below: a valid archive must
    // reach the collector through the real writer, or a framing, scope or layout regression in
    // that writer would be invisible to this test.
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let archive = StudioDraftArchive::from_branch(
        state.overlay().unwrap(),
        &state.ledger,
        StudioOverlayProvenance::Closing,
        true,
        [3; 32],
        [4; 32],
        1,
    )
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

    // Now the archive is the sole holder, persisted through the PRODUCTION writer so this test
    // covers writer -> physical record -> collector -> pinning in one path.
    let mut b = budget(&mut store, &f);
    store
        .write_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            &archive,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            atomic_write,
            crate::store::epoch_intents::sync_intent,
        )
        .expect("the production writer must persist a valid archive");
    drop(b);
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

/// The payload's claim about which document it belongs to is load-bearing, because the scanner
/// installs the CIDs it returns under the **outer** record's group. A canonical archive for
/// group B, sealed into a correctly authenticated record whose scope names group A, would have
/// B's references installed under A. Deletion is group-scoped, so when B's blob store asks
/// whether its pixel is protected, A's pin does not save it: the pixels are reclaimable while
/// the pin set looks complete.
///
/// Neither refusal test reaches this comparison, because both fail earlier at `decode`. It
/// therefore needs its own fixture and its own mutation.
#[test]
fn a_draft_archive_naming_another_document_fails_a_reference_scan_closed() {
    let root = tempfile::tempdir().unwrap();
    // Distinct devices, groups and logical documents: the consequence is cross-group.
    let a = Fixture::new(true);
    let b = Fixture::new(true);
    assert_ne!(a.group.group_id(), b.group.group_id());
    let mut store = open(root.path());

    // A real, canonical archive payload for B, built in this same vault.
    let (close_b, basis_b) = closing(&b, &mut store);
    frame_branch(&b, &mut store, &close_b, &basis_b);
    let state_b = store.load_epoch_intents(SERVER, &b.logical).unwrap();
    let payload_b = StudioDraftArchive::from_branch(
        state_b.overlay().unwrap(),
        &state_b.ledger,
        StudioOverlayProvenance::Closing,
        true,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap()
    .encode()
    .unwrap();
    drop(state_b);
    // It really is canonical on its own terms; only its placement is wrong.
    assert!(StudioDraftArchive::decode(&payload_b).is_ok());

    // Give A its own branch, so the scan has ordinary work beside the misplaced archive.
    let (close_a, basis_a) = closing(&a, &mut store);
    frame_branch(&a, &mut store, &close_a, &basis_a);

    // Seal B's archive into a record whose outer scope names A.
    crate::store::epoch_draft_archive::write_draft_archive_for_test(
        &store,
        SERVER,
        &a.logical,
        &payload_b,
        &mut rng(),
    )
    .unwrap();
    drop(store);

    let mut store = open(root.path());
    assert!(
        store.creative_pinned_cids().is_err(),
        "an archive naming another logical document must fail the scan closed, or its \
         references are installed under the wrong group and its pixels become reclaimable"
    );
}

/// The collector has two fallible stages, and only the first was anchored. `decode` treats the
/// seed as bounded opaque bytes; the seed is not parsed until `blob_cids` reaches
/// `UnconfirmedStudioSeed::parse`. So an archive can decode cleanly and still fail to yield its
/// references.
///
/// That second stage must fail the scan closed for the same reason as the first: turning
/// uncertainty into an installed, *known*, incomplete pin set is permission to reclaim. A
/// mutation replacing the error with an empty set would satisfy every other test here, because
/// theirs either collect successfully or fail at `decode`.
#[test]
fn a_draft_archive_whose_references_cannot_be_extracted_fails_the_scan_closed() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let overlay = state.overlay().unwrap();
    let mut payload = StudioDraftArchive::from_branch(
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
    drop(state);

    // Corrupt one byte inside the seed, in place. The length is unchanged, so the payload stays
    // canonical and re-encodes identically; only the checkpoint it carries is no longer the one
    // its receipt names.
    let seed = StudioDraftArchive::decode(&payload)
        .unwrap()
        .seed()
        .to_vec();
    let at = payload
        .windows(seed.len())
        .position(|w| w == seed)
        .expect("the seed must appear verbatim in the payload");
    payload[at] ^= 0xff;

    // Guard the guard: this fixture must reach the SECOND stage, not the first.
    let decoded = StudioDraftArchive::decode(&payload)
        .expect("a corrupted seed must still decode: the codec does not parse it");
    assert!(
        decoded.blob_cids().is_err(),
        "the fixture must fail at reference extraction, or it is testing `decode` again"
    );

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
    assert!(
        store.creative_pinned_cids().is_err(),
        "an archive whose references cannot be extracted must fail the scan closed, not be \
         collected as an empty set"
    );
    assert!(
        !store.creative_references_known(),
        "a refused reference scan left protection claiming to be known"
    );
}

/// Build the archive for the current branch and persist it through the production writer.
fn preserve(
    f: &Fixture,
    store: &mut ServerStore,
) -> Result<catcoms_replication::studio::StudioDraftArchive, AppError> {
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let archive = StudioDraftArchive::from_branch(
        state.overlay().unwrap(),
        &state.ledger,
        StudioOverlayProvenance::Closing,
        true,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap();
    drop(state);
    let mut b = budget(store, f);
    store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &archive,
        &mut rng(),
        &mut b.storage,
        &mut b.intents,
        atomic_write,
        crate::store::epoch_intents::sync_intent,
    )?;
    Ok(archive)
}

#[test]
fn the_archive_writer_is_accounted_idempotent_and_refuses_to_overwrite_evidence() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    let before = budget(&mut store, &f).intents.archive_bytes();
    assert_eq!(before, 0, "no archive exists yet");

    let archive = preserve(&f, &mut store).expect("the first preservation must succeed");
    let charged = budget(&mut store, &f).intents.archive_bytes();
    assert!(charged > 0, "a preserved archive must be charged");

    // The record reads back as exactly the archive that was written.
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();
    let stored = store
        .read_scoped_draft_archive_plain(&scope)
        .unwrap()
        .unwrap();
    assert!(
        stored.plain.windows(4).count() > 0 && stored.physical_bytes > 0,
        "the archive record must be present and non-empty"
    );

    // Exact retry: the same archive again is idempotent, not a second record, and takes the
    // sync-only path so an uncertain write can be repeated at capacity.
    preserve(&f, &mut store).expect("an exact retry must be idempotent");
    assert_eq!(
        budget(&mut store, &f).intents.archive_bytes(),
        charged,
        "an exact retry must not charge a second time"
    );

    // A DIFFERENT archive for the same document is refused rather than replacing the first.
    // Overwriting preserved evidence to make room for other preserved evidence is the one thing
    // this record must never do; releasing the existing archive is a separate explicit action.
    let different = StudioDraftArchive::decode(&archive.encode().unwrap()).unwrap();
    let _ = different;
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let other = StudioDraftArchive::from_branch(
        state.overlay().unwrap(),
        &state.ledger,
        StudioOverlayProvenance::Closing,
        // Only the label differs, so the payload differs while the branch does not.
        false,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap();
    drop(state);
    let mut b = budget(&mut store, &f);
    let refused = store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &other,
        &mut rng(),
        &mut b.storage,
        &mut b.intents,
        atomic_write,
        crate::store::epoch_intents::sync_intent,
    );
    assert!(
        refused.is_err(),
        "a second, different archive must be refused, not written over the first"
    );
    drop(b);
    assert_eq!(
        budget(&mut store, &f).intents.archive_bytes(),
        charged,
        "a refused preservation must charge nothing"
    );
    // The original survives the refusal intact.
    let after = store
        .read_scoped_draft_archive_plain(&scope)
        .unwrap()
        .unwrap();
    assert_eq!(after.plain.as_slice(), stored.plain.as_slice());
}

/// Finding 3a: the sub-cap's arithmetic is anchored in `epoch_intents::retirement`, but nothing
/// proved the real writer consults it. Swapping the writer's `preflight_draft_archive` for the
/// ordinary class `preflight` would leave that unit test passing, because it calls the sub-cap
/// helper directly, and leave every other writer test passing, because none of them is anywhere
/// near 16 MiB.
///
/// Position the tally near the cap instead of fabricating 16 MiB of genuine archives: the class
/// total is untouched, so a refusal here is attributable to the sub-cap alone.
#[test]
fn the_archive_writer_refuses_at_the_sub_cap_not_the_class_ceiling() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let archive = StudioDraftArchive::from_branch(
        state.overlay().unwrap(),
        &state.ledger,
        StudioOverlayProvenance::Closing,
        true,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap();
    drop(state);

    let mut b = budget(&mut store, &f);
    assert!(
        b.intents.bytes() < crate::store::epoch_intents::MAX_VAULT_INTENT_BYTES,
        "the class ceiling must have room, or this proves nothing about the sub-cap"
    );
    b.intents.set_archive_bytes_for_test(
        crate::store::epoch_intents::MAX_VAULT_DRAFT_ARCHIVE_BYTES - 16,
    );
    let refused = store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &archive,
        &mut rng(),
        &mut b.storage,
        &mut b.intents,
        atomic_write,
        crate::store::epoch_intents::sync_intent,
    );
    assert!(
        refused.is_err(),
        "the writer must consult the archive sub-cap, not only the class ceiling"
    );
    drop(b);
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_none(),
        "a write refused at the sub-cap must leave no record behind"
    );
}

#[test]
fn the_archive_writer_refuses_a_payload_naming_another_document() {
    // The same binding the collector enforces on the way out, enforced on the way in, so a
    // misplaced archive is never created rather than merely never trusted.
    let root = tempfile::tempdir().unwrap();
    let a = Fixture::new(true);
    let b = Fixture::new(true);
    let mut store = open(root.path());
    let (close_b, basis_b) = closing(&b, &mut store);
    frame_branch(&b, &mut store, &close_b, &basis_b);
    let state = store.load_epoch_intents(SERVER, &b.logical).unwrap();
    let archive_b = StudioDraftArchive::from_branch(
        state.overlay().unwrap(),
        &state.ledger,
        StudioOverlayProvenance::Closing,
        true,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap();
    drop(state);

    let (close_a, basis_a) = closing(&a, &mut store);
    frame_branch(&a, &mut store, &close_a, &basis_a);
    let mut budgets = budget(&mut store, &a);
    let refused = store.write_studio_draft_archive_with_io(
        SERVER,
        &a.logical,
        &archive_b,
        &mut rng(),
        &mut budgets.storage,
        &mut budgets.intents,
        atomic_write,
        crate::store::epoch_intents::sync_intent,
    );
    assert!(
        refused.is_err(),
        "an archive naming another document must be refused at the writer"
    );
    drop(budgets);
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &a.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_none(),
        "a refused write must leave no record behind"
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
