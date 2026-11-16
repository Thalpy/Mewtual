//! Finite control IO ends independently from the SAME retained logical root reader.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::mpsc::sync_channel;
use std::time::Duration;

#[test]
fn actual_query_control_quiescence_waits_for_file_cleanup_but_not_logical_result_reader() {
    let original = super::super::promotion::promoted_text_fixture(0, 0x7ef70);
    let selected = selected_capture(&original);
    let (mut query, mut reader, mut capture) = prepare_query(&original, &selected, [70; 32]);
    let admitted = admit_query(&original, &mut query, &mut reader, &mut capture);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let resources = query.resources().clone();
    let head = owner.source().source().capture.target.join("head.bin");
    assert_eq!(text_query_registry_counts_for_test(store), (1, 1));
    assert_eq!(text_query_control_io_for_test(store), 1);
    let (started_tx, started_rx) = sync_channel(0);
    let (release_tx, release_rx) = sync_channel(0);
    query.set_after_run_source_for_test(Box::new(move || {
        started_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    }));
    let worker = std::thread::spawn(move || {
        let result = query.run(admitted);
        (query, result)
    });
    started_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head)
        .is_err());
    assert!(
        store
            .with_v2_text_current(&owner, |a| {
                let guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.require_text_control_io_quiescent()
            })
            .is_err(),
        "real open control Files, never a caller integer, block quiescence"
    );
    assert_eq!(text_query_control_io_for_test(store), 1);
    release_tx.send(()).unwrap();
    let (query, result) = worker.join().unwrap();
    result.unwrap();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head)
        .unwrap();
    drop(file);
    assert_eq!(text_query_control_io_for_test(store), 0);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 1));
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            query.validate_completed_under_current_guard(&mut guard)?;
            guard.require_text_control_io_quiescent()
        })
        .unwrap();
    drop(query);
    assert_eq!(text_query_registry_counts_for_test(store), (1, 0));
}
