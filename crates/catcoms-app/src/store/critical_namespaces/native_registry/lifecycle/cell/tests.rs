//! Real encrypted selected3 files and owning producer. No native command,
//! SDK or joined-member authority is manufactured by these storage tests.
use super::super::wal::tests::mounted;
use super::*;
use crate::store::critical_namespaces::native_registry::{
    operational::upgrade_source::owning::{self, at_edge, Edge},
    preparation::tests::{current_policy, SECRET},
};
use crate::store::persistence::native_registry::operational as cuts;
use crate::store::NativeIndexStore;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use std::{
    fs,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

struct NeverRng;
impl CryptoRng for NeverRng {}
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("retained/cold operation must not draw RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("retained/cold operation must not draw RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("retained/cold operation must not draw RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("retained/cold operation must not draw RNG")
    }
}
fn file(path: &Path, name: &str) -> PathBuf {
    path.join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(name)
}
fn selected(owner: &owning::Store) -> data::Index {
    let handler = owner.handler_for_test();
    let store = owner.actual_store_for_test().unwrap();
    let inventory = UpgradeInventory::capture(store, handler.debit(), handler.pool()).unwrap();
    preparation::decode_selected_data(store, &inventory)
        .unwrap()
        .0
}
pub(in crate::store::critical_namespaces::native_registry) fn request(
    owner: &owning::Store,
) -> rename::Request {
    let index = selected(owner);
    let row = &index.rows[0];
    rename::Request {
        id: row.id,
        identity: row.identity,
        expected_revision: row.metadata_revision,
        name: "my local group name".into(),
    }
}
fn reopen(path: &Path) -> owning::Store {
    NativeIndexStore::open_lifecycle3_for_test(path, SECRET, &mut NeverRng, current_policy())
        .unwrap()
}

#[test]
fn lifecycle3_cell_warm_retry_keeps_one_candidate_and_cas_is_not_a_receipt() {
    for (n, cut) in [
        cuts::Fault::Index3PartialStage,
        cuts::Fault::IndexStage,
        cuts::Fault::IndexRename,
        cuts::Fault::Index3Covered,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc601 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let input = request(&owner);
        let handler = owner.handler_for_test();
        let old = selected(&owner);
        cuts::arm_fault(cut);
        assert!(owner.rename_metadata_data(&input, &mut rng).is_err());
        assert!(cuts::take_fault_seen());
        assert!(handler.require_idle_cached().is_err());
        assert!(owner.row_count_data().is_err());
        let changed = rename::Request {
            name: "different retry".into(),
            ..request_for(&input)
        };
        assert!(owner.rename_metadata_data(&changed, &mut NeverRng).is_err());
        let frozen_stage = (cut == cuts::Fault::IndexStage).then(|| {
            fs::read(file(
                dir.path(),
                super::super::super::preparation::INDEX_STAGE_NAME,
            ))
            .unwrap()
        });
        let (covered, debt) = owner.rename_metadata_data(&input, &mut NeverRng).unwrap();
        let debt = debt.unwrap();
        assert_eq!(covered.revision, input.expected_revision + 1);
        assert_eq!(
            fs::read(file(
                dir.path(),
                super::super::super::preparation::INDEX_NAME
            ))
            .unwrap(),
            *debt._transaction.frame
        );
        if let Some(stage) = frozen_stage {
            assert_eq!(stage, *debt._transaction.frame);
        }
        assert!(
            owner.row_count_data().is_err(),
            "actual claim covers outgoing debt"
        );
        assert!(handler.require_idle_cached().is_err());
        drop(debt);
        handler.require_idle_cached().unwrap();
        assert_eq!(owner.row_count_data().unwrap(), old.rows.len());
        assert!(
            owner.rename_metadata_data(&input, &mut NeverRng).is_err(),
            "name equality cannot acknowledge stale revision"
        );
        let exact_current = rename::Request {
            expected_revision: covered.revision,
            ..request_for(&input)
        };
        let (observed, no_debt) = owner
            .rename_metadata_data(&exact_current, &mut NeverRng)
            .unwrap();
        assert_eq!(observed, covered);
        assert!(no_debt.is_none());
        let new = selected(&owner);
        assert_eq!(new.origin.generation, old.origin.generation);
        assert_eq!(new.origin.frame, old.origin.frame);
        assert_eq!(new.last_issued_id, old.last_issued_id);
        assert_eq!(new.rows[0].name, input.name);
        assert_eq!(new.rows[0].metadata_revision, covered.revision);
        assert!(new.rows[0].backend == old.rows[0].backend);
        drop(owner);
        let mut cold = reopen(dir.path());
        assert!(cold.rename_metadata_data(&input, &mut NeverRng).is_err());
        assert_eq!(selected(&cold).rows[0].name, input.name);
    }
}
fn request_for(input: &rename::Request) -> rename::Request {
    rename::Request {
        id: input.id,
        identity: input.identity,
        expected_revision: input.expected_revision,
        name: input.name.clone(),
    }
}

#[test]
fn lifecycle3_cell_actual_cold_open_recovers_every_supported_normal_cut_without_rng() {
    for (n, cut) in [
        cuts::Fault::Index3PartialStage,
        cuts::Fault::IndexStage,
        cuts::Fault::IndexRename,
        cuts::Fault::Index3Covered,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc611 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let input = request(&owner);
        let original = selected(&owner).rows[0].name.clone();
        cuts::arm_fault(cut);
        assert!(owner.rename_metadata_data(&input, &mut rng).is_err());
        assert!(cuts::take_fault_seen());
        drop(owner);
        let mut cold = reopen(dir.path());
        assert_eq!(cold.row_count_data().unwrap(), 1);
        let result = selected(&cold);
        if cut == cuts::Fault::Index3PartialStage {
            assert_eq!(result.rows[0].name, original);
            assert_eq!(result.rows[0].metadata_revision, 0);
        } else {
            assert_eq!(result.rows[0].name, input.name);
            assert_eq!(result.rows[0].metadata_revision, 1);
        }
        assert!(!file(
            dir.path(),
            super::super::super::preparation::INDEX_STAGE_NAME
        )
        .exists());
    }
}

#[test]
fn lifecycle3_cell_close_on_either_admission_side_never_exposes_warm_success() {
    for (n, edge) in [Edge::BeforeMetadataAdmission, Edge::AfterMetadataCommit]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc621 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let input = request(&owner);
        let before = fs::read(file(
            dir.path(),
            super::super::super::preparation::INDEX_NAME,
        ))
        .unwrap();
        let closing = owner.handler_for_test();
        at_edge(edge, Box::new(move || closing.close()));
        assert!(owner.rename_metadata_data(&input, &mut rng).is_err());
        assert!(owner.row_count_data().is_err());
        if edge == Edge::BeforeMetadataAdmission {
            assert_eq!(
                fs::read(file(
                    dir.path(),
                    super::super::super::preparation::INDEX_NAME
                ))
                .unwrap(),
                before
            );
            assert!(!file(
                dir.path(),
                super::super::super::preparation::INDEX_STAGE_NAME
            )
            .exists());
        }
        drop(owner);
        let cold = reopen(dir.path());
        assert_eq!(
            selected(&cold).rows[0].metadata_revision,
            u64::from(edge == Edge::AfterMetadataCommit)
        );
    }
}

