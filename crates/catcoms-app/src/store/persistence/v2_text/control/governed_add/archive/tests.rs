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

#[test]
fn governed_later_add_archive_binds_the_exact_predecessor_and_refuses_downgrade() {
    let first = GovernedArchiveLabels {
        context: TextContextIdData::from_bytes([1; 32]),
        before_head: TextHeadIdData::from_bytes([2; 32]),
        before_archive: ArchiveHeadId::from_bytes([3; 32]),
        header: OwnerHeaderId::from_bytes([4; 32]),
        state: OwnerStateCommitment::from_bytes([5; 32]),
        epoch: 1,
        authenticator: [6; 32],
    };
    let key = [29; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(171);
    let prior_archive = ArchiveHeadId::from_bytes([7; 32]);
    let prior =
        selected_head::GovernedHeadData::after_first_archive(first, prior_archive, 1).unwrap();
    let previous = prior.seal_data(&key, &mut rng).unwrap();
    let alternate = prior.seal_data(&key, &mut rng).unwrap();
    let labels = GovernedArchiveLabels {
        context: first.context,
        before_head: TextHeadIdData::of_data(&previous),
        before_archive: prior_archive,
        header: OwnerHeaderId::from_bytes([41; 32]),
        state: OwnerStateCommitment::from_bytes([51; 32]),
        epoch: 2,
        authenticator: [61; 32],
    };
    let records = LATER_BODY_CAPS.map(|(min, _)| vec![0x7b; min]);
    let [_, _, fence, request, entry, outcome, header_attempt, raw_header, governing, welcome_intent, welcome_attempt, raw_welcome, response] =
        records.each_ref().map(Vec::as_slice);
    let frames = GovernedArchiveFrames {
        parent_head: &previous,
        parent_archive: &[],
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
    let output = seal_later_data(&key, labels, &frames, &mut rng).unwrap();
    let selected = selected_head::GovernedHeadData::open_governed_data(&key, &output.head).unwrap();
    assert_eq!(selected.generation, 3);
    assert!(selected.transition == labels);
    assert_eq!(selected.archive, later_id(&output.archive));
    let opened = open_later_data(&key, &selected, &output.archive).unwrap();
    let (decoded, stored) = decode_later_data(&opened).unwrap();
    assert!(decoded == labels);
    assert_eq!(stored.ordered(), frames.ordered());

    assert!(selected_head::GovernedHeadData::open_data(&key, &output.head).is_err());
    assert!(open_data(&key, id_data(&output.archive), &output.archive).is_err());
    assert!(matches!(
        inspect_known_control_mac_data(&key, &output.head).unwrap(),
        TextControlMacInspectionData::Authenticated
    ));
    assert!(open_control_data(
        &key,
        TextControlKindData::Head.id_data(&output.head),
        &output.head
    )
    .is_err());
    assert!(open_later_data(&key, &prior, &output.archive).is_err());
    assert!(open_later_data(&key, &selected, &output.archive[..output.archive.len() - 1]).is_err());
    let substituted = GovernedArchiveFrames {
        parent_head: &alternate,
        ..frames
    };
    assert!(seal_later_data(&key, labels, &substituted, &mut rng).is_err());
    let embedded = GovernedArchiveFrames {
        parent_archive: &[0u8; 41],
        ..frames
    };
    assert!(seal_later_data(&key, labels, &embedded, &mut rng).is_err());
    for invalid in [
        GovernedArchiveLabels {
            before_archive: first.before_archive,
            ..labels
        },
        GovernedArchiveLabels {
            context: TextContextIdData::from_bytes([99; 32]),
            ..labels
        },
        GovernedArchiveLabels { epoch: 3, ..labels },
    ] {
        assert!(seal_later_data(&key, invalid, &frames, &mut rng).is_err());
    }
}
