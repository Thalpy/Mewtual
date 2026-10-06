//! Design 8.7 through the actor's receiver: the Unconfirmed Save's two control actions, over a real
//! preview driven to ready by the production queue and jobs.
//!
//! Agent 1's `unconfirmed_save` tests drive the store stages with mints they make themselves. These
//! drive the receiver, which makes its own mint attempt from its ready-preview cache at each stage,
//! so they are what shows the mint is made in the visit that consumes it, and that the scheduled
//! detach runs through the production background path.
use super::runtime::drive;
use super::*;
use crate::studio::{
    PreviewHarness, StudioControlAction, StudioControlRequest, StudioControlResponse,
    StudioReceiver, StudioSettlementState, StudioUnconfirmedOverlaySaveRequest,
    StudioUnconfirmedSaveOutcome,
};
use crate::studio_exchange::ServerStudioWatch;
use catcoms_replication::studio::StudioOverlayProvenance;

fn control(
    receiver: &mut StudioReceiver,
    p: &mut Pair,
    target: StudioTarget,
    action: StudioControlAction,
) -> Result<StudioControlResponse, AppError> {
    receiver
        .control(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            StudioControlRequest { target, action },
        )
        .map(|(_, _, response)| response.expect("a control response"))
}

fn save(
    receiver: &mut StudioReceiver,
    p: &mut Pair,
    target: StudioTarget,
    request: &StudioUnconfirmedOverlaySaveRequest,
) -> Result<StudioUnconfirmedSaveOutcome, AppError> {
    let action = StudioControlAction::SaveUnconfirmedOverlay(Box::new(
        StudioUnconfirmedOverlaySaveRequest {
            basis: request.basis,
            branch: request.branch,
            nonce: request.nonce,
            body: request.body.clone(),
        },
    ));
    match control(receiver, p, target, action)? {
        StudioControlResponse::UnconfirmedOverlaySaved { target: t, outcome } => {
            assert_eq!(t, target);
            Ok(outcome)
        }
        other => panic!("expected an Unconfirmed Save outcome, got {other:?}"),
    }
}

/// Run the receiver's background work the way the actor does, until it has none left.
async fn settle(receiver: &mut StudioReceiver, p: &mut Pair) {
    for _ in 0..8 {
        let Some(work) = receiver.detach(&mut p.bob) else {
            return;
        };
        receiver.complete(&mut p.bob, work.run(None).await);
    }
}

fn recorded_provenance(p: &mut Pair, target: StudioTarget) -> Option<StudioOverlayProvenance> {
    let logical = target.document(&p.bob.group_id()).unwrap();
    let state = p
        .b_store
        .load_epoch_intents_structural(SERVER, &logical)
        .unwrap();
    state.handoff_metadata().map(|m| m.provenance())
}

/// A receiver holding one real, ready preview of the channel's Index, and the ticket for a Save on
/// it. The saving member is a plain joiner, so no stage of the Save may consult a tenure (8.5).
async fn ready_receiver() -> (Pair, StudioReceiver, StudioTarget, [u8; 32], [u8; 32]) {
    let mut p = pages::proven_pair().await;
    let target = StudioTarget::Index { channel: channel() };
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target)
        .unwrap();
    let mut runtime = PreviewHarness::default();
    drive(&mut p, &mut runtime).await;
    let watch = ServerStudioWatch {
        inner: p.watch.inner.copy_binding(),
        mount: p.watch.mount.clone(),
        server: SERVER,
        target,
    };
    let logical = target.document(&p.bob.group_id()).unwrap();
    let epoch_id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let mut receiver = runtime.into_receiver(vec![(watch, epoch_id)]);
    assert_eq!(
        p.bob.observed_owner_tenure(),
        crate::studio::StudioOwnerTenure::Unknown
    );
    // The ticket, minted from the live preview in this visit.
    let StudioControlResponse::UnconfirmedOverlaySaveTicket {
        target: ticketed,
        basis,
        branch,
    } = control(
        &mut receiver,
        &mut p,
        target,
        StudioControlAction::BeginUnconfirmedOverlaySave,
    )
    .unwrap()
    else {
        panic!("expected a ticket")
    };
    assert_eq!(ticketed, target);
    (p, receiver, target, basis, branch)
}