#[test]
fn lifecycle3_cell_actual_post_effect_unwind_poison_refuses_until_cold_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc631);
    let mut owner = mounted(dir.path(), &mut rng);
    let input = request(&owner);
    at_edge(
        Edge::AfterMetadataCommit,
        Box::new(|| panic!("actual admitted effect boundary")),
    );
    assert!(catch_unwind(AssertUnwindSafe(
        || owner.rename_metadata_data(&input, &mut rng)
    ))
    .is_err());
    assert!(owner.row_count_data().is_err());
    assert!(owner.rename_metadata_data(&input, &mut NeverRng).is_err());
    drop(owner);
    let cold = reopen(dir.path());
    assert_eq!(selected(&cold).rows[0].name, input.name);
}

#[test]
fn lifecycle3_cell_completed_debt_holds_real_claim_and_physical_exclusion_through_io_drop() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc641);
    let mut owner = mounted(dir.path(), &mut rng);
    let input = request(&owner);
    let handler = owner.handler_for_test();
    let (_, debt) = owner.rename_metadata_data(&input, &mut rng).unwrap();
    let mut debt = debt.unwrap();
    let observed_busy = Arc::new(AtomicBool::new(false));
    let recorded = observed_busy.clone();
    debt._transaction.io.observe_retirement(Box::new(move || {
        recorded.store(handler.require_idle_cached().is_err(), Ordering::SeqCst);
    }));
    assert!(owner.row_count_data().is_err());
    drop(debt);
    assert!(observed_busy.load(Ordering::SeqCst));
    owner.row_count_data().unwrap();

    let next = rename::Request {
        expected_revision: 1,
        name: "second name".into(),
        ..request_for(&input)
    };
    let (_, debt) = owner.rename_metadata_data(&next, &mut rng).unwrap();
    let mut debt = debt.unwrap();
    let path = dir.path().to_path_buf();
    let excluded = Arc::new(AtomicBool::new(false));
    let recorded = excluded.clone();
    debt._transaction.io.observe_retirement(Box::new(move || {
        let denied = NativeIndexStore::open_lifecycle3_for_test(
            &path,
            SECRET,
            &mut NeverRng,
            current_policy(),
        );
        recorded.store(
            matches!(
                denied,
                Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
            ),
            Ordering::SeqCst,
        );
    }));
    drop(owner);
    drop(debt);
    assert!(
        excluded.load(Ordering::SeqCst),
        "finite original physical lease survives destination drop"
    );
    reopen(dir.path());
}

