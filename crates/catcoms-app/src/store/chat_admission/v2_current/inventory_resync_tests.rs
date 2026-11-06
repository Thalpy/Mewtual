//! Actual failed legacy scope writes before/after cold capture, claim and ownership. Saving
//! that authorized inventory debt is DATA maintenance; it never revives the cancelled source.
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::fixture;
use crate::store::persistence::v2_genesis_activation::{
    ColdV2GenesisCurrentHead, PreparedColdV2GenesisCapture,
};
use crate::store::GenesisSeedBudget;
use rand_chacha::ChaCha20Rng;
use std::cell::Cell;
use std::sync::atomic::Ordering;

fn read_head(
    store: &ServerStore,
    source: &GenesisSourceDataRef<'_>,
) -> Arc<ColdV2GenesisCurrentHead> {
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, source).unwrap();
    let resources = capture.read_resources();
    store
        .with_cold_v2_genesis_current_read(source, |admission| {
            let guard = store.lock_cold_current_genesis_group(admission, &resources)?;
            capture.capture_current(&guard)
        })
        .unwrap()
}

fn fail_other_group_issuance(
    store: &ServerStore,
    group: &[u8],
    rng: &mut ChaCha20Rng,
) -> (
    ChatAdmissionKey,
    ChatAdmissionCapture,
    ChatAdmissionPreparation,
) {
    let key = ChatAdmissionKey::new(group, [0xe4; 32]).unwrap();
    let capture = store.capture_chat_admission(key.clone()).unwrap();
    store
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    assert!(store
        .prepare_legacy_chat_admission(&capture, b"actual inventory resync debt", rng)
        .is_err());
    // The real issuer retains its nonce/owner BEFORE the injected physical failure.
    let issued = capture.issued.lock().unwrap().as_ref().unwrap().clone();
    {
        let runtime = store.chat_mount.locked().unwrap();
        assert!(runtime.dirty);
        assert!(runtime.table.as_ref().unwrap().get(&key) == Some(&issued.record));
    }
    (key, capture, issued)
}

fn table(store: &ServerStore) -> BTreeMap<ChatAdmissionKey, Record> {
    store
        .chat_mount
        .locked()
        .unwrap()
        .table
        .as_ref()
        .unwrap()
        .clone()
}
fn resync(store: &ServerStore, rng: &mut ChaCha20Rng) {
    let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
    store
        .resync_v2_current_scope_inventory(&buffers, rng)
        .unwrap();
    drop(buffers); // Actual large credit refunds OUTSIDE the inventory writer.
}

