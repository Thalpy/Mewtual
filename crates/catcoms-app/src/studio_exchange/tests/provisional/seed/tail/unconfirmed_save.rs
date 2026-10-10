//! G4-A1-S: Flow S over a real Unconfirmed basis, minted from a live preview.
//!
//! Every basis here comes from the production mint, `Server::mint_unconfirmed_overlay_basis`,
//! which is the only way to obtain one, and every Save goes through the production store stages.
//! Nothing is fabricated: the preview is fetched, parsed and tail-completed over the wire exactly
//! as the runtime does it.
//!
//! The saving member is a plain joiner, so it holds no observed owner tenure. That is asserted
//! once as a precondition: it is what shows no stage of an Unconfirmed Save consults a tenure
//! (design 8.5), where every Closing stage that authors would refuse.
use super::*;
use crate::store::{EpochRecordKind, StudioOverlayMint, StudioOverlayStart};
use crate::studio::StudioOwnerTenure;
use catcoms_replication::studio::{
    StudioLocalDraft, StudioOverlayProvenance, StudioOverlaySave, StudioUnconfirmedOverlayBasis,
};
use catcoms_replication::CloseRecord;
use catcoms_rt::Clock;

type Mint = Result<StudioUnconfirmedOverlayBasis, AppError>;

/// A ready preview whose authenticated tail is complete: the state the mint requires.
async fn complete_preview(p: &mut Pair) -> ServerPreparedProvisionalStudioSeed {
    let (seed, op, _) = ready(p).await;
    let pending = p
        .bob
        .prepare_provisional_studio_tail(&p.b_store, SERVER, seed)
        .unwrap();
    let completed = tail_response(p, pending, op).await;
    let prepared = p
        .bob
        .complete_provisional_studio_tail(&p.b_store, SERVER, completed)
        .unwrap()
        .unwrap()
        .prepare()
        .unwrap();
    assert!(
        prepared.tail_complete(),
        "the mint requires a complete tail"
    );
    prepared
}

fn mint(p: &Pair, prepared: &ServerPreparedProvisionalStudioSeed) -> Mint {
    p.bob
        .mint_unconfirmed_overlay_basis(&p.b_store, SERVER, prepared)
}

/// The two values a Save carries back, both from the store's own derivation: the basis
/// fingerprint and the branch `request_branch_id` names for it.
fn ticket(
    p: &mut Pair,
    target: StudioTarget,
    basis: &StudioUnconfirmedOverlayBasis,
) -> ([u8; 32], [u8; 32]) {
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    let branch = p
        .bob
        .sync
        .with_registry_context(|g, _, _, _| {
            store.studio_overlay_request_branch(SERVER, g, target, basis, &mut b)
        })
        .unwrap();
    (basis.fingerprint(), branch)
}

/// The synchronous adapter with the given mint attempts at S1b and at S3.
fn save_with(
    p: &mut Pair,
    target: StudioTarget,
    (start, commit): (Mint, Mint),
    (basis, branch): ([u8; 32], [u8; 32]),
    operation: DomainOp,
) -> Result<StudioOverlaySave, AppError> {
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    p.bob.sync.with_registry_context(|g, d, clock, rng| {
        store.save_studio_overlay(
            SERVER,
            g,
            target,
            d,
            StudioOverlayMint::unconfirmed(start),
            StudioOverlayMint::unconfirmed(commit),
            basis,
            branch,
            operation,
            clock.now_ms(),
            rng,
            &mut b,
        )
    })
}

/// The synchronous adapter with a real mint attempt from `prepared` at each stage, as a caller
/// holding a ready preview makes them.
fn save(
    p: &mut Pair,
    target: StudioTarget,
    prepared: &ServerPreparedProvisionalStudioSeed,
    ticket: ([u8; 32], [u8; 32]),
    operation: DomainOp,
) -> Result<StudioOverlaySave, AppError> {
    let mints = (mint(p, prepared), mint(p, prepared));
    save_with(p, target, mints, ticket, operation)
}

/// Two failed mint attempts, as a caller with no live preview has.
fn no_preview(reason: &str) -> (Mint, Mint) {
    (
        Err(invalid(reason.to_string())),
        Err(invalid(reason.to_string())),
    )
}

fn local(saved: StudioOverlaySave) -> StudioLocalDraft {
    match saved {
        StudioOverlaySave::Local(draft) => draft,
        other => panic!("expected a local acceptance, got {other:?}"),
    }
}

/// An exact retry's answer: the stored branch's basis and accepted count. S1a rebuilds no draft
/// (design 6.2; design 18.3 review, F1), so there is no projection to compare.
fn acknowledged(saved: StudioOverlaySave) -> ([u8; 32], usize) {
    match saved {
        StudioOverlaySave::Acknowledged { basis, accepted } => (basis, accepted),
        other => panic!("expected an exact-retry acknowledgement, got {other:?}"),
    }
}

