//! Actual whole warm founder and native review-catalog selection.
use super::*;
use crate::store::critical_namespaces::{
    governance_layout::GOVERNANCE_SEED,
    native_registry::preparation::tests::{current_policy, SECRET},
    Slot,
};
use crate::store::persistence::governance_layout::joined::review_hold::{
    with_warm_review_hook, WarmReviewCut, MARKER_NAME,
};
use crate::store::{NativeTextJoinedStorage, NativeTextSourceRetirementStatus};
use rand_core::SeedableRng;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

fn command_at(
    vault: &NativeTextVault,
    budget: &crate::store::NativeUiInvocationBudget,
    generation: u64,
) -> NativeTextSourceCommand {
    let mut candidate = vault
        .capture_ui(generation, budget.prepare().unwrap())
        .finish()
        .unwrap();
    let mut capture = Some(vault.admit_ui(&mut candidate).unwrap());
    vault.bind_native_source_command(&mut capture).unwrap()
}

#[tokio::test]
async fn warm_founder_review_selects_both_catalogs_with_same_source_across_lock_and_pressure() {
    use super::super::super::tests::selected_with_ui;
    let dir = tempfile::tempdir().unwrap();
    let generation = Arc::new(AtomicU64::new(7));
    let authorized = Arc::new(AtomicU64::new(7));
    let locked = Arc::new(AtomicBool::new(false));
    let (vault, budget) = selected_with_ui(
        dir.path(),
        true,
        (generation.clone(), authorized.clone(), locked.clone()),
    );
    let owner = prepared_mesh_founder(&vault, &budget, 0x15f3);
    let capture = |generation| {
        owner
            .prepare_creation(
                command_at(&vault, &budget, generation),
                request(1, "Original"),
                inputs(),
            )
            .unwrap()
    };
    capture(7).cover_founder_activation().unwrap();
    let current = dir.path().join(Slot::Current.name());
    let hold = dir.path().join(MARKER_NAME);
    let decision = dir
        .path()
        .join("chat-critical-namespaces.review-decision.marker");
    let claim = dir.path().join(GOVERNANCE_SEED);
    let stage = dir.path().join(Slot::Stage.name());
    let hold_scratch = dir
        .path()
        .join("chat-critical-namespaces.review-hold.claim");
    let decision_scratch = dir
        .path()
        .join("chat-critical-namespaces.review-decision.claim");
    let before = std::fs::read(&current).unwrap();
    assert!(matches!(
        capture(7).select_stored_review_runtime(),
        Err(NativeTextCreationRefusal::Storage)
    ));
    assert_eq!(std::fs::read(&current).unwrap(), before);
    assert!(!hold.exists() && !decision.exists() && !claim.exists() && !stage.exists());
    let (pool, activation, peer, phase0_peer, port) = {
        let job = capture(7);
        let body = job.loan.body.as_ref().unwrap();
        let network = body.network.as_ref().unwrap();
        let store = body
            .work
            .current_for_governed_add_test()
            .checked_store()
            .unwrap();
        (
            store
                .governance_layout_policy
                .as_ref()
                .unwrap()
                .current
                .archive
                .pending
                .pool
                .clone(),
            network.activation.as_ref().unwrap().clone(),
            network.actual_peer.clone().unwrap(),
            network.public.as_ref().unwrap().phase0_peer_data(),
            network.port,
        )
    };
    let calls = TRANSPORT_CALLS.with(|calls| calls.get());
    let pressure = Rc::new(RefCell::new(None));
    let exhaust = || {
        let held = pressure.clone();
        let pool = pool.clone();
        move || {
            let remaining = pool.capacity() - pool.charged_for_test();
            *held.borrow_mut() = Some(pool.try_reserve(remaining).unwrap());
            assert_eq!(pool.capacity(), pool.charged_for_test());
            Ok(())
        }
    };
    let funded = capture(7);
    exhaust()().unwrap();
    assert_eq!(
        funded.cover_founder_review_storage(),
        Err(NativeTextCreationRefusal::Busy)
    );
    drop(pressure.borrow_mut().take());
    assert_eq!(std::fs::read(&current).unwrap(), before);
    assert!([
        &hold,
        &decision,
        &claim,
        &stage,
        &hold_scratch,
        &decision_scratch
    ]
    .iter()
    .all(|path| !path.exists()));

    let foreign_marker = b"invalid review marker";
    std::fs::write(&hold, foreign_marker).unwrap();
    assert_eq!(
        capture(7).cover_founder_review_storage(),
        Err(NativeTextCreationRefusal::Storage)
    );
    assert_eq!(std::fs::read(&hold).unwrap().as_slice(), foreign_marker);
    assert_eq!(std::fs::read(&current).unwrap(), before);
    assert!([&claim, &stage, &hold_scratch, &decision_scratch]
        .iter()
        .all(|path| !path.exists()));
    std::fs::remove_file(&hold).unwrap();

    let (before_write, reached) = with_warm_review_hook(
        ReviewDeclarationKind::Hold,
        WarmReviewCut::Preflight,
        exhaust(),
        || capture(7).cover_founder_review_storage(),
    );
    assert!(reached);
    assert_eq!(before_write, Err(NativeTextCreationRefusal::Busy));
    drop(pressure.borrow_mut().take());
    assert_eq!(std::fs::read(&current).unwrap(), before);
    assert!(!hold.exists() && !claim.exists() && !stage.exists());

    let (requested, next_generation) = (locked.clone(), generation.clone());
    let hold_cost = exhaust();
    let (revoked, reached) = with_warm_review_hook(
        ReviewDeclarationKind::Hold,
        WarmReviewCut::Stage,
        move || {
            hold_cost()?;
            requested.store(true, Ordering::SeqCst);
            next_generation.store(8, Ordering::SeqCst);
            Ok(())
        },
        || capture(7).cover_founder_review_storage(),
    );
    assert!(reached);
    assert_eq!(revoked, Err(NativeTextCreationRefusal::Unavailable));
    drop(pressure.borrow_mut().take());
    let id10 = std::fs::read(&current).unwrap();
    let original_hold = std::fs::read(&hold).unwrap();
    assert_ne!(id10, before);
    assert!(!decision.exists() && !claim.exists() && !stage.exists());
    let old = vault.begin_ui_retirement(7).unwrap();
    drop(old);
    authorized.store(8, Ordering::SeqCst);
    locked.store(false, Ordering::SeqCst);
    assert!(matches!(
        capture(8).select_stored_review_runtime(),
        Err(NativeTextCreationRefusal::Storage)
    ));

    let (result, reached) = with_warm_review_hook(
        ReviewDeclarationKind::Decision,
        WarmReviewCut::Stage,
        exhaust(),
        || capture(8).cover_founder_review_storage(),
    );
    assert!(reached);
    assert_eq!(result, Ok(NativeTextCreationProgress::FounderStored));
    drop(pressure.borrow_mut().take());
    let id11 = std::fs::read(&current).unwrap();
    assert_ne!(id11, id10);
    assert_eq!(std::fs::read(&hold).unwrap(), original_hold);
    assert!(decision.is_file() && !claim.exists() && !stage.exists());
    // Make a valid older selection and a byte-identical fresh inode visible
    // in turn, with all selected review markers still in place. Warm retry
    // and row publication both require the actual renamed Stage owner.
    let parked = dir.path().join("test-original-review-current");
    std::fs::rename(&current, &parked).unwrap();
    for substitute in [&id10, &id11] {
        std::fs::write(&current, substitute).unwrap();
        assert_eq!(
            capture(8).cover_founder_review_storage(),
            Err(NativeTextCreationRefusal::Storage)
        );
        assert!(matches!(
            capture(8).select_stored_review_runtime(),
            Err(NativeTextCreationRefusal::Storage)
        ));
        assert_eq!(&std::fs::read(&current).unwrap(), substitute);
        std::fs::remove_file(&current).unwrap();
    }
    std::fs::rename(&parked, &current).unwrap();
    capture(8).cover_founder_review_storage().unwrap();
    {
        let job = capture(8);
        let body = job.loan.body.as_ref().unwrap();
        let network = body.network.as_ref().unwrap();
        let store = body
            .work
            .current_for_governed_add_test()
            .checked_store()
            .unwrap();
        assert!(store.has_mounted_review_hold().unwrap());
        assert!(store.has_mounted_review_decision().unwrap());
        assert!(Arc::ptr_eq(
            network.activation.as_ref().unwrap(),
            &activation
        ));
        assert_eq!(network.actual_peer.as_ref().unwrap(), &peer);
        assert_eq!(network.port, port);
        assert_eq!(TRANSPORT_CALLS.with(|calls| calls.get()), calls);
    }
    assert_eq!(std::fs::read(&current).unwrap(), id11);
    let row = capture(8).select_stored_review_runtime().unwrap();
    assert!(row.same_original(&capture(8).select_stored_review_runtime().unwrap()));
    let (_live, signal) = tokio::sync::watch::channel(false);
    let saved = owner
        .capture_member(
            &row,
            command_at(&vault, &budget, 8),
            owner.policy.prepare_member().unwrap(),
        )
        .finish()
        .unwrap()
        .save_invite(
            [0x63; 32],
            60_000,
            &catcoms_rt::RequestCancellation::new(signal, None),
        )
        .unwrap();
    let invite =
        crate::store::NativeTextInvitePacketData::decode(saved.packet_bytes_data()).unwrap();
    assert_eq!(invite.connected_peer_data(), phase0_peer);
}

