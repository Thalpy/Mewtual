//! Design 8.1: the Unconfirmed basis, its fingerprint domain and its provenance.
//!
//! Built from a real receipt and seed, taken from the settlement fixture through the production
//! Closing path, then re-parsed as an unconfirmed seed exactly as a preview would hold it. Nothing
//! here constructs a basis by any route production lacks; the mint is the production constructor.

// The seed parser and the mint are reserved for `catcoms-sync` by the repository gate (design 8.1
// (ii)). These tests are the other place that legitimately calls them.
#![allow(clippy::disallowed_methods)]
use super::archive::branch;
use super::*;
use crate::studio::{
    StudioDiscardConfirmation, StudioDisposalDecision, StudioOverlayAdmission, StudioOverlayBasis,
    StudioOverlayProvenance, StudioUnconfirmedOverlayBasis, UnconfirmedStudioSeed,
};
use crate::{InheritedCheckpoint, IntentLedger};

/// One receipt and seed, held both ways: as the Closing basis the fixture minted, and as the
/// unconfirmed seed a preview of the same checkpoint would hold.
struct Bases {
    closing: StudioClosingOverlayBasis,
    seed: UnconfirmedStudioSeed,
    receipt: Receipt,
}

fn bases(f: &mut Fixture) -> Bases {
    let (state, _, _, closing) = branch(f, 0);
    let overlay = state.overlay().unwrap();
    let receipt = overlay.receipt().clone();
    let seed =
        UnconfirmedStudioSeed::parse_live_transfer(overlay.target(), &receipt, overlay.seed())
            .unwrap();
    Bases {
        closing,
        seed,
        receipt,
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

/// Design 8.1 part 3: the mint does not trust the seed's retention. A receipt that names another
/// seed is refused when the retained bytes are re-parsed against it, and an observation time
/// outside the integer bound is refused before anything is built.
#[test]
fn the_mint_re_parses_the_retained_seed_against_the_receipt_it_is_given() {
    let mut f = Fixture::new(false);
    let b = bases(&mut f);
    let owner = f.owner.device_id();
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
    assert!(StudioUnconfirmedOverlayBasis::mint_from_live_preview(
        &b.seed,
        &other_seed,
        owner,
        owner,
        1,
        1
    )
    .is_err());
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
