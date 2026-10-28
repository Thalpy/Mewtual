//! Real StudioSession -> native command adapters -> sealed actor/vault -> second peer readback.
//! Node is provisioned by native CI; the root Cargo workspace has no Node prerequisite.
use super::*;
use catcoms_rt::Clock;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

async fn dispatch(state: &AppState, cmd: &str, args: &Value) -> Result<Value, String> {
    let text = |key: &str| args[key].as_str().ok_or_else(|| format!("missing {key}"));
    let server = args["server"].as_u64().ok_or("missing server")?;
    let target = || {
        recovery_ipc::target_with_kind(
            text("channel")?,
            args["object"].as_str(),
            args["objectKind"].as_str(),
        )
    };
    match cmd {
        "studio_list" => Ok(invoke(
            state,
            server,
            StudioRequest::Read {
                target: StudioTarget::Index {
                    channel: channel_id(text("channel")?)?,
                },
            },
        )
        .await?
        .unwrap_or(Value::Null)),
        "studio_read" => Ok(
            invoke(state, server, StudioRequest::Read { target: target()? })
                .await?
                .unwrap_or(Value::Null),
        ),
        "studio_apply" => Ok(invoke(
            state,
            server,
            apply_request_with_kind(
                text("channel")?,
                Some(text("object")?),
                args["objectKind"].as_str(),
                text("epochId")?,
                text("nonce")?,
                text("body")?.into(),
            )?,
        )
        .await?
        .unwrap_or(Value::Null)),
        "studio_overlay_lifecycle" => {
            recovery_ipc::invoke_control(
                state,
                server,
                target()?,
                app::studio::StudioControlAction::OverlayLifecycle,
            )
            .await
        }
        "studio_overlay_read" => super::super::inspection::read(state, server, target()?).await,
        "publish_pixa" => serde_json::to_value(
            crate::studio_exports::publish_pixa_with_state(state, server, text("bytesB64")?.into())
                .await?,
        )
        .map_err(|e| e.to_string()),
        "request_blob_bounded" => serde_json::to_value(
            crate::creative_blobs::request_blob_bounded_with_state(
                state,
                server,
                text("cid")?.into(),
                usize::try_from(args["maxBytes"].as_u64().ok_or("missing maxBytes")?)
                    .map_err(|e| e.to_string())?,
            )
            .await?,
        )
        .map_err(|e| e.to_string()),
        _ => Err(format!("unsupported acceptance invoke: {cmd}")),
    }
}

async fn session(state: &AppState, init: Value) -> Value {
    let desktop = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut child = Command::new("node")
        .current_dir(desktop)
        .args([
            "--experimental-strip-types",
            "--import",
            "./scripts/test-dom.mjs",
            "./scripts/studio-native-export-runner.mjs",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("native acceptance requires Node 22 and installed frontend dependencies");
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let work = async {
        input
            .write_all(format!("{init}\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        // Bound both IPC work and one child line. The child has no network listener or app
        // authority outside this private pipe, and unknown commands never reach a native API.
        for _ in 0..96 {
            let mut line = Vec::new();
            (&mut output)
                .take(16 * 1024 * 1024)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|e| e.to_string())?;
            if line.last() != Some(&b'\n') {
                return Err("child exited or exceeded acceptance line cap".into());
            }
            let request: Value = serde_json::from_slice(&line).map_err(|e| e.to_string())?;
            if request["done"] == true {
                return Ok(request["result"].clone());
            }
            let response = match dispatch(
                state,
                request["cmd"].as_str().ok_or("missing command")?,
                &request["args"],
            )
            .await
            {
                Ok(value) => json!({"id":request["id"],"value":value}),
                Err(error) => json!({"id":request["id"],"error":error}),
            };
            input
                .write_all(format!("{response}\n").as_bytes())
                .await
                .map_err(|e| e.to_string())?;
        }
        Err("acceptance invoke count exceeded".into())
    };
    let result: Result<Result<Value, String>, _> =
        tokio::time::timeout(Duration::from_secs(60), work).await;
    // Reap on failure as well as timeout, before assertions unwind. kill_on_drop additionally
    // covers cancellation of this parent test while an actual native invoke is outstanding.
    if !matches!(&result, Ok(Ok(_))) {
        let _ = child.start_kill();
    }
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    if status.is_err() {
        let _ = child.start_kill();
        // Reaping must not turn the test's failure path into an unbounded wait. On an OS
        // failure to terminate, kill_on_drop remains a final best-effort cleanup safeguard.
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    }
    let result = result
        .expect("bounded native/Node export session")
        .expect("native/Node export session");
    assert!(
        status.unwrap().unwrap().success(),
        "Node acceptance assertions failed"
    );
    result
}

#[tokio::test]
async fn native_media_shared_helpers_reject_bad_input_and_lock_without_retaining_slots() {
    let (_root, state, actor, task, drain) = fixture().await;
    assert!(
        crate::studio_exports::publish_pixa_with_state(&state, 7, "not-base64".into())
            .await
            .is_err()
    );
    assert!(crate::creative_blobs::request_blob_bounded_with_state(
        &state,
        7,
        "00".repeat(32),
        app::creative::MAX_BOUNDED_BLOB_BYTES + 1
    )
    .await
    .is_err());
    assert!(state.inline_downloads.lock().unwrap().is_empty());
    // A valid package must succeed before locking: invalid bytes would reject even if the
    // extracted publication helper accidentally stopped enforcing its session check.
    let vectors: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json"
    )))
    .unwrap();
    let valid = B64.encode(hex::decode(vectors["pixaHex"].as_str().unwrap()).unwrap());
    crate::studio_exports::publish_pixa_with_state(&state, 7, valid.clone())
        .await
        .unwrap();
    state.session_lock_requested.store(true, Ordering::Release);
    assert!(
        crate::studio_exports::publish_pixa_with_state(&state, 7, valid)
            .await
            .is_err()
    );
    assert!(
        crate::creative_blobs::request_blob_bounded_with_state(&state, 7, "00".repeat(32), 1)
            .await
            .is_err()
    );
    assert!(state.inline_downloads.lock().unwrap().is_empty());
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}

