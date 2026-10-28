//! Batch acceptance through the actual native parser, Ready/lease custody and sealed store.
//! Counts and exact original operations are checked independently of rendered projection.
use super::*;
use catcoms_replication::DomainOp;

fn score_target() -> StudioTarget {
    StudioTarget::Score {
        channel: channel_id(&channel()).unwrap(),
        object: [7; 16],
    }
}
fn note(step: u16, pad: u8) -> ScoreOp {
    ScoreOp::SetNote {
        step,
        row: 0,
        value: ScoreCell::Drum { pad },
    }
}
fn batch(epoch: &str, operations: &[(u8, ScoreOp)]) -> StudioRequest {
    apply_batch_request(
        &channel(),
        &hex::encode([7; 16]),
        Some("score"),
        epoch,
        operations
            .iter()
            .map(|(nonce, op)| StudioBatchOperationArgs {
                nonce: hex::encode([*nonce; 16]),
                body: String::from_utf8(op.encode().unwrap()).unwrap(),
            })
            .collect(),
    )
    .unwrap()
}
async fn create_score(state: &AppState) -> Value {
    invoke(
        state,
        7,
        StudioRequest::CreateScore {
            channel: score_target().channel(),
            object: [7; 16],
            nonce: [9; 16],
            title: "native batch".into(),
            ts: 123,
        },
    )
    .await
    .unwrap()
    .unwrap()
}
async fn read_score(state: &AppState) -> Value {
    invoke(
        state,
        7,
        StudioRequest::Read {
            target: score_target(),
        },
    )
    .await
    .unwrap()
    .unwrap()
}

/// Read the actual persisted signed ledger using the saved MLS identity. This independent
/// read grants no write/receipt authority and cannot mistake identical projections for retries.
async fn source(state: &AppState, accepted: &[(u8, ScoreOp)], absent: &[(u8, ScoreOp)]) -> usize {
    let guard = state.store.lock().await;
    let store = guard.as_ref().unwrap();
    let snapshot = store.load_server(7).unwrap();
    let mut sync = catcoms_sync::ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(91)),
        rng(),
        Box::new(ManualClock::new(999)),
    )
    .unwrap();
    sync.with_registry_context(|group, device, _, _| {
        let unit = store
            .load_studio_epoch(7, group, score_target(), device)
            .unwrap()
            .unwrap();
        let document = score_target().document(&group.group_id()).unwrap();
        for (operations, present) in [(accepted, true), (absent, false)] {
            for (nonce, body) in operations {
                let operation = DomainOp {
                    nonce: [*nonce; 16],
                    doc_type: document.doc_type,
                    logical_key: document.logical_key.clone(),
                    body: body.encode().unwrap(),
                };
                assert_eq!(
                    unit.contains_exact_operation(device.device_id(), &operation)
                        .unwrap(),
                    present,
                    "original signed operation {nonce}, expected presence {present}"
                );
            }
        }
        unit.op_count()
    })
}

async fn reopen(
    root: &std::path::Path,
    state: &Arc<AppState>,
    actor: ServerActor,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
) -> (
    ServerActor,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    let (group, device) = {
        let entries = state.servers.lock().await;
        (entries[&7].group_id.clone(), entries[&7].device_id)
    };
    state.servers.lock().await.clear();
    drop(state.store.lock().await.take());
    *state.store.lock().await =
        Some(ServerStore::open(root, b"studio-native", &mut rng()).unwrap());
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
    attach_blob_store(state, &mut server).await;
    let (actor, mut events, task) = spawn(server);
    let state_for_events = state.clone();
    let drain = tokio::spawn(async move {
        while events.recv().await.is_some() {
            let _ = require_unlocked_session(&state_for_events).await;
        }
    });
    install(state, actor.clone(), group, device, 2).await;
    (actor, task, drain)
}

#[tokio::test]
async fn native_batch_reopen_exact_retry_and_wrong_epoch_preserve_signed_union() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let (root, state, actor, task, drain) = fixture().await;
        let created = create_score(&state).await;
        let epoch = created["epochId"].as_str().unwrap();
        let ops = [(21, note(0, 3)), (22, note(1, 4))];
        let initial = source(&state, &[], &ops).await;
        let saved = invoke(&state, 7, batch(epoch, &ops))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(source(&state, &ops, &[]).await, initial + 2);
        let error = invoke(
            &state,
            7,
            batch(&hex::encode([99; 16]), &[(23, note(2, 5))]),
        )
        .await
        .unwrap_err();
        assert!(error.contains("edit belongs to a retired epoch"), "{error}");
        assert_eq!(
            source(&state, &ops, &[(23, note(2, 5))]).await,
            initial + 2,
            "wrong concrete epoch identity admits nothing"
        );
        let (actor, task, drain) = reopen(root.path(), &state, actor, task, drain).await;
        assert_eq!(
            invoke(&state, 7, batch(epoch, &ops))
                .await
                .unwrap()
                .unwrap(),
            saved
        );
        assert_eq!(
            source(&state, &ops, &[]).await,
            initial + 2,
            "reopen never re-applies exact nonces"
        );
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    })
    .await
    .expect("native batch acceptance exceeded its watchdog");
}

