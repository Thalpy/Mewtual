//! Inert fixed records + real AEAD; no signature, Current or publication permission is forged.
use super::super::control::open_control_data;
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
#[test]
fn exact_signature_record_and_immutable_cipher_candidate() {
    let value = RawSignatureRecordData {
        context: TextContextIdData::from_bytes([1; 32]),
        transaction: TextTransactionIdData::from_bytes([2; 32]),
        intent: TextIntentIdData::from_bytes([3; 32]),
        pre_intent_head: TextHeadIdData::from_bytes([4; 32]),
        public_parent: OwnerHeaderId::from_bytes([5; 32]),
        unsigned_commitment: [6; 32],
        action: ActionId::from_bytes([7; 32]),
        raw: [8; 64],
    };
    let raw = value.encode_data().unwrap();
    assert_eq!(raw.len(), 365);
    assert!(RawSignatureRecordData::decode_data(&raw).unwrap() == value);
    for n in 0..raw.len() {
        assert!(RawSignatureRecordData::decode_data(&raw[..n]).is_err());
    }
    let mut bad = raw.to_vec();
    bad.push(0);
    assert!(RawSignatureRecordData::decode_data(&bad).is_err());
    let mut bad = raw.to_vec();
    bad[raw.len() - 68..raw.len() - 64].copy_from_slice(&63u32.to_be_bytes());
    assert!(RawSignatureRecordData::decode_data(&bad).is_err());
    let mut rng = ChaCha20Rng::seed_from_u64(61);
    let prepared = value.prepare_frame_data(&[62; 32], &mut rng).unwrap();
    assert_eq!(prepared.frame_data().len(), 405);
    assert!(
        RawSignatureRecordData::decode_data(
            &open_control_data(&[62; 32], prepared.id_data(), prepared.frame_data()).unwrap()
        )
        .unwrap()
            == value
    );
    assert!(
        TextRawSignatureIdData::of_data(prepared.frame_data()).as_bytes()
            == prepared.id_data().as_bytes()
    );
    assert!(PublicationRecordData::decode_data(&raw).is_err());
    assert_eq!(format!("{value:?}"), "RawSignatureRecordData { .. }");
}
#[test]
fn publication_record_has_no_selecting_operational_head_backedge() {
    let value = PublicationRecordData {
        context: TextContextIdData::from_bytes([1; 32]),
        transaction: TextTransactionIdData::from_bytes([2; 32]),
        retain: TextRetainIdData::from_bytes([3; 32]),
        public_header: OwnerHeaderId::from_bytes([4; 32]),
        receipt: ReceiptDigestData::from_bytes([5; 32]),
        archive: ArchiveHeadId::from_bytes([6; 32]),
        ordinal: u64::MAX,
    };
    let raw = value.encode_data().unwrap();
    assert_eq!(raw.len(), 271);
    assert!(PublicationRecordData::decode_data(&raw).unwrap() == value);
    for n in 0..raw.len() {
        assert!(PublicationRecordData::decode_data(&raw[..n]).is_err());
    }
    let mut bad = raw.to_vec();
    bad.push(0);
    assert!(PublicationRecordData::decode_data(&bad).is_err());
    assert!(PublicationRecordData::decode_data(&vec![0; MAX_PUBLICATION_PLAIN + 1]).is_err());
    let mut rng = ChaCha20Rng::seed_from_u64(63);
    let prepared = value.prepare_frame_data(&[64; 32], &mut rng).unwrap();
    assert_eq!(prepared.frame_data().len(), 311);
    assert!(
        PublicationRecordData::decode_data(
            &open_control_data(&[64; 32], prepared.id_data(), prepared.frame_data()).unwrap()
        )
        .unwrap()
            == value
    );
    assert!(RawSignatureRecordData::decode_data(&raw).is_err());
    assert_eq!(format!("{value:?}"), "PublicationRecordData { .. }");
}