/// A new-authoring operation the empty seed accepts: a fresh Index entry, or a Flipnote title.
fn draft_op(p: &Pair, target: StudioTarget, nonce: u8, text: &str) -> DomainOp {
    let body = match target {
        StudioTarget::Flipnote { .. } => FlipnoteOp::SetHeader(FlipnoteHeader::Title(text.into()))
            .encode()
            .unwrap(),
        StudioTarget::Index { .. } => IndexOp::PutObject {
            object: [nonce; 16],
            kind: StudioKind::Flipnote,
            title: text.into(),
            created_by: p.bob.device_id,
            ts: 1,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
    };
    domain(target, body, nonce)
}

/// The document's Intents row as the structural inventory reports it: `None` when the vault holds
/// no Intents record at all, otherwise the live branch's persisted provenance and the exact bytes
/// the row is charged, so a comparison also catches a rewrite that kept the provenance.
fn recorded(p: &mut Pair, target: StudioTarget) -> Option<(Option<StudioOverlayProvenance>, u64)> {
    let logical = target.document(&p.bob.group_id()).unwrap();
    let mut scan = p.b_store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    let inventory = scan.finish().unwrap();
    let row = inventory.records().find(|entry| {
        entry.kind == EpochRecordKind::Intents
            && entry.server == SERVER
            && entry.document == logical
    })?;
    let facts = row
        .intent_facts()
        .expect("an Intents row carries its structural facts");
    Some((facts.provenance(), facts.charged_bytes()))
}

fn installed(p: &mut Pair, target: StudioTarget) -> bool {
    let store = &p.b_store;
    p.bob
        .sync
        .with_registry_context(|g, d, _, _| store.load_studio_epoch(SERVER, g, target, d))
        .unwrap()
        .is_some()
}

/// Receive the provider's installed epoch over the network: the realistic way a confirmed source
/// arrives on this member while a Save's plan is detached. It writes the source and leaves this
/// member's Intents record alone, so the commit's stamp still matches and only the re-run mint
/// check can see the change.
async fn receive_source(p: &mut Pair, target: StudioTarget) {
    let watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target)
        .unwrap();
    let mut provider = p.alice.studio_page_provider(&p.a_store, SERVER);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let mut pass = p
        .bob
        .begin_studio_receive(&mut p.b_store, &p.watch, p.alice.local_peer(), &mut b)
        .unwrap();
    let attempt = p
        .bob
        .prepare_studio_receive_step(&mut pass)
        .unwrap()
        .unwrap();
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        p.alice.sync_once().await.unwrap();
        assert!(p
            .alice
            .serve_studio_request_step(&mut p.a_store, &mut provider, &watch)
            .unwrap()
            .is_some());
    });
    assert_eq!(
        p.bob
            .complete_studio_receive_step(&mut pass, completed)
            .unwrap(),
        StudioReceiveState::PageReady
    );
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .persist_studio_receive_step(&mut p.b_store, &mut pass, &mut b)
        .unwrap();
    assert!(
        installed(p, target),
        "the received epoch is an installed source"
    );
}

/// Make the Index document an installed ordinary source on the saving member.
fn install_source(p: &mut Pair, target: StudioTarget) {
    let logical = target.document(&p.bob.group_id()).unwrap();
    let body = IndexOp::PutObject {
        object: [0x51; 16],
        kind: StudioKind::Flipnote,
        title: "installed".into(),
        created_by: p.bob.device_id,
        ts: 1,
        expiry: StudioExpiry::Never,
    }
    .encode()
    .unwrap();
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target,
                epoch_id: epoch_zero_id(logical.doc_type, &logical.logical_key),
                nonce: [0x51; 16],
                body,
            },
        )
        .unwrap()
        .unwrap();
}

async fn preview_pair(target: StudioTarget) -> Pair {
    let mut p = pages::proven_pair().await;
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target)
        .unwrap();
    assert_eq!(
        p.bob.observed_owner_tenure(),
        StudioOwnerTenure::Unknown,
        "the saving member must hold no tenure, or this does not show an Unconfirmed Save needs none"
    );
    p
}

fn index() -> StudioTarget {
    StudioTarget::Index { channel: channel() }
}

const INSTALLED: &str = "epoch studio: this document has an installed source; an unconfirmed \
                         draft cannot start or continue beside it";

