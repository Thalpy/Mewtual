//! Draft archive codec, against real branches built through the production basis path. A
//! synthetic basis would need a test-only constructor on `StudioClosingOverlayBasis`, which is
//! exactly the back door the overlay design forbids, so these reuse the settlement fixture.
use super::*;
use crate::studio::{StudioDraftArchive, StudioOverlayProvenance, MAX_STUDIO_DRAFT_ARCHIVE_BYTES};
use crate::IntentLedger;

/// A real accepted branch: real Closing basis, real ledger, real typed acceptance. Returns the
/// branch, its ledger, and the authored (body, timestamp) pairs in acceptance order.
#[allow(clippy::type_complexity)]
fn branch(
    f: &mut Fixture,
    count: usize,
) -> (
    StudioOverlayState,
    IntentLedger,
    Vec<(DomainOp, u64)>,
    StudioClosingOverlayBasis,
) {
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
    (metadata, ledger, ordered, basis)
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
        let (metadata, ledger, ordered, _basis) = branch(&mut f, 4);
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
    let (metadata, ledger, ordered, _basis) = branch(&mut f, 8);
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
        let (metadata, ledger, _, _basis) = branch(&mut f, 3);
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

/// A branch that is structurally consistent but that the ordered typed replay refuses. The
/// recipe is C1-TEST-002's, in `handoff.rs`: name a sound-effect operation the typed writer
/// does not support, which `checked_entries` never decodes, then retarget one canonical
/// accepted entry's fixed-width id and envelope at it so the record stays canonical. Duplicated
/// rather than shared because `handoff.rs` is a sibling test module whose helpers are private,
/// and because this branch is rebased often enough that cross-file edits cost more than the
/// duplication does. If that fixture's framing changes, this one must follow.
fn unreplayable_branch(f: &mut Fixture) -> (StudioOverlay, IntentLedger) {
    f.fill();
    let decision = f.decide(None);
    let _plan = f.plan(&decision);
    let basis = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());

    let supported = f.domain(f.title_body("a replayable title"));
    let supported_id = ledger.prepare(f.owner.device_id(), supported).unwrap();
    let unsupported = f.domain(
        FlipnoteOp::SetSfx {
            sfx: [0xc3; 16],
            frame: [1; 16],
            patch: [0xc3; 32],
            note: 7,
        }
        .encode()
        .unwrap(),
    );
    let unsupported_id = ledger
        .prepare(f.owner.device_id(), unsupported.clone())
        .unwrap();
    assert!(
        StudioOverlay::new(&basis)
            .append(&basis, &ledger, unsupported_id, 200)
            .is_err(),
        "the unsupported operation must be unacceptable through the ordinary path"
    );

    let mut accepted = StudioOverlay::new(&basis);
    accepted.append(&basis, &ledger, supported_id, 200).unwrap();
    let canonical = accepted.encode_vault(&ledger).unwrap();

    let entry = canonical.len() - 88;
    let mut spliced = canonical.clone();
    assert_eq!(&spliced[entry + 4..entry + 36], &supported_id[..]);
    spliced[entry + 4..entry + 36].copy_from_slice(&unsupported_id);
    let envelope = {
        let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-overlay-envelope/v1");
        hash.update(f.owner.device_id().as_bytes());
        hash.update(&unsupported.encode().unwrap());
        *hash.finalize().as_bytes()
    };
    spliced[entry + 40..entry + 72].copy_from_slice(&envelope);

    let structural = StudioOverlay::decode_vault_structural(&spliced, &ledger)
        .expect("structurally consistent branch must decode structurally");
    assert!(
        structural.read(&ledger).is_err(),
        "this fixture is worthless unless typed replay actually refuses it"
    );
    (structural, ledger)
}