/// A new Index entry, as a Save request under the given ticket.
fn new_entry(
    p: &mut Pair,
    (basis, branch): ([u8; 32], [u8; 32]),
    nonce: u8,
    object: [u8; 16],
) -> StudioUnconfirmedOverlaySaveRequest {
    let owner = p.bob.sync.with_registry_context(|_, d, _, _| d.device_id());
    StudioUnconfirmedOverlaySaveRequest {
        basis,
        branch,
        nonce: [nonce; 16],
        body: IndexOp::PutObject {
            object,
            kind: StudioKind::Flipnote,
            title: "drawn on a preview".into(),
            created_by: owner,
            ts: 1,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
    }
}

fn pending(p: &mut Pair, target: StudioTarget) -> usize {
    let logical = target.document(&p.bob.group_id()).unwrap();
    p.b_store
        .load_epoch_intents(SERVER, &logical)
        .unwrap()
        .pending()
        .len()
}

#[tokio::test]
async fn studio_actor_unconfirmed_save_schedules_commits_and_retries_from_the_live_preview() {
    let (mut p, mut receiver, target, basis, branch) = ready_receiver().await;
    let logical = target.document(&p.bob.group_id()).unwrap();

    // --- New authoring: the first visit captures and schedules; nothing is durable yet.
    let request = new_entry(&mut p, (basis, branch), 41, [8; 16]);
    assert!(matches!(
        save(&mut receiver, &mut p, target, &request).unwrap(),
        StudioUnconfirmedSaveOutcome::Scheduled
    ));
    assert_eq!(recorded_provenance(&mut p, target), None, "not durable yet");

    // The detached plan runs through the production background path, then the identical request
    // commits it, re-minting from the live preview in that visit.
    settle(&mut receiver, &mut p).await;
    let StudioUnconfirmedSaveOutcome::Saved {
        basis: saved_basis,
        accepted,
    } = save(&mut receiver, &mut p, target, &request).unwrap()
    else {
        panic!("the commit visit saves")
    };
    assert_eq!((saved_basis, accepted), (basis, 1));
    assert!(
        matches!(
            recorded_provenance(&mut p, target),
            Some(StudioOverlayProvenance::Unconfirmed { .. })
        ),
        "the branch records Unconfirmed provenance"
    );
    // Local draft data only: no installed source, no Registry pointer, no receipt. The one pending
    // intent is the draft's own accepted operation (the shared `assert_absent` demands none).
    let pointer = PointerKey::new(logical.doc_type, logical.logical_key.clone()).unwrap();
    p.bob.sync.with_registry_context(|g, d, _, _| {
        assert!(p
            .b_store
            .load_studio_epoch(SERVER, g, target, d)
            .unwrap()
            .is_none());
        assert!(p
            .b_store
            .load_registry_epoch(SERVER, g, pointer.bucket(), d)
            .unwrap()
            .is_none());
    });
    let receipts = p
        .b_store
        .load_epoch_owner_receipts(SERVER, &logical)
        .unwrap();
    assert!(receipts.pending().is_none() && receipts.published().is_none());
    assert_eq!(
        p.b_store
            .load_epoch_intents(SERVER, &logical)
            .unwrap()
            .pending()
            .len(),
        1
    );

    // --- An exact retry is acknowledged and adds nothing.
    assert!(matches!(
        save(&mut receiver, &mut p, target, &request).unwrap(),
        StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. }
    ));

    // --- S3 re-enters the live check in its own visit. New work is scheduled while the preview is
    // live. The preview goes away (expiry, eviction or restart) while the plan waits, and the
    // commit visit refuses with the mint's own reason, writing nothing.
    let second = StudioUnconfirmedOverlaySaveRequest {
        basis,
        branch,
        nonce: [42; 16],
        body: IndexOp::SetTitle {
            object: [8; 16],
            title: "after the preview went".into(),
        }
        .encode()
        .unwrap(),
    };
    let scheduled = save(&mut receiver, &mut p, target, &second).unwrap();
    assert!(
        matches!(scheduled, StudioUnconfirmedSaveOutcome::Scheduled),
        "a later append is scheduled too: {scheduled:?}"
    );
    settle(&mut receiver, &mut p).await;
    receiver.clear_previews();
    let refused = save(&mut receiver, &mut p, target, &second)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("no live preview"),
        "the commit visit mints from the live preview, not a parked basis: {refused}"
    );
    assert_eq!(
        p.b_store
            .load_epoch_intents(SERVER, &logical)
            .unwrap()
            .pending()
            .len(),
        1,
        "the refused commit wrote nothing"
    );

    // --- With no preview, accepted work is still answered; new authoring and a new ticket are
    // refused, with the mint's own reason.
    assert!(
        matches!(
            save(&mut receiver, &mut p, target, &request).unwrap(),
            StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. }
        ),
        "an exact retry needs no live preview"
    );
    let refused = control(
        &mut receiver,
        &mut p,
        target,
        StudioControlAction::BeginUnconfirmedOverlaySave,
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("no live preview"), "{refused}");
    // The same new work again: its plan was consumed by the refused commit, so this is a fresh
    // first visit, and S1b refuses it.
    let refused = save(&mut receiver, &mut p, target, &second)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("no live preview"),
        "new authoring with no preview surfaces the mint's refusal: {refused}"
    );
}

