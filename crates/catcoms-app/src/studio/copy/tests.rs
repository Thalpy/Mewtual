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

/// Copy between two Flipnotes: the source and destination holds told apart (L-2), and a Save
/// landing on the source mid-copy (L-1).
mod cross;

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

/// One document's epoch 0, made large enough to reach the production rotation threshold, sealed,
/// and given `accepted` as a real Closing-overlay entry. Returns the close the overlay is on.
///
/// The successor is NOT installed, so the branch is still live and further Closing-overlay Saves
/// can land on it. [`install_successor`] makes the document Open with the branch retained.
///
/// `seed` opens the history. `padding` is applied ten times, under nonces 10 to 19, each change
/// carrying a 220 KB message, which is what crosses the threshold. `accepted` lands under nonce 3.
fn closing_branch(
    server: &mut Node,
    store: &mut ServerStore,
    target: StudioTarget,
    seed: Vec<u8>,
    padding: Vec<u8>,
    accepted: Vec<u8>,
) -> catcoms_replication::CloseRecord {
    let logical = target.document(&server.group_id()).unwrap();
    let domain = |body: Vec<u8>, nonce: u8| DomainOp {
        body,
        nonce: [nonce; 16],
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
    };
    let mut b = budget(store, server);
    let close = server
        .sync
        .with_registry_context(|g, d, _, r| {
            let mut source =
                catcoms_replication::studio::StudioEpoch::new(g, target, d.device_id()).unwrap();
            let packet = source
                .edit_or_reseal(d, g, r, &domain(seed, 1), 100)
                .unwrap();
            store.ingest_studio_epoch(SERVER, g, target, d, &packet, r, &mut b)?;
            for n in 10..20u8 {
                let op = domain(padding.clone(), n);
                let mut copy = catcoms_replication::studio::StudioEpoch::restore(
                    &source.snapshot().unwrap(),
                    g,
                    target,
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
                store.ingest_studio_epoch(SERVER, g, target, d, &packet, r, &mut b)?;
            }
            let decision = source.new_owner_decision(g, d, 0, None).unwrap();
            store.seal_studio_epoch(
                SERVER,
                g,
                target,
                d,
                decision.receipt().clone(),
                0,
                r,
                &mut b,
            )?;
            Ok::<_, AppError>(decision.close().clone())
        })
        .unwrap();
    closing_save(server, store, target, &close, domain(accepted, 3));
    close
}

/// A real Closing-overlay Save of `operation` onto `target`'s live branch.
fn closing_save(
    server: &mut Node,
    store: &mut ServerStore,
    target: StudioTarget,
    close: &catcoms_replication::CloseRecord,
    operation: DomainOp,
) {
    let mut b = budget(store, server);
    let ticket = server
        .prepare_studio_closing_overlay(store, SERVER, target, close, &mut b)
        .unwrap();
    let StudioOverlaySave::Local(_) = server
        .save_studio_closing_overlay(
            store,
            SERVER,
            target,
            close,
            ticket.basis.fingerprint(),
            ticket.branch,
            operation,
            &mut b,
        )
        .unwrap()
    else {
        panic!("expected actual local acceptance")
    };
}

/// Install `target`'s pristine successor, so the document is Open and its branch is retained.
fn install_successor(
    server: &mut Node,
    store: &mut ServerStore,
    target: StudioTarget,
    close: &catcoms_replication::CloseRecord,
) {
    let capture = server
        .sync
        .with_registry_context(|g, d, _, _| store.capture_studio_source(SERVER, g, target, d))
        .unwrap()
        .expect("the installed source is present");
    let prepared = capture.rebuild().unwrap();
    assert!(server
        .sync
        .with_registry_context(|g, d, _, _| store.install_prepared_studio_source(g, d, prepared))
        .unwrap());
    let mut b = budget(store, server);
    server
        .sync
        .with_registry_context(|g, d, _, r| {
            store.install_sealed_studio_successor_for_test(SERVER, g, target, d, close, r, &mut b)
        })
        .unwrap();
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
        let device = server.device_id();
        let put = |object: [u8; 16], title: &str| {
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
        // The overlay entry the copy will take: a PutObject naming a Flipnote nobody created.
        let close = closing_branch(
            &mut server,
            &mut store,
            index,
            put([4; 16], "shared seed"),
            IndexOp::SetTitle {
                object: [4; 16],
                title: "padding".into(),
            }
            .encode()
            .unwrap(),
            put(OBJECT, "accepted overlay"),
        );
        // Install the pristine successor so the destination is Open.
        install_successor(&mut server, &mut store, index, &close);

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
    fn logical(&self) -> catcoms_replication::LogicalDocument {
        self.index.document(&self.server.group_id()).unwrap()
    }

    /// The branch the copies read from, as everything that records it.
    fn branch_identity(&self) -> ([u8; 32], [u8; 32], usize, Vec<u8>) {
        self.store
            .studio_branch_identity_for_test(SERVER, &self.logical())
    }

    /// Operations in the destination's installed source.
    fn destination_ops(&mut self) -> usize {
        let (store, index) = (&mut self.store, self.index);
        self.server
            .sync
            .with_registry_context(|g, d, _, _| {
                store.with_studio_source(SERVER, g, index, d, |s| Ok(s.op_count()))
            })
            .unwrap()
            .expect("the destination is installed")
    }

    /// What the actor does with C4's request: the ordinary Apply publication into the destination.
    fn publish(&mut self, request: StudioRequest) {
        self.server
            .studio_transaction(&mut self.store, SERVER, request)
            .unwrap();
    }

    /// A store restart: the same vault reopened. The previous handle is dropped first.
    fn restart(&mut self) {
        let placeholder_root = tempfile::tempdir().unwrap();
        let placeholder =
            ServerStore::open(placeholder_root.path(), b"placeholder", &mut rng()).unwrap();
        drop(std::mem::replace(&mut self.store, placeholder));
        self.store = ServerStore::open(self._root.path(), b"copy-probe", &mut rng()).unwrap();
    }

    /// A durable transfer hold on the index document, staged through the real handoff.
    fn stage_transfer_hold(&mut self) {
        let mut b = budget(&mut self.store, &mut self.server);
        let (store, index) = (&mut self.store, self.index);
        self.server
            .sync
            .with_registry_context(|g, d, _, r| {
                store.stage_studio_transfer_hold_for_test(SERVER, g, index, d, 0, r, &mut b);
                Ok::<_, AppError>(())
            })
            .unwrap();
    }

    /// The overlay entry's object, created for real but under ANOTHER channel's label. A
    /// Flipnote's logical key omits its channel, so this is the very record the probe reads for
    /// `OBJECT`, stored under a label that is not the copy's.
    fn create_object_under(&mut self, channel: [u8; 16]) {
        let target = StudioTarget::Flipnote {
            channel,
            object: OBJECT,
        };
        let mut b = budget(&mut self.store, &mut self.server);
        let store = &mut self.store;
        self.server
            .sync
            .with_registry_context(|g, d, _, r| {
                let logical = target.document(&g.group_id()).unwrap();
                let mut source =
                    catcoms_replication::studio::StudioEpoch::new(g, target, d.device_id())
                        .unwrap();
                let op = DomainOp {
                    body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("elsewhere".into()))
                        .encode()
                        .unwrap(),
                    nonce: [0x44; 16],
                    doc_type: logical.doc_type,
                    logical_key: logical.logical_key,
                };
                let packet = source.edit_or_reseal(d, g, r, &op, 100).unwrap();
                store.ingest_studio_epoch(SERVER, g, target, d, &packet, r, &mut b)
            })
            .unwrap();
    }

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

/// M3's missing half, N6 for a same-document copy, and the same retry across a store restart.
///
/// The copy is carried all the way through: C4's request is published exactly as the actor
/// publishes it, so "landed" is an operation in the destination rather than an accepted echo.
/// Then the identical echo is sent again, as a renderer does after an uncertain result. It must
/// be acknowledged as already saved rather than refused, and it must write nothing. The same
/// holds after the store is reopened. Throughout, the branch the value came from is untouched in
/// everything that records it (N6): its id, its content, its accepted count and its metadata
/// bytes.
#[tokio::test]
async fn a_copy_lands_once_and_its_exact_retry_is_acknowledged_without_a_second_operation() {
    let mut f = Fixture::new().await;
    f.create_object();
    let branch = f.branch_identity();

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
    drop(preview);

    let before = f.destination_ops();
    let (request, already) = f.apply(f.echo_for(&echo)).unwrap();
    assert!(!already, "nothing has landed yet");
    f.publish(request);
    assert_eq!(
        f.destination_ops(),
        before + 1,
        "the copy lands as exactly one operation"
    );
    assert_eq!(
        f.branch_identity(),
        branch,
        "N6: a same-document copy changes nothing that records the branch"
    );

    let (request, already) = f
        .apply(f.echo_for(&echo))
        .expect("an exact retry is acknowledged, not refused as stale");
    assert!(already, "the retry must say the copy already landed");
    f.publish(request);
    assert_eq!(
        f.destination_ops(),
        before + 1,
        "and publishing the retry writes nothing"
    );

    f.restart();
    let (_, already) = f
        .apply(f.echo_for(&echo))
        .expect("the exact retry is acknowledged after a restart too");
    assert!(already);
    assert_eq!(f.branch_identity(), branch);
}

/// L4: an ordinary Save is never reported as this copy having landed.
///
/// The ordinary Save here carries the copy's exact body under the renderer's exact nonce. That is
/// the one shape that used to be byte-identical to the copy's own operation, so C4's exact-retry
/// shortcut answered `already_saved` for work the copy never did. Copies now publish under their
/// own nonce domain. So the echo is re-planned instead: the ordinary Save moved the destination, so
/// the plan's projection no longer matches the preview's, and the echo is refused as stale. That is
/// what is actually true.
#[tokio::test]
async fn an_ordinary_save_of_the_same_bytes_is_never_reported_as_this_copy() {
    let mut f = Fixture::new().await;
    f.create_object();
    let prepared = f.plan().await;
    let echo = f.echo(&prepared.plan);
    drop(prepared);

    f.publish(StudioRequest::Apply {
        target: f.index,
        epoch_id: echo.epoch_id,
        nonce: echo.nonce,
        body: echo.body.clone(),
    });
    let result = f.apply(f.echo_for(&echo));
    assert!(
        !matches!(result, Ok((_, true))),
        "an ordinary Save must not be acknowledged as this copy having landed"
    );
    let refused = result.unwrap_err().to_string();
    assert!(
        refused.contains("copy preview is stale"),
        "the echo must be re-planned against the destination the Save moved: {refused}"
    );
}

/// M4 / design 6.3 C1': a transfer hold on the destination refuses the copy at C1, at C3 when it
/// was staged after C1, and at C4 when it was staged after the preview. Every refusal is by
/// message, so neither the publication path's own guard nor the source-stamp currency check can
/// stand in for the hold check. At C4 that publication guard would otherwise answer after a full
/// re-plan, with a generic reason instead of this copy's retryable one (the review's M1).
#[tokio::test]
async fn a_transfer_hold_on_the_destination_refuses_the_copy_at_c1_c3_and_c4() {
    let mut f = Fixture::new().await;
    f.create_object();

    let prepared = f.plan().await;
    let echo = f.echo(&prepared.plan);
    let before = f.destination_ops();
    f.stage_transfer_hold();
    let refused = f
        .server
        .finish_studio_copy_preview(&mut f.store, SERVER, f.index, prepared)
        .expect_err("a hold staged since C1 must refuse the preview")
        .to_string();
    assert!(refused.contains("transfer hold"), "C3 refusal: {refused}");

    let (choice, pool) = (f.choice(), f.pool.clone());
    let refused = f
        .server
        .begin_copy_with_pool(&f.store, SERVER, f.index, choice, &pool)
        .expect_err("C1 must refuse while the destination is held")
        .to_string();
    assert!(refused.contains("transfer hold"), "C1 refusal: {refused}");

    let refused = f
        .apply(echo)
        .expect_err("C4 must refuse while the destination is held")
        .to_string();
    assert!(refused.contains("transfer hold"), "C4 refusal: {refused}");
    assert_eq!(f.destination_ops(), before, "a refused apply wrote nothing");
    assert_eq!(
        pool.available_permits(),
        4,
        "a refused begin keeps no preparation slot"
    );
}

/// The wrong-object-channel Low: an object stored under another channel's label is a missing
/// target in this channel, not an error that fails the preview. C3 downgrades it like an absent
/// object, and C4 refuses the echo with the probe's own message.
#[tokio::test]
async fn an_object_stored_under_another_channel_label_is_missing_here_not_an_error() {
    let mut f = Fixture::new().await;
    f.create_object_under([0x77; 16]);

    let prepared = f.plan().await;
    assert_eq!(
        prepared.plan.disposition(),
        StudioRecoveryDisposition::Ready,
        "the planner cannot see the store, so only the probe can downgrade this"
    );
    let echo = f.echo(&prepared.plan);
    let preview = f
        .server
        .finish_studio_copy_preview(&mut f.store, SERVER, f.index, prepared)
        .expect("a wrong-channel object is a missing target, not a failed preview");
    assert_eq!(
        preview.value().disposition,
        StudioRecoveryDisposition::MissingTarget
    );
    drop(preview);
    let refused = f.apply(echo).unwrap_err().to_string();
    assert!(
        refused.contains("no longer exists in this channel"),
        "the C4 refusal must be the probe's: {refused}"
    );
}

/// Review of `b35e23d2`, MEDIUM-1: a Closing draft's accepted operation resent through the
/// Unconfirmed Save action is refused, never answered as saved Unconfirmed work.
///
/// This fixture's live branch is a Closing one with one accepted operation. The store's exact-retry
/// acknowledgement is kind-blind: it compares the request's basis and branch, and never reaches the
/// mint. The oracle shows that the store alone, given that request and a failed Unconfirmed mint,
/// does acknowledge it as `Local`, which the receiver would have reported as
/// `provenance:"unconfirmed"`. So the refusal is the receiver's guard.
#[tokio::test]
async fn a_closing_drafts_operation_resent_as_an_unconfirmed_save_is_refused() {
    let mut f = Fixture::new().await;
    let logical = f.logical();
    let state = f
        .store
        .load_epoch_intents_structural(SERVER, &logical)
        .unwrap();
    let metadata = state.handoff_metadata().unwrap();
    let basis = metadata.overlay().unwrap().basis();
    let branch = metadata.branch_id().unwrap();
    let device = f.server.device_id();
    let body = IndexOp::PutObject {
        object: OBJECT,
        kind: StudioKind::Flipnote,
        title: "accepted overlay".into(),
        created_by: device,
        ts: 100,
        expiry: StudioExpiry::Never,
    }
    .encode()
    .unwrap();

    // The oracle: the store acknowledges the resend.
    let mut b = budget(&mut f.store, &mut f.server);
    let store = &mut f.store;
    let index = f.index;
    let acknowledged = f
        .server
        .sync
        .with_registry_context(|g, d, clock, rng| {
            store.start_studio_overlay(
                SERVER,
                g,
                index,
                d,
                crate::store::StudioOverlayMint::unconfirmed(Err(invalid("no preview"))),
                basis,
                branch,
                crate::studio::domain(index, [3; 16], body.clone()),
                clock.now_ms(),
                rng,
                &mut b,
            )
        })
        .unwrap();
    assert!(
        matches!(
            acknowledged,
            crate::store::StudioOverlayStart::Settled(ref saved)
                if matches!(**saved, StudioOverlaySave::Acknowledged { .. })
        ),
        "precondition: the store alone acknowledges the resend"
    );

    let mut receiver = crate::studio::PreviewHarness::default().into_receiver(Vec::new());
    let answered = receiver
        .control(
            &mut f.server,
            &mut f.store,
            SERVER,
            crate::studio::StudioControlRequest {
                target: f.index,
                action: crate::studio::StudioControlAction::SaveUnconfirmedOverlay(Box::new(
                    crate::studio::StudioUnconfirmedOverlaySaveRequest {
                        basis,
                        branch,
                        nonce: [3; 16],
                        body,
                    },
                )),
            },
        )
        .map(|(_, _, response)| response);
    let refused = match answered {
        Err(error) => error.to_string(),
        Ok(response) => {
            panic!("a Closing draft must be refused by the Unconfirmed Save, got {response:?}")
        }
    };
    assert!(
        refused.contains("was not made on a preview"),
        "a Closing draft is refused by the receiver, not reported as Unconfirmed work: {refused}"
    );

    // The ticket names the same reason, and names it first. This receiver has no preview, so a
    // ticket that minted before checking would be refused for that instead (review of
    // `265b0756`, LOW-3).
    let ticket = receiver
        .control(
            &mut f.server,
            &mut f.store,
            SERVER,
            crate::studio::StudioControlRequest {
                target: f.index,
                action: crate::studio::StudioControlAction::BeginUnconfirmedOverlaySave,
            },
        )
        .map(|(_, _, response)| response);
    let refused = match ticket {
        Err(error) => error.to_string(),
        Ok(response) => panic!("a ticket for a Closing draft must be refused, got {response:?}"),
    };
    assert!(
        refused.contains("was not made on a preview"),
        "the ticket names the Closing draft before it mints: {refused}"
    );
}
