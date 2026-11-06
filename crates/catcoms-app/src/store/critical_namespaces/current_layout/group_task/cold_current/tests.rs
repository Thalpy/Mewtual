//! Actual remounted complete Current2, new paid SDK/read/cold capture, genuine task/owner.
//! The fixture never fabricates a row, receipt, observer, warm directory or SDK success.
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{fixture, ColdFixture};
use crate::store::owner_evidence::canonical::current_vacancy::pending_group_registration_fee_data_for_test;
use crate::store::persistence::v2_genesis_activation::{
    cold_writer::PreparedColdV2CurrentWriterReceipt, PreparedColdV2GenesisCapture,
};
use std::cell::{Cell, RefCell};

fn captured(number: u64) -> (ColdFixture, Arc<ColdV2GenesisCurrentHead>) {
    let f = fixture(number, false);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let head = store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            capture.capture_current(&store.lock_cold_current_genesis_group(admission, &resources)?)
        })
        .unwrap();
    (f, head)
}

#[test]
fn actual_original_claim_excludes_duplicates_and_drop_never_renews_it() {
    let (f, head) = captured(0xcc01);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let baseline = f.pool.charged_for_test();
    let mut task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    task.require_registered_cold_current_claim(store, &source, &head)
        .unwrap();
    assert!(task.directory_output().is_err());
    assert!(task.saved_directory().is_err());
    assert!(
        task.retry_directory(store, &source).is_err(),
        "cold never creates warm state"
    );
    assert!(
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).is_err()
    );
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    let original = task.observe_original();
    drop(task);
    assert!(
        f.pool.charged_for_test() > baseline,
        "inert paid entry retains original debt"
    );
    assert!(
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).is_err()
    );
    drop(original);
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    assert!(store
        .capture_chat_admission(
            crate::store::ChatAdmissionKey::new(
                b"independently available cold control",
                [0xa3; 32],
            )
            .unwrap()
        )
        .is_ok());
}

#[test]
fn every_first_registration_payment_refuses_before_entry_and_preserves_same_source() {
    const CAPACITY: usize = 1024 * 1024 * 1024;
    for phase in 0..3 {
        let (f, head) = captured(0xcc10 + phase);
        let store = f.layout.checked_store().unwrap();
        let source = f.source.borrow_source_data();
        let (backing, entry) = pending_group_registration_fee_data_for_test();
        let needed = COLD_TASK_FEE
            + if phase >= 1 { backing } else { 0 }
            + if phase == 2 { entry } else { 0 };
        let baseline = f.pool.charged_for_test();
        let blocker = f
            .pool
            .try_reserve(CAPACITY - baseline - needed + 1)
            .unwrap();
        assert!(
            PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone())
                .is_err()
        );
        assert!(store.require_no_pending_v2_group(&f.group).is_ok());
        head.retains_original(store, &source).unwrap();
        drop(blocker);
        assert_eq!(f.pool.charged_for_test(), baseline);
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head).unwrap();
    }
}

#[test]
fn actual_cached_scope_owner_handoff_is_exact_idempotent_and_keeps_same_life() {
    let (f, head) = captured(0xcc20);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    let original = task.observe_original();
    let mut scope = None;
    store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut scope)
        .unwrap();
    let scope_ref = scope.as_ref().unwrap();
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    let retired = store
        .with_cold_owned_current_admission(scope_ref, &source, &task, |admission| {
            assert!(task.require_cold_owned_handoff(admission).is_err());
            task.handoff_cold_current_owner(admission)
        })
        .unwrap();
    assert!(retired.is_some());
    drop(retired); // pool refunds are deliberately OUTSIDE all actual writers
    assert!(store.require_no_pending_v2_group(&f.group).is_ok());
    original.require_same_original(&task).unwrap();
    assert!(task
        .require_registered_cold_current_claim(store, &source, &head)
        .is_err());
    store
        .prepare_cold_v2_current_scope(&source, &task, head, &mut scope)
        .unwrap();
    let repeated = store
        .with_cold_owned_current_admission(scope.as_ref().unwrap(), &source, &task, |admission| {
            task.require_cold_owned_handoff(admission)?;
            task.handoff_cold_current_owner(admission)
        })
        .unwrap();
    assert!(repeated.is_none());
    original.require_same_original(&task).unwrap();
    // The exact first scope remains reusable after handoff; no task/owner sender is renewed.
}

