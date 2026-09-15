//! Shared integration fixture: authentic head/seed/tail service, real actor preview admission.
//! The provider deliberately omits a current-owner proof; no ready cache or delivery is injected.
use catcoms_app::{
    store::ServerStore,
    studio::{StudioRead, StudioRequest, StudioVaultLease},
    Server, ServerActor,
};
use catcoms_mls::MlsDevice;
use catcoms_replication::{
    registry_epoch::catchup::{RegistryOpPage, RegistryPageOutcome},
    studio::*,
    DomainOp, InheritedCheckpoint, Receipt,
};
use catcoms_rt::{Hub, ManualClock, PeerId};
use catcoms_sync::{
    checkpoint_exchange::CheckpointTarget, epoch_service::EpochServiceKind,
    receipt_head::ReceiptHeadSelection, ChannelSync,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::JoinHandle};

pub(super) const SERVER: u64 = 7;
pub(super) const OBJECT: [u8; 16] = [7; 16];
pub(super) const FRAME: [u8; 16] = [4; 16];
pub(super) const CID: [u8; 32] = [42; 32];
pub(super) const FRAME_BYTES: u64 = 237;
pub(super) const TITLE: &str = "signed preview tail";

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(815)
}

#[derive(Debug)]
pub(super) struct PreviewFixture {
    pub(super) store: Arc<Mutex<Option<ServerStore>>>,
    pub(super) actor: ServerActor,
    pub(super) clock: ManualClock,
    pub(super) target: StudioTarget,
    pub(super) expected: StudioProjection,
    pub(super) epoch_id: u128,
    pub(super) group: Vec<u8>,
    pub(super) device: catcoms_app::DeviceId,
    pub(super) author: String,
    root: tempfile::TempDir,
    actor_task: JoinHandle<()>,
    drain: JoinHandle<()>,
    provider: JoinHandle<()>,
}

impl PreviewFixture {
    pub(super) async fn new(art: bool) -> Self {
        tokio::time::timeout(Duration::from_secs(30), Self::build(art))
            .await
            .expect("preview fixture setup stalled")
    }

