//! Cross-crate result surface tested against real completed/partial candidates.
//! No raw constructor, payment/origin setter or synthetic operation is used.
use super::*;

#[test]
fn completed_result_is_exact_borrowed_data_and_retries_keep_original_buffers_without_sdk_work() {
    let bytes = source(4, false);
    let (d, g, _) = parent(&bytes);
    let (_peer, mut cell, used, frame, subject, reference) = prepared(&d, &g);
    cell.execute_private().unwrap();
    let pointers;
    {
        let result = cell.completed_result_data().unwrap();
        let again = cell.completed_result_data().unwrap();
        result.require_same_cell_data(&again).unwrap();
        result
            .require_original_parent_accounting_data(&d, &g)
            .unwrap();
        result
            .require_request_data(input(&frame, subject, reference))
            .unwrap();
        assert!(std::ptr::eq(
            result.membership_change_data(),
            cell.observation.as_ref().unwrap()
        ));
        assert_eq!(
            result.commit_data().as_ptr(),
            cell.commit.as_ref().unwrap().as_ptr()
        );
        assert_eq!(
            result.welcome_data().as_ptr(),
            cell.welcome.as_ref().unwrap().as_ptr()
        );
        assert_eq!(
            result.group_info_data().as_ptr(),
            cell.group_info.as_ref().unwrap().as_ptr()
        );
        assert_eq!(
            result.secret_snapshot_data().as_ptr(),
            cell.snapshot.as_ref().unwrap().as_ptr()
        );
        pointers = [
            result.commit_data().as_ptr(),
            result.welcome_data().as_ptr(),
            result.group_info_data().as_ptr(),
            result.secret_snapshot_data().as_ptr(),
        ];
        let request = result.request_data();
        let original = result.original_parent_data();
        assert!(request.key_package_frame_data() == frame.as_slice());
        assert_eq!(request.subject_data(), &subject);
        assert_eq!(request.invite_nonce_data(), &[29; 16]);
        assert_eq!(request.key_package_reference_data(), &reference);
        assert_eq!(request.credential_digest_data(), &cell.credential_digest);
        assert_eq!(request.validation_time_ms_data(), NOW);
        assert!(original.group_id_data().as_slice() == g.group_id_ref());
        assert!(original.author_public_key_data().as_slice() == d.public_key_ref());
        assert_eq!(original.epoch_data(), g.epoch());
        assert_eq!(
            original.own_leaf_index_data(),
            g.group.own_leaf_index().u32()
        );
        assert_eq!(original.authenticator_data(), &g.epoch_authenticator_id());
        assert_eq!(
            original.snapshot_encoding_digest_data(),
            &cell.original_snapshot_id_data()
        );
        assert_eq!(result.after_epoch_data(), g.epoch() + 1);
        assert_eq!(
            result.after_authenticator_data(),
            &cell.graph.group.epoch_authenticator_id()
        );
        assert_eq!(cell.faults.sdk_dispatches, 1);
        assert_eq!(cell.faults.merge_dispatches, 1);
        assert!(used.load(Ordering::SeqCst) > 0);
    }
    // Whole-cell retry requires the immutable loans to end first. It observes
    // cached Complete, so no fresh randomness/SDK/codec/provider entry runs.
    cell.execute_private().unwrap();
    {
        let result = cell.completed_result_data().unwrap();
        assert_eq!(
            pointers,
            [
                result.commit_data().as_ptr(),
                result.welcome_data().as_ptr(),
                result.group_info_data().as_ptr(),
                result.secret_snapshot_data().as_ptr()
            ]
        );
        assert_eq!(
            (cell.faults.sdk_dispatches, cell.faults.merge_dispatches),
            (1, 1)
        );
    }
    drop(cell);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn every_partial_or_failed_candidate_refuses_result_but_complete_lost_return_can_borrow_original() {
    let bytes = source(3, false);
    let (d, g, _) = parent(&bytes);
    for cut in [
        TestCut::AddEntered,
        TestCut::OutputsCached,
        TestCut::CommitSerialized,
        TestCut::WelcomeSerialized,
        TestCut::GroupInfoSerialized,
        TestCut::Serialized,
        TestCut::MergeEntered,
        TestCut::Merged,
        TestCut::ProviderClosed,
        TestCut::ObservationCached,
        TestCut::SnapshotCached,
        TestCut::Complete,
    ] {
        let (_peer, mut cell, used, _, _, _) = prepared(&d, &g);
        assert!(cell.completed_result_data().is_err());
        cell.faults.fail_at = Some(cut);
        assert!(cell.execute_private().is_err());
        let counts = (cell.faults.sdk_dispatches, cell.faults.merge_dispatches);
        if cut == TestCut::Complete {
            let result = cell.completed_result_data().unwrap();
            assert_eq!(result.after_epoch_data(), g.epoch() + 1);
        } else {
            assert!(cell.completed_result_data().is_err());
        }
        assert_eq!(
            counts,
            (cell.faults.sdk_dispatches, cell.faults.merge_dispatches)
        );
        drop(cell);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
    let (_peer, mut cell, _, _, _, _) = prepared(&d, &g);
    let failure = TextAddTestObservation::install(Some((Event::MergeStateWritten, 1)), None);
    assert!(cell.execute_private().is_err());
    assert_eq!(failure.count(Event::MergeStateWritten), 1);
    // Actual cached outputs plus a consumed SDK Pending cannot manufacture a
    // result from a sticky unknown merge. Root still owns this failed scratch.
    assert!(cell.outputs.is_some() && cell.commit.is_some() && cell.welcome.is_some());
    assert!(cell.graph.group.group.pending_commit().is_none());
    assert!(cell.completed_result_data().is_err());
    assert!(cell.execute_private().is_err());
    assert!(cell.completed_result_data().is_err());
}

#[test]
fn distinct_paid_forks_and_equal_cold_parent_labels_are_not_same_cell_or_accounting_origin() {
    let bytes = source(3, false);
    let (d, g, _) = parent(&bytes);
    let (_peer, mut first, first_used, frame, subject, reference) = prepared(&d, &g);
    // SAME complete input and parent still produce another separately paid
    // private Cell, never an alias of the original result identity.
    let (paid, second_used, _) = admission(limits());
    let mut paid = Some(paid);
    let mut second = None;
    PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut paid,
        &mut second,
    )
    .unwrap();
    let mut second = second.unwrap();
    first.execute_private().unwrap();
    second.execute_private().unwrap();
    let (cold_d, cold_g, _) = parent(&bytes);
    {
        let a = first.completed_result_data().unwrap();
        let b = second.completed_result_data().unwrap();
        a.require_original_parent_accounting_data(&d, &g).unwrap();
        b.require_original_parent_accounting_data(&d, &g).unwrap();
        assert!(a.require_same_cell_data(&b).is_err());
        assert!(b.require_same_cell_data(&a).is_err());
        // Cold rows/keys/epochs/authenticator match, but that other actual paid
        // graph pair has a different origin. Hash equality is no substitute.
        assert!(cold_g.epoch_authenticator_id() == g.epoch_authenticator_id());
        assert!(a
            .require_original_parent_accounting_data(&cold_d, &cold_g)
            .is_err());
        assert!(a
            .require_original_parent_accounting_data(&d, &cold_g)
            .is_err());
        assert!(a
            .require_original_parent_accounting_data(&cold_d, &g)
            .is_err());
        let mut bad_frame = frame.clone();
        bad_frame[0] ^= 1;
        let mut bad_subject = subject;
        bad_subject[0] ^= 1;
        let mut bad_reference = reference;
        bad_reference[0] ^= 1;
        for request in [
            input(&bad_frame, subject, reference),
            input(&frame, bad_subject, reference),
            input(&frame, subject, bad_reference),
            TextCurrentAddRequestData::from_untrusted_data(
                &frame, subject, [28; 16], reference, NOW,
            ),
            TextCurrentAddRequestData::from_untrusted_data(
                &frame,
                subject,
                [29; 16],
                reference,
                NOW + 1,
            ),
        ] {
            assert!(a.require_request_data(request).is_err());
        }
        a.require_request_data(input(&frame, subject, reference))
            .unwrap();
    }
    drop(first);
    drop(second);
    assert_eq!(first_used.load(Ordering::SeqCst), 0);
    assert_eq!(second_used.load(Ordering::SeqCst), 0);
}

#[test]
fn result_loans_survive_original_parent_drop_keep_exact_fee_alive_and_charge_all_view_layouts() {
    for group_first in [true, false] {
        let bytes = source(3, false);
        let (d, g, old_used) = parent(&bytes);
        let (_peer, mut cell, used, _, _, _) = prepared(&d, &g);
        cell.execute_private().unwrap();
        if group_first {
            drop(g);
            drop(d);
        } else {
            drop(d);
            drop(g);
        }
        assert_eq!(old_used.load(Ordering::SeqCst), TYPED);
        let counts = (cell.faults.sdk_dispatches, cell.faults.merge_dispatches);
        {
            let result = cell.completed_result_data().unwrap();
            // Only Cell's cached DATA is needed, even after BOTH original graph
            // halves drop. Current/source authority is deliberately not claimed.
            let snapshot = result.secret_snapshot_data();
            let request = result.request_data();
            let original = result.original_parent_data();
            let observation = result.membership_change_data();
            assert!(!snapshot.is_empty() && !request.key_package_frame_data().is_empty());
            assert_eq!(observation.before_epoch(), original.epoch_data());
            assert_eq!(result.after_epoch_data(), original.epoch_data() + 1);
            assert!(used.load(Ordering::SeqCst) > 0);
            assert_eq!(old_used.load(Ordering::SeqCst), TYPED);
            // Debug is explicitly redacted and does not format any borrowed
            // secret/MLS bytes. No Serialize or owned graph/copy API is present.
            assert_eq!(format!("{request:?}"), "TextCurrentAddRequestView { .. }");
            assert_eq!(
                format!("{original:?}"),
                format!(
                    "TextCurrentAddParentView {{ epoch: {}, .. }}",
                    original.epoch_data()
                )
            );
        }
        assert_eq!(
            counts,
            (cell.faults.sdk_dispatches, cell.faults.merge_dispatches)
        );
        let view_layout = completed_result_metadata_layout_bytes().unwrap();
        assert!(
            view_layout
                >= 4 * (std::mem::size_of::<CompletedTextCurrentAddData<'static>>()
                    + std::mem::size_of::<Result<CompletedTextCurrentAddData<'static>, MlsError>>(
                    )
                    + std::mem::size_of::<TextCurrentAddRequestView<'static>>()
                    + std::mem::size_of::<TextCurrentAddParentView<'static>>())
        );
        let total = TextCurrentAddAdmission::metadata_layout_bytes::<Fee>().unwrap();
        assert!(total >= view_layout + 4 * std::mem::size_of::<PreparedTextCurrentAdd>());
        drop(cell);
        assert_eq!(used.load(Ordering::SeqCst), 0);
        assert_eq!(old_used.load(Ordering::SeqCst), 0);
    }
}