#[test]
fn lost_handoff_callback_result_retains_the_exact_owner_and_outside_writer_debt() {
    let (f, head) = captured(0xcc23);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    let original = task.observe_original();
    let mut scope = None;
    store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut scope)
        .unwrap();
    let scope = scope.unwrap();
    let mut retired = None;
    let result: Result<(), AppError> =
        store.with_cold_owned_current_admission(&scope, &source, &task, |admission| {
            retired = task.handoff_cold_current_owner(admission)?;
            Err(AppError::Invalid(
                "injected lost outer handoff result".into(),
            ))
        });
    assert!(result.is_err());
    assert!(retired.is_some());
    drop(retired); // never discard paid debt inside the failed callback/writer
    original.require_same_original(&task).unwrap();
    let repeated = store
        .with_cold_owned_current_admission(&scope, &source, &task, |admission| {
            task.require_cold_owned_handoff(admission)?;
            task.handoff_cold_current_owner(admission)
        })
        .unwrap();
    assert!(repeated.is_none());
    assert!(task
        .require_registered_cold_current_claim(store, &source, &head)
        .is_err());
}

#[test]
fn failed_original_pin_proof_preserves_unset_handoff_and_exact_pending_debt() {
    let (f, head) = captured(0xcc24);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    let mut scope = None;
    store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut scope)
        .unwrap();
    let scope = scope.unwrap();
    let group = head.directory_data().0;
    let pin = group.join("pin.bin");
    let original_pin = std::fs::read(&pin).unwrap();
    let original_seed = std::fs::read(group.join("activation.seed")).unwrap();
    let mut damaged_pin = original_pin.clone();
    *damaged_pin.last_mut().unwrap() ^= 1;
    // Windows may prohibit moving the retained nonempty directory. Change bytes through
    // the original pin inode instead: the real held-file content proof must still refuse.
    std::fs::write(&pin, &damaged_pin).unwrap();
    let charged = f.pool.charged_for_test();
    assert!(store
        .with_cold_owned_current_admission(&scope, &source, &task, |admission| task
            .handoff_cold_current_owner(admission))
        .is_err());
    assert!(task.cold().unwrap().owner.get().is_none());
    task.require_registered_cold_current_claim(store, &source, &head)
        .unwrap();
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    assert_eq!(f.pool.charged_for_test(), charged);
    assert_eq!(
        std::fs::read(group.join("activation.seed")).unwrap(),
        original_seed
    );
    assert_eq!(std::fs::read(&pin).unwrap(), damaged_pin);
    std::fs::write(&pin, original_pin).unwrap();
    let retired = store
        .with_cold_owned_current_admission(&scope, &source, &task, |admission| {
            task.handoff_cold_current_owner(admission)
        })
        .unwrap();
    assert!(retired.is_some());
    drop(retired);
    store
        .with_cold_owned_current_admission(&scope, &source, &task, |admission| {
            task.require_cold_owned_handoff(admission)
        })
        .unwrap();
}

