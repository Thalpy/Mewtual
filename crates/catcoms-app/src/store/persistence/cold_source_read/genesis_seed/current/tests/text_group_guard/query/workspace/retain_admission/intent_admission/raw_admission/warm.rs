//! Recorded-original S prefix continuation through real Stage admissions. The cuts
//! write actual bytes to actual create-new Files; no parsed WAL or literal proof
//! flag substitutes for the original signed SDK/cell/Current/lease ownership.
use super::*;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::atomic::AtomicUsize;

const PREFIX: usize = 13;

fn run_without_key(
    f: &mut RawFixture,
    admitted: AdmittedV2TextRawSignatureStage,
) -> Result<(), AppError> {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let result = PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
        dispatches.clone(),
        None,
        || f.signature.run_stage(admitted),
    );
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    result
}

fn partial_fixture(cut: V2TextRawSignatureTestCut, number: u64) -> RawFixture {
    partial_fixture_len(cut, number, PREFIX)
}

fn partial_fixture_len(cut: V2TextRawSignatureTestCut, number: u64, len: usize) -> RawFixture {
    let mut f = raw_fixture(0, number);
    let (path, expected) = f.signature.original_output_for_test(cut).unwrap();
    assert!(expected.len() > len);
    f.signature.set_partial_write_fault_for_test(cut, len);
    let admitted = stage_s(&f);
    assert!(run_without_key(&mut f, admitted).is_err());
    assert_eq!(std::fs::read(path).unwrap(), expected[..len]);
    assert!(!f.signature.has_ready_data() && !f.signature.has_saved_data());
    assert!(!f.lane.workspace_active_for_test());
    f
}

fn no_new_s_nonce(f: &mut RawFixture) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    f.signature
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
}

/// Accept either a fresh physical refusal or the real admitted worker's refusal.
/// Outside slots own every outcome; an admitted marker is always run before another
/// Source attempt, including if a late Current postcheck had reported an error.
fn retry_stage(f: &mut RawFixture) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = f.signature.resources().clone();
    let mut lease = Some(PreparedV2TextPhysicalLease::prepare(
        store,
        owner.source(),
        &resources,
    )?);
    let mut admitted = None;
    let result = store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.admit_text_raw_signature_stage_into(
            &f.lane,
            &f.predecessor,
            &f.signature,
            &mut lease,
            &mut admitted,
        )
    });
    drop(owner);
    drop(lease);
    if let Some(admitted) = admitted {
        run_without_key(f, admitted)
    } else {
        result?;
        Err(crate::store::critical_namespaces::refused())
    }
}

fn assert_idle(f: &RawFixture) {
    assert!(!f.lane.workspace_active_for_test());
    let store = f.original.layout.checked_store().unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
}

#[test]
fn actual_same_s_wal_signature_and_head_prefixes_continue_original_ciphertext_without_new_nonce() {
    for (index, (cut, len)) in [
        (V2TextRawSignatureTestCut::WalCreated, PREFIX),
        (V2TextRawSignatureTestCut::SignatureCreated, PREFIX),
        (V2TextRawSignatureTestCut::HeadCreated, PREFIX),
        // An actually created and recorded empty File is a valid warm prefix. Its
        // provenance is the real create-fault above, never mere emptiness on disk.
        (V2TextRawSignatureTestCut::SignatureCreated, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = partial_fixture_len(cut, 0x7fd00 + index as u64 * 16, len);
        let dir = s_target(&f);
        let selected_i = std::fs::read(dir.join("head.bin")).unwrap();
        let (_, expected_ws) = f
            .signature
            .original_output_for_test(V2TextRawSignatureTestCut::WalCreated)
            .unwrap();
        let (signature_path, expected_signature) = f
            .signature
            .original_output_for_test(V2TextRawSignatureTestCut::SignatureCreated)
            .unwrap();
        let (head_stage, expected_head) = f
            .signature
            .original_output_for_test(V2TextRawSignatureTestCut::HeadCreated)
            .unwrap();
        no_new_s_nonce(&mut f);
        retry_stage(&mut f).unwrap();
        assert!(f.signature.has_ready_data() && !f.signature.has_saved_data());
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), selected_i);
        assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), *expected_ws);
        assert_eq!(std::fs::read(signature_path).unwrap(), *expected_signature);
        assert_eq!(std::fs::read(head_stage).unwrap(), *expected_head);
        assert_idle(&f);
        let commit = commit_s(&f);
        f.signature.run_commit(commit).unwrap();
        validate_saved_s(&f).unwrap();
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), *expected_head);
    }
}