#[tokio::test]
async fn native_batch_causal_refusal_keeps_only_original_durable_prefix_after_reopen() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let (root, state, actor, task, drain) = fixture().await;
        let created = create_score(&state).await;
        let epoch = created["epochId"].as_str().unwrap();
        // Item2 is valid grammar but outside this document's authored16 steps. No fake
        // authorization or I/O success flag can manufacture the durable-prefix evidence.
        let ops = [(31, note(0, 3)), (32, note(20, 4)), (33, note(1, 5))];
        let initial = source(&state, &[], &ops).await;
        let error = invoke(&state, 7, batch(epoch, &ops)).await.unwrap_err();
        assert!(error.contains("item 2 of 3"), "{error}");
        assert_eq!(source(&state, &ops[..1], &ops[1..]).await, initial + 1);
        let prefix = read_score(&state).await;
        let (actor, task, drain) = reopen(root.path(), &state, actor, task, drain).await;
        let retry = invoke(&state, 7, batch(epoch, &ops)).await.unwrap_err();
        assert!(retry.contains("item 2 of 3"), "{retry}");
        assert_eq!(read_score(&state).await, prefix);
        assert_eq!(
            source(&state, &ops[..1], &ops[1..]).await,
            initial + 1,
            "exact prefix retry cannot duplicate signed history or admit the tail"
        );
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    })
    .await
    .expect("native batch prefix acceptance exceeded its watchdog");
}

#[tokio::test]
async fn native_batch_original_ready_refuses_lock_and_replacement_before_admission() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        // Fresh fixtures isolate the fences: replacement must not pass because an earlier
        // test already changed the generation. These are actual Ready and native guards.
        for lock in [false, true] {
            let (_root, state, actor, task, drain) = fixture().await;
            let created = create_score(&state).await;
            let ops = [(41, note(0, 3))];
            let initial = source(&state, &[], &ops).await;
            let generation = unlocked_ui_session_generation(&state).await.unwrap();
            let ready = actor
                .studio_begin(batch(created["epochId"].as_str().unwrap(), &ops))
                .await
                .unwrap();
            if lock {
                state.session_lock_requested.store(true, Ordering::Release);
                state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
            } else {
                state.servers.lock().await.get_mut(&7).unwrap().instance = 2;
            }
            let error = authorize(&state, 7, 1, generation).unwrap_err();
            assert_eq!(
                error,
                if lock {
                    "Studio request belongs to a locked or changed UI session"
                } else {
                    "server changed before Studio commit"
                }
            );
            drop(ready);
            assert_eq!(
                actor.member_count().await,
                1,
                "refusal leaves the actor progressing"
            );
            assert_eq!(source(&state, &[], &ops).await, initial);
            assert!(state.ui_session_commit.try_lock().is_ok());
            assert!(persist_lock_for(&state, 7).try_lock_owned().is_ok());
            actor.shutdown().await;
            task.await.unwrap();
            drain.await.unwrap();
        }
    })
    .await
    .expect("native batch admission acceptance exceeded its watchdog");
}

#[tokio::test]
async fn native_batch_lost_invoke_after_worker_admission_retains_fences_and_exact_retry() {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
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
        let mut server = Server::found(
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
        attach_blob_store(&state, &mut server).await;
        let (group, device) = (server.group_id(), server.device_id());
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        install(&state, actor.clone(), group, device, 1).await;
        let created = create_score(&state).await;
        let epoch = created["epochId"].as_str().unwrap().to_owned();
        let ops = [(51, note(0, 3)), (52, note(1, 4))];
        let initial = source(&state, &[], &ops).await;
        let original_generation = unlocked_ui_session_generation(&state).await.unwrap();
        pause.armed.store(true, Ordering::Release);
        let _release_on_drop = ReleaseOnDrop(pause.clone());
        let request = batch(&epoch, &ops);
        let worker_state = state.clone();
        let invocation = tokio::spawn(async move { invoke(&worker_state, 7, request).await });
        // Actual vault sealing after native lease transfer, before the first batch item.
        // This proves admitted-worker cancellation; the separate causal test proves a
        // durable partial prefix. Neither claim substitutes for the other.
        entry.await.unwrap();
        invocation.abort();
        assert!(invocation.await.unwrap_err().is_cancelled());
        state.session_lock_requested.store(true, Ordering::Release);
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        assert!(
            state.ui_session_commit.try_lock().is_err(),
            "Lock cannot finish before this admitted worker"
        );
        assert!(state.store.try_lock().is_err());
        assert!(state.servers.try_lock().is_err());
        assert!(persist_lock_for(&state, 7).try_lock_owned().is_err());
        *pause.released.lock().unwrap() = true;
        pause.wake.notify_all();
        // Wait for the actual worker's full native fence return, not an arbitrary delay or
        // a discarded reply. The work may finish even though its invocation was cancelled.
        {
            let _commit = state.ui_session_commit.lock().await;
            let _store = state.store.lock().await;
            let _servers = state.servers.lock().await;
        }
        let (actor, task, drain) = reopen(root.path(), &state, actor, task, drain).await;
        assert!(require_ui_session_generation(&state, original_generation)
            .await
            .is_err());
        {
            let _commit = state.ui_session_commit.lock().await;
            state.session_lock_requested.store(false, Ordering::Release);
            // Model successful authenticated unlock of the reopened test vault. Clearing
            // the request alone must not authorize a newer generation or revive old work.
            assert!(ui_session_closed(&state));
            let current = state.ui_session_generation.load(Ordering::Acquire);
            state
                .ui_authorized_generation
                .store(current, Ordering::Release);
        }
        assert_eq!(source(&state, &ops, &[]).await, initial + 2);
        let saved = read_score(&state).await;
        assert_eq!(
            invoke(&state, 7, batch(&epoch, &ops))
                .await
                .unwrap()
                .unwrap(),
            saved
        );
        assert_eq!(source(&state, &ops, &[]).await, initial + 2);
        actor.shutdown().await;
        task.await.unwrap();
        drain.await.unwrap();
    })
    .await
    .expect("native batch cancellation acceptance exceeded its watchdog");
}