#[test]
fn install_and_receipt_refuse_before_handoff_then_real_binding_uses_same_writer() {
    let (f, head) = captured(0xcc21);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    let mut scope = None;
    store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut scope)
        .unwrap();
    let mut scope = scope.unwrap();
    let mut writer =
        PreparedColdV2WriterRegistration::prepare(store, &source, &task, &scope).unwrap();
    store
        .with_cold_owned_current_admission(&scope, &source, &task, |admission| {
            let mut owner = task.lock_cold_owned_current_owner_namespace(admission)?;
            assert!(owner.install_cold_writer(&mut writer).is_err());
            assert!(owner.lock_archive_root_for_cold_writer(&writer).is_err());
            Ok(())
        })
        .unwrap();
    assert!(store.require_no_pending_v2_group(&f.group).is_err());
    // A refused first installer has not made this original prepaid map redundant.
    // The caller's ordinary outside-writer epilogue must preserve it for SAME-instance
    // retry, rather than leave Runtime(None) with no backing on the next installation.
    let before_drain = f.pool.charged_for_test();
    writer.drain_unused_outside_writers();
    assert_eq!(f.pool.charged_for_test(), before_drain);
    let retired = store
        .with_cold_owned_current_admission(&scope, &source, &task, |admission| {
            task.handoff_cold_current_owner(admission)
        })
        .unwrap();
    drop(retired);
    let writer = RefCell::new(writer);
    let receipt = RefCell::new(PreparedColdV2CurrentWriterReceipt::new(head));
    let installs = Cell::new(0);
    store
        .finish_cold_v2_genesis_current(
            &mut scope,
            &source,
            &task,
            |admission| {
                installs.set(installs.get() + 1);
                let mut writer = writer.borrow_mut();
                let mut owner = task.lock_cold_owned_current_owner_namespace(admission)?;
                owner.install_cold_writer(&mut writer)?;
                // A named guard drops BEFORE the local RefMut. Tail-expression guard
                // temporaries otherwise outlive that writer loan on edition2021.
                let guard = owner.lock_archive_root_for_cold_writer(&writer)?;
                let outcome = receipt.borrow_mut().validate_current(&guard);
                outcome
            },
            |admission, receipt| {
                let writer = writer.borrow();
                let guard = task
                    .lock_cold_owned_current_owner_namespace(admission)?
                    .lock_archive_root_for_cold_writer(&writer)?;
                receipt.validate_under_owned_guard(&guard)
            },
        )
        .unwrap();
    assert_eq!(installs.get(), 1);
    scope.current_binding().unwrap();
    store
        .finish_cold_v2_genesis_current(
            &mut scope,
            &source,
            &task,
            |_| panic!("actual installed callback must never replay"),
            |admission, receipt| {
                let writer = writer.borrow();
                let guard = task
                    .lock_cold_owned_current_owner_namespace(admission)?
                    .lock_archive_root_for_cold_writer(&writer)?;
                receipt.validate_under_owned_guard(&guard)
            },
        )
        .unwrap();
    assert_eq!(installs.get(), 1);
}

#[test]
fn owner_cancellation_after_handoff_cannot_attach_or_renew_the_old_task() {
    let (f, head) = captured(0xcc22);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    let mut scope = None;
    store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut scope)
        .unwrap();
    let scope = scope.unwrap();
    drop(
        store
            .with_cold_owned_current_admission(&scope, &source, &task, |admission| {
                task.handoff_cold_current_owner(admission)
            })
            .unwrap(),
    );
    scope.cancel_original();
    assert!(store
        .with_cold_owned_current_admission(&scope, &source, &task, |_| Ok(()))
        .is_err());
    assert!(PreparedColdV2WriterRegistration::prepare(store, &source, &task, &scope).is_err());
    assert!(PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head).is_err());
    assert!(
        task.cold().unwrap().owner.get().is_some(),
        "OnceLock is never cleared to renew"
    );
}