#[test]
fn actual_original_s_prefix_retry_preserves_missing_replaced_nonprefix_and_oversize_outputs() {
    for (index, cut) in [
        V2TextRawSignatureTestCut::WalCreated,
        V2TextRawSignatureTestCut::SignatureCreated,
        V2TextRawSignatureTestCut::HeadCreated,
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = partial_fixture(cut, 0x7fd40 + index as u64 * 16);
        let dir = s_target(&f);
        let head = std::fs::read(dir.join("head.bin")).unwrap();
        let (path, expected) = f.signature.original_output_for_test(cut).unwrap();
        let prefix = expected[..PREFIX].to_vec();
        let moved = path.with_file_name("original-s-prefix-held-by-test");
        assert!(!moved.exists());
        std::fs::rename(&path, &moved).unwrap();
        assert!(retry_stage(&mut f).is_err());
        assert!(
            !path.exists(),
            "missing recorded output cannot be recreated"
        );
        assert_eq!(std::fs::read(&moved).unwrap(), prefix);
        assert_idle(&f);
        // Keeping the original File present at another name prevents native-ID reuse.
        std::fs::write(&path, &prefix).unwrap();
        let original_id =
            catcoms_storage::kept::session_id(&std::fs::File::open(&moved).unwrap()).unwrap();
        let foreign_id =
            catcoms_storage::kept::session_id(&std::fs::File::open(&path).unwrap()).unwrap();
        assert_ne!(original_id, foreign_id);
        assert!(retry_stage(&mut f).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), prefix);
        assert_eq!(std::fs::read(&moved).unwrap(), prefix);
        assert_idle(&f);
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&moved, &path).unwrap();

        let mut wrong = prefix.clone();
        wrong[0] ^= 0x80;
        std::fs::write(&path, &wrong).unwrap();
        assert!(retry_stage(&mut f).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), wrong);
        assert_idle(&f);
        std::fs::write(&path, &prefix).unwrap();
        let main = dir.join("text.wal");
        let valid_main = std::fs::read(&main).unwrap();
        let mut foreign_main = valid_main.clone();
        let at = foreign_main.len() / 2;
        foreign_main[at] ^= 0x40;
        std::fs::write(&main, &foreign_main).unwrap();
        assert!(retry_stage(&mut f).is_err());
        assert_eq!(std::fs::read(&main).unwrap(), foreign_main);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            prefix,
            "full actual WI/own WS must be proved BEFORE the first suffix byte"
        );
        assert_idle(&f);
        std::fs::write(&main, valid_main).unwrap();
        let mut oversized = expected.to_vec();
        oversized.push(0);
        std::fs::write(&path, &oversized).unwrap();
        assert!(retry_stage(&mut f).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), oversized);
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), head);
        assert!(!f.signature.has_ready_data() && !f.signature.has_saved_data());
        assert_idle(&f);
        // Restore only the SAME original File's actual bytes. A fresh positive Stage,
        // never the observed native ID, authorizes its known immutable continuation.
        std::fs::write(&path, prefix).unwrap();
        no_new_s_nonce(&mut f);
        retry_stage(&mut f).unwrap();
        assert!(f.signature.has_ready_data() && !f.signature.has_saved_data());
        assert_idle(&f);
    }
}

