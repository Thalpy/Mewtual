use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::future::Future;
use std::task::{Context, Poll, Waker};

type Node = Server<MemNetwork, ChaCha20Rng>;
fn capture(state: &AppState, server: u64) -> Result<Captured, &'static str> {
    super::capture(
        state,
        server,
        state.ui_session_generation.load(Ordering::SeqCst),
    )
}
struct Running {
    state: Arc<AppState>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    // Drop the actual mounted state/store before attempting Windows temp-directory cleanup.
    _root: tempfile::TempDir,
}
impl Drop for Running {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
async fn mounted() -> Running {
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::default());
    *state.store.lock().await =
        Some((ServerStore::open(root.path(), b"native ledger", &mut OsCryptoRng).unwrap()).into());
    *state.session_resumable.lock().await = true;
    finalize_unlock_session(&state, 0, false).await.unwrap();
    Running {
        state,
        tasks: Vec::new(),
        _root: root,
    }
}
async fn owner(peer: u64) -> Node {
    let mut node = Server::found(
        Hub::new().join(PeerId::from_u64(peer)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(peer),
        Box::new(ManualClock::new(1000)),
        "native owner",
    )
    .unwrap();
    node.subscribe_control().await.unwrap();
    node.open_channel(1).await.unwrap();
    node
}
fn install(
    state: &AppState,
    id: u64,
    instance: u64,
    node: Node,
) -> Vec<tokio::task::JoinHandle<()>> {
    let group = node.group_id();
    let device = node.device_id();
    let (actor, mut events, task) = spawn(node);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let mut servers = state.servers.try_lock().unwrap();
    if let Some(old) = servers.remove(&id) {
        old.files.retire();
    }
    servers.insert(
        id,
        crate::tests::snapshot_test_entry(instance, actor, group, device),
    );
    vec![task, drain]
}
fn value(reply: dto::Reply) -> serde_json::Value {
    serde_json::from_str(&dto::encode(reply)).unwrap()
}
async fn read_page(
    state: &AppState,
    captured: &Captured,
    cursor: Option<String>,
    limit: usize,
) -> serde_json::Value {
    for _ in 0..128 {
        let reply = value(
            execute(
                state,
                captured,
                Request::Page {
                    server: captured.server,
                    cursor: cursor.clone(),
                    limit,
                },
            )
            .await,
        );
        if reply["code"] != "busy" {
            return reply;
        }
    }
    panic!("finite real Actor read did not complete");
}
fn poll_once<F: Future>(future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[tokio::test]
async fn real_actor_retained_originals_have_exact_pages_details_and_actual_stale_reset() {
    let mut running = mounted().await;
    let mut node = owner(880).await;
    for i in 0..40 {
        node.send_message(1, &format!("native original {i}"))
            .await
            .unwrap();
    }
    running.tasks.extend(install(&running.state, 7, 1, node));
    let captured = capture(&running.state, 7).unwrap();
    let first = read_page(&running.state, &captured, None, 32).await;
    assert_eq!(first["status"], "page");
    assert_eq!(first["rows"].as_array().unwrap().len(), 32);
    assert_eq!(first["knownRowCount"], 40);
    assert_eq!(first["coverage"]["unattributedChanges"], "unknown");
    assert_eq!(first["availability"], "incomplete");
    let next = read_page(
        &running.state,
        &captured,
        Some(first["nextCursor"].as_str().unwrap().into()),
        32,
    )
    .await;
    assert_eq!(next["revision"], first["revision"]);
    assert_eq!(next["rows"].as_array().unwrap().len(), 8);
    assert!(next["nextCursor"].is_null());
    let row = first["rows"][0]["rowId"].as_str().unwrap().to_owned();
    let revision = first["revision"].as_str().unwrap().to_owned();
    let detail = value(
        execute(
            &running.state,
            &captured,
            Request::Detail {
                server: 7,
                row: row.clone(),
                revision: revision.clone(),
                offset: 0,
            },
        )
        .await,
    );
    assert_eq!(detail["status"], "detail");
    assert_eq!(detail["row"], first["rows"][0]);
    let original = hex::decode(detail["original"]["bytes"].as_str().unwrap()).unwrap();
    let signed = catcoms_replication::op::SignedOp::decode(&original).unwrap();
    assert!(signed.verify());
    assert_eq!(detail["signatureHex"], hex::encode(signed.signature));
    let refreshed = read_page(&running.state, &captured, None, 32).await;
    assert_ne!(refreshed["revision"], first["revision"]);
    let stale = value(
        execute(
            &running.state,
            &captured,
            Request::Detail {
                server: 7,
                row,
                revision,
                offset: 0,
            },
        )
        .await,
    );
    assert_eq!(stale["status"], "stale");
    let actual = captured
        .actor
        .admin_chat_ledger_revision(captured.scope.clone())
        .await
        .unwrap();
    assert_eq!(stale["revision"], actual);
}

#[tokio::test]
async fn real_originals_are_chunked_at_64k_even_when_decoding_is_too_large() {
    let mut running = mounted().await;
    let mut node = owner(888).await;
    node.send_message(1, &"x".repeat(64 * 1024)).await.unwrap();
    running.tasks.extend(install(&running.state, 7, 1, node));
    let captured = capture(&running.state, 7).unwrap();
    let page = read_page(&running.state, &captured, None, 32).await;
    let row = page["rows"][0]["rowId"].as_str().unwrap().to_owned();
    let revision = page["revision"].as_str().unwrap().to_owned();
    let first = value(
        execute(
            &running.state,
            &captured,
            Request::Detail {
                server: 7,
                row: row.clone(),
                revision: revision.clone(),
                offset: 0,
            },
        )
        .await,
    );
    let mut bytes = hex::decode(first["original"]["bytes"].as_str().unwrap()).unwrap();
    assert_eq!(bytes.len(), 64 * 1024);
    assert_eq!(first["legacyChange"]["status"], "too-large");
    let offset = first["original"]["nextOffset"].as_u64().unwrap() as usize;
    assert_eq!(offset, 64 * 1024);
    let last = value(
        execute(
            &running.state,
            &captured,
            Request::Detail {
                server: 7,
                row: row.clone(),
                revision: revision.clone(),
                offset,
            },
        )
        .await,
    );
    bytes.extend(hex::decode(last["original"]["bytes"].as_str().unwrap()).unwrap());
    assert!(last["original"]["nextOffset"].is_null());
    assert_eq!(
        bytes.len() as u64,
        first["original"]["totalBytes"].as_u64().unwrap()
    );
    assert!(catcoms_replication::op::SignedOp::decode(&bytes)
        .unwrap()
        .verify());
    let invalid = value(
        execute(
            &running.state,
            &captured,
            Request::Detail {
                server: 7,
                row,
                revision,
                offset: bytes.len() + 1,
            },
        )
        .await,
    );
    assert_eq!(invalid["code"], "invalid-request");
}

#[tokio::test]
async fn actual_joined_member_and_signed_role_revocation_precede_malformed_detail_metadata() {
    let mut running = mounted().await;
    let hub = Hub::new();
    let clock = ManualClock::new(1000);
    let owner_peer = PeerId::from_u64(889);
    let mut owner = Server::found(
        hub.join(owner_peer),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(889),
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    owner.subscribe_control().await.unwrap();
    owner.open_roles().await.unwrap();
    let invite = owner.mint_invite([23; 16], 60_000, Vec::new()).unwrap();
    let (joined, tick) = tokio::join!(
        Server::join(
            hub.join(PeerId::from_u64(890)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(890),
            Box::new(clock),
            "member",
            owner_peer,
            &invite
        ),
        owner.sync_once()
    );
    tick.unwrap();
    let mut member = joined.unwrap();
    member.open_roles().await.unwrap();
    member.open_channel(1).await.unwrap();
    member
        .send_message(1, "retained native role test")
        .await
        .unwrap();
    let device = member.device_id();
    running.tasks.extend(install(&running.state, 7, 1, member));
    let captured = capture(&running.state, 7).unwrap();
    let malformed = || Request::Detail {
        server: 7,
        row: "bad".into(),
        revision: "bad".into(),
        offset: 0,
    };
    assert_eq!(
        value(execute(&running.state, &captured, malformed()).await)["code"],
        "not-authorized"
    );
    owner.set_admin(&device, true).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if captured
                .actor
                .roles()
                .await
                .get(&device.to_string())
                .is_some_and(|role| role == "admin")
            {
                break;
            }
        }
    })
    .await
    .expect("actual signed admin grant not received");
    let page = read_page(&running.state, &captured, None, 32).await;
    assert_eq!(page["status"], "page");
    owner.set_admin(&device, false).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if captured
                .actor
                .roles()
                .await
                .get(&device.to_string())
                .is_some_and(|role| role == "member")
            {
                break;
            }
        }
    })
    .await
    .expect("actual signed revocation not received");
    assert_eq!(
        value(execute(&running.state, &captured, malformed()).await)["code"],
        "not-authorized"
    );
    assert_eq!(
        value(execute(&running.state, &captured, Request::Invalid { server: 7 }).await)["code"],
        "not-authorized"
    );
}

