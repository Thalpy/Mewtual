use super::*;
use crate::studio_exchange::{ServerStudioReceive, StudioReceiveState};

#[tokio::test]
async fn studio_ready_page_saves_before_due_owner_prepares_another_source() {
    for preparation_delay in [0, 6_000] {
        let mut p = proven_pair().await;
        let (proof, tick) = tokio::join!(
            p.alice
                .sync
                .request_catchup(p.bob.local_peer(), catcoms_wire::DocType::Wiki, 43),
            p.bob.sync_once(),
        );
        proof.unwrap();
        tick.unwrap();
        let other = StudioTarget::Flipnote {
            channel: channel(),
            object: [8; 16],
        };
        p.alice.sync.with_registry_context(|g, d, _, _| {
            crate::store::save_studio_source_fixture(&mut p.a_store, SERVER, g, d, other)
        });
        let mut receiver = crate::studio::StudioReceiver::default();
        receiver
            .run(
                &mut p.alice,
                &mut p.a_store,
                SERVER,
                Some(StudioRequest::Read { target: other }),
            )
            .unwrap();
        let op = title(91, "held page wins");
        let logical = target().document(&p.bob.group_id()).unwrap();
        p.bob
            .studio_transaction(
                &mut p.b_store,
                SERVER,
                StudioRequest::Apply {
                    target: target(),
                    epoch_id: epoch_zero_id(logical.doc_type, &logical.logical_key),
                    nonce: op.nonce,
                    body: op.body,
                },
            )
            .unwrap();
        // Save/read A displaces B, so an incorrectly prioritized owner turn would capture B.
        p.save(&title(92, "local source"));
        receiver
            .run(
                &mut p.alice,
                &mut p.a_store,
                SERVER,
                Some(StudioRequest::Read { target: target() }),
            )
            .unwrap();
        let watch = p
            .alice
            .watch_studio_epoch(&p.a_store, SERVER, target())
            .unwrap();
        let mut budget = budget(&mut p.alice, &mut p.a_store);
        let mut pass = p
            .alice
            .begin_studio_receive(&mut p.a_store, &watch, p.bob.local_peer(), &mut budget)
            .unwrap();
        let mut provider = p.bob.studio_page_provider(&p.b_store, SERVER);
        // Independent frontiers first restart, then request the complete bounded prefix.
        for _ in 0..2 {
            let job = p
                .alice
                .prepare_studio_receive_step(&mut pass)
                .unwrap()
                .unwrap();
            let (result, ()) = tokio::join!(job.fetch(), async {
                p.bob.sync_once().await.unwrap();
                p.bob
                    .serve_studio_request_step(&mut p.b_store, &mut provider, &p.watch)
                    .unwrap();
            });
            if p.alice
                .complete_studio_receive_step(&mut pass, result)
                .unwrap()
                == StudioReceiveState::PageReady
            {
                break;
            }
            p.clock.advance_ms(1000);
        }
        receiver.hold_page_for_test(target(), pass);
        p.clock.advance_ms(preparation_delay);
        let (_, updated) = receiver
            .run(&mut p.alice, &mut p.a_store, SERVER, None)
            .unwrap();
        assert_eq!(updated, Some(target()));
        assert!(
            receiver.detach(&mut p.alice).is_none(),
            "no owner capture before held page save"
        );
        assert!(!receiver.take_pause_notice());
        let state = p
            .alice
            .sync
            .with_registry_context(|g, d, _, _| p.a_store.load_studio_epoch(SERVER, g, target(), d))
            .unwrap()
            .unwrap();
        assert_eq!(state.op_count(), 2);
    }
}

