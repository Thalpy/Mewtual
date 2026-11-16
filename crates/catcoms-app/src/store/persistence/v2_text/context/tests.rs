//! Inert label/grammar controls only; no fixture here manufactures Saved/Current authority.
use super::*;

fn context(group: &[u8]) -> TextContextData {
    TextContextData::from_data(TextContextSpecData {
        layout: CurrentLayoutData {
            vault: [1; 32],
            root: [2; 32],
        },
        marker: TargetMarkerId::from_bytes([3; 32]),
        terminal: IntentId::from_bytes([4; 32]),
        source: BootstrapSourceId::from_bytes([5; 32]),
        source_context_digest: [6; 32],
        profile: OwnerFactId::from_bytes([7; 32]),
        admission: OwnerBodyId::from_bytes([8; 32]),
        signer: [9; 32],
        nonce: [10; 32],
        pin: V2GroupPinId::from_bytes([11; 32]),
        tenure: OwnerTenureId::from_bytes([12; 32]),
        genesis_parent: TextGenesisParentIdData::from_bytes([13; 32]),
        group,
    })
    .unwrap()
}

#[test]
fn stable_context_roundtrip_and_exact_maximum() {
    let c = context(&[14; 256]);
    let encoded = c.encode_data().unwrap();
    assert_eq!(encoded.len(), MAX_CONTEXT_PLAIN);
    let reopened = TextContextData::decode_data(&encoded).unwrap();
    assert!(reopened == c);
    assert!(reopened.id_data().unwrap() == c.id_data().unwrap());
    assert!(!format!("{c:?}").contains("14"));
    let mut changed = reopened.spec_data();
    changed.nonce[0] ^= 1;
    let changed = TextContextData::from_data(changed).unwrap();
    assert!(changed.id_data().unwrap() != c.id_data().unwrap());
}

#[test]
fn strict_context_outer_cap_lengths_prefixes_and_profile() {
    let c = context(b"inert group");
    let bytes = c.encode_data().unwrap();
    for n in 0..bytes.len() {
        assert!(TextContextData::decode_data(&bytes[..n]).is_err());
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(TextContextData::decode_data(&trailing).is_err());
    assert!(TextContextData::decode_data(&vec![0; MAX_CONTEXT_PLAIN + 1]).is_err());
    let mut declared = bytes.to_vec();
    let group_len = 4 + TAG.len() + 2 + 13 * 36;
    declared[group_len..group_len + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(TextContextData::decode_data(&declared).is_err());
    let mut revision = bytes.to_vec();
    let revision_offset = revision.len() - 16;
    revision[revision_offset..revision_offset + 8].copy_from_slice(&1u64.to_be_bytes());
    assert!(TextContextData::decode_data(&revision).is_err());
    let mut epoch = bytes.to_vec();
    let epoch_offset = epoch.len() - 8;
    epoch[epoch_offset..].copy_from_slice(&1u64.to_be_bytes());
    assert!(TextContextData::decode_data(&epoch).is_err());
}
