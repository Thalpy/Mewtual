use super::recovery as recovery_ipc;
use super::*;
// The shared fixture below reaches its crate through `super::app`.
use catcoms_app as app;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::future::Future;
mod export_acceptance;
/// The shared genuine-accepted-draft fixture, declared once for the whole Studio test tree.
/// `#[path]`-including it from two test modules would compile two unrelated copies of it.
#[path = "../../../../../crates/catcoms-app/tests/support/studio_inspection.rs"]
pub(in crate::studio) mod fixture;
mod preview;
mod receiver;
mod recovery;
mod recovery_restore;
mod sync_state;

#[test]
fn score_diagnostics_distinguish_effective_removal_from_raw_winner() {
    use app::studio::types::{
        ScoreCell, ScoreOp, ScoreProjection, ScoreRegister, ScoreSlot, ScoreSource, ScoreValue,
    };
    let document = app::studio::types::score_document(b"native-score-view", [9; 16]).unwrap();
    let mut score =
        ScoreProjection::read(&document, [7; 16], 0, &automerge::AutoCommit::new()).unwrap();
    let author = DeviceId::from_bytes([3; 32]);
    let source = |nonce: u8, body: ScoreOp| {
        let op = catcoms_replication::DomainOp {
            nonce: [nonce; 16],
            doc_type: document.doc_type,
            logical_key: document.logical_key.clone(),
            body: body.encode().unwrap(),
        };
        ScoreSource {
            op_id: op.id(&author),
            author,
            nonce: op.nonce,
        }
    };
    let cell = ScoreCell::Drum { pad: 0 };
    score.cells.insert(
        (0, 0),
        ScoreSlot {
            register: ScoreRegister {
                selected: ScoreValue {
                    source: source(
                        1,
                        ScoreOp::SetCell {
                            step: 0,
                            row: 0,
                            value: cell.clone(),
                        },
                    ),
                    value: Some(cell),
                },
                conflicts: vec![ScoreValue {
                    source: source(2, ScoreOp::ClearCell { step: 0, row: 0 }),
                    value: None,
                }],
            },
        },
    );
    let view = projection_content(&StudioProjection::Score(Box::new(score))).unwrap();
    assert_eq!(view["kind"], "score");
    assert_eq!(view["cells"]["0:0"]["removed"], true);
    assert_eq!(
        view["cells"]["0:0"]["register"]["selected"]["value"],
        json!({"k":"d","pad":0})
    );
    assert!(view["cells"]["0:0"]["register"]["conflicts"][0]["value"].is_null());
    assert_eq!(
        view["cells"]["0:0"]["register"]["selected"]["source"]["author"],
        author.to_string()
    );
}

#[test]
fn frame_sound_diagnostics_keep_unready_references_duration_and_conflicts() {
    let document = flipnote_document(b"native-sound-view", [9; 16]).unwrap();
    let mut art =
        FlipnoteFrameProjection::read(&document, [7; 16], 0, &automerge::AutoCommit::new())
            .unwrap();
    assert!(
        projection_content(&StudioProjection::Flipnote(Box::new(art.clone())))
            .unwrap()
            .get("sound")
            .is_none()
    );
    let operation = |duration_ms| FlipnoteOp::SetSfx {
        sfx: [1; 16],
        frame: [2; 16],
        patch: [3; 32],
        note: 60,
        duration_ms,
    };
    let value = |nonce, duration_ms| {
        let value = operation(duration_ms);
        let author = DeviceId::from_bytes([4; 32]);
        let op = catcoms_replication::DomainOp {
            doc_type: document.doc_type,
            logical_key: document.logical_key.clone(),
            nonce: [nonce; 16],
            body: value.encode().unwrap(),
        };
        FrameValue {
            source: FrameSource {
                op_id: op.id(&author),
                author,
                nonce: op.nonce,
                ts: 123,
            },
            value,
        }
    };
    let key = format!("s/{}", hex::encode([1; 16]));
    art.sound.records.insert(
        key.clone(),
        FrameRegister {
            selected: value(1, 600000),
            conflicts: vec![value(2, 250)],
        },
    );
    let projection = StudioProjection::Flipnote(Box::new(art));
    let content = projection_content(&projection).unwrap();
    assert_eq!(
        content["sound"]["records"][&key]["selected"]["value"]["duration_ms"],
        600000
    );
    assert_eq!(
        content["sound"]["records"][&key]["conflicts"][0]["value"]["duration_ms"],
        250
    );
    assert_eq!(
        content["sound"]["readiness"]["missingFrames"],
        json!([hex::encode([1; 16])])
    );
    assert_eq!(
        content["sound"]["readiness"]["missingPatches"],
        json!([hex::encode([1; 16])])
    );
    assert_eq!(content["sound"]["readiness"]["conflicts"], true);
    let ordinary = view(StudioView {
        epoch_id: 7,
        epoch: 0,
        phase: EpochPhase::Open,
        projection,
    })
    .unwrap();
    assert_eq!(ordinary["content"], content);
    assert_eq!(ordinary["group"], hex::encode(b"native-sound-view"));
}