#[test]
fn actual_pre_main_s_prefix_refuses_dependent_signature_head_or_seed_without_changing_wi() {
    let mut f = partial_fixture(V2TextRawSignatureTestCut::WalCreated, 0x7fd80);
    let dir = s_target(&f);
    let head = std::fs::read(dir.join("head.bin")).unwrap();
    let wi = std::fs::read(dir.join("text.wal")).unwrap();
    let (stage, expected_ws) = f
        .signature
        .original_output_for_test(V2TextRawSignatureTestCut::WalCreated)
        .unwrap();
    let (signature_path, signature) = f
        .signature
        .original_output_for_test(V2TextRawSignatureTestCut::SignatureCreated)
        .unwrap();
    let (head_path, candidate) = f
        .signature
        .original_output_for_test(V2TextRawSignatureTestCut::HeadCreated)
        .unwrap();
    for (path, bytes) in [
        (signature_path, signature.to_vec()),
        (head_path, candidate.to_vec()),
        (
            dir.join("text.seed.stage.v1"),
            b"unrelated seed work".to_vec(),
        ),
    ] {
        assert!(!path.exists());
        std::fs::write(&path, &bytes).unwrap();
        assert!(retry_stage(&mut f).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(&stage).unwrap(), expected_ws[..PREFIX]);
        assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), wi);
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), head);
        assert!(!f.signature.has_ready_data() && !f.signature.has_saved_data());
        assert_idle(&f);
        std::fs::remove_file(path).unwrap();
    }
    no_new_s_nonce(&mut f);
    retry_stage(&mut f).unwrap();
    assert!(f.signature.has_ready_data());
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), head);
}

#[test]
fn actual_s_prefix_admitted_before_stop_finishes_and_releases_all_native_custody() {
    for (index, cut) in [
        V2TextRawSignatureTestCut::WalCreated,
        V2TextRawSignatureTestCut::SignatureCreated,
        V2TextRawSignatureTestCut::HeadCreated,
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = partial_fixture(cut, 0x7fd90 + index as u64 * 16);
        let dir = s_target(&f);
        let selected_i = std::fs::read(dir.join("head.bin")).unwrap();
        no_new_s_nonce(&mut f);
        let stage = stage_s(&f);
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        owner.warm_scope().unwrap().revocation().revoke();
        // Mutate the disjoint actual signature owner while keeping the original
        // source ref long enough to prove fresh Current refuses in this branch.
        let dispatches = Arc::new(AtomicUsize::new(0));
        PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
            dispatches.clone(),
            None,
            || f.signature.run_stage(stage),
        )
        .unwrap();
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
        assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
        drop(owner);
        assert!(f.signature.has_ready_data() && !f.signature.has_saved_data());
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), selected_i);
        assert_idle(&f);
        let (signature_path, _) = f
            .signature
            .original_output_for_test(V2TextRawSignatureTestCut::SignatureCreated)
            .unwrap();
        let (head_path, _) = f
            .signature
            .original_output_for_test(V2TextRawSignatureTestCut::HeadCreated)
            .unwrap();
        for path in [dir.join("text.wal"), signature_path, head_path] {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(path)
                .unwrap();
            drop(file);
        }
        let RawFixture {
            signature,
            lane,
            predecessor,
            model_dropped,
            sdk_dropped,
            original,
        } = f;
        let TextFixture {
            owner,
            layout,
            _directory: directory,
        } = original;
        drop(owner);
        drop(lane);
        drop(predecessor);
        drop(layout);
        let session = catcoms_storage::acquire_vault_session(directory.path()).unwrap();
        assert!(!model_dropped.load(Ordering::SeqCst));
        assert!(!sdk_dropped.load(Ordering::SeqCst));
        drop(session);
        drop(signature);
        assert!(model_dropped.load(Ordering::SeqCst));
        assert!(sdk_dropped.load(Ordering::SeqCst));
    }
}
