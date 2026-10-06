use super::*;
use crate::IntentLedger;

pub(super) fn branch(
    f: &mut Fixture,
    count: usize,
) -> (StudioOverlayState, IntentLedger, Vec<(DomainOp, u64)>) {
    f.fill();
    let decision = f.decide(None);
    let plan = f.plan(&decision);
    let basis = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());
    let mut metadata = StudioOverlayState::new(&basis);
    let mut ordered = Vec::new();
    for n in 0..count {
        let op = f.domain(f.title_body(&format!("retained title {n}")));
        let ts = 123 + n as u64;
        let id = ledger.prepare(f.owner.device_id(), op.clone()).unwrap();
        metadata.append(&basis, &ledger, id, ts).unwrap();
        ordered.push((op, ts));
    }
    f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
    assert_eq!((f.source.epoch(), f.source.op_count()), (1, 0));
    (metadata, ledger, ordered)
}
// Widened for the disposal tests' transfer-hold case. The successor checkpoint above is the part
// that matters and the part the archive module's own `branch` deliberately skips: without a
// successor epoch there is nothing to hand off into, so `handoff_authority` refuses and a test
// that wanted a live transfer hold would fail while building its fixture.

pub(super) fn signing(
    f: &mut Fixture,
    metadata: &StudioOverlayState,
    ledger: &IntentLedger,
) -> StudioHandoffSigning {
    let authority = metadata.handoff_authority(&f.owner, &f.group, 0).unwrap();
    let source = f.source.copy_handoff_source(&f.group).unwrap();
    metadata
        .clone()
        .prepare_handoff_detached(source, ledger.clone(), authority)
        .unwrap()
}

#[test]
fn studio_handoff_preparation_one_turn_and_full_output_match_ordinary_edits() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, ordered) = branch(&mut f, 4);
        let source_before = f.source.snapshot().unwrap();
        let metadata_before = metadata.encode_vault(&ledger).unwrap();
        let ledger_before = ledger.encode().unwrap();
        let expected = metadata
            .overlay()
            .unwrap()
            .read(&ledger)
            .unwrap()
            .projection()
            .clone();
        // Independent production oracle: ordinary typed edits, with the ORIGINAL acceptance
        // order and timestamp, rather than using either new preparation or the batch adapter.
        let mut ordinary = f.source.copy_handoff_source(&f.group).unwrap();
        for (op, ts) in &ordered {
            ordinary
                .edit_or_reseal(&f.owner, &f.group, &mut f.rng, op, *ts)
                .unwrap();
        }
        let mut batch = signing(&mut f, &metadata, &ledger);
        assert_eq!(batch.remaining(), ordered.len());
        for n in 0..ordered.len() {
            assert!(batch.sign_next(&f.owner, &f.group, 0).unwrap());
            assert_eq!(
                batch.remaining(),
                ordered.len() - n - 1,
                "signing turn consumed more than one operation"
            );
            assert_eq!(
                f.source.snapshot().unwrap(),
                source_before,
                "private signing changed installed source"
            );
        }
        assert!(!batch.sign_next(&f.owner, &f.group, 0).unwrap());
        let (mut candidate, prepared) = batch.finish().unwrap().into_parts();
        assert_eq!(candidate.op_count(), ordered.len());
        assert_eq!(candidate.projection().unwrap(), expected);
        assert_eq!(
            candidate.doc.signed_log(),
            ordinary.doc.signed_log(),
            "prepared signing changed original full signed envelopes"
        );
        assert_eq!(candidate.snapshot().unwrap(), ordinary.snapshot().unwrap());
        assert!(prepared.matches_source_before(&mut f.source).unwrap());
        assert_eq!(
            prepared.evidence(&candidate, &ledger).unwrap(),
            StudioHandoffEvidence::Complete
        );
        assert!(prepared
            .complete(&candidate, &ledger)
            .unwrap()
            .overlay()
            .is_none());
        assert_eq!(metadata.encode_vault(&ledger).unwrap(), metadata_before);
        assert_eq!(ledger.encode().unwrap(), ledger_before);
        assert_eq!(f.source.snapshot().unwrap(), source_before);
        let restored = StudioEpoch::restore(
            &candidate.snapshot().unwrap(),
            &f.group,
            candidate.target,
            f.owner.device_id(),
        )
        .unwrap();
        assert_eq!(restored.projection().unwrap(), expected);
        assert_eq!(restored.doc.signed_log(), ordinary.doc.signed_log());
    }
}