#[test]
fn concurrent_actual_claims_cannot_steal_the_same_retained_original_head() {
    use std::sync::mpsc;
    use std::time::Duration;
    let (f, head) = captured(0xcc30);
    let f = Arc::new(f);
    let store = f.layout.checked_store().unwrap();
    let (workers, done) = store
        .chat_mount
        .with_writer_gate_for_test(|| {
            let (started_tx, started_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let mut workers = Vec::new();
            for _ in 0..2 {
                let f = f.clone();
                let head = head.clone();
                let started = started_tx.clone();
                let done = done_tx.clone();
                workers.push(std::thread::spawn(move || {
                    started.send(()).unwrap();
                    let result = PreparedV2GenesisGroupNamespace::from_cold_current(
                        f.layout.checked_store().unwrap(),
                        &f.source.borrow_source_data(),
                        head,
                    );
                    let _ = done.send(result);
                }));
            }
            drop(started_tx);
            drop(done_tx);
            started_rx.recv_timeout(Duration::from_secs(30)).unwrap();
            started_rx.recv_timeout(Duration::from_secs(30)).unwrap();
            (workers, done_rx)
        })
        .unwrap();
    let first = done.recv_timeout(Duration::from_secs(30)).unwrap();
    let second = done.recv_timeout(Duration::from_secs(30)).unwrap();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_ne!(first.is_ok(), second.is_ok());
    let task = first.or(second).unwrap();
    task.require_registered_cold_current_claim(store, &f.source.borrow_source_data(), &head)
        .unwrap();
}

#[test]
fn equal_independent_cold_receipt_cannot_replace_original_claim_or_scope() {
    let (f, head) = captured(0xcc31);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let second = store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            capture.capture_current(&store.lock_cold_current_genesis_group(admission, &resources)?)
        })
        .unwrap();
    assert!(!Arc::ptr_eq(&head, &second));
    assert_eq!(head.head_id_data(), second.head_id_data());
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    assert!(task
        .require_registered_cold_current_claim(store, &source, &second)
        .is_err());
    let mut scope = None;
    assert!(store
        .prepare_cold_v2_current_scope(&source, &task, second, &mut scope)
        .is_err());
    assert!(scope.is_none());
    task.require_registered_cold_current_claim(store, &source, &head)
        .unwrap();
    store
        .prepare_cold_v2_current_scope(&source, &task, head, &mut scope)
        .unwrap();
}

#[test]
fn post_receipt_refusal_retries_same_installed_writer_without_install_replay() {
    let (f, head) = captured(0xcc32);
    let store = f.layout.checked_store().unwrap();
    let source = f.source.borrow_source_data();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    let mut scope = None;
    store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut scope)
        .unwrap();
    let mut scope = scope.unwrap();
    drop(
        store
            .with_cold_owned_current_admission(&scope, &source, &task, |admission| {
                task.handoff_cold_current_owner(admission)
            })
            .unwrap(),
    );
    let writer = RefCell::new(
        PreparedColdV2WriterRegistration::prepare(store, &source, &task, &scope).unwrap(),
    );
    let mut prepared_receipt = PreparedColdV2CurrentWriterReceipt::new(head);
    let installs = Cell::new(0);
    let result = store.finish_cold_v2_genesis_current(
        &mut scope,
        &source,
        &task,
        |admission| {
            installs.set(installs.get() + 1);
            let mut writer = writer.borrow_mut();
            let mut owner = task.lock_cold_owned_current_owner_namespace(admission)?;
            owner.install_cold_writer(&mut writer)?;
            let guard = owner.lock_archive_root_for_cold_writer(&writer)?;
            prepared_receipt.validate_current(&guard)
        },
        |admission, receipt| {
            let writer = writer.borrow();
            receipt.validate_under_owned_guard(
                &task
                    .lock_cold_owned_current_owner_namespace(admission)?
                    .lock_archive_root_for_cold_writer(&writer)?,
            )?;
            Err(AppError::Invalid(
                "injected refusal after real receipt, before binding".into(),
            ))
        },
    );
    assert!(result.is_err());
    assert!(scope.current_binding().is_err());
    store
        .finish_cold_v2_genesis_current(
            &mut scope,
            &source,
            &task,
            |_| panic!("latched real receipt must not replay install"),
            |admission, receipt| {
                let writer = writer.borrow();
                let guard = task
                    .lock_cold_owned_current_owner_namespace(admission)?
                    .lock_archive_root_for_cold_writer(&writer)?;
                receipt.validate_under_owned_guard(&guard)
            },
        )
        .unwrap();
    assert_eq!(installs.get(), 1);
    scope.current_binding().unwrap();
}