/// The first append of a branch from a live preview, for both document kinds, then a second
/// append to the live branch from a fresh mint, then an exact retry after the preview is gone.
///
/// - The branch records `Unconfirmed` with the provider and observed MLS epoch the mint saw, and
///   no installed source is written (local acceptance carries no source writes).
/// - The second mint observes a later wall-clock time. Admission facts are not fingerprinted, so
///   the fresh basis still joins the live branch: a preview refreshed after the first Save must
///   not strand it.
/// - After a restart the prepared preview belongs to a replaced mount and cannot mint, yet the
///   exact retry is answered from the stored branch. A failed mint is a value consumed only
///   by new authoring, never a precondition of classification (V8's Unconfirmed analogue); and new
///   authoring is refused with exactly that mint's error, leaving the branch as it was.
#[tokio::test]
async fn unconfirmed_save_first_append_joins_survives_refresh_and_retries_without_a_preview() {
    for target in [index(), target()] {
        let mut p = preview_pair(target).await;
        let prepared = complete_preview(&mut p).await;
        let basis = mint(&p, &prepared).unwrap();
        let provider = p.alice.device_id;
        let epoch = p.bob.sync.with_registry_context(|g, _, _, _| g.epoch());
        let now = p.clock.now_ms();
        let observed = StudioOverlayProvenance::Unconfirmed {
            provider,
            observed_mls_epoch: epoch,
            observed_at_ms: now,
        };
        assert_eq!(basis.provenance(), observed);
        let first_ticket = ticket(&mut p, target, &basis);
        assert_eq!(
            recorded(&mut p, target),
            None,
            "no record before the first Save"
        );

        let first = draft_op(&p, target, 0x21, "first");
        let draft = local(save(&mut p, target, &prepared, first_ticket, first.clone()).unwrap());
        assert_eq!(draft.basis(), first_ticket.0);
        assert_eq!(draft.accepted(), 1);
        match (target, draft.projection()) {
            (StudioTarget::Index { .. }, StudioProjection::Index(view)) => {
                assert!(view.objects.contains_key(&[0x21; 16]))
            }
            (StudioTarget::Flipnote { .. }, StudioProjection::Flipnote(view)) => {
                assert!(view.title.is_some())
            }
            _ => panic!("draft projection of the wrong kind"),
        }
        assert_eq!(
            recorded(&mut p, target).map(|(provenance, _)| provenance),
            Some(Some(observed)),
            "the branch must record the admission facts its own mint observed"
        );
        assert!(
            !installed(&mut p, target),
            "local acceptance writes no source"
        );

        // A later mint: different observation time, same fingerprint, same live branch.
        p.clock.advance_ms(1_000);
        let refreshed = mint(&p, &prepared).unwrap();
        assert_ne!(refreshed.provenance(), observed);
        let refreshed_ticket = ticket(&mut p, target, &refreshed);
        assert_eq!(
            refreshed_ticket, first_ticket,
            "a refreshed mint names the same basis and the live branch"
        );
        let second = draft_op(&p, target, 0x22, "second");
        let draft = local(save(&mut p, target, &prepared, refreshed_ticket, second).unwrap());
        assert_eq!(draft.accepted(), 2);
        assert_eq!(
            recorded(&mut p, target).map(|(provenance, _)| provenance),
            Some(Some(observed)),
            "provenance is the branch's first admission, never a later append's observation"
        );

        // Restart: the preview is gone for good, and no mint can succeed.
        drop(p.b_store);
        p.b_store = open(p.b_root.path());
        let lost = mint(&p, &prepared).unwrap_err().to_string();
        let retried = acknowledged(
            save_with(
                &mut p,
                target,
                no_preview(&lost),
                first_ticket,
                first.clone(),
            )
            .unwrap(),
        );
        assert_eq!(
            retried,
            (first_ticket.0, 2),
            "the exact retry is answered from the stored branch"
        );

        let before = recorded(&mut p, target);
        let new_work = draft_op(&p, target, 0x23, "new after restart");
        let refused = save(&mut p, target, &prepared, first_ticket, new_work).unwrap_err();
        assert_eq!(
            refused.to_string(),
            lost,
            "new authoring surfaces the mint's own failure"
        );
        assert_eq!(
            recorded(&mut p, target),
            before,
            "the refusal changed nothing"
        );
        let unchanged = acknowledged(
            save_with(&mut p, target, no_preview("gone"), first_ticket, first).unwrap(),
        );
        assert_eq!(unchanged, (first_ticket.0, 2));
        assert!(!installed(&mut p, target));
    }
}

/// An Unconfirmed draft exists only where no installed source does (8.1, 8.5). Sync's mint cannot
/// see the store, so the store refuses at S1b, under custody, even though the preview is still
/// live and its mint succeeds.
#[tokio::test]
async fn unconfirmed_save_refuses_beside_an_installed_source() {
    let target = index();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    install_source(&mut p, target);
    assert!(
        mint(&p, &prepared).is_ok(),
        "the preview itself is still live"
    );
    // The ordinary Apply left its own Intents row; the refusal must leave it exactly as it is.
    let before = recorded(&mut p, target);
    assert_eq!(before.map(|(provenance, _)| provenance), Some(None));
    let operation = draft_op(&p, target, 0x31, "beside a source");
    let refused = save(&mut p, target, &prepared, ticket, operation).unwrap_err();
    assert_eq!(refused.to_string(), INSTALLED);
    assert_eq!(
        recorded(&mut p, target),
        before,
        "the refused Save opened no branch and rewrote nothing"
    );
}

