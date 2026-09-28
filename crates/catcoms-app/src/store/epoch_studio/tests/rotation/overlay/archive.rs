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
            &mut WriteHooks::None,
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

/// The archive for the current branch. Deterministic in the branch state and these constants,
/// so building it twice yields the same plaintext record and the second write is an exact retry.
fn archive_for(
    f: &Fixture,
    store: &mut ServerStore,
) -> catcoms_replication::studio::StudioDraftArchive {
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
    archive
}

/// Build the archive for the current branch and persist it through the production writer.
fn preserve(
    f: &Fixture,
    store: &mut ServerStore,
) -> Result<catcoms_replication::studio::StudioDraftArchive, AppError> {
    let archive = archive_for(f, store);
    let mut b = budget(store, f);
    store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &archive,
        &mut rng(),
        &mut b.storage,
        &mut b.intents,
        &mut WriteHooks::None,
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
        &mut WriteHooks::None,
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
        &mut WriteHooks::None,
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
        &mut WriteHooks::None,
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

/// Release one archive through the production path, with a freshly reconciled budget.
fn release(f: &Fixture, store: &mut ServerStore, expected: [u8; 32]) -> Result<(), AppError> {
    let mut b = budget(store, f);
    store.release_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        expected,
        &mut b.storage,
        &mut b.intents,
        &mut WriteHooks::None,
    )
}

/// The mirror of N19, and the test that makes release *mean* something.
///
/// N19 proves the archive is the sole thing keeping a disposed branch's pixels alive. On its own
/// that is only half a guarantee: a release that unlinked nothing, or that left a record the
/// scanner still read, would pass every other test in this module, because they all assert the
/// archive is present or is refused. Here the same vault is driven all the way round: the pixels
/// are pinned *because* of the archive, and reclaimable again *because* it was released, with
/// nothing else in the vault changing between the two observations.
///
/// This is also the only test that proves release reaches the scanner at all rather than merely
/// returning `Ok`.
#[test]
fn releasing_an_archive_makes_its_sole_references_reclaimable() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let operation_cids = frame_branch(&f, &mut store, &close, &basis);

    let archive = archive_for(&f, &mut store);
    let content = archive.archive_id().unwrap();
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let base_cids = state.overlay().unwrap().base_blob_cids().unwrap();
    drop(state);

    // Strip every other namer, exactly as N19 does, so the archive is the sole holder.
    let intent_scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    fs::remove_file(store.epoch_intent_path(&intent_scope)).unwrap();
    fs::remove_file(f.path(&store)).unwrap();

    let mut b = budget(&mut store, &f);
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
    drop(b);
    drop(store);

    let pinned = |pins: &crate::store::CreativeReferences, cid: &[u8; 32]| {
        pins.for_group(&f.group.group_id())
            .any(|held| *held == catcoms_storage::Cid::from_bytes(*cid))
    };

    // Held, on the strength of the archive alone.
    let mut store = open(root.path());
    let pins = store.creative_pinned_cids().unwrap();
    for cid in operation_cids.iter().chain(base_cids.iter()) {
        assert!(
            pinned(&pins, cid),
            "precondition broken: {cid:?} must be pinned by the archive before release"
        );
    }
    drop(pins);

    release(&f, &mut store, content).expect("release must succeed");

    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_none(),
        "release must remove the record"
    );

    drop(store);
    let mut store = open(root.path());
    let pins = store.creative_pinned_cids().unwrap();
    for cid in operation_cids.iter().chain(base_cids.iter()) {
        assert!(
            !pinned(&pins, cid),
            "{cid:?} is still pinned after its only holder was released, so the scan is reading \
             a record release was supposed to have destroyed"
        );
    }
}

