use super::*;
use catcoms_mls::MlsDevice;
use catcoms_wire::DocType;

fn legacy(delta: Vec<u8>) -> SignedOp {
    SignedOp::sign(&MlsDevice::generate().unwrap(), DocType::Channel, 7, delta).unwrap()
}

fn check_frame(op: &SignedOp) {
    let encoded = op.encode();
    let frame = SignedOpFrame::new(op).unwrap();
    assert_eq!(format!("{frame:?}"), "SignedOpFrame { .. }");
    assert_eq!(frame.encoded_len(), encoded.len());
    for chunk_size in [1, 7, 64 * 1024] {
        let mut reconstructed = Vec::new();
        let mut hash = blake3::Hasher::new();
        let mut offset = 0;
        while offset < frame.encoded_len() {
            let copied = frame.read_chunk(offset, chunk_size).unwrap();
            assert!(!copied.is_empty());
            let visited = frame
                .visit_chunk(offset, chunk_size, |part| {
                    hash.update(part);
                })
                .unwrap();
            assert_eq!(visited, copied.len());
            offset += copied.len();
            reconstructed.extend_from_slice(&copied);
        }
        assert_eq!(reconstructed, encoded);
        assert_eq!(hash.finalize().as_bytes(), &op.hash());
        assert!(frame.read_chunk(offset, chunk_size).unwrap().is_empty());
    }
    // Exact segments and their seams must agree even when a range starts/ends inside
    // a length prefix, signature or optional domain suffix.
    for offset in 0..encoded.len().min(300) {
        let copied = frame.read_chunk(offset, 11).unwrap();
        assert_eq!(copied, encoded[offset..encoded.len().min(offset + 11)]);
    }
}

#[test]
fn inspection_v1_frame_and_incremental_hash_match_original_bytes_at_every_chunk_boundary() {
    check_frame(&legacy(vec![0x5a; 128 * 1024 + 3]));
}

#[test]
fn inspection_domain_suffix_is_exact_without_enabling_its_distinct_verifier() {
    use automerge::transaction::Transactable;
    let device = MlsDevice::generate().unwrap();
    let mut doc = automerge::AutoCommit::new().with_actor(automerge::ActorId::from(
        device.device_id().as_bytes().to_vec(),
    ));
    doc.put(automerge::ROOT, "value", 1u64).unwrap();
    let delta = doc.get_last_local_change().unwrap().raw_bytes().to_vec();
    let domain = crate::epoch::DomainOp {
        nonce: [2; 16],
        doc_type: DocType::DocRegistry,
        logical_key: vec![1],
        body: b"{}".to_vec(),
    };
    let op = SignedOp::sign_domain(&device, DocType::DocRegistry, 9, delta, &domain).unwrap();
    assert!(op.verify());
    check_frame(&op);
    let frame = SignedOpFrame::new(&op).unwrap();
    assert_eq!(frame.signature_work_bytes(), 0);
    assert_eq!(
        frame.verify_signature(),
        InspectionSignature::NotChecked(InspectionSignatureLimit::UnsupportedDomain)
    );
}

#[test]
fn inspection_malformed_claimed_keys_remain_exactly_readable_and_are_never_verified() {
    let original = legacy(vec![1, 2, 3]);
    for length in [0, 1, 31, 33, 65_537] {
        let mut op = original.clone();
        op.author_pubkey = vec![0x7f; length];
        check_frame(&op);
        let frame = SignedOpFrame::new(&op).unwrap();
        assert_eq!(frame.signature_work_bytes(), 0);
        assert_eq!(frame.verify_signature(), InspectionSignature::Invalid);
    }
}

#[test]
fn inspection_signature_ceiling_is_an_explicit_result_without_erasing_large_originals() {
    let overhead = 4 + OP_DOMAIN.len() + 2 + 16 + 4 + 32 + 4;
    for extra in [0, 1] {
        let op = legacy(vec![
            0x42;
            MAX_INSPECTION_SIGNATURE_BYTES - overhead + extra
        ]);
        assert!(op.verify(), "the oversized original is genuinely signed");
        let frame = SignedOpFrame::new(&op).unwrap();
        if extra == 0 {
            assert_eq!(frame.signature_work_bytes(), MAX_INSPECTION_SIGNATURE_BYTES);
            assert_eq!(frame.verify_signature(), InspectionSignature::Valid);
        } else {
            assert_eq!(frame.signature_work_bytes(), 0);
            assert_eq!(
                frame.verify_signature(),
                InspectionSignature::NotChecked(InspectionSignatureLimit::TooLarge)
            );
        }
        assert_eq!(
            frame.read_chunk(64 * 1024, 64 * 1024).unwrap(),
            op.encode()[64 * 1024..128 * 1024]
        );
    }
}

