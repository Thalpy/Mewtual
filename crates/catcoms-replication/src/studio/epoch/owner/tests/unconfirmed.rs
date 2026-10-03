//! Design 8.1: the Unconfirmed basis, its fingerprint domain and its provenance.
//!
//! Built from a real receipt and seed, taken from the settlement fixture through the production
//! Closing path, then re-parsed as an unconfirmed seed exactly as a preview would hold it. Nothing
//! here constructs a basis by any route production lacks; the mint is the production constructor.

// The seed parser and the mint are reserved for `catcoms-sync` by the repository gate (design 8.1
// (ii)). These tests are the other place that legitimately calls them.
#![allow(clippy::disallowed_methods)]
use super::archive::branch;
use super::handoff::{branch as transferable_branch, signing};
use super::*;
use crate::studio::{
    StudioDiscardConfirmation, StudioDisposalDecision, StudioDraftArchive, StudioOverlayAdmission,
    StudioOverlayBasis, StudioOverlayProvenance, StudioOverlayRequestClass,
    StudioUnconfirmedOverlayBasis, UnconfirmedStudioSeed,
};
use crate::{InheritedCheckpoint, IntentLedger};

/// One receipt and seed, held both ways: as the Closing basis the fixture minted, and as the
/// unconfirmed seed a preview of the same checkpoint would hold.
struct Bases {
    closing: StudioClosingOverlayBasis,
    seed: UnconfirmedStudioSeed,
    receipt: Receipt,
    target: StudioTarget,
}

fn bases(f: &mut Fixture) -> Bases {
    let (state, _, _, closing) = branch(f, 0);
    seed_of(&state, closing)
}

/// The same receipt and seed a Closing branch was built on, re-parsed as a preview would hold it.
fn seed_of(state: &StudioOverlayState, closing: StudioClosingOverlayBasis) -> Bases {
    let overlay = state.overlay().unwrap();
    let receipt = overlay.receipt().clone();
    let target = overlay.target();
    let seed =
        UnconfirmedStudioSeed::parse_live_transfer(target, &receipt, overlay.seed()).unwrap();
    Bases {
        closing,
        seed,
        receipt,
        target,
    }
}

fn mint(
    b: &Bases,
    author: DeviceId,
    provider: DeviceId,
    epoch: u64,
    at: u64,
) -> StudioUnconfirmedOverlayBasis {
    StudioUnconfirmedOverlayBasis::mint_from_live_preview(
        &b.seed, &b.receipt, author, provider, epoch, at,
    )
    .unwrap()
}

/// A ledger holding one of the owner's operations, and its id.
fn one_intent(f: &mut Fixture, title: &str) -> (IntentLedger, [u8; 32]) {
    let mut ledger = IntentLedger::new(f.source.document().clone());
    let body = f.title_body(title);
    let op = f.domain(body);
    let id = ledger.prepare(f.owner.device_id(), op).unwrap();
    (ledger, id)
}

/// The provenance discriminant is the fingerprint's domain: over byte-identical receipt and seed,
/// a Closing and an Unconfirmed basis are different bases, and neither extends the other's branch.
#[test]
fn an_unconfirmed_basis_fingerprints_apart_from_a_closing_one_over_the_same_seed() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let b = bases(&mut f);
        let owner = f.owner.device_id();
        let unconfirmed = mint(&b, owner, owner, 1, 1_700_000_000_000);
        assert_ne!(unconfirmed.fingerprint(), b.closing.fingerprint());

        let (ledger, id) = one_intent(&mut f, "either way");
        let mut closing_branch = StudioOverlayState::new(&b.closing);
        assert!(matches!(
            closing_branch.append(&unconfirmed, &ledger, id, 1),
            Err(ReplError::EpochScope)
        ));
        let mut unconfirmed_branch = StudioOverlayState::new(&unconfirmed);
        assert!(matches!(
            unconfirmed_branch.append(&b.closing, &ledger, id, 1),
            Err(ReplError::EpochScope)
        ));
        // And each accepts its own, so the refusals above are about the basis and nothing else.
        closing_branch.append(&b.closing, &ledger, id, 1).unwrap();
        unconfirmed_branch
            .append(&unconfirmed, &ledger, id, 1)
            .unwrap();
    }
}

