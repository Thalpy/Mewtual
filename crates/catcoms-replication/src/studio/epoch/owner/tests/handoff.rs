use super::*;
use crate::IntentLedger;

fn branch(
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

fn signing(
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