/// S3 re-enters the live check. In the staged form, each of these happens while the plan is
/// detached, and the commit refuses and writes nothing:
///
/// - `received`: the confirmed epoch arrives over the network. This member's Intents record is
///   untouched, so the stamp still matches and the refusal can only come from the commit's own
///   installed-source check. This is the case that check exists for.
/// - `local`: an ordinary local Apply installs a source. That also moves the Intents record, so
///   the stamp answers first; it is kept to pin that the commit still writes nothing.
/// - `expiry`: the preview outlives its hint. The plan carries an owned basis, never the preview,
///   so only the commit's own mint attempt can authorize the write, and here it fails.
#[tokio::test]
async fn unconfirmed_save_commit_reruns_the_live_check_after_the_detached_plan() {
    for change in ["received", "local", "expiry"] {
        // A received source needs a provider that has one; the Flipnote fixture saves it there.
        let target = if change == "received" {
            target()
        } else {
            index()
        };
        let mut p = preview_pair(target).await;
        if change == "received" {
            p.save(&title(0x61, "confirmed elsewhere"));
        }
        let prepared = complete_preview(&mut p).await;
        let basis = mint(&p, &prepared).unwrap();
        let (fingerprint, branch) = ticket(&mut p, target, &basis);
        let operation = draft_op(&p, target, 0x41, "detached");
        let start = mint(&p, &prepared);
        let mut b = budget(&mut p.bob, &mut p.b_store);
        let store = &mut p.b_store;
        let captured = p.bob.sync.with_registry_context(|g, d, clock, rng| {
            store.start_studio_overlay(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(start),
                fingerprint,
                branch,
                operation,
                clock.now_ms(),
                rng,
                &mut b,
            )
        });
        let StudioOverlayStart::Captured(capture) = captured.unwrap() else {
            panic!("a first append must capture");
        };
        let plan = capture.plan().unwrap();

        let before = recorded(&mut p, target);
        let (commit, expected) = match change {
            "received" => {
                receive_source(&mut p, target).await;
                assert_eq!(
                    recorded(&mut p, target),
                    before,
                    "precondition: the receive left this member's Intents record alone"
                );
                (mint(&p, &prepared), INSTALLED.to_string())
            }
            "local" => {
                install_source(&mut p, target);
                (
                    mint(&p, &prepared),
                    "epoch studio: overlay record or context changed; retry".to_string(),
                )
            }
            _ => {
                // Past the hint's lifetime: the same preview can no longer mint.
                p.clock.advance_ms(60_000);
                let commit = mint(&p, &prepared);
                let expected = commit.as_ref().unwrap_err().to_string();
                (commit, expected)
            }
        };
        let before = recorded(&mut p, target);
        let mut b = budget(&mut p.bob, &mut p.b_store);
        let store = &mut p.b_store;
        let refused = p
            .bob
            .sync
            .with_registry_context(|g, d, _, rng| {
                store.commit_studio_overlay_with(
                    SERVER,
                    g,
                    target,
                    d,
                    StudioOverlayMint::unconfirmed(commit),
                    plan,
                    rng,
                    &mut b,
                )
            })
            .unwrap_err();
        assert_eq!(refused.to_string(), expected, "{change}");
        assert_eq!(
            recorded(&mut p, target),
            before,
            "{change}: nothing written"
        );
        assert!(
            before.is_none_or(|(provenance, _)| provenance.is_none()),
            "{change}: no branch was opened"
        );
    }
}

/// The refusals a Save can meet before any media work, worded for the Unconfirmed kind: a request
/// carrying another basis, a request naming a branch the next admission would not open, and a
/// basis minted for a different channel of the same Flipnote object.
///
/// The last one is why the store compares the basis's target. A Flipnote's logical key omits its
/// channel, so the identity admission compares cannot tell the two apart, and without the check a
/// branch would be opened from a basis for one channel while the request names another.
#[tokio::test]
async fn unconfirmed_save_refuses_stale_requests_and_a_basis_for_another_channel() {
    let target = target();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let (fingerprint, branch) = ticket(&mut p, target, &basis);

    let operation = draft_op(&p, target, 0x51, "other basis");
    let changed = save(&mut p, target, &prepared, ([0xee; 32], branch), operation).unwrap_err();
    assert_eq!(
        changed.to_string(),
        "epoch studio: unconfirmed overlay basis changed; prepare again from the current preview"
    );
    let operation = draft_op(&p, target, 0x52, "other branch");
    let stale = save(
        &mut p,
        target,
        &prepared,
        (fingerprint, [0xee; 32]),
        operation,
    )
    .unwrap_err();
    assert_eq!(
        stale.to_string(),
        "epoch studio: unconfirmed overlay request names a stale branch; prepare it again"
    );

    let StudioTarget::Flipnote { object, .. } = target else {
        unreachable!()
    };
    let elsewhere = StudioTarget::Flipnote {
        channel: crate::channel_id("elsewhere").to_be_bytes(),
        object,
    };
    assert_eq!(
        elsewhere.document(&p.bob.group_id()).unwrap(),
        target.document(&p.bob.group_id()).unwrap(),
        "precondition: the two targets share one logical document"
    );
    let operation = draft_op(&p, elsewhere, 0x53, "wrong channel");
    let crossed = save(
        &mut p,
        elsewhere,
        &prepared,
        (fingerprint, branch),
        operation,
    )
    .unwrap_err();
    assert_eq!(
        crossed.to_string(),
        format!(
            "epoch studio: {}",
            catcoms_replication::ReplError::EpochScope
        )
    );
    assert_eq!(
        recorded(&mut p, target),
        None,
        "every refusal wrote nothing"
    );
}