#[tokio::test]
async fn synchronous_capture_bounds_inflight_and_never_adopts_contended_or_replaced_registry() {
    let mut running = mounted().await;
    running
        .tasks
        .extend(install(&running.state, 7, 1, owner(881).await));
    let held = running.state.servers.lock().await;
    assert!(matches!(capture(&running.state, 7), Err("busy")));
    drop(held);
    let mut tickets = Vec::new();
    for _ in 0..INFLIGHT {
        tickets.push(capture(&running.state, 7).unwrap());
    }
    assert!(matches!(capture(&running.state, 7), Err("busy")));
    let old = tickets.pop().unwrap();
    drop(tickets);
    running
        .tasks
        .extend(install(&running.state, 7, 2, owner(882).await));
    let mut delivered = None;
    publish(
        &running.state,
        &old,
        "privileged original".into(),
        |reply| delivered = Some(reply),
    )
    .await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&delivered.unwrap()).unwrap()["code"],
        "server-superseded"
    );
    let replacement = capture(&running.state, 7).unwrap();
    assert!(!replacement.origin.is_cancelled());
    assert!(old.origin.is_cancelled());
}

#[tokio::test]
async fn cached_native_read_observer_does_not_keep_a_dropped_registry_actor_alive() {
    let running = mounted().await;
    let mut node = owner(893).await;
    node.send_message(1, "released with the real actor")
        .await
        .unwrap();
    let mut tasks = install(&running.state, 7, 1, node);
    let drain = tasks.pop().unwrap();
    let mut actor_task = tasks.pop().unwrap();
    let captured = capture(&running.state, 7).unwrap();
    assert_eq!(
        read_page(&running.state, &captured, None, 32).await["status"],
        "page"
    );
    drop(captured);
    {
        let mut servers = running.state.servers.lock().await;
        let entry = servers.remove(&7).unwrap();
        entry.files.retire(); // real row retirement precedes releasing its registry guard
        drop(entry); // last genuine owner drops; no explicit Actor shutdown is supplied
    }
    assert!(running
        .state
        .admin_chat_ledger
        .active
        .lock()
        .unwrap()
        .is_some());
    let ended = tokio::time::timeout(Duration::from_secs(10), &mut actor_task).await;
    if ended.is_err() {
        actor_task.abort();
        drain.abort();
        panic!("native cached read observer retained the removed actor sender");
    }
    ended.unwrap().unwrap();
    drain.await.unwrap();
    // No lock, Registry::retire or second ledger request was required to end the old Actor.
}

