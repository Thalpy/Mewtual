//! Codec and physical-prefix controls, not native authorization evidence.
use super::*;
use catcoms_replication::chat::join_bindings::{GenesisAnchorBindingData, JoinPayloadKindData};

fn plain() -> Vec<u8> {
    let route = b"exact original public plan";
    let spec = InviteBindingSpecData {
        anchor: GenesisAnchorBindingData {
            group: vec![1; 32],
            founder: [2; 32],
            profile: [3; 32],
            admission: [4; 32],
            tenure: [5; 32],
            header: [6; 32],
            state: [7; 32],
        },
        inviter: [8; 32],
        nonce: [9; 16],
        expires_at_ms: 90_000,
        policy: [10; 32],
        public_routing_plan: JoinPayloadKindData::RoutingPlan
            .commitment_data(route)
            .unwrap(),
    };
    let mut e = Encoder::new();
    e.put_bytes(TAG).unwrap();
    e.put_u16(1);
    e.put_u32(OBJECTS);
    e.put_u32(BYTES);
    e.put_bytes(&[11; 32]).unwrap();
    e.put_bytes(&[12; 32]).unwrap();
    e.put_u64(60_000);
    e.put_bytes(route).unwrap();
    e.put_bytes(&spec.signing_bytes_data().unwrap()).unwrap();
    e.finish()
}

#[test]
fn unsigned_invite_journal_roundtrip_preserves_exact_preimage_and_rejects_changed_request() {
    let original = plain();
    let intent = decode(&original, [11; 32]).unwrap();
    let result = intent.result(intent.request, &intent.route).unwrap();
    assert_eq!(result.0, *blake3::hash(&intent.preimage).as_bytes());
    assert_eq!(result.1, 90_000);
    let mut changed = intent.request;
    changed.token[0] ^= 1;
    assert!(intent.result(changed, &intent.route).is_err());
    changed = intent.request;
    changed.lifetime_ms += 1;
    assert!(intent.result(changed, &intent.route).is_err());
    assert!(intent.result(intent.request, b"replacement plan").is_err());
}

#[test]
fn unsigned_invite_journal_refuses_missing_future_torn_quota_context_and_trailing_evidence() {
    let original = plain();
    assert!(decode(&original, [13; 32]).is_err());
    for length in 0..original.len() {
        assert!(decode(&original[..length], [11; 32]).is_err());
    }
    let mut extra = original.clone();
    extra.push(0);
    assert!(decode(&extra, [11; 32]).is_err());
    // Closed version and independent count/byte quota words must match exactly.
    let version = 4 + TAG.len();
    for index in version..version + 10 {
        let mut altered = original.clone();
        altered[index] ^= 1;
        assert!(decode(&altered, [11; 32]).is_err(), "word byte {index}");
    }
    assert!(decode(&vec![0; MAX_PLAIN + 1], [11; 32]).is_err());
}

fn writer(directory: &Path, frame: Vec<u8>) -> (Pages, JournalFile) {
    let pages = directory.join("text.pages");
    fs::create_dir(&pages).unwrap();
    let file = crate::store::persistence::open_archive_directory(&pages).unwrap();
    let id = session_id(&file).unwrap();
    let object = JournalFile {
        frame: Some(Zeroizing::new(frame)),
        ..JournalFile::default()
    };
    ((pages, file, id), object)
}

#[test]
fn unsigned_invite_journal_finishes_only_its_own_exact_prefix_and_retains_the_same_native_file() {
    let directory = tempfile::tempdir().unwrap();
    let expected = vec![42; 512];
    let (pages, mut journal) = writer(directory.path(), expected.clone());
    fail_partial_write_for_test();
    assert!(journal.write_same(&pages, NAME).is_err());
    let path = pages.0.join(NAME);
    assert_eq!(fs::read(&path).unwrap(), expected[..128]);
    let id = journal.file.as_ref().unwrap().1;
    journal.cover(&pages, NAME).unwrap();
    assert_eq!(fs::read(&path).unwrap(), expected);
    assert_eq!(journal.file.as_ref().unwrap().1, id);
    fs::write(&path, b"foreign/torn bytes").unwrap();
    assert!(journal.write_same(&pages, NAME).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"foreign/torn bytes");
}

#[test]
fn unsigned_invite_journal_never_recreates_a_missing_spent_file() {
    let directory = tempfile::tempdir().unwrap();
    let (pages, mut journal) = writer(directory.path(), vec![42; 512]);
    journal.cover(&pages, NAME).unwrap();
    let path = pages.0.join(NAME);
    fs::rename(&path, path.with_extension("held")).unwrap();
    assert!(journal.write_same(&pages, NAME).is_err());
    assert!(journal.require_exact(&pages, NAME).is_err());
    assert!(!path.exists());
    fs::write(&path, vec![42; 512]).unwrap();
    assert!(
        journal.write_same(&pages, NAME).is_err(),
        "equal ciphertext in a replacement file is not the held original"
    );
}