/// A durably accepted Unconfirmed operation stays answerable after the confirmed epoch arrives.
///
/// The response to the first Save is lost, a source is then received over the network, and the
/// client retries exactly. Classification answers the retry before the installed-source check is
/// reached, so it is acknowledged rather than refused. New authoring beside the source refuses.
/// Without this, moving the presence check ahead of classification would pass every other test:
/// the restart retry has no source, and the remaining tests author new work.
#[tokio::test]
async fn unconfirmed_save_exact_retry_is_answered_beside_a_newly_received_source() {
    let target = target();
    let mut p = preview_pair(target).await;
    p.save(&title(0x71, "confirmed elsewhere"));
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let first = draft_op(&p, target, 0x72, "accepted, response lost");
    let accepted = local(save(&mut p, target, &prepared, ticket, first.clone()).unwrap());
    assert_eq!(accepted.accepted(), 1);

    receive_source(&mut p, target).await;
    let before = recorded(&mut p, target);
    let retried = acknowledged(save(&mut p, target, &prepared, ticket, first).unwrap());
    assert_eq!(retried, (ticket.0, 1), "acknowledged, not appended again");
    assert_eq!(recorded(&mut p, target), before, "the retry opened nothing");

    let new_work = draft_op(&p, target, 0x73, "new beside the source");
    let refused = save(&mut p, target, &prepared, ticket, new_work).unwrap_err();
    assert_eq!(refused.to_string(), INSTALLED);
}

/// Presence, not readability, is what refuses.
///
/// A stored source that is corrupt is still a source this device cannot vouch is absent; one
/// unlinked while the budget still accounts it must refuse through the budget rather than read as
/// absent. Both mint attempts are a sentinel failure, and the sentinel must NOT be what comes
/// back: the source check comes first, so hearing the sentinel would mean the damaged record was
/// taken for absence and the mint opened.
#[tokio::test]
async fn unconfirmed_save_refuses_a_corrupt_or_unlinked_source_rather_than_reading_it_as_absent() {
    for damage in ["corrupt", "unlinked"] {
        let target = index();
        let mut p = preview_pair(target).await;
        let prepared = complete_preview(&mut p).await;
        let basis = mint(&p, &prepared).unwrap();
        let ticket = ticket(&mut p, target, &basis);
        install_source(&mut p, target);
        let logical = target.document(&p.bob.group_id()).unwrap();
        let store = &p.b_store;
        let path = p
            .bob
            .sync
            .with_registry_context(|g, _, _, _| {
                store.studio_source_path_for_test(SERVER, g, target)
            })
            .unwrap();
        // Taken while the record is intact, so the budget still accounts it.
        let mut b = budget(&mut p.bob, &mut p.b_store);
        if damage == "corrupt" {
            std::fs::write(&path, b"not a sealed studio record").unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        let operation = draft_op(&p, target, 0x81, "beside damage");
        let sentinel = || StudioOverlayMint::unconfirmed(Err(invalid("sentinel mint failure")));
        let store = &mut p.b_store;
        let refused = p
            .bob
            .sync
            .with_registry_context(|g, d, clock, rng| {
                store.save_studio_overlay(
                    SERVER,
                    g,
                    target,
                    d,
                    sentinel(),
                    sentinel(),
                    ticket.0,
                    ticket.1,
                    operation,
                    clock.now_ms(),
                    rng,
                    &mut b,
                )
            })
            .unwrap_err()
            .to_string();
        assert!(
            !refused.contains("sentinel"),
            "{damage}: the damaged source was read as absent: {refused}"
        );
        let expected = if damage == "corrupt" {
            INSTALLED.to_string()
        } else {
            // The budget still accounts the record the probe found absent.
            format!(
                "epoch studio: {}",
                crate::store::epoch_budget::BudgetError::Inventory
            )
        };
        assert_eq!(refused, expected, "{damage}");
        let intents = p.b_store.load_epoch_intents(SERVER, &logical).unwrap();
        assert!(
            intents.overlay().is_none(),
            "{damage}: no branch was opened"
        );
        assert_eq!(
            intents.pending().len(),
            1,
            "{damage}: the ordinary intent is untouched"
        );
    }
}

/// A basis is evidence of one moment. One minted before an MLS epoch change and kept is refused,
/// at S1b, even though its fingerprint still matches and no source exists: it skipped every
/// recheck the sanctioned mint makes inside sync's hint callback (membership, epoch, expiry).
/// The sanctioned mint cannot produce this, which is why the test hands the store the stale value
/// directly, as a careless caller would.
#[tokio::test]
async fn unconfirmed_save_refuses_a_basis_minted_under_an_earlier_mls_epoch() {
    let target = index();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let stale = (mint(&p, &prepared), mint(&p, &prepared));
    let epoch = p.bob.sync.with_registry_context(|g, _, _, _| g.epoch());

    // A real membership commit: a third member joins, and the saving member processes it.
    p.bob.subscribe_control().await.unwrap();
    let invite = p.alice.mint_invite([92; 16], u64::MAX, vec![]).unwrap();
    let (third, tick) = tokio::join!(
        Server::join(
            Net::new(p.hub.join(PeerId::from_u64(3))),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(p.clock.clone()),
            "third",
            p.alice.local_peer(),
            &invite
        ),
        p.alice.sync_once()
    );
    third.unwrap();
    tick.unwrap();
    while p.bob.sync.with_registry_context(|g, _, _, _| g.epoch())
        != p.alice.sync.with_registry_context(|g, _, _, _| g.epoch())
    {
        p.bob.sync_once().await.unwrap();
    }
    assert_ne!(
        p.bob.sync.with_registry_context(|g, _, _, _| g.epoch()),
        epoch,
        "precondition: the MLS epoch moved"
    );

    let operation = draft_op(&p, target, 0x91, "stale basis");
    let refused = save_with(&mut p, target, stale, ticket, operation).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "epoch studio: unconfirmed overlay basis was minted under another membership; prepare \
         again from the current preview"
    );
    assert_eq!(recorded(&mut p, target), None, "nothing was written");
}