#[tokio::test]
async fn gate7_reopened_native_composition_exports_and_other_peer_reads_actual_pixa() {
    let (root, state, actor, task, drain) = fixture().await;
    let created = invoke(&state, 7, create()).await.unwrap().unwrap();
    let score = StudioTarget::Score {
        channel: target().channel(),
        object: [6; 16],
    };
    let music = invoke(
        &state,
        7,
        StudioRequest::CreateScore {
            channel: score.channel(),
            object: [6; 16],
            nonce: [30; 16],
            title: "exported music".into(),
            ts: 123,
        },
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
    let pixels =
        app::creative::validate_pixa(&hex::decode(vectors["pixaHex"].as_str().unwrap()).unwrap())
            .unwrap()
            .frames()[0]
            .to_vec();
    let mut second = pixels.clone();
    let last = second.len() - 1;
    second[last] = 1; // Same valid raster framing, visibly distinct palette index.
    let frames = [pixels, second];
    for (nonce, op) in [
        (
            31,
            ScoreOp::SetPatch {
                patch,
                descriptor: descriptor.clone(),
            },
        ),
        (
            32,
            ScoreOp::SetCell {
                step: 1,
                row: 2,
                value: ScoreCell::Note {
                    patch,
                    len: Some(2),
                    volume: None,
                },
            },
        ),
    ] {
        invoke(
            &state,
            7,
            StudioRequest::Apply {
                target: score,
                epoch_id: u128::from_str_radix(music["epochId"].as_str().unwrap(), 16).unwrap(),
                nonce: [nonce; 16],
                body: op.encode().unwrap(),
            },
        )
        .await
        .unwrap();
    }
    for (index, pix) in frames.iter().enumerate() {
        let cid = Cid::from_hex(&actor.publish_pix(pix.clone(), None).await.unwrap().cid).unwrap();
        invoke(
            &state,
            7,
            StudioRequest::Apply {
                target: target(),
                epoch_id: u128::from_str_radix(created["epochId"].as_str().unwrap(), 16).unwrap(),
                nonce: [40 + index as u8; 16],
                body: FlipnoteOp::InsertFrame {
                    frame: [1 + index as u8; 16],
                    after: (index == 1).then_some([1; 16]),
                    cid: *cid.as_bytes(),
                    bytes: pix.len() as u64,
                }
                .encode()
                .unwrap(),
            },
        )
        .await
        .unwrap();
    }
    for (nonce, op) in [
        (42, FlipnoteOp::SetPatch { patch, descriptor }),
        (
            43,
            FlipnoteOp::SetSfx {
                sfx: [3; 16],
                frame: [2; 16],
                patch,
                note: 72,
                duration_ms: 375,
            },
        ),
        (
            44,
            FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([6; 16]))),
        ),
        (45, FlipnoteOp::SetHeader(FlipnoteHeader::Fps(17))),
    ] {
        invoke(
            &state,
            7,
            StudioRequest::Apply {
                target: target(),
                epoch_id: u128::from_str_radix(created["epochId"].as_str().unwrap(), 16).unwrap(),
                nonce: [nonce; 16],
                body: op.encode().unwrap(),
            },
        )
        .await
        .unwrap();
    }
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
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
    let hub = Hub::new();
    let mut author = Server::restore(
        &snapshot,
        hub.join(PeerId::from_u64(10)),
        rng(),
        Box::new(catcoms_rt::SystemClock),
        "alice",
    )
    .unwrap();
    author.subscribe_control().await.unwrap();
    let invite = author.mint_invite([46; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Server::join(
            hub.join(PeerId::from_u64(11)),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(catcoms_rt::SystemClock),
            "bob",
            author.local_peer(),
            &invite
        ),
        author.sync_once()
    );
    tick.unwrap();
    let mut bob = joined.unwrap();
    // Authenticate the provider before actor-owned Studio page discovery; this uses the same
    // channel-index request as product admission, not a test-installed document or receipt.
    let (proof, tick) = tokio::join!(
        bob.request_channel_index_catchup(author.local_peer()),
        author.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    attach_blob_store(&state, &mut author).await;
    let group = author.group_id();
    let exporter = author.device_id();
    let (alice, mut events, a_task) = spawn(author);
    let a_drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    install(&state, alice.clone(), group.clone(), exporter, 2).await;
    let metadata = json!({"server":7,"channel":channel(),"object":hex::encode([7;16]),
        "group":hex::encode(&group),"exporter":exporter.to_string(),"recipe":vectors["patch"],
        "frames":frames.iter().map(|p| B64.encode(p)).collect::<Vec<_>>()});
    let mut init = metadata.clone();
    init["mode"] = "publish".into();
    init["me"] = exporter.to_string().into();
    let published = session(&state, init).await;
    let expected = invoke_read(&state).await;
    let record = expected["content"]["sound"]["records"]
        .as_object()
        .unwrap()
        .values()
        .map(|r| &r["selected"]["value"])
        .find(|op| op["op"] == "set_export")
        .unwrap();
    let package_cid = Cid::from_hex(record["cid"].as_str().unwrap()).unwrap();
    let b_root = tempfile::tempdir().unwrap();
    let b_state = Arc::new(AppState::default());
    *b_state.store.lock().await =
        Some(ServerStore::open(b_root.path(), b"studio-native", &mut rng()).unwrap());
    *b_state.session_resumable.lock().await = true;
    attach_blob_store(&b_state, &mut bob).await;
    let device = bob.device_id();
    let (bob, mut events, b_task) = spawn(bob);
    let b_drain = tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            assert!(!matches!(e.event, AppEvent::StudioReceivePaused));
        }
    });
    install(&b_state, bob.clone(), group.clone(), device, 1).await;
    for selected in [target(), score] {
        invoke(&b_state, 7, StudioRequest::Read { target: selected })
            .await
            .unwrap();
    }
    // Real time pacing here prevents synthetic deadlines outrunning detached CPU/network work.
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            receive_once(&state, 7, 2, &alice).await.unwrap();
            receive_once(&b_state, 7, 1, &bob).await.unwrap();
            if invoke(&b_state, 7, StudioRequest::Read { target: target() })
                .await
                .unwrap()
                .as_ref()
                == Some(&expected)
            {
                break;
            }
            catcoms_rt::SystemClock
                .sleep(Duration::from_millis(100))
                .await;
        }
    })
    .await
    .expect("cold peer receives generated export reference");
    assert!(!b_state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .blob_store(&hex::encode(&group))
        .unwrap()
        .has(&package_cid));
    let mut init = metadata;
    init["mode"] = "read".into();
    init["me"] = device.to_string().into();
    init["exportId"] = published["exportId"].clone();
    let received = session(&b_state, init).await;
    assert_eq!(received["bytesB64"], published["bytesB64"]);
    alice.shutdown().await;
    bob.shutdown().await;
    a_task.await.unwrap();
    b_task.await.unwrap();
    a_drain.await.unwrap();
    b_drain.await.unwrap();
}