    async fn build(art: bool) -> Self {
        let hub = Hub::new();
        let clock = ManualClock::new(1000);
        let mut alice = Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            "preview provider",
        )
        .unwrap();
        alice.subscribe_control().await.unwrap();
        let invite = alice.mint_invite([1; 16], u64::MAX, vec![]).unwrap();
        let (bob, tick) = tokio::join!(
            Server::join(
                hub.join(PeerId::from_u64(2)),
                MlsDevice::generate().unwrap(),
                rng(),
                Box::new(clock.clone()),
                "native reader",
                alice.local_peer(),
                &invite
            ),
            alice.sync_once()
        );
        tick.unwrap();
        let mut bob = bob.unwrap();
        alice.open_channel_index().await.unwrap();
        tokio::select! {
            proof = bob.request_channel_index_catchup(alice.local_peer()) => { proof.unwrap(); },
            _ = async { loop { alice.sync_once().await.unwrap(); } } => unreachable!(),
        }
        let snapshot = alice.snapshot().unwrap();
        drop(alice);
        let mut provider = ChannelSync::restore(
            &snapshot,
            hub.join(PeerId::from_u64(1)),
            rng(),
            Box::new(clock.clone()),
        )
        .unwrap();
        let channel = catcoms_app::channel_id("general").to_be_bytes();
        let target = if art {
            StudioTarget::Flipnote {
                channel,
                object: OBJECT,
            }
        } else {
            StudioTarget::Index { channel }
        };
        let (receipt, seed, tail, expected, epoch_id, author) =
            provider.with_registry_context(|g, d, _, r| {
                let logical = target.document(&g.group_id()).unwrap();
                let mut source = StudioEpoch::new(g, target, d.device_id()).unwrap();
                let body = if art {
                    FlipnoteOp::InsertFrame {
                        frame: FRAME,
                        after: None,
                        cid: CID,
                        bytes: FRAME_BYTES,
                    }
                    .encode()
                    .unwrap()
                } else {
                    IndexOp::PutObject {
                        object: OBJECT,
                        kind: StudioKind::Flipnote,
                        title: "seed title".into(),
                        created_by: d.device_id(),
                        ts: 123,
                        expiry: StudioExpiry::Never,
                    }
                    .encode()
                    .unwrap()
                };
                source
                    .edit_or_reseal(
                        d,
                        g,
                        r,
                        &DomainOp {
                            doc_type: logical.doc_type,
                            logical_key: logical.logical_key.clone(),
                            body,
                            nonce: [3; 16],
                        },
                        123,
                    )
                    .unwrap();
                let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
                let receipt = Receipt::sign(
                    logical.clone(),
                    0,
                    [7; 32],
                    seed.change_hash(),
                    0,
                    InheritedCheckpoint::EpochZero,
                    d,
                )
                .unwrap();
                let mut source = StudioEpoch::from_checkpoint(
                    g,
                    target,
                    d.device_id(),
                    receipt.clone(),
                    0,
                    seed.bytes(),
                )
                .unwrap();
                let body = if art {
                    FlipnoteOp::SetHeader(FlipnoteHeader::Title(TITLE.into()))
                        .encode()
                        .unwrap()
                } else {
                    IndexOp::SetTitle {
                        object: OBJECT,
                        title: TITLE.into(),
                    }
                    .encode()
                    .unwrap()
                };
                let tail = source
                    .edit_or_reseal(
                        d,
                        g,
                        r,
                        &DomainOp {
                            doc_type: logical.doc_type,
                            logical_key: logical.logical_key,
                            body,
                            nonce: [9; 16],
                        },
                        124,
                    )
                    .unwrap();
                (
                    receipt,
                    seed,
                    tail,
                    source.projection().unwrap(),
                    source.doc_id(),
                    d.device_id().to_string(),
                )
            });
        provider.enable_epoch_service();
        assert_eq!(bob.sync().studio_page_peers(), vec![PeerId::from_u64(1)]);
        let provider = tokio::spawn(async move {
            loop {
                provider.run_once().await.unwrap();
                while let Some(interest) = provider.reserve_epoch_service_interest() {
                    let matches_target = interest.target() == CheckpointTarget::Studio(target);
                    match interest.kind() {
                        EpochServiceKind::Head => {
                            provider
                                .serve_epoch_head_interest(&interest, None, |_, _, _, _| {
                                    Ok::<_, ()>(ReceiptHeadSelection {
                                        receipt: matches_target.then(|| receipt.clone()),
                                        prove: false,
                                    })
                                })
                                .unwrap()
                                .unwrap()
                                .unwrap();
                        }
                        EpochServiceKind::Seed => {
                            provider
                                .serve_epoch_seed_interest(&interest, |_, _, _, _| {
                                    Ok::<_, ()>(matches_target.then(|| seed.bytes().to_vec()))
                                })
                                .unwrap()
                                .unwrap()
                                .unwrap();
                        }
                        EpochServiceKind::Page => {
                            provider
                                .serve_epoch_page_interest(&interest, |_, _, _, _| {
                                    Ok::<_, ()>(
                                        if matches_target && interest.doc_id() == Some(epoch_id) {
                                            RegistryPageOutcome::Page(RegistryOpPage {
                                                operations: vec![tail.clone()],
                                                next: None,
                                            })
                                        } else {
                                            RegistryPageOutcome::CheckpointRequired
                                        },
                                    )
                                })
                                .unwrap()
                                .unwrap()
                                .unwrap();
                        }
                    }
                }
            }
        });
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::open(root.path(), b"native-preview", &mut rng()).unwrap();
        let group = bob.group_id();
        let device = bob.device_id();
        bob.set_blob_store(store.blob_store(&hex::encode(&group)).unwrap());
        let store = Arc::new(Mutex::new(Some(store)));
        let (actor, mut events, actor_task) = catcoms_app::spawn(bob);
        let drain = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                assert!(!matches!(
                    event.event,
                    catcoms_app::AppEvent::StudioReceivePaused
                ));
            }
        });
        Self {
            store,
            actor,
            clock,
            target,
            expected,
            epoch_id,
            group,
            device,
            author,
            root,
            actor_task,
            drain,
            provider,
        }
    }

    pub(super) async fn read(&self) -> Option<StudioRead> {
        self.actor
            .studio_begin(StudioRequest::Read {
                target: self.target,
            })
            .await
            .unwrap()
            .execute_read(StudioVaultLease::new(
                self.store.clone().try_lock_owned().unwrap(),
                SERVER,
                (),
            ))
            .await
            .unwrap()
    }

    pub(super) async fn wait_ready(&self) {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                assert!(!self.provider.is_finished(), "preview provider stopped");
                if let Some(StudioRead::AwaitingTenureReceipt(preview)) = self.read().await {
                    assert!(preview.delivery().is_current());
                    preview
                        .inspect(|id, projection| {
                            assert_eq!(id, self.epoch_id);
                            assert_eq!(projection, &self.expected);
                        })
                        .unwrap();
                    break;
                }
                self.actor
                    .studio_receive_begin()
                    .await
                    .unwrap()
                    .execute(StudioVaultLease::new(
                        self.store.clone().try_lock_owned().unwrap(),
                        SERVER,
                        (),
                    ))
                    .await
                    .unwrap();
                // Allow real detached I/O/parsers to finish; this is bounded polling, not a
                // timing assertion. Manual time only drives normal scheduler retry deadlines.
                tokio::time::sleep(Duration::from_millis(10)).await;
                self.clock.advance_ms(1000);
            }
        })
        .await
        .expect("actor never admitted the authentic provisional seed and signed tail");
    }

    pub(super) async fn shutdown(self) {
        self.actor.shutdown().await;
        self.actor_task.await.unwrap();
        self.drain.await.unwrap();
        self.provider.abort();
        assert!(self.provider.await.unwrap_err().is_cancelled());
        drop(self.root);
    }
}
