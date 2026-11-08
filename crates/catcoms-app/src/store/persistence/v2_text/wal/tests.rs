//! Strict plaintext/framing DATA only. Opaque head-frame fixtures below deliberately
//! establish NO head MAC, Current, Saved, verifier, deletion or signing authority.
use super::super::control::PreparedTextControlFrameData;
use super::super::records::{RawSignatureRecordData, TextIntentIdData};
use super::super::retirement::{RetirementEntryData, RetirementKindData};
use super::*;
use catcoms_replication::chat::ActionId;
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

struct SignatureWork {
    parent: Vec<u8>,
    candidate: Vec<u8>,
    signature: PreparedTextControlFrameData,
    carry: Zeroizing<Vec<u8>>,
}
fn signature_work() -> SignatureWork {
    let parent = vec![11; 64];
    let candidate = vec![12; 64];
    let mut roots = [
        TextHeadIdData::of_data(&parent),
        TextHeadIdData::from_bytes([1; 32]),
        TextHeadIdData::from_bytes([2; 32]),
    ];
    roots.sort();
    let carry = RetirementCarryData::from_data(&roots, &[])
        .unwrap()
        .encode_data()
        .unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(900);
    let signature = RawSignatureRecordData {
        context: TextContextIdData::from_bytes([1; 32]),
        transaction: TextTransactionIdData::from_bytes([2; 32]),
        intent: TextIntentIdData::from_bytes([3; 32]),
        pre_intent_head: TextHeadIdData::from_bytes([4; 32]),
        public_parent: OwnerHeaderId::from_bytes([5; 32]),
        unsigned_commitment: [6; 32],
        action: ActionId::from_bytes([7; 32]),
        raw: [8; 64],
    }
    .prepare_frame_data(&[9; 32], &mut rng)
    .unwrap();
    SignatureWork {
        parent,
        candidate,
        signature,
        carry,
    }
}
fn signature_wal<'a>(
    fixture: &'a SignatureWork,
    artifacts: &'a [TextWalArtifactData<'a>],
) -> TextWalFrameData<'a> {
    TextWalFrameData::Work(TextWorkWalFrameData {
        context: TextContextIdData::from_bytes([1; 32]),
        install_parent: TextHeadIdData::of_data(&fixture.parent),
        transaction: TextTransactionIdData::from_bytes([2; 32]),
        public_parent: OwnerHeaderId::from_bytes([5; 32]),
        operation: TextWorkWalOperationData::RawSignature,
        parent_frame: &fixture.parent,
        candidate: TextHeadIdData::of_data(&fixture.candidate),
        candidate_frame: &fixture.candidate,
        cleanup: TextWorkWalCleanupData::Carry(&fixture.carry),
        artifacts: TextWalArtifactsData::Borrowed(artifacts),
    })
}
fn artifacts(fixture: &SignatureWork) -> [TextWalArtifactData<'_>; 1] {
    [TextWalArtifactData::from_data(
        TextArtifactRoleData::Signature,
        *fixture.signature.id_data().as_bytes(),
        fixture.signature.frame_data(),
    )
    .unwrap()]
}

#[test]
fn work_codes_framing_and_borrowed_artifacts_round_trip_without_a_packed_copy() {
    let fixture = signature_work();
    let artifacts = artifacts(&fixture);
    let encoded = signature_wal(&fixture, &artifacts).encode_data().unwrap();
    let decoded = TextWalFrameData::decode_data(&encoded).unwrap();
    assert_eq!(decoded.encode_data().unwrap(), encoded);
    let TextWalFrameData::Work(decoded) = decoded else {
        panic!("WORK expected")
    };
    assert_eq!(decoded.operation, TextWorkWalOperationData::RawSignature);
    let entries: Vec<_> = decoded
        .artifacts
        .entries_data()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].role_data(), TextArtifactRoleData::Signature);
    assert_eq!(
        entries[0].id_data(),
        *fixture.signature.id_data().as_bytes()
    );
    assert_eq!(entries[0].frame_data(), fixture.signature.frame_data());
    let start = encoded.as_ptr() as usize;
    let end = start + encoded.len();
    let borrowed = entries[0].frame_data().as_ptr() as usize;
    assert!((start..end).contains(&borrowed));
    let mut raw = Decoder::new(&encoded);
    assert_eq!(raw.get_bytes().unwrap(), TAG);
    assert_eq!(raw.get_u16().unwrap(), 1);
    assert_eq!(raw.get_u8().unwrap(), 1); // WORK, not a legacy parser tag.
    for _ in 0..4 {
        raw.get_bytes().unwrap();
    }
    assert_eq!(raw.get_u8().unwrap(), 3); // S, closed operation byte.
}

#[test]
fn every_short_work_cut_unknown_code_extra_byte_and_hash_change_refuses() {
    let fixture = signature_work();
    let artifacts = artifacts(&fixture);
    let encoded = signature_wal(&fixture, &artifacts).encode_data().unwrap();
    for end in 0..encoded.len() {
        assert!(
            TextWalFrameData::decode_data(&encoded[..end]).is_err(),
            "cut={end}"
        );
    }
    let mut extra = encoded.to_vec();
    extra.push(0);
    assert!(TextWalFrameData::decode_data(&extra).is_err());
    let variant_at = 4 + TAG.len() + 2;
    for variant in [0, 4, 255] {
        let mut changed = encoded.to_vec();
        changed[variant_at] = variant;
        assert!(TextWalFrameData::decode_data(&changed).is_err());
    }
    let operation_at = variant_at + 1 + 4 * 36;
    for operation in [0, 5, 255] {
        let mut changed = encoded.to_vec();
        changed[operation_at] = operation;
        assert!(TextWalFrameData::decode_data(&changed).is_err());
    }
    let mut changed = encoded.to_vec();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    assert!(TextWalFrameData::decode_data(&changed).is_err());
    assert!(TextWalFrameData::decode_data(&vec![0; WAL_PLAIN_ALLOCATION_CAP + 1]).is_err());
}

