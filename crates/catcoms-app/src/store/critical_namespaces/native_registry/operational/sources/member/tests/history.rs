//! Real NativeRequired Host reads and real local saves. These Root fixtures do
//! not supply authenticated AppState/UI command credentials.
use super::*;
use crate::store::{MemberSaveJobReport, MemberSaveNativeAdmission};
use catcoms_rt::Clock;
use rand_core::SeedableRng;

async fn admit(
    owner: &Owner,
    row: &Row,
    vault: &NativeTextVault,
    budget: &NativeUiInvocationBudget,
    session: &RequestCancellation,
) -> NativeTextMemberSaveJob<MeshService, ChaCha20Rng> {
    let final_prepared = owner
        .capture_member(
            row,
            command(vault, budget),
            owner.policy.prepare_member().unwrap(),
        )
        .finish_admission([0xe7; 32])
        .unwrap();
    let commit = Arc::new(tokio::sync::Mutex::new(()));
    let mut slot = Box::new(None::<tokio::sync::OwnedMutexGuard<()>>);
    *slot = Some(commit.lock_owned().await);
    let native = MemberSaveNativeAdmission::from_boxed_custody(slot, session.clone());
    final_prepared.try_admit(native).unwrap()
}
async fn save(
    owner: &Owner,
    row: &Row,
    vault: &NativeTextVault,
    budget: &NativeUiInvocationBudget,
    session: &RequestCancellation,
) {
    assert_eq!(
        admit(owner, row, vault, budget, session).await.run(),
        MemberSaveJobReport::SavedLocal
    );
}

#[tokio::test]
async fn native_runtime_member_history_first_recovers_actual_interrupted_commit_without_new_signature(
) {
    use crate::store::persistence::v2_member_outbox::{
        member_outbox_call_at_for_test, member_outbox_disarm_for_test, MemberOutboxTestCut,
    };
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xec05);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let text = body("read first after actual commit unwind");
    prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&text), &session)
        .unwrap();
    #[cfg(windows)]
    let signed=crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test();
    let job = admit(&owner, &row, &vault, &budget, &session).await;
    member_outbox_call_at_for_test(
        MemberOutboxTestCut::FirstOutput,
        Box::new(|| panic!("actual history-first recovery commit cut")),
    );
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()));
    member_outbox_disarm_for_test();
    assert!(outcome.is_err());
    // Deliberately read BEFORE save preparation can reconcile Unresolved.
    let page = prepare(&owner, &row, &vault, &budget)
        .prepare_member_history(0, None, 8, &session)
        .unwrap();
    assert_eq!(page.items().len(), 1);
    assert_eq!((page.next(), page.end_exclusive()), (1, 1));
    let item = page.items().next().unwrap();
    let original = item.action().id();
    assert_eq!(&item.action().spec().body, &text);
    assert_eq!(item.original_bytes(), item.action().encode());
    assert_eq!(item.status(), NativeTextMemberHistoryStatus::Pending);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Saved {
            action: original,
            ordinal: 0
        })
    );
    let again = prepare(&owner, &row, &vault, &budget)
        .prepare_member_history(0, None, 8, &session)
        .unwrap();
    assert_eq!(again.items().len(), 1);
    assert_eq!(again.items().next().unwrap().action().id(), original);
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),signed);
}

#[tokio::test]
async fn native_runtime_member_history_distinguishes_presealed_from_saved_and_returns_whole_loan() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xec01);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    #[cfg(windows)]
    let before=crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test();
    let empty = prepare(&owner, &row, &vault, &budget)
        .prepare_member_history(0, None, 8, &session)
        .unwrap();
    assert_eq!(empty.items().len(), 0);
    assert_eq!((empty.next(), empty.end_exclusive()), (0, 0));
    drop(empty);
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),before);
    let text = body("read exact genuine saved original");
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let empty = prepare(&owner, &row, &vault, &budget)
        .prepare_member_history(0, None, 8, &session)
        .unwrap();
    assert_eq!(
        empty.items().len(),
        0,
        "presealed memory is not a saved message"
    );
    drop(empty);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Ready)
    );
    save(&owner, &row, &vault, &budget, &session).await;
    let page = prepare(&owner, &row, &vault, &budget)
        .prepare_member_history(0, None, 8, &session)
        .unwrap();
    assert_eq!((page.next(), page.end_exclusive()), (1, 1));
    let item = page.items().next().unwrap();
    assert_eq!(item.ordinal(), 0);
    assert_eq!(&item.action().spec().body, &text);
    assert_eq!(item.original_bytes(), item.action().encode());
    assert_eq!(item.status(), NativeTextMemberHistoryStatus::Pending);
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),before+1);
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let state = cell.state.lock().unwrap();
    assert!(
        !state.running,
        "paid DATA must leave after actual runtime restoration"
    );
    assert!(state.body.is_some());
}

