//! Genuine Mesh/whole NativeRequired founder. Synthetic UI proves no native
//! caller authentication. A selection must remain metadata after actual teardown.
use super::super::network::tests::prepared_mesh_founder;
use super::super::tests::{command, inputs, request, selected};
use super::*;
use catcoms_rt::Clock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[tokio::test]
async fn native_runtime_selection_requires_the_genuine_covered_active_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let owner = prepared_mesh_founder(&vault, &budget, 0xef01);
    assert!(matches!(
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap()
            .select_stored_runtime(),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_founder_activation()
        .unwrap();
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let stored = {
        let body = job.loan.body.as_ref().unwrap();
        let original = body.request.as_ref().unwrap();
        body.work
            .lookup_creation(original.invocation, original.digest)
            .unwrap()
            .unwrap()
    };
    assert_eq!(stored.state, crate::store::NativeCreationStateData::Founder);
    let selection = job.select_stored_runtime().unwrap();
    assert_eq!(selection.id_data(), stored.id);
}

#[tokio::test]
async fn native_runtime_selection_reuses_one_paid_allocation_through_actual_retirement() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let owner = prepared_mesh_founder(&vault, &budget, 0xef02);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_founder_activation()
        .unwrap();
    let pool = owner.policy.pool.clone();
    let charged = pool.charged_for_test();
    let first = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .select_stored_runtime()
        .unwrap();
    let mut retained = Vec::new();
    for _ in 0..32 {
        let next = owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap()
            .select_stored_runtime()
            .unwrap();
        assert!(first.same_original(&next));
        assert!(
            Arc::ptr_eq(&first.original, &next.original),
            "one actual canonical allocation, not unbounded retries on one debit"
        );
        retained.push(next);
        assert_eq!(pool.charged_for_test(), charged);
    }
    let stale = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    first.close_original_cached();
    let mut retirement = owner.prepare_retirement().unwrap();
    assert!(matches!(
        stale.select_stored_runtime(),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
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
    assert!(drained, "actual SDK/body/original TCP must drain");
    assert!(
        first.same_original(&retained[0]),
        "closure does not turn identity into authority"
    );
    drop(retirement);
    drop(owner);
    drop(vault);
    // The actual original files are free despite live selection handles.
    crate::store::NativeIndexStore::open_lifecycle3_for_test(
        directory.path(),
        crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
        &mut rand_chacha::ChaCha20Rng::from_seed([0xef; 32]),
        crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(),
    )
    .unwrap();
    assert_eq!(
        pool.charged_for_test(),
        ENTRY_BYTES,
        "only the exact selector's original debit remains"
    );
    drop(retained);
    assert_eq!(pool.charged_for_test(), ENTRY_BYTES);
    drop(first);
    assert_eq!(pool.charged_for_test(), 0);
}

#[tokio::test]
async fn native_runtime_selection_close_fences_only_the_original_row_with_its_whole_runtime_borrowed(
) {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let owner = prepared_mesh_founder(&vault, &budget, 0xeb01);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_founder_activation()
        .unwrap();
    let first = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .select_stored_runtime()
        .unwrap();
    let next = || {
        owner
            .prepare_creation(
                command(&vault, &budget),
                request(2, "Other"),
                NativeTextCreationInputs::new(
                    ChaCha20Rng::seed_from_u64(0xeb11),
                    ChaCha20Rng::seed_from_u64(0xeb12),
                    Box::new(catcoms_rt::ManualClock::new(100)),
                ),
            )
            .unwrap()
    };
    next().prepare_device().unwrap();
    next().prepare_network().unwrap();
    next().cover_pending_network().unwrap();
    next().prepare_transport().unwrap();
    next().prepare_founder().unwrap();
    next().cover_founder_activation().unwrap();
    let other = next().select_stored_runtime().unwrap();
    assert_ne!(first.id_data(), other.id_data());
    let old = owner
        .capture_member(
            &first,
            command(&vault, &budget),
            owner.policy.prepare_member().unwrap(),
        )
        .finish()
        .unwrap();
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = catcoms_rt::RequestCancellation::new(receiver, None);
    #[cfg(windows)]
    let before=crate::store::PreparedV2TextHost::<catcoms_net::MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test();
    {
        // This is the actual original book writer, while the whole first
        // runtime is already on loan. The cached fence must need neither.
        let _writer = owner.entries.lock().unwrap();
        first.close_original_cached();
        first.close_original_cached();
    }
    let body = catcoms_replication::chat::Body::Create {
        text: "other row still works".into(),
        reply_to: None,
    };
    let request = crate::store::V2MemberSignRequest {
        token: [0xeb; 32],
        channel: 7,
        body: &body,
        parents: &[],
        dependencies: &[],
    };
    assert_eq!(
        old.prepare_member_save(request, &session),
        Err(crate::store::V2MemberSaveRefusal::Unavailable)
    );
    let current = owner
        .capture_member(
            &other,
            command(&vault, &budget),
            owner.policy.prepare_member().unwrap(),
        )
        .finish()
        .unwrap();
    assert_eq!(
        current.prepare_member_save(
            crate::store::V2MemberSignRequest {
                token: [0xeb; 32],
                channel: 7,
                body: &body,
                parents: &[],
                dependencies: &[]
            },
            &session
        ),
        Ok(crate::store::V2MemberSaveOutcome::Ready)
    );
    #[cfg(windows)]
    assert_eq!(crate::store::PreparedV2TextHost::<catcoms_net::MeshService,ChaCha20Rng>::native_member_sign_dispatches_for_test(),before+1);
    assert!(matches!(
        owner
            .capture_member(
                &first,
                command(&vault, &budget),
                owner.policy.prepare_member().unwrap()
            )
            .finish(),
        Err(crate::store::V2MemberSaveRefusal::Unavailable)
    ));
}

#[tokio::test]
async fn native_runtime_selection_same_numeric_row_in_another_vault_is_foreign() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let (first, first_budget) = selected(first_dir.path());
    let (second, second_budget) = selected(second_dir.path());
    let first_owner = prepared_mesh_founder(&first, &first_budget, 0xef03);
    let second_owner = prepared_mesh_founder(&second, &second_budget, 0xef04);
    let mut rows = Vec::new();
    for (owner, vault, budget) in [
        (&first_owner, &first, &first_budget),
        (&second_owner, &second, &second_budget),
    ] {
        owner
            .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
            .unwrap()
            .cover_founder_activation()
            .unwrap();
        rows.push(
            owner
                .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
                .unwrap()
                .select_stored_runtime()
                .unwrap(),
        );
    }
    assert_eq!(rows[0].id_data(), rows[1].id_data());
    assert!(!rows[0].same_original(&rows[1]));
}
