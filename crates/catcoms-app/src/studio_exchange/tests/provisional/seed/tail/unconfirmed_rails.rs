//! Design 8.3's per-server and vault-wide rails for drafts made on a preview, at Flow S's two
//! admission points: S1b, before any media work, and S3, at the exact size the commit would write.
//!
//! Every Save here is real: a fetched, tail-complete preview, the production mint, and the
//! production store stages, split as the runtime runs them. What is not real is the rest of the
//! vault. Three genuine preview drafts on one server would need three previews of three documents,
//! so the rails are occupied with stand-in records through a test hook on the budget. The hook moves
//! only the Unconfirmed tally, so a refusal can only be a rail's.
use super::*;
use crate::store::{EpochRecordKind, EpochStudioBudget, StudioOverlayMint, StudioOverlayStart};
use catcoms_replication::studio::{StudioOverlayProvenance, StudioUnconfirmedOverlayBasis};

const SERVER_FULL: &str = "unconfirmed draft limit reached";
const SHARE_FULL: &str = "unconfirmed draft storage limit reached";

/// A ready preview whose authenticated tail is complete: the state the mint requires.
async fn complete_preview(p: &mut Pair) -> ServerPreparedProvisionalStudioSeed {
    let (seed, op, _) = ready(p).await;
    let pending = p
        .bob
        .prepare_provisional_studio_tail(&p.b_store, SERVER, seed)
        .unwrap();
    let completed = tail_response(p, pending, op).await;
    p.bob
        .complete_provisional_studio_tail(&p.b_store, SERVER, completed)
        .unwrap()
        .unwrap()
        .prepare()
        .unwrap()
}

fn mint(
    p: &Pair,
    prepared: &ServerPreparedProvisionalStudioSeed,
) -> Result<StudioUnconfirmedOverlayBasis, AppError> {
    p.bob
        .mint_unconfirmed_overlay_basis(&p.b_store, SERVER, prepared)
}

/// The fingerprint and branch a Save carries back, from the store's own derivation.
fn ticket(
    p: &mut Pair,
    target: StudioTarget,
    prepared: &ServerPreparedProvisionalStudioSeed,
) -> ([u8; 32], [u8; 32]) {
    let basis = mint(p, prepared).unwrap();
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    let branch = p
        .bob
        .sync
        .with_registry_context(|g, _, _, _| {
            store.studio_overlay_request_branch(SERVER, g, target, &basis, &mut b)
        })
        .unwrap();
    (basis.fingerprint(), branch)
}

/// A fresh Index entry, which the empty seed accepts as new authoring.
fn entry(p: &Pair, target: StudioTarget, nonce: u8) -> DomainOp {
    let body = IndexOp::PutObject {
        object: [nonce; 16],
        kind: StudioKind::Flipnote,
        title: "drawn on a preview".into(),
        created_by: p.bob.device_id,
        ts: 1,
        expiry: StudioExpiry::Never,
    }
    .encode()
    .unwrap();
    domain(target, body, nonce)
}

/// One Save split as the runtime runs it: S1 and S1b, the detached plan, then S3. `occupy` shapes
/// the budget both custody stages enter, as the rest of the vault would.
fn save(
    p: &mut Pair,
    target: StudioTarget,
    prepared: &ServerPreparedProvisionalStudioSeed,
    ticket: ([u8; 32], [u8; 32]),
    operation: DomainOp,
    occupy: impl Fn(&mut EpochStudioBudget),
) -> Result<catcoms_replication::studio::StudioLocalDraft, (&'static str, AppError)> {
    save_between(p, target, prepared, ticket, operation, &occupy, &occupy)
}