/// A cold held page has priority over new client work, but when the shared preparation pool is
/// full it cannot make inbound checkpoint service unreachable. The service step must at least
/// reserve the request; because capture cannot get a permit either, it stays explicitly
/// uncaptured and H3 is then allowed to sign past it until capacity returns.
#[tokio::test]
async fn a_cold_held_page_with_a_full_pool_still_reserves_checkpoint_service() {
    let mut p = proven_pair().await;
    let (proof, tick) = tokio::join!(
        p.alice
            .sync
            .request_catchup(p.bob.local_peer(), catcoms_wire::DocType::Wiki, 43),
        p.bob.sync_once(),
    );
    proof.unwrap();
    tick.unwrap();

    let other = StudioTarget::Flipnote {
        channel: channel(),
        object: [0xA8; 16],
    };
    p.alice.sync.with_registry_context(|g, d, _, _| {
        crate::store::save_studio_source_fixture(&mut p.a_store, SERVER, g, d, other)
    });
    let op = title(181, "held while pool full");
    let logical = target().document(&p.bob.group_id()).unwrap();
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: epoch_zero_id(logical.doc_type, &logical.logical_key),
                nonce: op.nonce,
                body: op.body,
            },
        )
        .unwrap();
    p.save(&title(182, "local source"));

    let mut receiver = crate::studio::StudioReceiver::default();
    receiver
        .run(
            &mut p.alice,
            &mut p.a_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    let watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target())
        .unwrap();
    let mut b = budget(&mut p.alice, &mut p.a_store);
    let mut pass = p
        .alice
        .begin_studio_receive(&mut p.a_store, &watch, p.bob.local_peer(), &mut b)
        .unwrap();
    let mut provider = p.bob.studio_page_provider(&p.b_store, SERVER);
    for _ in 0..2 {
        let job = p
            .alice
            .prepare_studio_receive_step(&mut pass)
            .unwrap()
            .unwrap();
        let (result, ()) = tokio::join!(job.fetch(), async {
            p.bob.sync_once().await.unwrap();
            p.bob
                .serve_studio_request_step(&mut p.b_store, &mut provider, &p.watch)
                .unwrap();
        });
        if p.alice
            .complete_studio_receive_step(&mut pass, result)
            .unwrap()
            == StudioReceiveState::PageReady
        {
            break;
        }
        p.clock.advance_ms(1000);
    }
    assert_eq!(pass.state(), StudioReceiveState::PageReady);
    receiver.hold_page_for_test(target(), pass);

    // Displace the target's sole prepared graph after the page is ready, then make the shared pool
    // deterministically full. Persisting the page and capturing a cold service request both need a
    // permit, so the ordering—not eventual completion—is the property under test.
    p.alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Read { target: other },
        )
        .unwrap();
    let pool = receiver.inject_overlay_pool_for_test(0);
    let request = p
        .bob
        .sync
        .prepare_checkpoint_head(
            p.alice.local_peer(),
            catcoms_sync::checkpoint_exchange::CheckpointTarget::Studio(target()),
        )
        .unwrap();
    let fetch = tokio::spawn(request.fetch());
    tokio::task::yield_now().await;
    for _ in 0..8 {
        if p.alice.sync.has_epoch_service_interest() {
            break;
        }
        p.alice.sync_once().await.unwrap();
    }
    assert!(p.alice.sync.has_epoch_service_interest());

    receiver
        .run(&mut p.alice, &mut p.a_store, SERVER, None)
        .unwrap();
    assert!(
        receiver.has_page_for_test(),
        "the blocked page was discarded"
    );
    assert_eq!(pool.available_permits(), 0);
    assert!(
        !p.alice.sync.has_epoch_service_interest(),
        "the early page return left the service request queued and unreachable"
    );
    assert_eq!(
        receiver.service_capture_for_test(),
        Some(false),
        "the full-pool service request was not retained as explicitly uncaptured"
    );
    assert!(
        !fetch.is_finished(),
        "the blocked request was spuriously answered"
    );
    fetch.abort();
}

pub(super) async fn proven_pair() -> Pair {
    let mut p = Pair::new().await;
    let (proof, tick) = tokio::join!(
        p.bob
            .sync
            .request_catchup(p.alice.local_peer(), catcoms_wire::DocType::Wiki, 43),
        p.alice.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    p
}

async fn transfer_page(
    p: &mut Pair,
    pass: &mut ServerStudioReceive,
    provider: &mut ServerStudioPageProvider,
    watch: &ServerStudioWatch,
) {
    let attempt = p.bob.prepare_studio_receive_step(pass).unwrap().unwrap();
    // The receiver's live Server is deliberately free while its request is suspended.
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        p.alice.sync_once().await.unwrap();
        assert!(p
            .alice
            .serve_studio_request_step(&mut p.a_store, provider, watch)
            .unwrap()
            .is_some());
    });
    assert_eq!(
        p.bob.complete_studio_receive_step(pass, completed).unwrap(),
        StudioReceiveState::PageReady
    );
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .persist_studio_receive_step(&mut p.b_store, pass, &mut b)
        .unwrap();
}

#[tokio::test]
async fn studio_network_pages_distinct_members_resume_durable_prefix_and_restart() {
    let mut p = proven_pair().await;
    for n in 1..=70 {
        p.save(&title(n, &format!("page {n}")));
    }
    let watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target())
        .unwrap();
    let mut provider = p.alice.studio_page_provider(&p.a_store, SERVER);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let mut pass = p
        .bob
        .begin_studio_receive(&mut p.b_store, &p.watch, p.alice.local_peer(), &mut b)
        .unwrap();
    for page in 1..=3 {
        transfer_page(&mut p, &mut pass, &mut provider, &watch).await;
        assert_eq!(pass.progress().saved_pages, page);
        p.clock.advance_ms(1000);
    }
    assert_eq!(pass.state(), StudioReceiveState::PrefixComplete);
    assert_eq!(pass.progress().accepted, 70);
    assert_eq!(
        p.pending(),
        70,
        "remote receipt never retires sender intents"
    );
    assert_eq!(p.state().unwrap().op_count(), 70);
    let snapshot = p.bob.snapshot().unwrap();
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    p.bob = Server::restore(
        &snapshot,
        p.bob.sync.transport().clone(),
        rng(),
        Box::new(p.clock.clone()),
        "bob",
    )
    .unwrap();
    assert_eq!(p.state().unwrap().op_count(), 70);
}