/// The binding must name the **archive**, not its branch.
///
/// This is the review finding that `content()` could not carry. `content` is the branch's content
/// identity, so two archives of one branch that differ only in a label share it, and `replayable`
/// is exactly such a label. Release-then-write is the only way to replace an archive, which makes
/// the dangerous sequence ordinary rather than exotic: read A, queue the confirmation, A gets
/// released by some other valid action, B is archived for the same branch, and the queued request
/// arrives. Under a `content` binding it destroys B, which the user never saw.
///
/// The fixture builds precisely that pair and asserts the stale token is refused, so it fails if
/// the binding ever reverts to branch content. The guard-the-guard assertions matter here: if the
/// two archives did not actually share `content`, or did not actually differ, the refusal below
/// would prove nothing about identity.
#[test]
fn a_stale_release_token_cannot_destroy_a_later_archive_of_the_same_branch() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    // A, and the token a UI read would have handed the user.
    let archive_a = preserve(&f, &mut store).expect("preserve A");
    let token_a = archive_a.archive_id().unwrap();

    // A is released by some other valid action.
    release(&f, &mut store, token_a).expect("A releases");

    // B: the same branch, same content identity, differing only in the `replayable` label.
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let archive_b = StudioDraftArchive::from_branch(
        state.overlay().unwrap(),
        &state.ledger,
        StudioOverlayProvenance::Closing,
        false,
        [3; 32],
        [4; 32],
        1,
    )
    .unwrap();
    drop(state);

    // Guard the guard, both halves.
    assert_eq!(
        archive_a.content(),
        archive_b.content(),
        "the fixture must produce two archives sharing branch content, or it cannot show that \
         binding to content is unsafe"
    );
    assert_ne!(
        token_a,
        archive_b.archive_id().unwrap(),
        "the two archives must have distinct archive identities, or there is nothing to refuse"
    );

    let mut b = budget(&mut store, &f);
    store
        .write_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            &archive_b,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            &mut WriteHooks::None,
        )
        .expect("B must persist");
    drop(b);

    // The queued confirmation for A arrives.
    let refused = release(&f, &mut store, token_a);
    assert!(
        refused.is_err(),
        "a token read from A must not destroy B: the user confirmed A"
    );
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_some(),
        "B must survive a stale token for A"
    );

    // B's own token still releases it, so the refusal was about identity rather than a blanket
    // failure that would make this test pass for the wrong reason.
    release(&f, &mut store, archive_b.archive_id().unwrap()).expect("B's own token must release B");
    assert!(store
        .read_scoped_draft_archive_plain(&scope)
        .unwrap()
        .is_none());
}

/// The binding is enforced at all. A confirmation literal proves the user typed something; it
/// cannot prove they typed it about the archive that is on disk now.
#[test]
fn release_refuses_an_archive_other_than_the_one_it_names() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");

    let mut wrong = archive.archive_id().unwrap();
    wrong[0] ^= 0xff;
    let refused = release(&f, &mut store, wrong);
    assert!(
        refused.is_err(),
        "release must refuse a content it was not asked to destroy"
    );

    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_some(),
        "a refused release must leave the archive intact"
    );
    // And the correct content still releases it, so the refusal was about identity and not a
    // blanket failure that would make this test pass for the wrong reason.
    release(&f, &mut store, archive.archive_id().unwrap()).expect("the named archive must release");
    assert!(store
        .read_scoped_draft_archive_plain(&scope)
        .unwrap()
        .is_none());
}

#[test]
fn release_refuses_when_no_archive_is_preserved() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    // Reporting success here would report the same thing whether the archive was already gone or
    // the scope was computed wrongly, and the second destroys the wrong evidence elsewhere.
    assert!(
        release(&f, &mut store, [0; 32]).is_err(),
        "release must not report success when it found nothing"
    );
}

