use super::*;
use crate::store::{epoch_budget::StorageScope, EpochIntentBudget};
use catcoms_mls::MlsDevice;
use catcoms_replication::{
    registry::{PointerKey, RegistryOp},
    InheritedCheckpoint, LogicalDocument, Receipt,
};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

mod head;
mod page_receive;

const SERVER: u64 = 91;
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(835)
}
fn inventory(store: &mut ServerStore, group: &[u8]) -> (EpochStorageBudget, EpochIntentBudget) {
    let mut scan = store.scan_epoch_storage_with_registry().unwrap();
    while !scan.step().unwrap().complete {}
    let inv = scan.finish().unwrap();
    (
        EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, group).unwrap(),
            inv.records_for_server(SERVER, group).unwrap(),
        )
        .unwrap(),
        EpochIntentBudget::from_inventory(&inv).unwrap(),
    )
}

struct Pair {
    hub: Arc<Hub>,
    _alice_root: tempfile::TempDir,
    bob_root: tempfile::TempDir,
    alice: Server<MemNetwork, ChaCha20Rng>,
    bob: Server<MemNetwork, ChaCha20Rng>,
    alice_store: ServerStore,
    bob_store: ServerStore,
    alice_budget: EpochStorageBudget,
    alice_intents: EpochIntentBudget,
    bob_budget: EpochStorageBudget,
    watch: ServerRegistryWatch,
    key: PointerKey,
    document: LogicalDocument,
    id: u128,
    clock: ManualClock,
}
impl Pair {
    async fn new() -> Self {
        let hub = Hub::new();
        let alice_net = hub.join(PeerId::from_u64(1));
        let bob_net = hub.join(PeerId::from_u64(2));
        let clock = ManualClock::new(1000);
        let mut alice = Server::found(
            alice_net,
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            "alice",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        let invite = alice.mint_invite([1; 16], u64::MAX, vec![]).unwrap();
        // The actual product admission handshake, not a shared same-device decrypt fixture.
        let (bob, tick) = tokio::join!(
            Server::join(
                bob_net,
                MlsDevice::generate().unwrap(),
                rng(),
                Box::new(clock.clone()),
                "bob",
                alice.local_peer(),
                &invite
            ),
            alice.sync_once()
        );
        tick.unwrap();
        let mut bob = bob.unwrap();
        assert_eq!(alice.member_count(), 2);
        assert_eq!(bob.member_count(), 2);
        let alice_root = tempfile::tempdir().unwrap();
        let bob_root = tempfile::tempdir().unwrap();
        let mut alice_store =
            ServerStore::open(alice_root.path(), b"receive-test", &mut rng()).unwrap();
        let mut bob_store =
            ServerStore::open(bob_root.path(), b"receive-test", &mut rng()).unwrap();
        let (alice_budget, alice_intents) = inventory(&mut alice_store, &alice.group_id());
        let (bob_budget, _) = inventory(&mut bob_store, &bob.group_id());
        let key = PointerKey::new(DocType::StudioObject, b"private-received-cat".to_vec()).unwrap();
        let document = registry_document(&alice.group_id(), key.bucket()).unwrap();
        let id = epoch_zero_id(DocType::DocRegistry, &document.logical_key);
        let watch = bob
            .watch_registry_epoch(&bob_store, SERVER, key.bucket())
            .unwrap();
        bob.flush_registry_subscriptions().await.unwrap();
        let mut pair = Self {
            hub,
            clock,
            _alice_root: alice_root,
            bob_root,
            alice,
            bob,
            alice_store,
            bob_store,
            alice_budget,
            alice_intents,
            bob_budget,
            watch,
            key,
            document,
            id,
        };
        pair.edit(1);
        pair
    }
    fn edit(&mut self, nonce: u8) {
        let op = RegistryOp::Put {
            key: self.key.clone(),
            epoch: nonce as u64,
        }
        .domain_op(&self.alice.group_id(), [nonce; 16])
        .unwrap();
        self.alice
            .sync
            .with_registry_context(|g, d, _, r| {
                self.alice_store.edit_registry_epoch(
                    SERVER,
                    g,
                    self.key.bucket(),
                    self.id,
                    d,
                    op,
                    r,
                    &mut self.alice_budget,
                    &mut self.alice_intents,
                )
            })
            .unwrap();
    }
    async fn send(&mut self) {
        // Exercise the previous slice's real sender plus the new run_once demultiplexer.
        let mut replay = self
            .alice
            .begin_registry_replay(
                &self.alice_store,
                SERVER,
                self.key.bucket(),
                self.id,
                &mut self.alice_budget,
                &mut self.alice_intents,
            )
            .unwrap();
        self.alice
            .send_registry_replay_step(
                &mut self.alice_store,
                &mut replay,
                &mut self.alice_budget,
                &mut self.alice_intents,
            )
            .await
            .unwrap();
        assert_eq!(replay.progress().submitted, 1);
        self.bob.sync_once().await.unwrap();
    }
    fn receive(&mut self) -> Result<Option<RegistryReceived>, AppError> {
        self.bob
            .receive_registry_step(&mut self.bob_store, &self.watch, &mut self.bob_budget)
    }
    fn state(&mut self) -> Option<EpochRegistryState> {
        self.bob
            .sync
            .with_registry_context(|g, d, _, _| {
                self.bob_store
                    .load_registry_epoch(SERVER, g, self.key.bucket(), d)
            })
            .unwrap()
    }
}

#[tokio::test]
async fn registry_receive_two_actual_members_persist_duplicate_and_restart() {
    let mut pair = Pair::new().await;
    assert_eq!(format!("{:?}", pair.watch), "ServerRegistryWatch { .. }");
    pair.send().await;
    assert!(pair.state().is_none(), "queued/authenticated is not saved");
    let result = pair.receive().unwrap().unwrap();
    assert_eq!(result.admission, Admission::Accepted);
    assert_eq!(result.state.projection().unwrap().pointers[&pair.key], 1);
    assert!(!format!("{result:?}").contains("private-received-cat"));
    assert!(pair.receive().unwrap().is_none());
    pair.send().await;
    let result = pair.receive().unwrap().unwrap();
    assert_eq!(result.admission, Admission::Duplicate);
    assert_eq!(result.state.op_count(), 1);
    assert_eq!(
        pair.alice_store
            .load_epoch_intents(SERVER, &pair.document)
            .unwrap()
            .pending()
            .len(),
        1
    );
    drop(pair.bob_store);
    pair.bob_store = ServerStore::open(pair.bob_root.path(), b"receive-test", &mut rng()).unwrap();
    assert_eq!(
        pair.state().unwrap().projection().unwrap().pointers[&pair.key],
        1
    );
    assert!(
        pair.receive().is_err(),
        "old physical-mount watch is not a write permit"
    );
    pair.bob.unwatch_registry_epoch(&pair.watch).unwrap();
    assert!(pair.bob.unwatch_registry_epoch(&pair.watch).is_err());
    pair.watch = pair
        .bob
        .watch_registry_epoch(&pair.bob_store, SERVER, pair.key.bucket())
        .unwrap();
    pair.bob_budget = inventory(&mut pair.bob_store, &pair.bob.group_id()).0;
    pair.send().await;
    assert_eq!(
        pair.receive().unwrap().unwrap().admission,
        Admission::Duplicate
    );
}

#[tokio::test]
async fn registry_page_network_serves_saved_history_to_an_actual_joined_member() {
    use catcoms_replication::registry_epoch::catchup::RegistryPageOutcome;
    use catcoms_sync::registry_catchup::RegistryPageQuery;
    let mut pair = Pair::new().await;
    for nonce in 2..=33 {
        pair.edit(nonce);
    }
    let watch = pair
        .alice
        .watch_registry_epoch(&pair.alice_store, SERVER, pair.key.bucket())
        .unwrap();
    let mut provider = pair
        .alice
        .begin_registry_page_provider(&pair.alice_store, SERVER, pair.key.bucket())
        .unwrap();
    pair.alice.flush_registry_subscriptions().await.unwrap();
    // A normal transport-bound signed catch-up exchange proves the endpoint first. The page
    // query must not disclose registry metadata to an arbitrary invite/discovery candidate.
    let alice_peer = pair.alice.local_peer();
    let (proof, tick) = tokio::join!(
        pair.bob
            .sync
            .request_catchup(alice_peer, DocType::Wiki, 123),
        pair.alice.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let mut cursor = None;
    let mut received = 0;
    for expected in [32, 1] {
        let query = RegistryPageQuery {
            bucket: pair.key.bucket(),
            doc_id: pair.id,
            heads: &[],
            seed: None,
            cursor: cursor.as_ref().map(
                |c: &catcoms_replication::registry_epoch::catchup::RegistryPageCursor| c.as_bytes(),
            ),
        };
        let (reply, ()) = tokio::join!(pair.bob.request_registry_page(alice_peer, query), async {
            pair.alice.sync_once().await.unwrap();
            assert!(pair
                .alice
                .serve_registry_request_step(&pair.alice_store, &mut provider, &watch)
                .unwrap()
                .is_some());
        });
        let RegistryPageOutcome::Page(page) = reply.unwrap().unwrap() else {
            panic!("expected saved page")
        };
        assert_eq!(page.operations.len(), expected);
        // A successful network exchange alone has not installed anything on the receiver.
        assert_eq!(pair.state().map_or(0, |state| state.op_count()), received);
        for op in &page.operations {
            let (admission, _) = pair
                .bob
                .sync
                .with_registry_context(|group, device, _, rng| {
                    pair.bob_store.ingest_registry_epoch(
                        SERVER,
                        group,
                        pair.key.bucket(),
                        device,
                        op,
                        rng,
                        &mut pair.bob_budget,
                    )
                })
                .unwrap();
            assert_eq!(admission, Admission::Accepted);
        }
        cursor = page.next;
        received += expected;
    }
    assert!(cursor.is_none());
    assert_eq!(pair.state().unwrap().op_count(), 33);
    drop(pair.bob_store);
    pair.bob_store = ServerStore::open(pair.bob_root.path(), b"receive-test", &mut rng()).unwrap();
    assert_eq!(
        pair.state().unwrap().projection().unwrap().pointers[&pair.key],
        33
    );
    assert_eq!(
        pair.alice_store
            .load_epoch_intents(SERVER, &pair.document)
            .unwrap()
            .pending()
            .len(),
        33
    );
}

#[tokio::test]
async fn registry_receive_rewatch_server_replacement_and_failed_budget_never_claim_success() {
    let mut pair = Pair::new().await;
    pair.send().await;
    let old = std::mem::replace(
        &mut pair.watch,
        pair.bob
            .watch_registry_epoch(&pair.bob_store, SERVER, pair.key.bucket())
            .unwrap(),
    );
    assert!(pair
        .bob
        .receive_registry_step(&mut pair.bob_store, &old, &mut pair.bob_budget)
        .is_err());
    assert!(
        pair.bob.unwatch_registry_epoch(&old).is_err(),
        "old generation cannot revoke a new watch"
    );
    assert!(
        pair.receive().unwrap().is_none(),
        "rewatch discards old queued generation"
    );
    pair.send().await;
    pair.bob_budget.invalidate();
    assert!(pair.receive().is_err());
    assert!(pair.state().is_none());
    pair.bob_budget = inventory(&mut pair.bob_store, &pair.bob.group_id()).0;
    assert!(
        pair.receive().unwrap().is_none(),
        "failed volatile input earns no ack or hidden retry"
    );
    pair.send().await;
    assert_eq!(
        pair.receive().unwrap().unwrap().admission,
        Admission::Accepted
    );
    let snapshot = pair.bob.snapshot().unwrap();
    pair.bob = Server::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(3)),
        rng(),
        Box::new(ManualClock::new(1000)),
        "bob",
    )
    .unwrap();
    assert!(
        pair.receive().is_err(),
        "same identity restore rejects old watch"
    );
}