/// Two requests interleaved on one document. A schedules; before A's caller returns, B's visit
/// finds A's parked plan.
///
/// B's visit finishes A's work, so the slot cannot be held hostage by a caller who never comes
/// back. But it reports `Busy`, never `Saved`: none of B was saved. A's own retry is then answered
/// as an exact retry, and B proceeds on its own.
#[tokio::test]
async fn studio_actor_unconfirmed_save_never_reports_another_requests_plan_as_its_own() {
    let (mut p, mut receiver, target, basis, branch) = ready_receiver().await;
    let a = new_entry(&mut p, (basis, branch), 51, [8; 16]);
    let b = new_entry(&mut p, (basis, branch), 52, [9; 16]);
    assert!(matches!(
        save(&mut receiver, &mut p, target, &a).unwrap(),
        StudioUnconfirmedSaveOutcome::Scheduled
    ));
    settle(&mut receiver, &mut p).await;

    let visit = save(&mut receiver, &mut p, target, &b).unwrap();
    assert!(
        matches!(visit, StudioUnconfirmedSaveOutcome::Busy),
        "B must not be told A's work was its own: {visit:?}"
    );
    assert_eq!(pending(&mut p, target), 1, "A's work landed in B's visit");

    assert!(
        matches!(
            save(&mut receiver, &mut p, target, &a).unwrap(),
            StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. }
        ),
        "A's retry is answered as an exact retry"
    );
    assert!(matches!(
        save(&mut receiver, &mut p, target, &b).unwrap(),
        StudioUnconfirmedSaveOutcome::Scheduled
    ));
    settle(&mut receiver, &mut p).await;
    assert!(matches!(
        save(&mut receiver, &mut p, target, &b).unwrap(),
        StudioUnconfirmedSaveOutcome::Saved { accepted: 2, .. }
    ));
    assert_eq!(pending(&mut p, target), 2);
}

/// Review of `b35e23d2`, HIGH-1 and MEDIUM-2. The slot is one per actor, so a plan parked for one
/// target whose caller never returns must not block a Save on another target.
///
/// A's plan for the Index is parked and A goes away. B, a Save on a Flipnote of the same channel,
/// finishes A's plan in its visit and is told `Busy`, since none of B was saved. The commit emits
/// a refresh notice for A's document. B's next visit is no longer blocked: it reaches its own mint,
/// which refuses because this fixture has no preview of B's Flipnote. A's work is durable, and
/// A's own retry is an exact retry.
#[tokio::test]
async fn studio_actor_unconfirmed_save_a_parked_plan_never_blocks_another_target() {
    let (mut p, mut receiver, index, basis, branch) = ready_receiver().await;
    let a = new_entry(&mut p, (basis, branch), 71, [8; 16]);
    assert!(matches!(
        save(&mut receiver, &mut p, index, &a).unwrap(),
        StudioUnconfirmedSaveOutcome::Scheduled
    ));
    settle(&mut receiver, &mut p).await;
    let _ = receiver.take_settlement_notices();

    let flipnote = StudioTarget::Flipnote {
        channel: channel(),
        object: [0x77; 16],
    };
    let b = StudioUnconfirmedOverlaySaveRequest {
        basis: [1; 32],
        branch: [2; 32],
        nonce: [72; 16],
        body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("b".into()))
            .encode()
            .unwrap(),
    };
    let visit = save(&mut receiver, &mut p, flipnote, &b).unwrap();
    assert!(
        matches!(visit, StudioUnconfirmedSaveOutcome::Busy),
        "B is told nothing of it was saved: {visit:?}"
    );
    assert!(
        receiver
            .take_settlement_notices()
            .contains(&(index, StudioSettlementState::RefreshRequired)),
        "finishing A's plan changed A's document, so its row is refreshed"
    );
    assert_eq!(pending(&mut p, index), 1, "A's work landed in B's visit");

    let refused = save(&mut receiver, &mut p, flipnote, &b)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("no live preview"),
        "B is no longer blocked by the slot; it reaches its own mint: {refused}"
    );
    assert!(matches!(
        save(&mut receiver, &mut p, index, &a).unwrap(),
        StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. }
    ));
}