/// Fail-closed on the way out as well as on the way in. An archive this build cannot parse is
/// refused rather than unlinked on the strength of its filename: the destructive path is the last
/// place to start trusting a record the reading path refuses.
#[test]
fn release_refuses_an_archive_it_cannot_decode() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    crate::store::epoch_draft_archive::write_draft_archive_for_test(
        &store,
        SERVER,
        &f.logical,
        b"not a draft archive payload",
        &mut rng(),
    )
    .unwrap();

    assert!(
        release(&f, &mut store, [0; 32]).is_err(),
        "release must refuse an undecodable archive rather than unlink it"
    );
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_some(),
        "a refused release must leave even an unreadable record in place for diagnosis"
    );
}

/// The payload's document claim, checked on the destructive path too.
///
/// This test exists because the mutation pass found the guard unanchored: deleting
/// `archive.document() != document` from release failed nothing, since the content comparison
/// happens to catch the cases the other tests build. It is reachable on its own, and the
/// consequence is specific: an archive for B sealed into A's record would be released by a user
/// who confirmed a release for A, destroying B's only preserved evidence while the dialog, the
/// scope and the content all agreed.
#[test]
fn release_refuses_an_archive_naming_another_document() {
    let root = tempfile::tempdir().unwrap();
    let a = Fixture::new(true);
    let b = Fixture::new(true);
    assert_ne!(a.group.group_id(), b.group.group_id());
    let mut store = open(root.path());

    // A genuine archive for B, built in this vault.
    let (close_b, basis_b) = closing(&b, &mut store);
    frame_branch(&b, &mut store, &close_b, &basis_b);
    let archive_b = archive_for(&b, &mut store);
    let content_b = archive_b.archive_id().unwrap();

    // Give A a branch of its own, then seal B's archive into A's record.
    let (close_a, basis_a) = closing(&a, &mut store);
    frame_branch(&a, &mut store, &close_a, &basis_a);
    crate::store::epoch_draft_archive::write_draft_archive_for_test(
        &store,
        SERVER,
        &a.logical,
        &archive_b.encode().unwrap(),
        &mut rng(),
    )
    .unwrap();

    // Release A, naming the content that really is in A's record. Only the document claim is
    // wrong, so nothing else in the function can catch this.
    let refused = release(&a, &mut store, content_b);
    assert!(
        refused.is_err(),
        "release must refuse an archive whose payload names another document"
    );
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &a.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_some(),
        "a refused release must leave the misplaced archive in place: it is still B's evidence"
    );
}

/// `verify_record` before the unlink. A budget that does not know the record must not be spent
/// destroying it: the mismatch means this process's accounting and the disk disagree, and the
/// safe response is to invalidate rather than to delete and hope.
#[test]
fn release_refuses_a_record_its_budget_does_not_know() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);

    // Build the budget FIRST, so it is reconciled against a vault with no archive.
    let mut b = budget(&mut store, &f);

    // Then place a genuine, decodable archive behind its back, bypassing accounting.
    let archive = archive_for(&f, &mut store);
    let content = archive.archive_id().unwrap();
    crate::store::epoch_draft_archive::write_draft_archive_for_test(
        &store,
        SERVER,
        &f.logical,
        &archive.encode().unwrap(),
        &mut rng(),
    )
    .unwrap();

    let refused = store.release_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        content,
        &mut b.storage,
        &mut b.intents,
        &mut WriteHooks::None,
    );
    assert!(
        refused.is_err(),
        "release must refuse when its budget never accounted the record it is destroying"
    );
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_some(),
        "a release refused on accounting must not have unlinked anything"
    );
}

