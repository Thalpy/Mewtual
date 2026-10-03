//! The copy object probe at C3 and C4, through the real Server copy stages.
//!
//! An Index `PutObject` proves nothing about the Flipnote it names, and the planner runs on a
//! detached worker with no store, so it cannot tell. `probe_copy_object` is what puts that check
//! back under custody. These tests reach it with an otherwise valid `Ready` plan - the planner's own
//! verdict is asserted first - so that a refusal or downgrade can only be the probe's.
use super::*;
use crate::store::EpochStudioBudget;
use crate::studio::StudioRequest;
use catcoms_mls::MlsDevice;
use catcoms_replication::studio::{
    FlipnoteHeader, FlipnoteOp, IndexOp, StudioExpiry, StudioKind, StudioOverlaySave,
};
use catcoms_replication::DomainOp;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SERVER: u64 = 7;
const OBJECT: [u8; 16] = [6; 16];

type Node = Server<MemNetwork, ChaCha20Rng>;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(452)
}

fn budget(store: &mut ServerStore, server: &mut Node) -> EpochStudioBudget {
    let mut scan = store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    let inventory = scan.finish().unwrap();
    server
        .sync
        .with_registry_context(|g, _, _, _| store.studio_storage_budget(SERVER, g, &inventory))
        .unwrap()
}

/// An Index Closing overlay holding one accepted `PutObject` for `OBJECT`, retained across an
/// installed Open successor, so it is a live copy source with a current destination. No Flipnote
/// for `OBJECT` exists yet. The same construction as the replay-exclusion fixture: a source large
/// enough to reach the production rotation threshold, sealed, then a real Closing-overlay Save.
struct Fixture {
    _hub: std::sync::Arc<Hub>,
    _root: tempfile::TempDir,
    server: Node,
    store: ServerStore,
    index: StudioTarget,
    clock: ManualClock,
    /// Private, so held previews never compete with other tests for the process-wide pool.
    pool: std::sync::Arc<tokio::sync::Semaphore>,
}

