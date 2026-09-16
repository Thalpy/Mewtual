//! Draft archive codec, against real branches built through the production basis path. A
//! synthetic basis would need a test-only constructor on `StudioClosingOverlayBasis`, which is
//! exactly the back door the overlay design forbids, so these reuse the settlement fixture.
use super::*;
use crate::studio::{StudioDraftArchive, StudioOverlayProvenance, MAX_STUDIO_DRAFT_ARCHIVE_BYTES};
use crate::IntentLedger;

/// A real accepted branch: real Closing basis, real ledger, real typed acceptance. Returns the
/// branch, its ledger, and the authored (body, timestamp) pairs in acceptance order.
fn branch(
    f: &mut Fixture,
    count: usize,
) -> (StudioOverlayState, IntentLedger, Vec<(DomainOp, u64)>) {
    f.fill();
    let decision = f.decide(None);
    // Seals the source: a Closing basis does not exist before its epoch closes.
    let _plan = f.plan(&decision);
    let basis = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());
    let mut metadata = StudioOverlayState::new(&basis);
    let mut ordered = Vec::new();
    for n in 0..count {
        let op = f.domain(f.title_body(&format!("archived title {n}")));
        let ts = 4242 + n as u64;
        let id = ledger.prepare(f.owner.device_id(), op.clone()).unwrap();
        metadata.append(&basis, &ledger, id, ts).unwrap();
        ordered.push((op, ts));
    }
    (metadata, ledger, ordered)
}

fn archive(
    metadata: &StudioOverlayState,
    ledger: &IntentLedger,
    replayable: bool,
) -> StudioDraftArchive {
    StudioDraftArchive::from_branch(
        metadata.overlay().unwrap(),
        ledger,
        StudioOverlayProvenance::Closing,
        replayable,
        [3; 32],
        [4; 32],
        7,
    )
    .unwrap()
}

#[test]
fn draft_archive_round_trips_every_field_of_a_real_branch() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, ordered) = branch(&mut f, 4);
        let overlay = metadata.overlay().unwrap();
        let built = archive(&metadata, &ledger, true);
        let bytes = built.encode().unwrap();
        let read = StudioDraftArchive::decode(&bytes).unwrap();

        // Identity and provenance survive.
        assert_eq!(read.provenance(), StudioOverlayProvenance::Closing);
        assert!(read.replayable());
        assert_eq!(read.document(), ledger.document());
        assert_eq!(read.target(), overlay.target());
        assert_eq!(read.author(), overlay.author());
        assert_eq!(read.basis(), overlay.basis());
        assert_eq!((read.branch(), read.content()), ([3; 32], [4; 32]));
        assert_eq!(read.generation(), 7);
        assert_eq!(read.accepted(), ordered.len());
        // Canonical: re-encoding the decoded value reproduces the exact bytes.
        assert_eq!(read.encode().unwrap(), bytes);
    }
}

#[test]
fn draft_archive_preserves_authored_order_against_opposite_hash_order() {
    // The manifest's sequence is the authored order. Operation ids are hashes and sort
    // independently of it, so a codec that reordered by id would still look self-consistent.
    let mut f = Fixture::new(false);
    let (metadata, ledger, ordered) = branch(&mut f, 8);
    let ids: Vec<[u8; 32]> = ordered
        .iter()
        .map(|(op, _)| op.id(&f.owner.device_id()))
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_ne!(ids, sorted, "fixture must not author in hash order");

    let bytes = archive(&metadata, &ledger, true).encode().unwrap();
    let read = StudioDraftArchive::decode(&bytes).unwrap();
    // Decoding validates sequence == index + 1 and id == operation.id(author) for every entry,
    // so a reordered or renumbered manifest cannot decode at all.
    assert_eq!(read.accepted(), ids.len());
    assert_eq!(read.encode().unwrap(), bytes);
}

#[test]
fn draft_archive_base_references_match_the_live_branch() {
    // Partial coverage, deliberately labelled. This fixture's operations are header edits,
    // which carry no PIX reference at all, so it can only establish the BASE half of
    // `blob_cids`. A version of this test that also compared the operation half would pass
    // with operation collection deleted, because both sides would be the base set: that exact
    // mutation was run and did not fail, which is why this assertion is scoped rather than
    // overstated. The operation half is asserted in the app crate against the Flipnote
    // fixture that publishes real PIX blobs, where the collector plugs into the inventory arm
    // and where M28 can actually observe a missing CID.
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 3);
        let overlay = metadata.overlay().unwrap();
        let base = overlay.base_blob_cids().unwrap();
        for (_, intent) in ledger.pending() {
            assert!(
                crate::studio::references::operation_blob_cid(&intent.operation)
                    .unwrap()
                    .is_none(),
                "header edits carry no CID; if this fires the scoping note above is stale"
            );
        }
        let read = StudioDraftArchive::decode(&archive(&metadata, &ledger, true).encode().unwrap())
            .unwrap();
        assert_eq!(read.blob_cids().unwrap(), base);
    }
}