/// `check_handoff_authority_after_resolution` answers for the active branch a resolution returns
/// to, and only when the caller holds the evidence that says it will return there.
///
/// P2 asks it of a Prepared branch whose evidence is Absent. A review found the "only after
/// Absent" contract documented but not enforced: on a Hold or Complete branch it would have given
/// the verdict of an active branch that branch never becomes again. Here every evidence value is
/// tried, and the Absent answer is compared, in both directions, with what the returned branch's
/// own `handoff_authority` says.
#[test]
fn authority_after_resolution_answers_only_for_the_branch_absent_evidence_returns() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let mut batch = signing(&mut f, &metadata, &ledger);
        while batch.sign_next(&f.owner, &f.group, 0).unwrap() {}
        let (candidate, prepared) = batch.finish().unwrap().into_parts();
        assert!(
            matches!(
                prepared.handoff_authority(&f.owner, &f.group, 0),
                Err(ReplError::EpochClosed)
            ),
            "precondition: the minting check refuses anything still Prepared"
        );

        // Absent: the pristine successor holds none of the branch. The verdict must be the
        // returned branch's, for a tenure that passes and one that does not.
        let absent = prepared.evidence(&f.source, &ledger).unwrap();
        assert_eq!(absent, StudioHandoffEvidence::Absent);
        let returned = prepared.return_to_active(&f.source, &ledger).unwrap();
        assert!(
            returned.handoff_authority(&f.owner, &f.group, 0).is_ok(),
            "precondition: the returned branch is transferable under its own tenure"
        );
        for tenure in [0, 7] {
            assert_eq!(
                prepared
                    .check_handoff_authority_after_resolution(
                        Some(absent),
                        &f.owner,
                        &f.group,
                        tenure
                    )
                    .is_ok(),
                returned
                    .handoff_authority(&f.owner, &f.group, tenure)
                    .is_ok(),
                "tenure {tenure}: Absent must answer as the branch it returns to"
            );
        }

        // Anything else: the branch is not going back to active, so no verdict is given for it.
        assert_eq!(
            prepared.evidence(&candidate, &ledger).unwrap(),
            StudioHandoffEvidence::Complete,
            "precondition: the candidate holds the whole branch"
        );
        for evidence in [
            Some(StudioHandoffEvidence::Complete),
            Some(StudioHandoffEvidence::Hold),
            None,
        ] {
            assert!(
                matches!(
                    prepared
                        .check_handoff_authority_after_resolution(evidence, &f.owner, &f.group, 0),
                    Err(ReplError::EpochClosed)
                ),
                "{evidence:?} must not get an active branch's verdict"
            );
        }

        // An Active branch is asked exactly as `handoff_authority` asks it; evidence is ignored.
        for evidence in [None, Some(StudioHandoffEvidence::Hold)] {
            assert!(metadata
                .check_handoff_authority_after_resolution(evidence, &f.owner, &f.group, 0)
                .is_ok());
        }
    }
}