#[test]
fn draft_archive_needs_no_typed_reconstruction() {
    // The claim is that archiving does not depend on typed replay. Every branch built through
    // `append` is replayable by construction, so passing `replayable: false` for one of those
    // proves nothing: an implementation that called `read` inside `from_branch` would pass.
    // This uses a branch typed replay genuinely refuses, so adding replay to `from_branch`
    // fails here. That mutation is the one this test exists for.
    let mut f = Fixture::new(true);
    let (overlay, ledger) = unreplayable_branch(&mut f);

    let built = StudioDraftArchive::from_branch(
        &overlay,
        &ledger,
        StudioOverlayProvenance::Closing,
        false,
        [3; 32],
        [4; 32],
        7,
    )
    .expect("a non-replayable branch must still archive");
    let bytes = built.encode().unwrap();
    let read = StudioDraftArchive::decode(&bytes).unwrap();
    assert!(!read.replayable());
    assert_eq!(read.accepted(), 1);
    assert_eq!(read.encode().unwrap(), bytes);
    // References still come out, because they never needed the replay either.
    assert_eq!(read.blob_cids().unwrap(), overlay.base_blob_cids().unwrap());
}

#[test]
fn draft_archive_records_the_replayable_label_without_changing_content() {
    let mut f = Fixture::new(false);
    let (metadata, ledger, _, _basis) = branch(&mut f, 2);
    let honest = archive(&metadata, &ledger, true).encode().unwrap();
    let labelled = archive(&metadata, &ledger, false).encode().unwrap();
    assert_ne!(honest, labelled, "the label must be in the bytes");
    assert!(!StudioDraftArchive::decode(&labelled).unwrap().replayable());
    assert_eq!(
        StudioDraftArchive::decode(&labelled)
            .unwrap()
            .blob_cids()
            .unwrap(),
        StudioDraftArchive::decode(&honest)
            .unwrap()
            .blob_cids()
            .unwrap()
    );
}

#[test]
fn draft_archive_round_trips_unconfirmed_provenance() {
    // The Unconfirmed wire branch carries three extra fields and is the one the preview-based
    // local-work path will use. Untested, a field could be dropped or reordered and only that
    // path would notice, long after this codec was called finished.
    let mut f = Fixture::new(true);
    let (metadata, ledger, _, _basis) = branch(&mut f, 2);
    let provenance = StudioOverlayProvenance::Unconfirmed {
        provider: f.owner.device_id(),
        observed_mls_epoch: 11,
        observed_at_ms: 1_700_000_000_123,
    };
    let built = StudioDraftArchive::from_branch(
        metadata.overlay().unwrap(),
        &ledger,
        provenance,
        true,
        [3; 32],
        [4; 32],
        7,
    )
    .unwrap();
    let bytes = built.encode().unwrap();
    let read = StudioDraftArchive::decode(&bytes).unwrap();
    assert_eq!(read.provenance(), provenance);
    assert_eq!(read.encode().unwrap(), bytes);

    // The provenance is in the bytes, not merely in the returned value.
    let closing = archive(&metadata, &ledger, true).encode().unwrap();
    assert_ne!(closing, bytes);
    assert_eq!(
        StudioDraftArchive::decode(&closing).unwrap().provenance(),
        StudioOverlayProvenance::Closing
    );
}

// The codec's constants, mirrored so a change on either side has to be made deliberately in
// both places rather than drifting silently past the proof below.
const ENTRY_OVERHEAD_BYTES_FOR_TEST: usize = 160;
const HEADER_BYTES_FOR_TEST: usize = 2048;