#[test]
fn a_new_physical_mount_requires_a_new_paid_head_and_never_adopts_the_old_task() {
    let (f, head) = captured(0xcc33);
    let policy = f
        .layout
        .checked_store()
        .unwrap()
        .current_layout_policy
        .clone()
        .unwrap();
    let old_task = PreparedV2GenesisGroupNamespace::from_cold_current(
        f.layout.checked_store().unwrap(),
        &f.source.borrow_source_data(),
        head.clone(),
    )
    .unwrap();
    let ColdFixture {
        layout,
        source,
        pool,
        group,
        signer,
        mut rng,
        directory,
    } = f;
    drop(source); // actual SDK/read/session resources drain before the physical reopen
    drop(layout);
    let mut opening = crate::store::PreparedV2CurrentLayoutStore::open(
        directory.path(),
        b"genuine task-free cold group reads",
        &mut rng,
        policy,
    )
    .unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    let source = store
        .prepare_genesis_seed(
            layout
                .read_saved_bootstrap_source_data(&group, signer)
                .unwrap(),
            crate::store::GenesisSeedBudget::new(
                8 * 1024 * 1024 + 64 * 1024,
                8 * 1024 * 1024,
                8 * 1024 * 1024,
                1024 * 1024,
                16 * 1024 * 1024,
            )
            .unwrap(),
        )
        .unwrap();
    let source_ref = source.borrow_source_data();
    assert!(old_task.retains_original(store, &source_ref).is_err());
    assert!(old_task
        .require_registered_cold_current_claim(store, &source_ref, &head)
        .is_err());
    assert!(PreparedV2GenesisGroupNamespace::from_cold_current(store, &source_ref, head).is_err());
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source_ref).unwrap();
    let resources = capture.read_resources();
    let new_head = store
        .with_cold_v2_genesis_current_read(&source_ref, |admission| {
            capture.capture_current(&store.lock_cold_current_genesis_group(admission, &resources)?)
        })
        .unwrap();
    let new_task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source_ref, new_head.clone())
            .unwrap();
    new_task
        .require_registered_cold_current_claim(store, &source_ref, &new_head)
        .unwrap();
    drop(new_task);
    drop(old_task);
    drop(source);
    drop(layout);
    assert!(
        pool.charged_for_test() > 0,
        "actual retained cold receipts still own their fees"
    );
}