/// Review of `b35e23d2`, LOW-1: a retry while this request's own plan is in flight is `Scheduled`,
/// not `Busy`. "Nothing of this request was saved" would be untrue of a plan already running.
#[tokio::test]
async fn studio_actor_unconfirmed_save_retry_of_its_own_scheduled_plan_is_pending() {
    let (mut p, mut receiver, target, basis, branch) = ready_receiver().await;
    let a = new_entry(&mut p, (basis, branch), 81, [8; 16]);
    for _ in 0..2 {
        let visit = save(&mut receiver, &mut p, target, &a).unwrap();
        assert!(
            matches!(visit, StudioUnconfirmedSaveOutcome::Scheduled),
            "a retry of its own in-flight plan is pending, not busy: {visit:?}"
        );
    }
    settle(&mut receiver, &mut p).await;
    assert!(matches!(
        save(&mut receiver, &mut p, target, &a).unwrap(),
        StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. }
    ));
}

/// The confirmed checkpoint arrives on this member: the very receipt and seed the preview was of,
/// installed by checkpoint adoption, as discovery installs it after a fresh owner proof.
///
/// The fixture's preview is of a synthetic candidate that the owner never installs, so there are no
/// pages to receive; adoption is the confirmed path this member really takes. The tenure is the
/// proof's claim (the owner's start, 0 here), handed in directly. The proof exchange itself is
/// discovery's, tested there.
fn install_confirmed_checkpoint(p: &mut Pair, target: StudioTarget) {
    let (receipt, seed) = candidate(p, target);
    install_checkpoint(p, target, &receipt, &seed);
}

/// Another owner checkpoint of the same document: the same empty projection, under another close.
/// Installed instead of the preview's, it is a confirmed source that is not the branch's base.
fn other_checkpoint(p: &mut Pair, target: StudioTarget) -> (Receipt, CheckpointSeed) {
    p.alice.sync.with_registry_context(|g, d, _, _| {
        let seed = StudioEpoch::new(g, target, d.device_id())
            .unwrap()
            .projection()
            .unwrap()
            .checkpoint([8; 32])
            .unwrap();
        let receipt = Receipt::sign(
            target.document(&g.group_id()).unwrap(),
            0,
            [8; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        (receipt, seed)
    })
}

fn install_checkpoint(
    p: &mut Pair,
    target: StudioTarget,
    receipt: &Receipt,
    seed: &CheckpointSeed,
) {
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let clock = p.clock.clone();
    let store = &mut p.b_store;
    let (outcome, _) = p
        .bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            store.adopt_studio_checkpoint(
                SERVER,
                g,
                target,
                d,
                receipt,
                Some(seed.bytes()),
                0,
                &clock,
                rng,
                &mut b,
            )
        })
        .unwrap();
    assert_eq!(outcome, crate::store::StudioAdoptionOutcome::Installed);
}

/// The lifecycle row's design 8.6 field, read through the ordinary control transaction.
fn reconciliation(
    receiver: &mut StudioReceiver,
    p: &mut Pair,
    target: StudioTarget,
) -> Option<catcoms_replication::studio::StudioOverlayUnconfirmedState> {
    match control(receiver, p, target, StudioControlAction::OverlayLifecycle).unwrap() {
        StudioControlResponse::OverlayLifecycle(lifecycle) => lifecycle.unconfirmed,
        other => panic!("expected the lifecycle row, got {other:?}"),
    }
}

/// Design 8.6 on a real Unconfirmed branch, derived on every read and never written. With no
/// installed source it is awaiting one. When the confirmed checkpoint is installed, the installed
/// source is the very checkpoint the branch was based on, and the row says so. Nothing about the
/// branch changes: it is still Unconfirmed, still local, still this device's.
#[tokio::test]
async fn studio_actor_unconfirmed_branch_reconciles_from_awaiting_to_confirmed_on_read() {
    use catcoms_replication::studio::StudioOverlayUnconfirmedState as U;
    let (mut p, mut receiver, target, basis, branch) = ready_receiver().await;
    assert_eq!(
        reconciliation(&mut receiver, &mut p, target),
        None,
        "no branch, no reconciliation"
    );
    let request = new_entry(&mut p, (basis, branch), 61, [8; 16]);
    assert!(matches!(
        save(&mut receiver, &mut p, target, &request).unwrap(),
        StudioUnconfirmedSaveOutcome::Scheduled
    ));
    settle(&mut receiver, &mut p).await;
    assert!(matches!(
        save(&mut receiver, &mut p, target, &request).unwrap(),
        StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. }
    ));
    assert_eq!(
        reconciliation(&mut receiver, &mut p, target),
        Some(U::AwaitingSource)
    );

    install_confirmed_checkpoint(&mut p, target);
    assert_eq!(
        reconciliation(&mut receiver, &mut p, target),
        Some(U::BaseConfirmed),
        "the installed source is the checkpoint the branch was based on"
    );
    assert!(
        matches!(
            recorded_provenance(&mut p, target),
            Some(StudioOverlayProvenance::Unconfirmed { .. })
        ),
        "a confirmed base promotes nothing: the branch is still Unconfirmed"
    );
    assert_eq!(pending(&mut p, target), 1);
}