#[test]
fn studio_handoff_preparation_partial_finish_and_changed_source_refuse() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let before = f.source.snapshot().unwrap();
        let mut batch = signing(&mut f, &metadata, &ledger);
        assert!(batch.sign_next(&f.owner, &f.group, 0).unwrap());
        assert_eq!(batch.remaining(), 1);
        assert!(
            matches!(batch.finish(), Err(ReplError::IntentConflict)),
            "partial signed batch escaped finish"
        );
        assert_eq!(f.source.snapshot().unwrap(), before);
        let authority = metadata.handoff_authority(&f.owner, &f.group, 0).unwrap();
        f.edit(f.title_body("independent successor progress"));
        let current = f.source.snapshot().unwrap();
        let changed = f.source.copy_handoff_source(&f.group).unwrap();
        assert!(matches!(
            metadata
                .clone()
                .prepare_handoff_detached(changed, ledger.clone(), authority),
            Err(ReplError::EpochClosed)
        ));
        assert_eq!(f.source.snapshot().unwrap(), current);
        assert_eq!(
            metadata
                .overlay()
                .unwrap()
                .read(&ledger)
                .unwrap()
                .accepted(),
            2
        );
    }
}

#[test]
fn studio_handoff_preparation_mls_change_rejects_next_signature_with_same_owner() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let before = f.source.snapshot().unwrap();
        let mut batch = signing(&mut f, &metadata, &ledger);
        assert!(batch.sign_next(&f.owner, &f.group, 0).unwrap());
        let epoch = f.group.epoch();
        let member = MlsDevice::generate().unwrap();
        f.group
            .add_member(&f.owner, member.key_package().unwrap())
            .unwrap();
        assert!(f.group.epoch() > epoch);
        // The existing owner remains designated; membership/key and observed tenure remain
        // valid. The actual receipt still verifies, so only the captured MLS epoch is stale.
        assert_eq!(f.group.designated_committer(), Some(f.owner.device_id()));
        assert_eq!(
            f.group.member_signature_key(&f.owner.device_id()),
            Some(f.owner.public_key_bytes())
        );
        metadata
            .overlay()
            .unwrap()
            .receipt()
            .verify_current_owner(&f.group, 0)
            .unwrap();
        assert!(
            matches!(
                batch.sign_next(&f.owner, &f.group, 0),
                Err(ReplError::EpochAuthority)
            ),
            "changed MLS epoch authorized another prepared signature"
        );
        assert_eq!(batch.remaining(), 1);
        assert_eq!(f.source.snapshot().unwrap(), before);
        // A fresh capture can still prepare and sign the same retained work under current context.
        let mut fresh = signing(&mut f, &metadata, &ledger);
        assert!(fresh.sign_next(&f.owner, &f.group, 0).unwrap());
        assert_eq!(fresh.remaining(), 1);
    }
}

#[test]
fn studio_handoff_preparation_wrong_signer_and_observed_tenure_preserve_pending() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let mut batch = signing(&mut f, &metadata, &ledger);
        let other = MlsDevice::generate().unwrap();
        for (device, tenure) in [(&other, 0), (&f.owner, 1)] {
            assert!(matches!(
                batch.sign_next(device, &f.group, tenure),
                Err(ReplError::EpochAuthority)
            ));
            assert_eq!(batch.remaining(), 2);
        }
        assert!(batch.sign_next(&f.owner, &f.group, 0).unwrap());
        assert_eq!(batch.remaining(), 1);
    }
}

#[test]
fn studio_handoff_preparation_source_owner_must_match_verified_authority() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let before = f.source.snapshot().unwrap();
        let authority = metadata.handoff_authority(&f.owner, &f.group, 0).unwrap();
        let wrong_owner = MlsDevice::generate().unwrap().device_id();
        assert_ne!(wrong_owner, f.owner.device_id());
        let wrong = StudioEpoch::prepare_vault_source(
            &before,
            &f.group.group_id(),
            f.source.target,
            f.owner.device_id(),
            wrong_owner,
        )
        .unwrap();
        assert_eq!(wrong.projection().unwrap(), f.source.projection().unwrap());
        assert_eq!(
            (
                wrong.doc_id(),
                wrong.epoch(),
                wrong.op_count(),
                wrong.phase()
            ),
            (f.source.doc_id(), 1, 0, EpochPhase::Open)
        );
        metadata
            .overlay()
            .unwrap()
            .receipt()
            .verify_current_owner(&f.group, 0)
            .unwrap();
        assert!(
            matches!(
                metadata
                    .clone()
                    .prepare_handoff_detached(wrong, ledger.clone(), authority),
                Err(ReplError::EpochAuthority)
            ),
            "captured source owner bypassed verified handoff authority"
        );
        assert_eq!(f.source.snapshot().unwrap(), before);
        assert_eq!(signing(&mut f, &metadata, &ledger).remaining(), 2);
    }
}