fn save_response(outcome: app::studio::StudioUnconfirmedSaveOutcome) -> InvokeResponse {
    InvokeResponse::Control(
        app::studio::StudioControlResponse::UnconfirmedOverlaySaved {
            target: StudioTarget::Index {
                channel: 7u128.to_be_bytes(),
            },
            outcome,
        },
    )
}

/// MAP K13: final native fences must not relabel a Save that may already be durable as a clean
/// refusal. Conversely, outcomes which explicitly say this request wrote nothing stay plain.
#[test]
fn native_withheld_save_classification_follows_the_typed_outcome() {
    use app::studio::types::StudioOverlaySaveRefusal;
    use app::studio::StudioUnconfirmedSaveOutcome as O;

    assert_eq!(
        withheld_write(&save_response(O::Saved {
            basis: [3; 32],
            accepted: 1,
        })),
        Some(WithheldWrite::Save)
    );
    assert_eq!(
        withheld_write(&save_response(O::Scheduled)),
        Some(WithheldWrite::Save),
        "the detached worker can commit after native stops waiting"
    );
    for outcome in [
        O::Busy,
        O::Paused,
        O::Refused {
            reason: StudioOverlaySaveRefusal::NonceConflict,
            message: "diagnostic".into(),
        },
    ] {
        assert_eq!(withheld_write(&save_response(outcome)), None);
    }
}

#[test]
fn every_destructive_or_copy_success_is_classified_before_a_final_fence() {
    use app::studio::StudioControlResponse as R;

    assert_eq!(
        withheld_write(&InvokeResponse::Control(R::OverlayArchiveReleased)),
        Some(WithheldWrite::Release)
    );
    assert_eq!(
        withheld_write(&InvokeResponse::Control(R::OverlayCopyApplied {
            destination: StudioTarget::Index {
                channel: 7u128.to_be_bytes(),
            },
            already_saved: false,
        })),
        Some(WithheldWrite::Copy)
    );
    assert_eq!(
        withheld_write(&InvokeResponse::Control(R::OverlayCopyRefused {
            destination: StudioTarget::Index {
                channel: 7u128.to_be_bytes(),
            },
            reason: app::studio::StudioOverlayCopyRefusal::StalePreview,
        })),
        None,
        "a typed refusal is emitted before publication and must not be relabeled uncertain"
    );
}

#[test]
fn scheduled_repair_is_uncertain_when_a_final_session_fence_withholds_its_reply() {
    use app::studio::{StudioControlResponse as R, StudioFaultScope, StudioRepairStart};
    for (start, expected) in [
        (StudioRepairStart::Scheduled, Some(WithheldWrite::Repair)),
        (StudioRepairStart::Busy, None),
    ] {
        assert_eq!(
            withheld_write(&InvokeResponse::Control(R::RepairStarted {
                target: StudioTarget::Index { channel: [7; 16] },
                scope: StudioFaultScope::Source,
                start
            })),
            expected
        );
    }
}