/// As [`save`], with the rest of the vault allowed to change between the two custody stages, as
/// it does when another draft is admitted while this one's plan is detached.
fn save_between(
    p: &mut Pair,
    target: StudioTarget,
    prepared: &ServerPreparedProvisionalStudioSeed,
    (basis, branch): ([u8; 32], [u8; 32]),
    operation: DomainOp,
    at_start: &dyn Fn(&mut EpochStudioBudget),
    at_commit: &dyn Fn(&mut EpochStudioBudget),
) -> Result<catcoms_replication::studio::StudioLocalDraft, (&'static str, AppError)> {
    let attempt = mint(p, prepared);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    at_start(&mut b);
    let store = &mut p.b_store;
    let started = p
        .bob
        .sync
        .with_registry_context(|g, d, clock, rng| {
            store.start_studio_overlay(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(attempt),
                basis,
                branch,
                operation,
                clock.now_ms(),
                rng,
                &mut b,
            )
        })
        .map_err(|error| ("S1b", error))?;
    let capture = match started {
        StudioOverlayStart::Captured(capture) => capture,
        // Answered at S1, before any rail: an exact retry of accepted work.
        StudioOverlayStart::Settled(saved) => match *saved {
            catcoms_replication::studio::StudioOverlaySave::Local(draft) => return Ok(draft),
            other => panic!("expected a local answer, got {other:?}"),
        },
    };
    let plan = capture.plan().unwrap();
    let attempt = mint(p, prepared);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    at_commit(&mut b);
    let store = &mut p.b_store;
    p.bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            store.commit_studio_overlay_with(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(attempt),
                plan,
                rng,
                &mut b,
            )
        })
        .map_err(|error| ("S3", error))
}

/// The document's Intents row: its live provenance and its charged bytes, or `None` with no row.
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
    let facts = row.intent_facts().unwrap();
    Some((facts.provenance(), facts.charged_bytes()))
}

async fn preview_pair() -> (Pair, StudioTarget, ServerPreparedProvisionalStudioSeed) {
    let mut p = pages::proven_pair().await;
    let target = StudioTarget::Index { channel: channel() };
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target)
        .unwrap();
    let prepared = complete_preview(&mut p).await;
    (p, target, prepared)
}