/// SIGN-TEST-002 (core signing review). The captured authority is bound to the branch's receipt.
///
/// Authority is captured from a branch opened on the first close's receipt R1. A second close
/// cycle then produces R2, a branch on it, and R2's successor. That branch, its ledger and its
/// successor are consistent with one another, so `check_overlay_successor` accepts them, and R1
/// still verifies as the current owner's receipt, so every signing turn's `check_live` would pass
/// as well. The receipt comparison in `prepare_handoff_detached` is the one thing that refuses.
/// Without it, a branch prepared against R2 would be signed under authority captured for R1: in an
/// A -> B -> A tenure, authority from one tenure signing a branch opened in another.
#[test]
fn studio_handoff_preparation_refuses_authority_captured_for_another_receipt() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (first, _, _) = branch(&mut f, 1);
        let r1 = first.overlay().unwrap().receipt().clone();
        let authority = first.handoff_authority(&f.owner, &f.group, 0).unwrap();

        // A second cycle, from the epoch-1 successor `branch` left installed.
        f.fill();
        let decision = f.decide(Some(&r1));
        let plan = f.plan(&decision);
        let basis = f
            .source
            .prepare_closing_overlay(decision.close(), &f.group, 0)
            .unwrap();
        let mut ledger = IntentLedger::new(f.source.document().clone());
        let mut second = StudioOverlayState::new(&basis);
        for n in 0..2u64 {
            let op = f.domain(f.title_body(&format!("second-cycle title {n}")));
            let id = ledger.prepare(f.owner.device_id(), op).unwrap();
            second.append(&basis, &ledger, id, 300 + n).unwrap();
        }
        f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
        assert_eq!((f.source.epoch(), f.source.op_count()), (2, 0));
        assert_ne!(
            second.overlay().unwrap().receipt(),
            &r1,
            "precondition: two distinct receipts"
        );
        r1.verify_current_owner(&f.group, 0)
            .expect("precondition: the captured receipt still verifies, so only binding refuses");

        let before = f.source.snapshot().unwrap();
        let source = f.source.copy_handoff_source(&f.group).unwrap();
        assert!(
            matches!(
                second
                    .clone()
                    .prepare_handoff_detached(source, ledger.clone(), authority),
                Err(ReplError::EpochScope)
            ),
            "authority captured for one receipt prepared a branch opened on another"
        );
        assert_eq!(f.source.snapshot().unwrap(), before);

        // Positive control: the second branch's own authority prepares and signs it.
        let mut own = signing(&mut f, &second, &ledger);
        assert_eq!(own.remaining(), 2);
        assert!(own.sign_next(&f.owner, &f.group, 0).unwrap());
    }
}

