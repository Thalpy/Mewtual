//! Genuine public profile/Admission/header signatures; no actual selected source or Current.
use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::acceptance_tree::TreeContext;
use catcoms_replication::chat::owner_header::HeaderSpec;
use catcoms_replication::chat::owner_profile::{
    AdmissionBody, AdmissionOrigin, AdmissionSpec, ProfileSpec, SignedOwnerProfile,
};
use catcoms_replication::chat::owner_state::{
    encode_state, virtual_genesis_state, CurrentAdmissionSlot, StateMeta,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
fn genesis() -> TextArchiveData {
    let key = DeviceKeypair::from_seed(&[81; 32]);
    let group = [82; 256];
    let profile = SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: group.to_vec(),
            founder: key.verifying_key().to_bytes(),
            creation: [83; 32],
        },
        |b| Ok(key.sign(b)),
    )
    .unwrap();
    let body = AdmissionBody::new(AdmissionSpec {
        group: group.to_vec(),
        profile: profile.id(),
        subject: profile.spec().founder,
        origin: AdmissionOrigin::Founder {
            creation: profile.spec().creation,
        },
    })
    .unwrap();
    let image = encode_state(
        StateMeta {
            group: &group,
            profile: profile.id(),
            issuer: profile.spec().founder,
            tenure: profile.tenure(),
            parent: profile.genesis_parent(),
            previous: virtual_genesis_state(&profile).unwrap(),
            sequence: 1,
            kind: HeaderKind::Admission,
            body: body.id(),
            mls_epoch: 0,
        },
        &[CurrentAdmissionSlot {
            admission: body.id(),
            subject: profile.spec().founder,
            tip_body: body.id(),
            accepted_count: 0,
            accepted_root: TreeContext {
                profile: profile.id(),
                admission: body.admission_id(),
            }
            .empty_hash(),
        }],
    )
    .unwrap();
    let state = StateFrame::decode(&image).unwrap();
    let m = state.meta();
    let header = SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: m.group.to_vec(),
            profile: m.profile,
            issuer: m.issuer,
            tenure: m.tenure,
            parent: m.parent,
            sequence: m.sequence,
            kind: m.kind,
            body: m.body,
            state: state.commitment(),
        },
        |b| Ok(key.sign(b)),
    )
    .unwrap();
    TextArchiveData::from_data(TextArchiveSpecData {
        context: TextContextIdData::from_bytes([84; 32]),
        manifest: PrivateChatPageId::from_bytes([85; 32]),
        manifest_sequence: 1,
        header: &header,
        image: &image,
        previous: None,
        operation: TextArchiveOperationData::Genesis,
    })
    .unwrap()
}
#[test]
fn genuine_genesis_retention_successor_hits_exact_maximum_without_private_root_cycle() {
    let old = genesis();
    let mut rng = ChaCha20Rng::seed_from_u64(86);
    let sealed = old.prepare_frame_data(&[87; 32], &mut rng).unwrap();
    let old_id = ArchiveHeadId::from_bytes(*sealed.id_data().as_bytes());
    let retain = TextArchiveData::from_data(TextArchiveSpecData {
        previous: Some(old_id),
        manifest: PrivateChatPageId::from_bytes([88; 32]),
        manifest_sequence: 2,
        operation: TextArchiveOperationData::RetainAction,
        ..old.spec_data()
    })
    .unwrap();
    retain.require_successor_data(&old, old_id).unwrap();
    let raw = retain.encode_data().unwrap();
    assert_eq!(raw.len(), MAX_ARCHIVE_PLAIN);
    let parsed = TextArchiveData::decode_data(&raw).unwrap();
    assert!(parsed.encode_data().unwrap() == raw);
    parsed.require_successor_data(&old, old_id).unwrap();
    assert!(retain
        .require_successor_data(&old, ArchiveHeadId::from_bytes([89; 32]))
        .is_err());
    let bad = TextArchiveData::from_data(TextArchiveSpecData {
        manifest_sequence: 3,
        ..retain.spec_data()
    })
    .unwrap();
    assert!(bad.require_successor_data(&old, old_id).is_err());
    assert_eq!(format!("{retain:?}"), "TextArchiveData { .. }");
}
#[test]
fn strict_framing_and_invalid_same_header_id_signature_are_rejected() {
    let old = genesis();
    let raw = old.encode_data().unwrap();
    for n in 0..raw.len() {
        assert!(TextArchiveData::decode_data(&raw[..n]).is_err());
    }
    let mut bad = raw.to_vec();
    bad.push(0);
    assert!(TextArchiveData::decode_data(&bad).is_err());
    assert!(TextArchiveData::decode_data(&vec![0; MAX_ARCHIVE_PLAIN + 1]).is_err());
    let mut header = old.header.encode();
    let n = header.len();
    header[n - 1] ^= 1;
    let header = SignedOwnerHeader::decode(&header).unwrap();
    assert_eq!(header.id(), old.header.id());
    assert!(!header.verify());
    assert!(TextArchiveData::from_data(TextArchiveSpecData {
        header: &header,
        ..old.spec_data()
    })
    .is_err());
    assert!(TextArchiveData::from_data(TextArchiveSpecData {
        operation: TextArchiveOperationData::Append,
        ..old.spec_data()
    })
    .is_err());
    let mut bad = raw.to_vec();
    let field = 4 + TAG.len() + 2 + 2 * 36 + 8;
    bad[field..field + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(TextArchiveData::decode_data(&bad).is_err());
}