fn refused_by(
    result: Result<impl std::fmt::Debug, (&'static str, AppError)>,
    stage: &str,
    text: &str,
) {
    match result {
        Err((at, error)) => {
            assert_eq!(at, stage, "refused at the wrong stage: {error}");
            assert!(error.to_string().contains(text), "{error}");
        }
        Ok(value) => panic!("expected a refusal at {stage} ({text}), got {value:?}"),
    }
}

/// The per-server count. With three drafts on this server already, a fourth branch is refused at
/// S1b and nothing is written. A competing draft admitted while this one's plan is detached is
/// caught at S3, from the budget that stage enters, and nothing is written either. With two it is
/// admitted, so the refusals were the count's.
#[tokio::test]
async fn studio_unconfirmed_rail_refuses_a_fourth_draft_on_a_server_at_s1b() {
    let (mut p, target, prepared) = preview_pair().await;
    let ticket = ticket(&mut p, target, &prepared);

    let op = entry(&p, target, 0x41);
    let full = save(&mut p, target, &prepared, ticket, op, |b| {
        b.occupy_unconfirmed_rails_for_test(SERVER, 3, u64::MAX)
    });
    refused_by(full, "S1b", SERVER_FULL);
    assert_eq!(
        recorded(&mut p, target),
        None,
        "a refused draft wrote nothing"
    );

    let op = entry(&p, target, 0x41);
    let raced = save_between(
        &mut p,
        target,
        &prepared,
        ticket,
        op,
        &|b| b.occupy_unconfirmed_rails_for_test(SERVER, 2, u64::MAX),
        &|b| b.occupy_unconfirmed_rails_for_test(SERVER, 3, u64::MAX),
    );
    refused_by(raced, "S3", SERVER_FULL);
    assert_eq!(recorded(&mut p, target), None, "S3's refusal wrote nothing");

    let op = entry(&p, target, 0x41);
    let draft = save(&mut p, target, &prepared, ticket, op, |b| {
        b.occupy_unconfirmed_rails_for_test(SERVER, 2, u64::MAX)
    })
    .expect("a third draft on the server is admitted");
    assert_eq!(draft.accepted(), 1);
}

/// A live branch is one of the server's drafts, not an extra one: it keeps appending on a full
/// server. The vault-wide share then refuses growth, at S1b when the record as it stands already
/// fills the room, and at S3 when only the size the commit would write passes it. Neither writes.
/// An exact retry of accepted work is still answered with both rails full.
#[tokio::test]
async fn studio_unconfirmed_rail_lets_a_live_branch_append_and_refuses_growth_past_the_share() {
    let (mut p, target, prepared) = preview_pair().await;
    let ticket = ticket(&mut p, target, &prepared);
    let op = entry(&p, target, 0x51);
    save(&mut p, target, &prepared, ticket, op, |_| {}).unwrap();

    let op = entry(&p, target, 0x52);
    let draft = save(&mut p, target, &prepared, ticket, op, |b| {
        b.occupy_unconfirmed_rails_for_test(SERVER, 3, u64::MAX)
    })
    .expect("the live branch appends on a full server");
    assert_eq!(draft.accepted(), 2);

    let before = recorded(&mut p, target);
    let (_, current) = before.expect("the draft's record");
    // Room for the record exactly as it stands: S1b passes, and the larger record S3 would write
    // does not fit.
    let op = entry(&p, target, 0x53);
    let grown = save(&mut p, target, &prepared, ticket, op, |b| {
        b.occupy_unconfirmed_rails_for_test(SERVER, 0, current)
    });
    refused_by(grown, "S3", SHARE_FULL);
    assert_eq!(
        recorded(&mut p, target),
        before,
        "S3's refusal wrote nothing"
    );

    // One byte less: the record as it stands already overflows, so S1b refuses before media work.
    let op = entry(&p, target, 0x53);
    let full = save(&mut p, target, &prepared, ticket, op, |b| {
        b.occupy_unconfirmed_rails_for_test(SERVER, 0, current - 1)
    });
    refused_by(full, "S1b", SHARE_FULL);
    assert_eq!(
        recorded(&mut p, target),
        before,
        "S1b's refusal wrote nothing"
    );

    // With both rails full, accepted work is still answered: an exact retry is settled at S1,
    // before any rail, and writes nothing new. Only growth is refused, never acknowledgement.
    let op = entry(&p, target, 0x52);
    let retried = save(&mut p, target, &prepared, ticket, op, |b| {
        b.occupy_unconfirmed_rails_for_test(SERVER, 3, 0)
    })
    .expect("an exact retry is answered at capacity");
    assert_eq!(retried.accepted(), 2);
    assert_eq!(recorded(&mut p, target), before);
}

/// The tally the rails read. A fresh budget takes it from the inventory's authenticated facts:
/// this draft's record, on this server, at its charged bytes. And the commit's own budget follows
/// its write, so a later check on that same budget sees the draft it just admitted.
#[tokio::test]
async fn studio_unconfirmed_rail_tally_comes_from_the_inventory_and_follows_the_commit() {
    let (mut p, target, prepared) = preview_pair().await;
    let fresh = budget(&mut p.bob, &mut p.b_store);
    assert!(
        fresh.unconfirmed_tally_for_test().is_empty(),
        "precondition: no draft yet"
    );
    let ticket = ticket(&mut p, target, &prepared);

    // The commit stage by hand, to read its budget after the write.
    let op = entry(&p, target, 0x61);
    let attempt = mint(&p, &prepared);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    let StudioOverlayStart::Captured(capture) = p
        .bob
        .sync
        .with_registry_context(|g, d, clock, rng| {
            store.start_studio_overlay(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(attempt),
                ticket.0,
                ticket.1,
                op,
                clock.now_ms(),
                rng,
                &mut b,
            )
        })
        .unwrap()
    else {
        panic!("new authoring captures")
    };
    let plan = capture.plan().unwrap();
    let attempt = mint(&p, &prepared);
    let store = &mut p.b_store;
    p.bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            store.commit_studio_overlay_with(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(attempt),
                plan,
                rng,
                &mut b,
            )
        })
        .unwrap();
    let (provenance, charged) = recorded(&mut p, target).unwrap();
    assert!(matches!(
        provenance,
        Some(StudioOverlayProvenance::Unconfirmed { .. })
    ));
    assert_eq!(
        b.unconfirmed_tally_for_test(),
        vec![(SERVER, charged)],
        "the commit's budget counts the draft it just wrote, at its written size"
    );
    let fresh = budget(&mut p.bob, &mut p.b_store);
    assert_eq!(
        fresh.unconfirmed_tally_for_test(),
        vec![(SERVER, charged)],
        "a fresh budget counts it from the inventory"
    );
    // And after the vault is reopened: the tally is rebuilt from what was persisted, not carried.
    drop(fresh);
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    let reopened = budget(&mut p.bob, &mut p.b_store);
    assert_eq!(
        reopened.unconfirmed_tally_for_test(),
        vec![(SERVER, charged)],
        "a reopened vault counts the draft it holds"
    );
}