#[test]
fn inspection_wrong_signature_or_device_identity_is_not_positive_attribution() {
    let original = legacy(b"retained body".to_vec());
    assert_eq!(
        SignedOpFrame::new(&original).unwrap().verify_signature(),
        InspectionSignature::Valid
    );
    let mut corrupt = original.clone();
    corrupt.signature[0] ^= 1;
    assert_eq!(
        SignedOpFrame::new(&corrupt).unwrap().verify_signature(),
        InspectionSignature::Invalid
    );
    let mut wrong_device = original;
    wrong_device.author_device = MlsDevice::generate().unwrap().device_id();
    assert_eq!(
        SignedOpFrame::new(&wrong_device)
            .unwrap()
            .verify_signature(),
        InspectionSignature::Invalid
    );
}

#[test]
fn inspection_invalid_ranges_refuse_before_visiting_and_exact_end_is_empty() {
    let op = legacy(vec![1, 2, 3]);
    let frame = SignedOpFrame::new(&op).unwrap();
    for (offset, limit, error) in [
        (0, 0, FrameReadError::ChunkLimit),
        (
            0,
            MAX_INSPECTION_FRAME_CHUNK + 1,
            FrameReadError::ChunkLimit,
        ),
        (frame.encoded_len() + 1, 1, FrameReadError::Offset),
        (usize::MAX, 1, FrameReadError::Offset),
    ] {
        assert_eq!(
            frame.visit_chunk(offset, limit, |_| panic!("invalid read visited bytes")),
            Err(error)
        );
        assert_eq!(frame.read_chunk(offset, limit), Err(error));
    }
    assert_eq!(
        frame.visit_chunk(frame.encoded_len(), 1, |_| panic!("end visited bytes")),
        Ok(0)
    );
}

#[test]
fn inspection_small_chunk_of_huge_original_never_calls_full_encoding_or_signing_preimage() {
    let mut op = legacy(vec![1]);
    op.author_pubkey = vec![0x7f; 8 * 1024 * 1024];
    op.delta = vec![0x42; 8 * 1024 * 1024];
    let frame = SignedOpFrame::new(&op).unwrap();
    take_observed_work();
    assert_eq!(
        frame
            .read_chunk(0, MAX_INSPECTION_FRAME_CHUNK)
            .unwrap()
            .len(),
        MAX_INSPECTION_FRAME_CHUNK
    );
    let mut visited = 0;
    frame
        .visit_chunk(8 * 1024 * 1024, 23, |part| visited += part.len())
        .unwrap();
    assert_eq!(visited, 23);
    assert_eq!(frame.verify_signature(), InspectionSignature::Invalid);
    let observed = take_observed_work();
    assert_eq!(observed.full_encodes, 0);
    assert_eq!(observed.preimage_bytes, 0);
}

#[test]
fn inspection_measured_verification_preimage_never_exceeds_the_fixed_work_cap() {
    let overhead = 4 + OP_DOMAIN.len() + 2 + 16 + 4 + 32 + 4;
    for extra in [0, 1] {
        let op = legacy(vec![
            0x42;
            MAX_INSPECTION_SIGNATURE_BYTES - overhead + extra
        ]);
        let frame = SignedOpFrame::new(&op).unwrap();
        take_observed_work();
        let status = frame.verify_signature();
        let observed = take_observed_work();
        assert_eq!(observed.full_encodes, 0);
        assert!(observed.preimage_bytes <= MAX_INSPECTION_SIGNATURE_BYTES);
        if extra == 0 {
            assert_eq!(status, InspectionSignature::Valid);
            assert_eq!(observed.preimage_bytes, MAX_INSPECTION_SIGNATURE_BYTES);
        } else {
            assert_eq!(
                status,
                InspectionSignature::NotChecked(InspectionSignatureLimit::TooLarge)
            );
            assert_eq!(observed.preimage_bytes, 0);
        }
    }
}
