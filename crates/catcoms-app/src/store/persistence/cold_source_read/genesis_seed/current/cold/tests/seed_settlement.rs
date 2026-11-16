//! A genuine newly paid cold source/SDK/head/owner/node enters SAME-mount cleanup. Its
//! original PresentColdHead remains seed-dependent; only the new actual saved cleanup
//! receipt validates absence. This is not a reconstructed warm or Absent capture receipt.
use super::*;

#[test]
fn cold_original_node_retries_after_unlink_and_keeps_original_head_owner_and_writer() {
    let (mut prepared, layout, mut rng, directory) = caller(0x5ec01);
    prepared.retry(&layout, &mut rng).unwrap();
    let mut current = prepared.take_current(&layout).unwrap().unwrap();
    let store = layout.checked_store().unwrap();
    let head = current.head.clone();
    let source = current.data().source;
    let owner = current.scope.owner_identity();
    let nonce = current.scope.nonce();
    let writer = current.writer.observe_original();
    let group = head.directory_data().0.to_path_buf();
    store.critical_namespace_fault.store(212, Ordering::SeqCst);
    assert!(current.retry_seed_settlement(&layout, &mut rng).is_err());
    assert!(!group.join("activation.seed").exists());
    assert!(current.validate_original(&layout).is_err());
    store.critical_namespace_fault.store(0, Ordering::SeqCst);
    current.retry_seed_settlement(&layout, &mut rng).unwrap();
    assert!(Arc::ptr_eq(&head, &current.head));
    assert!(Arc::ptr_eq(&owner, &current.scope.owner_identity()));
    assert_eq!(current.scope.nonce(), nonce);
    assert_eq!(current.data().source, source);
    current.writer.require_same_observation(&writer).unwrap();
    current.validate_original(&layout).unwrap();
    // The original cold receipt has not been silently converted into an absent role.
    let root = store.archive_bootstrap_root.lock().unwrap();
    assert!(head
        .validate_under_archive_root(store, &current.source(), root.as_ref().unwrap())
        .is_err());
    drop(root);
    drop(current);
    drop(prepared);
    drop(writer);
    drop(head);
    drop(layout);
    drop(directory);
}

#[test]
fn cancelled_cold_writer_after_unlink_cannot_finish_or_renew_the_original_node() {
    let (mut prepared, layout, mut rng, directory) = caller(0x5ec02);
    prepared.retry(&layout, &mut rng).unwrap();
    let mut current = prepared.take_current(&layout).unwrap().unwrap();
    let store = layout.checked_store().unwrap();
    let source = current.data().source;
    let group = current.head.directory_data().0.to_path_buf();
    store.critical_namespace_fault.store(212, Ordering::SeqCst);
    assert!(current.retry_seed_settlement(&layout, &mut rng).is_err());
    store.critical_namespace_fault.store(0, Ordering::SeqCst);
    current.writer.cancel_original();
    assert!(current.retry_seed_settlement(&layout, &mut rng).is_err());
    assert!(current.validate_original(&layout).is_err());
    assert_eq!(current.data().source, source);
    assert!(!group.join("activation.seed").exists());
    assert!(group.join("pin.bin").is_file());
    drop(current);
    drop(prepared);
    drop(layout);
    drop(directory);
}