#[tokio::test]
async fn warm_founder_review_uncertain_decision_requires_cold_recovery_of_the_exact_current() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = super::super::super::tests::selected_joined(dir.path());
    let owner = prepared_mesh_founder(&vault, &budget, 0x15f5);
    let capture = || {
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap()
    };
    capture().cover_founder_activation().unwrap();
    let current = dir.path().join(Slot::Current.name());
    let claim = dir.path().join(GOVERNANCE_SEED);
    let stage = dir.path().join(Slot::Stage.name());
    let net = dir.path().join("servers").join("1.native-net-v1");
    let exact_net = std::fs::read(&net).unwrap();
    let (interrupted, reached) = with_warm_review_hook(
        ReviewDeclarationKind::Decision,
        WarmReviewCut::Selected,
        || {
            Err(AppError::Io(
                "injected founder-review interruption after Current".into(),
            ))
        },
        || capture().cover_founder_review_storage(),
    );
    assert!(reached);
    assert_eq!(
        interrupted,
        Err(NativeTextCreationRefusal::ReviewCatalogReopen)
    );
    assert!(claim.is_file() && !stage.exists());
    let exact = std::fs::read(&current).unwrap();
    assert_eq!(
        capture().cover_founder_review_storage(),
        Err(NativeTextCreationRefusal::ReviewCatalogReopen)
    );
    assert_eq!(
        capture().prepare_device(),
        Err(NativeTextCreationRefusal::ReviewCatalogReopen)
    );
    assert!(matches!(
        capture().select_stored_runtime(),
        Err(NativeTextCreationRefusal::ReviewCatalogReopen)
    ));
    assert_eq!(std::fs::read(&current).unwrap(), exact);
    assert_eq!(std::fs::read(&net).unwrap(), exact_net);
    let mut retirement = owner.prepare_retirement().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while retirement.try_retire().unwrap() == NativeTextSourceRetirementStatus::Pending {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop((retirement, owner, vault));
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0x15f6);
    let cold = NativeTextJoinedStorage::open_for_native_startup(
        dir.path(),
        SECRET,
        &mut rng,
        current_policy(),
    )
    .unwrap();
    assert_eq!(cold.row_count_data().unwrap(), 1);
    assert!(!cold
        .select_review_storage_before_founder_startup(&mut rng)
        .unwrap());
    assert!(!claim.exists() && !stage.exists());
    assert_eq!(std::fs::read(&current).unwrap(), exact);
    assert_eq!(std::fs::read(&net).unwrap(), exact_net);
}