#[tokio::test]
async fn native_runtime_member_history_refuses_invalid_ranges_cancellation_and_original_source_close(
) {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xec02);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    for (start, end, max) in [(0, None, 0), (0, None, 9), (2, Some(1), 1)] {
        assert!(matches!(
            prepare(&owner, &row, &vault, &budget)
                .prepare_member_history(start, end, max, &session),
            Err(V2MemberSaveRefusal::Stale)
        ));
    }
    let (sender, receiver) = tokio::sync::watch::channel(false);
    sender.send_replace(true);
    let cancelled = RequestCancellation::new(receiver, None);
    assert!(matches!(
        prepare(&owner, &row, &vault, &budget).prepare_member_history(0, None, 8, &cancelled),
        Err(V2MemberSaveRefusal::Cancelled)
    ));
    let job = prepare(&owner, &row, &vault, &budget);
    owner.entries.lock().unwrap()[0].owner.close(); // actual original key admission fence
    assert!(matches!(
        job.prepare_member_history(0, None, 8, &session),
        Err(V2MemberSaveRefusal::Unavailable)
    ));
}

#[tokio::test]
async fn native_runtime_member_history_late_cancellation_drops_page_without_resigning_or_losing_history(
) {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xec03);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let text = body("saved despite cancelled view");
    prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&text), &session)
        .unwrap();
    save(&owner, &row, &vault, &budget, &session).await;
    AFTER_HISTORY.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            sender.send_replace(true);
        }));
    });
    assert!(matches!(
        prepare(&owner, &row, &vault, &budget).prepare_member_history(0, None, 8, &session),
        Err(V2MemberSaveRefusal::Cancelled)
    ));
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let fresh = RequestCancellation::new(receiver, None);
    let page = prepare(&owner, &row, &vault, &budget)
        .prepare_member_history(0, None, 8, &fresh)
        .unwrap();
    assert_eq!(page.items().len(), 1);
    assert_eq!(&page.items().next().unwrap().action().spec().body, &text);
    assert!(matches!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &fresh),
        Ok(V2MemberSaveOutcome::Saved { .. })
    ));
}

#[tokio::test]
async fn native_runtime_member_history_page_keeps_data_without_pinning_original_vault_or_sdk() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xec04);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let text = body("historical DATA after actual shutdown");
    prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&text), &session)
        .unwrap();
    save(&owner, &row, &vault, &budget, &session).await;
    let page = prepare(&owner, &row, &vault, &budget)
        .prepare_member_history(0, None, 8, &session)
        .unwrap();
    let mut retirement = owner.prepare_retirement().unwrap();
    let mut drained = false;
    for _ in 0..1000 {
        if retirement.try_retire().unwrap() == NativeTextSourceRetirementStatus::Drained {
            drained = true;
            break;
        }
        catcoms_rt::SystemClock
            .sleep(std::time::Duration::from_millis(10))
            .await;
    }
    assert!(
        drained,
        "real SDK/body/original TCP drained with history page still held"
    );
    drop(retirement);
    drop(owner);
    drop(vault);
    crate::store::NativeIndexStore::open_lifecycle3_for_test(
        directory.path(),
        crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
        &mut ChaCha20Rng::from_seed([0xec; 32]),
        crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(),
    )
    .unwrap();
    assert_eq!(&page.items().next().unwrap().action().spec().body, &text);
}
