//! Inert A/chunk grammar DATA: these labels create no actual signed action or SavedA.
use super::*;
fn retained() -> RetainedActionData {
    let actions = [
        TextChunkDescriptorData {
            page: PrivateChatPageId::from_bytes([20; 32]),
            plain_bytes: (459 + 61440) as u32,
        },
        TextChunkDescriptorData {
            page: PrivateChatPageId::from_bytes([21; 32]),
            plain_bytes: (459 + 4617) as u32,
        },
    ];
    let ciphers = [
        TextChunkDescriptorData {
            page: PrivateChatPageId::from_bytes([22; 32]),
            plain_bytes: (168 + 61440) as u32,
        },
        TextChunkDescriptorData {
            page: PrivateChatPageId::from_bytes([23; 32]),
            plain_bytes: (168 + 61440) as u32,
        },
        TextChunkDescriptorData {
            page: PrivateChatPageId::from_bytes([24; 32]),
            plain_bytes: (168 + 8236) as u32,
        },
    ];
    RetainedActionData::from_data(RetainedActionSpecData {
        context: TextContextIdData::from_bytes([1; 32]),
        transaction: TextTransactionIdData::from_bytes([2; 32]),
        token: [3; 32],
        request: TextRequestDigestData::from_bytes([4; 32]),
        action: ActionId::from_bytes([5; 32]),
        original: OriginalActionDigest::from_bytes([6; 32]),
        cipher: cipher_digest(&[7; 24], &vec![8; 131092]).unwrap(),
        action_bytes: 66057,
        cipher_bytes: 131116,
        channel: 9,
        actions: &actions,
        ciphers: &ciphers,
    })
    .unwrap()
}
#[test]
fn exact_retain_maximum_and_complete_prefix_length_descriptor_matrix() {
    let a = retained();
    let raw = a.encode_data().unwrap();
    assert_eq!(raw.len(), MAX_RETAIN_PLAIN);
    assert!(RetainedActionData::decode_data(&raw).unwrap() == a);
    assert_eq!(a.descriptors_data().0.len(), 2);
    assert_eq!(a.descriptors_data().1.len(), 3);
    for n in 0..raw.len() {
        assert!(RetainedActionData::decode_data(&raw[..n]).is_err());
    }
    let mut bad = raw.to_vec();
    bad.push(0);
    assert!(RetainedActionData::decode_data(&bad).is_err());
    assert!(RetainedActionData::decode_data(&vec![0; MAX_RETAIN_PLAIN + 1]).is_err());
    let base = 4 + RETAIN_TAG.len() + 2;
    let list = base + 7 * 36 + 8 + 16;
    for (at, value) in [
        (list, 0),
        (list, 3),
        (list + 1 + 2 * 40, 0),
        (list + 1 + 2 * 40, 4),
    ] {
        let mut bad = raw.to_vec();
        bad[at] = value;
        assert!(RetainedActionData::decode_data(&bad).is_err());
    }
    let mut bad = raw.to_vec();
    bad[base..base + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(RetainedActionData::decode_data(&bad).is_err());
    let mut bad = raw.to_vec();
    bad[list + 1 + 40..list + 1 + 40 + 36].copy_from_slice(&raw[list + 1..list + 1 + 36]);
    assert!(RetainedActionData::decode_data(&bad).is_err());
    let mut bad = raw.to_vec();
    bad[list + 1 + 2 * 40 + 1 + 36..list + 1 + 2 * 40 + 1 + 40]
        .copy_from_slice(&1u32.to_be_bytes());
    assert!(RetainedActionData::decode_data(&bad).is_err());
    assert_eq!(format!("{a:?}"), "RetainedActionData { .. }");
    assert!(RetainedActionData::frame_id_data(&[0; 40]).is_err());
    assert!(
        RetainedActionData::frame_id_data(&[0; 41]).unwrap() == TextRetainIdData::of_data(&[0; 41])
    );
}
#[test]
fn maximum_cipher_chunks_join_exact_original_and_reject_reordered_or_foreign() {
    let a = retained();
    let cipher = vec![8; 131092];
    let mut joined = Vec::new();
    let mut total_plain = 0;
    for index in 0..3 {
        let plain =
            CipherChunkData::encode_data(a.context, a.transaction, &[7; 24], &cipher, index)
                .unwrap();
        total_plain += plain.len();
        let chunk = CipherChunkData::decode_data(&plain).unwrap();
        chunk.require_descriptor_data(&a, index).unwrap();
        assert!(chunk.require_descriptor_data(&a, (index + 1) % 3).is_err());
        joined.extend_from_slice(chunk.payload_data());
        let mut foreign = a.clone();
        foreign.context = TextContextIdData::from_bytes([11; 32]);
        assert!(chunk.require_descriptor_data(&foreign, index).is_err());
        for n in [0, 4, plain.len() - 1] {
            assert!(CipherChunkData::decode_data(&plain[..n]).is_err());
        }
        let mut bad = plain.to_vec();
        bad.push(0);
        assert!(CipherChunkData::decode_data(&bad).is_err());
        let mut bad = plain.to_vec();
        bad[CIPHER_CHUNK_HEADER_BYTES - 4..CIPHER_CHUNK_HEADER_BYTES]
            .copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(CipherChunkData::decode_data(&bad).is_err());
    }
    assert_eq!(total_plain + 3 * 86, 131878);
    assert_eq!(&joined[..24], &[7; 24]);
    assert_eq!(&joined[24..], cipher);
    let mut canonical = Encoder::new();
    put(&mut canonical, &joined).unwrap();
    assert!(TextCipherOriginalDigestData::of_data(&canonical.finish()) == a.cipher);
    assert!(CipherChunkData::encode_data(a.context, a.transaction, &[7; 24], &cipher, 3).is_err());
    assert!(cipher_digest(&[7; 24], &[]).is_err());
    assert!(cipher_digest(&[7; 24], &vec![0; 131093]).is_err());
    assert!(CipherChunkData::decode_data(&vec![0; MAX_CIPHER_CHUNK_PLAIN + 1]).is_err());
}