/// Each fingerprint is the documented derivation under its own domain, recomputed here as an
/// oracle rather than read back.
///
/// The Closing half is a compatibility pin: every existing vault's branches were fingerprinted
/// under that domain, and changing it would change every `branch_id` on disk. The Unconfirmed half
/// pins the separation itself. Comparing the two fingerprints cannot, because their source
/// identities already differ.
#[test]
fn each_fingerprint_is_its_documented_derivation_under_its_own_domain() {
    let mut f = Fixture::new(true);
    let b = bases(&mut f);
    let owner = f.owner.device_id();
    let unconfirmed = mint(&b, owner, owner, 1, 1);
    let closing = StudioOverlayBasis::from(&b.closing);
    let preview = StudioOverlayBasis::from(&unconfirmed);
    assert_eq!(
        closing.fingerprint(),
        blake3::derive_key(
            "catcoms/studio-closing-overlay-basis/v1",
            &closing.encoding_for_test()
        ),
        "the Closing domain every existing vault was written under"
    );
    assert_eq!(
        preview.fingerprint(),
        blake3::derive_key(
            "catcoms/studio-unconfirmed-overlay-basis/v1",
            &preview.encoding_for_test()
        )
    );
    assert_ne!(
        preview.fingerprint(),
        blake3::derive_key(
            "catcoms/studio-closing-overlay-basis/v1",
            &preview.encoding_for_test()
        ),
        "an Unconfirmed base must not fingerprint under the Closing domain"
    );
}

/// The fingerprint is the base's identity, not the observation that produced it. Two mints that
/// differ only in provider, MLS epoch and wall time are the same base, and a Save re-minted later
/// from a refreshed preview must extend the branch. The author is part of the base.
#[test]
fn the_unconfirmed_fingerprint_binds_the_author_and_not_the_admission_facts() {
    let mut f = Fixture::new(true);
    let b = bases(&mut f);
    let owner = f.owner.device_id();
    let first = mint(&b, owner, owner, 1, 1_700_000_000_000);
    let refreshed = mint(
        &b,
        owner,
        DeviceId::from_bytes([9; 32]),
        7,
        1_700_000_999_000,
    );
    assert_eq!(first.fingerprint(), refreshed.fingerprint());
    assert_ne!(
        first.provenance(),
        refreshed.provenance(),
        "precondition: the two mints really did observe different facts"
    );
    assert_ne!(
        mint(&b, DeviceId::from_bytes([5; 32]), owner, 1, 1).fingerprint(),
        first.fingerprint(),
        "another author is another base"
    );

    let (mut ledger, first_id) = one_intent(&mut f, "first save");
    let mut state = StudioOverlayState::new(&first);
    state.append(&first, &ledger, first_id, 1).unwrap();
    let body = f.title_body("later save");
    let op = f.domain(body);
    let later_id = ledger.prepare(owner, op).unwrap();
    state
        .append(&refreshed, &ledger, later_id, 2)
        .expect("a re-mint from a refreshed preview extends the same branch");
    assert_eq!(
        state.provenance(),
        first.provenance(),
        "provenance is recorded once, at admission, and a later Save does not rewrite it"
    );
}