#[test]
fn closing_scheduled_save_uses_the_same_uncertain_delivery_classification() {
    let response =
        InvokeResponse::Control(app::studio::StudioControlResponse::ClosingOverlaySaved {
            target: StudioTarget::Index { channel: [7; 16] },
            outcome: app::studio::StudioUnconfirmedSaveOutcome::Scheduled,
        });
    assert_eq!(withheld_write(&response), Some(WithheldWrite::Save));
}

#[tokio::test]
async fn native_studio_superseded_view_is_rejected_after_conversion() {
    let (_root, state, actor, task, drain) = fixture().await;
    let target = StudioTarget::Index {
        channel: channel_id(&channel()).unwrap(),
    };
    let result = invoke_custody(
        &state,
        7,
        InvokeRequest::Document(StudioRequest::Read { target }),
        |response| {
            let InvokeResponse::Document(Some(read)) = response else {
                panic!("empty Index read");
            };
            let value = read_view(read)?;
            // Admit the next native request after the old conversion has actually succeeded.
            // Dropping it cannot revive the older response; the session/actor remain unchanged.
            let newer = requests::ViewRequest::begin(&state, 7, target);
            assert!(newer.is_current());
            Ok(value)
        },
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        "Studio view request was superseded; refresh"
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(81)
}
fn channel() -> String {
    catcoms_app::channel_id("general").to_string()
}
fn target() -> StudioTarget {
    StudioTarget::Flipnote {
        channel: channel_id(&channel()).unwrap(),
        object: [7; 16],
    }
}
fn create() -> StudioRequest {
    StudioRequest::Create {
        channel: channel_id(&channel()).unwrap(),
        object: [7; 16],
        nonce: [9; 16],
        title: "moon cat".into(),
        ts: 123,
    }
}
pub(super) async fn install(
    state: &AppState,
    actor: ServerActor,
    group_id: Vec<u8>,
    device_id: DeviceId,
    instance: u64,
) {
    state.servers.lock().await.insert(
        7,
        ServerEntry {
            actor,
            instance,
            group_id,
            device_id,
            invite: None,
            name: "test".into(),
            bootstrap: vec![],
            bootstrap_owners: HashMap::new(),
            interface_routes: None,
            rendezvous: vec![],
            mesh: None,
            is_dm: false,
            switchboard: false,
            record_seq: 0,
            persist: PersistCounters::default(),
        },
    );
}
async fn fixture() -> (
    tempfile::TempDir,
    Arc<AppState>,
    ServerActor,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::default());
    *state.store.lock().await =
        Some(ServerStore::open(root.path(), b"studio-native", &mut rng()).unwrap());
    *state.session_resumable.lock().await = true;
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    attach_blob_store(&state, &mut server).await;
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, mut events, task) = spawn(server);
    install(&state, actor.clone(), group_id, device_id, 1).await;
    let s = state.clone();
    // Model the real forwarder's vault-dependent session check. This used to be the other half
    // of the proposed queue-held-vault deadlock; the Ready handoff must coexist with it.
    let drain = tokio::spawn(async move {
        while events.recv().await.is_some() {
            let _ = require_unlocked_session(&s).await;
        }
    });
    (root, state, actor, task, drain)
}