impl Fixture {
    async fn new() -> Self {
        let hub = Hub::new();
        let clock = ManualClock::new(1000);
        let mut server = Server::found(
            hub.join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(clock.clone()),
            "copy-probe",
        )
        .unwrap();
        let index = StudioTarget::Index {
            channel: crate::channel_id("general").to_be_bytes(),
        };
        let root = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(root.path(), b"copy-probe", &mut rng()).unwrap();
        let logical = index.document(&server.group_id()).unwrap();
        let domain = |body: Vec<u8>, nonce: u8| DomainOp {
            body,
            nonce: [nonce; 16],
            doc_type: logical.doc_type,
            logical_key: logical.logical_key.clone(),
        };
        let put = |object: [u8; 16], title: &str, device: crate::DeviceId| {
            IndexOp::PutObject {
                object,
                kind: StudioKind::Flipnote,
                title: title.into(),
                created_by: device,
                ts: 100,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap()
        };

        let mut b = budget(&mut store, &mut server);
        let close = server
            .sync
            .with_registry_context(|g, d, _, r| {
                let mut source =
                    catcoms_replication::studio::StudioEpoch::new(g, index, d.device_id()).unwrap();
                let seed = domain(put([4; 16], "shared seed", d.device_id()), 1);
                let packet = source.edit_or_reseal(d, g, r, &seed, 100).unwrap();
                store.ingest_studio_epoch(SERVER, g, index, d, &packet, r, &mut b)?;
                let title = domain(
                    IndexOp::SetTitle {
                        object: [4; 16],
                        title: "padding".into(),
                    }
                    .encode()
                    .unwrap(),
                    9,
                );
                for n in 10..20u8 {
                    let mut op = title.clone();
                    op.nonce = [n; 16];
                    let mut copy = catcoms_replication::studio::StudioEpoch::restore(
                        &source.snapshot().unwrap(),
                        g,
                        index,
                        d.device_id(),
                    )
                    .unwrap();
                    let packet = copy.edit_or_reseal(d, g, r, &op, 100).unwrap();
                    let opened = packet
                        .open(&g.channel_secret(d, packet.doc_type, packet.doc_id).unwrap())
                        .unwrap();
                    let mut change = automerge::Change::from_bytes(opened.delta)
                        .unwrap()
                        .decode();
                    change.message = Some("x".repeat(220_000));
                    let change = automerge::Change::from(change);
                    let signed = catcoms_replication::SignedOp::sign_domain(
                        d,
                        logical.doc_type,
                        source.doc_id(),
                        change.raw_bytes().to_vec(),
                        &op,
                    )
                    .unwrap();
                    let packet = catcoms_replication::SealedOp::seal(&signed, g, d, r).unwrap();
                    source.ingest(&packet, g, d).unwrap();
                    store.ingest_studio_epoch(SERVER, g, index, d, &packet, r, &mut b)?;
                }
                let decision = source.new_owner_decision(g, d, 0, None).unwrap();
                store.seal_studio_epoch(
                    SERVER,
                    g,
                    index,
                    d,
                    decision.receipt().clone(),
                    0,
                    r,
                    &mut b,
                )?;
                Ok::<_, AppError>(decision.close().clone())
            })
            .unwrap();

        // The overlay entry the copy will take: a PutObject naming a Flipnote nobody created.
        let accepted = domain(put(OBJECT, "accepted overlay", server.device_id()), 3);
        let mut b = budget(&mut store, &mut server);
        let ticket = server
            .prepare_studio_closing_overlay(&mut store, SERVER, index, &close, &mut b)
            .unwrap();
        let StudioOverlaySave::Local(_) = server
            .save_studio_closing_overlay(
                &mut store,
                SERVER,
                index,
                &close,
                ticket.basis.fingerprint(),
                ticket.branch,
                accepted,
                &mut b,
            )
            .unwrap()
        else {
            panic!("expected actual local acceptance")
        };

        // Install the pristine successor so the destination is Open.
        let capture = server
            .sync
            .with_registry_context(|g, d, _, _| store.capture_studio_source(SERVER, g, index, d))
            .unwrap()
            .expect("the installed source is present");
        let prepared = capture.rebuild().unwrap();
        assert!(
            server
                .sync
                .with_registry_context(
                    |g, d, _, _| store.install_prepared_studio_source(g, d, prepared)
                )
                .unwrap()
        );
        let mut b = budget(&mut store, &mut server);
        server
            .sync
            .with_registry_context(|g, d, _, r| {
                store.install_sealed_studio_successor_for_test(
                    SERVER, g, index, d, &close, r, &mut b,
                )
            })
            .unwrap();

        Self {
            _hub: hub,
            _root: root,
            server,
            store,
            index,
            clock,
            pool: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }

    fn choice(&self) -> StudioOverlayCopyChoice {
        StudioOverlayCopyChoice {
            destination: self.index,
            item: StudioRecoveryItem::Object { id: OBJECT },
            mode: StudioRecoveryMode::Restore,
        }
    }

    /// C1 and C2 only: the planner's own verdict, before any custody probe.
    async fn plan(&mut self) -> StudioPreparedCopy {
        let choice = self.choice();
        let pool = self.pool.clone();
        self.server
            .begin_copy_with_pool(&self.store, SERVER, self.index, choice, &pool)
            .unwrap()
            .plan()
            .await
            .unwrap()
    }

    /// The renderer's echo of a plan, as C4 receives it.
    fn echo(&self, plan: &StudioOverlayCopyPlan) -> StudioOverlayCopyApply {
        StudioOverlayCopyApply {
            destination: self.index,
            item: StudioRecoveryItem::Object { id: OBJECT },
            mode: StudioRecoveryMode::Restore,
            epoch_id: plan.epoch_id(),
            expected_projection: plan.fingerprint(),
            nonce: [0x5a; 16],
            body: plan.body().cloned().expect("a Ready plan carries a body"),
        }
    }

    fn apply(&mut self, apply: StudioOverlayCopyApply) -> Result<(StudioRequest, bool), AppError> {
        self.server
            .prepare_studio_copy_apply(&mut self.store, SERVER, self.index, apply)
    }

    /// The Flipnote the overlay entry names, created for real in the same channel.
    fn create_object(&mut self) {
        let flipnote = StudioTarget::Flipnote {
            channel: self.index.channel(),
            object: OBJECT,
        };
        self.server
            .studio_transaction(
                &mut self.store,
                SERVER,
                StudioRequest::Apply {
                    target: flipnote,
                    epoch_id: catcoms_replication::epoch_zero_id(
                        catcoms_wire::DocType::StudioObject,
                        &OBJECT,
                    ),
                    nonce: [0x33; 16],
                    body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("the real object".into()))
                        .encode()
                        .unwrap(),
                },
            )
            .unwrap();
    }

    fn object_path(&mut self) -> std::path::PathBuf {
        let flipnote = StudioTarget::Flipnote {
            channel: self.index.channel(),
            object: OBJECT,
        };
        let store = &self.store;
        self.server
            .sync
            .with_registry_context(|g, _, _, _| {
                store.studio_source_path_for_test(SERVER, g, flipnote)
            })
            .unwrap()
    }
}

/// The missing object, at both stages, and the object that disappears between them.
///
/// Three halves on one fixture, each reaching the probe with the planner's `Ready`:
///
/// 1. never created: C3 downgrades to `MissingTarget`, and C4 refuses the planner's echo;
/// 2. created: C3 says `Ready` and C4 passes - the control that shows the halves around it are the
///    probe's doing and not something else about the fixture;
/// 3. cleaned up after a `Ready` preview: C4 refuses that same echo.
///
/// Every refusal is asserted by message, so a destination-stamp or currency refusal - which would
/// also stop the copy - cannot stand in for the probe's.
#[tokio::test]
async fn the_copy_probe_refuses_an_object_that_is_missing_or_disappears_before_apply() {
    let mut f = Fixture::new().await;

    // 1. Never created.
    let prepared = f.plan().await;
    assert_eq!(
        prepared.plan.disposition(),
        StudioRecoveryDisposition::Ready,
        "the planner cannot see the store, so it must call this Ready; otherwise the probe is not \
         what downgrades it"
    );
    let echo = f.echo(&prepared.plan);
    let preview = f
        .server
        .finish_studio_copy_preview(&mut f.store, SERVER, f.index, prepared)
        .unwrap();
    assert_eq!(
        preview.value().disposition,
        StudioRecoveryDisposition::MissingTarget,
        "C3 must tell the user the object is missing rather than offer a copy that names it"
    );
    let refused = f
        .apply(echo)
        .expect_err("C4 must refuse to publish an entry for an object that does not exist")
        .to_string();
    assert!(
        refused.contains("no longer exists"),
        "the C4 refusal must be the probe's, got: {refused}"
    );

    // 2. The control: the object exists, so both stages let the copy through.
    f.create_object();
    let prepared = f.plan().await;
    let echo = f.echo(&prepared.plan);
    let preview = f
        .server
        .finish_studio_copy_preview(&mut f.store, SERVER, f.index, prepared)
        .unwrap();
    assert_eq!(
        preview.value().disposition,
        StudioRecoveryDisposition::Ready
    );
    let (_, already) = f
        .apply(f.echo_for(&echo))
        .expect("with the object present, C4 must accept the echo");
    assert!(
        !already,
        "nothing was applied, so this is not an exact retry"
    );

    // 3. Cleaned up after that Ready preview, before the apply.
    let path = f.object_path();
    std::fs::remove_file(&path).expect("the object's record must exist to be removed");
    let refused = f
        .apply(echo)
        .expect_err("an object that disappeared after the preview must not be published")
        .to_string();
    assert!(
        refused.contains("no longer exists"),
        "the C4 refusal must be the probe's, got: {refused}"
    );
}

impl Fixture {
    /// A second copy of an echo, since the request is consumed by value.
    fn echo_for(&self, echo: &StudioOverlayCopyApply) -> StudioOverlayCopyApply {
        StudioOverlayCopyApply {
            destination: echo.destination,
            item: echo.item,
            mode: echo.mode,
            epoch_id: echo.epoch_id,
            expected_projection: echo.expected_projection,
            nonce: echo.nonce,
            body: echo.body.clone(),
        }
    }

