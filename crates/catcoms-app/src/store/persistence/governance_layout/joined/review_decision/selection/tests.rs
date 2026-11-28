use super::*;
use crate::store::persistence::v2_review_hold::tests::{fixture, scope};
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::{
    owner_bodies::{
        ExactReviewOriginal, ReviewDecision, ReviewDecisionBody, ReviewSpec, ReviewStreamPosition,
        UnsignedReviewStreamEntry,
    },
    owner_facts::OwnerFactId,
    owner_header::{OwnerHeaderId, OwnerStateCommitment},
    V2RemovedCreateRecord,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn review_index_recovers_the_original_unindexed_v1_selection() {
    let mut rng = ChaCha20Rng::seed_from_u64(722);
    let key = JournalKey::new(&[29; 32], &[30; 32]);
    let scope = scope();
    let (held, original) = fixture(scope, 21, 22);
    let target = V2RemovedCreateRecord::decode(&held)
        .unwrap()
        .target_data()
        .clone();
    let decision = ExactReviewDecisionBody::new(
        ReviewDecisionBody::new(ReviewSpec {
            target,
            intent: [23; 16],
            decision: ReviewDecision::Approve,
            previous: None,
        })
        .unwrap(),
        ExactReviewOriginal {
            digest: original.original,
            remove_header: original.removal.0,
            remove_epoch: original.removal.1,
        },
    )
    .unwrap();
    let founder = DeviceKeypair::from_seed(&[74; 32]);
    let pin = FounderGenesisAnchorData {
        group: scope.group.to_vec(),
        founder: founder.verifying_key().to_bytes(),
        profile: scope.profile,
        admission: OwnerFactId::from_bytes([24; 32]),
        tenure: scope.tenure,
        header: OwnerHeaderId::from_bytes([25; 32]),
        state: OwnerStateCommitment::from_bytes([26; 32]),
    };
    let position = ReviewStreamPosition {
        sequence: 1,
        previous: None,
        membership_header: original.removal.0,
        membership_epoch: original.removal.1,
    };
    let unsigned = UnsignedReviewStreamEntry::prepare(&pin, position, &decision).unwrap();
    let signature = founder.sign(unsigned.signing_bytes());
    let entry = unsigned.attach_signature(&pin, signature).unwrap();
    let request = *decision.id().as_bytes();
    let controls = [[27; 32], [28; 32], [31; 32]];
    let record = RetainedDecisionRecord::prepare(
        &key,
        &pin,
        (&entry, &decision, &held),
        (request, controls[2]),
        &mut rng,
    )
    .unwrap();

    // Emit the earlier format without the current head or WAL writer.
    let mut head = Encoder::with_capacity(LEGACY_HEAD_BYTES);
    head.put_bytes(b"catcoms/private-chat-v2-review-journal-head/v1\0")
        .unwrap();
    head.put_u64(1);
    head.put_bytes(entry.id().as_bytes()).unwrap();
    head.put_bytes(&blake3::derive_key(RECORD_DIGEST_DOMAIN, &record))
        .unwrap();
    head.put_bytes(&[0; 32]).unwrap();
    let head = key.seal(head, &mut rng).unwrap();
    let mut body = Encoder::with_capacity(MAX_SELECTION_BYTES);
    let [intent, attempt, raw] = &controls;
    for field in [
        &b"catcoms/private-chat-v2-review-journal-selection/v1\0"[..],
        intent,
        attempt,
        raw,
        &[],
        record.as_slice(),
        head.as_slice(),
    ] {
        body.put_bytes(field).unwrap();
    }
    let encrypted = key.seal(body, &mut rng).unwrap();
    let mut legacy = b"CTV2RVJ\x01".to_vec();
    legacy.extend_from_slice(&u32::try_from(encrypted.len() + 12).unwrap().to_be_bytes());
    legacy.extend_from_slice(&encrypted);
    let mut wrong_version = legacy.clone();
    wrong_version[7] = 2;
    assert!(DecisionSelection::open(&key, &wrong_version).is_err());
    let plain = DecisionSelection::open(&key, &legacy).unwrap();
    let mut crossed = DecisionSelection::decode(&plain).unwrap();
    crossed.pages = Some([b"unexpected intent page", b"unexpected case page"]);
    assert!(crossed.verify_data(&key, &pin, (&entry, request)).is_err());

    let verify = |bytes: &[u8]| {
        let plain = DecisionSelection::open(&key, bytes).unwrap();
        let selection = DecisionSelection::decode(&plain).unwrap();
        selection.require_controls(controls).unwrap();
        assert!(selection
            .verify_data(&key, &pin, (&entry, request))
            .unwrap()
            .roots
            .is_none());
    };
    verify(&legacy);
    let temp = tempfile::tempdir().unwrap();
    let root = HeldDirectory::open(temp.path().to_path_buf()).unwrap();
    let pending = root.child(disk::SELECTION_WAL);
    std::fs::write(&pending, &legacy).unwrap();
    for _ in 0..2 {
        let mut wal = disk::cover_selection_wal(
            &root,
            None,
            |bytes| {
                verify(bytes.unwrap());
                Ok(())
            },
            || panic!("the earlier WAL must never be regenerated"),
        )
        .unwrap();
        let plain = DecisionSelection::open(&key, &wal.bytes).unwrap();
        let transaction = DecisionSelection::decode(&plain).unwrap();
        let selected = transaction
            .verify_data(&key, &pin, (&entry, request))
            .unwrap();
        disk::select_covered_wal(&root, &mut wal, &transaction, selected, None, || Ok(())).unwrap();
        assert_eq!(
            std::fs::read(root.child(disk::HEAD_CURRENT))
                .unwrap()
                .as_slice(),
            head.as_slice()
        );
        assert_eq!(std::fs::read(&pending).unwrap(), legacy);
    }
}