/// What else moves the tally (review of the rails, MEDIUM-1 and LOW-1).
///
/// - **Ordinary growth.** Once the confirmed checkpoint is installed (8.6 `baseConfirmed`), an
///   ordinary edit lands in the same intent record as the live branch. The tally counts that
///   record whole, so it grows too, and no rail is consulted: the share is admission policy at
///   Flow S, never a refusal of ordinary editing (that would self-lock, as the archive sub-cap's
///   reasoning explains). Pinned here so the choice is a test, not an accident.
/// - **Disposal.** Disposing the branch leaves terminal metadata, which never counts. The
///   disposal's own budget drops the record as it writes, and a fresh budget agrees.
#[tokio::test]
async fn studio_unconfirmed_rail_tally_follows_ordinary_growth_and_drops_a_disposed_branch() {
    let (mut p, target, prepared) = preview_pair().await;
    let ticket = ticket(&mut p, target, &prepared);
    let op = entry(&p, target, 0x71);
    save(&mut p, target, &prepared, ticket, op, |_| {}).unwrap();
    let (_, drafted) = recorded(&mut p, target).unwrap();

    // The confirmed checkpoint arrives, then an ordinary edit on the installed source.
    let (receipt, seed) = candidate(&mut p, target);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let clock = p.clock.clone();
    let store = &mut p.b_store;
    let (_, installed) = p
        .bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            store.adopt_studio_checkpoint(
                SERVER,
                g,
                target,
                d,
                &receipt,
                Some(seed.bytes()),
                0,
                &clock,
                rng,
                &mut b,
            )
        })
        .unwrap();
    let edit = entry(&p, target, 0x72);
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target,
                epoch_id: installed.doc_id(),
                nonce: edit.nonce,
                body: edit.body,
            },
        )
        .unwrap();
    let (provenance, grown) = recorded(&mut p, target).unwrap();
    assert!(
        matches!(
            provenance,
            Some(StudioOverlayProvenance::Unconfirmed { .. })
        ),
        "precondition: the branch is still live"
    );
    assert!(
        grown > drafted,
        "precondition: the ordinary edit grew the record"
    );
    assert_eq!(
        budget(&mut p.bob, &mut p.b_store).unconfirmed_tally_for_test(),
        vec![(SERVER, grown)],
        "the tally counts the record whole, ordinary intents included"
    );

    // Disposal, on a budget of its own, read back after the write.
    let logical = target.document(&p.bob.group_id()).unwrap();
    let (branch, content, accepted, _) =
        p.b_store.studio_branch_identity_for_test(SERVER, &logical);
    let request = crate::store::StudioOverlayDisposalRequest {
        branch,
        content,
        accepted,
        mode: crate::store::StudioDisposalRequestMode::Discard(
            catcoms_replication::studio::StudioDiscardConfirmation::parse(
                catcoms_replication::studio::StudioDiscardConfirmation::TOKEN,
            )
            .unwrap(),
        ),
    };
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    p.bob
        .sync
        .with_registry_context(|g, d, clock, rng| {
            store.dispose_studio_overlay(
                SERVER,
                &logical,
                target,
                g,
                d,
                request,
                clock.now_ms(),
                rng,
                &mut b,
            )
        })
        .unwrap();
    assert!(
        b.unconfirmed_tally_for_test().is_empty(),
        "the disposal's own budget drops the record"
    );
    assert!(
        budget(&mut p.bob, &mut p.b_store)
            .unconfirmed_tally_for_test()
            .is_empty(),
        "terminal metadata never counts"
    );
}