/// The other two app-level arms of design 8.6 (re-review of `5ccc4647`, LOW-5). A confirmed source
/// that is not the branch's base, here another owner checkpoint under another close, reads
/// `BaseSuperseded`. The same source made unreadable on disk reads `SourceUnreadable`, never
/// `AwaitingSource`: a record is there, and it cannot be vouched for as absent.
#[tokio::test]
async fn studio_actor_unconfirmed_branch_reconciles_superseded_then_unreadable() {
    use catcoms_replication::studio::StudioOverlayUnconfirmedState as U;
    let (mut p, mut receiver, target, basis, branch) = ready_receiver().await;
    let request = new_entry(&mut p, (basis, branch), 91, [8; 16]);
    assert!(matches!(
        save(&mut receiver, &mut p, target, &request).unwrap(),
        StudioUnconfirmedSaveOutcome::Scheduled
    ));
    settle(&mut receiver, &mut p).await;
    assert!(matches!(
        save(&mut receiver, &mut p, target, &request).unwrap(),
        StudioUnconfirmedSaveOutcome::Saved { accepted: 1, .. }
    ));

    let (receipt, seed) = other_checkpoint(&mut p, target);
    install_checkpoint(&mut p, target, &receipt, &seed);
    assert_eq!(
        reconciliation(&mut receiver, &mut p, target),
        Some(U::BaseSuperseded),
        "a confirmed source from another checkpoint supersedes the branch's base"
    );

    let store = &p.b_store;
    let path = p
        .bob
        .sync
        .with_registry_context(|g, _, _, _| store.studio_source_path_for_test(SERVER, g, target))
        .unwrap();
    std::fs::write(&path, b"not a sealed studio record").unwrap();
    assert_eq!(
        reconciliation(&mut receiver, &mut p, target),
        Some(U::SourceUnreadable),
        "a source that is there but unreadable is neither awaited nor compared"
    );
}

/// Both actions go through the receiver and its scope checks; neither can be reached through the
/// control transaction, and neither answers for a channel this server does not know.
#[tokio::test]
async fn studio_unconfirmed_save_actions_refuse_outside_the_receiver_and_an_unknown_channel() {
    let mut p = pages::proven_pair().await;
    let target = StudioTarget::Index { channel: channel() };
    for action in [
        StudioControlAction::BeginUnconfirmedOverlaySave,
        StudioControlAction::SaveUnconfirmedOverlay(Box::new(
            StudioUnconfirmedOverlaySaveRequest {
                basis: [1; 32],
                branch: [2; 32],
                nonce: [3; 16],
                body: Vec::new(),
            },
        )),
    ] {
        let refused = p
            .bob
            .studio_control_transaction(
                &mut p.b_store,
                SERVER,
                StudioControlRequest { target, action },
            )
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("requires the actor's live preview"),
            "{refused}"
        );
    }

    let mut receiver = PreviewHarness::default().into_receiver(Vec::new());
    let unknown = StudioTarget::Index {
        channel: 0xdead_u128.to_be_bytes(),
    };
    let refused = control(
        &mut receiver,
        &mut p,
        unknown,
        StudioControlAction::BeginUnconfirmedOverlaySave,
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("unknown Studio channel"), "{refused}");

    // A body past the operation bound is refused by the request's own validation, before any
    // channel check or store read.
    let oversized = StudioControlAction::SaveUnconfirmedOverlay(Box::new(
        StudioUnconfirmedOverlaySaveRequest {
            basis: [1; 32],
            branch: [2; 32],
            nonce: [3; 16],
            body: vec![0; catcoms_replication::epoch::MAX_DOMAIN_OP_BYTES + 1],
        },
    ));
    let refused = control(&mut receiver, &mut p, target, oversized)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("exceeds the operation bound"), "{refused}");
}
