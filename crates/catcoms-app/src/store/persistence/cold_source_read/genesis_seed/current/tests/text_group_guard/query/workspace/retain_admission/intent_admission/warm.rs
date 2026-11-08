//! Recorded-original I prefix continuation through real Stage admissions. The cuts
//! write actual bytes to actual create-new Files; no parsed WAL or literal proof
//! flag substitutes for the original SDK/cell/Current/lease ownership.
use super::*;
use std::os::windows::fs::OpenOptionsExt;

const PREFIX: usize = 13;

fn partial_fixture(cut: V2TextIntentTestCut, number: u64) -> IntentFixture {
    let mut f = intent_fixture(0, number);
    let (path, expected) = f.intent.original_output_for_test(cut).unwrap();
    assert!(expected.len() > PREFIX);
    f.intent.set_partial_write_fault_for_test(cut, PREFIX);
    let admitted = stage_i(&f);
    assert!(f.intent.run_stage(admitted).is_err());
    assert_eq!(std::fs::read(path).unwrap(), expected[..PREFIX]);
    assert!(!f.intent.has_ready_data() && !f.intent.has_saved_data());
    assert!(!f.lane.workspace_active_for_test());
    f
}

fn no_new_i_nonce(f: &mut IntentFixture) {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    f.intent
        .prepare_candidates(store, owner.source(), &mut NeverRng)
        .unwrap();
}

/// Accept either a fresh physical refusal or the real admitted worker's refusal.
/// Outside slots own every outcome; an admitted marker is always run before another
/// Source attempt, including if a late Current postcheck had reported an error.
fn retry_stage(f: &mut IntentFixture) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = f.intent.resources().clone();
    let mut lease = Some(PreparedV2TextPhysicalLease::prepare(
        store,
        owner.source(),
        &resources,
    )?);
    let mut admitted = None;
    let result = store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, &resources)?;
        guard.admit_text_intent_stage_into(
            &f.lane,
            &f.predecessor,
            &f.intent,
            &mut lease,
            &mut admitted,
        )
    });
    drop(owner);
    drop(lease);
    if let Some(admitted) = admitted {
        f.intent.run_stage(admitted)
    } else {
        result?;
        Err(crate::store::critical_namespaces::refused())
    }
}

fn assert_idle(f: &IntentFixture) {
    assert!(!f.lane.workspace_active_for_test());
    let store = f.original.layout.checked_store().unwrap();
    assert_eq!(
        crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test(store),
        0
    );
}

#[test]
fn actual_same_i_wal_intent_and_head_prefixes_continue_original_ciphertext_without_new_nonce() {
    for (index, cut) in [
        V2TextIntentTestCut::WalCreated,
        V2TextIntentTestCut::IntentCreated,
        V2TextIntentTestCut::HeadCreated,
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = partial_fixture(cut, 0x7fc00 + index as u64 * 16);
        let dir = i_target(&f);
        let selected_a = std::fs::read(dir.join("head.bin")).unwrap();
        let (_, expected_wi) = f
            .intent
            .original_output_for_test(V2TextIntentTestCut::WalCreated)
            .unwrap();
        let (intent_path, expected_intent) = f
            .intent
            .original_output_for_test(V2TextIntentTestCut::IntentCreated)
            .unwrap();
        let (head_stage, expected_head) = f
            .intent
            .original_output_for_test(V2TextIntentTestCut::HeadCreated)
            .unwrap();
        no_new_i_nonce(&mut f);
        retry_stage(&mut f).unwrap();
        assert!(f.intent.has_ready_data() && !f.intent.has_saved_data());
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), selected_a);
        assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), *expected_wi);
        assert_eq!(std::fs::read(intent_path).unwrap(), *expected_intent);
        assert_eq!(std::fs::read(head_stage).unwrap(), *expected_head);
        assert_idle(&f);
        let commit = commit_i(&f);
        f.intent.run_commit(commit).unwrap();
        validate_saved_i(&f).unwrap();
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), *expected_head);
    }
}