/// The staged success path, and what "replacement" really covers.
///
/// - S1b mints from preview A; the plan runs; the preview is then fetched again as B, a separate
///   discovery, seed and tail; the commit mints from B and succeeds. A and B are previews of the
///   same candidate, so they bind the same seed and receipt and differ only in admission facts.
/// - The branch keeps A's admission facts: provenance is recorded at admission, once.
/// - After a restart, a third preview C of the same candidate appends new work to the
///   reconstructed branch, so the basis kind survives a reload through the app path.
#[tokio::test]
async fn unconfirmed_save_commits_from_a_replacement_preview_and_after_restart() {
    let target = index();
    let mut p = preview_pair(target).await;
    let a = complete_preview(&mut p).await;
    let first_basis = mint(&p, &a).unwrap();
    let admitted = first_basis.provenance();
    let first_ticket = ticket(&mut p, target, &first_basis);
    let operation = draft_op(&p, target, 0xa1, "staged");
    let start = mint(&p, &a);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    let captured = p.bob.sync.with_registry_context(|g, d, clock, rng| {
        store.start_studio_overlay(
            SERVER,
            g,
            target,
            d,
            StudioOverlayMint::unconfirmed(start),
            first_ticket.0,
            first_ticket.1,
            operation,
            clock.now_ms(),
            rng,
            &mut b,
        )
    });
    let StudioOverlayStart::Captured(capture) = captured.unwrap() else {
        panic!("a first append must capture");
    };
    let plan = capture.plan().unwrap();

    drop(a);
    p.clock.advance_ms(1_000);
    let replacement = complete_preview(&mut p).await;
    let commit = mint(&p, &replacement);
    assert_eq!(
        commit.as_ref().unwrap().fingerprint(),
        first_ticket.0,
        "precondition: the replacement previews the same candidate"
    );
    assert_ne!(commit.as_ref().unwrap().provenance(), admitted);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    let committed = p
        .bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            store.commit_studio_overlay_with(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(commit),
                plan,
                rng,
                &mut b,
            )
        })
        .unwrap();
    assert_eq!(committed.accepted(), 1);
    assert_eq!(
        recorded(&mut p, target).map(|(provenance, _)| provenance),
        Some(Some(admitted)),
        "the branch keeps its first admission's facts"
    );

    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target)
        .unwrap();
    let after_restart = complete_preview(&mut p).await;
    let basis = mint(&p, &after_restart).unwrap();
    assert_eq!(
        ticket(&mut p, target, &basis),
        first_ticket,
        "the reconstructed branch is live"
    );
    let operation = draft_op(&p, target, 0xa2, "after restart");
    let draft = local(save(&mut p, target, &after_restart, first_ticket, operation).unwrap());
    assert_eq!(draft.accepted(), 2);
    assert_eq!(
        recorded(&mut p, target).map(|(provenance, _)| provenance),
        Some(Some(admitted))
    );
}