    fn finish(&mut self, action: StudioControlAction) -> StudioControlResponse {
        self.server
            .studio_control_transaction(
                &mut self.store,
                SERVER,
                StudioControlRequest {
                    target: self.index,
                    action,
                },
            )
            .unwrap()
    }
}

/// Whether native could read the value right now, for any delivered variant.
fn readable(response: &StudioControlResponse) -> Result<(), String> {
    match response {
        StudioControlResponse::OverlayExport(v) => v.inspect(|_| ()),
        StudioControlResponse::OverlayArchived(v) => v.inspect(|_| ()),
        StudioControlResponse::OverlayCopyPreview(v) => v.inspect(|_| ()),
        other => panic!("not a delivered overlay result: {other:?}"),
    }
}

/// The review's parked-result regression, for export, archive and copy preview alike.
///
/// Each used to copy its payload out of the job and return it bare, which gave the job's shared
/// preparation slot back to the pool the moment the finish visit returned and skipped the delivery
/// fence entirely. Each is now held to the inspection's contract, and this checks both halves on a
/// private pool so no other test in the process can move the count:
///
/// - **the slot:** still held after the finish visit, still held by native's delivery after the
///   response itself is dropped, and returned only when that delivery is dropped;
/// - **the fence:** nothing is readable before the actor begins the handoff, it is readable while
///   the handoff is current, and an expired handoff revokes it. (That the actor processes nothing
///   else while the handoff is outstanding is `inspection`'s actor-level test.)
#[tokio::test]
async fn export_archive_and_copy_preview_keep_their_slot_and_fence_through_delivery() {
    let mut f = Fixture::new().await;
    f.create_object();
    let pool = std::sync::Arc::new(tokio::sync::Semaphore::new(4));

    for what in ["export", "archive", "copy preview"] {
        let mut response = match what {
            "export" | "archive" => {
                let prepared = f
                    .server
                    .begin_inspection_with_pool(&f.store, SERVER, f.index, &pool)
                    .unwrap()
                    .rebuild_for_archive()
                    .await
                    .unwrap();
                f.finish(if what == "export" {
                    StudioControlAction::FinishOverlayExport(Box::new(prepared))
                } else {
                    StudioControlAction::FinishOverlayArchive(Box::new(prepared))
                })
            }
            _ => {
                let choice = f.choice();
                let prepared = f
                    .server
                    .begin_copy_with_pool(&f.store, SERVER, f.index, choice, &pool)
                    .unwrap()
                    .plan()
                    .await
                    .unwrap();
                f.finish(StudioControlAction::FinishOverlayCopyPreview(Box::new(
                    prepared,
                )))
            }
        };
        assert_eq!(
            pool.available_permits(),
            3,
            "{what}: the finished result must still hold its job's slot"
        );
        assert!(
            readable(&response).is_err(),
            "{what}: a result the actor has not handed off must not be readable"
        );

        let handoff = response
            .begin_delivery(std::sync::Arc::new(f.clock.clone()))
            .unwrap_or_else(|| panic!("{what} must be a delivered variant"));
        readable(&response).unwrap_or_else(|e| panic!("{what}: a current delivery reads: {e}"));
        let delivery = response
            .delivery()
            .expect("the handoff installs a delivery");

        f.clock.advance_ms(5_000);
        assert!(
            readable(&response).is_err() && !delivery.is_current(),
            "{what}: an expired handoff must revoke the result"
        );
        drop(response);
        assert_eq!(
            pool.available_permits(),
            3,
            "{what}: native's delivery must keep the slot after the response is dropped"
        );
        drop(delivery);
        assert_eq!(
            pool.available_permits(),
            4,
            "{what}: dropping the last delivery must return the slot"
        );
        drop(handoff);
    }
}
