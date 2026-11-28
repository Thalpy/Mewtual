//! Exercise the actual original founder after Add and Remove, before its
//! first restart. The survivor holds posts the founder never received.
use super::*;
use crate::store::{
    NativeTextFounderReviewExchange, NativeTextFounderReviewReply, NativeTextFounderReviewResult,
    NativeTextInitialReviewRequest, NativeTextInvitePacketData, NativeTextJoinedRowSelection,
    NativeTextJoinedSelectionPreparation, NativeTextMemberPreparation, NativeTextSourceCommand,
    NativeTextVault, NativeUiInvocationBudget,
};
use catcoms_discovery::EndpointDialScheduler;
use catcoms_net::MeshService;
use catcoms_replication::chat::owner_bodies::{ExactReviewOriginal, ReviewDecision};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_sync::owner_v2::V2RemovedReviewQuery;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{
    cell::RefCell,
    net::Ipv4Addr,
    path::Path,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

pub(super) struct WarmFounderUi {
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
    locked: Arc<AtomicBool>,
}

impl WarmFounderUi {
    pub(super) fn new() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(7)),
            authorized: Arc::new(AtomicU64::new(7)),
            locked: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn values(&self) -> (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>) {
        (
            self.generation.clone(),
            self.authorized.clone(),
            self.locked.clone(),
        )
    }

    pub(super) fn command(
        &self,
        vault: &NativeTextVault,
        budget: &NativeUiInvocationBudget,
    ) -> NativeTextSourceCommand {
        let mut candidate = vault
            .capture_ui(
                self.generation.load(Ordering::SeqCst),
                budget.prepare().unwrap(),
            )
            .finish()
            .unwrap();
        let mut capture = Some(vault.admit_ui(&mut candidate).unwrap());
        vault.bind_native_source_command(&mut capture).unwrap()
    }
}

