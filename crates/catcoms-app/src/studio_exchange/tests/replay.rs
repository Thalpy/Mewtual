use super::*;
use crate::studio::StudioReceiver;
use catcoms_replication::{EpochPhase, InheritedCheckpoint, Receipt};

fn install_empty(p: &mut Pair) -> (Receipt, u128) {
    let (receipt, seed) = p.alice.sync.with_registry_context(|g, d, _, _| {
        let empty =
            catcoms_replication::studio::StudioEpoch::new(g, target(), d.device_id()).unwrap();
        let seed = empty.projection().unwrap().checkpoint([81; 32]).unwrap();
        let receipt = Receipt::sign(
            target().document(&g.group_id()).unwrap(),
            0,
            [81; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        (receipt, seed)
    });
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let epoch = p.bob.sync.with_registry_context(|g, d, _, rng| {
        let (_, state) = p
            .b_store
            .adopt_studio_checkpoint(
                SERVER,
                g,
                target(),
                d,
                &receipt,
                Some(seed.bytes()),
                0,
                &p.clock,
                rng,
                &mut b,
            )
            .unwrap();
        let epoch = state.doc_id();
        p.b_store.retain_studio_source(g, d, state);
        epoch
    });
    (receipt, epoch)
}
fn own_title(p: &mut Pair, epoch: u128, n: u8, text: &str) {
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: epoch,
                nonce: [n; 16],
                body: title(n, text).body,
            },
        )
        .unwrap();
}
fn count(p: &Pair) -> usize {
    p.b_store
        .load_epoch_intents(SERVER, &target().document(&p.bob.group_id()).unwrap())
        .unwrap()
        .pending()
        .len()
}
fn watch(p: &mut Pair) -> StudioReceiver {
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    receiver
}

#[tokio::test]
async fn studio_replay_nonowner_restarts_replays_selected_own_title_and_archives_only_old_evidence()
{
    let mut p = Pair::new().await;
    let doc = target().document(&p.bob.group_id()).unwrap();
    let initial = epoch_zero_id(doc.doc_type, &doc.logical_key);
    own_title(&mut p, initial, 91, "A");
    own_title(&mut p, initial, 92, "B");
    let (_, epoch) = install_empty(&mut p);
    assert_eq!(count(&p), 2);
    let mut receiver = watch(&mut p);
    let (_saved, updated) = receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    assert_eq!(updated, Some(target()));
    let StudioProjection::Flipnote(art) = p.state().unwrap().projection().unwrap() else {
        panic!()
    };
    assert_eq!(art.title.unwrap().selected.value, "B");
    assert_eq!(count(&p), 2);
    // Restart after replay Save but before publication: B is a current signed-log operation,
    // not a new overwrite. Only superseded A moves to durably flushed manual recovery.
    drop(receiver);
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    let mut receiver = watch(&mut p);
    p.clock.advance_ms(1000);
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    assert_eq!(count(&p), 1);
    assert_eq!(p.state().unwrap().op_count(), 1);
    assert!(
        p.b_store
            .load_epoch_recovery(SERVER, &doc)
            .unwrap()
            .retained()
            .len()
            > 0
    );
    assert_eq!(p.state().unwrap().doc_id(), epoch);
    assert!(!receiver.replay_state_for_test().0);
    // The next owner checkpoint includes B. A non-owner has the seed but not the closure:
    // its exact old envelope is preserved in adoption recovery, so it leaves pending under
    // the manual policy, NOT an invented seed-only receipt proof. Ordinary cycles do not leak.
    let projection = p.state().unwrap().projection().unwrap();
    let seed = projection.checkpoint([82; 32]).unwrap();
    let receipt = p.alice.sync.with_registry_context(|g, d, _, _| {
        Receipt::sign(
            target().document(&g.group_id()).unwrap(),
            1,
            [82; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap()
    });
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            p.b_store.adopt_studio_checkpoint(
                SERVER,
                g,
                target(),
                d,
                &receipt,
                Some(seed.bytes()),
                0,
                &p.clock,
                rng,
                &mut b,
            )
        })
        .unwrap();
    let mut receiver = watch(&mut p);
    p.clock.advance_ms(1000);
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    assert_eq!(count(&p), 0);
    assert_eq!(p.state().unwrap().epoch(), 2);
    assert_eq!(
        p.b_store
            .load_epoch_recovery(SERVER, &doc)
            .unwrap()
            .retained()
            .len(),
        2
    );
}

#[tokio::test]
async fn studio_replay_sealed_active_pass_clears_and_stays_bounded() {
    let mut p = Pair::new().await;
    let doc = target().document(&p.bob.group_id()).unwrap();
    let initial = epoch_zero_id(doc.doc_type, &doc.logical_key);
    own_title(&mut p, initial, 91, "A");
    own_title(&mut p, initial, 92, "B");
    let (_, epoch) = install_empty(&mut p);
    let mut receiver = watch(&mut p);
    receiver
        .replay_step_for_test(&mut p.bob, &mut p.b_store, SERVER)
        .unwrap();
    assert!(receiver.replay_state_for_test().0);
    let receipt = p.alice.sync.with_registry_context(|g, d, _, _| {
        Receipt::sign(
            target().document(&g.group_id()).unwrap(),
            1,
            [82; 32],
            [82; 32],
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap()
    });
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .sync
        .with_registry_context(|g, d, _, rng| {
            p.b_store
                .seal_studio_epoch(SERVER, g, target(), d, receipt, 0, rng, &mut b)
        })
        .unwrap();
    for _ in 0..100 {
        p.clock.advance_ms(1000);
        receiver
            .replay_step_for_test(&mut p.bob, &mut p.b_store, SERVER)
            .unwrap();
        assert_eq!(receiver.replay_state_for_test(), (false, 1));
    }
    assert_eq!(p.state().unwrap().phase(), EpochPhase::Closing);
    assert_eq!(p.state().unwrap().doc_id(), epoch);
    assert_eq!(count(&p), 2, "seal does not authorize ledger removal");
}