#[tokio::test]
async fn native_studio_save_restart_reopen_real_blob_and_complete_projection() {
    let (root, state, actor, task, drain) = fixture().await;
    let created = invoke(&state, 7, create()).await.unwrap().unwrap();
    assert_eq!(created["publication"], "local");
    assert_eq!(created["provisional"], true);
    assert_eq!(created["phase"], "open");
    let mut pix = vec![
        80, 73, 88, 49, 191, 143, 3, 1, 19, 18, 24, 2, 232, 230, 240, 3, 151, 125, 242, 0, 224,
        122, 184,
    ];
    pix.extend((0..108).flat_map(|_| [255, 0]));
    let cid = actor.publish_pix(pix.clone(), None).await.unwrap().cid;
    let body = FlipnoteOp::InsertFrame {
        frame: [1; 16],
        after: None,
        cid: *Cid::from_hex(&cid).unwrap().as_bytes(),
        bytes: pix.len() as u64,
    }
    .encode()
    .unwrap();
    let request = || {
        apply_request(
            &channel(),
            Some(&hex::encode([7; 16])),
            created["epochId"].as_str().unwrap(),
            &hex::encode([8; 16]),
            String::from_utf8(body.clone()).unwrap(),
        )
        .unwrap()
    };
    let saved = invoke(&state, 7, request()).await.unwrap().unwrap();
    assert_eq!(
        saved["content"]["frames"][hex::encode([1; 16])]["pixels"]["selected"]["value"]["cid"],
        cid
    );
    assert_eq!(
        saved["content"]["frames"][hex::encode([1; 16])]["pixels"]["selected"]["source"]["author"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    // Gate 7 keeps all object families and media references in the SAME sealed-vault restart.
    // Parser-only/native-shaped fixtures would miss a saved wrapper, custody or family mismatch.
    let score_target = StudioTarget::Score {
        channel: target().channel(),
        object: [6; 16],
    };
    let score_created = invoke(
        &state,
        7,
        StudioRequest::CreateScore {
            channel: target().channel(),
            object: [6; 16],
            nonce: [21; 16],
            title: "linked music".into(),
            ts: 123,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let score_saved = invoke(
        &state,
        7,
        apply_request_with_kind(
            &channel(),
            Some(&hex::encode([6; 16])),
            Some("score"),
            score_created["epochId"].as_str().unwrap(),
            &hex::encode([22; 16]),
            String::from_utf8(
                ScoreOp::SetCell {
                    step: 15,
                    row: 24,
                    value: ScoreCell::Drum { pad: 9 },
                }
                .encode()
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let vectors: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json"
    )))
    .unwrap();
    let descriptor = app::studio::types::StudioPatch::new(&vectors["patch"]).unwrap();
    let patch = descriptor.id();
    let package = hex::decode(vectors["pixaHex"].as_str().unwrap()).unwrap();
    let package_cid = actor.publish_pixa(package.clone(), None).await.unwrap().cid;
    for (nonce, op) in [
        (23, FlipnoteOp::SetPatch { patch, descriptor }),
        (
            24,
            FlipnoteOp::SetSfx {
                sfx: [2; 16],
                frame: [1; 16],
                patch,
                note: 60,
                duration_ms: 120,
            },
        ),
        (
            25,
            FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([6; 16]))),
        ),
        (
            26,
            FlipnoteOp::SetExport {
                export: [3; 16],
                cid: *Cid::from_hex(&package_cid).unwrap().as_bytes(),
                bytes: package.len() as u64,
                expiry: StudioExpiry::Never,
            },
        ),
    ] {
        invoke(
            &state,
            7,
            apply_request(
                &channel(),
                Some(&hex::encode([7; 16])),
                created["epochId"].as_str().unwrap(),
                &hex::encode([nonce; 16]),
                String::from_utf8(op.encode().unwrap()).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    }
    let saved = invoke(&state, 7, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved["content"]["sound"]["readiness"]["missingFrames"],
        json!([])
    );
    assert_eq!(
        saved["content"]["sound"]["readiness"]["missingPatches"],
        json!([])
    );
    assert_eq!(
        saved["content"]["sound"]["records"][format!("s/{}", hex::encode([2; 16]))]["selected"]
            ["value"]["duration_ms"],
        120
    );
    assert_eq!(score_saved["content"]["kind"], "score");
    assert_eq!(
        score_saved["content"]["cells"]["15:24"]["register"]["selected"]["value"],
        json!({"k":"d","pad":9})
    );
    let records = saved["content"]["sound"]["records"].as_object().unwrap();
    assert_eq!(
        records["h/score"]["selected"]["value"]["value"],
        hex::encode([6; 16])
    );
    assert_eq!(
        records[&format!("p/{}", hex::encode(patch))]["selected"]["value"]["descriptor"],
        vectors["patch"]
    );
    let export = records
        .values()
        .map(|r| &r["selected"]["value"])
        .find(|v| v["op"] == "set_export")
        .expect("export must be present in native DTO");
    assert_eq!(export["cid"], package_cid);
    assert_eq!(export["bytes"], package.len());
    assert_eq!(export["export"], hex::encode([3; 16]));
    assert_eq!(saved["publication"], "local");
    assert_eq!(saved["provisional"], true);
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    let (group_id, device_id) = {
        let entries = state.servers.lock().await;
        let e = &entries[&7];
        (e.group_id.clone(), e.device_id)
    };
    state.servers.lock().await.clear();
    drop(state.store.lock().await.take());
    *state.store.lock().await =
        Some(ServerStore::open(root.path(), b"studio-native", &mut rng()).unwrap());
    let snapshot = state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(7)
        .unwrap();
    let mut server = Server::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(2)),
        rng(),
        Box::new(ManualClock::new(999)),
        "alice",
    )
    .unwrap();
    attach_blob_store(&state, &mut server).await;
    let (actor, mut events, task) = spawn(server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    install(&state, actor.clone(), group_id, device_id, 2).await;
    assert_eq!(
        invoke(&state, 7, StudioRequest::Read { target: target() })
            .await
            .unwrap()
            .unwrap(),
        saved
    );
    assert_eq!(invoke(&state, 7, request()).await.unwrap().unwrap(), saved);
    assert_eq!(
        invoke(
            &state,
            7,
            StudioRequest::Read {
                target: score_target
            }
        )
        .await
        .unwrap()
        .unwrap(),
        score_saved
    );
    assert!(
        invoke(
            &state,
            7,
            StudioRequest::Read {
                target: StudioTarget::Flipnote {
                    channel: target().channel(),
                    object: [6; 16],
                }
            }
        )
        .await
        .is_err(),
        "native callers cannot reinterpret the reopened Score as art"
    );
    assert_eq!(
        actor
            .request_blob_bounded(Cid::from_hex(&package_cid).unwrap(), package.len(), None)
            .await
            .unwrap()
            .unwrap(),
        package
    );
    assert_eq!(
        actor
            .request_blob_bounded(Cid::from_hex(&cid).unwrap(), pix.len(), None)
            .await
            .unwrap()
            .unwrap(),
        pix
    );
    let index = invoke(
        &state,
        7,
        StudioRequest::Read {
            target: StudioTarget::Index {
                channel: channel_id(&channel()).unwrap(),
            },
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        index["content"]["objects"][hex::encode([7; 16])]["expiry"]["selected"]["value"],
        json!({"kind":"unrecorded"})
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_studio_ready_refuses_every_busy_or_stale_fence_without_waiting() {
    let (_root, state, actor, task, drain) = fixture().await;
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    for gate in 0..5 {
        let ready = actor.studio_begin(create()).await.unwrap();
        match gate {
            0 => {
                let _guard = state.store.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
            1 => {
                let _guard = state.ui_session_commit.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
            2 => {
                let lock = persist_lock_for(&state, 7);
                let _guard = lock.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
            3 => {
                let _guard = state.servers.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
            _ => {
                let _guard = state.session_resumable.lock().await;
                assert!(authorize(&state, 7, 1, generation).is_err());
            }
        }
        drop(ready);
        assert_eq!(actor.member_count().await, 1);
    }
    for mode in 0..3 {
        let ready = actor.studio_begin(create()).await.unwrap();
        // Each fence must reject independently, not inherit the stale generation
        // from the previous case and accidentally leave incarnation untested.
        let generation = state.ui_session_generation.load(Ordering::Acquire);
        if mode == 0 {
            state.session_lock_requested.store(true, Ordering::Release);
        }
        if mode == 1 {
            state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        }
        if mode == 2 {
            state.servers.lock().await.get_mut(&7).unwrap().instance = 2;
        }
        assert!(authorize(&state, 7, 1, generation).is_err());
        drop(ready);
        state.session_lock_requested.store(false, Ordering::Release);
        assert_eq!(actor.member_count().await, 1);
    }
    assert!(
        state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server(7)
            .is_err(),
        "no Studio snapshot write"
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_studio_lease_retains_fences_and_cancelled_receiver_cannot_create() {
    let (_root, state, actor, task, drain) = fixture().await;
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    let ready = actor.studio_begin(create()).await.unwrap();
    let lease = authorize(&state, 7, 1, generation).unwrap();
    assert!(state.store.try_lock().is_err());
    assert!(state.servers.try_lock().is_err());
    assert!(state.ui_session_commit.try_lock().is_err());
    assert!(persist_lock_for(&state, 7).try_lock_owned().is_err());
    // Poll through lease transfer but cancel the result receiver before the single-threaded
    // actor runs. The command must release its owned native guards and perform no transaction.
    let mut future = Box::pin(ready.execute(lease));
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(future);
    assert_eq!(actor.member_count().await, 1);
    assert!(state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(7)
        .is_err());
    assert!(state.servers.try_lock().is_ok());
    assert!(state.ui_session_commit.try_lock().is_ok());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[test]
fn native_studio_contract_rejects_bad_ids_bodies_and_preserves_expiry_states() {
    for bad in [
        "0",
        "ABABABABABABABABABABABABABABABAB",
        "gggggggggggggggggggggggggggggggg",
    ] {
        assert!(id(bad).is_err());
    }
    for bad in ["-1", "01", "+1", "340282366920938463463374607431768211456"] {
        assert!(channel_id(bad).is_err());
    }
    assert_eq!(
        expiry(StudioExpiry::Unrecorded),
        json!({"kind":"unrecorded"})
    );
    assert_eq!(expiry(StudioExpiry::Never), json!({"kind":"never"}));
    assert_eq!(expiry(StudioExpiry::At(0)), json!({"kind":"at","ms":0}));
    assert!(apply_request(
        &channel(),
        None,
        &"0".repeat(32),
        &"1".repeat(32),
        "x".repeat(65537)
    )
    .is_err());
    assert!(apply_request(
        &channel(),
        None,
        &"0".repeat(32),
        &"1".repeat(32),
        "{}".into()
    )
    .is_err());
    assert!(!format!("{:?}", create()).contains("moon cat"));
}

#[test]
fn native_object_family_and_score_recovery_choices_are_explicit_and_closed() {
    let object = hex::encode([9; 16]);
    let channel = channel();
    let typed = recovery_ipc::target_with_kind(&channel, Some(&object), Some("score")).unwrap();
    assert!(matches!(typed, StudioTarget::Score { .. }));
    assert!(matches!(
        recovery_ipc::target_with_kind(&channel, Some(&object), None).unwrap(),
        StudioTarget::Flipnote { .. }
    ));
    assert!(recovery_ipc::target_with_kind(&channel, None, Some("score")).is_err());
    for bad in ["", "Score", "index", "audio"] {
        assert!(recovery_ipc::target_with_kind(&channel, Some(&object), Some(bad)).is_err());
    }
    let body = String::from_utf8(
        ScoreOp::SetCell {
            step: 0,
            row: 0,
            value: ScoreCell::Drum { pad: 0 },
        }
        .encode()
        .unwrap(),
    )
    .unwrap();
    assert!(apply_request_with_kind(
        &channel,
        Some(&object),
        Some("score"),
        &hex::encode([1; 16]),
        &hex::encode([2; 16]),
        body.clone()
    )
    .is_ok());
    assert!(apply_request_with_kind(
        &channel,
        Some(&object),
        None,
        &hex::encode([1; 16]),
        &hex::encode([2; 16]),
        body
    )
    .is_err());
    let value = hex::encode([3; 32]);
    for choice in [
        json!({"kind":"scoreCell","step":255,"row":24,"value":value}),
        json!({"kind":"scorePatch","patch":value,"value":value}),
        json!({"kind":"scoreHeader","field":"steps","value":value}),
    ] {
        serde_json::from_value::<recovery_ipc::ChoiceInput>(choice)
            .unwrap()
            .checked()
            .unwrap();
    }
    for bad in [
        json!({"kind":"scoreCell","step":256,"row":0,"value":value}),
        json!({"kind":"scoreCell","step":0,"row":25,"value":value}),
        json!({"kind":"scoreCell","step":0,"row":0,"value":value,"author":value}),
        json!({"kind":"scoreHeader","field":"fps","value":value}),
        json!({"kind":"scorePatch","patch":"AB".repeat(32),"value":value}),
    ] {
        assert!(serde_json::from_value::<recovery_ipc::ChoiceInput>(bad)
            .map_err(|e| e.to_string())
            .and_then(|c| c.checked())
            .is_err());
    }
}

#[tokio::test]
async fn native_score_create_edit_and_wrong_family_refusal_use_real_custody() {
    let (_root, state, actor, task, drain) = fixture().await;
    let channel_id = channel_id(&channel()).unwrap();
    let created = invoke(
        &state,
        7,
        StudioRequest::CreateScore {
            channel: channel_id,
            object: [7; 16],
            nonce: [9; 16],
            title: "authored music".into(),
            ts: 123,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(created["content"]["kind"], "score");
    let request = apply_request_with_kind(
        &channel(),
        Some(&hex::encode([7; 16])),
        Some("score"),
        created["epochId"].as_str().unwrap(),
        &hex::encode([8; 16]),
        String::from_utf8(
            ScoreOp::SetCell {
                step: 0,
                row: 0,
                value: ScoreCell::Drum { pad: 4 },
            }
            .encode()
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let saved = invoke(&state, 7, request).await.unwrap().unwrap();
    assert_eq!(
        saved["content"]["cells"]["0:0"]["register"]["selected"]["value"]["pad"],
        4
    );
    assert!(invoke(&state, 7, StudioRequest::Read { target: target() })
        .await
        .is_err());
    assert_eq!(
        invoke(
            &state,
            7,
            StudioRequest::Read {
                target: StudioTarget::Score {
                    channel: channel_id,
                    object: [7; 16]
                }
            }
        )
        .await
        .unwrap()
        .unwrap(),
        saved
    );
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn native_studio_cancelled_lease_releases_native_fences_without_a_save() {
    let (_root, state, actor, task, drain) = fixture().await;
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    let ready = actor.studio_begin(create()).await.unwrap();
    let (slot, signal) = claim_internal_inline_download(&state).unwrap();
    let cancellation = RequestCancellation::new(signal, Some(slot.request_keepalive()));
    let lease = authorize(&state, 7, 1, generation)
        .unwrap()
        .with_cancellation(cancellation);
    // Native cancellation is the same live signal the actor now owns, not just a reply filter.
    cancel_all_inline_downloads(&state);
    assert!(ready.execute(lease).await.is_err());
    assert!(state.store.try_lock().is_ok());
    assert!(state.servers.try_lock().is_ok());
    assert!(state.ui_session_commit.try_lock().is_ok());
    assert!(state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_server(7)
        .is_err());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

struct Pause {
    armed: AtomicBool,
    entered: StdMutex<Option<tokio::sync::oneshot::Sender<()>>>,
    released: StdMutex<bool>,
    wake: std::sync::Condvar,
}
struct PausingRng {
    inner: ChaCha20Rng,
    pause: Arc<Pause>,
}
// Never strand a blocking worker if an assertion fails while it is deliberately paused.
struct ReleaseOnDrop(Arc<Pause>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}
impl PausingRng {
    fn checkpoint(&self) {
        if self.pause.armed.swap(false, Ordering::AcqRel) {
            self.pause
                .entered
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            let mut released = self.pause.released.lock().unwrap();
            while !*released {
                released = self.pause.wake.wait(released).unwrap();
            }
        }
    }
}
impl rand_core::CryptoRng for PausingRng {}
impl rand_core::RngCore for PausingRng {
    fn next_u32(&mut self) -> u32 {
        self.checkpoint();
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.checkpoint();
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, out: &mut [u8]) {
        self.checkpoint();
        self.inner.fill_bytes(out)
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(out);
        Ok(())
    }
}

#[tokio::test]
async fn native_studio_running_worker_retains_fences_after_invoke_or_actor_abort() {
    for abort_actor in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::default());
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"studio-native", &mut rng()).unwrap());
        *state.session_resumable.lock().await = true;
        let (entered, entry) = tokio::sync::oneshot::channel();
        let pause = Arc::new(Pause {
            armed: AtomicBool::new(false),
            entered: StdMutex::new(Some(entered)),
            released: StdMutex::new(false),
            wake: std::sync::Condvar::new(),
        });
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            PausingRng {
                inner: rng(),
                pause: pause.clone(),
            },
            Box::new(ManualClock::new(123)),
            "alice",
        )
        .unwrap();
        let group = server.group_id();
        let device = server.device_id();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        install(&state, actor.clone(), group.clone(), device, 1).await;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        let ready = actor.studio_begin(create()).await.unwrap();
        let lease = authorize(&state, 7, 1, generation).unwrap();
        pause.armed.store(true, Ordering::Release);
        let _release_on_drop = ReleaseOnDrop(pause.clone());
        let invoke = tokio::spawn(ready.execute(lease));
        entry.await.unwrap(); // Real worker paused at the first vault seal's injected RNG.
        invoke.abort();
        assert!(invoke.await.unwrap_err().is_cancelled());
        if abort_actor {
            task.abort();
        }
        state.session_lock_requested.store(true, Ordering::Release);
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        // Neither lock completion nor leave/reinstall nor another snapshot can pass its fence.
        assert!(state.store.try_lock().is_err());
        assert!(state.ui_session_commit.try_lock().is_err());
        assert!(state.servers.try_lock().is_err());
        assert!(persist_lock_for(&state, 7).try_lock_owned().is_err());
        *pause.released.lock().unwrap() = true;
        pause.wake.notify_all();
        let snapshot = state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server(7)
            .unwrap();
        if abort_actor {
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            actor.shutdown().await;
            task.await.unwrap();
        }
        drain.await.unwrap();
        // After actor abort, its detached worker can still be dropping the remaining lease
        // fields when the store guard becomes available. Wait for actual fence release;
        // the assertions above already prove they remained held while the worker was paused.
        drop(
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let commit = state.ui_session_commit.lock().await;
                let servers = state.servers.lock().await;
                (commit, servers)
            })
            .await
            .expect("completed native worker did not release its fences"),
        );
        // A cancelled response is never shown, but an already-started save may finish. Reopen
        // the actual saved source with the durably saved MLS snapshot and verify the outcome.
        let server = Server::restore(
            &snapshot,
            Hub::new().join(PeerId::from_u64(2)),
            rng(),
            Box::new(ManualClock::new(999)),
            "alice",
        )
        .unwrap();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        install(&state, actor.clone(), group, device, 2).await;
        state.session_lock_requested.store(false, Ordering::Release);
        let reopened = invoke_read(&state).await;
        assert_eq!(
            reopened["content"]["title"]["selected"]["value"],
            "moon cat"
        );
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    }
}
async fn invoke_read(state: &AppState) -> Value {
    invoke(state, 7, StudioRequest::Read { target: target() })
        .await
        .unwrap()
        .unwrap()
}