/// SIGN-TEST-001 (core signing review). Pre-sign typed admission refuses an over-cap branch.
///
/// `local_policy` in `PreparedOverlayChanges::prepare` is the handoff path's only editor-cap check
/// before signing: `finish` validates typed changes as ingest does, which accepts over-cap content
/// as deterministic overflow. An honestly appended branch never reaches it over the cap, because
/// `append` replays under the same policy against the same base. A structurally decoded one can:
/// the structural decoder checks bounds, scope, ledger membership, sequence, author, envelope and
/// canonical encoding, and replays nothing. Production's only caller, H2, uses the replaying
/// decoder, which also refuses this branch, so here the check is defence in depth for any caller
/// that does not.
///
/// So the branch is built honestly to exactly the Index cap of `MAX_INDEX_OBJECTS` (the base holds
/// one object, the branch adds 63), plus one title edit, and that last entry is then retargeted in
/// the record's bytes at one more `PutObject`, exactly as the C-1 decoder test retargets one.
/// Preparation must refuse with `EpochBound` before any signature. The unspliced record is the
/// positive control.
#[test]
fn studio_handoff_preparation_refuses_a_vault_decoded_branch_over_the_local_cap() {
    let mut f = Fixture::new(false);
    f.fill();
    let decision = f.decide(None);
    let plan = f.plan(&decision);
    let basis = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());
    let mut state = StudioOverlayState::new(&basis);
    let put = |f: &mut Fixture, n: u8| {
        f.domain(
            IndexOp::PutObject {
                object: [n; 16],
                kind: StudioKind::Flipnote,
                title: format!("object {n}"),
                created_by: f.owner.device_id(),
                ts: 100,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
        )
    };
    // Object ids 2..=64; the base's own object is [1; 16].
    for n in 2..=64u8 {
        let op = put(&mut f, n);
        let id = ledger.prepare(f.owner.device_id(), op).unwrap();
        state.append(&basis, &ledger, id, 200).unwrap();
    }
    let edit = f.domain(f.title_body("the last honest entry"));
    let edit_id = ledger.prepare(f.owner.device_id(), edit.clone()).unwrap();
    state.append(&basis, &ledger, edit_id, 201).unwrap();
    let over = put(&mut f, 65);
    let over_id = ledger.prepare(f.owner.device_id(), over.clone()).unwrap();
    assert!(
        matches!(
            state.clone().append(&basis, &ledger, over_id, 202),
            Err(ReplError::EpochBound)
        ),
        "precondition: the 65th object is over the cap on the ordinary path"
    );
    f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();

    // Retarget the last entry (the title edit) at the over-cap PutObject: id, then envelope.
    let canonical = state.encode_vault(&ledger).unwrap();
    let at = canonical
        .windows(32)
        .position(|w| w == edit_id)
        .expect("the title edit's entry id is in the record");
    assert_eq!(
        canonical.windows(32).filter(|w| *w == edit_id).count(),
        1,
        "the entry id must be unambiguous in the record"
    );
    let envelope = |op: &DomainOp| {
        let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-overlay-envelope/v1");
        hash.update(f.owner.device_id().as_bytes());
        hash.update(&op.encode().unwrap());
        *hash.finalize().as_bytes()
    };
    assert_eq!(
        &canonical[at + 36..at + 68],
        &envelope(&edit),
        "entry layout: the envelope follows the id after its length prefix"
    );
    let mut spliced = canonical.clone();
    spliced[at..at + 32].copy_from_slice(&over_id);
    spliced[at + 36..at + 68].copy_from_slice(&envelope(&over));
    let decoded = StudioOverlayState::decode_vault_structural(&spliced, &ledger)
        .expect("a structurally consistent record decodes without replay");
    assert!(
        matches!(
            StudioOverlayState::decode_vault(&spliced, &ledger),
            Err(ReplError::EpochBound)
        ),
        "the replaying decoder refuses it; only the structural route reaches the signer"
    );

    let before = f.source.snapshot().unwrap();
    let authority = decoded.handoff_authority(&f.owner, &f.group, 0).unwrap();
    let source = f.source.copy_handoff_source(&f.group).unwrap();
    assert!(
        matches!(
            decoded.prepare_handoff_detached(source, ledger.clone(), authority),
            Err(ReplError::EpochBound)
        ),
        "an over-cap branch reached signing past the local editor cap"
    );
    assert_eq!(f.source.snapshot().unwrap(), before);

    // Positive control: the honest record, decoded the same way, prepares in full.
    let honest = StudioOverlayState::decode_vault_structural(&canonical, &ledger).unwrap();
    assert_eq!(signing(&mut f, &honest, &ledger).remaining(), 64);
}