pub(super) async fn prove_original_warm_hold(
    founder: impl Fn() -> NativeTextMemberPreparation<MeshService, ChaCha20Rng>,
    holder: impl Fn() -> NativeTextJoinedSelectionPreparation<ChaCha20Rng>,
    (selection, invite, token): (
        &NativeTextJoinedRowSelection,
        &NativeTextInvitePacketData<'_>,
        [u8; 32],
    ),
    (channel, removed, actions): (u128, OwnerFactId, &[ActionId; 2]),
    fresh: &RequestCancellation,
) {
    let scheduler = EndpointDialScheduler::new(Default::default());
    holder()
        .prepare_reconnect(
            selection,
            invite,
            token,
            Ipv4Addr::LOCALHOST.into(),
            fresh.clone(),
        )
        .unwrap()
        .connect(&scheduler)
        .await
        .unwrap();
    let contact = holder().sign_member_contact(selection, fresh).unwrap();
    assert!(holder()
        .maintain_archive_relay_catalogue(selection, channel, fresh)
        .unwrap());
    let mut provider = holder()
        .attach_dependency_provider(selection, fresh.clone())
        .unwrap();
    let turn = async |exchange: NativeTextFounderReviewExchange,
                      provider: &mut crate::store::NativeTextJoinedDependencyProvider|
           -> NativeTextFounderReviewReply {
        let serve = async {
            let incoming = provider
                .next()
                .await
                .expect("the original holder receives the request");
            holder()
                .serve_dependency_request(
                    selection,
                    channel,
                    incoming,
                    fresh.clone(),
                    &mut ChaCha20Rng::seed_from_u64(0x8282),
                )
                .unwrap();
        };
        let (reply, ()) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(exchange.request(), serve)
        })
        .await
        .unwrap();
        reply.unwrap()
    };

    let (cancel, stopped) = tokio::sync::watch::channel(false);
    let old = RequestCancellation::new(stopped, None);
    let stale = founder()
        .start_founder_removed_review(channel, removed, contact, &old)
        .unwrap();
    let prepared = founder()
        .prepare_founder_removed_review(stale.clone(), V2RemovedReviewQuery::Page(None), &old)
        .unwrap();
    let reply = turn(prepared, &mut provider).await;
    cancel.send(true).unwrap();
    assert!(founder()
        .receive_founder_removed_review(reply, fresh)
        .is_err());
    assert!(founder()
        .prepare_founder_removed_review(stale, V2RemovedReviewQuery::Page(None), fresh)
        .is_err());

    let sweep = founder()
        .start_founder_removed_review(channel, removed, contact, fresh)
        .unwrap();
    let prepared = founder()
        .prepare_founder_removed_review(sweep.clone(), V2RemovedReviewQuery::Page(None), fresh)
        .unwrap();
    let reply = turn(prepared, &mut provider).await;
    let NativeTextFounderReviewResult::Page(page) = founder()
        .receive_founder_removed_review(reply, fresh)
        .unwrap()
    else {
        panic!("the original warm row must authenticate the survivor's inventory")
    };
    let (offered, offer, continuation) = page.current_data().unwrap();
    assert_eq!(offered.len(), 2);
    assert!(actions.iter().all(|action| offered.contains(action)));
    assert!(continuation.is_none());
    let (cancel, stopped) = tokio::sync::watch::channel(false);
    let old_exact = RequestCancellation::new(stopped, None);
    let prepared = founder()
        .prepare_founder_removed_review(
            sweep.clone(),
            V2RemovedReviewQuery::Original {
                offer,
                action: actions[0],
            },
            &old_exact,
        )
        .unwrap();
    let reply = turn(prepared, &mut provider).await;
    cancel.send(true).unwrap();
    assert!(founder()
        .receive_founder_removed_review(reply, fresh)
        .is_err());
    founder()
        .audit_founder_held_posts(channel, None, 3, fresh)
        .unwrap()
        .with_records(|records, unavailable| {
            assert!(records.is_some_and(|records| records.is_empty()));
            assert_eq!(unavailable, 0);
        })
        .unwrap();
    for action in actions {
        let prepared = founder()
            .prepare_founder_removed_review(
                sweep.clone(),
                V2RemovedReviewQuery::Original {
                    offer,
                    action: *action,
                },
                fresh,
            )
            .unwrap();
        let reply = turn(prepared, &mut provider).await;
        assert!(matches!(
            founder().receive_founder_removed_review(reply, fresh),
            Ok(NativeTextFounderReviewResult::Held(held)) if held == *action
        ));
    }
    founder()
        .audit_founder_held_posts(channel, None, 3, fresh)
        .unwrap()
        .with_records(|records, unavailable| {
            let records = records.unwrap();
            assert_eq!(records.len(), 2);
            assert_eq!(unavailable, 0);
            assert!(records
                .iter()
                .all(|case| case.admission == removed && actions.contains(&case.signed.id())));
        })
        .unwrap();
}