#[test]
fn cold_and_warm_share_sixteen_slots_and_only_actual_owner_handoff_frees_cold_slot() {
    use super::super::tests::{additional_saved_origin, archive, SECRET};
    use crate::store::critical_namespaces::{
        archive_current_namespace_codec as layout_codec, read_pool::CriticalCatalogReadPool,
    };
    use crate::store::persistence::v2_genesis_activation::PreparedV2GenesisActivation;
    use crate::store::{
        CanonicalFilenameInventoryPolicy, GenesisSeedBudget, PreparedV2CurrentLayoutStore,
        V2CurrentLayoutPolicy,
    };
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    let directory = tempfile::tempdir().unwrap();
    let pool = CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xcc40);
    drop(
        ServerStore::open(directory.path(), SECRET, &mut rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap(),
    );
    // Generate every source with the real producer before ID3; original source limits and
    // full source/debit files survive each physical reopen, never a synthetic GroupId list.
    let mut origins = Vec::new();
    for index in 0..17u8 {
        origins.push(additional_saved_origin(
            directory.path(),
            &pool,
            &mut rng,
            0xcc4000 + u64::from(index),
            index + 1,
        ));
    }
    let policy = || {
        V2CurrentLayoutPolicy::new(
            archive(&pool),
            CanonicalFilenameInventoryPolicy::new(40, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
            4 * layout_codec::MAX_DECLARATION_FRAME,
        )
        .unwrap()
    };
    let budget = GenesisSeedBudget::new(
        1024 * 1024,
        512 * 1024,
        512 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    let mut opening =
        PreparedV2CurrentLayoutStore::open(directory.path(), SECRET, &mut rng, policy()).unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    {
        let store = layout.checked_store().unwrap();
        let seed = store
            .prepare_genesis_seed(
                layout
                    .read_saved_bootstrap_source_data(&origins[0].0, origins[0].1)
                    .unwrap(),
                budget,
            )
            .unwrap();
        let source = seed.borrow_source_data();
        let mut task = layout.prepare_genesis_group_namespace(&source).unwrap();
        task.retry_directory(store, &source).unwrap();
        let mut activation =
            PreparedV2GenesisActivation::prepare(store, &source, &mut rng).unwrap();
        let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
        let saved = store
            .with_vacant_v2_genesis_admission(&source, |admission| {
                activation.save_seed(&task.lock_vacant(admission)?)
            })
            .unwrap();
        let mut scope = None;
        store
            .reserve_v2_genesis_initializing(&source, &task, saved, &mut scope, &buffers, &mut rng)
            .unwrap();
        let scope = scope.as_mut().unwrap();
        let mut writer = crate::store::owner_evidence::canonical::v2_registration::PreparedV2WriterRegistration
            ::prepare(store, &source, &task, scope).unwrap();
        store
            .finish_v2_genesis_current(scope, &source, &task, &buffers, &mut rng, |admission, _| {
                let mut owner = task.lock_initializing_owner_namespace(admission)?;
                owner.install_writer(&mut writer)?;
                activation.save_head(&owner.lock_archive_root_for_writer(&writer)?)
            })
            .unwrap();
    }
    drop(layout);
    let mut opening =
        PreparedV2CurrentLayoutStore::open(directory.path(), SECRET, &mut rng, policy()).unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    let mut seeds = Vec::new();
    for (group, signer) in &origins {
        seeds.push(
            store
                .prepare_genesis_seed(
                    layout
                        .read_saved_bootstrap_source_data(group, *signer)
                        .unwrap(),
                    budget,
                )
                .unwrap(),
        );
    }
    let source = seeds[0].borrow_source_data();
    let mut capture = PreparedColdV2GenesisCapture::prepare(store, &source).unwrap();
    let resources = capture.read_resources();
    let head = store
        .with_cold_v2_genesis_current_read(&source, |admission| {
            capture.capture_current(&store.lock_cold_current_genesis_group(admission, &resources)?)
        })
        .unwrap();
    let task =
        PreparedV2GenesisGroupNamespace::from_cold_current(store, &source, head.clone()).unwrap();
    let mut warm = Vec::new();
    for seed in &seeds[1..16] {
        warm.push(
            layout
                .prepare_genesis_group_namespace(&seed.borrow_source_data())
                .unwrap(),
        );
    }
    assert!(layout
        .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
        .is_err());
    let mut scope = None;
    store
        .prepare_cold_v2_current_scope(&source, &task, head.clone(), &mut scope)
        .unwrap();
    assert!(
        layout
            .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
            .is_err(),
        "actual new owner alone cannot retire the pending claim"
    );
    let retired = store
        .with_cold_owned_current_admission(scope.as_ref().unwrap(), &source, &task, |admission| {
            task.handoff_cold_current_owner(admission)
        })
        .unwrap();
    assert!(retired.is_some());
    drop(retired);
    let next = layout
        .prepare_genesis_group_namespace(&seeds[16].borrow_source_data())
        .unwrap();
    task.require_registered_cold_current_claim(store, &source, &head)
        .unwrap_err();
    store
        .with_cold_owned_current_admission(scope.as_ref().unwrap(), &source, &task, |admission| {
            task.require_cold_owned_handoff(admission)
        })
        .unwrap();
    drop(next);
    drop(warm);
}