/// The mint binds to the exact receipt the seed was proven against when it was parsed, by
/// equality, with no re-parse on the actor (review of `47a73463`, M2). That is strictly tighter
/// than a re-parse. A receipt naming another seed is refused, and so is a different, validly signed
/// receipt over the SAME seed, which a re-parse would have accepted. An observation time outside
/// the integer bound is refused before anything is built.
#[test]
fn the_mint_binds_the_receipt_the_seed_was_proven_against() {
    let mut f = Fixture::new(false);
    let b = bases(&mut f);
    let owner = f.owner.device_id();
    let refused = |receipt: &Receipt| {
        matches!(
            StudioUnconfirmedOverlayBasis::mint_from_live_preview(
                &b.seed, receipt, owner, owner, 1, 1
            ),
            Err(ReplError::EpochScope)
        )
    };
    let other_seed = Receipt::sign(
        b.receipt.document.clone(),
        b.receipt.closed_epoch,
        b.receipt.close_record_hash,
        [0xEE; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &f.owner,
    )
    .unwrap();
    assert!(refused(&other_seed), "a receipt naming another seed");

    let other_signer = Receipt::sign(
        b.receipt.document.clone(),
        b.receipt.closed_epoch,
        b.receipt.close_record_hash,
        b.receipt.seed_change_hash,
        0,
        InheritedCheckpoint::EpochZero,
        &MlsDevice::generate().unwrap(),
    )
    .unwrap();
    assert_ne!(other_signer, b.receipt);
    assert!(
        UnconfirmedStudioSeed::parse_live_transfer(b.target, &other_signer, b.seed.seed_bytes())
            .is_ok(),
        "precondition: the re-signed receipt re-parses the same seed, so only equality refuses it"
    );
    assert!(refused(&other_signer), "another receipt over the same seed");

    assert!(matches!(
        StudioUnconfirmedOverlayBasis::mint_from_live_preview(
            &b.seed,
            &b.receipt,
            owner,
            owner,
            1,
            u64::MAX
        ),
        Err(ReplError::EpochBound)
    ));
}

/// Review (i) change 2: the kind is not persisted, so the decoder must restore it from the
/// record's provenance. Otherwise a reloaded Unconfirmed branch fingerprints under the Closing
/// domain, its `branch_id` changes across a restart, and no later Save can name it again.
#[test]
fn an_unconfirmed_branch_keeps_its_identity_across_a_reload() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let b = bases(&mut f);
        let owner = f.owner.device_id();
        let basis = mint(&b, owner, owner, 3, 1_700_000_000_000);
        let (mut ledger, id) = one_intent(&mut f, "kept");
        let mut state = StudioOverlayState::new(&basis);
        state.append(&basis, &ledger, id, 1).unwrap();
        assert_eq!(state.provenance(), basis.provenance());

        let bytes = state.encode_vault(&ledger).unwrap();
        assert_eq!(
            bytes.first(),
            Some(&3),
            "Unconfirmed provenance is not v2-expressible"
        );
        for read in [
            StudioOverlayState::decode_vault(&bytes, &ledger).unwrap(),
            StudioOverlayState::decode_vault_structural(&bytes, &ledger).unwrap(),
        ] {
            assert_eq!(read.branch_id(), state.branch_id());
            assert_eq!(read.overlay().unwrap().basis(), basis.fingerprint());
            assert_eq!(read.provenance(), basis.provenance());
        }

        // And the reloaded branch accepts the next Save's re-mint.
        let mut read = StudioOverlayState::decode_vault(&bytes, &ledger).unwrap();
        let body = f.title_body("after a restart");
        let op = f.domain(body);
        let next = ledger.prepare(owner, op).unwrap();
        let remint = mint(&b, owner, owner, 4, 1_700_000_500_000);
        read.append(&remint, &ledger, next, 2).unwrap();
    }
}