#[tokio::test]
async fn registry_receive_closing_is_quarantine_not_accepted_or_replayed_as_local() {
    let mut pair = Pair::new().await;
    pair.send().await;
    pair.receive().unwrap().unwrap();
    let checkpoint = pair
        .state()
        .unwrap()
        .projection()
        .unwrap()
        .checkpoint([8; 32])
        .unwrap();
    let receipt = pair
        .alice
        .sync
        .with_registry_context(|_, d, _, _| {
            Receipt::sign(
                pair.document.clone(),
                0,
                [8; 32],
                checkpoint.change_hash(),
                0,
                InheritedCheckpoint::EpochZero,
                d,
            )
        })
        .unwrap();
    pair.bob
        .sync
        .with_registry_context(|g, d, _, r| {
            pair.bob_store.seal_registry_epoch(
                SERVER,
                g,
                pair.key.bucket(),
                d,
                receipt,
                0,
                r,
                &mut pair.bob_budget,
            )
        })
        .unwrap();
    // A second real authored edit must not be mistaken for the exact duplicate of edit 1.
    let op = RegistryOp::Put {
        key: pair.key.clone(),
        epoch: 2,
    }
    .domain_op(&pair.alice.group_id(), [2; 16])
    .unwrap();
    let (packet, _) = pair
        .alice
        .sync
        .with_registry_context(|g, d, _, r| {
            pair.alice_store.edit_registry_epoch(
                SERVER,
                g,
                pair.key.bucket(),
                pair.id,
                d,
                op,
                r,
                &mut pair.alice_budget,
                &mut pair.alice_intents,
            )
        })
        .unwrap();
    pair.alice
        .sync
        .publish_local_registry_once(pair.id, packet)
        .await
        .unwrap();
    pair.bob.sync_once().await.unwrap();
    let result = pair.receive().unwrap().unwrap();
    assert_ne!(result.admission, Admission::Accepted);
    assert_eq!(result.state.op_count(), 1);
    assert_eq!(result.state.quarantined_len(), 1);
    assert_eq!(
        pair.bob_store
            .load_epoch_intents(SERVER, &pair.document)
            .unwrap()
            .pending()
            .len(),
        0,
        "a remote edit is never an intent authored by this receiver"
    );
}