/// SIGN-TEST-001, the aggregate half. Pre-sign gate admission refuses an honest branch whose signed
/// form would not fit the successor epoch, before any signature.
///
/// Reachable by valid input, which is why it needs a test of its own: a title may be up to
/// `MAX_DOMAIN_OP_BYTES`, and the per-document intent budget is `MAX_INTENT_BYTES_PER_DOCUMENT`,
/// so dozens of near-maximal title edits append honestly. Nothing on the append path consults
/// the epoch gate, yet each signed form carries the title twice (domain op and Automerge change),
/// so together they exceed `MAX_EPOCH_BYTES` in the pristine successor. The titles are
/// incompressible so the Automerge delta cannot shrink below the gate.
///
/// The refusal must come from `prepare_handoff_detached` itself. Without the probe the batch would
/// sign every operation and only then be refused by `finish`'s own gate, having spent every turn.
///
/// Assembled as the app's 256-operation fixture assembles its branch, because appending this many
/// near-maximal operations one at a time replays the whole growing branch on every append, which
/// takes many minutes in a debug build. Each entry comes from a one-operation typed append; the
/// entries are concatenated with consecutive sequences; and the full replaying decoder, which
/// applies the same per-operation policy `append` does, must accept the result. The last
/// operation then goes through the real `StudioOverlayState::append`, so the record is the v2
/// form production writes and the branch is checked by one more complete replay.
#[test]
fn studio_handoff_preparation_refuses_an_honest_branch_over_the_successor_gate() {
    const OPS: usize = 48;
    let mut f = Fixture::new(false);
    f.fill();
    let decision = f.decide(None);
    let plan = f.plan(&decision);
    let basis = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());
    let mut total = 0usize;
    let mut ids = Vec::new();
    for n in 0..OPS as u64 {
        // Printable, deterministic and incompressible enough that DEFLATE gains little. 60 KiB
        // rather than the full 64: JSON escapes `"` and `\`, which the domain-op bound counts.
        let mut stream = blake3::Hasher::new()
            .update(&n.to_be_bytes())
            .finalize_xof();
        let mut raw = vec![0u8; 60 * 1024];
        stream.fill(&mut raw);
        let title: String = raw.iter().map(|b| char::from(0x21 + b % 94)).collect();
        let op = f.domain(f.title_body(&title));
        assert!(op.encode().unwrap().len() <= crate::epoch::MAX_DOMAIN_OP_BYTES);
        total += op.encode().unwrap().len();
        ids.push(ledger.prepare(f.owner.device_id(), op).unwrap());
    }
    assert!(
        total <= crate::epoch::MAX_INTENT_BYTES_PER_DOCUMENT,
        "precondition: the branch fits the intent budget, so it is valid local work"
    );
    // Entry layout as the C-1 test and the app fixture use it: the last 88 bytes of a one-entry
    // record, with the sequence at 72..80; the prefix ends with the next sequence and the count.
    let mut prefix = Vec::new();
    let mut entries = Vec::new();
    for (n, id) in ids[..OPS - 1].iter().enumerate() {
        let mut single = StudioOverlay::new(&basis);
        single.append(&basis, &ledger, *id, 400 + n as u64).unwrap();
        let bytes = single.encode_vault(&ledger).unwrap();
        let split = bytes.len() - 88;
        if prefix.is_empty() {
            prefix.extend_from_slice(&bytes[..split]);
        }
        let mut entry = bytes[split..].to_vec();
        entry[72..80].copy_from_slice(&(n as u64 + 1).to_be_bytes());
        entries.extend_from_slice(&entry);
    }
    let end = prefix.len();
    prefix[end - 12..end - 4].copy_from_slice(&(OPS as u64).to_be_bytes());
    prefix[end - 4..].copy_from_slice(&((OPS - 1) as u32).to_be_bytes());
    prefix.extend_from_slice(&entries);
    let mut state = StudioOverlayState::decode_vault(&prefix, &ledger)
        .expect("the replaying decoder accepts every operation under the append policy");
    state
        .append(&basis, &ledger, ids[OPS - 1], 400 + OPS as u64)
        .unwrap();
    assert_eq!(state.overlay().unwrap().accepted(), OPS);
    f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
    let before = f.source.snapshot().unwrap();
    let authority = state.handoff_authority(&f.owner, &f.group, 0).unwrap();
    let source = f.source.copy_handoff_source(&f.group).unwrap();
    assert!(
        matches!(
            state
                .clone()
                .prepare_handoff_detached(source, ledger.clone(), authority),
            Err(ReplError::EpochBound)
        ),
        "a branch over the successor's epoch budget reached signing"
    );
    assert_eq!(f.source.snapshot().unwrap(), before);
}