#[tokio::test]
async fn studio_network_pages_cancelled_attempt_and_replaced_watch_cannot_advance() {
    let mut p = proven_pair().await;
    p.save(&title(1, "saved"));
    let watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target())
        .unwrap();
    let mut provider = p.alice.studio_page_provider(&p.a_store, SERVER);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let mut pass = p
        .bob
        .begin_studio_receive(&mut p.b_store, &p.watch, p.alice.local_peer(), &mut b)
        .unwrap();
    drop(
        p.bob
            .prepare_studio_receive_step(&mut pass)
            .unwrap()
            .unwrap(),
    );
    assert_eq!(pass.state(), StudioReceiveState::Paused);
    assert_eq!(pass.progress().attempts, 1);
    pass.retry();
    assert!(
        p.bob
            .prepare_studio_receive_step(&mut pass)
            .unwrap()
            .is_none(),
        "cancellation does not refund pacing"
    );
    p.clock.advance_ms(1000);
    let attempt = p
        .bob
        .prepare_studio_receive_step(&mut pass)
        .unwrap()
        .unwrap();
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        p.alice.sync_once().await.unwrap();
        p.alice
            .serve_studio_request_step(&mut p.a_store, &mut provider, &watch)
            .unwrap();
    });
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target())
        .unwrap();
    assert!(p
        .bob
        .complete_studio_receive_step(&mut pass, completed)
        .is_err());
    assert_eq!(pass.state(), StudioReceiveState::Stopped);
    assert!(p.state().is_none());
}

#[tokio::test]
async fn studio_network_pages_removed_channel_stops_completion_and_persistence() {
    use automerge::{transaction::Transactable, ROOT};
    for after_completion in [false, true] {
        let mut p = proven_pair().await;
        p.alice.open_channel_index().await.unwrap();
        p.bob.open_channel_index().await.unwrap();
        let channel = p.alice.create_channel("page-test").await.unwrap().id;
        p.bob.create_channel("page-test").await.unwrap();
        let target = StudioTarget::Flipnote {
            channel: channel.to_be_bytes(),
            object: [7; 16],
        };
        let doc = target.document(&p.alice.group_id()).unwrap();
        let id = epoch_zero_id(doc.doc_type, &doc.logical_key);
        p.alice
            .studio_transaction(
                &mut p.a_store,
                SERVER,
                StudioRequest::Apply {
                    target,
                    epoch_id: id,
                    nonce: [1; 16],
                    body: title(1, "remote").body,
                },
            )
            .unwrap();
        let a_watch = p
            .alice
            .watch_studio_epoch(&p.a_store, SERVER, target)
            .unwrap();
        let b_watch = p
            .bob
            .watch_studio_epoch(&p.b_store, SERVER, target)
            .unwrap();
        let mut provider = p.alice.studio_page_provider(&p.a_store, SERVER);
        let mut b = budget(&mut p.bob, &mut p.b_store);
        let mut pass = p
            .bob
            .begin_studio_receive(&mut p.b_store, &b_watch, p.alice.local_peer(), &mut b)
            .unwrap();
        let attempt = p
            .bob
            .prepare_studio_receive_step(&mut pass)
            .unwrap()
            .unwrap();
        let (completed, ()) = tokio::join!(attempt.fetch(), async {
            while !p.alice.sync.studio_has_page_request(&a_watch.inner) {
                p.alice.sync_once().await.unwrap();
            }
            p.alice
                .serve_studio_request_step(&mut p.a_store, &mut provider, &a_watch)
                .unwrap();
        });
        let completed = if after_completion {
            p.bob
                .complete_studio_receive_step(&mut pass, completed)
                .unwrap();
            None
        } else {
            Some(completed)
        };
        p.bob
            .sync
            .post(
                catcoms_wire::DocType::ChannelIndex,
                crate::CHANNEL_INDEX_DOC,
                |d| d.delete(ROOT, format!("{channel:032x}")),
            )
            .await
            .unwrap();
        if let Some(completed) = completed {
            assert!(p
                .bob
                .complete_studio_receive_step(&mut pass, completed)
                .is_err());
        } else {
            assert!(p
                .bob
                .persist_studio_receive_step(&mut p.b_store, &mut pass, &mut b)
                .is_err());
        }
        assert_eq!(pass.state(), StudioReceiveState::Stopped);
        assert_eq!(pass.progress().saved_pages, 0);
        assert!(p
            .bob
            .sync
            .with_registry_context(|g, d, _, _| p.b_store.load_studio_epoch(SERVER, g, target, d))
            .unwrap()
            .is_none());
    }
}