#[test]
fn real_inventory_debt_resyncs_before_cold_head_claim_and_owner_without_renewal() {
    let mut f = fixture(0xce41, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let original_source = blake3::hash(source.source().data().view_data().snapshot);
    let source_id = source.data().source;
    let context = source.data().context.spec_data();
    let own_key = ChatAdmissionKey::new(context.group, context.signer).unwrap();
    let calls = Cell::new(0);

    // The first gap precedes ANY ColdHead or registered task, so source-specific helpers
    // cannot repair it. This is a real authorized legacy candidate, not a dirty-bit fixture.
    let (_, _early_capture, _early_issued) = fail_other_group_issuance(
        store,
        b"actual inventory debt before cold head read",
        &mut f.rng,
    );
    let early = table(store);
    assert!(store
        .with_cold_v2_genesis_current_read(&source, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .is_err());
    assert_eq!(calls.get(), 0);
    resync(store, &mut f.rng);
    assert!(table(store) == early);
    let head = read_head(store, &source);
    let ids = (
        head.activation_id_data(),
        head.head_id_data(),
        head.pin_id_data(),
    );
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    let task_observation = task.observe_original();

    // A second gap exists after the actual registered claim but before owner insertion.
    let (_, _claimed_capture, _claimed_issued) = fail_other_group_issuance(
        store,
        b"actual inventory debt before cold owner",
        &mut f.rng,
    );
    let claimed = table(store);
    let mut slot = None;
    assert!(store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut slot)
        .is_err());
    assert!(slot.is_none());
    assert!(!store
        .chat_mount
        .locked()
        .unwrap()
        .owners
        .contains_key(&own_key));
    task.require_registered_cold_current_claim(store, &source, &head)
        .unwrap();
    resync(store, &mut f.rng);
    assert!(table(store) == claimed);
    task.require_registered_cold_current_claim(store, &source, &head)
        .unwrap();
    store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut slot)
        .unwrap();
    let preparation = slot.as_ref().unwrap();
    let original_owner = preparation.owner_identity();
    let original_record = table(store).get(&own_key).unwrap().clone();
    let original_basis = preparation.scope_basis_data().clone();
    assert_eq!(
        preparation.progress_data(),
        ColdV2CurrentProgressData::Unbound
    );
    assert!(preparation.current_binding().is_err());

    let (other_key, _other_capture, other_issued) = fail_other_group_issuance(
        store,
        b"actual inventory debt before cold handoff",
        &mut f.rng,
    );
    let dirty = table(store);
    assert!(store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut slot)
        .is_err());
    let preparation = slot.as_ref().unwrap();
    assert!(store
        .with_cold_owned_current_admission(preparation, &source, &task, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .is_err());
    assert_eq!(calls.get(), 0, "dirty must precede handoff/install");

    let charged = f.pool.charged_for_test();
    store
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    let insufficient = f.pool.try_reserve(SCOPE_BUFFER_FEE - 1).unwrap();
    assert!(store
        .resync_v2_current_scope_inventory(&insufficient, &mut f.rng)
        .is_err());
    assert!(store.chat_mount.locked().unwrap().dirty);
    assert!(store.chat_fence_pre_rename_failure.load(Ordering::SeqCst));
    assert!(table(store) == dirty);
    drop(insufficient);
    assert_eq!(f.pool.charged_for_test(), charged);
    let foreign_pool = CriticalCatalogReadPool::new(SCOPE_BUFFER_FEE).unwrap();
    let foreign_credit = foreign_pool.try_reserve(SCOPE_BUFFER_FEE).unwrap();
    assert!(store
        .resync_v2_current_scope_inventory(&foreign_credit, &mut f.rng)
        .is_err());
    assert!(table(store) == dirty);
    assert!(store.chat_fence_pre_rename_failure.load(Ordering::SeqCst));
    drop(foreign_credit);

    let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
    assert_eq!(f.pool.charged_for_test(), charged + SCOPE_BUFFER_FEE);
    let installed_proofs = Cell::new(0);
    assert!(store
        .resync_cold_v2_genesis_current(
            preparation,
            &source,
            &task,
            &buffers,
            &mut f.rng,
            |_, _| {
                installed_proofs.set(installed_proofs.get() + 1);
                Err(refused())
            }
        )
        .is_err());
    assert_eq!(
        installed_proofs.get(),
        0,
        "Unbound has no installed proof role"
    );
    // Pressure refusals did not consume the armed physical seam. The first fully paid
    // actual flush now fails there, retaining SAME data/owner and credit for an explicit retry.
    assert!(store
        .resync_v2_current_scope_inventory(&buffers, &mut f.rng)
        .is_err());
    assert!(!store.chat_fence_pre_rename_failure.load(Ordering::SeqCst));
    assert!(store.chat_mount.locked().unwrap().dirty);
    assert!(table(store) == dirty);
    store
        .resync_v2_current_scope_inventory(&buffers, &mut f.rng)
        .unwrap();
    {
        let runtime = store.chat_mount.locked().unwrap();
        assert!(!runtime.dirty);
        assert!(runtime.table.as_ref().unwrap() == &dirty);
        assert!(runtime.table.as_ref().unwrap().get(&own_key) == Some(&original_record));
        assert!(runtime.table.as_ref().unwrap().get(&other_key) == Some(&other_issued.record));
        let owner = runtime.owners.get(&own_key).unwrap();
        assert!(Arc::ptr_eq(&owner.life.identity(), &original_owner));
        assert!(owner.life.live());
        assert!(Arc::ptr_eq(
            &runtime.owners.get(&other_key).unwrap().life.identity(),
            &other_issued.owner_identity,
        ));
    }
    assert!(preparation.scope_basis_data() == &original_basis);
    assert!(Arc::ptr_eq(preparation.head(), &head));
    assert_eq!(
        preparation.progress_data(),
        ColdV2CurrentProgressData::Unbound
    );
    assert!(preparation.current_binding().is_err());
    task_observation.require_same_original(&task).unwrap();
    assert!(
        (
            head.activation_id_data(),
            head.head_id_data(),
            head.pin_id_data()
        ) == ids
    );
    assert!(blake3::hash(source.source().data().view_data().snapshot) == original_source);
    drop(buffers);
    assert_eq!(f.pool.charged_for_test(), charged);
    let retired = store
        .with_cold_owned_current_admission(preparation, &source, &task, |admission| {
            task.handoff_cold_current_owner(admission)
        })
        .unwrap();
    drop(retired); // Exact paid registry entry retires outside admission/pins/Root.

    // INTENTIONAL contract change from the superseded source-specific helper: saving
    // already-authorized debt is legal after source/owner cancellation. It is not activation.
    let (_, _cancelled_capture, _cancelled_issued) = fail_other_group_issuance(
        store,
        b"actual inventory debt after source cancellation",
        &mut f.rng,
    );
    let cancelled_debt = table(store);
    preparation.cancel_original();
    drop(f.source); // The actual original read/SDK dies too; task/head keep only weak identity.
    resync(store, &mut f.rng);
    {
        let runtime = store.chat_mount.locked().unwrap();
        assert!(!runtime.dirty);
        assert!(runtime.table.as_ref().unwrap() == &cancelled_debt);
        let owner = runtime.owners.get(&own_key).unwrap();
        assert!(Arc::ptr_eq(&owner.life.identity(), &original_owner));
        assert!(
            !owner.life.live(),
            "maintenance must not revive the publisher"
        );
    }
    assert!(preparation.current_binding().is_err());
    // A NEW real read/SDK with equal bytes cannot adopt the old owner/task identity.
    let read = store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let budget = GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let replacement = store.prepare_genesis_seed(read, budget).unwrap();
    let replacement_source = replacement.borrow_source_data();
    assert!(replacement_source.data().source == source_id);
    assert!(task.retains_original(store, &replacement_source).is_err());
    assert!(store
        .prepare_cold_v2_current_scope(&replacement_source, &task, head.clone(), &mut slot)
        .is_err());
    assert!(store
        .with_cold_v2_genesis_current_read(&replacement_source, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .is_err());
    assert_eq!(calls.get(), 0);
    assert!(table(store) == cancelled_debt);
}

#[test]
fn inventory_repair_requires_original_namespace_and_live_mount_before_any_scope_write() {
    let mut f = fixture(0xce42, false);
    let store = f.layout.checked_store().unwrap();
    let (_, _capture, _issued) = fail_other_group_issuance(
        store,
        b"actual inventory debt under namespace proof",
        &mut f.rng,
    );
    let expected = table(store);
    let marker = store
        .dir
        .join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
    let original = std::fs::read(&marker).unwrap();
    let mut changed = original.clone();
    *changed.last_mut().unwrap() ^= 1;
    std::fs::write(&marker, changed).unwrap(); // Same inode, invalid original ID3 content.
    let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
    store
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    assert!(store
        .resync_v2_current_scope_inventory(&buffers, &mut f.rng)
        .is_err());
    assert!(store.chat_fence_pre_rename_failure.load(Ordering::SeqCst));
    assert!(store.chat_mount.locked().unwrap().dirty);
    assert!(table(store) == expected);
    std::fs::write(&marker, &original).unwrap();
    // Equal authentic bytes in a foreign file object are not the original mounted marker.
    let displaced = f.directory.path().join("held-inventory-resync-marker");
    std::fs::rename(&marker, &displaced).unwrap();
    std::fs::write(&marker, &original).unwrap();
    assert!(store
        .resync_v2_current_scope_inventory(&buffers, &mut f.rng)
        .is_err());
    assert!(store.chat_fence_pre_rename_failure.load(Ordering::SeqCst));
    assert!(table(store) == expected);
    std::fs::remove_file(&marker).unwrap();
    std::fs::rename(&displaced, &marker).unwrap(); // Restore the SAME actual held object.
    store.chat_mount_revoker().revoke();
    assert!(store
        .resync_v2_current_scope_inventory(&buffers, &mut f.rng)
        .is_err());
    assert!(store.chat_fence_pre_rename_failure.load(Ordering::SeqCst));
    assert!(store.chat_mount.runtime.lock().unwrap().dirty);
    assert!(
        store
            .chat_mount
            .runtime
            .lock()
            .unwrap()
            .table
            .as_ref()
            .unwrap()
            == &expected
    );
    drop(buffers);
}