/// C-1. The structural decoder must accept exactly the records the full decoder accepts, produce
/// the identical state, and keep every check except the ordered typed reconstruction.
#[test]
fn studio_overlay_structural_decode_matches_full_decode_and_keeps_its_entry_checks() {
    for art in [false, true] {
        for count in [1usize, 4] {
            let mut f = Fixture::new(art);
            let (metadata, ledger, _) = branch(&mut f, count);
            let encoded = metadata.encode_vault(&ledger).unwrap();

            let full = StudioOverlayState::decode_vault(&encoded, &ledger).unwrap();
            let structural =
                StudioOverlayState::decode_vault_structural(&encoded, &ledger).unwrap();
            // Canonical re-encoding is the state's complete observable content, so equal bytes
            // from both decoders establish that nothing but the replay was skipped.
            assert_eq!(
                structural.encode_vault(&ledger).unwrap(),
                full.encode_vault(&ledger).unwrap(),
                "structural decode produced a different state"
            );
            assert_eq!(structural.encode_vault(&ledger).unwrap(), encoded);
            assert_eq!(structural.target(), full.target());
            assert_eq!(structural.is_prepared(), full.is_prepared());
            assert_eq!(structural.has_completed(), full.has_completed());
            assert_eq!(
                structural.minimum_new_basis_closed_epoch(),
                full.minimum_new_basis_closed_epoch()
            );
            let (a, b) = (structural.overlay().unwrap(), full.overlay().unwrap());
            assert_eq!(
                (a.basis(), a.author(), a.target()),
                (b.basis(), b.author(), b.target())
            );
            // The structural result is still a complete branch: its projection is available on
            // demand, it is simply not computed during decoding.
            assert_eq!(
                a.read(&ledger).unwrap().projection(),
                b.read(&ledger).unwrap().projection()
            );

            // A ledger missing one annotated entry must be refused by BOTH decoders. This is the
            // predicate M8 weakens, and it is the reason `checked_entries` stays in the
            // structural path rather than being left to the ordered replay.
            let mut short = IntentLedger::new(ledger.document().clone());
            for (_, intent) in ledger.pending().take(count - 1) {
                short
                    .prepare(intent.author, intent.operation.clone())
                    .unwrap();
            }
            assert!(
                StudioOverlayState::decode_vault_structural(&encoded, &short).is_err(),
                "structural decode accepted a branch whose entry is absent from the ledger"
            );
            assert!(StudioOverlayState::decode_vault(&encoded, &short).is_err());

            // Trailing bytes and a truncated record are refused without replay.
            let mut trailing = encoded.clone();
            trailing.push(0);
            assert!(StudioOverlayState::decode_vault_structural(&trailing, &ledger).is_err());
            assert!(StudioOverlayState::decode_vault_structural(
                &encoded[..encoded.len() - 1],
                &ledger
            )
            .is_err());
        }
    }
}