#[test]
fn draft_archive_constants_cover_the_real_encoder() {
    // ENTRY_OVERHEAD_BYTES and HEADER_BYTES are padding constants, not schema-derived
    // expressions, so MAX_STUDIO_DRAFT_ARCHIVE_BYTES is only trustworthy if something measures
    // them against the encoder that actually runs. Difference two real encodings to recover the
    // per-entry framing and the fixed header, extrapolate the document to the largest shape the
    // codec admits, and require the result still to fit. A new field, wider framing or a larger
    // field maximum fails here instead of silently narrowing the bound.
    let mut f = Fixture::new(true);
    let (metadata, ledger, ordered, basis) = branch(&mut f, 2);
    let overlay = metadata.overlay().unwrap();
    let document = ledger.document().clone();

    let body = ordered[0].0.encode().unwrap().len();
    assert_eq!(
        body,
        ordered[1].0.encode().unwrap().len(),
        "differencing requires both entries to carry equal bodies"
    );

    // Worst case in every optional field: Unconfirmed carries three extra values, and the
    // integers are at their widest encoding.
    let worst = StudioOverlayProvenance::Unconfirmed {
        provider: f.owner.device_id(),
        observed_mls_epoch: u64::MAX,
        observed_at_ms: u64::MAX,
    };
    let encode = |overlay: &StudioOverlay| {
        StudioDraftArchive::from_branch(overlay, &ledger, worst, true, [3; 32], [4; 32], u64::MAX)
            .unwrap()
            .encode()
            .unwrap()
            .len()
    };
    let mut single = StudioOverlay::new(&basis);
    single
        .append(&basis, &ledger, ordered[0].0.id(&f.owner.device_id()), 1)
        .unwrap();
    let (one, two) = (encode(&single), encode(overlay));

    // One entry's complete cost, then its framing alone.
    let entry_framing = (two - one).saturating_sub(body);
    assert!(
        entry_framing <= ENTRY_OVERHEAD_BYTES_FOR_TEST,
        "measured per-entry framing {entry_framing} exceeds ENTRY_OVERHEAD_BYTES"
    );

    // Establish the document maxima rather than assuming them, so this extrapolation cannot go
    // stale if `LogicalDocument::new` changes what it admits.
    let (max_server, max_key) = (256usize, 192usize);
    let build = |server: usize, key: usize| {
        LogicalDocument::new(vec![7; server], document.doc_type, vec![9; key]).is_ok()
    };
    assert!(build(max_server, max_key), "assumed maxima must be legal");
    assert!(!build(max_server + 1, max_key), "server id maximum moved");
    assert!(!build(max_server, max_key + 1), "logical key maximum moved");

    let receipt = overlay.receipt().encode().len();
    let seed = overlay.seed().len();
    let header = one.saturating_sub(receipt + seed + body + entry_framing);
    let at_maximum =
        header + (max_server - document.server_id.len()) + (max_key - document.logical_key.len());
    assert!(
        at_maximum <= HEADER_BYTES_FOR_TEST,
        "header at the maximal document shape is {at_maximum}, over HEADER_BYTES"
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
    let (metadata, ledger, ordered, _basis) = branch(&mut f, 2);
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
fn draft_archive_rejects_a_changed_body_under_an_unchanged_id() {
    // `DomainOp::id` hashes the logical key, author and nonce and NOT the body, so an id alone
    // cannot establish that an entry carries the accepted operation: a different body under the
    // same nonce keeps the same id. That is why the archive carries the envelope, which does
    // hash the body, and why the swapped-id test below is not sufficient on its own.
    //
    // Alter one character of a title inside an encoded archive. Every length is unchanged, so
    // the record stays canonical; the nonce, author and logical key are untouched, so the id
    // check still passes. Only the envelope comparison can reject this.
    let mut f = Fixture::new(false);
    let (metadata, ledger, _, _basis) = branch(&mut f, 2);
    let bytes = archive(&metadata, &ledger, true).encode().unwrap();
    assert!(StudioDraftArchive::decode(&bytes).is_ok());

    let needle = b"archived title 0";
    let at = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("the title must appear verbatim in the archived body");
    let mut tampered = bytes.clone();
    tampered[at + needle.len() - 1] = b'X';
    assert_eq!(tampered.len(), bytes.len(), "length must not change");

    // The id is genuinely still correct for this entry: prove that, so the refusal below is
    // attributable to the envelope and not to a coincidentally broken identity.
    let ops: Vec<_> = ledger.pending().map(|(id, _)| *id).collect();
    assert!(
        ops.iter()
            .any(|id| tampered.windows(32).any(|w| w == id.as_slice())),
        "the entry ids must survive the tamper untouched"
    );

    assert!(
        StudioDraftArchive::decode(&tampered).is_err(),
        "an entry body that does not match its accepted envelope must reject"
    );
}

#[test]
fn draft_archive_rejects_noncanonical_and_out_of_bounds_bytes() {
    let mut f = Fixture::new(false);
    let (metadata, ledger, _, _basis) = branch(&mut f, 3);
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
