//! Semantic mutation of a REAL first-A WAL, not a manufactured Ready/Saved receipt.
use super::*;
use crate::store::persistence::v2_text::{
    RetirementCarryData, RetirementEntryData, RetirementKindData, TextWalFrameData,
    TextWorkWalCleanupData,
};

#[test]
fn genuine_first_a_wal_rejects_unrelated_token_retirement_despite_valid_outer_shape_and_mac() {
    let mut f = retain_fixture(0, 0x7fa40);
    // Capture the genuine source BEFORE A. First-A has a proven Empty token root,
    // so there cannot be any replaced old token page, even if outer WORK permits ten.
    let parent = selected_capture(&f.original);
    let admitted = stage_admission(&f);
    f.transaction.run_stage(admitted).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let wal_path = owner
        .source()
        .source()
        .original_target_data()
        .0
        .join("text.wal");
    let original = std::fs::read(&wal_path).unwrap();
    let key = store.keys.db_key().unwrap();
    let derived = blake3::derive_key("catcoms/private-chat-text-wal-key/v1", &key);
    let plain =
        catcoms_crypto::unseal(&derived, &crate::store::unframe(&original).unwrap()).unwrap();
    let TextWalFrameData::Work(mut wal) = TextWalFrameData::decode_data(&plain).unwrap() else {
        panic!("genuine Stage produced the actual A WORK envelope");
    };
    wal.verify_first_retained_a_data(&key, parent.selected_data().unwrap())
        .unwrap();
    let unrelated = RetirementCarryData::from_data(
        &[wal.install_parent],
        &[RetirementEntryData {
            kind: RetirementKindData::TokenIndex,
            id: [0x6b; 32],
            frame_bytes: 87,
        }],
    )
    .unwrap()
    .encode_data()
    .unwrap();
    wal.cleanup = TextWorkWalCleanupData::Carry(&unrelated);
    // It is deliberately VALID generic outer WORK geometry. This test must fail at
    // first-A semantics, rather than accidentally relying on a length/order/MAC error.
    let hostile_plain = TextWalFrameData::Work(wal).encode_data().unwrap();
    let hostile = crate::store::frame(
        &catcoms_crypto::seal(
            &derived,
            &hostile_plain,
            &mut ChaCha20Rng::seed_from_u64(0x7fa41),
        )
        .unwrap(),
    );
    let authenticated =
        catcoms_crypto::unseal(&derived, &crate::store::unframe(&hostile).unwrap()).unwrap();
    let TextWalFrameData::Work(wal) = TextWalFrameData::decode_data(&authenticated).unwrap() else {
        panic!("same supported authenticated WORK variant");
    };
    assert!(wal
        .verify_first_retained_a_data(&key, parent.selected_data().unwrap())
        .is_err());
    assert_eq!(
        std::fs::read(wal_path).unwrap(),
        original,
        "the original WAL is preserved"
    );
    assert!(
        !f.transaction.has_saved_data(),
        "parsing/authenticating a WAL never installs A"
    );
}