#[test]
fn draft_archive_needs_no_typed_reconstruction() {
    // A branch that cannot be replayed must still archive, or a preserving disposal would be
    // unavailable to exactly the work that most needs preserving.
    let mut f = Fixture::new(false);
    let (metadata, ledger, _) = branch(&mut f, 2);
    let honest = archive(&metadata, &ledger, true).encode().unwrap();
    let labelled = archive(&metadata, &ledger, false).encode().unwrap();
    assert_ne!(honest, labelled, "the label must be in the bytes");
    let read = StudioDraftArchive::decode(&labelled).unwrap();
    assert!(!read.replayable());
    // The label changes nothing else: same content, same references.
    assert_eq!(read.accepted(), 2);
    assert_eq!(
        read.blob_cids().unwrap(),
        StudioDraftArchive::decode(&honest)
            .unwrap()
            .blob_cids()
            .unwrap()
    );
}

#[test]
fn draft_archive_rejects_an_entry_id_that_does_not_bind_its_body() {
    // An entry's id binds its author and nonce. If the codec accepted an id that disagreed
    // with the body beside it, an archive could name accepted work it does not actually
    // contain, and a preserving disposal would destroy the real operation while claiming to
    // have kept it. Swap two ids so each entry names the other's operation: every other field
    // stays canonical, the count and sequences are untouched, and the manifest still looks
    // internally ordered, so only the id-to-body check can reject this.
    let mut f = Fixture::new(false);
    let (metadata, ledger, ordered) = branch(&mut f, 2);
    let bytes = archive(&metadata, &ledger, true).encode().unwrap();
    assert!(StudioDraftArchive::decode(&bytes).is_ok());

    let first = ordered[0].0.id(&f.owner.device_id());
    let second = ordered[1].0.id(&f.owner.device_id());
    let at = |needle: &[u8; 32]| {
        bytes
            .windows(32)
            .position(|w| w == needle)
            .unwrap_or_else(|| panic!("entry id must appear verbatim in the archive"))
    };
    let (a, b) = (at(&first), at(&second));
    let mut swapped = bytes.clone();
    swapped[a..a + 32].copy_from_slice(&second);
    swapped[b..b + 32].copy_from_slice(&first);
    assert_ne!(swapped, bytes, "the fixture must actually alter the bytes");

    assert!(
        StudioDraftArchive::decode(&swapped).is_err(),
        "an entry id that does not derive from its own author and body must reject"
    );
}

#[test]
fn draft_archive_rejects_noncanonical_and_out_of_bounds_bytes() {
    let mut f = Fixture::new(false);
    let (metadata, ledger, _) = branch(&mut f, 3);
    let bytes = archive(&metadata, &ledger, true).encode().unwrap();
    assert!(StudioDraftArchive::decode(&bytes).is_ok());

    // Unknown version.
    let mut wrong_version = bytes.clone();
    wrong_version[0] = 2;
    assert!(StudioDraftArchive::decode(&wrong_version).is_err());

    // Unknown provenance tag.
    let mut wrong_provenance = bytes.clone();
    wrong_provenance[1] = 9;
    assert!(StudioDraftArchive::decode(&wrong_provenance).is_err());

    // A replayable flag that is neither 0 nor 1 must not be coerced.
    let mut wrong_flag = bytes.clone();
    wrong_flag[2] = 2;
    assert!(StudioDraftArchive::decode(&wrong_flag).is_err());

    // Trailing data.
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(StudioDraftArchive::decode(&trailing).is_err());

    // Truncation.
    assert!(StudioDraftArchive::decode(&bytes[..bytes.len() - 1]).is_err());

    // Over the payload bound, checked before any allocation of the contents.
    assert!(StudioDraftArchive::decode(&vec![0; MAX_STUDIO_DRAFT_ARCHIVE_BYTES + 1]).is_err());

    // An empty manifest is not an archive: disposal of nothing is not a disposal.
    assert!(StudioDraftArchive::decode(&[]).is_err());
}