/// Release is the one operation in this family that cannot be expressed as a replacement, so it
/// ends with both budgets closed. Proving that matters because the alternative, subtracting the
/// freed bytes by hand, is a second representation of occupancy maintained beside the inventory's
/// and drifting the first time a subtraction is wrong.
#[test]
fn release_closes_both_budgets_so_the_next_write_must_reconcile() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");

    let mut b = budget(&mut store, &f);
    store
        .release_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            archive.archive_id().unwrap(),
            &mut b.storage,
            &mut b.intents,
            &mut WriteHooks::None,
        )
        .expect("release must succeed");

    assert!(
        b.storage.requires_reconciliation(),
        "the storage budget still believes it accounts a record that no longer exists"
    );
    // The intent budget was poisoned before the unlink and never restored, so it cannot authorise
    // a further write either.
    let again = store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &archive,
        &mut rng(),
        &mut b.storage,
        &mut b.intents,
        &mut WriteHooks::None,
    );
    assert!(
        again.is_err(),
        "a budget carried across a release must not authorise the next archive write"
    );
    drop(b);

    // A reconciled budget writes again normally: the closure is a fail-closed step, not damage.
    preserve(&f, &mut store).expect("a rebuilt budget must be able to preserve again");
}

/// The intent budget release was given is unusable afterwards, with the storage budget rebuilt so
/// it cannot be the thing that refuses.
///
/// **This does not attribute the refusal to a particular rail, and an earlier version of this
/// comment claimed it did.** Release closes the intent side three ways - `intents.begin_write()`,
/// the `intent_generation` rotation, and the record map that no longer matches disk - and
/// mutating away *both* of the first two leaves this test passing, because the map check refuses
/// first. The rails are not dead: they cover a write to a **different** document in the same
/// group, whose map entry the release did not disturb. That case needs two documents under one
/// group, which this fixture cannot build cheaply, and is recorded as test debt for the disposal
/// slice where such a fixture exists anyway.
///
/// What this test does prove, and what it is worth keeping for, is the end-to-end property: a
/// budget carried across a release cannot authorise the next write.
#[test]
fn release_leaves_the_intent_budget_it_was_given_unusable() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");

    let mut b = budget(&mut store, &f);
    store
        .release_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            archive.archive_id().unwrap(),
            &mut b.storage,
            &mut b.intents,
            &mut WriteHooks::None,
        )
        .expect("release must succeed");
    let stale_intents = b.intents;
    drop(b.storage);

    // A storage budget that genuinely describes the post-release disk, so it cannot be the thing
    // that refuses below.
    let mut fresh = budget(&mut store, &f);
    assert!(
        !fresh.storage.requires_reconciliation(),
        "the rebuilt storage budget must be usable, or this test proves nothing about intents"
    );

    let mut intents = stale_intents;
    let refused = store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &archive,
        &mut rng(),
        &mut fresh.storage,
        &mut intents,
        &mut WriteHooks::None,
    );
    assert!(
        refused.is_err(),
        "the intent budget release was given must be unusable on its own terms"
    );
}

/// A budget release never touched is also unusable afterwards.
///
/// **Like the test above, this does not isolate the rotation**, and saying otherwise was wrong.
/// The bystander budget was built while the archive still existed, so its record map disagrees
/// with disk after the release and refuses before the generation is ever compared: deleting
/// `self.intent_generation = Arc::new(())` leaves this test green. Isolating the rotation needs a
/// probe whose map entry the release did not change, which means a second document in the same
/// group. Recorded as test debt for the disposal slice.
///
/// The property it does establish is still worth having and is not covered by the test above: the
/// damage is not confined to the budget release was handed. Any budget captured before it is
/// refused too.
#[test]
fn a_budget_release_never_touched_is_also_refused_afterwards() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");

    // Two budgets from the same pre-release inventory. `b` goes into the release; `bystander`
    // never touches it.
    let mut b = budget(&mut store, &f);
    let bystander = budget(&mut store, &f);
    assert!(
        !bystander.intents.requires_reconciliation_for_test(),
        "the bystander must start usable, or the refusal below is not attributable to rotation"
    );

    store
        .release_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            archive.archive_id().unwrap(),
            &mut b.storage,
            &mut b.intents,
            &mut WriteHooks::None,
        )
        .expect("release must succeed");
    drop(b);

    // Fresh storage, stale-but-never-poisoned intents: only the generation can refuse.
    let mut fresh = budget(&mut store, &f);
    let mut intents = bystander.intents;
    let refused = store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &archive,
        &mut rng(),
        &mut fresh.storage,
        &mut intents,
        &mut WriteHooks::None,
    );
    assert!(
        refused.is_err(),
        "a budget captured before the release must be refused by the rotated generation"
    );
}

