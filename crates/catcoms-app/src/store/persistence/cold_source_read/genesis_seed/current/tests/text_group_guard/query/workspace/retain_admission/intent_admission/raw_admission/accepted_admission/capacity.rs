//! Actual B credit refusal preserves the whole already-signed S and Core candidate.
//! The held credit is a real logical debit, not a fake paid receipt or changed limit.
use super::*;
use std::sync::atomic::AtomicUsize;

fn files(target: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut paths: Vec<_> = std::fs::read_dir(target)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect();
    paths.extend(
        std::fs::read_dir(target.join("text.pages"))
            .unwrap()
            .map(|entry| entry.unwrap().path()),
    );
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[test]
fn actual_b_new_metadata_capacity_refusal_preserves_whole_s_candidate_files_then_same_input_succeeds(
) {
    let dispatches = Arc::new(AtomicUsize::new(0));
    PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
        dispatches.clone(),
        None,
        || {
            let mut f = raw_fixture(0, 0x80010);
            let stage = stage_s(&f);
            f.signature.run_stage(stage).unwrap();
            let commit = commit_s(&f);
            f.signature.run_commit(commit).unwrap();
            let candidate_counts = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
            let _observe = f
                .signature
                .observe_candidate_work_for_test(candidate_counts.clone(), false);
            let store = f.original.layout.checked_store().unwrap();
            f.signature
                .materialize_candidate_after_saved_s(store, &f.original.owner)
                .unwrap();
            let digest = f.signature.verified_candidate_digest_for_test().unwrap();
            assert_eq!(candidate_counts[0].load(Ordering::SeqCst), 1);
            assert_eq!(candidate_counts[1].load(Ordering::SeqCst), 1);
            assert_eq!(dispatches.load(Ordering::SeqCst), 1);
            let RawFixture {
                signature,
                lane,
                predecessor,
                model_dropped,
                sdk_dropped,
                original,
            } = f;
            let store = original.layout.checked_store().unwrap();
            let pool = store
                .archive_bootstrap_policy
                .as_ref()
                .unwrap()
                .pending
                .pool
                .clone();
            let owner = original.owner.borrow_current().unwrap();
            let target = owner.source().source().capture.target.to_path_buf();
            let before_files = files(&target);
            let before = pool.charged_for_test();
            let required = PreparedV2TextAcceptedCreate::metadata_fee_bytes_for_test();
            assert!(required > 1);
            // CAPACITY is the actual genuine warm fixture's configured pool constant.
            // Leave exactly one byte less than B's actual named debit, without allocating
            // that many payload bytes or changing the account's configured capacity.
            let held_bytes = CAPACITY
                .checked_sub(before)
                .unwrap()
                .checked_sub(required - 1)
                .unwrap();
            let held = pool.try_reserve(held_bytes).unwrap();
            assert_eq!(CAPACITY - pool.charged_for_test(), required - 1);
            let identity = signature.observe_original();
            let mut input = Some(signature);
            let mut output = None;
            assert!(PreparedV2TextAcceptedCreate::take_into(
                store,
                owner.source(),
                &mut input,
                &mut output
            )
            .is_err());
            assert!(output.is_none());
            let preserved = input.as_ref().unwrap();
            identity
                .require_same_original(&preserved.observe_original())
                .unwrap();
            assert_eq!(
                preserved.verified_candidate_digest_for_test().unwrap(),
                digest
            );
            preserved.saved_raw_signature().unwrap();
            assert_eq!(files(&target), before_files);
            assert_eq!(pool.charged_for_test(), before + held_bytes);
            assert_eq!(dispatches.load(Ordering::SeqCst), 1);
            assert_eq!(candidate_counts[0].load(Ordering::SeqCst), 1);
            assert_eq!(candidate_counts[1].load(Ordering::SeqCst), 1);
            assert!(!model_dropped.load(Ordering::SeqCst));
            assert!(!sdk_dropped.load(Ordering::SeqCst));
            drop(held);
            assert_eq!(pool.charged_for_test(), before);
            PreparedV2TextAcceptedCreate::take_into(store, owner.source(), &mut input, &mut output)
                .unwrap();
            assert!(input.is_none() && output.is_some());
            assert_eq!(pool.charged_for_test(), before + required);
            assert_eq!(files(&target), before_files);
            assert_eq!(dispatches.load(Ordering::SeqCst), 1);
            assert_eq!(candidate_counts[0].load(Ordering::SeqCst), 1);
            assert_eq!(candidate_counts[1].load(Ordering::SeqCst), 1);
            drop(output);
            drop(predecessor);
            drop(lane);
            assert!(model_dropped.load(Ordering::SeqCst));
            assert!(sdk_dropped.load(Ordering::SeqCst));
        },
    );
}
