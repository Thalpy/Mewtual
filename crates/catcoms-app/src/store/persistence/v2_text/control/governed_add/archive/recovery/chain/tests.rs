use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_archive_chain_discovery_requires_named_original_files_and_the_pinned_parent() {
    let directory = tempfile::tempdir().unwrap();
    let dir = directory.path();
    let key = [31; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(172);
    let anchor = (
        TextHeadIdData::from_bytes([2; 32]),
        ArchiveHeadId::from_bytes([3; 32]),
    );
    let first = GovernedArchiveLabels {
        context: TextContextIdData::from_bytes([1; 32]),
        before_head: anchor.0,
        before_archive: anchor.1,
        header: OwnerHeaderId::from_bytes([4; 32]),
        state: OwnerStateCommitment::from_bytes([5; 32]),
        epoch: 1,
        authenticator: [6; 32],
    };
    // This exercises the physical walk. The real signed journals and SDK
    // outcome are verified separately before native activation.
    let records = BODY_CAPS.map(|(min, _)| vec![0x7c; min]);
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
    let first_archive = seal_data(&key, first, &frames, &mut rng).unwrap();
    let archive_id = id_data(&first_archive);
    let head = selected_head::GovernedHeadData::after_first_archive(first, archive_id, 1)
        .unwrap()
        .seal_data(&key, &mut rng)
        .unwrap();
    let latest = GovernedArchiveLabels {
        before_head: TextHeadIdData::of_data(&head),
        before_archive: archive_id,
        header: OwnerHeaderId::from_bytes([42; 32]),
        state: OwnerStateCommitment::from_bytes([43; 32]),
        epoch: 2,
        authenticator: [44; 32],
        ..first
    };
    let latest = seal_later_data(
        &key,
        latest,
        &GovernedArchiveFrames {
            parent_head: &head,
            parent_archive: &[],
            ..frames
        },
        &mut rng,
    )
    .unwrap();
    let selected = selected_head::GovernedHeadData::open_governed_data(&key, &latest.head).unwrap();
    let capture = |anchor| {
        let mut buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
        capture_predecessors(
            &key,
            dir,
            &selected,
            &latest.archive,
            anchor,
            &mut buffer,
            &mut || Ok(()),
        )
    };
    let expected = archive_child(dir, archive_id.as_bytes()).unwrap();
    assert!(capture(anchor).is_err());
    std::fs::write(&expected, &*first_archive).unwrap();
    assert!(capture((TextHeadIdData::from_bytes([45; 32]), anchor.1)).is_err());
    let originals = capture(anchor).unwrap();
    let [original] = originals.as_slice() else {
        panic!("one original predecessor");
    };
    let mut buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
    let observed = original.reread(dir, &mut buffer).unwrap();
    assert_eq!(observed.frame.as_slice(), first_archive.as_slice());
    drop(observed);
    require_exact_predecessors(dir, &originals).unwrap();

    let mut corrupt = (*first_archive).clone();
    corrupt[0] ^= 1;
    let mut buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
    std::fs::write(&expected, &corrupt).unwrap();
    assert!(original.reread(dir, &mut buffer).is_err());
    assert!(require_exact_predecessors(dir, &originals).is_err());
    std::fs::write(&expected, &*first_archive).unwrap();
    let mut buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
    assert!(original.reread(dir, &mut buffer).is_ok());
    require_exact_predecessors(dir, &originals).unwrap();

    std::fs::rename(&expected, dir.join("original-file.bin")).unwrap();
    std::fs::write(&expected, &*first_archive).unwrap();
    let mut buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
    assert!(original.reread(dir, &mut buffer).is_err());
    std::fs::write(&expected, corrupt).unwrap();
    assert!(capture(anchor).is_err());
}
