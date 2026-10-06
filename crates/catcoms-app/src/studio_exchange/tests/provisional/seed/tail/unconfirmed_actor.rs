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
    StudioReceiver, StudioUnconfirmedOverlaySaveRequest, StudioUnconfirmedSaveOutcome,
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
}