/// A plan is committed only with a mint of its own kind. An Unconfirmed plan handed a Closing
/// mint is refused before anything reads the store, with text that names that mistake rather
/// than a missing source or a changed Closing basis.
#[tokio::test]
async fn unconfirmed_plan_refuses_a_closing_commit_mint_by_name() {
    let target = index();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let operation = draft_op(&p, target, 0xb1, "wrong kind at commit");
    let start = mint(&p, &prepared);
    let tenure = p.bob.observed_owner_tenure();
    // Minting `b` makes `stale` stale. The commit is handed `stale`, so if the kind check ever ran
    // after budget entry the answer would be "Studio budget is stale" instead: this pins it first.
    let mut stale = budget(&mut p.bob, &mut p.b_store);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    let refused = p
        .bob
        .sync
        .with_registry_context(|g, d, clock, rng| {
            let StudioOverlayStart::Captured(capture) = store.start_studio_overlay(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(start),
                ticket.0,
                ticket.1,
                operation,
                clock.now_ms(),
                rng,
                &mut b,
            )?
            else {
                panic!("a first append must capture");
            };
            let plan = capture.plan()?;
            let logical = target.document(&g.group_id()).unwrap();
            let close = CloseRecord::sign(&logical, 4, 0, vec![[1; 32]], d).unwrap();
            store.commit_studio_overlay_with(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::Closing {
                    close: &close,
                    tenure,
                },
                plan,
                rng,
                &mut stale,
            )
        })
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "epoch studio: overlay plan and commit mint are for different kinds of draft"
    );
    assert_eq!(recorded(&mut p, target), None, "nothing was written");
}

/// The reverse pairing: a real Closing plan, made by the founder with a Known tenure on an
/// installed Closing source, handed an Unconfirmed mint, refuses by name. The mint is a sentinel
/// failure, and the fixture's own budget mints leave the commit's budget stale, so hearing either
/// the sentinel or "stale" would mean the kind check no longer runs first.
#[tokio::test]
async fn closing_plan_refuses_an_unconfirmed_commit_mint_by_name() {
    let mut p = pages::proven_pair().await;
    // The capture fixture authors Flipnote title edits, so its target must be a Flipnote.
    let target = target();
    let mut b = budget(&mut p.alice, &mut p.a_store);
    let store = &mut p.a_store;
    let refused = p
        .alice
        .sync
        .with_registry_context(|g, d, _, rng| {
            let capture =
                crate::store::studio_closing_capture_fixture(store, SERVER, g, d, target, false);
            let plan = capture.plan()?;
            store.commit_studio_overlay_with(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(Err(invalid("sentinel mint failure"))),
                plan,
                rng,
                &mut b,
            )
        })
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "epoch studio: overlay plan and commit mint are for different kinds of draft"
    );
}

/// A live Unconfirmed branch survives a membership change. The freshness guard compares a fresh
/// mint with the CURRENT group, never with the epoch the branch recorded at admission, so after a
/// third member joins, a preview fetched under the new MLS epoch appends to the same branch and the
/// branch keeps its first admission's facts. Tightening the guard to the branch's recorded epoch
/// would strand every live draft at the first membership change; this is what would catch it.
#[tokio::test]
async fn unconfirmed_branch_accepts_a_fresh_mint_after_an_mls_epoch_change() {
    let target = index();
    let mut p = preview_pair(target).await;
    let first_preview = complete_preview(&mut p).await;
    let basis = mint(&p, &first_preview).unwrap();
    let admitted = basis.provenance();
    let first_ticket = ticket(&mut p, target, &basis);
    let first = draft_op(&p, target, 0xc1, "before the join");
    assert_eq!(
        local(save(&mut p, target, &first_preview, first_ticket, first).unwrap()).accepted(),
        1
    );
    let epoch = p.bob.sync.with_registry_context(|g, _, _, _| g.epoch());

    p.bob.subscribe_control().await.unwrap();
    let invite = p.alice.mint_invite([93; 16], u64::MAX, vec![]).unwrap();
    let (third, tick) = tokio::join!(
        Server::join(
            Net::new(p.hub.join(PeerId::from_u64(3))),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(p.clock.clone()),
            "third",
            p.alice.local_peer(),
            &invite
        ),
        p.alice.sync_once()
    );
    third.unwrap();
    tick.unwrap();
    while p.bob.sync.with_registry_context(|g, _, _, _| g.epoch())
        != p.alice.sync.with_registry_context(|g, _, _, _| g.epoch())
    {
        p.bob.sync_once().await.unwrap();
    }
    assert_ne!(
        p.bob.sync.with_registry_context(|g, _, _, _| g.epoch()),
        epoch,
        "precondition: the MLS epoch moved"
    );

    drop(first_preview);
    let after = complete_preview(&mut p).await;
    let fresh = mint(&p, &after).unwrap();
    assert_eq!(ticket(&mut p, target, &fresh), first_ticket);
    let second = draft_op(&p, target, 0xc2, "after the join");
    let draft = local(save(&mut p, target, &after, first_ticket, second).unwrap());
    assert_eq!(draft.accepted(), 2);
    assert_eq!(
        recorded(&mut p, target).map(|(provenance, _)| provenance),
        Some(Some(admitted)),
        "the branch keeps its first admission's facts across the membership change"
    );
}

