//! Resume only the same genuinely admitted B Stage's recorded outputs. Each cut
//! creates and writes a real prefix while the original native lease is still held.
//! These tests never construct a receipt, repair permission, or replacement plan.
use super::*;
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::atomic::AtomicUsize;

const PREFIX: usize = 13;

fn run_without_key(
    f: &mut AcceptedFixture,
    admitted: AdmittedV2TextAcceptedCreateStage,
) -> Result<(), AppError> {
    let calls = Arc::new(AtomicUsize::new(0));
    let result = PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
        calls.clone(),
        None,
        || f.accepted.run_stage(admitted),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    result
}

fn assert_idle(f: &AcceptedFixture) {
    assert!(!f.lane.workspace_active_for_test());
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(
            f.original.layout.checked_store().unwrap(),
        ),
        0,
    );
}

fn partial_fixture(cut: V2TextAcceptedCreateTestCut, len: usize, number: u64) -> AcceptedFixture {
    let mut f = accepted_fixture(0, number);
    let (path, expected) = f.accepted.original_output_for_test(cut).unwrap();
    assert!(len < expected.len());
    f.accepted.set_partial_write_fault_for_test(cut, len);
    let stage = stage_b(&f);
    assert!(run_without_key(&mut f, stage).is_err());
    assert_eq!(std::fs::read(path).unwrap(), expected[..len]);
    assert!(!f.accepted.has_ready_data() && !f.accepted.has_saved_data());
    assert_idle(&f);
    f
}

/// A failed outer admission may still have installed a real marker before its late
/// check. Always finish that finite work before interpreting the returned refusal.
fn retry_stage(f: &mut AcceptedFixture) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = f.accepted.resources().clone();
    let mut lease = Some(PreparedV2TextPhysicalLease::prepare(
        store,
        owner.source(),
        &resources,
    )?);
    let mut admitted = None;
    let admission = store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.admit_text_accepted_create_stage_into(
            &f.lane,
            &f.predecessor,
            &f.accepted,
            &mut lease,
            &mut admitted,
        )
    });
    drop(owner);
    drop(lease);
    if let Some(stage) = admitted {
        run_without_key(f, stage)?;
    }
    admission
}

fn no_new_nonce(f: &mut AcceptedFixture) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    f.accepted
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
}

#[test]
fn actual_same_b_wal_p_h_u_and_head_prefixes_finish_original_bytes_without_another_key_or_nonce() {
    for (index, (cut, len, prefix)) in [
        (V2TextAcceptedCreateTestCut::WalCreated, PREFIX, None),
        (
            V2TextAcceptedCreateTestCut::ArtifactCreated,
            PREFIX,
            Some("p-"),
        ),
        (
            V2TextAcceptedCreateTestCut::ArchiveCreated,
            PREFIX,
            Some("h-"),
        ),
        (
            V2TextAcceptedCreateTestCut::PublicationCreated,
            PREFIX,
            Some("u-"),
        ),
        (V2TextAcceptedCreateTestCut::HeadCreated, PREFIX, None),
        // An actual cached-created zero-length file is distinct from an unknown or
        // absent empty file; only the former may use this original continuation.
        (
            V2TextAcceptedCreateTestCut::PublicationCreated,
            0,
            Some("u-"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = partial_fixture(cut, len, 0x81500 + index as u64 * 16);
        let target = b_target(&f);
        let selected_s = std::fs::read(target.join("head.bin")).unwrap();
        let (path, expected) = f.accepted.original_output_for_test(cut).unwrap();
        if let Some(prefix) = prefix {
            assert!(path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(prefix));
        }
        let (_, expected_wb) = f
            .accepted
            .original_output_for_test(V2TextAcceptedCreateTestCut::WalCreated)
            .unwrap();
        no_new_nonce(&mut f);
        retry_stage(&mut f).unwrap();
        assert!(f.accepted.has_ready_data() && !f.accepted.has_saved_data());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_s);
        assert_eq!(
            std::fs::read(target.join("text.wal")).unwrap(),
            *expected_wb
        );
        let final_path = if cut == V2TextAcceptedCreateTestCut::WalCreated {
            target.join("text.wal")
        } else {
            path
        };
        assert_eq!(std::fs::read(final_path).unwrap(), *expected);
        assert_idle(&f);
        let commit = commit_b(&f);
        f.accepted.run_commit(commit).unwrap();
        validate_saved_b(&f).unwrap();
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_b_prefix_retry_preserves_missing_replaced_nonprefix_oversize_and_unknown_objects() {
    for scenario in 0..5 {
        let cut = match scenario {
            0 => V2TextAcceptedCreateTestCut::WalCreated,
            1 => V2TextAcceptedCreateTestCut::ArchiveCreated,
            2 => V2TextAcceptedCreateTestCut::PublicationCreated,
            3 => V2TextAcceptedCreateTestCut::HeadCreated,
            _ => V2TextAcceptedCreateTestCut::ArtifactCreated,
        };
        let mut f = if scenario == 4 {
            accepted_fixture(0, 0x81600 + scenario * 16)
        } else {
            partial_fixture(cut, PREFIX, 0x81600 + scenario * 16)
        };
        let target = b_target(&f);
        let selected_s = std::fs::read(target.join("head.bin")).unwrap();
        let (path, expected) = f.accepted.original_output_for_test(cut).unwrap();
        let mut moved = None;
        match scenario {
            0 => std::fs::remove_file(&path).unwrap(),
            1 => {
                let other = path.with_extension("original-prefix");
                std::fs::rename(&path, &other).unwrap();
                std::fs::write(&path, &expected[..PREFIX]).unwrap();
                moved = Some(other);
            }
            2 => {
                let mut wrong = expected[..PREFIX].to_vec();
                wrong[0] ^= 1;
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .write_all(&wrong)
                    .unwrap();
            }
            3 => std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(expected.len() as u64 + 1)
                .unwrap(),
            // The exact same prefix at an unrecorded object is still unknown.
            _ => std::fs::write(&path, &expected[..PREFIX]).unwrap(),
        }
        let preserved = std::fs::read(&path).ok();
        assert!(retry_stage(&mut f).is_err());
        assert_eq!(std::fs::read(&path).ok(), preserved);
        if let Some(other) = moved {
            assert_eq!(std::fs::read(other).unwrap(), expected[..PREFIX]);
        }
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_s);
        assert!(!f.accepted.has_ready_data() && !f.accepted.has_saved_data());
        assert_idle(&f);
    }
}

#[test]
fn actual_pre_main_b_prefix_refuses_artifact_head_or_seed_dependents_without_replacing_ws() {
    for (index, name) in [None, Some("head.stage"), Some("text.seed.stage.v1")]
        .into_iter()
        .enumerate()
    {
        let mut f = partial_fixture(
            V2TextAcceptedCreateTestCut::WalCreated,
            PREFIX,
            0x81700 + index as u64 * 16,
        );
        let target = b_target(&f);
        let ws = std::fs::read(target.join("text.wal")).unwrap();
        let (wal_stage, wb) = f
            .accepted
            .original_output_for_test(V2TextAcceptedCreateTestCut::WalCreated)
            .unwrap();
        let dependent = match name {
            Some(name) => target.join(name),
            None => {
                f.accepted
                    .original_output_for_test(V2TextAcceptedCreateTestCut::ArtifactCreated)
                    .unwrap()
                    .0
            }
        };
        std::fs::write(&dependent, b"unrecorded dependent").unwrap();
        assert!(retry_stage(&mut f).is_err());
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), ws);
        assert_eq!(std::fs::read(wal_stage).unwrap(), wb[..PREFIX]);
        assert_eq!(std::fs::read(dependent).unwrap(), b"unrecorded dependent");
        assert!(!f.accepted.has_ready_data() && !f.accepted.has_saved_data());
        assert_idle(&f);
    }
}

