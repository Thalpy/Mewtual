use crate::{transaction::Transactable, ActorId, AutoCommit, Automerge, Change, ROOT};

fn writer() -> AutoCommit {
    AutoCommit::new().with_actor(ActorId::from(vec![7]))
}

fn committed(doc: &mut AutoCommit, text: &str) -> Change {
    doc.put(ROOT, "text", text).unwrap();
    doc.get_last_local_change().unwrap().clone()
}

#[test]
fn readonly_observers_leave_pending_transaction_open_and_rollbackable() {
    let mut doc = writer();
    let first = committed(&mut doc, "saved");
    let instance = doc.history_instance_id();
    doc.put(ROOT, "text", "unfinished").unwrap();
    let pending = doc.pending_ops();
    assert!(pending > 0);

    assert_eq!(doc.applied_change_hashes(), &[first.hash()]);
    assert!(doc.queued_changes().is_empty());
    assert!(doc.has_applied_change(&first.hash()));
    assert_eq!(doc.history_instance_id(), instance);
    assert_eq!(doc.pending_ops(), pending);
    assert_eq!(doc.rollback(), pending);
    assert_eq!(doc.applied_change_hashes(), &[first.hash()]);
}

#[test]
fn inventory_borrows_existing_graph_and_queue_storage() {
    let mut source = writer();
    let first = committed(&mut source, "first");
    let parent = committed(&mut source, "parent");
    let orphan = committed(&mut source, "orphan");
    let mut doc = writer();
    doc.apply_changes([first.clone(), orphan.clone()]).unwrap();

    // Pointer equality pins borrowing rather than cloned/reconstructed change data. The public
    // observers do not inspect operation bodies, actor strings, metadata or parent collections.
    assert!(std::ptr::eq(
        doc.applied_change_hashes().as_ptr(),
        doc.doc.change_graph.all_hashes().as_ptr(),
    ));
    assert!(std::ptr::eq(
        &doc.queued_changes()[0],
        &doc.doc.queue.as_slice()[0],
    ));
    assert_eq!(doc.queued_changes()[0].raw_bytes(), orphan.raw_bytes());
    assert!(!doc.has_applied_change(&orphan.hash()));

    let instance = doc.history_instance_id();
    doc.apply_changes([parent]).unwrap();
    assert_eq!(doc.history_instance_id(), instance);
    assert!(doc.queued_changes().is_empty());
    assert_eq!(doc.applied_change_hashes().len(), 3);
    assert!(doc.has_applied_change(&orphan.hash()));
}

#[test]
fn saved_unsigned_orphan_is_observable_then_moves_to_applied_graph() {
    let mut source = writer();
    let first = committed(&mut source, "first");
    let parent = committed(&mut source, "parent");
    let orphan = committed(&mut source, "orphan");
    let mut doc = Automerge::new();
    doc.apply_changes([first.clone(), orphan.clone()]).unwrap();
    let original_instance = doc.history_instance_id();

    // Full upstream saves retain orphan originals even though applied-graph stats omit them.
    // A bare change chunk missing its parents remains subject to upstream's existing load rules.
    let saved = doc.save();
    let mut loaded = Automerge::load(&saved).unwrap();
    assert_ne!(loaded.history_instance_id(), original_instance);
    assert_eq!(loaded.applied_change_hashes(), &[first.hash()]);
    assert_eq!(loaded.queued_changes().len(), 1);
    assert_eq!(loaded.queued_changes()[0].hash(), orphan.hash());
    assert_eq!(loaded.queued_changes()[0].raw_bytes(), orphan.raw_bytes());
    assert!(!loaded.has_applied_change(&orphan.hash()));

    let loaded_instance = loaded.history_instance_id();
    loaded.apply_changes([parent]).unwrap();
    assert_eq!(loaded.history_instance_id(), loaded_instance);
    assert!(loaded.queued_changes().is_empty());
    assert!(loaded.has_applied_change(&orphan.hash()));
}

#[test]
fn tokens_hold_identity_but_new_clone_fork_and_load_do_not_reuse_it() {
    let mut doc = writer();
    committed(&mut doc, "first");
    let instance = doc.history_instance_id();
    assert_eq!(instance.clone(), instance);
    assert_eq!(doc.history_instance_id(), instance);
    assert_ne!(writer().history_instance_id(), instance);
    assert_ne!(doc.clone().history_instance_id(), instance);
    assert_ne!(doc.fork().history_instance_id(), instance);

    let bytes = doc.save();
    let loaded = AutoCommit::load(&bytes).unwrap();
    assert_eq!(loaded.applied_change_hashes(), doc.applied_change_hashes());
    assert_ne!(loaded.history_instance_id(), instance);
    assert_ne!(
        loaded.clone().history_instance_id(),
        loaded.history_instance_id()
    );
}

#[test]
fn moving_or_mutating_same_document_preserves_instance_identity() {
    let mut doc = writer();
    let instance = doc.history_instance_id();
    committed(&mut doc, "first");
    assert_eq!(doc.history_instance_id(), instance);
    let mut moved = doc;
    committed(&mut moved, "second");
    assert_eq!(moved.history_instance_id(), instance);
}

#[test]
fn incremental_load_replacement_is_fresh_but_merge_keeps_original_instance() {
    let mut source = writer();
    let first = committed(&mut source, "first");
    let first_save = source.save();
    let mut target = writer();
    let empty_instance = target.history_instance_id();
    target.load_incremental(&first_save).unwrap();
    let loaded_instance = target.history_instance_id();
    assert_ne!(loaded_instance, empty_instance);
    assert!(target.has_applied_change(&first.hash()));

    // Upstream replaces an empty Automerge during load, but merges into a populated document.
    // This token follows that actual replacement boundary, not the spelling of the caller API.
    let second = committed(&mut source, "second");
    let second_save = source.save();
    target.load_incremental(&second_save).unwrap();
    assert_eq!(target.history_instance_id(), loaded_instance);
    assert!(target.has_applied_change(&second.hash()));
}
