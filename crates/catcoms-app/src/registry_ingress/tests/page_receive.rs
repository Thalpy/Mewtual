use super::*;
use crate::registry_catchup::{
    RegistryReceiveState as State, ServerRegistryPageProvider, ServerRegistryReceive,
};

async fn prepare(pair: &mut Pair) -> (ServerRegistryWatch, ServerRegistryPageProvider) {
    let watch = pair
        .alice
        .watch_registry_epoch(&pair.alice_store, SERVER, pair.key.bucket())
        .unwrap();
    pair.alice.flush_registry_subscriptions().await.unwrap();
    let provider = pair
        .alice
        .begin_registry_page_provider(&pair.alice_store, SERVER, pair.key.bucket())
        .unwrap();
    let peer = pair.alice.local_peer();
    let (proof, tick) = tokio::join!(
        pair.bob.sync.request_catchup(peer, DocType::Wiki, 124),
        pair.alice.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    (watch, provider)
}
fn begin(pair: &mut Pair) -> ServerRegistryReceive {
    pair.bob
        .begin_registry_receive(
            &mut pair.bob_store,
            &pair.watch,
            pair.alice.local_peer(),
            &mut pair.bob_budget,
        )
        .unwrap()
}
async fn fetch(
    pair: &mut Pair,
    pass: &mut ServerRegistryReceive,
    watch: &ServerRegistryWatch,
    provider: &mut ServerRegistryPageProvider,
) {
    assert_eq!(
        fetch_state(pair, pass, watch, provider).await,
        State::PageReady
    );
}
async fn fetch_state(
    pair: &mut Pair,
    pass: &mut ServerRegistryReceive,
    watch: &ServerRegistryWatch,
    provider: &mut ServerRegistryPageProvider,
) -> State {
    let (result, ()) = tokio::join!(pair.bob.fetch_registry_receive_step(pass), async {
        pair.alice.sync_once().await.unwrap();
        assert!(pair
            .alice
            .serve_registry_request_step(&pair.alice_store, provider, watch)
            .unwrap()
            .is_some());
    });
    result.unwrap()
}

#[tokio::test]
async fn registry_receiver_independent_member_edits_converge_after_empty_frontier_fallback() {
    let mut pair = Pair::new().await;
    let (watch, mut provider) = prepare(&mut pair).await;
    let key = (0..10_000)
        .map(|n| {
            PointerKey::new(DocType::StudioObject, format!("bob-cat-{n}").into_bytes()).unwrap()
        })
        .find(|key| key.bucket() == pair.key.bucket())
        .unwrap();
    let op = RegistryOp::Put {
        key: key.clone(),
        epoch: 42,
    }
    .domain_op(&pair.bob.group_id(), [200; 16])
    .unwrap();
    let (_, mut bob_intents) = inventory(&mut pair.bob_store, &pair.bob.group_id());
    pair.bob
        .sync
        .with_registry_context(|group, device, _, rng| {
            pair.bob_store.edit_registry_epoch(
                SERVER,
                group,
                key.bucket(),
                pair.id,
                device,
                op,
                rng,
                &mut pair.bob_budget,
                &mut bob_intents,
            )
        })
        .unwrap();
    let mut pass = begin(&mut pair);
    // Alice cannot recognize Bob's independent head. One bounded empty-frontier fallback
    // admits Alice's branch without erasing Bob's already durable edit or spinning forever.
    assert_eq!(
        fetch_state(&mut pair, &mut pass, &watch, &mut provider).await,
        State::Ready
    );
    assert_eq!(pass.progress().received_pages, 0);
    assert_eq!(pass.progress().attempts, 1);
    assert_eq!(pair.state().unwrap().op_count(), 1);
    pair.clock.advance_ms(1000);
    fetch(&mut pair, &mut pass, &watch, &mut provider).await;
    assert_eq!(
        persist(&mut pair, &mut pass).unwrap(),
        State::PrefixComplete
    );
    assert_eq!(pass.progress().attempts, 2);
    let bob_projection = pair.state().unwrap().projection().unwrap();
    assert_eq!(bob_projection.pointers[&pair.key], 1);
    assert_eq!(bob_projection.pointers[&key], 42);

    // Return Bob's branch through the real durable-intent sender and watch drain. Both
    // actual joined identities now hold the union, not just a fabricated shared-key fixture.
    let mut replay = pair
        .bob
        .begin_registry_replay(
            &pair.bob_store,
            SERVER,
            key.bucket(),
            pair.id,
            &mut pair.bob_budget,
            &mut bob_intents,
        )
        .unwrap();
    pair.bob
        .send_registry_replay_step(
            &mut pair.bob_store,
            &mut replay,
            &mut pair.bob_budget,
            &mut bob_intents,
        )
        .await
        .unwrap();
    pair.alice.sync_once().await.unwrap();
    let received = pair
        .alice
        .receive_registry_step(&mut pair.alice_store, &watch, &mut pair.alice_budget)
        .unwrap()
        .unwrap();
    assert_eq!(received.state.projection().unwrap(), bob_projection);
}
fn persist(pair: &mut Pair, pass: &mut ServerRegistryReceive) -> Result<State, AppError> {
    pair.bob
        .persist_registry_receive_step(&mut pair.bob_store, pass, &mut pair.bob_budget)
}

#[tokio::test]
async fn registry_receiver_saves_before_continuing_and_duplicate_pages_still_progress() {
    let mut pair = Pair::new().await;
    for n in 2..=33 {
        pair.edit(n);
    }
    let (watch, mut provider) = prepare(&mut pair).await;
    let mut first = begin(&mut pair);
    let mut duplicate = begin(&mut pair); // same initial frontier, before the first save
    fetch(&mut pair, &mut first, &watch, &mut provider).await;
    assert!(pair.state().is_none());
    assert_eq!(first.progress().saved_pages, 0);
    assert_eq!(
        pair.bob
            .fetch_registry_receive_step(&mut first)
            .await
            .unwrap(),
        State::PageReady
    );
    assert_eq!(
        first.progress().attempts,
        1,
        "no continuation before saving"
    );
    assert_eq!(persist(&mut pair, &mut first).unwrap(), State::Ready);
    assert_eq!(first.progress().accepted, 32);
    pair.clock.advance_ms(1000);
    fetch(&mut pair, &mut duplicate, &watch, &mut provider).await;
    assert_eq!(persist(&mut pair, &mut duplicate).unwrap(), State::Ready);
    assert_eq!(duplicate.progress().duplicates, 32);
    assert_eq!(duplicate.progress().accepted, 0);
    pair.clock.advance_ms(1000);
    fetch(&mut pair, &mut first, &watch, &mut provider).await;
    pair.bob_budget.invalidate();
    assert!(persist(&mut pair, &mut first).is_err());
    assert_eq!(first.state(), State::Paused);
    assert_eq!(first.progress().saved_pages, 1);
    assert_eq!(pair.state().unwrap().op_count(), 32);
    first.retry();
    assert_eq!(first.state(), State::PageReady);
    pair.bob_budget = inventory(&mut pair.bob_store, &pair.bob.group_id()).0;
    pair.clock.advance_ms(1000);
    assert_eq!(
        persist(&mut pair, &mut first).unwrap(),
        State::PrefixComplete
    );
    assert_eq!(first.progress().accepted, 33);
    assert_eq!(
        first.progress().attempts,
        2,
        "save retry must not fetch again"
    );
    assert_eq!(pair.state().unwrap().op_count(), 33);
    drop(first);
    drop(duplicate);
    let mut fresh = begin(&mut pair);
    fetch(&mut pair, &mut fresh, &watch, &mut provider).await;
    assert_eq!(
        fresh.progress().received_operations,
        0,
        "fresh pass derives saved heads"
    );
    assert_eq!(
        persist(&mut pair, &mut fresh).unwrap(),
        State::PrefixComplete
    );
    assert_eq!(
        pair.alice_store
            .load_epoch_intents(SERVER, &pair.document)
            .unwrap()
            .pending()
            .len(),
        33
    );
    drop(pair.bob_store);
    pair.bob_store = ServerStore::open(pair.bob_root.path(), b"receive-test", &mut rng()).unwrap();
    assert_eq!(
        pair.state().unwrap().projection().unwrap().pointers[&pair.key],
        33
    );
    assert!(
        persist(&mut pair, &mut fresh).is_err(),
        "old mount is no write permit"
    );
}

#[tokio::test]
async fn registry_receiver_cancelled_fetch_keeps_retry_pacing_and_fixed_lifetime() {
    use std::future::Future;
    let mut pair = Pair::new().await;
    let (watch, mut provider) = prepare(&mut pair).await;
    let mut pass = begin(&mut pair);
    let mut request = Box::pin(pair.bob.fetch_registry_receive_step(&mut pass));
    assert!(request
        .as_mut()
        .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
        .is_pending());
    drop(request);
    assert_eq!(pass.state(), State::Paused);
    pass.retry();
    assert_eq!(
        pair.bob
            .fetch_registry_receive_step(&mut pass)
            .await
            .unwrap(),
        State::Ready
    );
    assert_eq!(pass.progress().attempts, 1);
    // Handle the abandoned request. A cancelled caller does not promise zero provider work.
    pair.alice.sync_once().await.unwrap();
    pair.alice
        .serve_registry_request_step(&pair.alice_store, &mut provider, &watch)
        .unwrap();
    pair.clock.advance_ms(1000);
    fetch(&mut pair, &mut pass, &watch, &mut provider).await;
    assert_eq!(
        persist(&mut pair, &mut pass).unwrap(),
        State::PrefixComplete
    );
    assert_eq!(pass.progress().attempts, 2);
    let mut expired = begin(&mut pair);
    pair.clock.advance_ms(600_000);
    assert_eq!(
        pair.bob
            .fetch_registry_receive_step(&mut expired)
            .await
            .unwrap(),
        State::RestartRequired
    );
    expired.retry();
    assert_eq!(expired.state(), State::RestartRequired);
    assert_eq!(expired.progress().attempts, 0);
}

#[tokio::test]
async fn registry_receiver_pass_capacity_and_watch_replacement_revoke_before_io() {
    let mut pair = Pair::new().await;
    let (_watch, _provider) = prepare(&mut pair).await;
    let mut passes: Vec<_> = (0..4).map(|_| begin(&mut pair)).collect();
    assert!(pair
        .bob
        .begin_registry_receive(
            &mut pair.bob_store,
            &pair.watch,
            pair.alice.local_peer(),
            &mut pair.bob_budget
        )
        .is_err());
    pair.watch = pair
        .bob
        .watch_registry_epoch(&pair.bob_store, SERVER, pair.key.bucket())
        .unwrap();
    assert!(
        pair.bob
            .begin_registry_receive(
                &mut pair.bob_store,
                &pair.watch,
                pair.alice.local_peer(),
                &mut pair.bob_budget
            )
            .is_err(),
        "rewatch cannot refund still-retained pages"
    );
    for pass in &mut passes {
        assert!(pair.bob.fetch_registry_receive_step(pass).await.is_err());
        assert_eq!(pass.state(), State::Stopped);
        assert_eq!(pass.progress().attempts, 0);
    }
    passes.pop();
    let pass = begin(&mut pair);
    assert_eq!(pass.state(), State::Ready);
    assert!(
        pair.state().is_none(),
        "pass construction never creates an empty file"
    );
}

#[tokio::test]
async fn registry_receiver_empty_terminal_page_cannot_complete_after_a_receipt_seals_it() {
    let mut pair = Pair::new().await;
    let (watch, mut provider) = prepare(&mut pair).await;
    let mut initial = begin(&mut pair);
    fetch(&mut pair, &mut initial, &watch, &mut provider).await;
    assert_eq!(
        persist(&mut pair, &mut initial).unwrap(),
        State::PrefixComplete
    );
    pair.clock.advance_ms(1000);
    let mut empty = begin(&mut pair);
    fetch(&mut pair, &mut empty, &watch, &mut provider).await;
    assert_eq!(empty.progress().received_operations, 0);
    let seed = pair
        .state()
        .unwrap()
        .projection()
        .unwrap()
        .checkpoint([7; 32])
        .unwrap();
    let receipt = pair
        .alice
        .sync
        .with_registry_context(|_, device, _, _| {
            Receipt::sign(
                pair.document.clone(),
                0,
                [7; 32],
                seed.change_hash(),
                0,
                InheritedCheckpoint::EpochZero,
                device,
            )
        })
        .unwrap();
    pair.bob
        .sync
        .with_registry_context(|group, device, _, rng| {
            pair.bob_store.seal_registry_epoch(
                SERVER,
                group,
                pair.key.bucket(),
                device,
                receipt,
                0,
                rng,
                &mut pair.bob_budget,
            )
        })
        .unwrap();
    assert!(persist(&mut pair, &mut empty).is_err());
    assert_eq!(empty.state(), State::Paused);
    assert_eq!(empty.progress().saved_pages, 0);
    assert_eq!(
        pair.state().unwrap().phase(),
        catcoms_replication::EpochPhase::Closing
    );
}

#[tokio::test]
async fn registry_receiver_mls_advance_discards_pending_page_without_saving_or_retry_loop() {
    let mut pair = Pair::new().await;
    // Join establishes membership but callers explicitly opt into ongoing control traffic.
    pair.bob.subscribe_control().await.unwrap();
    let (watch, mut provider) = prepare(&mut pair).await;
    let mut pass = begin(&mut pair);
    fetch(&mut pair, &mut pass, &watch, &mut provider).await;
    let old_epoch = pair
        .bob
        .sync
        .with_registry_context(|group, _, _, _| group.epoch());
    let invite = pair.alice.mint_invite([99; 16], u64::MAX, vec![]).unwrap();
    let (carol, tick) = tokio::join!(
        Server::join(
            pair.hub.join(PeerId::from_u64(3)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(pair.clock.clone()),
            "carol",
            pair.alice.local_peer(),
            &invite,
        ),
        pair.alice.sync_once()
    );
    tick.unwrap();
    let _carol = carol.unwrap();
    // Deliver the actual signed membership commit, not a fabricated epoch counter.
    pair.bob.sync_once().await.unwrap();
    assert_eq!(
        pair.bob
            .sync
            .with_registry_context(|group, _, _, _| group.epoch()),
        old_epoch + 1
    );
    assert!(persist(&mut pair, &mut pass).is_err());
    assert_eq!(pass.state(), State::RestartRequired);
    assert_eq!(pass.progress().saved_pages, 0);
    assert_eq!(pass.progress().persist_attempts, 1);
    assert!(pair.state().is_none());
    pass.retry();
    assert_eq!(pass.state(), State::RestartRequired);
    assert_eq!(
        persist(&mut pair, &mut pass).unwrap(),
        State::RestartRequired
    );
    assert_eq!(pass.progress().persist_attempts, 1);
}