/// The `intent_generation` rotation, isolated at last.
///
/// The two tests above cannot do this, and the previous review round was right that I gave up on
/// it too early. Their probes write a record whose map entry the release changed, so
/// `preflight`'s `records.get(&id) != old` refuses before the generation is ever compared, and
/// deleting the rotation leaves them green.
///
/// The fix is a **different document in the same group**. A's release does not touch B's intent
/// record, so B's map entry still matches disk and the record check passes. The rotation is then
/// the only fence left, which is exactly the production-reachable hazard: an `EpochStudioBudget`
/// is per `(server, group)`, a group holds many documents, and a budget captured before a release
/// on A would otherwise go on spending stale vault accounting on B.
///
/// The control at the end is what makes the refusal attributable. Without it, a refusal for an
/// unrelated reason - a malformed operation, a document the group will not accept - would satisfy
/// the assertion and prove nothing.
#[test]
fn a_stale_intent_budget_cannot_write_another_document_after_a_release() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");

    // Second document, same group, untouched by the release below.
    let other_target = StudioTarget::Flipnote {
        channel: [7; 16],
        object: [10; 16],
    };
    let other = other_target.document(&f.group.group_id()).unwrap();
    assert_ne!(
        other.logical_key, f.logical.logical_key,
        "the probe must address a different document, or its record entry moves with the release"
    );
    let operation = |n: u8| DomainOp {
        nonce: [n; 16],
        doc_type: other.doc_type,
        logical_key: other.logical_key.clone(),
        body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("bystander".into()))
            .encode()
            .unwrap(),
    };

    // Captured before the release, and never handed to it.
    let bystander = budget(&mut store, &f);

    let mut b = budget(&mut store, &f);
    store
        .release_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            archive.archive_id().unwrap(),
            &mut b.storage,
            &mut b.intents,
            &mut WriteHooks::None,
        )
        .expect("release must succeed");
    drop(b);

    // Fresh storage so it cannot be the refusing party; stale intents whose record map for B is
    // still correct, so the map cannot be the refusing party either.
    let mut fresh = budget(&mut store, &f);
    let mut stale = bystander.intents;
    let refused = store.prepare_epoch_intent(
        SERVER,
        &other,
        operation(0x41),
        &f.device,
        &f.group,
        &mut rng(),
        &mut fresh.storage,
        &mut stale,
    );
    assert!(
        refused.is_err(),
        "an intent budget captured before a release went on to authorise a write to another \
         document in the same group, spending vault accounting that the release invalidated"
    );

    // Control: the same write with budgets rebuilt after the release must succeed, so the refusal
    // above is attributable to the stale budget and not to the document, the operation or the
    // group.
    let mut good = budget(&mut store, &f);
    store
        .prepare_epoch_intent(
            SERVER,
            &other,
            operation(0x42),
            &f.device,
            &f.group,
            &mut rng(),
            &mut good.storage,
            &mut good.intents,
        )
        .expect("a reconciled budget must be able to write this very document");
}