#[test]
fn lifecycle3_cell_last_tuple_proof_rejects_equal_byte_marker_or_catalog_before_selection() {
    for (n, marker) in [true, false].into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc651 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let input = request(&owner);
        let index = file(dir.path(), super::super::super::preparation::INDEX_NAME);
        let old_index = fs::read(&index).unwrap();
        let target = if marker {
            file(dir.path(), super::super::super::preparation::MARKER_NAME)
        } else {
            dir.path()
                .join(crate::store::critical_namespaces::Slot::Current.name())
        };
        let displaced = target.with_extension("original-for-test");
        cuts::before_target_proof(
            cuts::RenameTarget::Index,
            Box::new(move || {
                let bytes = fs::read(&target).unwrap();
                fs::rename(&target, displaced).unwrap();
                fs::write(&target, bytes).unwrap();
            }),
        );
        assert!(owner.rename_metadata_data(&input, &mut rng).is_err());
        assert!(cuts::take_rename_hook_seen());
        assert_eq!(
            fs::read(&index).unwrap(),
            old_index,
            "changed static object must prevent index promotion"
        );
        assert!(owner.rename_metadata_data(&input, &mut NeverRng).is_err());
        assert_eq!(fs::read(&index).unwrap(), old_index);
    }
}

#[test]
fn lifecycle3_cell_cold_exposure_rejects_replacement_after_rename_or_cleanup_inspection() {
    for (n, rename) in [true, false].into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc661 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        if rename {
            let input = request(&owner);
            cuts::arm_fault(cuts::Fault::IndexStage);
            assert!(owner.rename_metadata_data(&input, &mut rng).is_err());
            assert!(cuts::take_fault_seen());
        } else {
            fs::write(
                file(
                    dir.path(),
                    super::super::super::preparation::INDEX_STAGE_NAME,
                ),
                fs::read(file(
                    dir.path(),
                    super::super::super::preparation::INDEX_NAME,
                ))
                .unwrap(),
            )
            .unwrap();
        }
        drop(owner);
        let index = file(dir.path(), super::super::super::preparation::INDEX_NAME);
        let displaced = index.with_extension("original-for-test");
        at_edge(
            Edge::AfterInspection,
            Box::new(move || {
                let same = fs::read(&index).unwrap();
                fs::rename(&index, displaced).unwrap();
                fs::write(index, same).unwrap();
            }),
        );
        let refused = NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
        );
        assert!(
            refused.is_err(),
            "same-byte replacement cannot be adopted by a fresh post-repair capture"
        );
    }
}

#[test]
fn lifecycle3_cell_cleanup_repeats_barrier_and_never_reports_visible_unlink_as_complete() {
    use crate::store::persistence::native_registry::lifecycle::upgrade::index::IndexStageCleanup;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc671);
    let owner = mounted(dir.path(), &mut rng);
    let handler = owner.handler_for_test();
    let store = owner.actual_store_for_test().unwrap();
    let index = file(dir.path(), super::super::super::preparation::INDEX_NAME);
    let stage = file(
        dir.path(),
        super::super::super::preparation::INDEX_STAGE_NAME,
    );
    let selected = fs::read(&index).unwrap();
    let mut inputs = vec![selected.clone()];
    inputs.extend((0..40).map(|length| vec![0; length]));
    let mut bad_mac = selected.clone();
    *bad_mac.last_mut().unwrap() ^= 1;
    inputs.push(bad_mac);
    for bytes in inputs {
        for cut in [
            cuts::Fault::IndexStageUnlink,
            cuts::Fault::Index3CleanupCovered,
        ] {
            fs::write(&stage, &bytes).unwrap();
            let inventory =
                UpgradeInventory::capture(store, handler.debit(), handler.pool()).unwrap();
            let proof = wal::classify(store, &inventory).unwrap();
            let mut io = IndexStageCleanup::prepare(store, inventory, proof).unwrap();
            cuts::arm_fault(cut);
            assert!(io.run(store).is_err());
            assert!(cuts::take_fault_seen());
            assert!(!stage.exists());
            assert!(
                io.require_complete(store).is_err(),
                "visible removal alone is not covering completion"
            );
            // An unlink flag cannot skip the real directory barrier on retry.
            crate::store::persistence::native_registry::lifecycle::upgrade::index::take_cleanup_covers();
            io.run(store).unwrap();
            assert_eq!(crate::store::persistence::native_registry::lifecycle::upgrade::index::take_cleanup_covers(), 1);
            io.require_complete(store).unwrap();
            assert_eq!(fs::read(&index).unwrap(), selected);
        }
    }
}

mod compatibility;

mod refresh;