#[tokio::test]
async fn completely_unpolled_original_work_stays_revoked_after_actual_lock_and_unlock() {
    let mut running = mounted().await;
    let mut node = owner(883).await;
    node.send_message(1, "must not be read after lock")
        .await
        .unwrap();
    running.tasks.extend(install(&running.state, 7, 1, node));
    let captured = capture(&running.state, 7).unwrap();
    let entered_generation = captured.generation;
    // This future is deliberately never polled before a real completed native lock barrier.
    let future = execute(
        &running.state,
        &captured,
        Request::Page {
            server: 7,
            cursor: None,
            limit: 32,
        },
    );
    lock_session_inner(&running.state, None).await.unwrap();
    let fresh = running.state.ui_session_generation.load(Ordering::SeqCst);
    finalize_unlock_session(&running.state, fresh, false)
        .await
        .unwrap();
    assert_eq!(value(future.await)["code"], "session-unavailable");
    assert!(matches!(
        super::capture(&running.state, 7, entered_generation),
        Err("session-unavailable")
    ));
    assert_eq!(running.state.admin_chat_ledger.budget.charged_bytes(), 0);
    assert!(!capture(&running.state, 7).unwrap().origin.is_cancelled());
}

#[tokio::test]
async fn retiring_queued_reads_frees_all_global_permits_before_an_old_actor_ready_releases() {
    let mut running = mounted().await;
    running
        .tasks
        .extend(install(&running.state, 7, 1, owner(891).await));
    let mut healthy = owner(892).await;
    healthy
        .send_message(1, "another group remains readable")
        .await
        .unwrap();
    running.tasks.extend(install(&running.state, 8, 2, healthy));
    let actor = running.state.servers.lock().await[&7].actor.clone();
    let expected_context = actor.durable_send_context().await.unwrap();
    // The REAL actor is parked in its production Ready/lease barrier. No mock receiver or
    // scheduling delay stands in for the command queue that all eight reads wait behind.
    let ready = actor
        .prepare_durable_send(catcoms_app::durable_chat::DurableSendRequest {
            token: [93; 16],
            expected_context,
            channel: 1,
            text: "held without native save custody".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap();
    let mut queued = Vec::new();
    for _ in 0..INFLIGHT {
        let captured = capture(&running.state, 7).unwrap();
        let state = running.state.clone();
        let mut future = Box::pin(async move {
            let reply = execute(
                &state,
                &captured,
                Request::Page {
                    server: 7,
                    cursor: None,
                    limit: 32,
                },
            )
            .await;
            let mut delivered = None;
            publish(&state, &captured, dto::encode(reply), |reply| {
                delivered = Some(reply)
            })
            .await;
            drop(captured); // exact production permit lifetime, including final publication
            serde_json::from_str::<serde_json::Value>(&delivered.unwrap()).unwrap()
        });
        assert!(poll_once(future.as_mut()).is_pending());
        queued.push(future);
    }
    assert_eq!(
        running.state.admin_chat_ledger.inflight.available_permits(),
        0
    );
    // The real first capture for another pane must retire the old native scope even while the
    // global quota is full. It returns Busy only until those cancelled reads release custody.
    assert!(matches!(capture(&running.state, 8), Err("busy")));
    for mut future in queued {
        let Poll::Ready(reply) = poll_once(future.as_mut()) else {
            panic!("retired original read still waits behind unrelated actor Ready");
        };
        assert_eq!(reply["code"], "session-unavailable");
    }
    assert_eq!(
        running.state.admin_chat_ledger.inflight.available_permits(),
        INFLIGHT
    );
    let captured = capture(&running.state, 8).unwrap();
    assert_eq!(
        read_page(&running.state, &captured, None, 32).await["status"],
        "page"
    );
    // This must STILL be parked: progress above came from original-scope cancellation, not
    // accidental release of the old actor. Dropping Ready afterward merely cancels its send.
    let mut old_actor_probe = Box::pin(actor.roles());
    assert!(poll_once(old_actor_probe.as_mut()).is_pending());
    drop(old_actor_probe);
    drop(ready);
}

#[tokio::test]
async fn final_publication_orders_both_lock_wins_and_reply_wins_with_real_commit_barriers() {
    let mut running = mounted().await;
    let mut node = owner(884).await;
    node.send_message(1, "admitted before lock").await.unwrap();
    running.tasks.extend(install(&running.state, 7, 1, node));
    let captured = capture(&running.state, 7).unwrap();
    let encoded = read_page(&running.state, &captured, None, 32)
        .await
        .to_string();
    let mut locking = Box::pin(lock_session_inner(&running.state, None));
    let mut delivered = None;
    publish(&running.state, &captured, encoded, |reply| {
        // Reply has the actual commit+registry guards. Lock may register cancellation now, but
        // cannot report completion while the original synchronous resolver still owns custody.
        assert!(poll_once(locking.as_mut()).is_pending());
        delivered = Some(reply);
    })
    .await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&delivered.unwrap()).unwrap()["status"],
        "page"
    );
    locking.await.unwrap();

    let fresh = running.state.ui_session_generation.load(Ordering::SeqCst);
    finalize_unlock_session(&running.state, fresh, false)
        .await
        .unwrap();
    let captured = capture(&running.state, 7).unwrap();
    let encoded = read_page(&running.state, &captured, None, 32)
        .await
        .to_string();
    let guard = running.state.ui_session_commit.lock().await;
    let mut locking = Box::pin(lock_session_inner(&running.state, None));
    assert!(poll_once(locking.as_mut()).is_pending());
    let mut delivered = None;
    let mut late = Box::pin(publish(&running.state, &captured, encoded, |reply| {
        delivered = Some(reply)
    }));
    assert!(poll_once(late.as_mut()).is_pending());
    drop(guard);
    locking.await.unwrap();
    late.await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&delivered.unwrap()).unwrap()["code"],
        "session-unavailable"
    );
}