#[test]
fn actual_b_suffix_waits_for_full_same_file_original_ws_or_own_wb_before_any_write() {
    for (index, cut) in [
        V2TextAcceptedCreateTestCut::WalCreated,
        V2TextAcceptedCreateTestCut::PublicationCreated,
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = partial_fixture(cut, PREFIX, 0x81880 + index as u64 * 16);
        let target = b_target(&f);
        let main = target.join("text.wal");
        let original = std::fs::read(&main).unwrap();
        let selected_s = std::fs::read(target.join("head.bin")).unwrap();
        let (partial, expected) = f.accepted.original_output_for_test(cut).unwrap();
        // Corrupt AFTER the real fresh Stage admission. An earlier admission
        // rejection must not hide a missing full-journal check in the worker.
        let admitted = stage_b(&f);
        let held = std::fs::File::open(&main).unwrap();
        let id = catcoms_storage::kept::session_id(&held).unwrap();
        drop(held);
        let mut changed = original.clone();
        let middle = changed.len() / 2;
        changed[middle] ^= 1;
        // Keep the actual native object and length. Metadata-only checks cannot
        // distinguish this corruption from the originally authenticated journal.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&main)
            .unwrap()
            .write_all(&changed)
            .unwrap();
        let held = std::fs::File::open(&main).unwrap();
        assert_eq!(catcoms_storage::kept::session_id(&held).unwrap(), id);
        assert_eq!(held.metadata().unwrap().len(), original.len() as u64);
        drop(held);
        assert!(run_without_key(&mut f, admitted).is_err());
        assert_eq!(std::fs::read(&main).unwrap(), changed);
        assert_eq!(std::fs::read(&partial).unwrap(), expected[..PREFIX]);
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_s);
        assert!(!f.accepted.has_ready_data() && !f.accepted.has_saved_data());
        assert_idle(&f);

        // Restore the same journal, then reuse the SAME original B and its recorded
        // partial. Refusal must not discard the candidate or force another nonce.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&main)
            .unwrap()
            .write_all(&original)
            .unwrap();
        no_new_nonce(&mut f);
        retry_stage(&mut f).unwrap();
        let commit = commit_b(&f);
        f.accepted.run_commit(commit).unwrap();
        validate_saved_b(&f).unwrap();
    }
}

#[test]
fn actual_b_prefix_admitted_before_scope_stop_finishes_then_releases_real_files_and_io() {
    let mut f = partial_fixture(V2TextAcceptedCreateTestCut::ArchiveCreated, PREFIX, 0x81800);
    let target = b_target(&f);
    let (archive, expected) = f
        .accepted
        .original_output_for_test(V2TextAcceptedCreateTestCut::ArchiveCreated)
        .unwrap();
    let stage = stage_b(&f);
    let owner = f.original.owner.borrow_current().unwrap();
    owner.warm_scope().unwrap().revocation().revoke();
    drop(owner);
    run_without_key(&mut f, stage).unwrap();
    assert!(f.accepted.has_ready_data() && !f.accepted.has_saved_data());
    assert_eq!(std::fs::read(&archive).unwrap(), *expected);
    assert_idle(&f);
    // Real exclusive opens detect leaked handles, rather than trusting counters.
    for path in [
        archive,
        target.join("text.wal"),
        target.join("head.stage"),
        target.join("head.bin"),
    ] {
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap();
        drop(held);
    }
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
}
