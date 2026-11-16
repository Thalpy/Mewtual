//! A live warm SavedSeed is not group vacancy after a genuine no-row cold claim. Both
//! serializer orderings use actual readers/tasks/receipts and complete their winning path.
#![cfg(windows)]
use super::*;
use crate::store::owner_evidence::canonical::v2_registration::cold_initializing::PreparedColdV2InitializingWriterRegistration;
use crate::store::persistence::v2_genesis_activation::{
    initializing_repair::PreparedColdV2GenesisRepair,
    pre_initializing::{PreparedBornV2GenesisRepairCapture, PreparedColdV2PreInitializingCapture},
    PreparedV2GenesisActivation,
};

#[test]
fn live_warm_seed_and_new_preclaim_serialize_both_row1_birth_orderings() {
    for cold_first in [true, false] {
        let f = fixture(0xcb90 + u64::from(cold_first));
        let store = f.layout.checked_store().unwrap();
        let warm_source = f.seed.borrow_source_data();
        let mut rng = ChaCha20Rng::seed_from_u64(0xcb90 + u64::from(cold_first));
        let mut warm_task = f
            .layout
            .prepare_genesis_group_namespace(&warm_source)
            .unwrap();
        warm_task.retry_directory(store, &warm_source).unwrap();
        let mut activation =
            PreparedV2GenesisActivation::prepare(store, &warm_source, &mut rng).unwrap();
        let saved = store
            .with_vacant_v2_genesis_admission(&warm_source, |a| {
                activation.save_seed(&warm_task.lock_vacant(a)?)
            })
            .unwrap();
        // A truly separate paid read+SDK on the SAME mount. No warm proof adoption.
        let read = f
            .layout
            .read_saved_bootstrap_source_data(&f.group, f.signer)
            .unwrap();
        let cold_seed = store.prepare_genesis_seed(read, sdk_budget()).unwrap();
        let cold_source = cold_seed.borrow_source_data();
        let mut capture =
            PreparedColdV2PreInitializingCapture::prepare(store, &cold_source).unwrap();
        let resources = capture.read_resources();
        let origin = store
            .with_cold_v2_genesis_vacant_read(&cold_source, |a| {
                let guard = store.lock_cold_vacant_genesis_group(a, &resources)?;
                capture.capture_vacant(&guard)
            })
            .unwrap();
        let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
        let mut warm_scope = None;
        if cold_first {
            let mut task = PreparedV2GenesisGroupNamespace::from_cold_pre_initializing(
                store,
                &cold_source,
                origin.clone(),
            )
            .unwrap();
            // Former defect inserted Warm owner+row despite this actual foreign slot, then
            // both genuine callers stranded. Fixed guard rejects BEFORE warm row effects.
            assert!(store
                .reserve_v2_genesis_initializing(
                    &warm_source,
                    &warm_task,
                    saved,
                    &mut warm_scope,
                    &buffers,
                    &mut NeverRng
                )
                .is_err());
            assert!(warm_scope.is_none());
            let mut born = None;
            store
                .prepare_cold_v2_genesis_birth(&cold_source, &task, origin.clone(), &mut born)
                .unwrap();
            let mut born_capture =
                PreparedBornV2GenesisRepairCapture::prepare(store, &cold_source, origin).unwrap();
            let loan = born_capture.read_resources();
            store
                .save_born_v2_genesis_row1(
                    born.as_mut().unwrap(),
                    &cold_source,
                    &task,
                    &buffers,
                    &mut rng,
                    |a| {
                        let guard = store.lock_durable_born_v2_genesis_group(a, &loan)?;
                        born_capture.capture_born(&guard)
                    },
                )
                .unwrap();
            let retired = store
                .with_born_v2_genesis_admission(born.as_ref().unwrap(), &cold_source, &task, |a| {
                    task.handoff_born_v2_genesis_owner(a)
                })
                .unwrap();
            let mut scope = None;
            let old_scope = store
                .convert_born_to_initializing(&mut born, &mut scope, &cold_source, &task)
                .unwrap();
            let old_task = task
                .normalize_born_successor(store, &cold_source, scope.as_ref().unwrap())
                .unwrap();
            drop(retired);
            drop(old_scope);
            drop(old_task);
            drop(born_capture);
            drop(capture);
            drop(resources);
            let scope = scope.as_mut().unwrap();
            let mut writer = PreparedColdV2InitializingWriterRegistration::prepare(
                store,
                &cold_source,
                &task,
                scope,
            )
            .unwrap();
            let mut worker = PreparedColdV2GenesisRepair::new(scope.seed().clone());
            store
                .finish_cold_v2_initializing_current(
                    scope,
                    &cold_source,
                    &task,
                    &buffers,
                    &mut rng,
                    |a| {
                        let mut owner = task.lock_cold_initializing_owner_namespace(a)?;
                        owner.install_initializing_writer(&mut writer)?;
                        let guard = owner.lock_archive_root_for_initializing_writer(&writer)?;
                        worker.repair(&guard)
                    },
                )
                .unwrap();
            writer.drain_unused_outside_writers();
            drop(buffers);
            store
                .validate_cold_repaired_v2_genesis_current(scope, &cold_source, &task)
                .unwrap();
        } else {
            store
                .reserve_v2_genesis_initializing(
                    &warm_source,
                    &warm_task,
                    saved,
                    &mut warm_scope,
                    &buffers,
                    &mut rng,
                )
                .unwrap();
            // The physical receipt was genuinely captured while row0, BEFORE warm won.
            // Claim installation must recheck actual Runtime, not trust captured absence.
            let charged = f.pool.charged_for_test();
            assert!(PreparedV2GenesisGroupNamespace::from_cold_pre_initializing(
                store,
                &cold_source,
                origin,
            )
            .is_err());
            assert!(store.require_no_pending_v2_group(&f.group).is_ok());
            assert_eq!(f.pool.charged_for_test(), charged);
            let mut entered = false;
            assert!(store
                .with_cold_v2_genesis_vacant_read(&cold_source, |a| {
                    entered = true;
                    let guard = store.lock_cold_vacant_genesis_group(a, &resources)?;
                    capture.capture_vacant(&guard)
                })
                .is_err());
            assert!(
                !entered,
                "actual warm row/owner wins before any cold capture callback"
            );
            let scope = warm_scope.as_mut().unwrap();
            let mut writer=crate::store::owner_evidence::canonical::v2_registration::PreparedV2WriterRegistration::prepare(store,&warm_source,&warm_task,scope).unwrap();
            store
                .finish_v2_genesis_current(
                    scope,
                    &warm_source,
                    &warm_task,
                    &buffers,
                    &mut rng,
                    |a, _| {
                        let mut owner = warm_task.lock_initializing_owner_namespace(a)?;
                        owner.install_writer(&mut writer)?;
                        let guard = owner.lock_archive_root_for_writer(&writer)?;
                        activation.save_head(&guard)
                    },
                )
                .unwrap();
            writer.drain_unused_outside_writers();
            drop(buffers);
            store
                .validate_v2_genesis_current(scope, &warm_source, &warm_task)
                .unwrap();
        }
    }
}
