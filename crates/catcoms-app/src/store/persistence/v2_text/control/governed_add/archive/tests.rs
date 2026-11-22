use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_add_archive_retains_the_exact_original_records_and_parent() {
    let labels = GovernedArchiveLabels {
        context: TextContextIdData::from_bytes([1; 32]),
        before_head: TextHeadIdData::from_bytes([2; 32]),
        before_archive: ArchiveHeadId::from_bytes([3; 32]),
        header: OwnerHeaderId::from_bytes([4; 32]),
        state: OwnerStateCommitment::from_bytes([5; 32]),
        epoch: 1,
        authenticator: [6; 32],
    };
    let records = BODY_CAPS.map(|(least, _)| vec![0x7a; least]);
    let [parent_head, parent_archive, fence, request, entry, outcome, header_attempt, raw_header, governing, welcome_intent, welcome_attempt, raw_welcome, response] =
        records.each_ref().map(Vec::as_slice);
    let frames = GovernedArchiveFrames {
        parent_head,
        parent_archive,
        fence,
        request,
        entry,
        outcome,
        header_attempt,
        raw_header,
        governing,
        welcome_intent,
        welcome_attempt,
        raw_welcome,
        response,
    };
    let key = [13; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(170);
    let frozen = seal_data(&key, labels, &frames, &mut rng).unwrap();
    let id = id_data(&frozen);
    let plain = open_data(&key, id, &frozen).unwrap();
    let (restored, embedded) = decode_data(&plain).unwrap();
    assert!(restored == labels);
    assert_eq!(embedded.ordered(), frames.ordered());
    assert_eq!(frozen.len(), frames.frame_bytes().unwrap());

    let changed = seal_data(&key, labels, &frames, &mut rng).unwrap();
    assert!(open_data(&key, id, &changed).is_err());
    assert!(open_data(&key, id, &frozen[..frozen.len() - 1]).is_err());
    let invalid = GovernedArchiveFrames {
        fence: &frames.fence[1..],
        ..frames
    };
    assert!(seal_data(&key, labels, &invalid, &mut rng).is_err());
    let old_kind = TextControlKindData::Archive;
    assert!(open_control_data(&key, old_kind.id_data(&frozen), &frozen).is_err());
}