#[tokio::test]
async fn a_new_server_pane_retires_only_native_scope_and_closed_arguments_do_not_allocate_large_fields(
) {
    let mut running = mounted().await;
    running
        .tasks
        .extend(install(&running.state, 7, 1, owner(885).await));
    running
        .tasks
        .extend(install(&running.state, 8, 2, owner(886).await));
    let old = capture(&running.state, 7).unwrap();
    let fresh = capture(&running.state, 8).unwrap();
    assert!(old.scope.is_cancelled());
    assert!(!fresh.scope.is_cancelled());
    let invalid = arguments(COMMANDS[1], &InvokeBody::Json(serde_json::json!({
        "server": 8, "rowId": "x".repeat(1024 * 1024), "expectedRevision": "bad", "frameOffset": 0
    }))).unwrap();
    assert!(matches!(invalid, Request::Invalid { server: 8 }));
    assert_eq!(
        value(execute(&running.state, &fresh, invalid).await)["code"],
        "invalid-request"
    );
    assert!(arguments(
        COMMANDS[0],
        &InvokeBody::Json(
            serde_json::json!({"server":MAX_SAFE_INTEGER+1,"cursor":null,"limit":32})
        )
    )
    .is_err());
}

#[test]
fn synchronous_manual_command_inventory_and_acl_are_explicit() {
    let policy: serde_json::Value =
        serde_json::from_str(include_str!("../../security/command-policy.json")).unwrap();
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../../capabilities/main.json")).unwrap();
    assert_eq!(
        COMMANDS,
        &["get_admin_chat_ledger", "get_admin_chat_ledger_detail"]
    );
    for command in COMMANDS {
        let entry = policy
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["command"] == *command)
            .unwrap();
        assert_eq!(entry["surfaces"], serde_json::json!(["main"]));
        assert!(capability["permissions"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(format!(
                "allow-{}",
                command.replace('_', "-")
            ))));
    }
    let source = include_str!("../lib.rs");
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains(".invoke_handler(native_text::with_commands(admin_chat_ledger::with_commands(tauri::generate_handler!["));
    // The separate public moderation endpoint remains its original advisory projection.
    assert!(!source[source.find("async fn get_moderation(").unwrap()
        ..source.find("async fn get_moderation(").unwrap() + 1500]
        .contains("admin_chat_ledger"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_mockruntime_router_captures_before_spawn_and_acl_denies_secondary_window() {
    let queued = Arc::new(StdMutex::new(Vec::<NativeTask>::new()));
    let tasks = queued.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .invoke_handler(router_with_spawn(
            |_| false,
            move |future| tasks.lock().unwrap().push(future),
        ))
        .build(context)
        .unwrap();
    let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let secondary = tauri::WebviewWindowBuilder::new(&app, "security-approval", Default::default())
        .build()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let state = app.state::<AppState>();
    *state.store.lock().await =
        Some((ServerStore::open(root.path(), b"native router", &mut OsCryptoRng).unwrap()).into());
    *state.session_resumable.lock().await = true;
    finalize_unlock_session(&state, 0, false).await.unwrap();
    let mut node = owner(887).await;
    node.send_message(1, "never disclosed after old router task")
        .await
        .unwrap();
    let actor_tasks = install(&state, 7, 1, node);
    let request =
        |window: &tauri::WebviewWindow<tauri::test::MockRuntime>| tauri::webview::InvokeRequest {
            cmd: COMMANDS[0].into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: window.url().unwrap(),
            body: InvokeBody::Json(serde_json::json!({"server":7,"cursor":null,"limit":32})),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        };
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let webview: &tauri::Webview<tauri::test::MockRuntime> = main.as_ref();
    webview.clone().on_message(
        request(&main),
        Box::new(move |_, _, result, _, _| {
            let _ = tx.send(result);
        }),
    );
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(queued.lock().unwrap().len(), 1);
    lock_session_inner(&state, None).await.unwrap();
    let fresh = state.ui_session_generation.load(Ordering::SeqCst);
    finalize_unlock_session(&state, fresh, false).await.unwrap();
    let task = queued.lock().unwrap().pop().unwrap();
    task.await;
    let tauri::ipc::InvokeResponse::Ok(response) = rx.await.unwrap() else {
        panic!("closed DTO expected");
    };
    assert_eq!(
        response.deserialize::<serde_json::Value>().unwrap()["code"],
        "session-unavailable"
    );
    let response = tauri::test::get_ipc_response(&secondary, request(&secondary));
    assert!(
        response.is_err(),
        "Tauri's actual command ACL denies the restricted window before our router"
    );
    assert!(queued.lock().unwrap().is_empty());
    for task in actor_tasks {
        task.abort();
    }
    *state.store.lock().await = None;
}