/// Review (ii) change 4: admission takes its provenance from the basis, and a label that disagrees
/// is refused in BOTH directions. The dangerous one is an Unconfirmed base labelled Closing, which
/// the handoff guard (keyed on the label) would otherwise let through to signing.
#[test]
fn admission_refuses_a_basis_under_the_other_provenance() {
    let mut f = Fixture::new(true);
    let b = bases(&mut f);
    let owner = f.owner.device_id();
    let unconfirmed = mint(&b, owner, owner, 1, 1_700_000_000_000);

    // A disposed first branch, so a second generation can be admitted.
    let (ledger, id) = one_intent(&mut f, "disposed");
    let mut first = StudioOverlayState::new(&b.closing);
    first.append(&b.closing, &ledger, id, 1).unwrap();
    let (vacant, _) = first
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(
                StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap(),
            ),
            first.branch_content(&ledger).unwrap(),
            1,
            1,
        )
        .unwrap();
    let next = StudioOverlayAdmission::New { generation: 2 };

    assert!(matches!(
        vacant.new_admitted(&unconfirmed, next, StudioOverlayProvenance::Closing),
        Err(ReplError::IntentConflict)
    ));
    assert!(matches!(
        vacant.new_admitted(&b.closing, next, unconfirmed.provenance()),
        Err(ReplError::IntentConflict)
    ));
    // Each under its own label is admitted, so the refusals are the label check and nothing else.
    assert_eq!(
        vacant
            .new_admitted(&unconfirmed, next, unconfirmed.provenance())
            .unwrap()
            .provenance(),
        unconfirmed.provenance()
    );
    assert_eq!(
        vacant
            .new_admitted(&b.closing, next, StudioOverlayProvenance::Closing)
            .unwrap()
            .provenance(),
        StudioOverlayProvenance::Closing
    );
}

/// Design 8.5: an Unconfirmed branch can never be handed off. Its authority can be captured, since
/// the receipt is a real one, and the detached preparation still refuses it by provenance before
/// touching the source.
#[test]
fn an_unconfirmed_branch_is_never_prepared_for_handoff() {
    let mut f = Fixture::new(true);
    let b = bases(&mut f);
    let owner = f.owner.device_id();
    let basis = mint(&b, owner, owner, 1, 1_700_000_000_000);
    let (ledger, id) = one_intent(&mut f, "never signed");
    let mut state = StudioOverlayState::new(&basis);
    state.append(&basis, &ledger, id, 1).unwrap();
    let authority = state.handoff_authority(&f.owner, &f.group, 0).unwrap();
    let source = f.source.copy_handoff_source(&f.group).unwrap();
    assert!(matches!(
        state.prepare_handoff_detached(source, ledger, authority),
        Err(ReplError::EpochAuthority)
    ));
}

/// The decode-path fence against a relabelled record (review of `47a73463`, M3).
///
/// An Unconfirmed record whose outer label is rewritten to Closing would decode, fingerprint under
/// the Closing domain, and pass the handoff guard, which keys on the label. Only generation 2 and
/// later can carry the relabel: at generation 1 the result is v2-expressible and the canonical
/// re-encode check refuses it already. So the record here is a generation-2 preview branch
/// admitted after a transfer, which leaves no disposal manifest and so ends in exactly the
/// generation, the provenance block and a zero presence byte. Its only defence is "a Closing label
/// needs a complete source identity", and removing that rule makes this test fail.
#[test]
fn a_generation_two_preview_record_relabelled_closing_is_refused() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, _) = transferable_branch(&mut f, 2);
    let mut batch = signing(&mut f, &metadata, &ledger);
    while batch.sign_next(&f.owner, &f.group, 0).unwrap() {}
    let (candidate, prepared) = batch.finish().unwrap().into_parts();
    let transferred = prepared.complete(&candidate, &ledger).unwrap();

    // The transfer raised the basis floor, so generation 2 needs a base from the successor's own
    // close (as lifecycle's crafted-record test explains), here taken as a preview would hold it.
    let first = metadata.overlay().unwrap().receipt().clone();
    f.fill();
    let decision = f.decide(Some(&first));
    let _plan = f.plan(&decision);
    let closing = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let state = StudioOverlayState::new(&closing);
    let b = seed_of(&state, closing);
    let owner = f.owner.device_id();
    let preview = mint(&b, owner, owner, 5, 1_700_000_000_000);
    let mut revived = IntentLedger::new(ledger.document().clone());
    let body = f.title_body("generation two from a preview");
    let op = f.domain(body);
    let id = revived.prepare(owner, op).unwrap();
    let mut g2 = transferred
        .new_admitted(
            &preview,
            StudioOverlayAdmission::New { generation: 2 },
            preview.provenance(),
        )
        .unwrap();
    g2.append(&preview, &revived, id, 900)
        .expect("precondition: the generation-2 preview branch accepts its first Save");
    let bytes = g2.encode_vault(&revived).unwrap();
    StudioOverlayState::decode_vault(&bytes, &revived)
        .expect("precondition: the genuine record decodes");

    // generation(8) | tag 1 | provider (4-byte length + 32) | epoch(8) | at(8) | presence 0.
    let n = bytes.len();
    assert_eq!(bytes[0], 3);
    assert_eq!(
        bytes[n - 1],
        0,
        "no disposal manifest, or the offsets are wrong"
    );
    assert_eq!(
        u64::from_be_bytes(bytes[n - 62..n - 54].try_into().unwrap()),
        2,
        "the generation must sit right before the provenance block"
    );
    assert_eq!(
        bytes[n - 54],
        1,
        "the Unconfirmed tag follows the generation"
    );
    let relabelled = [&bytes[..n - 54], &[0u8, 0u8][..]].concat();
    assert!(matches!(
        StudioOverlayState::decode_vault(&relabelled, &revived),
        Err(ReplError::Malformed)
    ));
    assert!(matches!(
        StudioOverlayState::decode_vault_structural(&relabelled, &revived),
        Err(ReplError::Malformed)
    ));
}