pub(super) fn prove_warm_selection_across_lock(
    founder: impl Fn() -> NativeTextMemberPreparation<MeshService, ChaCha20Rng>,
    (ui, vault, path): (&WarmFounderUi, &Arc<NativeTextVault>, &Path),
    (channel, action, removal): (u128, ActionId, (OwnerHeaderId, u64)),
    caller: &RequestCancellation,
) {
    use crate::store::persistence::governance_layout::joined::review_decision::{
        with_review_control_fault, ReviewControlFault, AFTER_REVIEW_KEY_FOR_TEST,
        BEFORE_REVIEW_KEY_FOR_TEST,
    };
    use crate::store::NativeTextHeldReviewDecision;

    let audit = founder()
        .audit_founder_held_posts(channel, None, 3, caller)
        .unwrap();
    let (target, other) = audit
        .with_records(|records, unavailable| {
            assert_eq!(unavailable, 0);
            let records = records.unwrap();
            let exact = |held: &crate::store::NativeTextHeldCase| {
                assert_eq!(held.removal, removal);
                assert_eq!(held.decision, NativeTextHeldReviewDecision::Undecided);
                (
                    held.case,
                    held.signed.id(),
                    ExactReviewOriginal {
                        digest: held.original,
                        remove_header: removal.0,
                        remove_epoch: removal.1,
                    },
                )
            };
            (
                exact(
                    records
                        .iter()
                        .find(|held| held.signed.id() == action)
                        .unwrap(),
                ),
                exact(
                    records
                        .iter()
                        .find(|held| held.signed.id() != action)
                        .unwrap(),
                ),
            )
        })
        .unwrap();
    drop(audit);
    let request = || {
        NativeTextInitialReviewRequest::new(channel, target, [0x96; 16], ReviewDecision::Approve)
    };

    let retired = Rc::new(RefCell::new(None));
    let arm_lock = |through: u64, after_key: bool| {
        let (generation, _, locked) = ui.values();
        let vault = vault.clone();
        let completed = retired.clone();
        let hook: Box<dyn FnOnce()> = Box::new(move || {
            locked.store(true, Ordering::SeqCst);
            generation.store(through + 1, Ordering::SeqCst);
            let retiring = vault.begin_ui_retirement(through).unwrap();
            let drain = retiring.observe_drain().unwrap();
            let mut batch = retiring.into_batch().unwrap();
            batch.close_owners();
            assert_eq!(
                drain.is_drained_cached(),
                !after_key,
                "only the actual completed key cut holds the original UI open",
            );
            *completed.borrow_mut() = Some((drain, batch));
        });
        let slot = if after_key {
            &AFTER_REVIEW_KEY_FOR_TEST
        } else {
            &BEFORE_REVIEW_KEY_FOR_TEST
        };
        slot.with(|slot| assert!(slot.replace(Some(hook)).is_none()));
    };
    let resume = |generation| {
        let (drain, batch) = retired
            .borrow_mut()
            .take()
            .expect("lock reached the exact key cut");
        assert!(drain.is_drained_cached());
        drop(batch);
        ui.authorized.store(generation, Ordering::SeqCst);
        ui.locked.store(false, Ordering::SeqCst);
    };
    let journal = path.join("chat-v2-review-decisions");
    let attempt = journal.join("key.attempt");

    arm_lock(7, false);
    assert_eq!(
        founder().select_founder_review(request(), caller),
        Err(V2MemberSaveRefusal::Stale)
    );
    let covered_attempt = std::fs::read(&attempt).unwrap();
    assert!(!journal.join("key.raw").exists());
    resume(8);

    arm_lock(8, true);
    assert_eq!(
        founder().select_founder_review(request(), caller),
        Err(V2MemberSaveRefusal::Stale)
    );
    assert_eq!(std::fs::read(&attempt).unwrap(), covered_attempt);
    assert!(journal.join("key.raw").is_file());
    resume(9);

    let (selected, entered_again) =
        with_review_control_fault(ReviewControlFault::BeforeFreshKey, || {
            founder().select_founder_review(request(), caller)
        });
    assert!(!entered_again, "the warm retry recovers the covered Raw");
    let selected = selected.unwrap();
    assert_eq!(selected.position_data().sequence, 1);
    assert_eq!(selected.position_data().membership_header, removal.0);
    assert_eq!(selected.position_data().membership_epoch, removal.1);
    assert_eq!(
        selected.case_step_data().unwrap().choice_data(),
        ReviewDecision::Approve
    );
    assert_eq!(
        founder().select_founder_review(request(), caller).unwrap(),
        selected
    );
    founder().audit_founder_held_posts(channel, None, 3, caller).unwrap()
        .with_records(|records, unavailable| {
            assert_eq!(unavailable, 0);
            let held = records.unwrap().iter().find(|held| held.signed.id() == action).unwrap();
            assert!(matches!(held.decision, NativeTextHeldReviewDecision::Selected {choice: ReviewDecision::Approve, public_entry, ..} if public_entry == selected.id()));
        }).unwrap();

    let lost_request =
        || NativeTextInitialReviewRequest::new(channel, other, [0x97; 16], ReviewDecision::Reject);
    let (lost, key_ran) =
        with_review_control_fault(ReviewControlFault::LostReviewKeyResult, || {
            founder().select_founder_review(lost_request(), caller)
        });
    assert!(lost.is_err() && key_ran);
    let ambiguous = std::fs::read(&attempt).unwrap();
    assert!(!journal.join("key.raw").exists());
    let (retry, reached) = with_review_control_fault(ReviewControlFault::BeforeFreshKey, || {
        founder().select_founder_review(lost_request(), caller)
    });
    assert!(retry.is_err() && !reached);
    assert_eq!(std::fs::read(&attempt).unwrap(), ambiguous);
    assert!(!journal.join("key.raw").exists());
}
