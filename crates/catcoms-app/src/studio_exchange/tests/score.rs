//! Real typed Score actor/save/exchange regressions. No UI hints or parser fallback select
//! this family: every caller names an explicit target, checked again against its saved wrapper.
use super::*;
use catcoms_replication::studio::{ScoreCell, ScoreHeader, ScoreOp};
use tokio::sync::Mutex;

fn score() -> StudioTarget {
    StudioTarget::Score {
        channel: channel(),
        object: [8; 16],
    }
}

#[tokio::test]
async fn studio_score_actor_create_exchange_clear_exact_retry_and_reopen_keep_family() {
    let Pair {
        _a_root,
        b_root,
        alice,
        mut bob,
        a_store,
        mut b_store,
        wire,
        ..
    } = Pair::new().await;
    let index = StudioTarget::Index { channel: channel() };
    let watch = bob.watch_studio_epoch(&b_store, SERVER, score()).unwrap();
    let index_watch = bob.watch_studio_epoch(&b_store, SERVER, index).unwrap();
    bob.flush_studio_subscriptions().await.unwrap();
    let group = alice.group_id();
    let store = Arc::new(Mutex::new(Some(a_store)));
    let (actor, mut events, task) = crate::spawn(alice);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let create = || StudioRequest::CreateScore {
        channel: channel(),
        object: [8; 16],
        nonce: [88; 16],
        title: "score cat".into(),
        ts: 1000,
    };
    let created = actor_save::save(&actor, &store, create())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(created.projection, StudioProjection::Score(_)));
    assert_eq!(wire.attempts.load(Ordering::SeqCst), 2);
    let mut b = budget(&mut bob, &mut b_store);
    for selected in [&watch, &index_watch] {
        bob.sync_once().await.unwrap();
        assert_eq!(
            bob.receive_studio_step(&mut b_store, selected, &mut b)
                .unwrap()
                .unwrap()
                .admission,
            Admission::Accepted
        );
    }
    let kind = bob.sync.with_registry_context(|g, d, _, _| {
        let state = b_store
            .load_studio_epoch(SERVER, g, index, d)
            .unwrap()
            .unwrap();
        let StudioProjection::Index(view) = state.projection().unwrap() else {
            panic!()
        };
        view.objects[&[8; 16]].creations[0].value.kind
    });
    assert!(matches!(kind, StudioKind::Score));
    let set = ScoreOp::SetCell {
        step: 0,
        row: 24,
        value: ScoreCell::Drum { pad: 9 },
    }
    .encode()
    .unwrap();
    let request = || StudioRequest::Apply {
        target: score(),
        epoch_id: created.epoch_id,
        nonce: [89; 16],
        body: set.clone(),
    };
    actor_save::save(&actor, &store, request())
        .await
        .unwrap()
        .unwrap();
    bob.sync_once().await.unwrap();
    assert_eq!(
        bob.receive_studio_step(&mut b_store, &watch, &mut b)
            .unwrap()
            .unwrap()
            .admission,
        Admission::Accepted
    );
    let clear = StudioRequest::Apply {
        target: score(),
        epoch_id: created.epoch_id,
        nonce: [90; 16],
        body: ScoreOp::ClearCell { step: 0, row: 24 }.encode().unwrap(),
    };
    let cleared = actor_save::save(&actor, &store, clear)
        .await
        .unwrap()
        .unwrap();
    bob.sync_once().await.unwrap();
    assert_eq!(
        bob.receive_studio_step(&mut b_store, &watch, &mut b)
            .unwrap()
            .unwrap()
            .admission,
        Admission::Accepted
    );
    let retried = actor_save::save(&actor, &store, request())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retried.projection, cleared.projection,
        "exact retry does not resurrect a cleared cell"
    );
    bob.sync_once().await.unwrap();
    assert_eq!(
        bob.receive_studio_step(&mut b_store, &watch, &mut b)
            .unwrap()
            .unwrap()
            .admission,
        Admission::Duplicate
    );
    let before = bob.sync.with_registry_context(|g, d, _, _| {
        b_store
            .load_studio_epoch(SERVER, g, score(), d)
            .unwrap()
            .unwrap()
            .projection()
            .unwrap()
    });
    assert_eq!(before, cleared.projection);
    // An opposite-kind request has the same logical/physical id but cannot reinterpret this
    // cold authenticated wrapper as art, nor create another interpretation under that id.
    drop(b_store);
    b_store = open(b_root.path());
    bob.sync.with_registry_context(|g, d, _, _| {
        assert!(b_store
            .load_studio_epoch(
                SERVER,
                g,
                StudioTarget::Flipnote {
                    channel: channel(),
                    object: [8; 16]
                },
                d
            )
            .is_err());
        let restored = b_store
            .load_studio_epoch(SERVER, g, score(), d)
            .unwrap()
            .unwrap();
        assert_eq!(restored.projection().unwrap(), before);
    });
    assert!(b_store.creative_pinned_cids().unwrap().is_empty());
    let wrong = StudioRequest::Create {
        channel: channel(),
        object: [8; 16],
        nonce: [91; 16],
        title: "wrong family".into(),
        ts: 1000,
    };
    assert!(actor_save::save(&actor, &store, wrong).await.is_err());
    let held = store.lock().await;
    assert_eq!(
        held.as_ref()
            .unwrap()
            .load_epoch_intents(SERVER, &score().document(&group).unwrap())
            .unwrap()
            .pending()
            .len(),
        3
    );
    drop(held);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn studio_score_cold_scope_selected_page_catchup_persists_and_reopens() {
    let mut p = pages::proven_pair().await;
    let logical = score().document(&p.alice.group_id()).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    for (nonce, op) in [
        (
            1,
            ScoreOp::SetHeader(ScoreHeader::Title("page score".into())),
        ),
        (
            2,
            ScoreOp::SetCell {
                step: 15,
                row: 0,
                value: ScoreCell::Drum { pad: 0 },
            },
        ),
    ] {
        p.alice
            .studio_transaction(
                &mut p.a_store,
                SERVER,
                StudioRequest::Apply {
                    target: score(),
                    epoch_id: id,
                    nonce: [nonce; 16],
                    body: op.encode().unwrap(),
                },
            )
            .unwrap();
    }
    let receiver = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, score())
        .unwrap();
    let provider_watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, score())
        .unwrap();
    let mut provider = p.alice.studio_page_provider(&p.a_store, SERVER);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let mut pages = p
        .bob
        .begin_studio_receive(&mut p.b_store, &receiver, p.alice.local_peer(), &mut b)
        .unwrap();
    for _ in 0..4 {
        if pages.state() == StudioReceiveState::PrefixComplete {
            break;
        }
        if let Some(attempt) = p.bob.prepare_studio_receive_step(&mut pages).unwrap() {
            let (completed, ()) = tokio::join!(attempt.fetch(), async {
                p.alice.sync_once().await.unwrap();
                p.alice
                    .serve_studio_request_step(&mut p.a_store, &mut provider, &provider_watch)
                    .unwrap()
                    .unwrap();
            });
            p.bob
                .complete_studio_receive_step(&mut pages, completed)
                .unwrap();
        }
        let mut b = budget(&mut p.bob, &mut p.b_store);
        p.bob
            .persist_studio_receive_step(&mut p.b_store, &mut pages, &mut b)
            .unwrap();
    }
    assert_eq!(pages.state(), StudioReceiveState::PrefixComplete);
    assert_eq!(pages.progress().accepted, 2);
    let expected = p.alice.sync.with_registry_context(|g, d, _, _| {
        p.a_store
            .load_studio_epoch(SERVER, g, score(), d)
            .unwrap()
            .unwrap()
            .projection()
            .unwrap()
    });
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    p.bob.sync.with_registry_context(|g, d, _, _| {
        assert_eq!(
            p.b_store
                .load_studio_epoch(SERVER, g, score(), d)
                .unwrap()
                .unwrap()
                .projection()
                .unwrap(),
            expected
        )
    });
}