/// An Unconfirmed branch's identity carries through everything that records it: an archive of the
/// reloaded branch, a disposal manifest and its round trip, and the classification of a delayed
/// request against that manifest. After the disposal a Closing branch is admitted at the next
/// generation, so the kinds can alternate across generations (review of `47a73463`, L1).
#[test]
fn an_unconfirmed_branch_archives_and_disposes_under_its_own_basis() {
    let mut f = Fixture::new(false);
    let b = bases(&mut f);
    let owner = f.owner.device_id();
    let preview = mint(&b, owner, owner, 2, 1_700_000_000_000);
    let (ledger, id) = one_intent(&mut f, "preview work, archived then disposed");
    let intent = ledger
        .pending()
        .find(|(key, _)| **key == id)
        .map(|(_, intent)| intent.clone())
        .unwrap();
    let mut state = StudioOverlayState::new(&preview);
    state.append(&preview, &ledger, id, 1).unwrap();
    let read =
        StudioOverlayState::decode_vault(&state.encode_vault(&ledger).unwrap(), &ledger).unwrap();
    let branch = read.branch_id().unwrap();
    let content = read.branch_content(&ledger).unwrap();

    let archive = StudioDraftArchive::from_branch(
        read.overlay().unwrap(),
        &ledger,
        read.provenance(),
        true,
        branch,
        content,
        read.branch_generation(),
    )
    .unwrap();
    assert_eq!(archive.basis(), preview.fingerprint());
    let decoded = StudioDraftArchive::decode(&archive.encode().unwrap()).unwrap();
    assert_eq!(decoded.basis(), preview.fingerprint());
    assert_eq!(decoded.provenance(), preview.provenance());

    let (after, _) = read
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(
                StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap(),
            ),
            content,
            1,
            1,
        )
        .unwrap();
    let after =
        StudioOverlayState::decode_vault(&after.encode_vault(&ledger).unwrap(), &ledger).unwrap();
    let manifest = after.disposed().unwrap();
    assert_eq!(manifest.basis, preview.fingerprint());
    assert_eq!(manifest.branch, branch);
    assert_eq!(manifest.provenance, preview.provenance());
    assert!(matches!(
        after.classify_request(b.target, branch, &intent).unwrap(),
        StudioOverlayRequestClass::Disposed(_)
    ));
    assert_eq!(
        after
            .new_admitted(
                &b.closing,
                StudioOverlayAdmission::New { generation: 2 },
                StudioOverlayProvenance::Closing
            )
            .unwrap()
            .provenance(),
        StudioOverlayProvenance::Closing
    );
}