/// The S1b binding beyond the target (review L1): a basis is refused before any media work when it
/// was minted for another device, or names another group's document.
///
/// The sanctioned mint always uses the minting device and its own group, so both are built by
/// handing the store a real basis from the wrong place. For the author, the provider Alice offers
/// the store Bob's basis; without the check it would be refused only at S2 by `append`, with a
/// different error, after media admission. For the document, a basis from a second, independent
/// group is offered to the first; without that check the author check would answer instead, since
/// the other group's member is another device.
#[tokio::test]
async fn unconfirmed_save_refuses_a_basis_for_another_device_or_group_at_s1b() {
    let target = target();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let operation = draft_op(&p, target, 0xd1, "under another device");
    let mints = (mint(&p, &prepared), mint(&p, &prepared));
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    let refused = p
        .alice
        .sync
        .with_registry_context(|g, d, clock, rng| {
            store.save_studio_overlay(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(mints.0),
                StudioOverlayMint::unconfirmed(mints.1),
                ticket.0,
                ticket.1,
                operation,
                clock.now_ms(),
                rng,
                &mut b,
            )
        })
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "epoch studio: unconfirmed overlay basis was minted for another device"
    );

    let mut other = preview_pair(target).await;
    assert_ne!(
        other.bob.group_id(),
        p.bob.group_id(),
        "precondition: two independent groups"
    );
    let elsewhere_preview = complete_preview(&mut other).await;
    let elsewhere = (
        mint(&other, &elsewhere_preview),
        mint(&other, &elsewhere_preview),
    );
    let fingerprint = elsewhere.0.as_ref().unwrap().fingerprint();
    let operation = draft_op(&p, target, 0xd2, "from another group");
    let refused = save_with(
        &mut p,
        target,
        elsewhere,
        (fingerprint, ticket.1),
        operation,
    )
    .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "epoch studio: unconfirmed overlay basis is for another group or document"
    );
    assert_eq!(recorded(&mut p, target), None, "nothing was written");
}

/// Agent 2's review M1, Agent 1's half: the H1 handoff probe transfers only Closing branches.
///
/// An Unconfirmed branch is never transferable (8.5): refused at H1 while no source is installed,
/// and at H2's provenance guard once one is. Selecting it therefore took a reservation and a
/// doubling backoff each period, or a capture and a detached worker, for nothing. The probe must
/// read it, memoise it as having nothing to transfer, start no job and hold nothing back, both
/// before and after a confirmed source arrives.
#[tokio::test]
async fn the_handoff_probe_leaves_an_unconfirmed_branch_alone() {
    let target = target();
    let mut p = preview_pair(target).await;
    // The provider's own edit, so it holds a source it can later serve to Bob (as in
    // `unconfirmed_save_exact_retry_is_answered_beside_a_newly_received_source`).
    p.save(&title(0x40, "confirmed elsewhere"));
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let operation = draft_op(&p, target, 0x41, "a draft on a preview");
    local(save(&mut p, target, &prepared, ticket, operation).unwrap());
    assert!(
        matches!(
            recorded(&mut p, target),
            Some((Some(StudioOverlayProvenance::Unconfirmed { .. }), _))
        ),
        "precondition: this member's live branch is Unconfirmed"
    );

    // A receiver watching the document, as the saving actor's is, and one probe as a background
    // turn runs it. Whole visits would also start catch-up fetches from Alice, which this pair
    // does not serve, and which the probe does not need.
    let mut receiver = crate::studio::StudioReceiver::default();
    receiver.inject_overlay_pool_for_test(4);
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    let probe = |p: &mut Pair, receiver: &mut crate::studio::StudioReceiver| {
        receiver.handoff_probe_for_test(&mut p.bob, &mut p.b_store, SERVER);
        let now = p.clock.monotonic_ms();
        assert!(
            !receiver.handoff_held_for_test(target, now),
            "the probe selected an Unconfirmed branch and backed the document off"
        );
        assert!(
            !receiver.handoff_has_job_for_test(),
            "the probe started a handoff of an Unconfirmed branch"
        );
        assert!(
            receiver.handoff_quiet_for_test(&p.b_store, target),
            "the probe never recorded the Unconfirmed branch as having nothing to transfer"
        );
    };
    probe(&mut p, &mut receiver);

    // A confirmed source arrives (a Studio write, which does not rotate the intent generation).
    // The branch is still Unconfirmed, and a fresh receiver, which has no memo to rely on, must
    // read it again and leave it alone in this state too. The receiver's own watch replaced the
    // pair's page watch, so that is re-established for the receive first.
    drop(receiver);
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target)
        .unwrap();
    receive_source(&mut p, target).await;
    assert!(matches!(
        recorded(&mut p, target),
        Some((Some(StudioOverlayProvenance::Unconfirmed { .. }), _))
    ));
    let mut receiver = crate::studio::StudioReceiver::default();
    receiver.inject_overlay_pool_for_test(4);
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    probe(&mut p, &mut receiver);
}
