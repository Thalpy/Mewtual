use super::*;

#[test]
fn review_selection_refuses_scratch_changed_during_source_recheck() {
    let temp = tempfile::tempdir().unwrap();
    let root = HeldDirectory::open(temp.path().to_path_buf()).unwrap();
    let scratch = root.child(SELECTION_SCRATCH);
    std::fs::write(&scratch, b"authentic selection scratch").unwrap();
    let mut calls = 0;
    let result = cover_selection_wal(
        &root,
        |frame| {
            assert_eq!(frame.unwrap(), b"authentic selection scratch");
            calls += 1;
            if calls == 2 {
                std::fs::write(&scratch, b"substituted selection data!").unwrap();
            }
            Ok(())
        },
        || panic!("an existing complete scratch must be verified"),
    );
    assert!(result.is_err());
    assert_eq!(calls, 2);
    assert!(!root.child(SELECTION_WAL).exists());
    assert_eq!(
        std::fs::read(scratch).unwrap(),
        b"substituted selection data!"
    );
}

#[test]
fn review_selection_refuses_head_stage_changed_during_source_recheck() {
    let temp = tempfile::tempdir().unwrap();
    let root = HeldDirectory::open(temp.path().to_path_buf()).unwrap();
    let wal = root.child(SELECTION_WAL);
    std::fs::write(&wal, b"already covered WAL").unwrap();
    let mut wal = HeldDecisionFile::read(wal, MAX_SELECTION_BYTES)
        .unwrap()
        .unwrap();
    let transaction = DecisionSelection {
        controls: [[1; 32], [2; 32], [3; 32]],
        previous_head: None,
        record: b"expected record",
        head: b"expected head",
    };
    let head = DecisionHead {
        sequence: 1,
        entry: [1; 32],
        record: [2; 32],
        previous_head: [0; 32],
    };
    let stage = root.child(HEAD_STAGE);
    let result = select_covered_wal(&root, &mut wal, &transaction, head, || {
        if stage.is_file() {
            std::fs::write(&stage, b"replaced head").unwrap();
        }
        Ok(())
    });
    assert!(result.is_err());
    assert!(!root.child(HEAD_CURRENT).exists());
    assert_eq!(std::fs::read(stage).unwrap(), b"replaced head");
    assert_eq!(
        std::fs::read(root.child(RECORD_DIRECTORY).join("0000000000000001.record")).unwrap(),
        transaction.record,
    );
}

#[test]
fn review_selection_refuses_to_start_again_without_the_covering_wal() {
    let temp = tempfile::tempdir().unwrap();
    let root = HeldDirectory::open(temp.path().to_path_buf()).unwrap();
    let records = root.child(RECORD_DIRECTORY);
    std::fs::create_dir(&records).unwrap();
    let first = records.join("0000000000000001.record");
    std::fs::write(&first, b"existing first record").unwrap();
    let result = cover_selection_wal(
        &root,
        |frame| {
            assert!(frame.is_none());
            Ok(())
        },
        || panic!("a missing WAL with retained outputs cannot start anew"),
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(first).unwrap(), b"existing first record");
    assert!(!root.child(SELECTION_SCRATCH).exists());
    assert!(!root.child(SELECTION_WAL).exists());
}