#[test]
fn counts_role_kinds_cleanup_and_same_parent_candidate_never_synthesize_work() {
    let fixture = signature_work();
    let original = artifacts(&fixture);
    let duplicate = [original[0], original[0]];
    assert!(signature_wal(&fixture, &duplicate).encode_data().is_err());
    let mut value = signature_wal(&fixture, &original);
    let TextWalFrameData::Work(work) = &mut value else {
        unreachable!()
    };
    work.cleanup = TextWorkWalCleanupData::None;
    assert!(value.encode_data().is_err());
    let mut value = signature_wal(&fixture, &original);
    let TextWalFrameData::Work(work) = &mut value else {
        unreachable!()
    };
    work.candidate_frame = &fixture.parent;
    work.candidate = work.install_parent;
    assert!(value.encode_data().is_err());
    let mut value = signature_wal(&fixture, &original);
    let TextWalFrameData::Work(work) = &mut value else {
        unreachable!()
    };
    work.operation = TextWorkWalOperationData::Unsigned;
    assert!(value.encode_data().is_err());
    assert!(TextWalArtifactData::from_data(
        TextArtifactRoleData::Signature,
        [0; 32],
        fixture.signature.frame_data(),
    )
    .is_err());
    let mut rng = ChaCha20Rng::seed_from_u64(901);
    let page = PreparedPrivateChatPage::prepare(&[9; 32], b"bounded DATA page", &mut rng).unwrap();
    let entry = TextWalArtifactData::from_data(
        TextArtifactRoleData::TokenIndex,
        *page.id().as_bytes(),
        page.as_bytes(),
    )
    .unwrap();
    assert_eq!(entry.object_kind(), 1);
    assert!(TextWalArtifactData::from_data(
        TextArtifactRoleData::Signature,
        *page.id().as_bytes(),
        page.as_bytes()
    )
    .is_err());
}

#[test]
fn wd_wc_bind_exact_plan_selected_frame_and_reclaimed_totals_without_deletion_authority() {
    let context = TextContextIdData::from_bytes([1; 32]);
    let transaction = TextTransactionIdData::from_bytes([2; 32]);
    let selected_frame = [21; 64];
    let selected = TextHeadIdData::of_data(&selected_frame);
    let carry = RetirementCarryData::from_data(
        &[
            TextHeadIdData::from_bytes([1; 32]),
            TextHeadIdData::from_bytes([2; 32]),
            TextHeadIdData::from_bytes([3; 32]),
            TextHeadIdData::from_bytes([4; 32]),
        ],
        &[
            RetirementEntryData {
                kind: RetirementKindData::TokenIndex,
                id: [3; 32],
                frame_bytes: 87,
            },
            RetirementEntryData {
                kind: RetirementKindData::Intent,
                id: [4; 32],
                frame_bytes: 41,
            },
            RetirementEntryData {
                kind: RetirementKindData::Signature,
                id: [5; 32],
                frame_bytes: 405,
            },
        ],
    )
    .unwrap();
    let reclaimed = carry.totals_data().unwrap();
    let plan = RetirementPlanData::from_data(context, transaction, selected, carry).unwrap();
    let raw_plan = plan.encode_data().unwrap();
    let plan_commit = TextRetirementPlanIdData::of_data(&raw_plan);
    let completion = TextWalFrameData::Completion(TextCompletionWalFrameData {
        context,
        transaction,
        selected,
        plan_commit,
        public_header: OwnerHeaderId::from_bytes([8; 32]),
        reclaimed,
        plan: &raw_plan,
    });
    let encoded_wc = completion.encode_data().unwrap();
    assert_eq!(
        TextWalFrameData::decode_data(&encoded_wc)
            .unwrap()
            .encode_data()
            .unwrap(),
        encoded_wc
    );
    let mut rng = ChaCha20Rng::seed_from_u64(902);
    let wc = PreparedTextControlFrameData::prepare_data(
        TextControlKindData::Wal,
        &[9; 32],
        &encoded_wc,
        &mut rng,
    )
    .unwrap();
    let deletion = TextWalFrameData::Deletion(TextDeletionWalFrameData {
        context,
        transaction,
        selected,
        plan_commit,
        plan: &raw_plan,
        selected_frame: &selected_frame,
        completion_frame: wc.frame_data(),
    });
    let encoded_wd = deletion.encode_data().unwrap();
    assert_eq!(
        TextWalFrameData::decode_data(&encoded_wd)
            .unwrap()
            .encode_data()
            .unwrap(),
        encoded_wd
    );
    let mut wrong = TextWalFrameData::Completion(TextCompletionWalFrameData {
        context,
        transaction,
        selected,
        plan_commit,
        public_header: OwnerHeaderId::from_bytes([8; 32]),
        reclaimed: RetiredTotalsData {
            bytes: reclaimed.bytes + 1,
            objects: reclaimed.objects,
        },
        plan: &raw_plan,
    });
    assert!(wrong.encode_data().is_err());
    let TextWalFrameData::Completion(ref mut wrong_wc) = wrong else {
        unreachable!()
    };
    wrong_wc.reclaimed = reclaimed;
    wrong_wc.plan_commit = TextRetirementPlanIdData::from_bytes([0; 32]);
    assert!(wrong.encode_data().is_err());
    for end in 0..encoded_wd.len() {
        assert!(TextWalFrameData::decode_data(&encoded_wd[..end]).is_err());
    }
    for end in 0..encoded_wc.len() {
        assert!(TextWalFrameData::decode_data(&encoded_wc[..end]).is_err());
    }
}