/// C-1. Patch only the acceptance sequence of the single retained entry, leaving its id, envelope,
/// author, count and every other field untouched. `checked_entries` is the sole guard that rejects
/// it, so this isolates the predicate that must run in the structural path.
#[test]
fn studio_overlay_structural_decode_rejects_a_wrong_acceptance_sequence() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 1);
        // Patch the BRANCH encoding, not the enclosing version-2 state record: the state's
        // trailing bytes are its completed-acknowledgement fields, so patching there would be
        // caught by an unrelated check and the fixture would prove nothing.
        let encoded = metadata.overlay().unwrap().encode_vault(&ledger).unwrap();
        // One entry occupies the last 88 bytes, with its big-endian sequence at offset 72.
        // The same layout the accepted 256-operation fixture relies on.
        let entry = encoded.len() - 88;
        assert!(crate::studio::StudioOverlay::decode_vault_structural(&encoded, &ledger).is_ok());
        let mut patched = encoded.clone();
        assert_eq!(
            u64::from_be_bytes(patched[entry + 72..entry + 80].try_into().unwrap()),
            1,
            "the fixture must be patching the accepted entry's sequence field"
        );
        patched[entry + 72..entry + 80].copy_from_slice(&2u64.to_be_bytes());
        assert_eq!(
            patched.len(),
            encoded.len(),
            "only the sequence value may change"
        );
        assert!(
            crate::studio::StudioOverlay::decode_vault_structural(&patched, &ledger).is_err(),
            "structural decode accepted sequence 2 for the first accepted entry"
        );
        assert!(crate::studio::StudioOverlay::decode_vault(&patched, &ledger).is_err());
    }
}

/// C1-TEST-002 / N24. The behavioural claim of the structural decoder is that it does NOT replay.
/// Every branch built through `append` is replayable by construction, so those fixtures cannot
/// distinguish "skips the replay" from "replays and happens to succeed". This one can.
///
/// The branch names an operation the ordered replay refuses: a sound-effect change, which the
/// typed writer does not support yet. `checked_entries` never decodes an operation body, so the
/// record is fully consistent structurally: the entry is in the ledger, authored by the basis
/// author, with the exact envelope hash, sequence 1 and canonical re-encoding. Only the typed
/// reconstruction rejects it.
#[test]
fn studio_overlay_structural_decode_accepts_a_branch_the_full_decoder_cannot_replay() {
    let mut f = Fixture::new(true);
    f.fill();
    let decision = f.decide(None);
    // Seal and prepare settlement, so the source is actually Closing and can mint a basis.
    let _plan = f.plan(&decision);
    let basis = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());

    // A supported operation, used only to obtain one canonical accepted entry.
    let supported = f.domain(f.title_body("a replayable title"));
    let supported_id = ledger.prepare(f.owner.device_id(), supported).unwrap();
    // An operation the typed writer does not support. It decodes as a FlipnoteOp, so it reaches
    // the writer, and the writer refuses it.
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

    // Retarget the single entry at the unsupported intent. An entry is 88 bytes: a length-prefixed
    // id at 4..36, a length-prefixed envelope at 40..72, then sequence and timestamp. Only those
    // two fixed-width fields change, so the record stays canonical.
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
    assert_eq!(spliced.len(), canonical.len());

    // Structural decoding accepts it: bounds, scope, ledger membership, sequence, author,
    // envelope and canonical re-encoding all hold.
    let structural = StudioOverlay::decode_vault_structural(&spliced, &ledger)
        .expect("a canonical, structurally consistent branch must decode structurally");
    assert_eq!(structural.encode_vault(&ledger).unwrap(), spliced);
    assert_eq!(structural.basis(), basis.fingerprint());

    // The ordered typed reconstruction refuses it, and so does the full decoder. This is the
    // boundary C-1 moves, stated as a test rather than as an argument: a decoder that replayed
    // unconditionally could not have returned the value above.
    assert!(
        structural.read(&ledger).is_err(),
        "an unsupported operation must refuse in ordered replay"
    );
    assert!(
        StudioOverlay::decode_vault(&spliced, &ledger).is_err(),
        "the full decoder must refuse a branch it cannot replay"
    );
    // Guard the fixture: the unmodified canonical record replays fine, so the refusal above comes
    // from the retargeted entry rather than from anything else in the encoding.
    assert_eq!(
        StudioOverlay::decode_vault(&canonical, &ledger)
            .unwrap()
            .read(&ledger)
            .unwrap()
            .accepted(),
        1
    );
}