/// Finding 2: the closure must hold across the failure that actually returns early.
///
/// A successful `remove_file` followed by a failed parent sync returns
/// `CommittedButNotDurable` before the end of the happy path. If the storage budget were closed
/// at the end instead of before the destructive phase, the caller would keep a budget claiming a
/// record this process can no longer see. The injected failure after the unlink reaches the same
/// early-return shape, and is the one a hook can produce.
#[test]
fn a_release_that_fails_after_the_unlink_still_closes_both_budgets() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");

    let mut b = budget(&mut store, &f);
    assert!(
        !b.storage.requires_reconciliation(),
        "the budget must start usable"
    );
    {
        let mut after = |_op: CompletedOperation, _tag: WriteTag, _p: &std::path::Path| {
            AfterIntercept::Fail(AppError::Io("injected failure after the unlink".into()))
        };
        let mut hooks = WriteHooks::Hooked {
            before: None,
            before_sync: None,
            before_unlink: None,
            after: Some(&mut after),
        };
        let failed = store.release_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            archive.archive_id().unwrap(),
            &mut b.storage,
            &mut b.intents,
            &mut hooks,
        );
        assert!(failed.is_err(), "the injected failure must fail the call");
    }

    // The archive really is gone, so the budget really is stale.
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_none(),
        "the unlink completed, so this is the stale-accounting case and not a no-op"
    );
    assert!(
        b.storage.requires_reconciliation(),
        "a release that failed after the unlink left a usable storage budget describing a record \
         that no longer exists"
    );
    let refused = store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &archive,
        &mut rng(),
        &mut b.storage,
        &mut b.intents,
        &mut WriteHooks::None,
    );
    assert!(
        refused.is_err(),
        "neither budget may authorise a write after a release failed past the unlink"
    );
}

/// I-4 for the destructive path: release must rotate `inventory_generation`.
///
/// This is what stops a reference scan that observed the pre-release vault from later installing
/// a protection set built when the archive still existed. The rotation happens inside
/// `epoch_mutation_guard()`, and the Slice 3 mutation set did not anchor that call: every other
/// release test would pass with the guard replaced by a direct syscall.
///
/// Stated at the generation rather than by parking a real cursor across the release. A cursor
/// fixture would exercise more of the path, but `EpochStorageCursor` is Agent 1's actively
/// changing C-3 surface, and a test of Agent 2's guard that breaks whenever that API moves is a
/// test of the wrong thing. The generation is the contract both sides agree on: cursors refuse by
/// comparing against exactly this value, so a release that rotates it cannot be overtaken, and a
/// release that does not rotate it fails here.
#[test]
fn release_rotates_the_inventory_generation_so_a_scan_cannot_overtake_it() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");

    let before = store.inventory_generation();
    release(&f, &mut store, archive.archive_id().unwrap()).expect("release must succeed");
    let after = store.inventory_generation();

    assert!(
        !std::sync::Arc::ptr_eq(&before, &after),
        "release removed an inventoried record without rotating the inventory generation, so a \
         scan that captured the pre-release token could still install a protection set naming \
         the archive's CIDs"
    );
}

/// The remediation guarantee the sub-cap's grandfathering exists to provide.
///
/// `from_inventory` deliberately does not refuse an over-cap vault, because refusing would be
/// self-locking: every accounted write needs a budget, so an over-cap vault would lose unrelated
/// intent writes **and** lose the release that is its only way back under the policy. That
/// argument is only sound if release actually works on such a vault. Until now it was inferred
/// from two separately tested pieces; this executes it.
///
/// The tally is positioned with the test-only setter rather than by fabricating 16 MiB of real
/// archives, so the class ceiling is untouched and a refusal would be attributable to the sub-cap
/// alone. A mutation adding an archive-cap check to release must fail this test.
#[test]
fn an_over_cap_vault_can_still_release_its_way_back_under_the_policy() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");

    let mut b = budget(&mut store, &f);
    b.intents
        .set_archive_bytes_for_test(crate::store::epoch_intents::MAX_VAULT_DRAFT_ARCHIVE_BYTES + 1);
    // Guard the guard, stated on the tally rather than by attempting a write. A write probe is
    // the wrong instrument here: re-writing the same archive is an exact retry, which takes the
    // sync-only path and deliberately skips the sub-cap, so it would succeed and the assertion
    // would report the opposite of the truth. That growth really is refused above the cap is
    // already proved by `the_archive_writer_refuses_at_the_sub_cap_not_the_class_ceiling`.
    assert!(
        b.intents.archive_bytes() > crate::store::epoch_intents::MAX_VAULT_DRAFT_ARCHIVE_BYTES,
        "the fixture must actually be over the sub-cap, or the release below proves nothing"
    );
    store
        .release_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            archive.archive_id().unwrap(),
            &mut b.storage,
            &mut b.intents,
            &mut WriteHooks::None,
        )
        .expect("an over-cap vault must still be able to release: it is the only way back");
    drop(b);

    // Rebuilt from disk, the vault is back under the policy and can write again.
    let rebuilt = budget(&mut store, &f);
    assert_eq!(
        rebuilt.intents.archive_bytes(),
        0,
        "the released archive must be gone from the observed tally"
    );
    drop(rebuilt);
    preserve(&f, &mut store).expect("a vault back under the cap must be able to preserve again");
}

