//! Genuine accepted Closing draft; no ready result, stamp or delivery guard is injected.
use super::app;
use app::{store::ServerStore, studio::*, Server, ServerActor};
use catcoms_mls::MlsDevice;
use catcoms_replication::{studio::*, Admission, DomainOp, SealedOp, SignedOp};
use catcoms_rt::{Hub, ManualClock, PeerId};
use catcoms_sync::ChannelSync;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::JoinHandle};

pub(super) const SERVER: u64 = 7;
pub(super) const ELEMENT: [u8; 16] = [4; 16];
pub(super) const TITLE: &str = "retained local draft";
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(913)
}

pub(super) struct InspectionFixture {
    pub(super) store: Arc<Mutex<Option<ServerStore>>>,
    pub(super) actor: ServerActor,
    pub(super) clock: ManualClock,
    pub(super) target: StudioTarget,
    pub(super) expected: StudioProjection,
    pub(super) basis: [u8; 32],
    pub(super) group: Vec<u8>,
    pub(super) device: app::DeviceId,
    root: tempfile::TempDir,
    task: JoinHandle<()>,
    drain: JoinHandle<()>,
}
impl InspectionFixture {
    pub(super) async fn new(art: bool) -> Self {
        // Setup uses real large signed operations to reach the production rotation threshold.
        let hub = Hub::new();
        let clock = ManualClock::new(1000);
        let mut server = Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            "inspection",
        )
        .unwrap();
        let channel = app::channel_id("general").to_be_bytes();
        let target = if art {
            StudioTarget::Flipnote {
                channel,
                object: [7; 16],
            }
        } else {
            StudioTarget::Index { channel }
        };
        let root = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(root.path(), b"inspection", &mut rng()).unwrap();
        let mut context = ChannelSync::restore(
            &server.snapshot().unwrap(),
            hub.join(PeerId::from_u64(9)),
            rng(),
            Box::new(clock.clone()),
        )
        .unwrap();
        let (close, mut budget, title, group, device) =
            context.with_registry_context(|g, d, _, r| {
                let logical = target.document(&g.group_id()).unwrap();
                let mut scan = store.scan_epoch_storage_with_studio().unwrap();
                while !scan.step().unwrap().complete {}
                let inventory = scan.finish().unwrap();
                let mut budget = store.studio_storage_budget(SERVER, g, &inventory).unwrap();
                let mut source = StudioEpoch::new(g, target, d.device_id()).unwrap();
                let domain = |body, nonce| DomainOp {
                    body,
                    nonce: [nonce; 16],
                    doc_type: logical.doc_type,
                    logical_key: logical.logical_key.clone(),
                };
                let insert = domain(
                    if art {
                        FlipnoteOp::InsertFrame {
                            frame: ELEMENT,
                            after: None,
                            cid: [42; 32],
                            bytes: 237,
                        }
                        .encode()
                        .unwrap()
                    } else {
                        IndexOp::PutObject {
                            object: ELEMENT,
                            kind: StudioKind::Flipnote,
                            title: "shared seed".into(),
                            created_by: d.device_id(),
                            ts: 100,
                            expiry: StudioExpiry::Never,
                        }
                        .encode()
                        .unwrap()
                    },
                    1,
                );
                let packet = source.edit_or_reseal(d, g, r, &insert, 100).unwrap();
                assert_eq!(
                    store
                        .ingest_studio_epoch(SERVER, g, target, d, &packet, r, &mut budget)
                        .unwrap()
                        .0,
                    Admission::Accepted
                );
                let title = domain(
                    if art {
                        FlipnoteOp::SetHeader(FlipnoteHeader::Title(TITLE.into()))
                            .encode()
                            .unwrap()
                    } else {
                        IndexOp::SetTitle {
                            object: ELEMENT,
                            title: TITLE.into(),
                        }
                        .encode()
                        .unwrap()
                    },
                    2,
                );
                for n in 10..20 {
                    let mut op = title.clone();
                    op.nonce = [n; 16];
                    let mut copy =
                        StudioEpoch::restore(&source.snapshot().unwrap(), g, target, d.device_id())
                            .unwrap();
                    let packet = copy.edit_or_reseal(d, g, r, &op, 100).unwrap();
                    let signed = packet
                        .open(&g.channel_secret(d, packet.doc_type, packet.doc_id).unwrap())
                        .unwrap();
                    let mut change = automerge::Change::from_bytes(signed.delta)
                        .unwrap()
                        .decode();
                    change.message = Some("x".repeat(220_000));
                    let change = automerge::Change::from(change);
                    let signed = SignedOp::sign_domain(
                        d,
                        logical.doc_type,
                        source.doc_id(),
                        change.raw_bytes().to_vec(),
                        &op,
                    )
                    .unwrap();
                    let packet = SealedOp::seal(&signed, g, d, r).unwrap();
                    assert_eq!(source.ingest(&packet, g, d).unwrap(), Admission::Accepted);
                    assert_eq!(
                        store
                            .ingest_studio_epoch(SERVER, g, target, d, &packet, r, &mut budget)
                            .unwrap()
                            .0,
                        Admission::Accepted
                    );
                }
                let decision = source.new_owner_decision(g, d, 0, None).unwrap();
                store
                    .seal_studio_epoch(
                        SERVER,
                        g,
                        target,
                        d,
                        decision.receipt().clone(),
                        0,
                        r,
                        &mut budget,
                    )
                    .unwrap();
                (
                    decision.close().clone(),
                    budget,
                    title,
                    g.group_id(),
                    d.device_id(),
                )
            });
        drop(context);
        let basis = server
            .prepare_studio_closing_overlay(&mut store, SERVER, target, &close, &mut budget)
            .unwrap()
            .fingerprint();
        let StudioOverlaySave::Local(draft) = server
            .save_studio_closing_overlay(
                &mut store,
                SERVER,
                target,
                &close,
                basis,
                title,
                &mut budget,
            )
            .unwrap()
        else {
            panic!("expected actual local acceptance")
        };
        let expected = draft.projection().clone();
        assert_eq!(draft.accepted(), 1);
        let store = Arc::new(Mutex::new(Some(store)));
        let (actor, mut events, task) = app::spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        Self {
            store,
            actor,
            clock,
            target,
            expected,
            basis,
            group,
            device,
            root,
            task,
            drain,
        }
    }
    pub(super) async fn capture(&self) -> StudioInspectionPreparation {
        let StudioControlResponse::OverlayPreparation(job) = self
            .control(StudioControlAction::InspectOverlay)
            .await
            .unwrap()
        else {
            panic!("not a real capture")
        };
        job
    }
    pub(super) async fn control(
        &self,
        action: StudioControlAction,
    ) -> Result<StudioControlResponse, String> {
        tokio::time::timeout(Duration::from_secs(30), async {
            self.actor
                .studio_control_begin(StudioControlRequest {
                    target: self.target,
                    action,
                })
                .await?
                .execute(StudioVaultLease::new(
                    self.store.clone().try_lock_owned().unwrap(),
                    SERVER,
                    (),
                ))
                .await
        })
        .await
        .expect("inspection custody stalled")
    }
    pub(super) fn records(&self) -> std::collections::BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(self.root.path().join("servers"))
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                (
                    path.file_name().unwrap().to_str().unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                )
            })
            .collect()
    }
    pub(super) async fn shutdown(self) {
        self.actor.shutdown().await;
        self.task.await.unwrap();
        self.drain.await.unwrap();
    }
}