#[test]
fn actual_original_i_prefix_retry_preserves_missing_replaced_nonprefix_and_oversize_outputs() {
    for (index, cut) in [
        V2TextIntentTestCut::WalCreated,
        V2TextIntentTestCut::IntentCreated,
        V2TextIntentTestCut::HeadCreated,
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = partial_fixture(cut, 0x7fc40 + index as u64 * 16);
        let dir = i_target(&f);
        let head = std::fs::read(dir.join("head.bin")).unwrap();
        let (path, expected) = f.intent.original_output_for_test(cut).unwrap();
        let prefix = expected[..PREFIX].to_vec();
        let moved = path.with_file_name("original-i-prefix-held-by-test");
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
            "full actual WA/own WI must be proved BEFORE the first suffix byte"
        );
        assert_idle(&f);
        std::fs::write(&main, valid_main).unwrap();
        let mut oversized = expected.to_vec();
        oversized.push(0);
        std::fs::write(&path, &oversized).unwrap();
        assert!(retry_stage(&mut f).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), oversized);
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), head);
        assert!(!f.intent.has_ready_data() && !f.intent.has_saved_data());
        assert_idle(&f);
        // Restore only the SAME original File's actual bytes. A fresh positive Stage,
        // never the observed native ID, authorizes its known immutable continuation.
        std::fs::write(&path, prefix).unwrap();
        no_new_i_nonce(&mut f);
        retry_stage(&mut f).unwrap();
        assert!(f.intent.has_ready_data() && !f.intent.has_saved_data());
        assert_idle(&f);
    }
}

#[test]
fn actual_pre_main_i_prefix_refuses_dependent_intent_head_or_seed_without_changing_wa() {
    let mut f = partial_fixture(V2TextIntentTestCut::WalCreated, 0x7fc80);
    let dir = i_target(&f);
    let head = std::fs::read(dir.join("head.bin")).unwrap();
    let wa = std::fs::read(dir.join("text.wal")).unwrap();
    let (stage, expected_wi) = f
        .intent
        .original_output_for_test(V2TextIntentTestCut::WalCreated)
        .unwrap();
    let (intent_path, intent) = f
        .intent
        .original_output_for_test(V2TextIntentTestCut::IntentCreated)
        .unwrap();
    let (head_path, candidate) = f
        .intent
        .original_output_for_test(V2TextIntentTestCut::HeadCreated)
        .unwrap();
    for (path, bytes) in [
        (intent_path, intent.to_vec()),
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
        assert_eq!(std::fs::read(&stage).unwrap(), expected_wi[..PREFIX]);
        assert_eq!(std::fs::read(dir.join("text.wal")).unwrap(), wa);
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), head);
        assert!(!f.intent.has_ready_data() && !f.intent.has_saved_data());
        assert_idle(&f);
        std::fs::remove_file(path).unwrap();
    }
    no_new_i_nonce(&mut f);
    retry_stage(&mut f).unwrap();
    assert!(f.intent.has_ready_data());
    assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), head);
}

#[test]
fn actual_prefix_continuation_admitted_before_stop_finishes_and_releases_all_native_custody() {
    for (index, cut) in [
        V2TextIntentTestCut::WalCreated,
        V2TextIntentTestCut::IntentCreated,
        V2TextIntentTestCut::HeadCreated,
    ]
    .into_iter()
    .enumerate()
    {
        let mut f = partial_fixture(cut, 0x7fc90 + index as u64 * 16);
        let dir = i_target(&f);
        let a = std::fs::read(dir.join("head.bin")).unwrap();
        no_new_i_nonce(&mut f);
        let stage = stage_i(&f);
        let owner = f.original.owner.borrow_current().unwrap();
        owner.warm_scope().unwrap().revocation().revoke();
        drop(owner);
        f.intent.run_stage(stage).unwrap();
        assert!(f.intent.has_ready_data() && !f.intent.has_saved_data());
        assert_eq!(std::fs::read(dir.join("head.bin")).unwrap(), a);
        assert_idle(&f);
        let (intent_path, _) = f
            .intent
            .original_output_for_test(V2TextIntentTestCut::IntentCreated)
            .unwrap();
        let (head_path, _) = f
            .intent
            .original_output_for_test(V2TextIntentTestCut::HeadCreated)
            .unwrap();
        for path in [dir.join("text.wal"), intent_path, head_path] {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(path)
                .unwrap();
            drop(file);
        }
        let IntentFixture {
            intent,
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
        drop(intent);
        assert!(model_dropped.load(Ordering::SeqCst));
        assert!(sdk_dropped.load(Ordering::SeqCst));
    }
}