/// Requirement 3 for the destructive path: the store owns the unlink, and a hook decides only on
/// either side of it.
///
/// The asymmetry is the point. Refusing before leaves the archive whole. Refusing after cannot
/// put it back, and the test says so explicitly rather than leaving a reader to assume a failed
/// call means an unchanged vault: for a destructive operation that assumption is exactly wrong,
/// and a caller that retried on error expecting idempotence would be surprised by the refusal
/// from the now-absent record instead.
#[test]
fn release_consults_hooks_on_both_sides_of_its_unlink() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let archive = preserve(&f, &mut store).expect("preserve");
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();

    // Refusing before the unlink leaves the record exactly where it was.
    let mut tags = Vec::new();
    {
        let mut before_unlink = |tag: WriteTag, _p: &std::path::Path| {
            tags.push(tag);
            AfterIntercept::Fail(AppError::Io("injected refusal before the unlink".into()))
        };
        let mut hooks = WriteHooks::Hooked {
            before: None,
            before_sync: None,
            before_unlink: Some(&mut before_unlink),
            after: None,
        };
        let mut b = budget(&mut store, &f);
        let refused = store.release_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            archive.archive_id().unwrap(),
            &mut b.storage,
            &mut b.intents,
            &mut hooks,
        );
        assert!(refused.is_err(), "a refusal before the unlink must fail");
    }
    assert_eq!(
        tags,
        vec![WriteTag::Archive],
        "the release must carry the archive tag"
    );
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_some(),
        "a release refused before the unlink must leave the archive intact"
    );

    // Refusing after it still fails the call, and the archive is gone: the operation completed.
    {
        let mut after = |op: CompletedOperation, tag: WriteTag, _p: &std::path::Path| {
            assert_eq!(tag, WriteTag::Archive);
            assert_eq!(op, CompletedOperation::Unlink, "release removes");
            AfterIntercept::Fail(AppError::Io("injected failure after the unlink".into()))
        };
        let mut hooks = WriteHooks::Hooked {
            before: None,
            before_sync: None,
            before_unlink: None,
            after: Some(&mut after),
        };
        let mut b = budget(&mut store, &f);
        let refused = store.release_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            archive.archive_id().unwrap(),
            &mut b.storage,
            &mut b.intents,
            &mut hooks,
        );
        assert!(refused.is_err(), "a failure after the unlink must fail");
    }
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_none(),
        "the unlink had already happened: a failure after it cannot restore the archive"
    );
}

