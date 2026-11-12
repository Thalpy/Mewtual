//! Real SAME-nextA staged continuation from WC. FirstClear/firstA stay unchanged.
use super::*;
use std::io::Write;
const PREFIX: usize = 13;
fn staged_fixture(
    cut: V2TextRetainTestCut,
    prefix: Option<usize>,
    number: u64,
) -> (NextCreateFixture, std::path::PathBuf, Vec<u8>) {
    let mut t = next_create_fixture(0, number);
    prepare_next_a(&mut t, number + 10);
    let (path, expected) = t
        .transaction
        .as_ref()
        .unwrap()
        .original_completed_wal_output_for_test()
        .unwrap();
    // An unknown stage is never adopted even if it occupies the correct fixed name.
    let unknown = b"unknown stage, preserve without issuing write permission";
    std::fs::write(&path, unknown).unwrap();
    assert!(retry(&mut t).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), unknown);
    std::fs::remove_file(&path).unwrap();
    if let Some(len) = prefix {
        let target = path.clone();
        let bytes = expected[..len].to_vec();
        t.transaction.as_mut().unwrap().set_work_pause_for_test(
            cut,
            Box::new(move || {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(target)
                    .unwrap();
                file.write_all(&bytes).unwrap();
                file.sync_all().unwrap();
            }),
        );
    }
    t.transaction.as_mut().unwrap().set_work_fault_for_test(cut);
    let admitted = next_stage(&t);
    assert!(t.transaction.as_mut().unwrap().run_stage(admitted).is_err());
    let len = if cut == V2TextRetainTestCut::WalStaged {
        expected.len()
    } else {
        prefix.unwrap_or(0)
    };
    assert_eq!(std::fs::read(&path).unwrap(), expected[..len]);
    assert!(!t.lane.workspace_active_for_test());
    assert!(
        !t.transaction.as_ref().unwrap().has_ready_data()
            && !t.transaction.as_ref().unwrap().has_saved_data()
    );
    (t, path, expected.to_vec())
}
fn retry(t: &mut NextCreateFixture) -> Result<(), AppError> {
    let store = t.original.layout.checked_store()?;
    let owner = t.original.owner.borrow_current()?;
    let resources = t.transaction.as_ref().unwrap().resources().clone();
    let mut physical = Some(PreparedV2TextPhysicalLease::prepare(
        store,
        owner.source(),
        &resources,
    )?);
    let mut admitted = None;
    let admission = store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.admit_text_completed_retain_stage_into(
            &t.lane,
            t.predecessor.as_ref().unwrap(),
            t.transaction.as_ref().unwrap(),
            &mut physical,
            &mut admitted,
        )
    });
    drop(owner);
    if let Some(marker) = admitted {
        t.transaction.as_mut().unwrap().run_stage(marker)?;
        admission
    } else {
        admission
    }
}
#[test]
fn actual_zero_partial_and_complete_next_wa_stage_retry_same_original_then_select() {
    for (index, (cut, prefix)) in [
        (V2TextRetainTestCut::WalCreated, None),
        (V2TextRetainTestCut::WalCreated, Some(PREFIX)),
        (V2TextRetainTestCut::WalStaged, None),
    ]
    .into_iter()
    .enumerate()
    {
        let (mut t, path, expected) = staged_fixture(cut, prefix, 0x83500 + index as u64);
        let store = t.original.layout.checked_store().unwrap();
        let owner = t.original.owner.borrow_current().unwrap();
        t.transaction
            .as_mut()
            .unwrap()
            .prepare_candidates(store, owner.source(), &mut NeverRng)
            .unwrap();
        drop(owner);
        retry(&mut t).unwrap();
        assert!(!path.exists());
        let target = t
            .original
            .owner
            .borrow_current()
            .unwrap()
            .source()
            .source()
            .capture
            .target
            .to_path_buf();
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), expected);
        let commit = next_commit(&t);
        t.transaction.as_mut().unwrap().run_commit(commit).unwrap();
        assert!(t
            .transaction
            .as_ref()
            .unwrap()
            .saved_retained_action()
            .is_ok());
        assert!(!t.lane.workspace_active_for_test());
    }
}
#[test]
fn actual_owned_next_wa_stage_preserves_missing_foreign_bad_prefix_and_prior_wc_tamper() {
    let (mut t, path, expected) =
        staged_fixture(V2TextRetainTestCut::WalCreated, Some(PREFIX), 0x83510);
    let original = expected[..PREFIX].to_vec();
    let held = path.with_extension("next-owned-original");
    std::fs::rename(&path, &held).unwrap();
    assert!(retry(&mut t).is_err());
    assert!(!path.exists());
    std::fs::rename(&held, &path).unwrap();
    std::fs::rename(&path, &held).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(retry(&mut t).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&held, &path).unwrap();
    let mut bad = original.clone();
    bad[2] ^= 1;
    std::fs::write(&path, &bad).unwrap();
    assert!(retry(&mut t).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bad);
    let oversized = vec![0x7a; expected.len() + 1];
    std::fs::write(&path, &oversized).unwrap();
    assert!(retry(&mut t).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), oversized);
    std::fs::write(&path, &original).unwrap();
    // Full actual WC is compared after admission BEFORE any prefix suffix too.
    let marker = next_stage(&t);
    let main = t
        .original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .join("text.wal");
    let wc = std::fs::read(&main).unwrap();
    let mut changed = wc.clone();
    let at = changed.len() / 2;
    changed[at] ^= 1;
    std::fs::write(&main, &changed).unwrap();
    assert!(t.transaction.as_mut().unwrap().run_stage(marker).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read(&main).unwrap(), changed);
    std::fs::write(&main, wc).unwrap();
    retry(&mut t).unwrap();
    let commit = next_commit(&t);
    t.transaction.as_mut().unwrap().run_commit(commit).unwrap();
}