/// Requirement 3 for this path: the caller decides *around* the physical operation, it does not
/// supply one. Both the replacement and the exact-retry sync are the store's own, and a hook can
/// only refuse on either side of them.
///
/// The ordering this pins down is the one a caller-supplied writer could not be trusted to keep:
/// the after decision runs once the bytes are already in place, so a failure there is a durable
/// record that was never charged, repairable by exact retry rather than by overwriting evidence.
#[test]
fn the_archive_writer_consults_hooks_on_both_sides_of_its_own_operations() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    let scope = crate::store::epoch_draft_archive::scope_bytes(SERVER, &f.logical).unwrap();

    // Refusing before the replacement leaves no record and charges nothing.
    let archive = archive_for(&f, &mut store);
    let mut tags = Vec::new();
    {
        let mut before = |tag: WriteTag, _p: &std::path::Path, bytes: &[u8]| {
            tags.push(tag);
            assert!(
                !bytes.is_empty(),
                "the hook must see the record being written"
            );
            Intercept::Fail(AppError::Io(
                "injected refusal before the archive write".into(),
            ))
        };
        let mut hooks = WriteHooks::Hooked {
            before: Some(&mut before),
            before_sync: None,
            before_unlink: None,
            after: None,
        };
        let mut b = budget(&mut store, &f);
        let refused = store.write_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            &archive,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            &mut hooks,
        );
        assert!(
            refused.is_err(),
            "a refusal before the write must fail the call"
        );
    }
    assert_eq!(
        tags,
        vec![WriteTag::Archive],
        "the archive write must carry its own tag"
    );
    assert!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .is_none(),
        "a write refused before the replacement must leave no record behind"
    );

    // Refusing after it leaves the record durable but uncharged, and poisons the budget that
    // was mid-write: an uncertain write must not leave either accounting usable.
    let mut b = budget(&mut store, &f);
    {
        let mut after = |op: CompletedOperation, tag: WriteTag, _p: &std::path::Path| {
            assert_eq!(tag, WriteTag::Archive);
            assert_eq!(
                op,
                CompletedOperation::Write,
                "the first preservation replaces"
            );
            AfterIntercept::Fail(AppError::Io(
                "injected failure after the archive write".into(),
            ))
        };
        let mut hooks = WriteHooks::Hooked {
            before: None,
            before_sync: None,
            before_unlink: None,
            after: Some(&mut after),
        };
        let refused = store.write_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            &archive,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            &mut hooks,
        );
        assert!(
            refused.is_err(),
            "a failure after the write must fail the call"
        );
    }
    // The same budget was poisoned before the first possible I/O and never restored.
    let poisoned = store.write_studio_draft_archive_with_io(
        SERVER,
        &f.logical,
        &archive,
        &mut rng(),
        &mut b.storage,
        &mut b.intents,
        &mut WriteHooks::None,
    );
    assert!(
        poisoned.is_err(),
        "an uncertain write must leave the intent budget unusable until it is rebuilt"
    );
    drop(b);
    let stored = store
        .read_scoped_draft_archive_plain(&scope)
        .unwrap()
        .expect("the bytes were in place before the after decision ran");
    let physical = stored.physical_bytes;

    // The exact retry takes the sync branch, and that operation is hooked too: the size it is
    // asked to flush is the record's, and refusing leaves the record itself untouched.
    let mut saw_len = None;
    {
        let mut before_sync = |tag: WriteTag, _p: &std::path::Path, len: u64| {
            assert_eq!(tag, WriteTag::Archive);
            saw_len = Some(len);
            AfterIntercept::Fail(AppError::Io(
                "injected refusal before the archive sync".into(),
            ))
        };
        let mut hooks = WriteHooks::Hooked {
            before: None,
            before_sync: Some(&mut before_sync),
            before_unlink: None,
            after: None,
        };
        let mut b = budget(&mut store, &f);
        let refused = store.write_studio_draft_archive_with_io(
            SERVER,
            &f.logical,
            &archive,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            &mut hooks,
        );
        assert!(
            refused.is_err(),
            "a refusal before the sync must fail the call"
        );
    }
    assert_eq!(
        saw_len,
        Some(physical),
        "the sync decision must be given the record's own physical size"
    );
    assert_eq!(
        store
            .read_scoped_draft_archive_plain(&scope)
            .unwrap()
            .expect("the record survives a refused sync")
            .physical_bytes,
        physical,
        "a refused sync must not disturb the record it was going to flush"
    );

    // And the whole sequence is repairable: an exact retry through the unhooked writer succeeds.
    preserve(&f, &mut store).expect("an exact retry must repair an uncharged durable archive");
}
