//! Actual Tauri ACL/router, real selected3 vault and AppState finalization.
//! Scheduling barriers never replace a secret, owner or native generation.
use super::*;
use tauri::test::MockRuntime;
use tokio::sync::{oneshot, Notify};

const SECRET: &str = "native text fixture secret";
type Queue = Arc<StdMutex<Vec<Task>>>;
type Reply = oneshot::Receiver<tauri::ipc::InvokeResponse>;

pub(crate) struct AdmissionBarrier {
    armed: AtomicBool,
    arrived: Notify,
    release: Notify,
}
impl AdmissionBarrier {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            armed: AtomicBool::new(true),
            arrived: Notify::new(),
            release: Notify::new(),
        })
    }
    pub(crate) async fn wait_once(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.arrived.notify_one();
            self.release.notified().await;
        }
    }
}

pub(super) struct HistoryReplyProbe {
    pub(super) action: catcoms_replication::chat::ActionId,
    pub(super) edge: Arc<AdmissionBarrier>,
    pub(super) attempts: std::sync::atomic::AtomicUsize,
}

fn existing_backend(invoke: tauri::ipc::Invoke<MockRuntime>) -> bool {
    invoke.resolver.reject("existing backend");
    true
}

fn app(
    path: PathBuf,
    edge: Option<Arc<AdmissionBarrier>>,
    enabled: bool,
) -> (
    tauri::App<MockRuntime>,
    tauri::WebviewWindow<MockRuntime>,
    Queue,
) {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let mut state = initial_state_selected(enabled);
    state.native_text.directory = Some(path);
    state.native_text.final_edge = edge;
    let queue: Queue = Arc::new(StdMutex::new(Vec::new()));
    let capture = queue.clone();
    let handler: Box<dyn Fn(tauri::ipc::Invoke<MockRuntime>) -> bool + Send + Sync> = if enabled {
        Box::new(router(existing_backend, true, move |task| {
            capture.lock().unwrap().push(task)
        }))
    } else {
        // Exercise the actual production selection wrapper, rather than
        // asserting a fixture bool or an optimizable constant.
        Box::new(with_commands(existing_backend))
    };
    let app = tauri::test::mock_builder()
        .manage(state)
        .invoke_handler(handler)
        .build(context)
        .unwrap();
    let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    (app, main, queue)
}

fn request(
    window: &tauri::WebviewWindow<MockRuntime>,
    generation: &str,
    secret: &str,
) -> tauri::webview::InvokeRequest {
    tauri::webview::InvokeRequest {
        cmd: "unlock".into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: window.url().unwrap(),
        body: InvokeBody::Json(
            serde_json::json!({ "passphrase": secret, "expectedUiGeneration": generation, "trace": null }),
        ),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.into(),
    }
}
fn invoke(window: &tauri::WebviewWindow<MockRuntime>, generation: &str, secret: &str) -> Reply {
    let (tx, rx) = oneshot::channel();
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        request(window, generation, secret),
        Box::new(move |_, _, result, _, _| {
            let _ = tx.send(result);
        }),
    );
    rx
}
#[track_caller]
fn run(queue: &Queue) -> Task {
    queue
        .lock()
        .unwrap()
        .pop()
        .expect("actual router scheduled one task")
}
fn success(reply: tauri::ipc::InvokeResponse) {
    let body = match reply {
        tauri::ipc::InvokeResponse::Ok(body) => body,
        tauri::ipc::InvokeResponse::Err(error) => panic!("actual unlock refused: {}", error.0),
    };
    let restored = body.deserialize::<serde_json::Value>().unwrap();
    assert_eq!(restored["backend"], "text-v2");
    assert!(
        restored["servers"].is_array(),
        "native session has its own closed envelope, including zero rows"
    );
}
fn refused(reply: tauri::ipc::InvokeResponse) {
    assert!(matches!(reply, tauri::ipc::InvokeResponse::Err(_)));
}

fn found_request(window: &tauri::WebviewWindow<MockRuntime>, id: u8, name: &str) -> Reply {
    let mut request = request(window, "1", SECRET);
    request.cmd = "found_server".into();
    request.body = InvokeBody::Json(serde_json::json!({
        "requestId":hex::encode([id;32]), "displayName":"Founder", "serverName":name,
        "advertise":"", "relay":"", "rendezvous":"", "isDm":false,
    }));
    let (tx, rx) = oneshot::channel();
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        request,
        Box::new(move |_, _, result, _, _| {
            let _ = tx.send(result);
        }),
    );
    rx
}

fn found_value(reply: tauri::ipc::InvokeResponse) -> serde_json::Value {
    match reply {
        tauri::ipc::InvokeResponse::Ok(body) => body.deserialize().unwrap(),
        tauri::ipc::InvokeResponse::Err(error) => panic!("native founding refused: {error:?}"),
    }
}

async fn hold_vault_metadata(
    state: &AppState,
) -> catcoms_app::store::NativeTextVaultAuthentication {
    timeout(Duration::from_secs(2), async {
        loop {
            let preparation = {
                let store = state.store.lock().await;
                let vault = store.as_ref().unwrap().as_text_v2().unwrap();
                vault.prepare_native_authentication(state.native_text.ui_budget.prepare().unwrap())
            };
            if !preparation.is_capture_busy_cached() {
                break preparation;
            }
            drop(preparation);
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the original vault's metadata slot became available")
}

fn resume_request(window: &tauri::WebviewWindow<MockRuntime>) -> Reply {
    resume_request_with_reply(window, false)
}
fn resume_request_with_reply(
    window: &tauri::WebviewWindow<MockRuntime>,
    panic_on_reply: bool,
) -> Reply {
    let mut request = request(window, "1", SECRET);
    request.cmd = "resume_session".into();
    request.body = InvokeBody::Json(serde_json::json!({}));
    let (tx, rx) = oneshot::channel();
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        request,
        Box::new(move |_, _, result, _, _| {
            if panic_on_reply {
                panic!("actual listing resolver unwind");
            }
            let _ = tx.send(result);
        }),
    );
    rx
}

async fn listing_warning_seen(
    app: &tauri::App<MockRuntime>,
    main: &tauri::WebviewWindow<MockRuntime>,
    queue: &Queue,
) {
    let bad = snapshot_request(main, Some(r#"{"version":999}"#));
    run(queue).await;
    assert!(snapshot_result(bad.await.unwrap())["continuity_error"].is_string());
    let generation = app
        .state::<AppState>()
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let warning = invoke(main, &generation, SECRET);
    run(queue).await;
    let tauri::ipc::InvokeResponse::Err(error) = warning.await.unwrap() else {
        panic!("expected actual screen-state warning")
    };
    assert!(error.0.as_str().unwrap().contains("Unlock again"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_listing_busy_reply_cannot_consume_the_actual_authenticated_acknowledgement() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    listing_warning_seen(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let held = [
        state
            .native_text
            .listing
            .prepare(&state.native_text.ui_budget)
            .unwrap(),
        state
            .native_text
            .listing
            .prepare(&state.native_text.ui_budget)
            .unwrap(),
    ];
    let generation = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let busy = invoke(&main, &generation, SECRET);
    run(&queue).await;
    refused(busy.await.unwrap());
    assert!(
        state
            .native_text
            .ui_state
            .snapshots
            .close_decision(None)
            .notice
            .is_some(),
        "a rejected listing must retain the acknowledgement"
    );
    drop(held);
    let admitted = invoke(&main, &generation, SECRET);
    run(&queue).await;
    success(admitted.await.unwrap());
    assert!(
        state
            .native_text
            .ui_state
            .snapshots
            .close_decision(None)
            .notice
            .is_none(),
        "only genuine successful restoration consumes it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_listing_actual_resolver_unwind_and_lost_results_drop_outside_guards_with_original_payment(
) {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let watch = Arc::new(listing::DropWatch {
        commit: state.ui_session_commit.clone(),
        store: state.store.clone(),
        table: state.servers.clone(),
        observed: std::sync::atomic::AtomicUsize::new(0),
        outside: AtomicBool::new(true),
    });
    *state.native_text.listing_drop_watch.lock().unwrap() = Some(watch.clone());
    for _ in 0..3 {
        let reply = resume_request_with_reply(&main, true);
        let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
        assert!(task.await.unwrap_err().is_panic());
        assert!(reply.await.is_err());
    }
    for _ in 0..3 {
        drop(resume_request(&main));
        run(&queue).await;
    }
    let reply = resume_request(&main);
    run(&queue).await;
    assert_eq!(found_value(reply.await.unwrap())["backend"], "text-v2");
    assert_eq!(watch.observed.load(Ordering::SeqCst), 7);
    assert!(watch.outside.load(Ordering::SeqCst));
    *state.native_text.listing_drop_watch.lock().unwrap() = None;
    let one = state
        .native_text
        .listing
        .prepare(&state.native_text.ui_budget)
        .unwrap();
    let two = state
        .native_text
        .listing
        .prepare(&state.native_text.ui_budget)
        .unwrap();
    drop((one, two));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_listing_empty_unlock_and_two_real_rows_resume_with_exact_contexts() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    let locked = resume_request(&main);
    assert_eq!(found_value(locked.await.unwrap()), serde_json::Value::Null);
    assert!(queue.lock().unwrap().is_empty());
    let unlocked = invoke(&main, "1", SECRET);
    run(&queue).await;
    let empty = found_value(unlocked.await.unwrap());
    assert_eq!(empty, serde_json::json!({"backend":"text-v2","servers":[]}));
    let mut found = Vec::new();
    for (id, name) in [(1, "First"), (2, "Second")] {
        let reply = found_request(&main, id, name);
        run(&queue).await;
        found.push(found_value(reply.await.unwrap()));
    }
    let resumed = resume_request(&main);
    run(&queue).await;
    let resumed = found_value(resumed.await.unwrap());
    assert_eq!(resumed["backend"], "text-v2");
    assert_eq!(resumed["servers"].as_array().unwrap().len(), 2);
    for created in &found {
        let row = resumed["servers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["server"] == created["server"])
            .unwrap();
        assert_eq!(row["context"], created["context"]);
        assert_eq!(row["channel"], created["channel"]);
        assert_eq!(row["display_name"], "Founder");
    }
    let state = app.state::<AppState>();
    lock_session_outcome_inner(&state, None).await;
    assert_eq!(
        found_value(resume_request(&main).await.unwrap()),
        serde_json::Value::Null
    );
    let generation = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let reauthenticated = invoke(&main, &generation, SECRET);
    run(&queue).await;
    assert_eq!(found_value(reauthenticated.await.unwrap()), resumed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_listing_empty_snapshot_cannot_omit_a_later_genuine_row_or_adopt_a_new_generation() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let edge = AdmissionBarrier::new();
    *state.native_text.listing_reply_edge.lock().unwrap() = Some(edge.clone());
    let empty = resume_request(&main);
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let created = found_request(&main, 1, "Inserted");
    run(&queue).await;
    found_value(created.await.unwrap());
    edge.release.notify_one();
    task.await.unwrap();
    refused(empty.await.unwrap());
    *state.native_text.listing_reply_edge.lock().unwrap() = None;
    let old = resume_request(&main);
    lock_session_outcome_inner(&state, None).await;
    // Unlock is an independent authoring admission, not the listing slot. Poll
    // the newer unlock before the deliberately unpolled original resume.
    unlock_current(&app, &main, &queue).await;
    run(&queue).await;
    refused(old.await.unwrap());
    let valid = resume_request(&main);
    run(&queue).await;
    assert_eq!(
        found_value(valid.await.unwrap())["servers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_listing_same_id_replacement_and_whole_close_with_held_output_refuse_stale_reply() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let created = found_request(&main, 1, "Original");
    run(&queue).await;
    let created = found_value(created.await.unwrap());
    let state = app.state::<AppState>();
    let edge = AdmissionBarrier::new();
    *state.native_text.listing_reply_edge.lock().unwrap() = Some(edge.clone());
    let old = resume_request(&main);
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let replacement = replacement_table(&state, &created).await;
    let original = {
        let mut table = state.servers.lock().await;
        std::mem::replace(&mut *table, replacement)
    };
    edge.release.notify_one();
    task.await.unwrap();
    refused(old.await.unwrap());
    let replacement = {
        let mut table = state.servers.lock().await;
        std::mem::replace(&mut *table, original)
    };
    let edge = AdmissionBarrier::new();
    *state.native_text.listing_reply_edge.lock().unwrap() = Some(edge.clone());
    let old = resume_request(&main);
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let close = register_vault_close(&state).await.unwrap();
    timeout(
        Duration::from_secs(60),
        sources::close_server_rows(&state, &SystemClock),
    )
    .await
    .unwrap()
    .unwrap();
    catcoms_storage::acquire_vault_session(&path).unwrap();
    edge.release.notify_one();
    task.await.unwrap();
    refused(old.await.unwrap());
    drop((close, replacement));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_listing_unlock_keeps_its_actual_authentication_when_a_live_foreign_mount_replaces_it(
) {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("original"), None, true);
    let (foreign, other, other_queue) = self::app(root.path().join("foreign"), None, true);
    unlock_current(&app, &main, &queue).await;
    unlock_current(&foreign, &other, &other_queue).await;
    listing_warning_seen(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let foreign_state = foreign.state::<AppState>();
    let generation = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let edge = AdmissionBarrier::new();
    *state.native_text.reply_edge.lock().unwrap() = Some(edge.clone());
    let reply = invoke(&main, &generation, SECRET);
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let moved = foreign_state.store.lock().await.take().unwrap();
    let original = state.store.lock().await.replace(moved).unwrap();
    edge.release.notify_one();
    task.await.unwrap();
    refused(reply.await.unwrap());
    let moved = state.store.lock().await.replace(original).unwrap();
    *foreign_state.store.lock().await = Some(moved);
    assert!(
        state
            .native_text
            .ui_state
            .snapshots
            .close_decision(None)
            .notice
            .is_some(),
        "stale listing cannot consume its acknowledgement"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_found_reply_rechecks_actual_mount_even_when_original_is_still_live() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let (foreign_app, foreign_main, foreign_queue) =
        self::app(root.path().join("foreign"), None, true);
    unlock_current(&foreign_app, &foreign_main, &foreign_queue).await;
    let foreign_state = foreign_app.state::<AppState>();
    let state = app.state::<AppState>();
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let edge = AdmissionBarrier::new();
    *state.native_text.text_reply_edge.lock().unwrap() = Some(edge.clone());
    let reply = found_request(&main, 1, "Original");
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let foreign = foreign_state.store.lock().await.take().unwrap();
    let original = state.store.lock().await.replace(foreign).unwrap();
    let target = state.native_text.ui_state.original_target().unwrap();
    assert!(target.is_for_original_vault(original.as_text_v2().unwrap()));
    assert_eq!(
        state.ui_session_generation.load(Ordering::SeqCst),
        generation
    );
    edge.release.notify_one();
    task.await.unwrap();
    refused(reply.await.unwrap());
    let foreign = state.store.lock().await.replace(original).unwrap();
    *foreign_state.store.lock().await = Some(foreign);
    *state.native_text.text_reply_edge.lock().unwrap() = None;
    let retry = found_request(&main, 1, "Original");
    run(&queue).await;
    assert_eq!(found_value(retry.await.unwrap())["backend"], "text-v2");
    assert_eq!(state.servers.lock().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_send_reply_rechecks_installed_row_without_relying_on_owner_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let found = found_request(&main, 1, "Original");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let state = app.state::<AppState>();
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let account = keep_session::current(&state, generation).unwrap();
    let scope = state
        .servers
        .lock()
        .await
        .native(&found["server"].as_u64().unwrap())
        .unwrap()
        .scope(account)
        .unwrap();
    let edge = AdmissionBarrier::new();
    *state.native_text.text_reply_edge.lock().unwrap() = Some(edge.clone());
    let reply = native_send_request(&main, &found, 3, "Saved original");
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let mut replacement = replacement_table(&state, &found).await;
    let original = {
        let mut table = state.servers.lock().await;
        std::mem::replace(&mut *table, replacement)
    };
    assert!(
        !scope.is_cancelled(),
        "parked owner deliberately remains live"
    );
    assert_eq!(
        state.ui_session_generation.load(Ordering::SeqCst),
        generation
    );
    edge.release.notify_one();
    task.await.unwrap();
    refused(reply.await.unwrap());
    replacement = {
        let mut table = state.servers.lock().await;
        std::mem::replace(&mut *table, original)
    };
    *state.native_text.text_reply_edge.lock().unwrap() = None;
    let retry = native_send_request(&main, &found, 3, "Saved original");
    run(&queue).await;
    let saved = found_value(retry.await.unwrap());
    assert_eq!(saved["disposition"], "saved-local");
    assert_eq!(saved["ordinal"], "0");
    drop(replacement); // outside guards, AFTER exact retry; closes its Source
}

/// A real nonempty table holding the SAME numeric ID/context but a different
/// native incarnation. Keep the original owner live so negative cancellation
/// cannot mask a missing positive identity check.
async fn replacement_table(
    state: &AppState,
    found: &serde_json::Value,
) -> server_table::ServerTable {
    let mut replacement = server_table::ServerTable::default();
    let credit = replacement.native_budget().prepare().unwrap();
    let mut candidate = {
        let table = state.servers.lock().await;
        let data = table
            .native(&found["server"].as_u64().unwrap())
            .unwrap()
            .data();
        Some(
            server_table::PreparedNativeRow::new(
                server_table::NativeRowSpec {
                    selection: data.selection.clone(),
                    instance: data.instance + 1,
                    target: data.target.clone(),
                    request: data.request,
                    name: &data.name,
                    display_name: &data.display_name,
                    is_dm: data.is_dm,
                    channel: data.channel,
                },
                credit,
            )
            .unwrap(),
        )
    };
    replacement.install_native(&mut candidate).unwrap();
    replacement
}

fn native_history_request(
    window: &tauri::WebviewWindow<MockRuntime>,
    found: &serde_json::Value,
    start: &str,
    end: Option<&str>,
    limit: usize,
) -> Reply {
    native_history_request_with_reply(window, found, start, end, limit, false)
}

fn native_received_request(
    window: &tauri::WebviewWindow<MockRuntime>,
    found: &serde_json::Value,
) -> Reply {
    let mut request = request(window, "1", SECRET);
    request.cmd = "get_message_page".into();
    request.body = InvokeBody::Json(serde_json::json!({
        "server": found["server"],
        "channel": found["channel"],
        "expectedContext": found["context"],
        "receivedAction": hex::encode([249; 32]),
    }));
    let (tx, rx) = oneshot::channel();
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        request,
        Box::new(move |_, _, result, _, _| {
            let _ = tx.send(result);
        }),
    );
    rx
}
fn native_history_request_with_reply(
    window: &tauri::WebviewWindow<MockRuntime>,
    found: &serde_json::Value,
    start: &str,
    end: Option<&str>,
    limit: usize,
    panic_on_reply: bool,
) -> Reply {
    let mut request = request(window, "1", SECRET);
    request.cmd = "get_message_page".into();
    request.body = InvokeBody::Json(
        serde_json::json!({"server":found["server"],"channel":found["channel"],"expectedContext":found["context"],"start":start,"endExclusive":end,"limit":limit}),
    );
    let (tx, rx) = oneshot::channel();
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        request,
        Box::new(move |_, _, result, _, _| {
            if panic_on_reply {
                panic!("actual history resolver unwind");
            }
            let _ = tx.send(result);
        }),
    );
    rx
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_history_reads_actual_saved_originals_and_keeps_first_end_across_new_saves() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let found = found_request(&main, 1, "History");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let empty = native_history_request(&main, &found, "0", None, 8);
    run(&queue).await;
    let empty = found_value(empty.await.unwrap());
    assert_eq!(empty["scope"], "own-saved-originals");
    assert_eq!(empty["next"], "0");
    assert_eq!(empty["end_exclusive"], "0");
    let mut originals = Vec::new();
    for (token, text) in [(4, "First original"), (5, "Second original")] {
        let saved = native_send_request(&main, &found, token, text);
        run(&queue).await;
        originals.push(found_value(saved.await.unwrap())["message_id"].clone());
    }
    let first = native_history_request(&main, &found, "0", None, 1);
    run(&queue).await;
    let first = found_value(first.await.unwrap());
    assert_eq!(first["backend"], "text-v2");
    assert_eq!(first["next"], "1");
    assert_eq!(first["end_exclusive"], "2");
    assert_eq!(first["items"][0]["action"], originals[0]);
    assert_eq!(first["items"][0]["status"], "pending");
    assert_eq!(first["items"][0]["channel"], found["channel"]);
    assert_eq!(first["items"][0]["body"]["text"], "First original");
    let late = native_send_request(&main, &found, 6, "Later original");
    run(&queue).await;
    assert_eq!(found_value(late.await.unwrap())["ordinal"], "2");
    let second = native_history_request(&main, &found, "1", Some("2"), 1);
    run(&queue).await;
    let second = found_value(second.await.unwrap());
    assert_eq!(second["next"], "2");
    assert_eq!(second["end_exclusive"], "2");
    assert_eq!(second["items"][0]["action"], originals[1]);
    let terminal = native_history_request(&main, &found, "2", Some("2"), 8);
    run(&queue).await;
    assert!(found_value(terminal.await.unwrap())["items"]
        .as_array()
        .unwrap()
        .is_empty());
    let current = native_history_request(&main, &found, "0", None, 8);
    run(&queue).await;
    assert_eq!(
        found_value(current.await.unwrap())["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_history_unrun_admission_is_bounded_and_stale_context_never_selects_another_row(
) {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let found = found_request(&main, 1, "History");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    for _ in 0..3 {
        let pending = native_history_request(&main, &found, "0", None, 8);
        let overflow = native_history_request(&main, &found, "0", None, 8);
        refused(overflow.await.unwrap());
        assert_eq!(queue.lock().unwrap().len(), 1);
        queue.lock().unwrap().clear();
        assert!(pending.await.is_err());
    }
    for (start, end, limit) in [("01", None, 8), ("0", None, 9), ("2", Some("1"), 1)] {
        refused(
            native_history_request(&main, &found, start, end, limit)
                .await
                .unwrap(),
        );
        assert!(queue.lock().unwrap().is_empty());
    }
    let mut foreign = found.clone();
    foreign["context"] = serde_json::json!(hex::encode([9; 32]));
    refused(
        native_history_request(&main, &foreign, "0", None, 8)
            .await
            .unwrap(),
    );
    let valid = native_history_request(&main, &found, "0", None, 8);
    run(&queue).await;
    assert_eq!(found_value(valid.await.unwrap())["next"], "0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_history_real_resolver_unwind_and_lost_observers_release_original_work_and_credits(
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let workers = AdmissionBarrier::new();
    *state.native_text.founder_workers_edge.lock().unwrap() = Some(workers.clone());
    let found = found_request(&main, 1, "History");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    // The drop probe checks the shared mutexes. Park the founder's unrelated
    // connected workers so their independent traffic cannot trip that probe.
    timeout(Duration::from_secs(5), workers.arrived.notified())
        .await
        .expect("the original founder workers are parked before taking the shared guards");
    let saved = native_send_request(&main, &found, 4, "Original");
    run(&queue).await;
    found_value(saved.await.unwrap());
    let watch = Arc::new(history::DropWatch {
        commit: state.ui_session_commit.clone(),
        store: state.store.clone(),
        table: state.servers.clone(),
        observed: std::sync::atomic::AtomicUsize::new(0),
        outside: AtomicBool::new(true),
    });
    *state.native_text.history_drop_watch.lock().unwrap() = Some(watch.clone());
    // Three actual callback unwinds exceed the two-credit policy if any paid
    // output survives its task. No mocked Root result or fake save is used.
    for _ in 0..3 {
        let reply = native_history_request_with_reply(&main, &found, "0", None, 8, true);
        let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
        assert!(task.await.unwrap_err().is_panic());
        assert!(reply.await.is_err());
        assert!(state.ui_session_commit.try_lock().is_ok());
        assert!(state.store.try_lock().is_ok());
        assert!(state.servers.try_lock().is_ok());
    }
    for _ in 0..3 {
        drop(native_history_request(&main, &found, "0", None, 8));
        run(&queue).await;
    }
    let reply = native_history_request(&main, &found, "0", None, 8);
    run(&queue).await;
    assert_eq!(
        found_value(reply.await.unwrap())["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(watch.observed.load(Ordering::SeqCst), 7);
    assert!(
        watch.outside.load(Ordering::SeqCst),
        "actual page/body dropped outside all native guards while original credit remained live"
    );
    *state.native_text.history_drop_watch.lock().unwrap() = None;
    let close = register_vault_close(&state).await.unwrap();
    workers.release.notify_one();
    timeout(
        Duration::from_secs(60),
        sources::close_server_rows(&state, &SystemClock),
    )
    .await
    .unwrap()
    .unwrap();
    catcoms_storage::acquire_vault_session(&path).unwrap();
    drop(close);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_state_critical4_real_sealed_roundtrip_and_lock_preserve_exact_retry_data() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let token = "01".repeat(32);
    // Deliberately not an actual row's context: this journal stores caller DATA
    // and cannot grant signing authority. The native send factory remains the
    // independent original-object/generation enforcement boundary.
    let json=serde_json::json!({"version":4,"drafts":{},"readMarks":{},"nativeTextIntents":{token.clone():{"kind":"create","token":token,"server":7,"channel":"9","expectedContext":"02".repeat(32),"text":"exact original\n🐈"}}}).to_string();
    let saved = ui_request(&main, "save_ui_state", Some(&json));
    run(&queue).await;
    assert!(matches!(
        saved.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
    assert_eq!(read_ui(&main, &queue).await, json);
    let state = app.state::<AppState>();
    let outcome = lock_session_outcome_inner(&state, Some(json.clone())).await;
    assert!(outcome.continuity_error.is_none());
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, json);
    let ciphertext = std::fs::read(path.join("ui-state.bin")).unwrap();
    for malformed in [
        serde_json::json!({"version":5,"drafts":{},"readMarks":{}}),
        serde_json::json!({"version":4,"drafts":{},"readMarks":{},"nativeTextIntents":{token.clone():{"kind":"future","token":token}}}),
        serde_json::json!({"version":3,"drafts":{},"readMarks":{},"nativeTextIntents":{}}),
    ] {
        let malformed = malformed.to_string();
        let refused_write = ui_request(&main, "save_ui_state", Some(&malformed));
        if !queue.lock().unwrap().is_empty() {
            run(&queue).await;
        }
        refused(refused_write.await.unwrap());
        assert_eq!(
            std::fs::read(path.join("ui-state.bin")).unwrap(),
            ciphertext
        );
        assert_eq!(read_ui(&main, &queue).await, json);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_history_reply_rechecks_same_id_incarnation_and_does_not_hold_source_across_lock(
) {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let found = found_request(&main, 1, "History");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let saved = native_send_request(&main, &found, 4, "Original");
    run(&queue).await;
    found_value(saved.await.unwrap());
    let state = app.state::<AppState>();
    let edge = AdmissionBarrier::new();
    *state.native_text.text_reply_edge.lock().unwrap() = Some(edge.clone());
    let reply = native_history_request(&main, &found, "0", None, 8);
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let replacement = replacement_table(&state, &found).await;
    let original = {
        let mut table = state.servers.lock().await;
        std::mem::replace(&mut *table, replacement)
    };
    edge.release.notify_one();
    task.await.unwrap();
    refused(reply.await.unwrap());
    let replacement = {
        let mut table = state.servers.lock().await;
        std::mem::replace(&mut *table, original)
    };
    let edge = AdmissionBarrier::new();
    *state.native_text.text_reply_edge.lock().unwrap() = Some(edge.clone());
    let reply = native_history_request(&main, &found, "0", None, 8);
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    timeout(
        Duration::from_secs(60),
        lock_session_outcome_inner(&state, None),
    )
    .await
    .unwrap();
    edge.release.notify_one();
    task.await.unwrap();
    refused(reply.await.unwrap());
    *state.native_text.text_reply_edge.lock().unwrap() = None;
    unlock_current(&app, &main, &queue).await;
    let retry = native_history_request(&main, &found, "0", None, 8);
    run(&queue).await;
    assert_eq!(
        found_value(retry.await.unwrap())["items"][0]["body"]["text"],
        "Original"
    );
    let edge = AdmissionBarrier::new();
    *state.native_text.text_reply_edge.lock().unwrap() = Some(edge.clone());
    let reply = native_history_request(&main, &found, "0", None, 8);
    let task = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let close = register_vault_close(&state).await.unwrap();
    timeout(
        Duration::from_secs(60),
        sources::close_server_rows(&state, &SystemClock),
    )
    .await
    .unwrap()
    .unwrap();
    // Unlike UI lock, whole close drains actual Root/Source/SDK/Physical work.
    // The still-paid page must not retain any of those execution owners.
    catcoms_storage::acquire_vault_session(root.path().join("vault")).unwrap();
    edge.release.notify_one();
    task.await.unwrap();
    refused(reply.await.unwrap());
    drop(close);
    drop(replacement);
}

fn native_send_request(
    window: &tauri::WebviewWindow<MockRuntime>,
    found: &serde_json::Value,
    token: u8,
    text: &str,
) -> Reply {
    let mut request = request(window, "1", SECRET);
    request.cmd = "send_message".into();
    request.body = InvokeBody::Json(
        serde_json::json!({"server":found["server"],"channel":found["channel"],
        "expectedContext":found["context"],"retryToken":hex::encode([token;32]),"text":text,"replyTo":null}),
    );
    let (tx, rx) = oneshot::channel();
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        request,
        Box::new(move |_, _, result, _, _| {
            let _ = tx.send(result);
        }),
    );
    rx
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_mutations_preserve_exact_originals_and_the_observed_revision() {
    let directory = tempfile::tempdir().unwrap();
    let (app, window, queue) = app(directory.path().join("vault"), None, true);
    unlock_current(&app, &window, &queue).await;
    let found = found_request(&window, 1, "Native revisions");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let create = native_send_request(&window, &found, 31, "original text");
    run(&queue).await;
    let create = found_value(create.await.unwrap());
    let target = &create["message_id"];
    let view = |action: &serde_json::Value| {
        invite_command(
            &window,
            "get_message_page",
            serde_json::json!({
                "server":found["server"], "channel":found["channel"], "expectedContext":found["context"],
                "ownAction":action,
            }),
        )
    };
    let original = view(target);
    run(&queue).await;
    let original = found_value(original.await.unwrap());
    assert_eq!(original["scope"], "own-message-revision");
    assert_eq!(original["item"]["text"], "original text");
    assert_eq!(&original["item"]["revision"], target);
    let mutate = |command: &str,
                  token: u8,
                  message: &serde_json::Value,
                  revision: &serde_json::Value,
                  text: Option<&str>| {
        let mut body = serde_json::json!({
            "server":found["server"], "channel":found["channel"], "expectedContext":found["context"],
            "retryToken":hex::encode([token;32]), "messageId":message, "expectedRevision":revision,
        });
        if let Some(text) = text {
            body["text"] = text.into();
        }
        invite_command(&window, command, body)
    };
    let first = mutate("edit_message", 32, target, target, Some("first edit"));
    run(&queue).await;
    let first = found_value(first.await.unwrap());
    assert_eq!(first["disposition"], "saved-local");
    assert_ne!(first["message_id"], *target);
    let unrelated = native_send_request(&window, &found, 33, "unrelated message");
    run(&queue).await;
    let unrelated = found_value(unrelated.await.unwrap());
    let invalid = mutate(
        "edit_message",
        34,
        target,
        &unrelated["message_id"],
        Some("second edit"),
    );
    run(&queue).await;
    refused(invalid.await.unwrap());
    let second = mutate(
        "edit_message",
        34,
        target,
        &first["message_id"],
        Some("second edit"),
    );
    run(&queue).await;
    let second = found_value(second.await.unwrap());
    assert_eq!(second["disposition"], "saved-local");
    let read = view(target);
    run(&queue).await;
    let read = found_value(read.await.unwrap());
    assert_eq!(&read["action"], target);
    assert_eq!(read["item"]["revision"], second["message_id"]);
    assert_eq!(read["item"]["text"], "second edit");
    let indirect = view(&second["message_id"]);
    run(&queue).await;
    assert!(found_value(indirect.await.unwrap())["item"].is_null());
    let crossed = invite_command(
        &window,
        "get_message_page",
        serde_json::json!({
            "server":found["server"], "channel":found["channel"], "expectedContext":found["context"],
            "ownAction":target, "receivedAction":target,
        }),
    );
    refused(crossed.await.unwrap());
    let retry = mutate(
        "edit_message",
        34,
        target,
        &first["message_id"],
        Some("second edit"),
    );
    run(&queue).await;
    assert_eq!(found_value(retry.await.unwrap()), second);
    let stale = mutate("edit_message", 34, target, target, Some("second edit"));
    run(&queue).await;
    refused(stale.await.unwrap());
    let stale = mutate("delete_message", 35, target, &first["message_id"], None);
    run(&queue).await;
    refused(stale.await.unwrap());
    let removed = mutate("delete_message", 35, target, &second["message_id"], None);
    run(&queue).await;
    let removed = found_value(removed.await.unwrap());
    assert_eq!(removed["disposition"], "saved-local");
    let read = view(target);
    run(&queue).await;
    let read = found_value(read.await.unwrap());
    assert_eq!(read["scanning"], false);
    assert!(read["item"].is_null());
    let after_delete = mutate(
        "edit_message",
        36,
        target,
        &removed["message_id"],
        Some("resurrected text"),
    );
    run(&queue).await;
    refused(after_delete.await.unwrap());
    let retry = mutate("delete_message", 35, target, &second["message_id"], None);
    run(&queue).await;
    assert_eq!(found_value(retry.await.unwrap()), removed);
    let audit = native_history_request(&window, &found, "0", None, 8);
    run(&queue).await;
    let audit = found_value(audit.await.unwrap());
    assert_eq!(audit["end_exclusive"], "5");
    assert_eq!(audit["items"][0]["body"]["text"], "original text");
    assert_eq!(audit["items"][3]["body"]["text"], "second edit");
    assert_eq!(audit["items"][4]["action"], removed["message_id"]);
    assert_eq!(
        audit["items"][4]["body"],
        serde_json::json!({"kind":"delete","create":target})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_found_router_installs_two_original_rows_retries_and_releases_real_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let first_reply = found_request(&main, 1, "First");
    run(&queue).await;
    let first = found_value(first_reply.await.unwrap());
    assert_eq!(first["backend"], "text-v2");
    let id = first["server"].as_u64().unwrap();
    let state = app.state::<AppState>();
    let (instance, selection) = {
        let table = state.servers.lock().await;
        assert_eq!(table.len(), 1);
        assert!(
            table.legacy(&id).is_none(),
            "no dummy actor backs native text"
        );
        let row = table.native(&id).unwrap().data();
        (row.instance, row.selection.clone())
    };
    let retry = found_request(&main, 1, "First");
    run(&queue).await;
    assert_eq!(found_value(retry.await.unwrap()), first);
    {
        let table = state.servers.lock().await;
        let row = table.native(&id).unwrap().data();
        assert_eq!(row.instance, instance);
        assert!(row.selection.same_original(&selection));
    }
    let conflict = found_request(&main, 1, "Changed");
    run(&queue).await;
    refused(conflict.await.unwrap());
    let occupied = hold_vault_metadata(&state).await;
    let busy = Arc::new(Notify::new());
    *state.native_text.found_setup_busy.lock().unwrap() = Some(busy.clone());
    let second = found_request(&main, 2, "Second");
    let second_task = tokio::spawn(run(&queue));
    timeout(Duration::from_secs(5), busy.notified())
        .await
        .expect("the second Found reached the busy original metadata slot");
    drop(occupied);
    second_task.await.unwrap();
    let second = found_value(second.await.unwrap());
    *state.native_text.found_setup_busy.lock().unwrap() = None;
    assert_ne!(second["server"], first["server"]);
    assert_eq!(state.servers.lock().await.native_iter().count(), 2);
    // Both rows must really sign/save using the production funded two-context
    // mount. Installing two UI rows alone establishes no such capacity.
    let occupied = hold_vault_metadata(&state).await;
    let post = native_send_request(&main, &first, 9, "First native message");
    let post_task = tokio::spawn(run(&queue));
    timeout(
        Duration::from_secs(5),
        state.native_text.send_preparation_busy.notified(),
    )
    .await
    .expect("native send retries the original metadata lookup");
    drop(occupied);
    post_task.await.unwrap();
    let post = found_value(post.await.unwrap());
    assert_eq!(post["disposition"], "saved-local");
    assert_eq!(post["ordinal"], "0");
    let retry = native_send_request(&main, &first, 9, "First native message");
    run(&queue).await;
    assert_eq!(found_value(retry.await.unwrap()), post);
    let changed = native_send_request(&main, &first, 9, "Changed text");
    run(&queue).await;
    refused(changed.await.unwrap());
    let other = native_send_request(&main, &second, 9, "Other native context");
    run(&queue).await;
    let other = found_value(other.await.unwrap());
    assert_eq!(other["disposition"], "saved-local");
    assert_eq!(other["ordinal"], "0");
    assert_ne!(other["message_id"], post["message_id"]);
    let exhausted = found_request(&main, 3, "Third");
    run(&queue).await;
    refused(exhausted.await.unwrap());
    assert_eq!(state.servers.lock().await.len(), 2);
    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    assert!(state.store.lock().await.is_none());
    assert!(state.servers.lock().await.is_empty());
    // A retained paid selector cannot retain the SDK or actual Physical owner.
    catcoms_storage::acquire_vault_session(&path).unwrap();
    drop(close);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_connected_join_keeps_legacy_invitation_handlers_outside_native_mode() {
    for enabled in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("vault");
        let (_app, main, queue) = self::app(path.clone(), None, enabled);
        for command in [
            "join_server",
            "preview_invite",
            "get_invite",
            "mint_invite_fresh",
            "send_dm_invite",
        ] {
            let (send, reply) = oneshot::channel();
            let mut request = request(&main, "1", SECRET);
            request.cmd = command.into();
            request.body = InvokeBody::Json(serde_json::json!({}));
            let view: &tauri::Webview<MockRuntime> = main.as_ref();
            view.clone().on_message(
                request,
                Box::new(move |_, _, result, _, _| {
                    let _ = send.send(result);
                }),
            );
            let tauri::ipc::InvokeResponse::Err(error) = reply.await.unwrap() else {
                panic!("invitation IPC reached a worker");
            };
            let expected = if enabled
                && matches!(
                    command,
                    "join_server" | "preview_invite" | "mint_invite_fresh"
                ) {
                listing::STALE
            } else if enabled {
                "Invitations are unavailable in this Chat V2 checkpoint."
            } else {
                "existing backend"
            };
            assert_eq!(error.0, serde_json::json!(expected), "{command}");
            assert!(queue.lock().unwrap().is_empty());
        }
        assert!(!path.exists());
    }
}

fn preview_request(window: &tauri::WebviewWindow<MockRuntime>, hex: String) -> Reply {
    invite_command(
        window,
        "preview_invite",
        serde_json::json!({ "inviteHex": hex }),
    )
}

fn invite_command(
    window: &tauri::WebviewWindow<MockRuntime>,
    command: &str,
    body: serde_json::Value,
) -> Reply {
    let (send, reply) = oneshot::channel();
    let mut invocation = request(window, "1", SECRET);
    invocation.cmd = command.into();
    invocation.body = InvokeBody::Json(body);
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        invocation,
        Box::new(move |_, _, result, _, _| {
            let _ = send.send(result);
        }),
    );
    reply
}

async fn command_value_after_busy(
    window: &tauri::WebviewWindow<MockRuntime>,
    queue: &Queue,
    command: &str,
    busy: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    match command_after_busy(window, queue, command, busy, input).await {
        tauri::ipc::InvokeResponse::Ok(body) => body.deserialize().unwrap(),
        tauri::ipc::InvokeResponse::Err(error) => panic!("{command} refused: {error:?}"),
    }
}

async fn command_after_busy(
    window: &tauri::WebviewWindow<MockRuntime>,
    queue: &Queue,
    command: &str,
    busy: &str,
    input: serde_json::Value,
) -> tauri::ipc::InvokeResponse {
    // A transient admission refusal can precede scheduling or come from the
    // worker. Every retry uses the same original request data.
    let retry_until = SystemClock.monotonic_ms().saturating_add(5_000);
    loop {
        let reply = invite_command(window, command, input.clone());
        let pending = queue.lock().unwrap().pop();
        if let Some(work) = pending {
            work.await;
        }
        match reply.await.unwrap() {
            tauri::ipc::InvokeResponse::Err(error)
                if error.0.as_str() == Some(busy) && SystemClock.monotonic_ms() < retry_until =>
            {
                SystemClock.sleep(Duration::from_millis(25)).await;
            }
            result => return result,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_review_does_not_adopt_an_unlock_after_router_admission() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let found = found_request(&main, 17, "Review source");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let old = invite_command(
        &main,
        "check_native_chat_removed_posts",
        serde_json::json!({
            "server": found["server"], "expectedContext": found["context"],
            "expectedAdmission": "a1".repeat(32),
            "contact": "a2".repeat(catcoms_sync::V2_MEMBER_CONTACT_BYTES),
        }),
    );
    // Actual router admission succeeded. Nothing in the worker has run yet.
    let task = run(&queue);
    let sent = native_send_request(&main, &found, 46, "Send while a device check is queued");
    run(&queue).await;
    found_value(sent.await.unwrap());
    let state = app.state::<AppState>();
    let previous = state.ui_session_generation.load(Ordering::SeqCst);
    lock_session_outcome_inner(&state, None).await;
    unlock_current(&app, &main, &queue).await;
    assert_ne!(state.ui_session_generation.load(Ordering::SeqCst), previous);
    task.await;
    let tauri::ipc::InvokeResponse::Err(error) = old.await.unwrap() else {
        panic!("stale review invocation returned a result");
    };
    assert_eq!(error.0, serde_json::json!(listing::STALE));
    let holds = path.join("chat-v2-removed-holds");
    if holds.exists() {
        assert!(std::fs::read_dir(holds).unwrap().next().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_membership_dispatch_uses_the_original_founder_row() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let found = found_request(&main, 5, "Roster source");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let request =
        serde_json::json!({ "server": found["server"], "expectedContext": found["context"] });
    let legacy = invite_command(
        &main,
        "get_members",
        serde_json::json!({ "server": found["server"] }),
    );
    let tauri::ipc::InvokeResponse::Err(error) = legacy.await.unwrap() else {
        panic!("legacy membership uses the existing backend");
    };
    assert_eq!(error.0, serde_json::json!("existing backend"));
    refused(
        invite_command(
            &main,
            "get_native_chat_members",
            serde_json::json!({
                "server": found["server"], "expectedContext": "73".repeat(32),
            }),
        )
        .await
        .unwrap(),
    );
    let other =
        tauri::WebviewWindowBuilder::new(&app, "unregistered-membership", Default::default())
            .build()
            .unwrap();
    refused(
        invite_command(&other, "get_native_chat_members", request.clone())
            .await
            .unwrap(),
    );
    assert!(queue.lock().unwrap().is_empty());
    let roster = command_value_after_busy(
        &main,
        &queue,
        "get_native_chat_members",
        membership::BUSY,
        request.clone(),
    )
    .await;
    let [original] = roster["members"].as_array().unwrap().as_slice() else {
        panic!("selected genesis has one original founder");
    };
    assert_eq!(original["founder"], true);
    assert_eq!(original["you"], true);
    assert_eq!(roster["canRemove"], false);
    assert_eq!(original["admission"].as_str().unwrap().len(), 64);
    let saved = native_send_request(&main, &found, 45, "Saved before the first Add");
    run(&queue).await;
    let saved = found_value(saved.await.unwrap());

    let stale = invite_command(&main, "get_native_chat_members", request.clone());
    let worker = run(&queue);
    let state = app.state::<AppState>();
    let _close = register_vault_close(&state).await.unwrap();
    worker.await;
    refused(stale.await.unwrap());
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop(_close);
    drop(state);
    drop((other, main, app));

    let (reopened, window, cold_queue) = self::app(path, None, true);
    unlock_current(&reopened, &window, &cold_queue).await;
    let conflicting = invite_command(
        &window,
        "found_server",
        serde_json::json!({
            "requestId": found["context"], "displayName": "Founder", "serverName": "Roster source",
            "advertise": "127.0.0.1:46955", "relay": "", "rendezvous": "", "isDm": false,
        }),
    );
    run(&cold_queue).await;
    refused(conflicting.await.unwrap());
    let exact = found_request(&window, 5, "Roster source");
    run(&cold_queue).await;
    assert_eq!(found_value(exact.await.unwrap()), found);
    let restored = command_value_after_busy(
        &window,
        &cold_queue,
        "get_native_chat_members",
        membership::BUSY,
        request.clone(),
    )
    .await;
    assert_eq!(restored, roster);
    let resumed = resume_request(&window);
    run(&cold_queue).await;
    let resumed = found_value(resumed.await.unwrap());
    let [cold] = resumed["servers"].as_array().unwrap().as_slice() else {
        panic!("the original founder is installed after cold unlock");
    };
    assert_eq!(cold["server"], found["server"]);
    assert_eq!(cold["context"], found["context"]);
    assert_eq!(cold["channel"], found["channel"]);
    let state = reopened.state::<AppState>();
    let server = cold["server"].as_u64().unwrap();
    let request_digest = {
        let table = state.servers.lock().await;
        let row = table.native(&server).unwrap();
        row.data().selection.founder().unwrap().request_data()
    };
    let mut wrong = cold.clone();
    wrong["context"] = serde_json::json!(hex::encode(request_digest));
    assert_ne!(wrong["context"], cold["context"]);
    let refused_history = native_history_request(&window, &wrong, "0", None, 1);
    refused(refused_history.await.unwrap());
    assert!(cold_queue.lock().unwrap().is_empty());
    let local = joined_work(&state, server).await;
    let account =
        keep_session::current(&state, state.ui_session_generation.load(Ordering::SeqCst)).unwrap();
    let foreground = local.foreground(account).await.unwrap();
    let page = native_history_request(&window, cold, "0", None, 1);
    let work = cold_queue.lock().unwrap().pop();
    drop(foreground);
    if let Some(work) = work {
        work.await;
    }
    let page = found_value(page.await.unwrap());
    assert_eq!(page["items"][0]["action"], saved["message_id"]);
    assert_eq!(
        page["items"][0]["body"]["text"],
        "Saved before the first Add"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_founder_frame_capture_survives_transient_row_contention() {
    let root = tempfile::tempdir().unwrap();
    let (app, window, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &window, &queue).await;
    let found = found_request(&window, 1, "Original receiver");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let state = app.state::<AppState>();
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let table = state.servers.lock().await;
    let row = table.native(&found["server"].as_u64().unwrap()).unwrap();
    let source = connected::Source::for_row(
        generation,
        &state.native_text.ui_state.original_target().unwrap(),
        row,
        keep_session::current(&state, generation).unwrap(),
        &mut Some(state.native_text.ui_budget.prepare().unwrap()),
    )
    .unwrap();

    let mut capture = Box::pin(receiving::capture_founder_frame(&state, &source));
    pending_once(capture.as_mut()).await;
    drop(table);
    let (permit, prepared) = timeout(Duration::from_secs(5), capture)
        .await
        .expect("capture retained the original receiver while the table was busy")
        .unwrap();
    let caller = source.negative();
    let roster = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        prepared.governed_roster(&caller)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(roster.members().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_membership_selected_eviction_notifies_the_original_joined_row() {
    native_text_membership_removal_reopen(MembershipAfterRemove::Retired).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_recovered_founder_admits_a_fresh_join_after_selected_remove() {
    native_text_membership_removal_reopen(MembershipAfterRemove::NewMember).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_cold_founder_finishes_a_complete_staged_add() {
    native_text_membership_removal_reopen(MembershipAfterRemove::StagedAdd).await;
}

struct RetainedRemovalHistory {
    visible: serde_json::Value,
    selected_edit: serde_json::Value,
    deleted: serde_json::Value,
    deleted_audit: serde_json::Value,
}

fn removal_history_args(
    row: &serde_json::Value,
    mut arguments: serde_json::Value,
) -> serde_json::Value {
    arguments["server"] = row["server"].clone();
    arguments["channel"] = row["channel"].clone();
    arguments["expectedContext"] = row["context"].clone();
    arguments
}

async fn check_retained_removal_history(
    window: &tauri::WebviewWindow<MockRuntime>,
    queue: &Queue,
    row: &serde_json::Value,
    expected: &RetainedRemovalHistory,
) {
    for (action, text) in [
        (&expected.visible, Some("edited before removal")),
        (&expected.deleted, None),
    ] {
        for (key, scope) in [
            ("archivedAction", "received-archive"),
            ("receivedAction", "received-live"),
        ] {
            let mut args = serde_json::json!({});
            args[key] = action.clone();
            let result = command_value_after_busy(
                window,
                queue,
                "get_message_page",
                history::BUSY,
                removal_history_args(row, args),
            )
            .await;
            assert_eq!(result["scope"], scope);
            if let Some(text) = text {
                assert_eq!(&result["item"]["action"], action);
                assert_eq!(result["item"]["text"], text);
            } else {
                assert!(result["item"].is_null());
            }
        }
    }
    let audit = command_value_after_busy(
        window,
        queue,
        "get_message_page",
        history::BUSY,
        removal_history_args(
            row,
            serde_json::json!({"auditAction":expected.deleted,"auditAfter":null,"limit":8}),
        ),
    )
    .await;
    assert_eq!(audit["scope"], "selected-shared-history-audit");
    assert_eq!(audit["items"], expected.deleted_audit);
}

async fn check_reply_to_removed_author(
    window: &tauri::WebviewWindow<MockRuntime>,
    queue: &Queue,
    row: &serde_json::Value,
    target: &serde_json::Value,
    token: u8,
) {
    let text = format!("reply from the current member {token}");
    let reply = command_value_after_busy(
        window,
        queue,
        "send_message",
        sending::BUSY,
        removal_history_args(
            row,
            serde_json::json!({
                "retryToken": hex::encode([token; 32]), "text": text, "replyTo": target,
            }),
        ),
    )
    .await;
    assert_eq!(reply["disposition"], "saved-local");

    let saved = native_history_request(window, row, "0", None, 8);
    run(queue).await;
    let saved = found_value(saved.await.unwrap());
    let original = saved["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["action"] == reply["message_id"])
        .expect("the current signer saved the reply's exact original");
    assert_eq!(original["body"]["reply_to"], *target);
    assert_eq!(original["body"]["text"], text);
}

async fn archive_originals_before_removal(
    (founder_window, founder_queue, founder): (
        &tauri::WebviewWindow<MockRuntime>,
        &Queue,
        &serde_json::Value,
    ),
    (member_window, member_queue, member): (
        &tauri::WebviewWindow<MockRuntime>,
        &Queue,
        &serde_json::Value,
    ),
) -> RetainedRemovalHistory {
    let mut targets = Vec::with_capacity(2);
    let mut selected_edit = serde_json::Value::Null;
    let mut deleted_audit = serde_json::Value::Null;
    for (token, mutation, edited) in [
        (51, "edit_message", Some("edited before removal")),
        (53, "delete_message", None),
    ] {
        let create = command_value_after_busy(
            member_window, member_queue, "send_message", sending::BUSY,
            removal_history_args(member, serde_json::json!({
                "retryToken":hex::encode([token;32]), "text":"original before removal", "replyTo":null,
            })),
        ).await;
        let action = create["message_id"].clone();
        let mut input = serde_json::json!({
            "retryToken":hex::encode([token + 1;32]), "messageId":action, "expectedRevision":action,
        });
        if let Some(text) = edited {
            input["text"] = text.into();
        }
        let revision = command_value_after_busy(
            member_window,
            member_queue,
            mutation,
            sending::BUSY,
            removal_history_args(member, input),
        )
        .await;
        assert_ne!(revision["message_id"], action);
        let records = timeout(Duration::from_secs(30), async {
            loop {
                let audit = command_value_after_busy(
                    founder_window,
                    founder_queue,
                    "get_message_page",
                    history::BUSY,
                    removal_history_args(
                        founder,
                        serde_json::json!({"auditAction":action,"auditAfter":null,"limit":8}),
                    ),
                )
                .await;
                if let Some(items) = audit["items"].as_array() {
                    if items
                        .iter()
                        .any(|item| item["action"] == revision["message_id"])
                    {
                        assert_eq!(items.len(), 2);
                        assert!(items.iter().any(|item| item["action"] == action));
                        break audit["items"].clone();
                    }
                }
                SystemClock.sleep(Duration::from_millis(35)).await;
            }
        })
        .await
        .expect("the founder selected the exact TCP Create and revision before Remove");
        if edited.is_none() {
            deleted_audit = records;
        } else {
            selected_edit = revision["message_id"].clone();
        }
        targets.push(action);
    }
    let [visible, deleted] = targets.try_into().unwrap();
    let history = RetainedRemovalHistory {
        visible,
        selected_edit,
        deleted,
        deleted_audit,
    };
    check_retained_removal_history(founder_window, founder_queue, founder, &history).await;
    history
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_rejoin_exchange_uses_the_cold_original_pending_over_tcp() {
    native_text_membership_removal_reopen(MembershipAfterRemove::RejoinExchange).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_rejoin_received_data_recovers_without_network_or_key_entry() {
    native_text_membership_removal_reopen(MembershipAfterRemove::RejoinReceived).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_rejoin_welcome_covers_a_closed_outcome_and_holds_ambiguous_entry() {
    native_text_membership_removal_reopen(MembershipAfterRemove::RejoinWelcome).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_rejoin_prefix_matches_the_closed_replacement_and_reopens_exactly() {
    native_text_membership_removal_reopen(MembershipAfterRemove::RejoinPrefix).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_rejoin_index_recovers_saved_invite_and_preserves_original_membership() {
    native_text_membership_removal_reopen(MembershipAfterRemove::RejoinIndex).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_rejoin_activates_the_replacement_row_on_startup() {
    native_text_membership_removal_reopen(MembershipAfterRemove::RejoinActive).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_routes_same_vault_rejoin_and_drains_the_original_file_loan() {
    native_text_membership_removal_reopen(MembershipAfterRemove::RejoinRouted).await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MembershipAfterRemove {
    Retired,
    NewMember,
    StagedAdd,
    RejoinExchange,
    RejoinReceived,
    RejoinWelcome,
    RejoinPrefix,
    RejoinIndex,
    RejoinActive,
    RejoinRouted,
}

async fn check_routed_rejoin(
    path: PathBuf,
    old_port: u16,
    founder: (
        &tauri::WebviewWindow<MockRuntime>,
        &Queue,
        &serde_json::Value,
    ),
    retired: &serde_json::Value,
) {
    let invite = command_value_after_busy(
        founder.0,
        founder.1,
        "mint_invite_fresh",
        invite_mint::BUSY,
        serde_json::json!({"server":founder.2["server"],"expectedContext":founder.2["context"]}),
    )
    .await;
    let args = serde_json::json!({"inviteHex":invite,"approvedHost":"127.0.0.1"});
    for (first, expected) in [
        (true, "restart-to-continue"),
        (false, "restart-to-activate"),
    ] {
        let (app, main, queue) = app(path.clone(), None, true);
        unlock_current(&app, &main, &queue).await;
        let state = app.state::<AppState>();
        assert!(ui_session_closed(&state));
        assert!(retired_ui_current(
            &state,
            state.ui_session_generation.load(Ordering::SeqCst)
        ));
        if first {
            let edge = AdmissionBarrier::new();
            *state.native_text.rejoin_file_edge.lock().unwrap() = Some(edge.clone());
            let old = invite_command(&main, "rejoin_server", args.clone());
            let work = tokio::spawn(run(&queue));
            timeout(Duration::from_secs(10), edge.arrived.notified())
                .await
                .unwrap();
            assert!(state.store.lock().await.is_none());
            let mut locked = invite_command(
                &main,
                "lock_session",
                serde_json::json!({"uiStateJson":null}),
            );
            let locking = tokio::spawn(run(&queue));
            timeout(Duration::from_secs(10), async {
                while state.ui_session_generation.load(Ordering::SeqCst) == 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(matches!(
                locked.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ));
            edge.release.notify_one();
            work.await.unwrap();
            locking.await.unwrap();
            assert!(found_value(locked.await.unwrap())["continuity_error"].is_null());
            refused(old.await.unwrap());
            assert!(!path.join("chat-critical-namespaces.rejoin.stage").exists());
            assert!(matches!(
                state.store.lock().await.as_ref(),
                Some(MountedVault::TextV2Retired(_))
            ));
            unlock_current(&app, &main, &queue).await;
        } else {
            let edge = AdmissionBarrier::new();
            *state.native_text.rejoin_network_edge.lock().unwrap() = Some(edge.clone());
            let old = invite_command(&main, "rejoin_server", args.clone());
            let work = tokio::spawn(run(&queue));
            timeout(Duration::from_secs(20), edge.arrived.notified())
                .await
                .unwrap();
            assert!(matches!(
                state.store.lock().await.as_ref(),
                Some(MountedVault::TextV2Retired(_))
            ));
            let locked = invite_command(
                &main,
                "lock_session",
                serde_json::json!({"uiStateJson":null}),
            );
            timeout(Duration::from_secs(10), run(&queue))
                .await
                .expect("lock does not join the original TCP reply");
            assert!(found_value(locked.await.unwrap())["continuity_error"].is_null());
            // Authenticate another generation while a genuine, verified
            // response still belongs to the retired invocation.
            unlock_current(&app, &main, &queue).await;
            assert!(!work.is_finished());
            edge.release.notify_one();
            work.await.unwrap();
            refused(old.await.unwrap());
            assert!(!path
                .join("chat-critical-namespaces.rejoin-received.bin")
                .exists());
        }
        let reply = invite_command(&main, "rejoin_server", args.clone());
        timeout(Duration::from_secs(60), run(&queue))
            .await
            .expect("real native rejoin cut");
        assert_eq!(
            found_value(reply.await.unwrap()),
            serde_json::json!({"step":expected})
        );
        assert!(state.servers.lock().await.is_empty());
        let close = register_vault_close(&state).await.unwrap();
        sources::close_server_rows(&state, &SystemClock)
            .await
            .unwrap();
        drop((close, state));
        assert!(catcoms_storage::vault::acquire_vault_session(&path).is_ok());
    }
    let invite = hex::decode(invite.as_str().unwrap()).unwrap();
    check_active_rejoin(path, old_port, founder, (retired, None, &invite)).await;
}

async fn check_active_rejoin(
    path: std::path::PathBuf,
    old_port: u16,
    (founder, founder_queue, founder_row): (
        &tauri::WebviewWindow<MockRuntime>,
        &Queue,
        &serde_json::Value,
    ),
    (retired, expected, invite): (&serde_json::Value, Option<u64>, &[u8]),
) {
    // Rejoining must work even if another process owns A1's old port.
    let _occupied_a1 = std::net::TcpListener::bind(("0.0.0.0", old_port)).unwrap();
    let (member, window, queue) = app(path, None, true);
    unlock_current(&member, &window, &queue).await;
    let resume = resume_request(&window);
    run(&queue).await;
    let result = found_value(resume.await.unwrap());
    let rows = result["servers"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let active = &rows[0];
    let active_id = active["server"].as_u64().expect("a new registered row");
    if let Some(expected) = expected {
        assert_eq!(active_id, expected);
    }
    assert_ne!(active["context"], retired["context"]);
    let old = native_send_request(&window, retired, 190, "The removed row cannot publish");
    let old_work = queue.lock().unwrap().pop();
    if let Some(work) = old_work {
        work.await;
    }
    refused(old.await.unwrap());

    let exact = invite_command(
        &window,
        "join_server",
        serde_json::json!({
            "inviteHex": hex::encode(invite), "approvedHost": "127.0.0.1",
            "displayName": "Member", "serverName": "Joined group", "isDm": false,
            "allowSwitchboards": false,
        }),
    );
    run(&queue).await;
    let returned = found_value(exact.await.unwrap());
    assert_eq!(returned["server"].as_u64(), Some(active_id));
    assert_eq!(returned["context"], active["context"]);

    let post = native_send_request(&window, active, 191, "Saved by the replacement admission");
    run(&queue).await;
    let post = found_value(post.await.unwrap());
    assert_eq!(post["disposition"], "saved-local");
    let reply = native_send_request(
        founder,
        founder_row,
        192,
        "Received by the replacement admission",
    );
    run(founder_queue).await;
    let reply = found_value(reply.await.unwrap());
    assert_eq!(reply["disposition"], "saved-local");
    async fn wait_for_archive(
        window: &tauri::WebviewWindow<MockRuntime>,
        queue: &Queue,
        row: &serde_json::Value,
        action: &serde_json::Value,
    ) -> serde_json::Value {
        timeout(Duration::from_secs(40), async {
            loop {
                let item = command_value_after_busy(
                    window,
                    queue,
                    "get_message_page",
                    history::BUSY,
                    removal_history_args(row, serde_json::json!({"archivedAction": action})),
                )
                .await;
                if !item["item"].is_null() {
                    return item;
                }
                SystemClock.sleep(Duration::from_millis(40)).await;
            }
        })
        .await
        .expect("the other native member selects the exact original under A2")
    }
    let found = wait_for_archive(founder, founder_queue, founder_row, &post["message_id"]).await;
    assert_eq!(found["item"]["text"], "Saved by the replacement admission");
    let joined = wait_for_archive(&window, &queue, active, &reply["message_id"]).await;
    assert_eq!(
        joined["item"]["text"],
        "Received by the replacement admission"
    );

    // The next governing operation uses A2's own covered Outcome before
    // retiring the replacement and emitting its terminal notification.
    use tauri::Listener;
    let (notice, removed) = oneshot::channel();
    member.once_any("chat-v2-membership", move |event| {
        let value: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
        let _ = notice.send(value);
    });
    let roster = command_value_after_busy(
        founder, founder_queue, "get_native_chat_members", membership::BUSY,
        serde_json::json!({"server": founder_row["server"], "expectedContext": founder_row["context"]}),
    ).await;
    let replacement = roster["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["founder"] == false)
        .expect("the replacement is the only nonfounder admission");
    let result = command_value_after_busy(
        founder,
        founder_queue,
        "remove_native_chat_member",
        membership::BUSY,
        serde_json::json!({
            "server": founder_row["server"], "expectedContext": founder_row["context"],
            "expectedAdmission": replacement["admission"],
        }),
    )
    .await;
    assert!(result.is_null());
    let removed = timeout(Duration::from_secs(40), removed)
        .await
        .expect("the covered A2 self-Remove publishes its terminal result")
        .unwrap();
    assert_eq!(removed["server"].as_u64(), Some(active_id));
    assert_eq!(removed["context"], active["context"]);
    assert_eq!(removed["removed"], true);

    let state = member.state::<AppState>();
    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop((close, state));
}

async fn native_text_membership_removal_reopen(scenario: MembershipAfterRemove) {
    let join_after_restart = matches!(
        scenario,
        MembershipAfterRemove::NewMember | MembershipAfterRemove::StagedAdd
    );
    let repair_staged_add = scenario == MembershipAfterRemove::StagedAdd;
    use catcoms_app::store::NativeTextInvitePacketData;
    use tauri::Listener;

    let root = tempfile::tempdir().unwrap();
    let founder_path = root.path().join("founder");
    let (founder, main, founder_queue) = app(founder_path.clone(), None, true);
    let member_path = root.path().join("member");
    let (member, window, member_queue) = app(member_path.clone(), None, true);
    unlock_current(&founder, &main, &founder_queue).await;
    unlock_current(&member, &window, &member_queue).await;
    let found = found_request(&main, 1, "Founder group");
    run(&founder_queue).await;
    let found = found_value(found.await.unwrap());
    let invite = invite_command(
        &main,
        "mint_invite_fresh",
        serde_json::json!({"server": found["server"], "expectedContext": found["context"]}),
    );
    run(&founder_queue).await;
    let invite = found_value(invite.await.unwrap());
    let (notice, selected_add) = oneshot::channel();
    founder.once_any("chat-v2-membership", move |event| {
        let notice_data: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
        let _ = notice.send(notice_data);
    });
    let joined = invite_command(
        &window,
        "join_server",
        serde_json::json!({
            "inviteHex": invite, "approvedHost": "127.0.0.1", "displayName": "Member",
            "serverName": "Joined group", "isDm": false, "allowSwitchboards": false,
        }),
    );
    timeout(Duration::from_secs(45), run(&member_queue))
        .await
        .expect("the member joins over the founder's original TCP listener");
    let joined = found_value(joined.await.unwrap());
    assert_eq!(
        timeout(Duration::from_secs(10), selected_add)
            .await
            .unwrap()
            .unwrap(),
        serde_json::json!({"server": found["server"], "context": found["context"], "removed": false}),
    );

    let roster = command_value_after_busy(
        &main,
        &founder_queue,
        "get_native_chat_members",
        membership::BUSY,
        serde_json::json!({"server": found["server"], "expectedContext": found["context"]}),
    )
    .await;
    let admission = roster["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["founder"] == false)
        .expect("the selected roster names the joined admission");

    let retained_history = if scenario == MembershipAfterRemove::Retired {
        Some(
            archive_originals_before_removal(
                (&main, &founder_queue, &found),
                (&window, &member_queue, &joined),
            )
            .await,
        )
    } else {
        None
    };
    if let Some(history) = &retained_history {
        use catcoms_replication::chat::ActionId;
        let selected = |value: &serde_json::Value| {
            ActionId::from_bytes(membership::decode_exact_hex(value.as_str().unwrap()).unwrap())
        };
        let target = selected(&history.visible);
        let edit = selected(&history.selected_edit);
        let unresolved = ActionId::from_bytes([235; 32]);
        let state = founder.state::<AppState>();
        let work = state
            .servers
            .lock()
            .await
            .native(&found["server"].as_u64().unwrap())
            .unwrap()
            .data()
            .member_work
            .clone();
        assert_eq!(work.reserve_archive_change(edit, target), Some(true));
        assert_eq!(work.reserve_archive_change(unresolved, target), Some(true));
        let incomplete = command_after_busy(
            &main,
            &founder_queue,
            "get_message_page",
            history::BUSY,
            removal_history_args(
                &found,
                serde_json::json!({"receivedAction":history.visible}),
            ),
        )
        .await;
        let tauri::ipc::InvokeResponse::Err(error) = incomplete else {
            panic!("one selected revision must not hide another unresolved revision");
        };
        assert_eq!(error.0, "A received change is waiting to be saved.");
        let outstanding = work.unsettled_archive_changes(target).unwrap();
        assert_eq!(
            outstanding.into_iter().flatten().collect::<Vec<_>>(),
            vec![unresolved]
        );
        work.settle_archive_change(unresolved, target);
    }

    let (notice, selected_eviction) = oneshot::channel();
    member.once_any("chat-v2-membership", move |event| {
        let notice_data: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
        let _ = notice.send(notice_data);
    });
    let removal = command_value_after_busy(
        &main,
        &founder_queue,
        "remove_native_chat_member",
        membership::BUSY,
        serde_json::json!({
            "server": found["server"], "expectedContext": found["context"],
            "expectedAdmission": admission["admission"],
        }),
    )
    .await;
    assert!(removal.is_null());
    let selected_eviction = timeout(Duration::from_secs(40), selected_eviction)
        .await
        .expect("closing the joined worker must preserve the selected eviction notice")
        .unwrap();
    assert_eq!(
        selected_eviction,
        serde_json::json!({
            "server": joined["server"], "context": joined["context"], "removed": true,
        })
    );
    let send = native_send_request(&window, &joined, 45, "Removed admission cannot send");
    let pending = member_queue.lock().unwrap().pop();
    if let Some(work) = pending {
        work.await;
    }
    refused(send.await.unwrap());
    if let Some(history) = &retained_history {
        check_retained_removal_history(&main, &founder_queue, &found, history).await;
        check_reply_to_removed_author(&main, &founder_queue, &found, &history.visible, 91).await;
    }

    // Only the authenticated selected A1 Remove explains why this row
    // cannot be activated. An outcome with its selector removed is still
    // ambiguous and must not be classified as an eligible retired member.
    let state = member.state::<AppState>();
    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop((close, state));
    drop((window, member));
    let removed_response = |path: std::path::PathBuf| async move {
        let (app, main, queue) = app(path, None, true);
        let state = app.state::<AppState>();
        let generation = state
            .ui_session_generation
            .load(Ordering::SeqCst)
            .to_string();
        let reply = invoke(&main, &generation, SECRET);
        run(&queue).await;
        match reply.await.unwrap() {
            tauri::ipc::InvokeResponse::Err(error) => {
                assert!(state.store.lock().await.is_none());
                error.0
            }
            accepted @ tauri::ipc::InvokeResponse::Ok(_) => {
                assert!(matches!(
                    state.store.lock().await.as_ref(),
                    Some(MountedVault::TextV2Retired(_))
                ));
                assert!(state.servers.lock().await.is_empty());
                let data = found_value(accepted);
                assert!(ui_session_closed(&state));
                assert!(require_unlocked_session(&state).await.is_err());
                let post = invite_command(&main, "send_message", serde_json::json!({}));
                assert!(matches!(
                    post.await.unwrap(),
                    tauri::ipc::InvokeResponse::Err(_)
                ));
                let resume = invite_command(&main, "resume_session", serde_json::json!({}));
                run(&queue).await;
                assert_eq!(found_value(resume.await.unwrap()), data);
                let lock = invite_command(
                    &main,
                    "lock_session",
                    serde_json::json!({"unexpected": true}),
                );
                run(&queue).await;
                let lock = found_value(lock.await.unwrap());
                assert!(lock["continuity_error"].is_null());
                let resume = invite_command(&main, "resume_session", serde_json::json!({}));
                assert_eq!(found_value(resume.await.unwrap()), serde_json::Value::Null);
                let generation = lock["generation"].as_str().unwrap();
                let wrong = invoke(&main, generation, "wrong retired secret");
                run(&queue).await;
                assert!(matches!(
                    wrong.await.unwrap(),
                    tauri::ipc::InvokeResponse::Err(_)
                ));
                let reopen = invoke(&main, generation, SECRET);
                run(&queue).await;
                assert_eq!(found_value(reopen.await.unwrap()), data);
                let credit = state.native_text.ui_budget.prepare().unwrap();
                let authentication_work = {
                    let store = state.store.lock().await;
                    let Some(MountedVault::TextV2Retired(vault)) = store.as_ref() else {
                        panic!("the recovery session keeps its original affinity");
                    };
                    vault.prepare_native_authentication(credit)
                };
                let close = register_vault_close(&state).await.unwrap();
                let (result, ()) =
                    tokio::join!(sources::close_server_rows(&state, &SystemClock), async {
                        timeout(
                            Duration::from_secs(5),
                            state.native_text.source_work_waiting.notified(),
                        )
                        .await
                        .expect("close waits for the retired mount's actual authentication work");
                        drop(authentication_work);
                    },);
                result.unwrap();
                drop(close);
                assert!(state.store.lock().await.is_none());
                data
            }
        }
    };
    let retired_reply = serde_json::json!({"backend":"text-v2","servers":[],"recovery":"removed"});
    assert_eq!(removed_response(member_path.clone()).await, retired_reply,);
    let wait_for_original_vault = || async {
        timeout(Duration::from_secs(5), async {
            loop {
                match catcoms_storage::vault::acquire_vault_session(&member_path) {
                    Ok(session) => return drop(session),
                    Err(catcoms_storage::StorageError::VaultBusy) => {
                        SystemClock.sleep(Duration::from_millis(20)).await;
                    }
                    Err(error) => panic!("original vault session: {error}"),
                }
            }
        })
        .await
        .expect("the previous native TCP owner released the vault");
    };
    wait_for_original_vault().await;
    if scenario == MembershipAfterRemove::RejoinRouted {
        let original = catcoms_app::store::NativeTextJoinedStorage::open_for_native_startup(
            &member_path,
            SECRET.as_bytes(),
            &mut OsCryptoRng,
            boot::current_policy().unwrap(),
        )
        .unwrap();
        let (_, port) = original.recovered_join_transport_data().unwrap().unwrap();
        drop(original);
        check_routed_rejoin(member_path, port, (&main, &founder_queue, &found), &joined).await;
        return;
    }
    let selector = std::fs::read_dir(&member_path)
        .unwrap()
        .find_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| {
                    name.starts_with("chat-critical-namespaces.joined-member-selected.")
                        && name.ends_with(".2.bin")
                })
                .then(|| entry.path())
        })
        .expect("the covered A1 removal has an original selector");
    let parked = selector.with_extension("held-for-test");
    std::fs::rename(&selector, &parked).unwrap();
    assert_eq!(
        removed_response(member_path.clone()).await,
        serde_json::json!("Could not unlock the existing Chat V2 vault."),
    );
    assert!(parked.is_file() && !selector.exists());
    std::fs::rename(parked, &selector).unwrap();
    wait_for_original_vault().await;
    let body = selector.with_file_name(
        selector
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .replace("joined-member-selected", "joined-member-add"),
    );
    let parked_body = body.with_extension("held-for-test");
    std::fs::rename(&body, &parked_body).unwrap();
    assert_eq!(
        removed_response(member_path.clone()).await,
        serde_json::json!("Could not unlock the existing Chat V2 vault."),
    );
    assert!(parked_body.is_file() && selector.is_file() && !body.exists());
    std::fs::rename(parked_body, body).unwrap();
    wait_for_original_vault().await;
    assert_eq!(removed_response(member_path.clone()).await, retired_reply,);

    if !join_after_restart {
        use catcoms_app::store::{NativeTextJoinedRecovery, NativeTextJoinedStorage};
        wait_for_original_vault().await;
        let catalog = member_path.join("chat-critical-namespaces.bin");
        let before = std::fs::read(&catalog).unwrap();
        let old_port = std::cell::Cell::new(0);
        let open_retired_with_ui = |atoms, inspect_index| {
            let mut rng = OsCryptoRng;
            let registry = Registry::default();
            let storage = NativeTextJoinedStorage::open_for_native_startup(
                &member_path,
                SECRET.as_bytes(),
                &mut rng,
                boot::current_policy().unwrap(),
            )
            .unwrap();
            let retired_port = storage.recovered_join_transport_data().unwrap().unwrap().1;
            old_port.set(retired_port);
            let (_lifetime, signal) = tokio::sync::watch::channel(false);
            let negative = catcoms_rt::RequestCancellation::new(signal, None);
            let recovered = if inspect_index {
                // This fixture must inspect a covered Index4 from the retired
                // writer. Ordinary startup selects its replacement instead.
                storage
                    .bind_recovered_native_runtime(OsCryptoRng, Box::new(SystemClock), &negative)
                    .map_err(|(_, error)| error)
                    .unwrap()
                    .into_recovered_joined_membership(
                        atoms,
                        registry.ui_budget.clone(),
                        &registry.sources.policy(),
                        &negative,
                    )
                    .unwrap()
            } else {
                let _unavailable_old_port = std::net::TcpListener::bind(("0.0.0.0", retired_port))
                    .expect("an unrelated listener can own the retired A1 port");
                storage
                    .into_native_startup_membership(
                        (atoms, None),
                        registry.ui_budget.clone(),
                        &registry.sources.policy(),
                        &negative,
                        (OsCryptoRng, Box::new(SystemClock)),
                        (OsCryptoRng, Box::new(SystemClock)),
                    )
                    .unwrap()
            };
            let NativeTextJoinedRecovery::Removed(retired) = recovered else {
                panic!("a proven terminal row must retain retired storage only");
            };
            assert!(
                inspect_index || retired.removal_data().unwrap().is_some(),
                "offline replay must never wait on the unavailable A1 TCP actor"
            );
            (retired, registry.ui_budget.clone())
        };
        let open_retired = || {
            open_retired_with_ui(
                (
                    Arc::new(AtomicU64::new(1)),
                    Arc::new(AtomicU64::new(0)),
                    Arc::new(AtomicBool::new(false)),
                ),
                false,
            )
            .0
        };
        let mut retired = open_retired();
        let mut rng = OsCryptoRng;
        timeout(Duration::from_secs(10), async {
            while !retired.prepare_rejoin_catalog(&mut rng).unwrap() {
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the original transport drains before rejoin preparation");
        struct NeverRng;
        impl rand_core::RngCore for NeverRng {
            fn next_u32(&mut self) -> u32 {
                panic!("retired catalog retry drew randomness")
            }
            fn next_u64(&mut self) -> u64 {
                panic!("retired catalog retry drew randomness")
            }
            fn fill_bytes(&mut self, _: &mut [u8]) {
                panic!("retired catalog retry drew randomness")
            }
            fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
                panic!("retired catalog retry drew randomness")
            }
        }
        impl rand_core::CryptoRng for NeverRng {}
        struct RevokeAfterJournal<'a> {
            path: &'a Path,
            signal: &'a tokio::sync::watch::Sender<bool>,
        }
        impl RevokeAfterJournal<'_> {
            fn before_draw(&self) {
                if self.path.is_file() {
                    self.signal.send_replace(true);
                }
            }
        }
        impl rand_core::RngCore for RevokeAfterJournal<'_> {
            fn next_u32(&mut self) -> u32 {
                self.before_draw();
                OsCryptoRng.next_u32()
            }
            fn next_u64(&mut self) -> u64 {
                self.before_draw();
                OsCryptoRng.next_u64()
            }
            fn fill_bytes(&mut self, dest: &mut [u8]) {
                self.before_draw();
                OsCryptoRng.fill_bytes(dest);
            }
            fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
                self.before_draw();
                OsCryptoRng.try_fill_bytes(dest)
            }
        }
        impl rand_core::CryptoRng for RevokeAfterJournal<'_> {}
        let proof = retired.removal_data().unwrap().unwrap();
        assert!(retired.cover_selected_rejoin_catalog().is_err());
        assert_eq!(proof.2, 2);
        assert_eq!(std::fs::read(&catalog).unwrap(), before);
        assert!(!member_path
            .join("chat-critical-namespaces.replace.tmp")
            .exists());

        let selector_bytes = std::fs::read(&selector).unwrap();
        let selector_backup = selector.with_extension("original-retired-owner-test");
        std::fs::rename(&selector, &selector_backup).unwrap();
        std::fs::write(&selector, &selector_bytes).unwrap();
        assert!(
            retired.removal_data().is_err(),
            "equal bytes cannot replace the same held selector"
        );
        assert!(retired.prepare_rejoin_catalog(&mut NeverRng).is_err());
        std::fs::remove_file(&selector).unwrap();
        std::fs::rename(&selector_backup, &selector).unwrap();

        let file_name = selector.file_name().unwrap().to_str().unwrap();
        let fork = file_name
            .replace("joined-member-selected", "joined-member-header")
            .replace(".2.bin", ".2.conflict.bin");
        let fork = member_path.join(fork);
        std::fs::write(&fork, b"unresolved late record").unwrap();
        assert!(
            retired.removal_data().is_err(),
            "late governing evidence suspends the old proof"
        );
        assert_eq!(std::fs::read(&fork).unwrap(), b"unresolved late record");
        std::fs::remove_file(fork).unwrap();
        assert_eq!(retired.removal_data().unwrap(), Some(proof));
        assert!(retired.prepare_rejoin_catalog(&mut NeverRng).unwrap());
        let stage = member_path.join("chat-critical-namespaces.rejoin.stage");
        assert!(!stage.exists());
        assert!(retired.stage_rejoin_catalog(&mut NeverRng).unwrap());
        let bytes = std::fs::read(&stage).unwrap();
        assert_eq!(std::fs::read(&catalog).unwrap(), before);
        assert_ne!(bytes, before);
        let held_stage = stage.with_extension("held-stage-for-test");
        std::fs::rename(&stage, &held_stage).unwrap();
        std::fs::write(&stage, &bytes).unwrap();
        assert!(retired.stage_rejoin_catalog(&mut NeverRng).is_err());
        assert_eq!(std::fs::read(&stage).unwrap(), bytes);
        std::fs::remove_file(&stage).unwrap();
        std::fs::rename(&held_stage, &stage).unwrap();
        assert!(retired.stage_rejoin_catalog(&mut NeverRng).unwrap());
        drop(retired);
        wait_for_original_vault().await;

        let mut reopened = open_retired();
        timeout(Duration::from_secs(10), async {
            while !reopened.stage_rejoin_catalog(&mut NeverRng).unwrap() {
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("a complete authenticated Stage is reused on cold open");
        assert_eq!(std::fs::read(&stage).unwrap(), bytes);
        assert_eq!(std::fs::read(&catalog).unwrap(), before);
        drop(reopened);
        wait_for_original_vault().await;

        // A fresh owner has no authority to finish a short Stage write.
        let partial = &bytes[..bytes.len() / 2];
        std::fs::write(&stage, partial).unwrap();
        let mut retired = open_retired();
        timeout(Duration::from_secs(10), async {
            loop {
                match retired.stage_rejoin_catalog(&mut NeverRng) {
                    Err(_) => break,
                    Ok(false) => SystemClock.sleep(Duration::from_millis(10)).await,
                    Ok(true) => panic!("partial cold Stage claimed a completed write"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(&stage).unwrap(), partial);
        assert_eq!(std::fs::read(&catalog).unwrap(), before);
        drop(retired);
        wait_for_original_vault().await;

        // Select the same complete Stage through the production writer.
        // The old mount retains only its historical proof after the rename.
        let replacement = command_value_after_busy(
            &main,
            &founder_queue,
            "mint_invite_fresh",
            invite_mint::BUSY,
            serde_json::json!({"server": found["server"], "expectedContext": found["context"]}),
        )
        .await;
        let replacement_bytes = hex::decode(replacement.as_str().unwrap()).unwrap();
        let replacement = NativeTextInvitePacketData::decode(&replacement_bytes).unwrap();
        let now = replacement.binding().spec_data().expires_at_ms - 1;
        let catalog_lock = Arc::new(AtomicBool::new(false));
        let catalog_generation = Arc::new(AtomicU64::new(1));
        let catalog_authorized = Arc::new(AtomicU64::new(1));
        std::fs::write(&stage, &bytes).unwrap();
        let mut retired = open_retired();
        timeout(Duration::from_secs(10), async {
            while !retired.stage_rejoin_catalog(&mut NeverRng).unwrap() {
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let conflict = member_path.join("chat-critical-namespaces.replace.tmp");
        std::fs::write(&conflict, b"unresolved competing catalog").unwrap();
        assert!(retired.select_rejoin_catalog(&mut NeverRng).is_err());
        assert_eq!(std::fs::read(&catalog).unwrap(), before);
        assert_eq!(std::fs::read(&stage).unwrap(), bytes);
        std::fs::remove_file(conflict).unwrap();
        drop(retired);
        wait_for_original_vault().await;
        std::fs::remove_file(&stage).unwrap();
        let (mut retired, catalog_budget) = open_retired_with_ui(
            (
                catalog_generation.clone(),
                catalog_authorized.clone(),
                catalog_lock.clone(),
            ),
            false,
        );
        timeout(Duration::from_secs(10), async {
            while retired.removal_data().unwrap().is_none() {
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        assert!(retired
            .prepare_native_authentication(catalog_budget.prepare().unwrap())
            .verify(b"wrong retired vault secret")
            .is_err());
        let authentication = retired
            .prepare_native_authentication(catalog_budget.prepare().unwrap())
            .verify(SECRET.as_bytes())
            .unwrap();
        assert!(retired.matches_native_authentication_cached(&authentication));
        let mut catalog_capture = retired
            .prepare_rejoin_ui(1, catalog_budget.prepare().unwrap())
            .finish()
            .unwrap();
        let catalog_ui = retired.admit_rejoin_ui(&mut catalog_capture).unwrap();
        let (_invocation, signal) = tokio::sync::watch::channel(false);
        let caller = catcoms_rt::RequestCancellation::new(signal, None);
        let first_packet = hex::decode(invite.as_str().unwrap()).unwrap();
        let first_packet = NativeTextInvitePacketData::decode(&first_packet).unwrap();
        assert!(
            retired
                .select_rejoin_catalog_for_invite(
                    &catalog_ui,
                    &first_packet,
                    0,
                    &caller,
                    &mut NeverRng,
                )
                .is_err()
        );
        assert!(matches!(
            retired.select_rejoin_catalog_for_invite(
                &catalog_ui,
                &replacement,
                u64::MAX,
                &caller,
                &mut NeverRng,
            ),
            Err(catcoms_app::AppError::ChatV2RejoinNeedsFreshInvite)
        ));
        assert_eq!(std::fs::read(&catalog).unwrap(), before);
        assert!(!stage.exists());
        #[derive(Debug)]
        struct FirstStageOnly {
            path: PathBuf,
            revoked: AtomicBool,
        }
        impl FirstStageOnly {
            fn new(path: PathBuf) -> Self {
                Self {
                    path,
                    revoked: AtomicBool::new(false),
                }
            }
        }
        #[async_trait::async_trait]
        impl catcoms_rt::RequestAuthority for FirstStageOnly {
            fn authorized(&self) -> bool {
                if self.path.is_file() {
                    self.revoked.store(true, Ordering::SeqCst);
                }
                !self.revoked.load(Ordering::SeqCst)
            }
            async fn revoked(&self) {
                std::future::pending::<()>().await;
            }
        }
        // The original caller can create and sync Stage. Its authority
        // refuses the independent rename as soon as the actual name exists.
        let cancelled_after_stage = caller
            .clone()
            .with_authority(Arc::new(FirstStageOnly::new(stage.clone())));
        assert!(retired
            .select_rejoin_catalog_for_invite(
                &catalog_ui,
                &replacement,
                now,
                &cancelled_after_stage,
                &mut rng,
            )
            .is_err());
        let bytes = std::fs::read(&stage).unwrap();
        assert_eq!(std::fs::read(&catalog).unwrap(), before);
        assert_ne!(bytes, before);
        catalog_lock.store(true, Ordering::SeqCst);
        assert!(retired
            .select_rejoin_catalog_for_invite(
                &catalog_ui,
                &replacement,
                now,
                &caller,
                &mut NeverRng,
            )
            .is_err());
        catalog_generation.store(2, Ordering::SeqCst);
        catalog_authorized.store(2, Ordering::SeqCst);
        catalog_lock.store(false, Ordering::SeqCst);
        assert!(retired
            .select_rejoin_catalog_for_invite(
                &catalog_ui,
                &replacement,
                now,
                &caller,
                &mut NeverRng,
            )
            .is_err());
        assert_eq!(std::fs::read(&catalog).unwrap(), before);
        let mut next_capture = retired
            .prepare_rejoin_ui(2, catalog_budget.prepare().unwrap())
            .finish()
            .unwrap();
        let next_ui = retired.admit_rejoin_ui(&mut next_capture).unwrap();
        assert!(retired
            .select_rejoin_catalog_for_invite(&next_ui, &replacement, now, &caller, &mut NeverRng,)
            .unwrap());
        assert!(matches!(
            retired.select_rejoin_catalog_for_invite(
                &next_ui,
                &replacement,
                now,
                &caller,
                &mut NeverRng,
            ),
            Err(catcoms_app::AppError::ChatV2RejoinCatalogReopen)
        ));
        assert!(retired.select_rejoin_catalog(&mut NeverRng).unwrap());
        assert_eq!(std::fs::read(&catalog).unwrap(), bytes);
        assert!(!stage.exists());
        assert!(
            retired.removal_data().is_err(),
            "the old Current lost custody"
        );
        assert!(matches!(
            catcoms_storage::vault::acquire_vault_session(&member_path),
            Err(catcoms_storage::StorageError::VaultBusy)
        ));
        drop((next_ui, next_capture, catalog_ui, catalog_capture, retired));
        wait_for_original_vault().await;

        for relative in [
            "chat-critical-namespaces.replace.tmp",
            "chat-critical-namespaces.native-registry.seed",
            "chat-critical-namespaces.native-index3.seed",
            "chat-owner-v2/native-registry.stage",
        ] {
            let conflict = member_path.join(relative);
            assert!(conflict.parent().unwrap().is_dir());
            assert!(!conflict.exists());
            std::fs::write(&conflict, b"unresolved original work").unwrap();
            let result = NativeTextJoinedStorage::open_for_native_startup(
                &member_path,
                SECRET.as_bytes(),
                &mut NeverRng,
                boot::current_policy().unwrap(),
            );
            assert!(result.is_err(), "ID9 must refuse {relative}");
            assert_eq!(
                std::fs::read(&conflict).unwrap(),
                b"unresolved original work"
            );
            assert_eq!(std::fs::read(&catalog).unwrap(), bytes);
            std::fs::remove_file(conflict).unwrap();
        }
        let reopened = open_retired();
        assert!(!reopened.matches_native_authentication_cached(&authentication));
        drop(authentication);
        timeout(Duration::from_secs(10), async {
            loop {
                if let Some(covered) = reopened.cover_selected_rejoin_catalog().unwrap() {
                    assert_eq!(covered, *blake3::hash(&bytes).as_bytes());
                    assert_eq!(reopened.removal_data().unwrap(), Some(proof));
                    break;
                }
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cold startup covers the original ID9 and retains only terminal A1");
        let foreign = catalog.with_extension("held-cold-rejoin-for-test");
        std::fs::rename(&catalog, &foreign).unwrap();
        std::fs::write(&catalog, &bytes).unwrap();
        assert!(reopened.cover_selected_rejoin_catalog().is_err());
        std::fs::remove_file(&catalog).unwrap();
        std::fs::rename(foreign, &catalog).unwrap();
        assert_eq!(
            reopened.cover_selected_rejoin_catalog().unwrap(),
            Some(*blake3::hash(&bytes).as_bytes())
        );
        assert_eq!(std::fs::read(&catalog).unwrap(), bytes);
        assert!(!stage.exists());
        drop(reopened);
        wait_for_original_vault().await;

        let generation = Arc::new(AtomicU64::new(1));
        let authorized = Arc::new(AtomicU64::new(1));
        let lock = Arc::new(AtomicBool::new(false));
        let atoms = || (generation.clone(), authorized.clone(), lock.clone());
        let open = || open_retired_with_ui(atoms(), false);
        let (mut retired, ui_budget) = open();
        timeout(Duration::from_secs(10), async {
            while retired.removal_data().unwrap().is_none() {
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut candidate = retired
            .prepare_rejoin_ui(1, ui_budget.prepare().unwrap())
            .finish()
            .unwrap();
        let old_ui = retired.admit_rejoin_ui(&mut candidate).unwrap();
        let (_live, signal) = tokio::sync::watch::channel(false);
        let caller = catcoms_rt::RequestCancellation::new(signal, None);
        let attempt = member_path.join("chat-critical-namespaces.rejoin-attempt.bin");
        let pending = member_path.join("chat-critical-namespaces.rejoin-pending.bin");
        let token = if scenario == MembershipAfterRemove::RejoinActive {
            joining::request_token(&replacement)
        } else {
            [95; 32]
        };
        let inputs = (&replacement, token, now);
        lock.store(true, Ordering::SeqCst);
        assert!(retired
            .cover_rejoin_pending(&old_ui, inputs, &caller, &mut NeverRng)
            .is_err());
        generation.store(2, Ordering::SeqCst);
        authorized.store(2, Ordering::SeqCst);
        lock.store(false, Ordering::SeqCst);
        assert!(retired
            .cover_rejoin_pending(&old_ui, inputs, &caller, &mut NeverRng)
            .is_err());
        assert!(!attempt.exists() && !pending.exists());
        let mut candidate = retired
            .prepare_rejoin_ui(2, ui_budget.prepare().unwrap())
            .finish()
            .unwrap();
        let ui = retired.admit_rejoin_ui(&mut candidate).unwrap();
        let saved = if scenario == MembershipAfterRemove::RejoinActive {
            let expired = (&replacement, token, u64::MAX);
            assert!(matches!(
                retired.cover_rejoin_pending(&ui, expired, &caller, &mut NeverRng),
                Err(catcoms_app::AppError::ChatV2RejoinNeedsFreshInvite)
            ));
            assert!(!attempt.exists() && !pending.exists());
            let before_key = caller
                .clone()
                .with_authority(Arc::new(FirstStageOnly::new(attempt.clone())));
            assert!(retired
                .cover_rejoin_pending(&ui, inputs, &before_key, &mut OsCryptoRng)
                .is_err());
            assert!(before_key.is_cancelled() && !attempt.exists() && !pending.exists());

            let (revoke, signal) = tokio::sync::watch::channel(false);
            let original = catcoms_rt::RequestCancellation::new(signal, None);
            let mut rng = RevokeAfterJournal {
                path: &attempt,
                signal: &revoke,
            };
            // The first draw after the synced Attempt follows successful
            // JoinerDevice entry. The original signed Pending must survive,
            // while this invocation can no longer receive a receipt.
            assert!(retired
                .cover_rejoin_pending(&ui, inputs, &original, &mut rng)
                .is_err());
            assert!(original.is_cancelled() && pending.is_file());
            // Authentic covered Pending retains its original request
            // even if the invitation expires before this reply is recovered.
            retired.cover_rejoin_pending(&ui, expired, &caller, &mut NeverRng)
        } else {
            retired.cover_rejoin_pending(&ui, inputs, &caller, &mut OsCryptoRng)
        }
        .unwrap()
        .unwrap();
        assert_ne!(saved.request_digest, [0; 32]);
        let attempt_bytes = std::fs::read(&attempt).unwrap();
        let pending_bytes = std::fs::read(&pending).unwrap();
        for (path, expected) in [(&attempt, &attempt_bytes), (&pending, &pending_bytes)] {
            let original = path.with_extension("held-rejoin-test");
            std::fs::rename(path, &original).unwrap();
            std::fs::write(path, expected).unwrap();
            assert!(retired
                .cover_rejoin_pending(&ui, inputs, &caller, &mut NeverRng)
                .is_err());
            assert_eq!(&std::fs::read(path).unwrap(), expected);
            std::fs::remove_file(path).unwrap();
            std::fs::rename(original, path).unwrap();
        }
        let warm = retired
            .cover_rejoin_pending(&ui, (&replacement, token, u64::MAX), &caller, &mut NeverRng)
            .unwrap()
            .unwrap();
        assert_eq!(warm.request_digest, saved.request_digest);
        assert_eq!(warm.context, saved.context);
        drop((old_ui, ui, candidate, retired));
        wait_for_original_vault().await;

        if scenario == MembershipAfterRemove::RejoinActive {
            // Restore only the genuine catalog from before ID9. The two
            // files for the exact covered Pending still bind ID9 ciphertext.
            std::fs::write(&catalog, &before).unwrap();
            let (device, window, work) = app(member_path.clone(), None, true);
            unlock_current(&device, &window, &work).await;
            let input = serde_json::json!({
                "inviteHex": hex::encode(&replacement_bytes),
                "approvedHost": "127.0.0.1",
            });
            let reply = invite_command(&window, "rejoin_server", input);
            run(&work).await;
            let tauri::ipc::InvokeResponse::Err(error) = reply.await.unwrap() else {
                panic!("a rolled-back catalog cannot select another rejoin");
            };
            assert_eq!(
                error.0,
                serde_json::json!("Rejoin did not finish. Try the same invitation again.")
            );
            assert!(!stage.exists());
            assert_eq!(std::fs::read(&catalog).unwrap(), before);
            assert_eq!(std::fs::read(&attempt).unwrap(), attempt_bytes);
            assert_eq!(std::fs::read(&pending).unwrap(), pending_bytes);
            let state = device.state::<AppState>();
            let close = register_vault_close(&state).await.unwrap();
            sources::close_server_rows(&state, &SystemClock)
                .await
                .unwrap();
            drop((close, state));
            drop((window, device));
            wait_for_original_vault().await;
            std::fs::write(&catalog, &bytes).unwrap();
        }

        let (mut reopened, ui_budget) = open();
        let mut candidate = reopened
            .prepare_rejoin_ui(2, ui_budget.prepare().unwrap())
            .finish()
            .unwrap();
        let ui = reopened.admit_rejoin_ui(&mut candidate).unwrap();
        let exact = timeout(Duration::from_secs(10), async {
            loop {
                if let Some(receipt) = reopened
                    .cover_rejoin_pending(
                        &ui,
                        (&replacement, token, u64::MAX),
                        &caller,
                        &mut NeverRng,
                    )
                    .unwrap()
                {
                    break receipt;
                }
                SystemClock.sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(exact.request_digest, saved.request_digest);
        assert_eq!(exact.context, saved.context);
        assert!(reopened
            .cover_rejoin_pending(&ui, (&replacement, [96; 32], now), &caller, &mut NeverRng,)
            .is_err());
        for path in [&attempt, &pending] {
            let original = path.with_extension("held-cold-rejoin-test");
            std::fs::rename(path, &original).unwrap();
            assert!(reopened
                .cover_rejoin_pending(&ui, inputs, &caller, &mut NeverRng)
                .is_err());
            assert!(!path.exists());
            std::fs::rename(original, path).unwrap();
        }
        let torn = &attempt_bytes[..attempt_bytes.len() / 2];
        std::fs::write(&attempt, torn).unwrap();
        assert!(reopened
            .cover_rejoin_pending(&ui, inputs, &caller, &mut NeverRng)
            .is_err());
        assert_eq!(std::fs::read(&attempt).unwrap(), torn);
        std::fs::write(&attempt, &attempt_bytes).unwrap();

        let partial = &pending_bytes[..pending_bytes.len() / 2];
        std::fs::write(&pending, partial).unwrap();
        assert!(reopened
            .cover_rejoin_pending(&ui, inputs, &caller, &mut NeverRng)
            .is_err());
        assert_eq!(std::fs::read(&pending).unwrap(), partial);
        assert_eq!(std::fs::read(&attempt).unwrap(), attempt_bytes);
        assert_eq!(std::fs::read(&catalog).unwrap(), bytes);
        assert_eq!(std::fs::read(&selector).unwrap(), selector_bytes);
        std::fs::write(&pending, &pending_bytes).unwrap();
        if matches!(
            scenario,
            MembershipAfterRemove::RejoinExchange
                | MembershipAfterRemove::RejoinReceived
                | MembershipAfterRemove::RejoinWelcome
                | MembershipAfterRemove::RejoinPrefix
                | MembershipAfterRemove::RejoinIndex
                | MembershipAfterRemove::RejoinActive
        ) {
            let admit = |owner: &catcoms_app::store::NativeTextRemovedJoinedVault, generation| {
                let mut prepared = owner
                    .prepare_rejoin_ui(generation, ui_budget.prepare().unwrap())
                    .finish()
                    .unwrap();
                owner.admit_rejoin_ui(&mut prepared).unwrap()
            };
            let held = pending.with_extension("exchange-foreign-for-test");
            std::fs::rename(&pending, &held).unwrap();
            std::fs::write(&pending, &pending_bytes).unwrap();
            assert!(reopened
                .prepare_rejoin_exchange(admit(&reopened, 2), &replacement, token, caller.clone())
                .is_err());
            std::fs::remove_file(&pending).unwrap();
            std::fs::rename(&held, &pending).unwrap();

            let scheduler = catcoms_discovery::EndpointDialScheduler::new(Default::default());
            let host = std::net::Ipv4Addr::LOCALHOST.into();
            let old = reopened
                .prepare_rejoin_exchange(admit(&reopened, 2), &replacement, token, caller.clone())
                .unwrap()
                .expect("the original A1 listener drained");
            lock.store(true, Ordering::SeqCst);
            generation.store(3, Ordering::SeqCst);
            authorized.store(3, Ordering::SeqCst);
            lock.store(false, Ordering::SeqCst);
            assert!(old
                .connect_approved_host(&replacement, host, &scheduler)
                .await
                .is_err());
            drop(old);

            let original_received =
                member_path.join("chat-critical-namespaces.rejoin-received.bin");
            let local_ui = admit(&reopened, 3);
            let invitation = (&replacement, token);
            assert!(reopened
                .recover_rejoin_received(&local_ui, invitation, &caller, &mut NeverRng)
                .unwrap()
                .is_none());
            assert!(!original_received.exists());
            let exchange = reopened
                .prepare_rejoin_exchange(admit(&reopened, 3), &replacement, token, caller.clone())
                .unwrap()
                .unwrap();
            exchange
                .connect_approved_host(&replacement, host, &scheduler)
                .await
                .unwrap();
            let delivery = timeout(Duration::from_secs(45), exchange.receive())
                .await
                .expect("the original founder answers A2 over its TCP listener")
                .unwrap();
            assert_eq!(delivery.request_digest_data(), saved.request_digest);
            assert!(delivery.is_for_original_cached(&reopened));
            assert_eq!(reopened.removal_data().unwrap(), Some(proof));
            assert_eq!(std::fs::read(&catalog).unwrap(), bytes);
            assert_eq!(std::fs::read(&pending).unwrap(), pending_bytes);

            if scenario != MembershipAfterRemove::RejoinExchange {
                use catcoms_app::store::NativeTextRejoinWelcomeStatus;
                let receipt = delivery
                    .cover(&mut reopened, invitation, &mut OsCryptoRng)
                    .unwrap();
                assert_eq!(receipt.request_digest, saved.request_digest);
                assert_ne!(receipt.delivery_digest, [0; 32]);
                let raw = std::fs::read(&original_received).unwrap();
                let renamed = original_received.with_extension("received-for-test");
                std::fs::rename(&original_received, &renamed).unwrap();
                std::fs::write(&original_received, &raw).unwrap();
                assert!(reopened
                    .recover_rejoin_received(&local_ui, invitation, &caller, &mut NeverRng)
                    .is_err());
                std::fs::remove_file(&original_received).unwrap();
                std::fs::rename(&renamed, &original_received).unwrap();
                assert_eq!(
                    reopened
                        .recover_rejoin_received(&local_ui, invitation, &caller, &mut NeverRng)
                        .unwrap(),
                    Some(receipt),
                );
                let welcome = if matches!(
                    scenario,
                    MembershipAfterRemove::RejoinWelcome
                        | MembershipAfterRemove::RejoinPrefix
                        | MembershipAfterRemove::RejoinIndex
                        | MembershipAfterRemove::RejoinActive
                ) {
                    let result = if scenario == MembershipAfterRemove::RejoinActive {
                        let (revoke, signal) = tokio::sync::watch::channel(false);
                        let original = catcoms_rt::RequestCancellation::new(signal, None);
                        let entry =
                            member_path.join("chat-critical-namespaces.rejoin-welcome-attempt.bin");
                        let outcome =
                            member_path.join("chat-critical-namespaces.rejoin-outcome.bin");
                        let before_sdk = caller
                            .clone()
                            .with_authority(Arc::new(FirstStageOnly::new(entry.clone())));
                        assert!(reopened
                            .cover_rejoin_welcome(
                                &local_ui,
                                invitation,
                                &before_sdk,
                                &mut OsCryptoRng
                            )
                            .is_err());
                        assert!(before_sdk.is_cancelled() && !entry.exists() && !outcome.exists());
                        let mut rng = RevokeAfterJournal {
                            path: &entry,
                            signal: &revoke,
                        };
                        // Outcome sealing is the first draw after the synced
                        // entry and successful Welcome SDK call.
                        assert!(reopened
                            .cover_rejoin_welcome(&local_ui, invitation, &original, &mut rng)
                            .is_err());
                        assert!(original.is_cancelled());
                        assert!(outcome.is_file());
                        reopened.cover_rejoin_welcome(&local_ui, invitation, &caller, &mut NeverRng)
                    } else {
                        reopened.cover_rejoin_welcome(
                            &local_ui,
                            invitation,
                            &caller,
                            &mut OsCryptoRng,
                        )
                    };
                    let NativeTextRejoinWelcomeStatus::Covered(result) = result.unwrap() else {
                        panic!("the exact covered delivery enters one Welcome");
                    };
                    assert_eq!(result.request_digest, receipt.request_digest);
                    assert_eq!(result.epoch, 3);
                    assert_eq!(
                        reopened
                            .cover_rejoin_welcome(&local_ui, invitation, &caller, &mut NeverRng)
                            .unwrap(),
                        NativeTextRejoinWelcomeStatus::Covered(result),
                    );
                    assert_eq!(reopened.removal_data().unwrap(), Some(proof));
                    Some(result)
                } else {
                    None
                };
                let prefix_path =
                    member_path.join("chat-critical-namespaces.rejoin-initial-prefix.bin");
                let covered_prefix = if matches!(
                    scenario,
                    MembershipAfterRemove::RejoinPrefix
                        | MembershipAfterRemove::RejoinIndex
                        | MembershipAfterRemove::RejoinActive,
                ) {
                    let result = reopened
                        .cover_rejoin_prefix(&local_ui, invitation, &caller, &mut OsCryptoRng)
                        .unwrap()
                        .unwrap();
                    let (head, active, epoch) = result.registration.selected_data();
                    assert_eq!(epoch, 3);
                    assert_eq!(head, welcome.unwrap().header);
                    assert_ne!(active, proof.1);
                    let original = std::fs::read(&prefix_path).unwrap();
                    let saved_file = prefix_path.with_extension("prefix-role-for-test");
                    std::fs::rename(&prefix_path, &saved_file).unwrap();
                    std::fs::write(&prefix_path, &original).unwrap();
                    assert!(reopened
                        .cover_rejoin_prefix(&local_ui, invitation, &caller, &mut NeverRng)
                        .is_err());
                    std::fs::remove_file(&prefix_path).unwrap();
                    std::fs::rename(saved_file, &prefix_path).unwrap();
                    let same = reopened
                        .cover_rejoin_prefix(&local_ui, invitation, &caller, &mut NeverRng)
                        .unwrap()
                        .unwrap();
                    assert_eq!(same.history_fingerprint, result.history_fingerprint);
                    assert_eq!(reopened.removal_data().unwrap(), Some(proof));
                    Some((result, original))
                } else {
                    None
                };
                let index_path =
                    member_path.join("chat-critical-namespaces.rejoin-native-index.bin");
                let covered_index = if matches!(
                    scenario,
                    MembershipAfterRemove::RejoinIndex | MembershipAfterRemove::RejoinActive
                ) {
                    let installed = reopened
                        .cover_rejoin_index(&local_ui, invitation, &caller, &mut OsCryptoRng)
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        installed.prefix_fingerprint,
                        covered_prefix.as_ref().unwrap().0.history_fingerprint,
                    );
                    let retired_id = joined["server"]
                        .as_u64()
                        .or_else(|| joined["server"].as_str()?.parse().ok())
                        .expect("the original join returns a numeric native row");
                    assert!(installed.row.id > retired_id);
                    let raw = std::fs::read(&index_path).unwrap();
                    let held = index_path.with_extension("held-index-for-test");
                    std::fs::rename(&index_path, &held).unwrap();
                    std::fs::write(&index_path, &raw).unwrap();
                    assert!(reopened
                        .cover_rejoin_index(&local_ui, invitation, &caller, &mut NeverRng)
                        .is_err());
                    std::fs::remove_file(&index_path).unwrap();
                    std::fs::rename(&held, &index_path).unwrap();
                    assert_eq!(
                        reopened
                            .cover_rejoin_index(&local_ui, invitation, &caller, &mut NeverRng)
                            .unwrap(),
                        Some(installed),
                    );
                    assert_eq!(reopened.removal_data().unwrap(), Some(proof));
                    Some((installed, raw))
                } else {
                    None
                };
                drop((local_ui, ui, candidate, reopened));
                wait_for_original_vault().await;

                let (mut cold, ui_budget) = open_retired_with_ui(atoms(), covered_index.is_some());
                timeout(Duration::from_secs(10), async {
                    while cold.removal_data().unwrap().is_none() {
                        SystemClock.sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                let mut capture = cold
                    .prepare_rejoin_ui(3, ui_budget.prepare().unwrap())
                    .finish()
                    .unwrap();
                let ui = cold.admit_rejoin_ui(&mut capture).unwrap();
                if let Some((expected, raw)) = covered_index {
                    let half = &raw[..raw.len() / 2];
                    std::fs::write(&index_path, half).unwrap();
                    assert!(cold
                        .recover_rejoin_index(&ui, &caller, &mut NeverRng)
                        .is_err());
                    assert_eq!(std::fs::read(&index_path).unwrap(), half);
                    std::fs::write(&index_path, &raw).unwrap();

                    let prefix = prefix_path.with_extension("held-prefix-for-test");
                    std::fs::rename(&prefix_path, &prefix).unwrap();
                    assert!(cold
                        .recover_rejoin_index(&ui, &caller, &mut NeverRng)
                        .is_err());
                    assert!(!prefix_path.exists());
                    assert_eq!(std::fs::read(&index_path).unwrap(), raw);
                    std::fs::rename(&prefix, &prefix_path).unwrap();

                    assert_eq!(
                        cold.recover_rejoin_index(&ui, &caller, &mut NeverRng)
                            .unwrap(),
                        Some(expected),
                    );
                    assert_eq!(
                        cold.recover_rejoin_index(&ui, &caller, &mut NeverRng)
                            .unwrap(),
                        Some(expected),
                    );
                    assert_eq!(cold.removal_data().unwrap(), Some(proof));
                    assert_eq!(std::fs::read(&index_path).unwrap(), raw);
                    drop((ui, capture, cold));
                    wait_for_original_vault().await;
                    if scenario == MembershipAfterRemove::RejoinActive {
                        // Put back the real pre-ID9 catalog and park the three
                        // original Remove files. A2's surviving Index must
                        // prevent this partial rollback from restoring A1.
                        let name = selector.file_name().unwrap().to_str().unwrap();
                        let roles = [
                            selector.clone(),
                            selector.with_file_name(
                                name.replace("joined-member-selected", "joined-member-add"),
                            ),
                            selector.with_file_name(
                                name.replace("joined-member-selected", "joined-member-header")
                                    .replace(".2.bin", ".2.first.bin"),
                            ),
                        ];
                        let parked = roles
                            .each_ref()
                            .map(|role| role.with_extension("rollback-for-test"));
                        for (role, saved) in roles.iter().zip(&parked) {
                            std::fs::rename(role, saved).unwrap();
                        }
                        std::fs::write(&catalog, &before).unwrap();
                        assert_eq!(
                            removed_response(member_path.clone()).await,
                            serde_json::json!("Could not unlock the existing Chat V2 vault."),
                        );
                        wait_for_original_vault().await;
                        assert_eq!(std::fs::read(&index_path).unwrap(), raw);
                        std::fs::write(&catalog, &bytes).unwrap();
                        for (role, saved) in roles.iter().zip(&parked) {
                            std::fs::rename(saved, role).unwrap();
                        }
                        check_active_rejoin(
                            member_path.clone(),
                            old_port.get(),
                            (&main, &founder_queue, &found),
                            (&joined, Some(expected.row.id), &replacement_bytes),
                        )
                        .await;
                    }
                    return;
                }
                let replay = cold
                    .cover_rejoin_pending(
                        &ui,
                        (&replacement, token, u64::MAX),
                        &caller,
                        &mut NeverRng,
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(replay.request_digest, saved.request_digest);
                if welcome.is_some() {
                    let backup = original_received.with_extension("missing-received-for-test");
                    let sdk =
                        member_path.join("chat-critical-namespaces.rejoin-welcome-attempt.bin");
                    let outcome = member_path.join("chat-critical-namespaces.rejoin-outcome.bin");
                    let before = (
                        std::fs::read(&sdk).unwrap(),
                        std::fs::read(&outcome).unwrap(),
                    );
                    std::fs::rename(&original_received, &backup).unwrap();
                    assert!(cold
                        .recover_rejoin_received(&ui, invitation, &caller, &mut NeverRng)
                        .is_err());
                    assert!(!original_received.exists());
                    assert_eq!(std::fs::read(sdk).unwrap(), before.0);
                    assert_eq!(std::fs::read(outcome).unwrap(), before.1);
                    std::fs::rename(&backup, &original_received).unwrap();
                }
                let partial = &raw[..raw.len() / 2];
                std::fs::write(&original_received, partial).unwrap();
                assert!(cold
                    .recover_rejoin_received(&ui, invitation, &caller, &mut NeverRng)
                    .is_err());
                assert_eq!(std::fs::read(&original_received).unwrap(), partial);
                std::fs::write(&original_received, &raw).unwrap();
                assert_eq!(
                    cold.recover_rejoin_received(&ui, invitation, &caller, &mut NeverRng)
                        .unwrap(),
                    Some(receipt),
                );
                assert_eq!(std::fs::read(&original_received).unwrap(), raw);
                assert_eq!(cold.removal_data().unwrap(), Some(proof));
                if let Some(result) = welcome {
                    assert_eq!(
                        cold.recover_rejoin_welcome(&ui, invitation, &caller, &mut NeverRng)
                            .unwrap(),
                        NativeTextRejoinWelcomeStatus::Covered(result),
                    );
                    assert_eq!(cold.removal_data().unwrap(), Some(proof));
                }
                if let Some((covered, original)) = covered_prefix {
                    let partial = &original[..original.len() / 2];
                    std::fs::write(&prefix_path, partial).unwrap();
                    assert!(cold
                        .cover_rejoin_prefix(&ui, invitation, &caller, &mut NeverRng)
                        .is_err());
                    assert_eq!(std::fs::read(&prefix_path).unwrap(), partial);
                    std::fs::write(&prefix_path, &original).unwrap();
                    let recovered = cold
                        .cover_rejoin_prefix(&ui, invitation, &caller, &mut NeverRng)
                        .unwrap()
                        .unwrap();
                    assert_eq!(recovered.registration, covered.registration);
                    assert_eq!(recovered.history_fingerprint, covered.history_fingerprint);
                    assert_eq!(std::fs::read(&prefix_path).unwrap(), original);
                }
                drop((ui, capture, cold));
                wait_for_original_vault().await;

                if scenario == MembershipAfterRemove::RejoinWelcome {
                    let sdk_attempt =
                        member_path.join("chat-critical-namespaces.rejoin-welcome-attempt.bin");
                    let sdk_outcome =
                        member_path.join("chat-critical-namespaces.rejoin-outcome.bin");
                    let attempt_frame = std::fs::read(&sdk_attempt).unwrap();
                    let held_outcome = sdk_outcome.with_extension("held-outcome-for-test");
                    std::fs::rename(&sdk_outcome, &held_outcome).unwrap();
                    let (mut held, paid) = open();
                    timeout(Duration::from_secs(10), async {
                        while held.removal_data().unwrap().is_none() {
                            SystemClock.sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .unwrap();
                    let mut capture = held
                        .prepare_rejoin_ui(3, paid.prepare().unwrap())
                        .finish()
                        .unwrap();
                    let ui = held.admit_rejoin_ui(&mut capture).unwrap();
                    held.cover_rejoin_pending(&ui, inputs, &caller, &mut NeverRng)
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        held.recover_rejoin_received(&ui, invitation, &caller, &mut NeverRng)
                            .unwrap(),
                        Some(receipt),
                    );
                    assert_eq!(
                        held.recover_rejoin_welcome(&ui, invitation, &caller, &mut NeverRng)
                            .unwrap(),
                        NativeTextRejoinWelcomeStatus::Held,
                    );
                    assert_eq!(std::fs::read(&sdk_attempt).unwrap(), attempt_frame);
                    assert!(!sdk_outcome.exists());
                    std::fs::rename(&held_outcome, &sdk_outcome).unwrap();
                    drop((ui, capture, held));
                    wait_for_original_vault().await;
                }

                let held_attempt = attempt.with_extension("rejoin-attempt-held-for-test");
                let held_pending = pending.with_extension("rejoin-pending-held-for-test");
                std::fs::rename(&attempt, &held_attempt).unwrap();
                std::fs::rename(&pending, &held_pending).unwrap();
                let (mut orphaned, ui_budget) = open();
                timeout(Duration::from_secs(10), async {
                    while orphaned.removal_data().unwrap().is_none() {
                        SystemClock.sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                let mut capture = orphaned
                    .prepare_rejoin_ui(3, ui_budget.prepare().unwrap())
                    .finish()
                    .unwrap();
                let ui = orphaned.admit_rejoin_ui(&mut capture).unwrap();
                assert!(orphaned
                    .cover_rejoin_pending(&ui, inputs, &caller, &mut NeverRng)
                    .is_err());
                assert!(!attempt.exists() && !pending.exists());
                assert_eq!(std::fs::read(&original_received).unwrap(), raw);
                std::fs::rename(&held_attempt, &attempt).unwrap();
                std::fs::rename(&held_pending, &pending).unwrap();
                assert_eq!(
                    orphaned
                        .cover_rejoin_pending(&ui, inputs, &caller, &mut NeverRng)
                        .unwrap()
                        .unwrap()
                        .request_digest,
                    saved.request_digest,
                );
                drop((ui, capture, orphaned));
            } else {
                lock.store(true, Ordering::SeqCst);
                assert!(!delivery.is_for_original_cached(&reopened));
                drop((delivery, local_ui, ui, candidate, reopened));
            }
            wait_for_original_vault().await;
            let founder_state = founder.state::<AppState>();
            let close = register_vault_close(&founder_state).await.unwrap();
            sources::close_server_rows(&founder_state, &SystemClock)
                .await
                .unwrap();
            drop((close, founder_state));
            return;
        }
        drop((ui, candidate, reopened));
        wait_for_original_vault().await;
    }

    // The selected Remove is already on disk. A different application
    // instance must use the original founder identity to mint its next invite.
    let founder_state = founder.state::<AppState>();
    let close = register_vault_close(&founder_state).await.unwrap();
    sources::close_server_rows(&founder_state, &SystemClock)
        .await
        .unwrap();
    drop((close, founder_state));
    drop((main, founder));

    let mint_args =
        serde_json::json!({"server": found["server"], "expectedContext": found["context"]});
    let (reopened, main, queue) = app(founder_path.clone(), None, true);
    unlock_current(&reopened, &main, &queue).await;
    let review_marker = founder_path.join("chat-critical-namespaces.review-hold.marker");
    let review_cover =
        std::fs::read(&review_marker).expect("cold founder provisions review storage");
    assert_eq!(review_cover.len(), 32);
    let decision_marker = founder_path.join("chat-critical-namespaces.review-decision.marker");
    let decision_head = founder_path
        .join("chat-v2-review-decisions")
        .join("head.current");
    let review_catalog = founder_path.join("chat-critical-namespaces.bin");
    let decision_cover = std::fs::read(&decision_marker).expect("startup covers both declarations");
    let decision_empty = std::fs::read(&decision_head).expect("startup covers the empty head");
    assert_eq!(decision_cover.len(), 32);
    assert_eq!(decision_empty.len(), 32);
    let selected_catalog = std::fs::read(&review_catalog).unwrap();
    let mut held = serde_json::json!({
        "server": found["server"], "channel": found["channel"], "expectedContext": found["context"],
        "heldAfter": null, "limit": 2,
    });
    let empty = command_value_after_busy(
        &main,
        &queue,
        "get_message_page",
        history::BUSY,
        held.clone(),
    )
    .await;
    assert_eq!(
        empty,
        serde_json::json!({
            "backend": "text-v2", "scope": "selected-held-posts", "items": [],
            "next": null, "unavailable": 0,
        })
    );
    held["heldAfter"] = serde_json::json!("93".repeat(32));
    refused(command_after_busy(&main, &queue, "get_message_page", history::BUSY, held).await);
    if let Some(history) = &retained_history {
        check_retained_removal_history(&main, &queue, &found, history).await;
        check_reply_to_removed_author(&main, &queue, &found, &history.visible, 92).await;
    }
    async fn mint(
        window: &tauri::WebviewWindow<MockRuntime>,
        queue: &Queue,
        args: &serde_json::Value,
    ) -> serde_json::Value {
        command_value_after_busy(
            window,
            queue,
            "mint_invite_fresh",
            invite_mint::BUSY,
            args.clone(),
        )
        .await
    }
    let cold = mint(&main, &queue, &mint_args).await;
    let warm_bytes = hex::decode(invite.as_str().unwrap()).unwrap();
    let cold_bytes = hex::decode(cold.as_str().unwrap()).unwrap();
    let warm = NativeTextInvitePacketData::decode(&warm_bytes).unwrap();
    let next = NativeTextInvitePacketData::decode(&cold_bytes).unwrap();
    let warm_binding = warm.binding().spec_data();
    let next_binding = next.binding().spec_data();
    assert!(next_binding.anchor == warm_binding.anchor);
    assert_eq!(next_binding.inviter, warm_binding.inviter);
    assert_ne!(next_binding.nonce, warm_binding.nonce);
    assert_eq!(next.policy_bytes(), warm.policy_bytes());
    assert_eq!(next.reservation_bytes(), warm.reservation_bytes());
    assert_eq!(
        mint(&main, &queue, &mint_args).await,
        cold,
        "same selected parent and token"
    );
    let state = reopened.state::<AppState>();
    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop((close, state));
    drop((main, reopened));

    let (reopened, main, queue) = app(founder_path.clone(), None, true);
    unlock_current(&reopened, &main, &queue).await;
    assert_eq!(
        mint(&main, &queue, &mint_args).await,
        cold,
        "cold replay returns the exact original packet"
    );
    assert_eq!(std::fs::read(review_marker).unwrap(), review_cover);
    assert_eq!(std::fs::read(decision_marker).unwrap(), decision_cover);
    assert_eq!(std::fs::read(decision_head).unwrap(), decision_empty);
    assert_eq!(std::fs::read(review_catalog).unwrap(), selected_catalog);
    if join_after_restart {
        fn group_directory(dir: &std::path::Path, depth: usize) -> Option<std::path::PathBuf> {
            assert!(depth < 12, "isolated fixture tree");
            if dir.join("text.add.response.v2").is_file() && dir.join("head.bin").is_file() {
                return Some(dir.to_owned());
            }
            std::fs::read_dir(dir).unwrap().find_map(|entry| {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    group_directory(&entry.path(), depth + 1)
                } else {
                    None
                }
            })
        }
        let before = if repair_staged_add {
            let directory = group_directory(&founder_path, 0).expect("governed founder directory");
            let head = directory.join("head.bin");
            Some((directory, std::fs::read(&head).unwrap()))
        } else {
            None
        };
        let (successor, window, successor_queue) = app(root.path().join("successor"), None, true);
        unlock_current(&successor, &window, &successor_queue).await;
        let request = invite_command(
            &window,
            "join_server",
            serde_json::json!({
                "inviteHex": cold, "approvedHost": "127.0.0.1", "displayName": "Successor",
                "serverName": "Recovered group", "isDm": false, "allowSwitchboards": false,
            }),
        );
        timeout(Duration::from_secs(45), run(&successor_queue))
            .await
            .expect("the fresh Add runs over the recovered founder TCP listener");
        let joined = found_value(request.await.unwrap());
        assert!(joined["server"].is_number());
        let new_roster = command_value_after_busy(
            &main,
            &queue,
            "get_native_chat_members",
            membership::BUSY,
            mint_args.clone(),
        )
        .await;
        let live = new_roster["members"].as_array().unwrap();
        assert_eq!(live.len(), 2, "founder and the new Admission only");
        assert!(live
            .iter()
            .all(|entry| entry["admission"] != admission["admission"]));
        let new_admission = live
            .iter()
            .find(|member| member["founder"] == false)
            .unwrap();

        let state = reopened.state::<AppState>();
        let close = register_vault_close(&state).await.unwrap();
        sources::close_server_rows(&state, &SystemClock)
            .await
            .unwrap();
        drop((close, state));
        drop((main, reopened));
        let repaired = if let Some((directory, parent_bytes)) = before {
            let head = directory.join("head.bin");
            let response = std::fs::read_dir(&directory)
                .unwrap()
                .find_map(|entry| {
                    let entry = entry.unwrap();
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("text.add.response.v2.next."))
                        .then(|| entry.path())
                })
                .expect("the covered later Add response");
            let stage_name = response.file_name().unwrap().to_str().unwrap().replacen(
                "text.add.response.v2",
                "text.add.head-stage.v2",
                1,
            );
            let stage = directory.join(stage_name);
            let new_head = std::fs::read(&head).unwrap();
            assert_ne!(new_head, parent_bytes);
            assert!(!stage.exists());
            // Recreate the physical cut before the durable Head rename.
            std::fs::rename(&head, &stage).unwrap();
            std::fs::write(&head, &parent_bytes).unwrap();

            let parked = response.with_extension("held-response");
            std::fs::rename(&response, &parked).unwrap();
            {
                let (blocked, main, queue) = app(founder_path.clone(), None, true);
                let state = blocked.state::<AppState>();
                let generation = state
                    .ui_session_generation
                    .load(Ordering::SeqCst)
                    .to_string();
                let reply = invoke(&main, &generation, SECRET);
                run(&queue).await;
                refused(reply.await.unwrap());
                assert!(state.store.lock().await.is_none());
            }
            assert_eq!(std::fs::read(&head).unwrap(), parent_bytes);
            assert_eq!(std::fs::read(&stage).unwrap(), new_head);
            assert!(!response.exists());
            std::fs::rename(parked, response).unwrap();

            timeout(Duration::from_secs(5), async {
                loop {
                    match catcoms_storage::vault::acquire_vault_session(&founder_path) {
                        Ok(vault) => return drop(vault),
                        Err(catcoms_storage::StorageError::VaultBusy) => {
                            SystemClock.sleep(Duration::from_millis(20)).await;
                        }
                        Err(other) => panic!("the original vault session refused: {other}"),
                    }
                }
            })
            .await
            .expect("the failed restore released its original actor lease");
            Some((stage, head, new_head))
        } else {
            None
        };
        let (reopened, main, queue) = app(founder_path.clone(), None, true);
        unlock_current(&reopened, &main, &queue).await;
        if let Some((stage, head, exact)) = repaired {
            assert!(!stage.exists());
            assert_eq!(std::fs::read(head).unwrap(), exact);
        }
        let cold_roster = command_value_after_busy(
            &main,
            &queue,
            "get_native_chat_members",
            membership::BUSY,
            mint_args.clone(),
        )
        .await;
        assert!(cold_roster["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["admission"] == new_admission["admission"]));
        let state = reopened.state::<AppState>();
        let close = register_vault_close(&state).await.unwrap();
        sources::close_server_rows(&state, &SystemClock)
            .await
            .unwrap();
        drop((close, state));
        drop((main, reopened));
        let state = successor.state::<AppState>();
        let close = register_vault_close(&state).await.unwrap();
        sources::close_server_rows(&state, &SystemClock)
            .await
            .unwrap();
        drop((close, state));
        drop((window, successor));
        return;
    }
    let state = reopened.state::<AppState>();
    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop((close, state));
    drop((main, reopened));

    fn find_selected_invite_raw(dir: &std::path::Path, depth: u8) -> Option<std::path::PathBuf> {
        assert!(depth < 12, "bounded isolated fixture tree");
        std::fs::read_dir(dir).unwrap().find_map(|entry| {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                find_selected_invite_raw(&entry.path(), depth + 1)
            } else if entry
                .file_name()
                .to_string_lossy()
                .starts_with("invite.raw.v2.")
            {
                Some(entry.path())
            } else {
                None
            }
        })
    }
    let raw = find_selected_invite_raw(&founder_path, 0).unwrap();
    let name = raw.file_name().unwrap().to_str().unwrap();
    let attempt = raw.with_file_name(name.replacen(".raw.", ".attempt.", 1));
    let intent = raw.with_file_name(name.replacen(".raw.", ".intent.", 1));
    let snapshot = [
        std::fs::read(&intent).unwrap(),
        std::fs::read(&attempt).unwrap(),
    ];
    let original = std::fs::read(&raw).unwrap();
    let torn = &original[..original.len() - 1];
    std::fs::write(&raw, torn).unwrap();

    let (reopened, main, queue) = app(founder_path, None, true);
    unlock_current(&reopened, &main, &queue).await;
    let invalid = command_after_busy(
        &main,
        &queue,
        "mint_invite_fresh",
        invite_mint::BUSY,
        mint_args,
    )
    .await;
    let tauri::ipc::InvokeResponse::Err(error) = invalid else {
        panic!("a new process published through a torn raw signature");
    };
    assert_eq!(error.0, serde_json::json!(invite_mint::FAILED));
    assert_eq!(std::fs::read(&raw).unwrap(), torn);
    assert_eq!(
        [
            std::fs::read(intent).unwrap(),
            std::fs::read(attempt).unwrap()
        ],
        snapshot
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_joined_dependency_provider_reopens_after_selected_add() {
    use catcoms_replication::chat::ActionId;
    use tauri::Listener;

    let root = tempfile::tempdir().unwrap();
    let (founder, main, founder_queue) = app(root.path().join("founder"), None, true);
    let (member, member_window, member_queue) = app(root.path().join("member"), None, true);
    let (newcomer, new_window, new_queue) = app(root.path().join("newcomer"), None, true);
    unlock_current(&founder, &main, &founder_queue).await;
    unlock_current(&member, &member_window, &member_queue).await;
    unlock_current(&newcomer, &new_window, &new_queue).await;
    let found = found_request(&main, 1, "Original room");
    run(&founder_queue).await;
    let found = found_value(found.await.unwrap());
    let mint = || {
        invite_command(
            &main,
            "mint_invite_fresh",
            serde_json::json!({"server": found["server"], "expectedContext": found["context"]}),
        )
    };
    let join = |window: &tauri::WebviewWindow<MockRuntime>, invite: &serde_json::Value, name| {
        invite_command(
            window,
            "join_server",
            serde_json::json!({
                "inviteHex": invite, "approvedHost": "127.0.0.1", "displayName": name,
                "serverName": "Original room", "isDm": false, "allowSwitchboards": false,
            }),
        )
    };
    let invite = mint();
    run(&founder_queue).await;
    let invite = found_value(invite.await.unwrap());
    let joined = join(&member_window, &invite, "Existing member");
    timeout(Duration::from_secs(45), run(&member_queue))
        .await
        .unwrap();
    let joined = found_value(joined.await.unwrap());
    let member_id = joined["server"].as_u64().unwrap();
    let member_state = member.state::<AppState>();
    let workers = assert_joined_connected_workers(&member_state, member_id).await;
    let member_generation = member_state.ui_session_generation.load(Ordering::SeqCst);
    let action_id = |text| ActionId::from_bytes(membership::decode_exact_hex(text).unwrap());
    let holder_selected = async |action, expected_target, phase: &str| {
        let work = joined_work(&member_state, member_id).await;
        let caller = keep_session::current(&member_state, member_generation).unwrap();
        timeout(Duration::from_secs(15), async {
            loop {
                let result = {
                    let _permit = work.foreground(caller.clone()).await.unwrap();
                    let (prepared, selection, channel, scope) =
                        capture_joined_worker(&member_state, member_id, member_generation).await;
                    prepared.selected_archive_target_data(&selection, channel, action, &scope)
                };
                if result == Ok(Some(expected_target)) {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the holder did not select the founder {phase} before the Add"));
    };
    // The eventual recipient is still outside the group. Only the existing
    // joined member can select these publisher originals before the Add.
    let offline = command_value_after_busy(
        &main,
        &founder_queue,
        "send_message",
        sending::BUSY,
        serde_json::json!({"server": found["server"], "channel": found["channel"],
            "expectedContext": found["context"], "retryToken": hex::encode([81; 32]),
            "text": "From the later offline founder", "replyTo": null}),
    )
    .await;
    assert_eq!(offline["disposition"], "saved-local");
    let offline_message = offline["message_id"].as_str().unwrap().to_owned();
    let offline_id = action_id(&offline_message);
    holder_selected(offline_id, offline_id, "Create").await;
    let offline_edited = command_value_after_busy(
        &main,
        &founder_queue,
        "edit_message",
        sending::BUSY,
        serde_json::json!({"server": found["server"], "channel": found["channel"],
            "expectedContext": found["context"], "retryToken": hex::encode([82; 32]),
            "messageId": offline_message, "expectedRevision": offline_message,
            "text": "Changed by the later offline founder"}),
    )
    .await;
    assert_eq!(offline_edited["disposition"], "saved-local");
    let offline_edit = offline_edited["message_id"].as_str().unwrap();
    let offline_edit_id = action_id(offline_edit);
    holder_selected(offline_edit_id, offline_id, "Edit").await;
    let offline_deleted = command_value_after_busy(
        &main,
        &founder_queue,
        "delete_message",
        sending::BUSY,
        serde_json::json!({"server": found["server"], "channel": found["channel"],
            "expectedContext": found["context"], "retryToken": hex::encode([83; 32]),
            "messageId": offline_message, "expectedRevision": offline_edit}),
    )
    .await;
    assert_eq!(offline_deleted["disposition"], "saved-local");
    let offline_delete = action_id(offline_deleted["message_id"].as_str().unwrap());
    holder_selected(offline_delete, offline_id, "Delete").await;
    let reopen = AdmissionBarrier::new();
    *member_state
        .native_text
        .join_dependency_reopen_edge
        .lock()
        .unwrap() = Some(reopen.clone());

    let (notice, selected_add) = oneshot::channel();
    member.once_any("chat-v2-membership", move |event| {
        let _ = notice.send(serde_json::from_str::<serde_json::Value>(event.payload()).unwrap());
    });
    let next_invite = mint();
    run(&founder_queue).await;
    let next_invite = found_value(next_invite.await.unwrap());
    assert_ne!(next_invite, invite);
    let new_state = newcomer.state::<AppState>();
    let startup = AdmissionBarrier::new();
    *new_state.native_text.join_startup_edge.lock().unwrap() = Some(startup.clone());
    let new_row = join(&new_window, &next_invite, "New member");
    let new_task = tokio::spawn(run(&new_queue));
    let new_row = found_value(
        timeout(Duration::from_secs(45), new_row)
            .await
            .unwrap()
            .unwrap(),
    );
    timeout(Duration::from_secs(5), startup.arrived.notified())
        .await
        .unwrap();
    let notice = timeout(Duration::from_secs(45), selected_add)
        .await
        .expect("the running desktop applies the successor Add automatically")
        .unwrap();
    assert_eq!(
        notice,
        serde_json::json!({
            "server": joined["server"], "context": joined["context"], "removed": false,
        })
    );
    timeout(Duration::from_secs(5), reopen.arrived.notified())
        .await
        .expect("the old dependency provider retired after the selected Add");
    let (original, _, _, _) =
        capture_joined_worker(&member_state, member_id, member_generation).await;
    let (transport, release_transport) = BlockingIoBarrier::new();
    let held = transport.clone();
    let holder = tokio::task::spawn_blocking(move || {
        original.hold_dependency_transport_for_test(|| held.wait_once())
    });
    timeout(Duration::from_secs(5), transport.arrived.notified())
        .await
        .expect("the original TCP transport capture mutex is held");
    reopen.release.notify_one();
    timeout(
        Duration::from_secs(5),
        member_state
            .native_text
            .join_dependency_transport_busy
            .notified(),
    )
    .await
    .expect("provider replacement reached the contended TCP capture");
    drop(release_transport);
    assert_eq!(holder.await.unwrap(), Some(()));

    let contact = command_value_after_busy(
        &member_window,
        &member_queue,
        "share_native_chat_contact",
        membership::BUSY,
        serde_json::json!({"server": joined["server"], "expectedContext": joined["context"]}),
    )
    .await;
    let raw = hex::decode(contact.as_str().unwrap()).unwrap();
    let signed = catcoms_sync::V2MemberContactData::decode(&raw).unwrap();
    let peer = PeerId::new(*blake3::hash(signed.address_data().0).as_bytes());
    let connect = command_value_after_busy(
        &new_window,
        &new_queue,
        "connect_native_chat_contact",
        membership::BUSY,
        serde_json::json!({"server": new_row["server"], "expectedContext": new_row["context"],
            "contact": contact, "approvedHost": "127.0.0.1", "remember": true}),
    )
    .await;
    assert!(connect.is_null());

    let founder_state = founder.state::<AppState>();
    let close = register_vault_close(&founder_state).await.unwrap();
    sources::close_server_rows(&founder_state, &SystemClock)
        .await
        .unwrap();
    drop(close);
    let saved = command_value_after_busy(
        &member_window,
        &member_queue,
        "send_message",
        sending::BUSY,
        serde_json::json!({"server": joined["server"], "channel": joined["channel"],
            "expectedContext": joined["context"], "retryToken": hex::encode([91; 32]),
            "text": "Parent after the Add", "replyTo": null}),
    )
    .await;
    assert_eq!(saved["disposition"], "saved-local");
    let parent = saved["message_id"].as_str().unwrap();
    let reply = command_value_after_busy(
        &member_window,
        &member_queue,
        "send_message",
        sending::BUSY,
        serde_json::json!({"server": joined["server"], "channel": joined["channel"],
            "expectedContext": joined["context"], "retryToken": hex::encode([92; 32]),
            "text": "Reply after the Add", "replyTo": parent}),
    )
    .await;
    assert_eq!(reply["disposition"], "saved-local");
    let missing = invite_command(
        &new_window,
        "get_message_page",
        serde_json::json!({"server": new_row["server"], "channel": new_row["channel"],
            "expectedContext": new_row["context"], "archivedAction": parent}),
    );
    run(&new_queue).await;
    assert!(found_value(missing.await.unwrap())["item"].is_null());

    // Keep C's gossip receiver parked, so only its real dependency exchange
    // can recover the parent's signed original from B on the direct TCP link.
    let action = ActionId::from_bytes(membership::decode_exact_hex(parent).unwrap());
    let id = new_row["server"].as_u64().unwrap();
    let generation = new_state.ui_session_generation.load(Ordering::SeqCst);
    let recovered = timeout(Duration::from_secs(30), async {
        loop {
            let (prepared, selection, channel, scope) =
                capture_joined_worker(&new_state, id, generation).await;
            if let Ok(exchange) = prepared.prepare_dependency_exchange(
                &selection,
                channel,
                peer,
                action,
                scope,
                &mut OsCryptoRng,
            ) {
                if let Ok(Ok(response)) = timeout(Duration::from_secs(3), exchange.receive()).await
                {
                    let (prepared, current, _, scope) =
                        capture_joined_worker(&new_state, id, generation).await;
                    if let Ok(Some(original)) =
                        prepared.open_dependency_response(&current, response, &scope)
                    {
                        break original.action_id_data();
                    }
                }
            }
            SystemClock.sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the joined worker serves the dependency after the TCP capture mutex is released");
    assert_eq!(recovered, action);
    assert_eq!(
        member_state.ui_session_generation.load(Ordering::SeqCst),
        member_generation
    );
    assert!(Arc::ptr_eq(
        &workers.upgrade().unwrap(),
        &joined_work(&member_state, member_id).await
    ));
    let reverse_contact = command_value_after_busy(
        &new_window,
        &new_queue,
        "share_native_chat_contact",
        membership::BUSY,
        serde_json::json!({"server": new_row["server"], "expectedContext": new_row["context"]}),
    )
    .await;
    command_value_after_busy(
        &member_window,
        &member_queue,
        "connect_native_chat_contact",
        membership::BUSY,
        serde_json::json!({"server": joined["server"], "expectedContext": joined["context"],
            "contact": reverse_contact, "approvedHost": "127.0.0.1", "remember": true}),
    )
    .await;
    let deleted = command_value_after_busy(
        &member_window,
        &member_queue,
        "delete_message",
        sending::BUSY,
        serde_json::json!({"server": joined["server"], "channel": joined["channel"],
            "expectedContext": joined["context"], "retryToken": hex::encode([93; 32]),
            "messageId": parent, "expectedRevision": parent}),
    )
    .await;
    assert_eq!(deleted["disposition"], "saved-local");
    let delete = ActionId::from_bytes(
        membership::decode_exact_hex(deleted["message_id"].as_str().unwrap()).unwrap(),
    );
    let reply_id = reply["message_id"].as_str().unwrap();
    let mut deepest_reply = reply_id.to_owned();
    for depth in 2u8..=5 {
        let saved = command_value_after_busy(
            &member_window,
            &member_queue,
            "send_message",
            sending::BUSY,
            serde_json::json!({"server": joined["server"], "channel": joined["channel"],
                "expectedContext": joined["context"], "retryToken": hex::encode([100 + depth; 32]),
                "text": format!("Nested reply {depth}"), "replyTo": deepest_reply}),
        )
        .await;
        assert_eq!(saved["disposition"], "saved-local");
        deepest_reply = saved["message_id"].as_str().unwrap().to_owned();
    }
    // Keep the provider occupied until it has taken ownership of a real
    // request on the known link. The same request must get its page after
    // the local permit retires, without depending on a second network send.
    let member_work = joined_work(&member_state, member_id).await;
    let member_scope = keep_session::current(&member_state, member_generation).unwrap();
    let permit = member_work.foreground(member_scope).await.unwrap();
    let (probe, selection, channel, scope) =
        capture_joined_worker(&new_state, id, generation).await;
    let probe = probe
        .prepare_history_exchange(&selection, channel, peer, scope, &mut OsCryptoRng)
        .unwrap();
    let (entered, waiting) = oneshot::channel();
    *member_state
        .native_text
        .join_dependency_pending
        .lock()
        .unwrap() = Some(entered);
    let response = tokio::spawn(probe.receive());
    timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the original history request reached the occupied provider")
        .unwrap();
    assert!(!response.is_finished());
    drop(permit);
    let response = timeout(Duration::from_secs(5), response)
        .await
        .expect("the original provider request resumed after local contention")
        .unwrap()
        .unwrap();
    let (prepared, selection, _, scope) = capture_joined_worker(&new_state, id, generation).await;
    let page = prepared
        .open_history_response(&selection, response, &scope)
        .unwrap();
    assert_eq!(page.data().unwrap().entries().len(), 7);
    drop((page, member_work));

    let (archive_notice, mut notices) = tokio::sync::mpsc::channel(8);
    let expected_context = new_row["context"].clone();
    let expected_target = parent.to_owned();
    let expected_deepest = deepest_reply.clone();
    let expected_offline = offline_message.clone();
    let notice_listener = newcomer.listen_any("chat-v2-received", move |event| {
        let change: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
        if change["context"] == expected_context {
            let affected = &change["action"];
            let index = if *affected == expected_target {
                0
            } else if *affected == expected_deepest {
                1
            } else if *affected == expected_offline {
                2
            } else {
                return;
            };
            let _ = archive_notice.try_send(index);
        }
    });
    // Let the ordinary joined workers start, but keep incoming gossip parked.
    // Only the automatic history worker can cover these targets in the archive.
    let parked_live = AdmissionBarrier::new();
    let paid_reply = Arc::new(HistoryReplyProbe {
        action,
        edge: AdmissionBarrier::new(),
        attempts: std::sync::atomic::AtomicUsize::new(0),
    });
    *new_state
        .native_text
        .join_history_receive_edge
        .lock()
        .unwrap() = Some(parked_live.clone());
    *new_state
        .native_text
        .join_history_reply_probe
        .lock()
        .unwrap() = Some(paid_reply.clone());
    startup.release.notify_one();
    timeout(Duration::from_secs(15), new_task)
        .await
        .unwrap()
        .unwrap();
    timeout(Duration::from_secs(10), parked_live.arrived.notified())
        .await
        .unwrap();
    let newcomer_work = joined_work(&new_state, id).await;
    let newcomer_scope = keep_session::current(&new_state, generation).unwrap();
    let recovery_deadline = SystemClock.monotonic_ms().saturating_add(45_000);
    timeout(Duration::from_secs(35), paid_reply.edge.arrived.notified())
        .await
        .expect("an exact reply reached the receiver before local staging");
    let original_attempts = paid_reply.attempts.load(Ordering::SeqCst);
    assert!(original_attempts > 0);
    let held = newcomer_work
        .foreground(newcomer_scope.clone())
        .await
        .unwrap();
    paid_reply.edge.release.notify_one();
    timeout(
        Duration::from_secs(5),
        new_state.native_text.join_history_staging_wait.notified(),
    )
    .await
    .expect("the receiver retained the paid reply across a real local admission timeout");
    drop(held);
    let reply = ActionId::from_bytes(membership::decode_exact_hex(&deepest_reply).unwrap());
    let expected = [
        (delete, action),
        (reply, reply),
        (offline_edit_id, offline_id),
        (offline_delete, offline_id),
    ];
    let mut progress = [false; 4];
    let recovery = timeout(
        Duration::from_millis(recovery_deadline.saturating_sub(SystemClock.monotonic_ms())),
        async {
            // Inspect only the affected target after an invalidation. An infrequent
            // fallback covers coalesced notices without queuing reads ahead of writes.
            loop {
                let affected = tokio::select! {
                    notice = notices.recv() => {
                        Some(notice.expect("the archive invalidation listener must stay installed"))
                    }
                    _ = SystemClock.sleep(Duration::from_secs(4)) => None,
                };
                for (index, ((signed, target), covered)) in
                    expected.iter().zip(&mut progress).enumerate()
                {
                    if *covered || affected.is_some_and(|value| value != index.min(2)) {
                        continue;
                    }
                    let permit = newcomer_work
                        .foreground(newcomer_scope.clone())
                        .await
                        .expect("the selected receiver must remain live");
                    let (prepared, selection, channel, scope) =
                        capture_joined_worker(&new_state, id, generation).await;
                    let (signed, target) = (*signed, *target);
                    *covered = tauri::async_runtime::spawn_blocking(move || {
                        let _permit = permit;
                        prepared
                            .selected_archive_target_data(&selection, channel, signed, &scope)
                            .is_ok_and(|selected| selected == Some(target))
                    })
                    .await
                    .unwrap();
                }
                if progress.iter().all(|covered| *covered) {
                    break;
                }
            }
        },
    )
    .await;
    if recovery.is_err() {
        parked_live.release.notify_one();
        for state in [&member_state, &new_state] {
            let _ = timeout(Duration::from_secs(15), async {
                if let Ok(_close) = register_vault_close(state).await {
                    let _ = sources::close_server_rows(state, &SystemClock).await;
                }
            })
            .await;
        }
        panic!(
            "automatic history before gossip: own Delete {}, deepest reply {}, carried Edit {}, carried Delete {}",
            progress[0], progress[1], progress[2], progress[3],
        );
    }
    assert_eq!(
        paid_reply.attempts.load(Ordering::SeqCst),
        original_attempts,
        "local staging contention must not request the same signed parent again"
    );
    *new_state
        .native_text
        .join_history_reply_probe
        .lock()
        .unwrap() = None;
    newcomer.unlisten(notice_listener);
    let reply_view = command_value_after_busy(
        &new_window,
        &new_queue,
        "get_message_page",
        history::BUSY,
        serde_json::json!({"server": new_row["server"], "channel": new_row["channel"],
            "expectedContext": new_row["context"], "receivedAction": reply_id}),
    )
    .await;
    assert_eq!(reply_view["item"]["text"], "Reply after the Add");
    let nested_view = command_value_after_busy(
        &new_window,
        &new_queue,
        "get_message_page",
        history::BUSY,
        serde_json::json!({"server": new_row["server"], "channel": new_row["channel"],
            "expectedContext": new_row["context"], "receivedAction": deepest_reply}),
    )
    .await;
    assert_eq!(nested_view["item"]["text"], "Nested reply 5");
    let deleted_view = command_value_after_busy(
        &new_window,
        &new_queue,
        "get_message_page",
        history::BUSY,
        serde_json::json!({"server": new_row["server"], "channel": new_row["channel"],
            "expectedContext": new_row["context"], "receivedAction": parent}),
    )
    .await;
    assert!(deleted_view["item"].is_null());
    let carried_view = command_value_after_busy(
        &new_window,
        &new_queue,
        "get_message_page",
        history::BUSY,
        serde_json::json!({"server": new_row["server"], "channel": new_row["channel"],
            "expectedContext": new_row["context"], "receivedAction": offline_message}),
    )
    .await;
    assert!(carried_view["item"].is_null());
    let forgotten = command_value_after_busy(
        &new_window,
        &new_queue,
        "forget_native_chat_contacts",
        membership::BUSY,
        serde_json::json!({"server": new_row["server"], "expectedContext": new_row["context"]}),
    )
    .await;
    assert!(forgotten.is_null());
    parked_live.release.notify_one();
    for state in [&member_state, &new_state] {
        let _close = register_vault_close(state).await.unwrap();
        sources::close_server_rows(state, &SystemClock)
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_expired_preview_reproves_the_original_pending_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("joiner");
    let (founder, main, founder_queue) = app(root.path().join("founder"), None, true);
    let (joiner, window, queue) = app(path.clone(), None, true);
    unlock_current(&founder, &main, &founder_queue).await;
    unlock_current(&joiner, &window, &queue).await;
    let found = found_request(&main, 1, "Founder");
    run(&founder_queue).await;
    let found = found_value(found.await.unwrap());
    let minted = invite_command(
        &main,
        "mint_invite_fresh",
        serde_json::json!({"server": found["server"], "expectedContext": found["context"]}),
    );
    run(&founder_queue).await;
    let code = found_value(minted.await.unwrap())
        .as_str()
        .unwrap()
        .to_owned();
    let wire = hex::decode(&code).unwrap();
    let invite = catcoms_app::store::NativeTextInvitePacketData::decode(&wire).unwrap();
    let expires = invite.binding().spec_data().expires_at_ms;

    let state = joiner.state::<AppState>();
    state
        .native_text
        .invite_preview_now
        .store(expires, Ordering::SeqCst);
    let fresh = preview_request(&window, code.clone());
    run(&queue).await;
    refused(fresh.await.unwrap());
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let target = state.native_text.ui_state.original_target().unwrap();
    let caller = keep_session::current(&state, generation).unwrap();
    let preparation = joining::capture_once(&state, generation, &target, |sources, command| {
        sources.capture_join_request(command)
    })
    .await
    .unwrap();
    let token = joining::request_token(&invite);
    preparation
        .cover(
            &invite,
            &invite.binding().spec_data().anchor,
            token,
            expires - 1,
            &caller,
            &mut OsCryptoRng,
        )
        .unwrap();
    let pending = path.join("chat-critical-namespaces.joined-pending.bin");
    let original = std::fs::read(&pending).unwrap();
    let occupied = {
        let store = state.store.lock().await;
        let vault = store.as_ref().unwrap().as_text_v2().unwrap();
        vault.prepare_native_authentication(state.native_text.ui_budget.prepare().unwrap())
    };
    assert!(!occupied.is_capture_busy_cached());
    let warm = preview_request(&window, code.clone());
    let release = async {
        SystemClock.sleep(Duration::from_millis(100)).await;
        drop(occupied);
    };
    tokio::join!(run(&queue), release);
    let warm = found_value(warm.await.unwrap());
    assert_eq!(warm["expires_at_ms"], expires);

    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop((close, caller, target, state));
    drop((window, joiner));
    let (reopened, window, queue) = app(path, None, true);
    unlock_current(&reopened, &window, &queue).await;
    let state = reopened.state::<AppState>();
    state
        .native_text
        .invite_preview_now
        .store(expires, Ordering::SeqCst);
    let cold = preview_request(&window, code);
    run(&queue).await;
    assert_eq!(found_value(cold.await.unwrap()), warm);
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let target = state.native_text.ui_state.original_target().unwrap();
    let caller = keep_session::current(&state, generation).unwrap();
    let preparation = joining::capture_once(&state, generation, &target, |sources, command| {
        sources.capture_joined_selection(command)
    })
    .await
    .unwrap();
    let mut wrong_token = token;
    wrong_token[0] ^= 1;
    assert!(preparation
        .require_recovery_invite(&invite, wrong_token, &caller)
        .is_err());
    assert_eq!(std::fs::read(pending).unwrap(), original);
    assert!(state.servers.lock().await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_connected_join_retries_original_welcome_after_authenticated_unlock() {
    use catcoms_rt::{MeshTransport, ProtocolId, TransportError, CHAT_V2_JOIN_KIND};
    use catcoms_sync::{DirectJoinLimits, PreparedV2JoinRequest};

    fn read_field<'a>(wire: &mut &'a [u8]) -> &'a [u8] {
        let bytes = *wire;
        let size = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        let (field, rest) = bytes[4..].split_at(size);
        *wire = rest;
        field
    }

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    assert!(path
        .join("chat-critical-namespaces.joined-pin.index")
        .is_file());
    let found = found_request(&main, 1, "Connected founder");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let id = found["server"].as_u64().unwrap();
    let state = app.state::<AppState>();
    let mint = || {
        invite_command(
            &main,
            "mint_invite_fresh",
            serde_json::json!({ "server": id, "expectedContext": found["context"] }),
        )
    };
    for arguments in [
        serde_json::json!({ "server": id }),
        serde_json::json!({ "server": id, "expectedContext": "fe".repeat(32) }),
    ] {
        let wrong = invite_command(&main, "mint_invite_fresh", arguments);
        refused(wrong.await.unwrap());
    }
    assert!(queue.lock().unwrap().is_empty());
    let first = mint();
    run(&queue).await;
    let original_code = found_value(first.await.unwrap());
    let retry = mint();
    run(&queue).await;
    assert_eq!(found_value(retry.await.unwrap()), original_code);
    let hex = original_code.as_str().unwrap().to_owned();
    let packet = hex::decode(&hex).unwrap();
    let invite = catcoms_app::store::NativeTextInvitePacketData::decode(&packet).unwrap();
    let expires = invite.binding().spec_data().expires_at_ms;
    let preview = preview_request(&main, hex.clone());
    run(&queue).await;
    assert_eq!(
        found_value(preview.await.unwrap()),
        serde_json::json!({
            "direct_routes": 0, "rendezvous_routes": 0, "switchboards": 0,
            "expires_at_ms": expires, "communication_mode": "peer_to_peer",
            "approved_host_required": true
        })
    );
    state
        .native_text
        .invite_preview_now
        .store(expires, Ordering::SeqCst);
    // Even this founder's exact code has no covered join in this vault.
    let expired = preview_request(&main, hex.clone());
    run(&queue).await;
    let tauri::ipc::InvokeResponse::Err(error) = expired.await.unwrap() else {
        panic!("an expired invite cannot start a new join");
    };
    assert_eq!(
        error.0,
        serde_json::json!("This Chat V2 invite has expired.")
    );
    state
        .native_text
        .invite_preview_now
        .store(0, Ordering::SeqCst);
    let trailing = preview_request(&main, format!("{hex}00"));
    run(&queue).await;
    refused(trailing.await.unwrap());
    let too_large = preview_request(
        &main,
        "00".repeat(catcoms_app::store::MAX_NATIVE_TEXT_INVITE_PACKET_BYTES + 1),
    );
    refused(too_large.await.unwrap());
    assert!(queue.lock().unwrap().is_empty());
    let request = PreparedV2JoinRequest::prepare(
        catcoms_mls::MlsDevice::generate().unwrap(),
        &invite.binding().spec_data().anchor,
        invite.binding(),
        [72; 32],
        expires - 1,
        DirectJoinLimits::new(512 * 1024, 512 * 1024, 64 * 1024).unwrap(),
    )
    .unwrap();
    let mut wire = vec![CHAT_V2_JOIN_KIND];
    wire.extend_from_slice(request.request_bytes_data());
    let wire = bytes::Bytes::from(wire);

    // The in-process test knows this exact original node listens on loopback.
    // Read the already authenticated reservation solely to address that fixture.
    let mut route = invite.reservation_bytes();
    read_field(&mut route);
    route = &route[2..];
    read_field(&mut route);
    read_field(&mut route);
    let peer = std::str::from_utf8(read_field(&mut route)).unwrap();
    read_field(&mut route);
    let port = u16::from_be_bytes(route[2..4].try_into().unwrap());
    let (client, _) = catcoms_net::MeshService::new_tcp(None, &[]).unwrap();
    client
        .dial(
            format!("/ip4/127.0.0.1/tcp/{port}/p2p/{peer}")
                .parse()
                .unwrap(),
        )
        .await
        .unwrap();
    let peer = invite.connected_peer_data();
    tokio::time::timeout(
        Duration::from_secs(10),
        client.wait_for_peer_connected(peer),
    )
    .await
    .unwrap()
    .unwrap();
    let send = || client.request_connected(peer, ProtocolId("/catcoms/rr/1"), wire.clone());
    let original = send().await.unwrap();
    request
        .validate_first_join_delivery_data(&original)
        .unwrap();

    let stale_preview = preview_request(&main, hex);
    let locked = snapshot_request(&main, None);
    run(&queue).await;
    assert!(snapshot_result(locked.await.unwrap())["continuity_error"].is_null());
    run(&queue).await;
    refused(stale_preview.await.unwrap());
    refused(mint().await.unwrap());
    assert!(matches!(send().await, Err(TransportError::NoResponse)));
    let occupied = hold_vault_metadata(&state).await;
    let release = async {
        SystemClock.sleep(Duration::from_millis(100)).await;
        drop(occupied);
    };
    tokio::join!(unlock_current(&app, &main, &queue), release);
    assert_eq!(send().await.unwrap(), original);

    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop(close);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_connected_join_gated_route_publishes_one_original_joined_row() {
    native_text_connected_join(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_unconfirmed_delete_invalidates_the_visible_founder_message() {
    native_text_connected_join(true).await;
}

async fn native_text_connected_join(block_delete_archive: bool) {
    let root = tempfile::tempdir().unwrap();
    let (founder, founder_window, founder_queue) = app(root.path().join("founder"), None, true);
    let joiner_path = root.path().join("joiner");
    let (joiner, joiner_window, joiner_queue) = app(joiner_path.clone(), None, true);
    unlock_current(&founder, &founder_window, &founder_queue).await;
    unlock_current(&joiner, &joiner_window, &joiner_queue).await;
    let found = found_request(&founder_window, 1, "Founder group");
    run(&founder_queue).await;
    let found = found_value(found.await.unwrap());
    let listing = resume_request(&founder_window);
    run(&founder_queue).await;
    let listing = found_value(listing.await.unwrap());
    let [warm_founder] = listing["servers"].as_array().unwrap().as_slice() else {
        panic!("expected the original founder row");
    };
    assert_eq!(warm_founder["name"], "Founder group");
    assert_eq!(warm_founder["display_name"], "Founder");
    let minted = invite_command(
        &founder_window,
        "mint_invite_fresh",
        serde_json::json!({ "server": found["server"], "expectedContext": found["context"] }),
    );
    run(&founder_queue).await;
    let code = found_value(minted.await.unwrap());
    let preview = preview_request(&joiner_window, code.as_str().unwrap().to_owned());
    run(&joiner_queue).await;
    let preview = found_value(preview.await.unwrap());
    assert_eq!(preview["approved_host_required"], true);
    assert_eq!(preview["direct_routes"], 0);
    let join = |host, display| {
        invite_command(
            &joiner_window,
            "join_server",
            serde_json::json!({
                "inviteHex": code, "approvedHost": host, "displayName": display,
                "serverName": "My joined group", "isDm": false, "allowSwitchboards": false,
            }),
        )
    };
    refused(join("localhost", "Member").await.unwrap());
    refused(join("0.0.0.0", "Member").await.unwrap());
    assert!(joiner_queue.lock().unwrap().is_empty());
    let joiner_state = joiner.state::<AppState>();
    let occupied = {
        let store = joiner_state.store.lock().await;
        let vault = store.as_ref().unwrap().as_text_v2().unwrap();
        vault.prepare_native_authentication(joiner_state.native_text.ui_budget.prepare().unwrap())
    };
    assert!(!occupied.is_capture_busy_cached());
    let startup = AdmissionBarrier::new();
    let history = AdmissionBarrier::new();
    *joiner_state.native_text.join_startup_edge.lock().unwrap() = Some(startup.clone());
    *joiner_state
        .native_text
        .history_capture_edge
        .lock()
        .unwrap() = Some(history.clone());
    let original = join("127.0.0.1", "Member");
    let join_task = tokio::spawn(joiner_queue.lock().unwrap().pop().unwrap());
    let initial_response = tokio::time::timeout(Duration::from_secs(30), async {
        let release = async {
            SystemClock.sleep(Duration::from_millis(100)).await;
            drop(occupied);
        };
        tokio::join!(original, release).0.unwrap()
    })
    .await
    .expect("the native Join completes over the founder TCP actor");
    assert!(
        matches!(initial_response, tauri::ipc::InvokeResponse::Ok(_)),
        "initial native join after metadata capture release"
    );
    let original = found_value(initial_response);
    assert_eq!(original["backend"], "text-v2");
    assert_eq!(original["name"], "My joined group");
    assert_eq!(original["display_name"], "Member");
    assert_eq!(original["channel"], found["channel"]);
    assert!(
        joiner_path.join("chat-v2-review-received").is_dir(),
        "the first native joined result already has its original receiver"
    );
    timeout(Duration::from_secs(5), startup.arrived.notified())
        .await
        .expect("Join published before starting the original row workers");
    let first_history = native_history_request(&joiner_window, &original, "0", None, 8);
    let history_task = tokio::spawn(joiner_queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(5), history.arrived.notified())
        .await
        .expect("first history read holds the original metadata slot");
    startup.release.notify_one();
    timeout(
        Duration::from_secs(5),
        joiner_state.native_text.join_startup_busy.notified(),
    )
    .await
    .expect("startup observed the same metadata slot busy while history was paused");
    {
        let _table = joiner_state.servers.lock().await;
        timeout(
            Duration::from_secs(5),
            joiner_state.native_text.join_startup_table_busy.notified(),
        )
        .await
        .expect("startup retries a contended row lookup before attaching workers");
    }
    let id = original["server"].as_u64().unwrap();
    let generation = joiner_state.ui_session_generation.load(Ordering::SeqCst);
    let (original_ingress, _, _, _) = capture_joined_worker(&joiner_state, id, generation).await;
    let (ingress, release_ingress) = BlockingIoBarrier::new();
    let held = ingress.clone();
    let holder = tokio::task::spawn_blocking(move || {
        original_ingress.hold_dependency_ingress_for_test(|| held.wait_once())
    });
    timeout(Duration::from_secs(5), ingress.arrived.notified())
        .await
        .expect("the original network ingress mutex is held");
    history.release.notify_one();
    timeout(
        Duration::from_secs(5),
        joiner_state
            .native_text
            .join_dependency_ingress_busy
            .notified(),
    )
    .await
    .expect("startup reached the actual contended dependency ingress");
    drop(release_ingress);
    assert_eq!(holder.await.unwrap(), Some(()));
    timeout(Duration::from_secs(15), history_task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        found_value(first_history.await.unwrap())["items"],
        serde_json::json!([])
    );
    timeout(Duration::from_secs(15), join_task)
        .await
        .expect("the original Join starts its workers after history drains")
        .unwrap();
    let state = joiner.state::<AppState>();
    let instance = state
        .servers
        .lock()
        .await
        .native(&id)
        .unwrap()
        .data()
        .instance;
    let original_publication = assert_joined_connected_workers(&state, id).await;
    let probe = native_received_request(&joiner_window, &original);
    run(&joiner_queue).await;
    assert_eq!(
        found_value(probe.await.unwrap()),
        serde_json::json!({"backend":"text-v2","scope":"received-live","item":null})
    );
    let archive_request = |row: &serde_json::Value, payload: serde_json::Value| {
        let mut body = payload;
        body["server"] = row["server"].clone();
        body["channel"] = row["channel"].clone();
        body["expectedContext"] = row["context"].clone();
        body
    };
    let reviewed = command_value_after_busy(
        &joiner_window,
        &joiner_queue,
        "get_message_page",
        history::BUSY,
        archive_request(&original, serde_json::json!({ "reviewedAfter": null })),
    )
    .await;
    assert_eq!(
        reviewed,
        serde_json::json!({
            "backend": "text-v2", "scope": "reviewed-archive-index",
            "status": "current", "ids": [], "next": null
        })
    );
    let empty_archive = serde_json::json!({
        "backend":"text-v2","scope":"received-archive-index","available":true,"ids":[],"more":false,
    });
    let probe = command_value_after_busy(
        &joiner_window,
        &joiner_queue,
        "get_message_page",
        history::BUSY,
        archive_request(
            &original,
            serde_json::json!({"archiveAfter":null,"limit":8}),
        ),
    )
    .await;
    assert_eq!(probe, empty_archive);
    let missing = hex::encode([249; 32]);
    let probe = command_value_after_busy(
        &joiner_window,
        &joiner_queue,
        "get_message_page",
        history::BUSY,
        archive_request(&original, serde_json::json!({"archivedAction":missing})),
    )
    .await;
    assert_eq!(
        probe,
        serde_json::json!({"backend":"text-v2","scope":"received-archive","item":null})
    );
    let work = joined_work(&state, id).await;
    let scope =
        keep_session::current(&state, state.ui_session_generation.load(Ordering::SeqCst)).unwrap();
    let target = catcoms_replication::chat::ActionId::from_bytes([249; 32]);
    let edit = catcoms_replication::chat::ActionId::from_bytes([248; 32]);
    let delete = catcoms_replication::chat::ActionId::from_bytes([247; 32]);
    {
        let _permit = work.foreground(scope.clone()).await.unwrap();
        assert_eq!(work.reserve_archive_change(edit, target), Some(true));
        assert_eq!(work.reserve_archive_change(delete, target), Some(true));
        work.settle_archive_change(edit, target);
    }
    let probe = command_after_busy(
        &joiner_window,
        &joiner_queue,
        "get_message_page",
        history::BUSY,
        archive_request(&original, serde_json::json!({"archivedAction":missing})),
    )
    .await;
    let tauri::ipc::InvokeResponse::Err(error) = probe else {
        panic!("the archive replied before its outstanding deletion settled");
    };
    assert_eq!(error.0, "A received change is waiting to be saved.");
    {
        let _permit = work.foreground(scope).await.unwrap();
        work.settle_archive_change(delete, target);
    }
    drop(work);
    let probe = command_value_after_busy(
        &joiner_window,
        &joiner_queue,
        "get_message_page",
        history::BUSY,
        archive_request(&original, serde_json::json!({"archivedAction":missing})),
    )
    .await;
    assert_eq!(
        probe,
        serde_json::json!({"backend":"text-v2","scope":"received-archive","item":null})
    );
    let probe = invite_command(
        &joiner_window,
        "get_message_page",
        archive_request(
            &original,
            serde_json::json!({"archiveAfter":null,"limit":8,"archivedAction":missing}),
        ),
    );
    refused(probe.await.unwrap());
    assert!(joiner_queue.lock().unwrap().is_empty());

    let founder_text = "Founder message on the installed native mesh";
    let sent = native_send_request(&founder_window, &found, 21, founder_text);
    run(&founder_queue).await;
    let sent = found_value(sent.await.unwrap());
    assert_eq!(sent["disposition"], "saved-local");
    let founder_action = sent["message_id"].as_str().unwrap();
    let mut raw = [0; 32];
    hex::decode_to_slice(founder_action, &mut raw).unwrap();
    let action = catcoms_replication::chat::ActionId::from_bytes(raw);
    timeout(Duration::from_secs(15), async {
        loop {
            let result = command_after_busy(
                &joiner_window,
                &joiner_queue,
                "get_message_page",
                history::BUSY,
                archive_request(
                    &original,
                    serde_json::json!({"receivedAction":founder_action}),
                ),
            )
            .await;
            if let tauri::ipc::InvokeResponse::Ok(raw) = result {
                let received: serde_json::Value = raw.deserialize().unwrap();
                if !received["item"].is_null() {
                    assert_eq!(received["item"]["text"], founder_text);
                    break;
                }
            }
            SystemClock.sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the automatic founder worker selected exact text on the joined receiver");

    use tauri::Listener;
    let delete_notices = block_delete_archive.then(|| {
        let (notices, changes) = tokio::sync::mpsc::unbounded_channel();
        let listener = founder.listen_any("chat-v2-received", move |event| {
            let body: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
            let _ = notices.send(body);
        });
        (listener, changes)
    });
    let member_text = "Joined message on the founder's installed receiver";
    let send = native_send_request(&joiner_window, &original, 20, member_text);
    run(&joiner_queue).await;
    let send = found_value(send.await.unwrap());
    let member_action = send["message_id"].as_str().unwrap();
    assert_eq!(send["ordinal"], "0");
    timeout(Duration::from_secs(18), async {
        loop {
            let received = invite_command(&founder_window, "get_message_page",
                archive_request(&found, serde_json::json!({"receivedAction":member_action})));
            let task = founder_queue.lock().unwrap().pop();
            if let Some(task) = task { task.await; }
            if let tauri::ipc::InvokeResponse::Ok(body) = received.await.unwrap() {
                let body: serde_json::Value = body.deserialize().unwrap();
                if !body["item"].is_null() {
                    assert_eq!(body, serde_json::json!({
                        "backend":"text-v2","scope":"received-live",
                        "item":{"action":member_action,"text":member_text,"certainty":"provisional"},
                    }));
                    break;
                }
            }
            SystemClock.sleep(Duration::from_millis(35)).await;
        }
        loop {
            let page = native_history_request(&joiner_window, &original, "0", None, 8);
            let task = joiner_queue.lock().unwrap().pop();
            if let Some(task) = task { task.await; }
            if let tauri::ipc::InvokeResponse::Ok(body) = page.await.unwrap() {
                let body: serde_json::Value = body.deserialize().unwrap();
                if body["items"][0]["status"] == "submitted" {
                    assert_eq!(body["items"][0]["action"], member_action);
                    break;
                }
            }
            SystemClock.sleep(Duration::from_millis(35)).await;
        }
    }).await.expect("the native founder received the member's original over TCP and the joined outbox checkpointed");

    let page = command_value_after_busy(
        &founder_window,
        &founder_queue,
        "get_message_page",
        history::BUSY,
        archive_request(&found, serde_json::json!({"archiveAfter":null,"limit":8})),
    )
    .await;
    assert_eq!(
        page,
        serde_json::json!({
            "backend":"text-v2","scope":"received-archive-index","available":true,
            "ids":[member_action],"more":false,
        })
    );
    let archived = command_value_after_busy(
        &founder_window,
        &founder_queue,
        "get_message_page",
        history::BUSY,
        archive_request(&found, serde_json::json!({"archivedAction":member_action})),
    )
    .await;
    assert_eq!(
        archived,
        serde_json::json!({
            "backend":"text-v2","scope":"received-archive",
            "item":{"action":member_action,"text":member_text,"certainty":"provisional"},
        })
    );
    let audit = |row: &serde_json::Value, target| {
        archive_request(
            row,
            serde_json::json!({"auditAction":target,"auditAfter":null,"limit":8}),
        )
    };
    if let Some((listener, mut changes)) = delete_notices {
        timeout(Duration::from_secs(12), async {
            while let Some(notice) = changes.recv().await {
                if notice["action"] == member_action {
                    return;
                }
            }
            panic!("the original Create notice did not arrive");
        })
        .await
        .unwrap();
        while changes.try_recv().is_ok() {}
        let wal = root
            .path()
            .join("founder/chat-v2-received-history/history.wal");
        assert!(!wal.exists());
        let obstruction = b"incomplete history write";
        std::fs::write(&wal, obstruction).unwrap();
        let deletion = command_value_after_busy(
            &joiner_window,
            &joiner_queue,
            "delete_message",
            sending::BUSY,
            archive_request(
                &original,
                serde_json::json!({"retryToken":hex::encode([30;32]), "messageId":member_action,
                    "expectedRevision":member_action}),
            ),
        )
        .await;
        assert_eq!(deletion["disposition"], "saved-local");
        let action_id = |hex: &str| {
            catcoms_replication::chat::ActionId::from_bytes(
                hex::decode(hex).unwrap().try_into().unwrap(),
            )
        };
        let action = action_id(deletion["message_id"].as_str().unwrap());
        let target = action_id(member_action);
        let state = founder.state::<AppState>();
        let work = joined_work(&state, found["server"].as_u64().unwrap()).await;
        timeout(Duration::from_secs(18), async {
            while let Some(notice) = changes.recv().await {
                if notice["action"] == member_action {
                    assert_eq!(notice["context"], found["context"]);
                    assert_eq!(notice["server"], found["server"]);
                    if work
                        .unsettled_archive_changes(target)
                        .unwrap()
                        .contains(&Some(action))
                    {
                        return;
                    }
                }
            }
            panic!("the invalidation listener closed");
        })
        .await
        .expect("the original founder row invalidates the displayed Create before settlement");
        let result = command_after_busy(
            &founder_window,
            &founder_queue,
            "get_message_page",
            history::BUSY,
            archive_request(&found, serde_json::json!({"receivedAction":member_action})),
        )
        .await;
        refused(result);
        assert_eq!(std::fs::read(&wal).unwrap(), obstruction);
        founder.unlisten(listener);
        std::fs::remove_file(wal).unwrap();
        for state in [founder.state::<AppState>(), joiner.state::<AppState>()] {
            let close = register_vault_close(&state).await.unwrap();
            sources::close_server_rows(&state, &SystemClock)
                .await
                .unwrap();
            drop(close);
        }
        return;
    }
    let warm_audit = command_value_after_busy(
        &founder_window,
        &founder_queue,
        "get_message_page",
        history::BUSY,
        audit(&found, None),
    )
    .await;
    assert_eq!(warm_audit["scope"], "selected-shared-history-audit");
    assert!(warm_audit["target"].is_null());
    assert_eq!(warm_audit["items"].as_array().unwrap().len(), 1);
    assert_eq!(warm_audit["items"][0]["action"], member_action);
    assert_eq!(warm_audit["items"][0]["body"]["text"], member_text);
    for field in ["signer", "admission", "original_digest"] {
        assert_eq!(warm_audit["items"][0][field].as_str().unwrap().len(), 64);
    }
    let denied = command_after_busy(
        &joiner_window,
        &joiner_queue,
        "get_message_page",
        history::BUSY,
        audit(&original, Some(member_action)),
    )
    .await;
    let tauri::ipc::InvokeResponse::Err(error) = denied else {
        panic!("a joined row returned the founder's retained records");
    };
    assert_eq!(
        error.0,
        "Only the server owner can inspect retained records."
    );

    // The saved native row can be reported after the actual founder retires.
    let locked = snapshot_request(&founder_window, None);
    run(&founder_queue).await;
    assert!(snapshot_result(locked.await.unwrap())["continuity_error"].is_null());
    let retry = join("127.0.0.1", "Member");
    run(&joiner_queue).await;
    assert_eq!(found_value(retry.await.unwrap()), original);
    let changed = join("127.0.0.1", "Changed member");
    run(&joiner_queue).await;
    refused(changed.await.unwrap());
    assert_eq!(
        state
            .servers
            .lock()
            .await
            .native(&id)
            .unwrap()
            .data()
            .instance,
        instance
    );
    let listed = resume_request(&joiner_window);
    run(&joiner_queue).await;
    let listed = found_value(listed.await.unwrap());
    assert_eq!(
        listed["servers"].as_array().unwrap(),
        std::slice::from_ref(&original)
    );
    let local = joined_work(&state, id)
        .await
        .foreground(
            keep_session::current(&state, state.ui_session_generation.load(Ordering::SeqCst))
                .unwrap(),
        )
        .await
        .unwrap();
    let empty = native_history_request(&joiner_window, &original, "0", None, 8);
    let pending = run(&joiner_queue);
    tokio::pin!(pending);
    assert!(timeout(Duration::from_millis(15), &mut pending)
        .await
        .is_err());
    drop(local);
    pending.await;
    let empty = found_value(empty.await.unwrap());
    assert_eq!(empty["end_exclusive"], "1");
    assert_eq!(empty["items"][0]["action"], member_action);
    let send = native_send_request(&joiner_window, &original, 9, "Joined message");
    run(&joiner_queue).await;
    let saved = found_value(send.await.unwrap());
    assert_eq!(saved["disposition"], "saved-local");
    assert!(saved["message_id"]
        .as_str()
        .is_some_and(|id| id.len() == 64));
    let history = native_history_request(&joiner_window, &original, "0", None, 8);
    run(&joiner_queue).await;
    let history = found_value(history.await.unwrap());
    assert_eq!(history["scope"], "own-saved-originals");
    assert_eq!(history["end_exclusive"], "2");
    assert_eq!(history["items"][1]["action"], saved["message_id"]);
    assert_eq!(history["items"][1]["body"]["text"], "Joined message");
    let locked = snapshot_request(&joiner_window, None);
    run(&joiner_queue).await;
    assert!(snapshot_result(locked.await.unwrap())["continuity_error"].is_null());

    let saved_pins =
        std::fs::read(joiner_path.join("chat-critical-namespaces.joined-pin.index")).unwrap();
    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    assert_joined_publication_stopped(original_publication).await;
    drop(close);
    drop(joiner);
    let (reopened, window, queue) = app(joiner_path.clone(), None, true);
    let wrong = invoke(&window, "1", "different secret");
    run(&queue).await;
    refused(wrong.await.unwrap());
    let state = reopened.state::<AppState>();
    assert!(state.store.lock().await.is_none());
    assert!(state.servers.lock().await.native(&id).is_none());
    let unlocked = invoke(&window, "1", SECRET);
    run(&queue).await;
    let restored = found_value(unlocked.await.unwrap());
    let restored = restored["servers"].as_array().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0], original);
    let code_text = code.as_str().unwrap();
    let wire = hex::decode(code_text).unwrap();
    let invite = catcoms_app::store::NativeTextInvitePacketData::decode(&wire).unwrap();
    let expires = invite.binding().spec_data().expires_at_ms;
    state
        .native_text
        .invite_preview_now
        .store(expires, Ordering::SeqCst);
    let expired = preview_request(&window, code_text.to_owned());
    run(&queue).await;
    let preview = found_value(expired.await.unwrap());
    assert_eq!(preview["expires_at_ms"], expires);
    assert_eq!(preview["approved_host_required"], true);
    let recovered_publication = assert_joined_connected_workers(&state, id).await;
    let retry = native_send_request(&window, &restored[0], 9, "Joined message");
    run(&queue).await;
    assert_eq!(found_value(retry.await.unwrap()), saved);
    let page = native_history_request(&window, &restored[0], "0", Some("2"), 8);
    run(&queue).await;
    assert_eq!(found_value(page.await.unwrap()), history);
    let changed = native_send_request(&window, &restored[0], 9, "Changed joined message");
    run(&queue).await;
    refused(changed.await.unwrap());

    let founder_state = founder.state::<AppState>();
    let close = register_vault_close(&founder_state).await.unwrap();
    sources::close_server_rows(&founder_state, &SystemClock)
        .await
        .unwrap();
    drop(close);
    drop(founder_window);
    drop(founder);

    let fresh = invite_command(
        &window,
        "send_message",
        archive_request(
            &restored[0],
            serde_json::json!({
                "retryToken":hex::encode([10;32]),
                "text":"Message after restart", "replyTo":member_action,
            }),
        ),
    );
    run(&queue).await;
    let fresh = found_value(fresh.await.unwrap());
    assert_eq!(fresh["disposition"], "saved-local");
    assert_eq!(fresh["ordinal"], "2");
    assert_ne!(fresh["message_id"], saved["message_id"]);
    let fixed_page = native_history_request(&window, &restored[0], "0", Some("2"), 8);
    run(&queue).await;
    assert_eq!(found_value(fixed_page.await.unwrap()), history);
    let probe = command_value_after_busy(
        &window,
        &queue,
        "get_message_page",
        history::BUSY,
        archive_request(
            &restored[0],
            serde_json::json!({"archiveAfter":null,"limit":8}),
        ),
    )
    .await;
    assert_eq!(
        probe,
        serde_json::json!({
            "backend":"text-v2","scope":"received-archive-index","available":true,"ids":[founder_action],"more":false,
        })
    );
    let probe = command_value_after_busy(
        &window,
        &queue,
        "get_message_page",
        history::BUSY,
        archive_request(
            &restored[0],
            serde_json::json!({"archivedAction":founder_action}),
        ),
    )
    .await;
    assert_eq!(
        probe,
        serde_json::json!({
            "backend":"text-v2","scope":"received-archive",
            "item":{"action":founder_action,"text":founder_text,"certainty":"provisional"},
        })
    );
    assert_eq!(
        std::fs::read(joiner_path.join("chat-critical-namespaces.joined-pin.index")).unwrap(),
        saved_pins
    );
    let (reopened_founder, founder_window, founder_queue) =
        app(root.path().join("founder"), None, true);
    let wrong = invoke(&founder_window, "1", "different secret");
    run(&founder_queue).await;
    refused(wrong.await.unwrap());
    let state = reopened_founder.state::<AppState>();
    assert!(state.store.lock().await.is_none());
    assert!(state.servers.lock().await.native_iter().next().is_none());
    let unlock = invoke(&founder_window, "1", SECRET);
    run(&founder_queue).await;
    let restored = found_value(unlock.await.unwrap());
    let rows = restored["servers"].as_array().unwrap();
    let [founder_row] = rows.as_slice() else {
        panic!("expected the restored founder row");
    };
    for field in ["server", "name", "display_name", "channel", "backend"] {
        assert_eq!(
            founder_row[field], warm_founder[field],
            "restored founder {field}"
        );
    }
    let archived = command_value_after_busy(
        &founder_window,
        &founder_queue,
        "get_message_page",
        history::BUSY,
        archive_request(
            founder_row,
            serde_json::json!({ "archivedAction": member_action }),
        ),
    )
    .await;
    assert_eq!(archived["item"]["action"], member_action);
    assert_eq!(archived["item"]["text"], member_text);
    let cold_audit = command_value_after_busy(
        &founder_window,
        &founder_queue,
        "get_message_page",
        history::BUSY,
        audit(founder_row, Some(member_action)),
    )
    .await;
    assert_eq!(cold_audit["target"], member_action);
    assert_eq!(cold_audit["items"], warm_audit["items"]);
    let page = native_history_request(&founder_window, founder_row, "0", None, 8);
    run(&founder_queue).await;
    let own = found_value(page.await.unwrap());
    assert_eq!(own["items"][0]["action"], founder_action);
    assert_eq!(own["items"][0]["body"]["text"], founder_text);
    let retry = native_send_request(&founder_window, founder_row, 21, founder_text);
    run(&founder_queue).await;
    assert_eq!(
        found_value(retry.await.unwrap())["message_id"],
        founder_action
    );
    let reply_to = |target: &str| {
        invite_command(
            &founder_window,
            "send_message",
            archive_request(
                founder_row,
                serde_json::json!({
                    "retryToken":hex::encode([22;32]),
                    "text":"Founder message after restart", "replyTo":target,
                }),
            ),
        )
    };
    let missing_parent = "ab".repeat(32);
    let unavailable = reply_to(&missing_parent);
    run(&founder_queue).await;
    refused(unavailable.await.unwrap());
    let send = reply_to(member_action);
    run(&founder_queue).await;
    let saved = found_value(send.await.unwrap());
    assert_eq!(saved["disposition"], "saved-local");
    assert_ne!(saved["message_id"], founder_action);
    let retry = reply_to(member_action);
    run(&founder_queue).await;
    assert_eq!(found_value(retry.await.unwrap()), saved);
    let changed = reply_to(founder_action);
    run(&founder_queue).await;
    refused(changed.await.unwrap());
    let page = native_history_request(&founder_window, founder_row, "0", None, 8);
    run(&founder_queue).await;
    let originals = found_value(page.await.unwrap());
    assert_eq!(originals["items"][1]["action"], saved["message_id"]);
    assert_eq!(originals["items"][1]["body"]["reply_to"], member_action);

    timeout(
        Duration::from_secs(15),
        state.native_text.founder_ingress_ready.notified(),
    )
    .await
    .expect("the cold founder has subscribed on its original mesh");
    let occupied = hold_vault_metadata(&state).await;
    let reconnect = invite_command(
        &window,
        "join_server",
        serde_json::json!({
            "inviteHex":code, "approvedHost":"127.0.0.1", "displayName":"Member",
            "serverName":"New group", "isDm":false, "allowSwitchboards":false,
        }),
    );
    run(&queue).await;
    let reconnect_response = reconnect.await.unwrap();
    assert!(
        matches!(reconnect_response, tauri::ipc::InvokeResponse::Ok(_)),
        "original joined row reconnect after restart"
    );
    assert_eq!(found_value(reconnect_response), original);
    timeout(
        Duration::from_secs(20),
        state.native_text.founder_receive_busy.notified(),
    )
    .await
    .expect("a reconnected member frame reached the held founder metadata slot");
    drop(occupied);
    for (window, queue, row, action, expected) in [
        (
            &window,
            &queue,
            &original,
            &saved["message_id"],
            "Founder message after restart",
        ),
        (
            &founder_window,
            &founder_queue,
            founder_row,
            &fresh["message_id"],
            "Message after restart",
        ),
    ] {
        let received = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let reply = command_after_busy(
                    window,
                    queue,
                    "get_message_page",
                    history::BUSY,
                    archive_request(row, serde_json::json!({"archivedAction":action})),
                )
                .await;
                match reply {
                    tauri::ipc::InvokeResponse::Ok(body) => {
                        let value = body.deserialize::<serde_json::Value>().unwrap();
                        if !value["item"].is_null() {
                            assert_eq!(&value["item"]["action"], action);
                            break value;
                        }
                    }
                    tauri::ipc::InvokeResponse::Err(error)
                        if error.0 == "A received change is waiting to be saved." => {}
                    tauri::ipc::InvokeResponse::Err(error) => {
                        panic!("the waiting archive read refused: {error:?}")
                    }
                }
                SystemClock.sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!("the offline original was not received after reconnect: {expected}")
        });
        assert_eq!(received["item"]["text"], expected);
    }
    assert_eq!(
        std::fs::read(joiner_path.join("chat-critical-namespaces.joined-pin.index")).unwrap(),
        saved_pins,
    );

    let joined_state = reopened.state::<AppState>();
    let packet = hex::decode(code.as_str().unwrap()).unwrap();
    let peer = catcoms_app::store::NativeTextInvitePacketData::decode(&packet)
        .unwrap()
        .connected_peer_data();
    let founder_generation = state.ui_session_generation.load(Ordering::SeqCst);
    let source = {
        let target = state.native_text.ui_state.original_target().unwrap();
        let account = keep_session::current(&state, founder_generation).unwrap();
        let mut paid = Some(state.native_text.ui_budget.prepare().unwrap());
        let table = state.servers.lock().await;
        let founder_id = founder_row["server"].as_u64().unwrap();
        connected::Source::for_row(
            founder_generation,
            &target,
            table.native(&founder_id).unwrap(),
            account,
            &mut paid,
        )
        .unwrap()
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let permit = source
                .member_work()
                .foreground(source.negative())
                .await
                .unwrap();
            if let Some(preparation) = source.capture(&state).await {
                let scope = source.negative();
                let channel = source.channel();
                let attached = tauri::async_runtime::spawn_blocking(move || {
                    let _permit = permit;
                    preparation.attach_founder_dependency_provider(channel, scope)
                })
                .await
                .unwrap();
                if matches!(attached, Err(catcoms_app::store::V2MemberSaveRefusal::Busy)) {
                    break;
                }
                drop(attached);
            } else {
                drop(permit);
            }
            SystemClock.sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the resumed native founder's dependency receiver owns the original lane");
    drop(source);
    let founder_saved = action;
    let joined_generation = joined_state.ui_session_generation.load(Ordering::SeqCst);
    let joined_work = joined_work(&joined_state, id).await;
    let joined_session = keep_session::current(&joined_state, joined_generation).unwrap();
    let mut last_refusal = String::new();
    let recovered_parent = tokio::time::timeout(Duration::from_secs(28), async {
        loop {
            let guard = joined_work
                .foreground(joined_session.clone())
                .await
                .unwrap();
            let (prepare, selection, channel, scope) =
                capture_joined_worker(&joined_state, id, joined_generation).await;
            let exchange = prepare.prepare_dependency_exchange(
                &selection,
                channel,
                peer,
                founder_saved,
                scope,
                &mut OsCryptoRng,
            );
            drop(guard);
            match exchange {
                Ok(exchange) => {
                    match tokio::time::timeout(Duration::from_secs(8), exchange.receive()).await {
                        Ok(Ok(reply)) => {
                            let _guard = joined_work
                                .foreground(joined_session.clone())
                                .await
                                .unwrap();
                            let (prepare, selection, _, scope) =
                                capture_joined_worker(&joined_state, id, joined_generation).await;
                            let original = prepare
                                .open_dependency_response(&selection, reply, &scope)
                                .unwrap()
                                .expect("the cold founder has its exact original outbox receipt");
                            assert_eq!(original.action_id_data(), founder_saved);
                            break;
                        }
                        Ok(Err(error)) => last_refusal = format!("transport reported {error:?}"),
                        Err(_) => last_refusal = "transport timed out".into(),
                    }
                }
                Err(error) => last_refusal = format!("preparation reported {error:?}"),
            }
            SystemClock.sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        recovered_parent.is_ok(),
        "the cold founder's native worker did not answer: {last_refusal}"
    );
    drop(joined_work);
    let close = register_vault_close(&joined_state).await.unwrap();
    sources::close_server_rows(&joined_state, &SystemClock)
        .await
        .unwrap();
    assert_joined_publication_stopped(recovered_publication).await;
    drop(close);
    // Changing only the original founder source must withhold the prepared
    // audit even though the UI generation and table entry still match.
    let edge = AdmissionBarrier::new();
    *state.native_text.text_reply_edge.lock().unwrap() = Some(edge.clone());
    let late = invite_command(
        &founder_window,
        "get_message_page",
        audit(founder_row, Some(member_action)),
    );
    let task = tokio::spawn(founder_queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(10), edge.arrived.notified())
        .await
        .expect("an authorized audit held its exact records before native publication");
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    {
        let table = state.servers.lock().await;
        let row = table
            .native(&founder_row["server"].as_u64().unwrap())
            .unwrap();
        row.data()
            .selection
            .founder()
            .unwrap()
            .close_original_cached();
    }
    edge.release.notify_one();
    task.await.unwrap();
    refused(late.await.unwrap());
    assert_eq!(
        state.ui_session_generation.load(Ordering::SeqCst),
        generation
    );
    *state.native_text.text_reply_edge.lock().unwrap() = None;
    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop(close);
}

async fn assert_joined_connected_workers(
    state: &AppState,
    id: u64,
) -> std::sync::Weak<server_table::NativeMemberWork> {
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let work = joined_work(state, id).await;
    let _local = work
        .foreground(keep_session::current(state, generation).unwrap())
        .await
        .unwrap();
    assert!(Arc::strong_count(&work) >= 3);
    let (preparation, selection, _, scope) = capture_joined_worker(state, id, generation).await;
    let result = tauri::async_runtime::spawn_blocking(move || {
        preparation.attach_dependency_provider_checked(&selection, scope)
    })
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(catcoms_app::store::NativeTextJoinedDependencyAttachRefusal::Occupied)
    ));
    let (preparation, selection, channel, scope) =
        capture_joined_worker(state, id, generation).await;
    let listener = tauri::async_runtime::spawn_blocking(move || {
        preparation.prepare_member_ingress(&selection, channel, scope)
    })
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(
        listener.open().await,
        Err(catcoms_net::NativeChatMemberGossipError::Busy)
    ));
    Arc::downgrade(&work)
}

async fn capture_joined_worker(
    state: &AppState,
    id: u64,
    generation: u64,
) -> (
    catcoms_app::store::NativeTextJoinedSelectionPreparation<OsCryptoRng>,
    catcoms_app::store::NativeTextJoinedRowSelection,
    u128,
    catcoms_rt::RequestCancellation,
) {
    let target = state.native_text.ui_state.original_target().unwrap();
    let mut ui = Some(
        founding::ui_capture(state, generation, &target)
            .await
            .unwrap(),
    );
    let mut preparation = None;
    let (selection, channel, scope) = {
        let _commit = require_ui_session_generation(state, generation)
            .await
            .unwrap();
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        let original = table.native(&id).unwrap();
        let server_table::NativeSelection::Joined(selection) = &original.data().selection else {
            panic!("the registered row must still be the original joined source");
        };
        let scope = original
            .scope(keep_session::current(state, generation).unwrap())
            .unwrap();
        state
            .native_text
            .sources
            .capture_joined_member(&store, &mut ui, selection, &mut preparation)
            .unwrap();
        (selection.clone(), original.data().channel, scope)
    };
    (preparation.unwrap(), selection, channel, scope)
}

async fn joined_work(state: &AppState, id: u64) -> Arc<server_table::NativeMemberWork> {
    state
        .servers
        .lock()
        .await
        .native(&id)
        .unwrap()
        .data()
        .member_work
        .clone()
}

async fn assert_joined_publication_stopped(
    original: std::sync::Weak<server_table::NativeMemberWork>,
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while original.strong_count() != 0 {
            SystemClock.sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the publication worker should release the original closed row");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_connected_join_cold_idle_unlock_requires_the_existing_vault() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (first, main, queue) = app(path.clone(), None, true);
    unlock_current(&first, &main, &queue).await;
    let pin_path = path.join("chat-critical-namespaces.joined-pin.index");
    let vault_path = path.join("vault.bin");
    let original = std::fs::read(&vault_path).unwrap();
    let pin = std::fs::read(&pin_path).unwrap();
    let identities = [&vault_path, &pin_path].map(|file| {
        catcoms_storage::kept::session_id(&std::fs::File::open(file).unwrap()).unwrap()
    });

    let (second, main, queue) = app(path.clone(), None, true);
    let contender = invoke(&main, "1", SECRET);
    run(&queue).await;
    refused(contender.await.unwrap());
    {
        let state = first.state::<AppState>();
        let close = register_vault_close(&state).await.unwrap();
        sources::close_server_rows(&state, &SystemClock)
            .await
            .unwrap();
        drop(close);
    }
    drop(first);

    let wrong = invoke(&main, "1", "a different secret");
    run(&queue).await;
    refused(wrong.await.unwrap());
    let state = second.state::<AppState>();
    assert_locked(&state).await;
    assert!(state.store.lock().await.is_none());
    unlock_current(&second, &main, &queue).await;
    for (file, identity) in [&vault_path, &pin_path].into_iter().zip(identities) {
        assert!(matches!(
            catcoms_storage::kept::session_at(file, identity),
            catcoms_storage::kept::SessionAt::Ours
        ));
    }
    assert_eq!(std::fs::read(&vault_path).unwrap(), original);
    assert_eq!(std::fs::read(&pin_path).unwrap(), pin);

    let founded = found_request(&main, 1, "Founded after restart");
    run(&queue).await;
    let founded = found_value(founded.await.unwrap());
    assert!(founded["server"].is_u64());
    let close = register_vault_close(&state).await.unwrap();
    sources::close_server_rows(&state, &SystemClock)
        .await
        .unwrap();
    drop(close);
    let (third, main, queue) = app(path, None, true);
    unlock_current(&third, &main, &queue).await;
    let row = resume_request(&main);
    run(&queue).await;
    let row = found_value(row.await.unwrap());
    let [reopened] = row["servers"].as_array().unwrap().as_slice() else {
        panic!("expected the original founder created after the first reopen");
    };
    assert_eq!(reopened["server"], founded["server"]);
    assert_eq!(reopened["context"], founded["context"]);
    assert_eq!(reopened["channel"], founded["channel"]);
    for (file, identity) in [&vault_path, &pin_path].into_iter().zip(identities) {
        assert!(matches!(
            catcoms_storage::kept::session_at(file, identity),
            catcoms_storage::kept::SessionAt::Ours
        ));
    }
    assert_eq!(std::fs::read(&vault_path).unwrap(), original);
    assert_eq!(std::fs::read(&pin_path).unwrap(), pin);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_found_final_publication_cannot_cross_actual_whole_close() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let edge = AdmissionBarrier::new();
    let state = app.state::<AppState>();
    *state.native_text.found_publication_edge.lock().unwrap() = Some(edge.clone());
    let reply = found_request(&main, 1, "Paused");
    let task = queue.lock().unwrap().pop().unwrap();
    let task = tokio::spawn(task);
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    assert!(state.servers.lock().await.is_empty());
    let close = register_vault_close(&state).await.unwrap();
    timeout(
        Duration::from_secs(60),
        sources::close_server_rows(&state, &SystemClock),
    )
    .await
    .unwrap()
    .unwrap();
    catcoms_storage::acquire_vault_session(&path).unwrap();
    edge.release.notify_one();
    task.await.unwrap();
    refused(reply.await.unwrap());
    assert!(state.servers.lock().await.is_empty());
    drop(close);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_found_old_unpolled_invoke_and_lost_reply_keep_exact_original_custody() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let before = index(&path);
    let old = found_request(&main, 1, "Original");
    assert_eq!(queue.lock().unwrap().len(), 1);
    lock_session_outcome_inner(&state, None).await;
    run(&queue).await;
    refused(old.await.unwrap());
    assert_eq!(
        index(&path),
        before,
        "old unpolled founding must perform no birth"
    );
    assert!(state.servers.lock().await.is_empty());
    unlock_current(&app, &main, &queue).await;

    let observer = found_request(&main, 1, "Original");
    drop(observer); // native job still owns all work despite the lost reply
    run(&queue).await;
    let (id, instance, original) = {
        let table = state.servers.lock().await;
        assert_eq!(table.native_iter().count(), 1);
        let (id, row) = table.native_iter().next().unwrap();
        (*id, row.data().instance, row.data().selection.clone())
    };
    let retry = found_request(&main, 1, "Original");
    run(&queue).await;
    assert_eq!(found_value(retry.await.unwrap())["server"], id);
    let table = state.servers.lock().await;
    assert_eq!(table.len(), 1);
    let row = table.native(&id).unwrap().data();
    assert_eq!(row.instance, instance);
    assert!(row.selection.same_original(&original));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_send_counted_final_preparation_retires_without_waiting_for_close_commit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let found = found_request(&main, 1, "Original");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let state = app.state::<AppState>();
    let edge = AdmissionBarrier::new();
    *state.native_text.send_final_edge.lock().unwrap() = Some(edge.clone());
    let reply = native_send_request(&main, &found, 4, "Before close");
    let send = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let prelude = register_vault_close(&state).await.unwrap();
    let handle = app.handle().clone();
    let closing = tokio::spawn(async move {
        sources::close_server_rows(&handle.state::<AppState>(), &SystemClock).await
    });
    // This signal follows an actual Root try_retire -> Pending, not merely a
    // task scheduling pause. The prepared Source work still belongs to send.
    timeout(
        Duration::from_secs(60),
        state.native_text.source_retirement_waiting.notified(),
    )
    .await
    .unwrap();
    assert!(!closing.is_finished());
    assert!(state.ui_session_commit.try_lock().is_ok());
    assert!(state.servers.try_lock().is_ok());
    assert_eq!(
        state.servers.lock().await.len(),
        1,
        "retain the same fenced row during incomplete drainage"
    );
    edge.release.notify_one();
    send.await.unwrap();
    refused(reply.await.unwrap());
    timeout(Duration::from_secs(60), closing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(state.servers.lock().await.is_empty());
    catcoms_storage::acquire_vault_session(&path).unwrap();
    drop(prelude);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_send_lost_observer_keeps_actual_admitted_io_ahead_of_lock_completion() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let found = found_request(&main, 1, "Original");
    run(&queue).await;
    let found = found_value(found.await.unwrap());
    let state = app.state::<AppState>();
    let (edge, release) = BlockingIoBarrier::new();
    *state.native_text.send_io_edge.lock().unwrap() = Some(edge.clone());
    let observer = native_send_request(&main, &found, 5, "Saved despite lost reply");
    let send = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    drop(observer);
    // The real admitted file job owns both guards before any IO starts. The
    // pause supplies no authority; releasing it executes actual encrypted IO.
    assert!(state.ui_session_commit.try_lock().is_err());
    let locking = lock_session_outcome_inner(&state, None);
    tokio::pin!(locking);
    pending_once(locking.as_mut()).await;
    assert!(state.session_lock_requested.load(Ordering::SeqCst));
    drop(release);
    send.await.unwrap();
    locking.await;
    *state.native_text.send_io_edge.lock().unwrap() = None;
    unlock_current(&app, &main, &queue).await;
    let retry = native_send_request(&main, &found, 5, "Saved despite lost reply");
    run(&queue).await;
    let saved = found_value(retry.await.unwrap());
    assert_eq!(saved["disposition"], "saved-local");
    assert_eq!(saved["ordinal"], "0");
    let again = native_send_request(&main, &found, 5, "Saved despite lost reply");
    run(&queue).await;
    assert_eq!(found_value(again.await.unwrap()), saved);
}
async fn assert_locked(state: &AppState) {
    assert!(require_unlocked_session(state).await.is_err());
    assert!(!*state.session_resumable.lock().await);
    assert!(state.session_lock_requested.load(Ordering::SeqCst));
}
fn index(path: &Path) -> Vec<u8> {
    std::fs::read(path.join("chat-owner-v2/native-registry.bin")).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_sources_real_boot_envelope_unpolled_and_unwind_drop_files_before_wake() {
    struct WakeAfterFiles {
        path: PathBuf,
        observed: Arc<AtomicBool>,
        released: Arc<AtomicBool>,
    }
    impl std::task::Wake for WakeAfterFiles {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.released.store(
                catcoms_storage::acquire_vault_session(&self.path).is_ok(),
                Ordering::SeqCst,
            );
            self.observed.store(true, Ordering::SeqCst);
        }
    }
    for unwind in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("vault");
        let state = initial_state_selected(true);
        let original = state.native_text.sources.boot.clone();
        let mut output = BootOutput {
            plan: Some(boot::Bootstrap::Fresh),
            outcome: Ok(()),
            _end: original.enter().unwrap(),
        };
        output
            .plan
            .as_mut()
            .unwrap()
            .advance(
                &path,
                SECRET.as_bytes(),
                startup_atoms(&state),
                state.native_text.ui_budget.clone(),
                state.native_text.sources.policy(),
                boot::NativeRowBudget::capture(&state).await,
            )
            .unwrap();
        assert!(matches!(
            catcoms_storage::acquire_vault_session(&path),
            Err(catcoms_storage::StorageError::VaultBusy)
        ));
        original.close();
        let observed = Arc::new(AtomicBool::new(false));
        let released = Arc::new(AtomicBool::new(false));
        let waker = std::task::Waker::from(Arc::new(WakeAfterFiles {
            path: path.clone(),
            observed: observed.clone(),
            released: released.clone(),
        }));
        let mut wait = Box::pin(original.wait());
        assert!(wait
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
            .is_pending());
        if unwind {
            let task = move || {
                let _output = output;
                panic!("real boot envelope unwind");
            };
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(task)).is_err());
        } else {
            let task = move || output;
            drop(task); // actual owning closure never polled
        }
        assert!(observed.load(Ordering::SeqCst));
        assert!(
            released.load(Ordering::SeqCst),
            "End woke before the actual vault destructor"
        );
        assert!(original.is_drained_cached());
        catcoms_storage::acquire_vault_session(&path).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_sources_ui_output_and_unlock_waiting_prelude_do_not_deadlock_close() {
    use catcoms_app::store::{CriticalCatalogReadPool, NativeUiStateBudget};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let budget = NativeUiStateBudget::new(
        CriticalCatalogReadPool::new(NativeUiStateBudget::required_capacity()).unwrap(),
    )
    .unwrap();
    let credit = budget.prepare().unwrap();
    let preparation = {
        let store = state.store.lock().await;
        store
            .as_ref()
            .unwrap()
            .as_text_v2()
            .unwrap()
            .prepare_ui_state(credit)
    };
    let output = preparation.read().unwrap(); // actual completed output owns UiWork
    let close_prelude = register_vault_close(&state).await.unwrap();
    let generation = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let reply = invoke(&main, &generation, SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let unlocking = tokio::spawn(task);
    timeout(
        Duration::from_secs(60),
        state.native_text.preludes.wait_blocked_for_test(),
    )
    .await
    .unwrap();
    assert!(
        state.vault_mount.try_lock().is_err(),
        "real unlock holds mount while awaiting prelude"
    );
    let handle = app.handle().clone();
    let closing = tokio::spawn(async move {
        sources::close_server_rows(&handle.state::<AppState>(), &SystemClock).await
    });
    timeout(
        Duration::from_secs(60),
        state.native_text.source_work_waiting.notified(),
    )
    .await
    .unwrap();
    assert!(
        !closing.is_finished(),
        "the actual original UiWork keeps close Pending"
    );
    assert!(
        state.ui_session_commit.try_lock().is_ok(),
        "native work drain must not hold UI commit"
    );
    assert!(
        state.servers.try_lock().is_ok(),
        "native work drain must not hold the server table"
    );
    assert!(matches!(
        catcoms_storage::acquire_vault_session(&path),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    drop(output);
    timeout(Duration::from_secs(60), closing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    // Metadata-only auth/UI negatives still exist, but actual files are free.
    catcoms_storage::acquire_vault_session(&path).unwrap();
    assert!(
        !unlocking.is_finished(),
        "close must finish WITHOUT joining the blocked unlock task"
    );
    drop(close_prelude);
    unlocking.await.unwrap();
    refused(reply.await.unwrap());
    assert!(state.store.lock().await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_sources_close_before_unpolled_boot_never_creates_a_vault() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    let state = app.state::<AppState>();
    let reply = invoke(&main, "1", SECRET);
    let _close = register_vault_close(&state).await.unwrap();
    let (_, may_destroy) = close_vault_window_plan_inner(&state, None, false).await;
    assert!(may_destroy);
    sources::close(&state, &SystemClock).await.unwrap();
    assert!(state.store.lock().await.is_none());
    assert!(state.native_text.bootstrap.lock().unwrap().is_none());
    run(&queue).await;
    refused(reply.await.unwrap());
    assert!(
        !path.exists(),
        "old queued startup must not create files after close"
    );
    // A later lock generation and the correct secret cannot revive the
    // irreversibly closed original bootstrap/controller.
    let later = lock_session_outcome_inner(&state, None).await;
    let retry = invoke(&main, &later.generation, SECRET);
    run(&queue).await;
    refused(retry.await.unwrap());
    assert!(!path.exists());
    sources::close(&state, &SystemClock).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_sources_close_waits_returned_boot_pair_then_drops_parked_original() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    let state = app.state::<AppState>();
    let edge = AdmissionBarrier::new();
    *state.native_text.boot_return_edge.lock().unwrap() = Some(edge.clone());
    let reply = invoke(&main, "1", SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let running = tokio::spawn(task);
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    assert!(path.exists());
    assert!(state.store.lock().await.is_none());
    let _close = register_vault_close(&state).await.unwrap();
    let (_, may_destroy) = close_vault_window_plan_inner(&state, None, false).await;
    assert!(may_destroy);
    let mut closing = Box::pin(sources::close(&state, &SystemClock));
    pending_once(closing.as_mut()).await;
    assert!(state.native_text.sources.is_closing_cached());
    assert!(
        !state.native_text.sources.boot.is_drained_cached(),
        "returned pair still owns real files"
    );
    edge.release.notify_one();
    timeout(Duration::from_secs(60), closing)
        .await
        .unwrap()
        .unwrap();
    running.await.unwrap();
    refused(reply.await.unwrap());
    assert!(state.store.lock().await.is_none());
    assert!(state.native_text.bootstrap.lock().unwrap().is_none());
    assert!(state.native_text.sources.boot.is_drained_cached());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_sources_close_waits_actual_auth_capsule_and_survives_lost_close_future() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let credit = state.native_text.ui_budget.prepare().unwrap();
    let auth = {
        let store = state.store.lock().await;
        store
            .as_ref()
            .unwrap()
            .as_text_v2()
            .unwrap()
            .prepare_native_authentication(credit)
    };
    let _close = register_vault_close(&state).await.unwrap();
    let edge = AdmissionBarrier::new();
    *state.native_text.source_work_edge.lock().unwrap() = Some(edge.clone());
    let handle = app.handle().clone();
    let closing =
        tokio::spawn(
            async move { sources::close(&handle.state::<AppState>(), &SystemClock).await },
        );
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    assert!(
        state.store.lock().await.is_some(),
        "actual Auth custody outlives source zero"
    );
    closing.abort();
    assert!(closing.await.unwrap_err().is_cancelled());
    // No failed-task flag substitutes for the old active original capsule.
    let mut retry = Box::pin(sources::close(&state, &SystemClock));
    pending_once(retry.as_mut()).await;
    drop(auth);
    timeout(Duration::from_secs(60), retry)
        .await
        .unwrap()
        .unwrap();
    assert!(state.store.lock().await.is_none());
    assert!(state.native_text.sources.is_closing_cached());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_sources_replacement_between_close_stages_preserves_both_original_owners() {
    let root = tempfile::tempdir().unwrap();
    let (old, old_main, old_queue) = app(root.path().join("old"), None, true);
    let (new, new_main, new_queue) = app(root.path().join("new"), None, true);
    unlock_current(&old, &old_main, &old_queue).await;
    unlock_current(&new, &new_main, &new_queue).await;
    let state = old.state::<AppState>();
    let foreign = new.state::<AppState>();
    let _close = register_vault_close(&state).await.unwrap();
    let edge = AdmissionBarrier::new();
    *state.native_text.source_work_edge.lock().unwrap() = Some(edge.clone());
    let handle = old.handle().clone();
    let closing =
        tokio::spawn(
            async move { sources::close(&handle.state::<AppState>(), &SystemClock).await },
        );
    timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    // Trusted test replacement, not a renderer-created owner. The final
    // comparison must refuse and leave this foreign facade completely alone.
    {
        let mut first = state.store.lock().await;
        let mut second = foreign.store.lock().await;
        std::mem::swap(&mut *first, &mut *second);
    }
    edge.release.notify_one();
    assert!(closing.await.unwrap().is_err());
    let credit = foreign.native_text.ui_budget.prepare().unwrap();
    let original = {
        let store = state.store.lock().await;
        assert!(state
            .native_text
            .sources
            .capture_retirement(&store)
            .is_err());
        // The foreign facade uses its actual matching budget. The old close
        // must leave its Host and original physical work admission untouched.
        store
            .as_ref()
            .unwrap()
            .as_text_v2()
            .unwrap()
            .prepare_native_authentication(credit)
    };
    assert!(original.verify(SECRET.as_bytes()).is_ok());
    {
        let mut first = state.store.lock().await;
        let mut second = foreign.store.lock().await;
        std::mem::swap(&mut *first, &mut *second);
    }
    sources::close(&state, &SystemClock).await.unwrap();
    assert!(state.store.lock().await.is_none());
    unlock_current(&new, &new_main, &new_queue).await;
    require_unlocked_session(&foreign).await.unwrap();
}

#[test]
fn native_text_arguments_are_bounded_canonical_and_cannot_select_a_protocol() {
    let good = serde_json::json!({"passphrase":SECRET,"expectedUiGeneration":"7"});
    assert_eq!(
        arguments(&InvokeBody::Json(good.clone()), 7)
            .unwrap()
            .as_slice(),
        SECRET.as_bytes()
    );
    for bad in [
        serde_json::json!({"passphrase":"","expectedUiGeneration":"7"}),
        serde_json::json!({"passphrase":SECRET,"expectedUiGeneration":"07"}),
        serde_json::json!({"passphrase":SECRET,"expectedUiGeneration":"8"}),
        serde_json::json!({"passphrase":SECRET,"expectedUiGeneration":7}),
        serde_json::json!({"passphrase":SECRET,"expectedUiGeneration":"7","protocol":"v2"}),
        serde_json::json!({"passphrase":"x".repeat(MAX_SECRET+1),"expectedUiGeneration":"7"}),
        serde_json::json!({"passphrase":SECRET,"expectedUiGeneration":"7","trace":"x".repeat(129)}),
        serde_json::json!({"passphrase":SECRET,"expectedUiGeneration":"7","trace":{}}),
    ] {
        assert!(arguments(&InvokeBody::Json(bad), 7).is_err());
    }
    assert!(arguments(&InvokeBody::Raw(vec![0; 16]), 7).is_err());
    assert!(arguments(&InvokeBody::Json(good), 8).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_production_policy_stays_off_and_actual_acl_denies_secondary_window() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, false);
    let tauri::ipc::InvokeResponse::Err(error) = invoke(&main, "0", SECRET).await.unwrap() else {
        panic!("production route unexpectedly selected the new native backend");
    };
    assert_eq!(error.0, serde_json::json!("existing backend"));
    assert!(queue.lock().unwrap().is_empty());
    assert!(!path.exists());
    drop(app);
    let (app, _, queue) = self::app(path.clone(), None, true);
    let other = tauri::WebviewWindowBuilder::new(&app, "security-approval", Default::default())
        .build()
        .unwrap();
    let response = tauri::test::get_ipc_response(&other, request(&other, "0", SECRET));
    assert!(
        response.is_err(),
        "actual Tauri command ACL must reject before scheduling"
    );
    assert!(queue.lock().unwrap().is_empty());
    assert!(!path.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_unpolled_invoke_cannot_adopt_a_later_lock_and_busy_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    let state = app.state::<AppState>();
    let first = lock_session_outcome_inner(&state, None).await;
    refused(invoke(&main, &first.generation, "").await.unwrap());
    assert!(queue.lock().unwrap().is_empty());
    assert!(!path.exists());
    assert!(matches!(
        state.native_text.bootstrap.lock().unwrap().as_ref(),
        Some(boot::Bootstrap::Fresh)
    ));
    let old = invoke(&main, &first.generation, SECRET);
    assert_eq!(queue.lock().unwrap().len(), 1);
    refused(invoke(&main, &first.generation, SECRET).await.unwrap());
    let second = lock_session_outcome_inner(&state, None).await;
    run(&queue).await;
    refused(old.await.unwrap());
    assert!(
        !path.exists(),
        "superseded work must not even start the fresh vault"
    );
    assert_locked(&state).await;
    let current = invoke(&main, &second.generation, SECRET);
    run(&queue).await;
    success(current.await.unwrap());
    require_unlocked_session(&state).await.unwrap();
    let bytes = index(&path);
    assert_eq!(
        state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .as_text_v2()
            .unwrap()
            .row_count_data()
            .unwrap(),
        0
    );
    let locked = lock_session_outcome_inner(&state, None).await;
    let wrong = invoke(&main, &locked.generation, "wrong secret");
    run(&queue).await;
    refused(wrong.await.unwrap());
    assert_locked(&state).await;
    assert_eq!(index(&path), bytes);
    let correct = invoke(&main, &locked.generation, SECRET);
    run(&queue).await;
    success(correct.await.unwrap());
    require_unlocked_session(&state).await.unwrap();
    assert_eq!(index(&path), bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_lock_wins_after_real_mount_but_retry_reuses_that_same_owner() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let edge = AdmissionBarrier::new();
    let (app, main, queue) = app(path.clone(), Some(edge.clone()), true);
    let state = app.state::<AppState>();
    let first = lock_session_outcome_inner(&state, None).await;
    let old = invoke(&main, &first.generation, SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let running = tokio::spawn(task);
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let before = index(&path);
    let locked = lock_session_outcome_inner(&state, None).await;
    edge.release.notify_one();
    running.await.unwrap();
    refused(old.await.unwrap());
    assert_locked(&state).await;
    assert_eq!(index(&path), before);
    assert!(matches!(
        state.native_text.bootstrap.lock().unwrap().as_ref(),
        Some(boot::Bootstrap::Mounted)
    ));
    // The retained correct-key mount must NOT authenticate this new secret.
    let wrong = invoke(&main, &locked.generation, "wrong retry secret");
    run(&queue).await;
    refused(wrong.await.unwrap());
    assert_locked(&state).await;
    let current = invoke(&main, &locked.generation, SECRET);
    run(&queue).await;
    success(current.await.unwrap());
    require_unlocked_session(&state).await.unwrap();
    assert_eq!(index(&path), before);
    // The reverse ordering is ordinary successful publication followed by
    // actual lock completion; it must close the just-authorized generation.
    lock_session_outcome_inner(&state, None).await;
    assert_locked(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_original_host_close_at_final_admission_never_reports_unlocked() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let edge = AdmissionBarrier::new();
    let (app, main, queue) = app(path.clone(), Some(edge.clone()), true);
    let state = app.state::<AppState>();
    let locked = lock_session_outcome_inner(&state, None).await;
    let reply = invoke(&main, &locked.generation, SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let running = tokio::spawn(task);
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let before = index(&path);
    {
        let store = state.store.lock().await;
        store
            .as_ref()
            .unwrap()
            .as_text_v2()
            .unwrap()
            .close_admission();
    }
    edge.release.notify_one();
    running.await.unwrap();
    refused(reply.await.unwrap());
    assert_locked(&state).await;
    let retry = invoke(&main, &locked.generation, SECRET);
    run(&queue).await;
    refused(retry.await.unwrap());
    assert_locked(&state).await;
    assert_eq!(index(&path), before);
    assert!(matches!(
        state.store.lock().await.as_ref(),
        Some(MountedVault::TextV2(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_lost_reply_keeps_mount_and_foreign_wrapper_cannot_unlock_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    let state = app.state::<AppState>();
    let first = lock_session_outcome_inner(&state, None).await;
    drop(invoke(&main, &first.generation, SECRET));
    run(&queue).await; // caller disappeared, actual owned native work still completes
    require_unlocked_session(&state).await.unwrap();
    let before = index(&path);
    let wrapper = std::fs::read(path.join("vault.bin")).unwrap();
    let locked = lock_session_outcome_inner(&state, None).await;
    let other_path = root.path().join("other");
    let other = ServerStore::open(&other_path, b"foreign", &mut OsCryptoRng).unwrap();
    drop(
        other
            .into_critical_namespace_store(b"foreign", &mut OsCryptoRng)
            .unwrap(),
    );
    std::fs::copy(other_path.join("vault.bin"), path.join("vault.bin")).unwrap();
    catcoms_storage::verify_vault_passphrase(&path, b"foreign").unwrap();
    let foreign = invoke(&main, &locked.generation, "foreign");
    run(&queue).await;
    refused(foreign.await.unwrap());
    assert_locked(&state).await;
    assert_eq!(index(&path), before);
    std::fs::write(path.join("vault.bin"), wrapper).unwrap();
    let correct = invoke(&main, &locked.generation, SECRET);
    run(&queue).await;
    success(correct.await.unwrap());
    require_unlocked_session(&state).await.unwrap();
    assert_eq!(index(&path), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_connected_join_existing_directory_is_preserved_and_never_tried_as_another_format(
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("unknown"), b"preserve exactly").unwrap();
    let (app, main, queue) = app(path.clone(), None, true);
    let state = app.state::<AppState>();
    let locked = lock_session_outcome_inner(&state, None).await;
    for _ in 0..2 {
        let reply = invoke(&main, &locked.generation, SECRET);
        run(&queue).await;
        refused(reply.await.unwrap());
        assert_locked(&state).await;
        assert!(state.store.lock().await.is_none());
        assert!(!path.join("vault.bin").exists());
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
        assert_eq!(
            std::fs::read(path.join("unknown")).unwrap(),
            b"preserve exactly"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_first_unlock_without_prior_lock_has_a_real_usable_generation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    let state = app.state::<AppState>();
    assert_eq!(state.ui_session_generation.load(Ordering::SeqCst), 1);
    assert_eq!(state.ui_authorized_generation.load(Ordering::SeqCst), 0);
    assert_locked(&state).await;
    refused(invoke(&main, "0", SECRET).await.unwrap());
    assert!(queue.lock().unwrap().is_empty());
    assert!(!path.exists());
    let reply = invoke(&main, "1", SECRET);
    run(&queue).await;
    success(reply.await.unwrap());
    require_unlocked_session(&state).await.unwrap();
    let credit = state.native_text.ui_budget.prepare().unwrap();
    let preparation = {
        let store = state.store.lock().await;
        store
            .as_ref()
            .unwrap()
            .as_text_v2()
            .unwrap()
            .capture_ui(1, credit)
    };
    let mut candidate = preparation.finish().unwrap();
    let observation = {
        let store = state.store.lock().await;
        let vault = store.as_ref().unwrap().as_text_v2().unwrap();
        let observation = vault.admit_ui(&mut candidate).unwrap();
        vault.require_ui(&observation).unwrap();
        assert!(vault.ui_is_current_cached(&observation));
        observation
    };
    lock_session_outcome_inner(&state, None).await;
    assert_locked(&state).await;
    {
        let store = state.store.lock().await;
        let vault = store.as_ref().unwrap().as_text_v2().unwrap();
        assert!(!vault.ui_is_current_cached(&observation));
    }
}

// These lifecycle tests use the real native router and a real mounted selected3
// owner. Hooks pause scheduling only; no fixture boolean grants key authority.
async fn capture_current_ui(state: &AppState) -> catcoms_app::store::NativeTextUiObservation {
    let credit = state.native_text.ui_budget.prepare().unwrap();
    let preparation = {
        let store = state.store.lock().await;
        store
            .as_ref()
            .unwrap()
            .as_text_v2()
            .unwrap()
            .capture_ui(state.ui_session_generation.load(Ordering::SeqCst), credit)
    };
    let mut candidate = preparation.finish().unwrap();
    let capture = {
        let store = state.store.lock().await;
        let vault = store.as_ref().unwrap().as_text_v2().unwrap();
        let capture = vault.admit_ui(&mut candidate).unwrap();
        vault.require_ui(&capture).unwrap();
        capture
    };
    drop(candidate);
    capture
}
async fn current_ui_is_live(
    state: &AppState,
    capture: &catcoms_app::store::NativeTextUiObservation,
) -> bool {
    let store = state.store.lock().await;
    store
        .as_ref()
        .unwrap()
        .as_text_v2()
        .unwrap()
        .require_ui(capture)
        .is_ok()
}
async fn unlock_current(
    app: &tauri::App<MockRuntime>,
    main: &tauri::WebviewWindow<MockRuntime>,
    queue: &Queue,
) {
    let generation = app
        .state::<AppState>()
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let reply = invoke(main, &generation, SECRET);
    run(queue).await;
    success(reply.await.unwrap());
}
async fn pending_once<F: Future + ?Sized>(mut future: std::pin::Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(
            future.as_mut().poll(cx).is_pending(),
            "expected an actual pending barrier"
        );
        std::task::Poll::Ready(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_retirement_collects_more_than_the_generation_cap_without_reviving_old_pins() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let mut old = Vec::new();
    for _ in 0..40 {
        let capture = capture_current_ui(&state).await;
        assert!(current_ui_is_live(&state, &capture).await);
        let outcome = lock_session_outcome_inner(&state, None).await;
        assert!(outcome.continuity_error.is_none());
        assert!(!current_ui_is_live(&state, &capture).await);
        old.push(capture);
        unlock_current(&app, &main, &queue).await;
    }
    let fresh = capture_current_ui(&state).await;
    assert!(current_ui_is_live(&state, &fresh).await);
    for capture in &old {
        assert!(!current_ui_is_live(&state, capture).await);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_retirement_coalesces_and_survives_a_lost_lock_waiter() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let original = capture_current_ui(&state).await;
    let retirement: Queue = Arc::new(StdMutex::new(Vec::new()));
    *state.native_text.retirement_queue.lock().unwrap() = Some(retirement.clone());
    let handle = app.handle().clone();
    let first =
        tokio::spawn(
            async move { lock_session_outcome_inner(&handle.state::<AppState>(), None).await },
        );
    tokio::time::timeout(
        Duration::from_secs(60),
        state.native_text.retirement_enqueued.notified(),
    )
    .await
    .unwrap();
    assert_eq!(retirement.lock().unwrap().len(), 1);
    let mut second = Box::pin(lock_session_outcome_inner(&state, None));
    pending_once(second.as_mut()).await;
    assert_eq!(
        retirement.lock().unwrap().len(),
        1,
        "both locks must use the same original batch"
    );
    assert!(!current_ui_is_live(&state, &original).await);
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    run(&retirement).await;
    assert!(second.await.continuity_error.is_none());
    assert!(retirement.lock().unwrap().is_empty());
    unlock_current(&app, &main, &queue).await;
    assert!(current_ui_is_live(&state, &capture_current_ui(&state).await).await);
    assert!(!current_ui_is_live(&state, &original).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_retirement_unpolled_task_drop_recovers_from_its_original_closed_counters() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let original = capture_current_ui(&state).await;
    let retirement: Queue = Arc::new(StdMutex::new(Vec::new()));
    *state.native_text.retirement_queue.lock().unwrap() = Some(retirement.clone());
    let handle = app.handle().clone();
    let locking =
        tokio::spawn(
            async move { lock_session_outcome_inner(&handle.state::<AppState>(), None).await },
        );
    tokio::time::timeout(
        Duration::from_secs(60),
        state.native_text.retirement_enqueued.notified(),
    )
    .await
    .unwrap();
    let unpolled = retirement.lock().unwrap().pop().unwrap();
    drop(unpolled); // Outside the queue lock: actual batch destruction closes its owners.
    assert!(locking.await.unwrap().continuity_error.is_none());
    unlock_current(&app, &main, &queue).await;
    assert!(current_ui_is_live(&state, &capture_current_ui(&state).await).await);
    assert!(!current_ui_is_live(&state, &original).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_retirement_publication_gap_panic_does_not_strand_the_next_real_unlock() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let original = capture_current_ui(&state).await;
    state
        .native_text
        .panic_retirement_publication
        .store(true, Ordering::SeqCst);
    let handle = app.handle().clone();
    let locking =
        tokio::spawn(
            async move { lock_session_outcome_inner(&handle.state::<AppState>(), None).await },
        );
    assert!(locking.await.unwrap_err().is_panic());
    assert_locked(&state).await;
    unlock_current(&app, &main, &queue).await;
    assert!(current_ui_is_live(&state, &capture_current_ui(&state).await).await);
    assert!(!current_ui_is_live(&state, &original).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_unlock_cannot_overtake_an_older_locks_unfinished_prelude() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let original = capture_current_ui(&state).await;
    let edge = AdmissionBarrier::new();
    *state.native_text.lock_prelude_edge.lock().unwrap() = Some(edge.clone());
    let handle = app.handle().clone();
    let locking =
        tokio::spawn(
            async move { lock_session_outcome_inner(&handle.state::<AppState>(), None).await },
        );
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let generation = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let mut reply = invoke(&main, &generation, SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let running = tokio::spawn(task);
    // This signal requires the actual native unlock to observe this exact
    // active prelude. Pending KDF or unrelated IO cannot satisfy the test.
    tokio::time::timeout(
        Duration::from_secs(60),
        state.native_text.preludes.wait_blocked_for_test(),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(require_unlocked_session(&state).await.is_err());
    edge.release.notify_one();
    assert!(locking.await.unwrap().continuity_error.is_none());
    running.await.unwrap();
    success(reply.await.unwrap());
    require_unlocked_session(&state).await.unwrap();
    assert!(*state.session_resumable.lock().await);
    assert!(!current_ui_is_live(&state, &original).await);
    assert!(current_ui_is_live(&state, &capture_current_ui(&state).await).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_late_lock_cleanup_cannot_close_a_correct_same_generation_unlock() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let original = capture_current_ui(&state).await;
    let edge = AdmissionBarrier::new();
    *state.native_text.lock_commit_edge.lock().unwrap() = Some(edge.clone());
    let handle = app.handle().clone();
    let locking =
        tokio::spawn(
            async move { lock_session_outcome_inner(&handle.state::<AppState>(), None).await },
        );
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    unlock_current(&app, &main, &queue).await;
    let replacement = capture_current_ui(&state).await;
    // Actual post-auth plaintext cache: deleting the late-cleanup exception
    // must fail even though G-1 separately preserves the Root UI owner.
    state.media_cache.lock().await.push(MediaChunk {
        server: 17,
        cid: "new-session".into(),
        manifest_version: [9; 32],
        index: 0,
        bytes: Arc::new(vec![1, 2, 3]),
    });
    edge.release.notify_one();
    assert!(locking.await.unwrap().continuity_error.is_none());
    require_unlocked_session(&state).await.unwrap();
    assert!(*state.session_resumable.lock().await);
    assert!(current_ui_is_live(&state, &replacement).await);
    assert!(!current_ui_is_live(&state, &original).await);
    assert_eq!(
        state.media_cache.lock().await.len(),
        1,
        "old lock cleared newly authenticated media"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_close_debt_acknowledgement_still_fences_the_current_generation() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let original = capture_current_ui(&state).await;
    *state.vault_window_close_debt.lock().await = Some("prior snapshot failed".into());
    let fence = register_vault_close(&state).await.unwrap();
    let (outcome, may_destroy) = close_vault_window_plan_inner(&state, None, true).await;
    assert!(may_destroy);
    assert!(outcome.continuity_error.is_some());
    retirement::older_than(&state, state.ui_session_generation.load(Ordering::SeqCst))
        .await
        .unwrap();
    assert!(!current_ui_is_live(&state, &original).await);
    let generation = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let mut reply = invoke(&main, &generation, SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let running = tokio::spawn(task);
    // This signal requires the actual native unlock to observe this exact
    // active prelude. Pending KDF or unrelated IO cannot satisfy the test.
    tokio::time::timeout(
        Duration::from_secs(60),
        state.native_text.preludes.wait_blocked_for_test(),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(require_unlocked_session(&state).await.is_err());
    drop(fence); // Simulate a deferred close; production retains this through destruction.
    running.await.unwrap();
    success(reply.await.unwrap());
    require_unlocked_session(&state).await.unwrap();
    assert!(!current_ui_is_live(&state, &original).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_close_cannot_discard_real_key_retirement_failure_as_snapshot_loss() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let original = capture_current_ui(&state).await;
    state.native_text.retirement.poison_for_test();
    let (outcome, may_destroy) = close_vault_window_plan_inner(&state, None, true).await;
    assert!(outcome.key_work_error.is_some());
    assert!(outcome.continuity_error.is_none());
    assert!(
        !may_destroy,
        "snapshot-loss acknowledgement bypassed failed key work"
    );
    assert!(
        state.vault_window_close_debt.lock().await.is_none(),
        "key failure became discardable continuity debt"
    );
    assert!(!current_ui_is_live(&state, &original).await);
    assert_locked(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_unlock_rechecks_prelude_after_wait_idle_before_committing() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    lock_session_outcome_inner(&state, None).await;
    // Consume the initial successful finalizer's observation, then force THIS
    // actual authenticated finalizer to wait on the real UI commit mutex.
    state.native_text.unlock_commit_wait.notified().await;
    let held = state.ui_session_commit.lock().await;
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let reply = invoke(&main, &generation.to_string(), SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let running = tokio::spawn(task);
    tokio::time::timeout(
        Duration::from_secs(60),
        state.native_text.unlock_commit_wait.notified(),
    )
    .await
    .unwrap();
    // Models a lock thread descheduled after prelude registration but before
    // its SC increment. A generation-only recheck cannot detect this boundary.
    let prelude = state.native_text.preludes.begin().unwrap();
    drop(held);
    running.await.unwrap();
    refused(reply.await.unwrap());
    assert_eq!(
        state.ui_session_generation.load(Ordering::SeqCst),
        generation
    );
    assert_locked(&state).await;
    drop(prelude);
    unlock_current(&app, &main, &queue).await;
    require_unlocked_session(&state).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_pre_auth_ticket_cannot_adopt_an_intervening_voice_session() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    lock_session_outcome_inner(&state, None).await;
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let before = index(&path);
    let edge = AdmissionBarrier::new();
    *state.native_text.ticket_edge.lock().unwrap() = Some(edge.clone());
    let reply = invoke(&main, &generation.to_string(), SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let running = tokio::spawn(task);
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    // This is the real Coordinator/Runtime replacement, with NO UI generation
    // change. A ticket sampled after KDF would incorrectly adopt this session.
    state.voice_authority.lock().unwrap().revoke_all();
    assert_eq!(
        state.ui_session_generation.load(Ordering::SeqCst),
        generation
    );
    edge.release.notify_one();
    running.await.unwrap();
    refused(reply.await.unwrap());
    assert_locked(&state).await;
    assert_eq!(index(&path), before);
    unlock_current(&app, &main, &queue).await;
    require_unlocked_session(&state).await.unwrap();
}

async fn unexpected_legacy_cold(
    _: u64,
    _: SessionTicket,
    _: Operation,
    _: String,
) -> Result<Vec<ReloadedServer>, AppError> {
    panic!("real mounted Legacy authentication must never reopen the vault")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_text_shared_legacy_authentication_waits_before_its_pre_auth_voice_ticket() {
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::default());
    let store = ServerStore::open(root.path(), b"correct horse", &mut OsCryptoRng).unwrap();
    *state.store.lock().await = Some(store.into());
    let edge = AdmissionBarrier::new();
    *state.native_text.lock_prelude_edge.lock().unwrap() = Some(edge.clone());
    let locking_state = state.clone();
    let locking =
        tokio::spawn(async move { lock_session_outcome_inner(&locking_state, None).await });
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let generation = state.ui_session_generation.load(Ordering::SeqCst);
    let unlock_state = state.clone();
    let unlocking = tokio::spawn(async move {
        unlock_authentication(
            &unlock_state,
            "correct horse".into(),
            generation.to_string(),
            None,
            unexpected_legacy_cold,
        )
        .await
    });
    tokio::time::timeout(
        Duration::from_secs(60),
        state.native_text.preludes.wait_blocked_for_test(),
    )
    .await
    .unwrap();
    assert!(require_unlocked_session(&state).await.is_err());
    edge.release.notify_one();
    assert!(locking.await.unwrap().continuity_error.is_none());
    assert!(unlocking.await.unwrap().unwrap().is_empty());
    require_unlocked_session(&state).await.unwrap();
    assert!(*state.session_resumable.lock().await);
    assert!(matches!(
        state.store.lock().await.as_ref(),
        Some(MountedVault::Legacy(_))
    ));
}

fn ui_request(
    window: &tauri::WebviewWindow<MockRuntime>,
    command: &str,
    json: Option<&str>,
) -> Reply {
    let (tx, rx) = oneshot::channel();
    let mut request = self::request(window, "0", SECRET);
    request.cmd = command.into();
    request.body = InvokeBody::Json(match json {
        None => serde_json::json!({}),
        Some(json) => serde_json::json!({"json":json}),
    });
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        request,
        Box::new(move |_, _, response, _, _| {
            let _ = tx.send(response);
        }),
    );
    rx
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_state_real_router_roundtrip_and_old_unpolled_request_refusal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    refused(ui_request(&main, "get_ui_state", None).await.unwrap());
    unlock_current(&app, &main, &queue).await;
    let first = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    let tauri::ipc::InvokeResponse::Ok(body) = first.await.unwrap() else {
        panic!("first-run UI read failed");
    };
    assert_eq!(body.deserialize::<String>().unwrap(), ui_state::EMPTY);
    let json = r#"{"version":3,"drafts":{"room":"kept draft"},"readMarks":{}}"#;
    let saved = ui_request(&main, "save_ui_state", Some(json));
    run(&queue).await;
    assert!(matches!(
        saved.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
    let raw = std::fs::read(path.join("ui-state.bin")).unwrap();
    assert!(!raw.windows(b"kept draft".len()).any(|w| w == b"kept draft"));
    let read = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    let tauri::ipc::InvokeResponse::Ok(body) = read.await.unwrap() else {
        panic!("saved UI read failed");
    };
    assert_eq!(body.deserialize::<String>().unwrap(), json);
    let stale = ui_request(
        &main,
        "save_ui_state",
        Some(r#"{"version":3,"drafts":{},"readMarks":{}}"#),
    );
    lock_session_outcome_inner(&app.state::<AppState>(), None).await;
    run(&queue).await;
    refused(stale.await.unwrap());
    assert_eq!(std::fs::read(path.join("ui-state.bin")).unwrap(), raw);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_state_present_empty_is_refused_and_preserved_by_actual_router() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    let budget = catcoms_app::store::NativeUiStateBudget::new(
        catcoms_app::store::CriticalCatalogReadPool::new(
            catcoms_app::store::NativeUiStateBudget::required_capacity(),
        )
        .unwrap(),
    )
    .unwrap();
    let credit = budget.prepare().unwrap();
    let prepared = {
        let state = app.state::<AppState>();
        let store = state.store.lock().await;
        store
            .as_ref()
            .unwrap()
            .as_text_v2()
            .unwrap()
            .prepare_ui_state(credit)
    };
    let written =
        tauri::async_runtime::spawn_blocking(move || prepared.save(b"", &mut OsCryptoRng))
            .await
            .unwrap()
            .unwrap();
    drop(written);
    let before = std::fs::read(path.join("ui-state.bin")).unwrap();
    let reply = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    refused(reply.await.unwrap());
    assert_eq!(std::fs::read(path.join("ui-state.bin")).unwrap(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_state_actual_responder_panic_releases_commit_before_paid_output() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let outside = Arc::new(AtomicBool::new(false));
    *state.native_text.ui_state.output_drop.lock().unwrap() = Some(outside.clone());
    let mut request = self::request(&main, "0", SECRET);
    request.cmd = "get_ui_state".into();
    request.body = InvokeBody::Json(serde_json::json!({}));
    let view: &tauri::Webview<MockRuntime> = main.as_ref();
    view.clone().on_message(
        request,
        Box::new(|_, _, _, _, _| {
            panic!("fixture panic in actual UI-state responder");
        }),
    );
    let task = queue.lock().unwrap().pop().unwrap();
    assert!(tokio::spawn(task).await.unwrap_err().is_panic());
    assert!(
        outside.load(Ordering::SeqCst),
        "Root output/slot/payment dropped under UI commit"
    );
    assert!(state.native_text.ui_state.lane.try_lock().is_ok());
    let next = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    assert!(
        matches!(next.await.unwrap(), tauri::ipc::InvokeResponse::Ok(_)),
        "actual Root UI slot must be reusable"
    );
}

/// Scheduling-only pause inside the actual blocking IO owner. A scoped release
/// always wakes it on a failed assertion/unwind; no test can strand the lane.
pub(crate) struct BlockingIoBarrier {
    arrived: Notify,
    released: StdMutex<bool>,
    changed: std::sync::Condvar,
}
struct ReleaseBlockingIo(Arc<BlockingIoBarrier>);
impl Drop for ReleaseBlockingIo {
    fn drop(&mut self) {
        *self
            .0
            .released
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
        self.0.changed.notify_all();
    }
}
impl BlockingIoBarrier {
    fn new() -> (Arc<Self>, ReleaseBlockingIo) {
        let value = Arc::new(Self {
            arrived: Notify::new(),
            released: StdMutex::new(false),
            changed: std::sync::Condvar::new(),
        });
        (value.clone(), ReleaseBlockingIo(value))
    }
    pub(crate) fn wait_once(&self) {
        self.arrived.notify_one();
        let (released, _) = self
            .changed
            .wait_timeout_while(
                self.released.lock().unwrap(),
                Duration::from_secs(60),
                |released| !*released,
            )
            .unwrap();
        let was_released = *released;
        drop(released); // Timeout assertion must never poison the release path.
        assert!(was_released, "blocking IO fixture was not released");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_state_completed_read_cannot_publish_after_actual_lock_registration() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let edge = AdmissionBarrier::new();
    *state.native_text.ui_state.after_io.lock().unwrap() = Some(edge.clone());
    let reply = ui_request(&main, "get_ui_state", None);
    let task = queue.lock().unwrap().pop().unwrap();
    let reading = tokio::spawn(task);
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let lock_edge = AdmissionBarrier::new();
    *state.native_text.lock_prelude_edge.lock().unwrap() = Some(lock_edge.clone());
    let handle = app.handle().clone();
    let locking =
        tokio::spawn(
            async move { lock_session_outcome_inner(&handle.state::<AppState>(), None).await },
        );
    tokio::time::timeout(Duration::from_secs(60), lock_edge.arrived.notified())
        .await
        .unwrap();
    // The plaintext has already been read and serialized. Only the actual final
    // generation/auth/request check prevents it reaching the old WebView now.
    edge.release.notify_one();
    reading.await.unwrap();
    refused(reply.await.unwrap());
    lock_edge.release.notify_one();
    assert!(locking.await.unwrap().continuity_error.is_none());
    assert_locked(&state).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_state_lost_observer_keeps_blocking_io_lane_until_actual_work_drops() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let (edge, release) = BlockingIoBarrier::new();
    *state.native_text.ui_state.before_io.lock().unwrap() = Some(edge.clone());
    let reply = ui_request(&main, "get_ui_state", None);
    let task = queue.lock().unwrap().pop().unwrap();
    let reading = tokio::spawn(task);
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    reading.abort();
    assert!(reading.await.unwrap_err().is_cancelled());
    assert!(reply.await.is_err(), "abandoned native observer replied");
    assert!(
        state.native_text.ui_state.lane.try_lock().is_err(),
        "lost observer released lane before actual IO owner"
    );
    let lock_edge = AdmissionBarrier::new();
    *state.native_text.lock_commit_edge.lock().unwrap() = Some(lock_edge.clone());
    let handle = app.handle().clone();
    let locking =
        tokio::spawn(
            async move { lock_session_outcome_inner(&handle.state::<AppState>(), None).await },
        );
    tokio::time::timeout(Duration::from_secs(60), lock_edge.arrived.notified())
        .await
        .unwrap();
    lock_edge.release.notify_one();
    // Receipt is issued only after THIS lock's real lane-acquisition future
    // returned Pending. A merely unscheduled JoinHandle cannot satisfy it.
    tokio::time::timeout(
        Duration::from_secs(60),
        state.native_text.ui_state.lock_waiting.notified(),
    )
    .await
    .unwrap();
    assert!(!locking.is_finished());
    drop(release); // always scoped: blocked work survives observer until HERE
    assert!(tokio::time::timeout(Duration::from_secs(60), locking)
        .await
        .unwrap()
        .unwrap()
        .continuity_error
        .is_none());
    assert!(state.native_text.ui_state.lane.try_lock().is_ok());
    *state.native_text.ui_state.before_io.lock().unwrap() = None;
    unlock_current(&app, &main, &queue).await;
    let next = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    assert!(matches!(
        next.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_target_reauthentication_keeps_the_same_original_canonical_pin() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let original = state.native_text.ui_state.original_target().unwrap();
    lock_session_outcome_inner(&state, None).await;
    unlock_current(&app, &main, &queue).await;
    let returned = state.native_text.ui_state.original_target().unwrap();
    assert!(
        Arc::ptr_eq(&original, &returned),
        "same-root reauth replaced canonical target"
    );
    assert!(original.is_live_cached());
    let next = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    assert!(matches!(
        next.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_target_actual_router_refuses_a_live_foreign_pin_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("first");
    let (first, main, queue) = app(path.clone(), None, true);
    let (second, other, other_queue) = app(root.path().join("second"), None, true);
    unlock_current(&first, &main, &queue).await;
    unlock_current(&second, &other, &other_queue).await;
    let json = r#"{"version":3,"drafts":{"room":"original"},"readMarks":{}}"#;
    let saved = ui_request(&main, "save_ui_state", Some(json));
    run(&queue).await;
    assert!(matches!(
        saved.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
    let before = std::fs::read(path.join("ui-state.bin")).unwrap();
    let state = first.state::<AppState>();
    let wrong = second
        .state::<AppState>()
        .native_text
        .ui_state
        .original_target()
        .unwrap();
    assert!(wrong.is_live_cached());
    let original = state.native_text.ui_state.install_target(wrong).unwrap();
    let rejected = ui_request(
        &main,
        "save_ui_state",
        Some(r#"{"version":3,"drafts":{},"readMarks":{}}"#),
    );
    run(&queue).await;
    refused(rejected.await.unwrap());
    let read = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    refused(read.await.unwrap());
    assert_eq!(std::fs::read(path.join("ui-state.bin")).unwrap(), before);
    // Genuine own-target positive control: failure was not a closed UI/pool.
    let retired = state.native_text.ui_state.install_target(original);
    drop(retired);
    let read = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    let tauri::ipc::InvokeResponse::Ok(body) = read.await.unwrap() else {
        panic!("own target refused");
    };
    assert_eq!(body.deserialize::<String>().unwrap(), json);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_target_finalizer_refuses_absent_or_foreign_store_before_authorization_mutations()
{
    for foreign in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let edge = AdmissionBarrier::new();
        let (first, main, queue) = app(root.path().join("first"), Some(edge.clone()), true);
        let (second, other, other_queue) = app(root.path().join("second"), None, true);
        unlock_current(&second, &other, &other_queue).await;
        let reply = invoke(&main, "1", SECRET);
        let task = queue.lock().unwrap().pop().unwrap();
        let running = tokio::spawn(task);
        tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
            .await
            .unwrap();
        let state = first.state::<AppState>();
        let original = state.store.lock().await.take().unwrap();
        // Retain the original affine owner open: cached original negatives must
        // stay positive so only the ACTUAL mounted-target check can refuse.
        assert!(original.as_text_v2().unwrap().row_count_data().is_ok());
        if foreign {
            let replacement = second
                .state::<AppState>()
                .store
                .lock()
                .await
                .take()
                .unwrap();
            *state.store.lock().await = Some(replacement);
        }
        edge.release.notify_one();
        running.await.unwrap();
        refused(reply.await.unwrap());
        assert_locked(&state).await;
        assert_eq!(state.ui_authorized_generation.load(Ordering::SeqCst), 0);
        assert!(state.native_text.ui_state.original_target().is_none());
        let displaced = state.store.lock().await.replace(original);
        if let Some(displaced) = displaced {
            *second.state::<AppState>().store.lock().await = Some(displaced);
        }
        // Genuine original-owner retry succeeds; missing/foreign check was not
        // a synthetic closed owner, failed secret, or an always-refusing fence.
        unlock_current(&first, &main, &queue).await;
        require_unlocked_session(&state).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_ui_target_unpolled_command_cannot_adopt_a_replacement_at_the_same_ui_epoch() {
    let root = tempfile::tempdir().unwrap();
    let path_a = root.path().join("first");
    let path_b = root.path().join("second");
    let (first, main, queue) = app(path_a.clone(), None, true);
    let (second, other, other_queue) = app(path_b.clone(), None, true);
    unlock_current(&first, &main, &queue).await;
    unlock_current(&second, &other, &other_queue).await;
    let a = r#"{"version":3,"drafts":{"room":"original A"},"readMarks":{}}"#;
    let b = r#"{"version":3,"drafts":{"room":"original B"},"readMarks":{}}"#;
    for (view, work, json) in [(&main, &queue, a), (&other, &other_queue, b)] {
        let saved = ui_request(view, "save_ui_state", Some(json));
        run(work).await;
        assert!(matches!(
            saved.await.unwrap(),
            tauri::ipc::InvokeResponse::Ok(_)
        ));
    }
    let before_a = std::fs::read(path_a.join("ui-state.bin")).unwrap();
    let before_b = std::fs::read(path_b.join("ui-state.bin")).unwrap();
    let state = first.state::<AppState>();
    let other_state = second.state::<AppState>();
    let original_target = state.native_text.ui_state.original_target().unwrap();
    let replacement_target = other_state.native_text.ui_state.original_target().unwrap();
    let old = ui_request(&main, "save_ui_state", Some(b));
    assert_eq!(
        queue.lock().unwrap().len(),
        1,
        "original task was not kept unpolled"
    );
    let replacement = other_state.store.lock().await.take().unwrap();
    let original = state.store.lock().await.replace(replacement).unwrap();
    assert!(original_target.is_for_original_vault(original.as_text_v2().unwrap()));
    assert!(replacement_target.is_live_cached());
    let retired = state
        .native_text
        .ui_state
        .install_target(replacement_target);
    drop(retired);
    for (left, right) in [
        (
            &state.ui_session_generation,
            &other_state.ui_session_generation,
        ),
        (
            &state.ui_authorized_generation,
            &other_state.ui_authorized_generation,
        ),
    ] {
        assert_eq!(left.load(Ordering::SeqCst), right.load(Ordering::SeqCst));
    }
    assert!(!state.session_lock_requested.load(Ordering::SeqCst));
    assert!(!other_state.session_lock_requested.load(Ordering::SeqCst));
    run(&queue).await; // FIRST poll: only pre-schedule original-target custody differs.
    refused(old.await.unwrap());
    assert_eq!(
        std::fs::read(path_a.join("ui-state.bin")).unwrap(),
        before_a
    );
    assert_eq!(
        std::fs::read(path_b.join("ui-state.bin")).unwrap(),
        before_b
    );
    // A NEW request succeeds on B+its installed target. Refreshing the old
    // request's target when first polled would therefore have corrupted B.
    let fresh_b = ui_request(&main, "save_ui_state", Some(a));
    run(&queue).await;
    assert!(matches!(
        fresh_b.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
    assert_ne!(
        std::fs::read(path_b.join("ui-state.bin")).unwrap(),
        before_b
    );
    let displaced = state.store.lock().await.replace(original).unwrap();
    *other_state.store.lock().await = Some(displaced);
    let retired = state.native_text.ui_state.install_target(original_target);
    drop(retired);
    let fresh_a = ui_request(&main, "get_ui_state", None);
    run(&queue).await;
    let tauri::ipc::InvokeResponse::Ok(body) = fresh_a.await.unwrap() else {
        panic!("original A refused");
    };
    assert_eq!(body.deserialize::<String>().unwrap(), a);
}

fn snapshot_request(window: &tauri::WebviewWindow<MockRuntime>, json: Option<&str>) -> Reply {
    let (tx, rx) = oneshot::channel();
    let mut value = request(window, "0", SECRET);
    value.cmd = "lock_session".into();
    value.body = InvokeBody::Json(serde_json::json!({ "uiStateJson": json }));
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        value,
        Box::new(move |_, _, reply, _, _| {
            let _ = tx.send(reply);
        }),
    );
    rx
}
async fn read_ui(window: &tauri::WebviewWindow<MockRuntime>, queue: &Queue) -> String {
    let read = ui_request(window, "get_ui_state", None);
    run(queue).await;
    let tauri::ipc::InvokeResponse::Ok(body) = read.await.unwrap() else {
        panic!("UI read refused");
    };
    body.deserialize::<String>().unwrap()
}
fn snapshot_result(reply: tauri::ipc::InvokeResponse) -> serde_json::Value {
    let tauri::ipc::InvokeResponse::Ok(body) = reply else {
        panic!("native lock bridge rejected");
    };
    body.deserialize::<serde_json::Value>().unwrap()
}
const SNAPSHOT: &str = r#"{"version":3,"drafts":{"room":"final native draft"},"readMarks":{}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_actual_lock_saves_and_locked_echo_cannot_replace_it() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let reply = snapshot_request(&main, Some(SNAPSHOT));
    run(&queue).await;
    assert!(snapshot_result(reply.await.unwrap())["continuity_error"].is_null());
    assert_locked(&app.state::<AppState>()).await;
    let echo = snapshot_request(&main, Some(r#"{"version":3,"drafts":{},"readMarks":{}}"#));
    run(&queue).await;
    assert!(snapshot_result(echo.await.unwrap())["continuity_error"].is_null());
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_storage_failure_retains_exact_bytes_and_unlock_retries() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    // Real atomic file publication refuses this directory, not a fake IO flag.
    std::fs::create_dir(path.join("ui-state.bin")).unwrap();
    let reply = snapshot_request(&main, Some(SNAPSHOT));
    run(&queue).await;
    assert!(snapshot_result(reply.await.unwrap())["continuity_error"].is_string());
    assert_locked(&app.state::<AppState>()).await;
    std::fs::remove_dir(path.join("ui-state.bin")).unwrap();
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_real_unpolled_job_drop_and_postwrite_panic_keep_retry_custody() {
    for after_write in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (app, main, queue) = app(root.path().join("vault"), None, true);
        unlock_current(&app, &main, &queue).await;
        let state = app.state::<AppState>();
        if after_write {
            state
                .native_text
                .ui_state
                .snapshots
                .panic_after_write
                .store(true, Ordering::SeqCst);
        } else {
            state
                .native_text
                .ui_state
                .snapshots
                .drop_job
                .store(true, Ordering::SeqCst);
        }
        let reply = snapshot_request(&main, Some(SNAPSHOT));
        run(&queue).await;
        assert!(snapshot_result(reply.await.unwrap())["continuity_error"].is_string());
        assert_locked(&state).await;
        unlock_current(&app, &main, &queue).await;
        assert_eq!(read_ui(&main, &queue).await, SNAPSHOT);
    }
}

const NEWER_SNAPSHOT: &str =
    r#"{"version":3,"drafts":{"room":"newer native draft"},"readMarks":{}}"#;

fn close_request(
    window: &tauri::WebviewWindow<MockRuntime>,
    json: Option<&str>,
    token: Option<&str>,
) -> Reply {
    let (tx, rx) = oneshot::channel();
    let mut request = self::request(window, "0", SECRET);
    request.cmd = "close_vault_window".into();
    request.body = InvokeBody::Json(serde_json::json!({ "uiStateJson": json,
        "discardContinuityError": token.is_some(), "discardContinuityRevision": token }));
    let view: &tauri::Webview<MockRuntime> = window.as_ref();
    view.clone().on_message(
        request,
        Box::new(move |_, _, response, _, _| {
            let _ = tx.send(response);
        }),
    );
    rx
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_newer_unpolled_ingress_is_saved_by_older_lock() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let older = snapshot_request(&main, Some(SNAPSHOT));
    let newer = snapshot_request(&main, Some(NEWER_SNAPSHOT));
    // Neither task has been polled. Its DATA must already be visible; a
    // scheduled future is neither custody nor an ordering guarantee.
    let older_task = queue.lock().unwrap().remove(0);
    older_task.await;
    assert!(snapshot_result(older.await.unwrap())["continuity_error"].is_null());
    drop(queue.lock().unwrap().pop());
    assert!(newer.await.is_err());
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, NEWER_SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_ordinary_actual_save_fences_a_slower_ingress_copy() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let (edge, release) = BlockingIoBarrier::new();
    *state
        .native_text
        .ui_state
        .snapshots
        .before_publish
        .lock()
        .unwrap() = Some(edge.clone());
    let window = main.clone();
    let receiving = tokio::task::spawn_blocking(move || snapshot_request(&window, Some(SNAPSHOT)));
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    // No snapshot is published yet. A watermark that exists only while pending
    // would miss this case and allow the older final copy to overwrite disk.
    let saved = ui_request(&main, "save_ui_state", Some(NEWER_SNAPSHOT));
    run(&queue).await;
    assert!(matches!(
        saved.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
    drop(release);
    let older = receiving.await.unwrap();
    run(&queue).await;
    assert!(snapshot_result(older.await.unwrap())["continuity_error"].is_null());
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, NEWER_SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_old_unpolled_close_cannot_close_a_reauthenticated_session() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let old = close_request(&main, Some(SNAPSHOT), None);
    let state = app.state::<AppState>();
    assert!(lock_session_outcome_inner(&state, None)
        .await
        .continuity_error
        .is_none());
    unlock_current(&app, &main, &queue).await;
    let saved = ui_request(&main, "save_ui_state", Some(NEWER_SNAPSHOT));
    run(&queue).await;
    assert!(matches!(
        saved.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
    run(&queue).await; // first poll of the old authenticated close
    let outcome = snapshot_result(old.await.unwrap());
    assert_eq!(outcome["deferred"], true);
    assert!(outcome["continuity_error"]
        .as_str()
        .unwrap()
        .contains("older close"));
    assert!(app.get_webview_window("main").is_some());
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, NEWER_SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_two_concurrent_closes_do_not_wait_on_each_others_long_fences() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    std::fs::create_dir(path.join("ui-state.bin")).unwrap();
    let a = close_request(&main, Some(SNAPSHOT), None);
    let b = close_request(&main, Some(NEWER_SNAPSHOT), None);
    let mut tasks = queue.lock().unwrap().drain(..).collect::<Vec<_>>();
    // Both real close preludes must coexist BEFORE either checks other finite
    // producers. Merely spawning two tasks would not prove this interleaving.
    let state = app.state::<AppState>();
    let guard = register_vault_close(&state).await.unwrap();
    let other = register_vault_close(&state).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            guard.wait_for_other_mutations(),
            other.wait_for_other_mutations()
        );
    })
    .await
    .expect("long close fences deadlocked");
    drop((guard, other));
    tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(tasks.pop().unwrap(), tasks.pop().unwrap());
    })
    .await
    .unwrap();
    for result in [a.await.unwrap(), b.await.unwrap()] {
        let outcome = snapshot_result(result);
        assert_eq!(outcome["deferred"], true);
        assert!(outcome["continuity_error"].is_string());
    }
    assert!(app.get_webview_window("main").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_actual_close_requires_the_exact_displayed_loss_revision() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    std::fs::create_dir(path.join("ui-state.bin")).unwrap();
    let first = close_request(&main, Some(SNAPSHOT), None);
    run(&queue).await;
    let first = snapshot_result(first.await.unwrap());
    assert_eq!(first["deferred"], true);
    assert_eq!(first["continuity_discardable"], true);
    let token = first["continuity_revision"].as_str().unwrap();
    // Wrong acknowledgements must retry IO, keep the window and issue a new
    // revision. That new failure must not be discardable by the original token.
    let wrong = close_request(&main, None, Some("0"));
    run(&queue).await;
    let wrong = snapshot_result(wrong.await.unwrap());
    assert_eq!(wrong["deferred"], true);
    assert_ne!(wrong["continuity_revision"], first["continuity_revision"]);
    let stale = close_request(&main, None, Some(token));
    run(&queue).await;
    let stale = snapshot_result(stale.await.unwrap());
    assert_eq!(stale["deferred"], true);
    assert!(app.get_webview_window("main").is_some());
    let exact = close_request(&main, None, stale["continuity_revision"].as_str());
    run(&queue).await;
    assert_eq!(snapshot_result(exact.await.unwrap())["deferred"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_overloaded_close_resolves_without_destroy_and_does_not_mask_disk_failure()
{
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let (app, main, queue) = app(path.clone(), None, true);
    unlock_current(&app, &main, &queue).await;
    std::fs::create_dir(path.join("ui-state.bin")).unwrap();
    let failing = snapshot_request(&main, Some(SNAPSHOT));
    run(&queue).await;
    assert!(snapshot_result(failing.await.unwrap())["continuity_error"].is_string());
    let held = (0..8)
        .map(|_| snapshot_request(&main, None))
        .collect::<Vec<_>>();
    let overflow = close_request(&main, None, None);
    assert_eq!(queue.lock().unwrap().len(), 8);
    let result = snapshot_result(overflow.await.unwrap());
    assert_eq!(result["deferred"], true);
    assert_eq!(result["continuity_discardable"], false);
    assert!(result["history_error"].is_string());
    assert!(app.get_webview_window("main").is_some());
    run(&queue).await;
    // Drainage resolves the control-only Busy, but not the retained actual IO.
    assert!(
        snapshot_result(held.into_iter().last().unwrap().await.unwrap())["continuity_error"]
            .is_string()
    );
    queue.lock().unwrap().clear();
    std::fs::remove_dir(path.join("ui-state.bin")).unwrap();
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_stale_auth_reply_cannot_consume_an_unseen_loss_warning() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let bad = snapshot_request(&main, Some(r#"{"version":999}"#));
    run(&queue).await;
    assert!(snapshot_result(bad.await.unwrap())["continuity_error"].is_string());
    let state = app.state::<AppState>();
    let edge = AdmissionBarrier::new();
    *state.native_text.reply_edge.lock().unwrap() = Some(edge.clone());
    let g = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let unseen = invoke(&main, &g, SECRET);
    let task = queue.lock().unwrap().pop().unwrap();
    let answering = tokio::spawn(task);
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    lock_session_outcome_inner(&state, None).await;
    edge.release.notify_one();
    answering.await.unwrap();
    refused(unseen.await.unwrap());
    let g = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let warning = invoke(&main, &g, SECRET);
    run(&queue).await;
    let tauri::ipc::InvokeResponse::Err(error) = warning.await.unwrap() else {
        panic!("unseen loss was acknowledged");
    };
    assert!(error.0.as_str().unwrap().contains("Unlock again"));
    assert_locked(&state).await;
    unlock_current(&app, &main, &queue).await; // only this explicit retry acknowledges it
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_slower_ordinary_copy_cannot_supersede_a_newer_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let (edge, release) = BlockingIoBarrier::new();
    *state.native_text.ui_state.before_copy.lock().unwrap() = Some(edge.clone());
    let window = main.clone();
    let receiving =
        tokio::task::spawn_blocking(move || ui_request(&window, "save_ui_state", Some(SNAPSHOT)));
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    let newer = snapshot_request(&main, Some(NEWER_SNAPSHOT));
    drop(release);
    let older = receiving.await.unwrap();
    run(&queue).await; // the earlier ordinary invocation copied/queued last
    assert!(matches!(
        older.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
    run(&queue).await;
    assert!(snapshot_result(newer.await.unwrap())["continuity_error"].is_null());
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, NEWER_SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_old_ordinary_writer_cannot_regress_a_superseding_durable_save() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let older = ui_request(&main, "save_ui_state", Some(SNAPSHOT));
    let snapshot = snapshot_request(&main, Some(SNAPSHOT));
    let newer = ui_request(&main, "save_ui_state", Some(NEWER_SNAPSHOT));
    run(&queue).await; // actual newer ordinary IO supersedes the snapshot
    assert!(matches!(
        newer.await.unwrap(),
        tauri::ipc::InvokeResponse::Ok(_)
    ));
    let old = queue.lock().unwrap().remove(0);
    old.await; // same authorized generation, but earlier ingress
    refused(older.await.unwrap());
    run(&queue).await;
    assert!(snapshot_result(snapshot.await.unwrap())["continuity_error"].is_null());
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, NEWER_SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_lost_observer_retains_actual_io_and_unlock_waits_for_its_result() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let (edge, release) = BlockingIoBarrier::new();
    *state
        .native_text
        .ui_state
        .snapshots
        .before_io
        .lock()
        .unwrap() = Some(edge.clone());
    let reply = snapshot_request(&main, Some(SNAPSHOT));
    let saving = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    tokio::time::timeout(Duration::from_secs(60), edge.arrived.notified())
        .await
        .unwrap();
    saving.abort();
    assert!(saving.await.unwrap_err().is_cancelled());
    assert!(reply.await.is_err());
    assert!(state.native_text.ui_state.lane.try_lock().is_err());
    let g = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let reopened = invoke(&main, &g, SECRET);
    let opening = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    tokio::time::timeout(
        Duration::from_secs(60),
        state.native_text.ui_state.lock_waiting.notified(),
    )
    .await
    .unwrap();
    assert!(!opening.is_finished());
    drop(release);
    tokio::time::timeout(Duration::from_secs(60), opening)
        .await
        .unwrap()
        .unwrap();
    success(reopened.await.unwrap());
    assert_eq!(read_ui(&main, &queue).await, SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_old_admitted_success_cannot_erase_a_newer_delayed_capture_loss() {
    let root = tempfile::tempdir().unwrap();
    let (app, main, queue) = app(root.path().join("vault"), None, true);
    unlock_current(&app, &main, &queue).await;
    let state = app.state::<AppState>();
    let earlier = snapshot_request(&main, Some(SNAPSHOT));
    let (copy_edge, release_copy) = BlockingIoBarrier::new();
    *state
        .native_text
        .ui_state
        .snapshots
        .before_publish
        .lock()
        .unwrap() = Some(copy_edge.clone());
    let window = main.clone();
    let copying = tokio::task::spawn_blocking(move || {
        let oversized = "x".repeat(MAX_UI_STATE_BYTES + 1);
        snapshot_request(&window, Some(&oversized))
    });
    tokio::time::timeout(Duration::from_secs(60), copy_edge.arrived.notified())
        .await
        .unwrap();
    let (io_edge, release_io) = BlockingIoBarrier::new();
    *state
        .native_text
        .ui_state
        .snapshots
        .before_io
        .lock()
        .unwrap() = Some(io_edge.clone());
    let saving = tokio::spawn(queue.lock().unwrap().pop().unwrap());
    tokio::time::timeout(Duration::from_secs(60), io_edge.arrived.notified())
        .await
        .unwrap();
    // The older real Job is admitted. Publish the newer failed capture before
    // allowing that actual disk write to succeed and record its outcome.
    drop(release_copy);
    let lost = copying.await.unwrap();
    drop(release_io);
    saving.await.unwrap();
    assert!(snapshot_result(earlier.await.unwrap())["continuity_error"]
        .as_str()
        .unwrap()
        .contains("too large"));
    drop(queue.lock().unwrap().pop());
    assert!(lost.await.is_err());
    let close = close_request(&main, None, None);
    run(&queue).await;
    let close = snapshot_result(close.await.unwrap());
    assert_eq!(close["deferred"], true);
    assert!(close["continuity_error"]
        .as_str()
        .unwrap()
        .contains("too large"));
    let g = state
        .ui_session_generation
        .load(Ordering::SeqCst)
        .to_string();
    let warning = invoke(&main, &g, SECRET);
    run(&queue).await;
    refused(warning.await.unwrap());
    assert_locked(&state).await;
    unlock_current(&app, &main, &queue).await;
    assert_eq!(read_ui(&main, &queue).await, SNAPSHOT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_snapshot_foreign_origin_loss_can_be_acknowledged_without_writing_either_vault() {
    let root = tempfile::tempdir().unwrap();
    let path_a = root.path().join("first");
    let path_b = root.path().join("second");
    let (first, main, queue) = app(path_a.clone(), None, true);
    let (second, other, other_queue) = app(path_b.clone(), None, true);
    unlock_current(&first, &main, &queue).await;
    unlock_current(&second, &other, &other_queue).await;
    for (window, requests, json) in [
        (&main, &queue, SNAPSHOT),
        (&other, &other_queue, NEWER_SNAPSHOT),
    ] {
        let saved = ui_request(window, "save_ui_state", Some(json));
        run(requests).await;
        assert!(matches!(
            saved.await.unwrap(),
            tauri::ipc::InvokeResponse::Ok(_)
        ));
    }
    let before_a = std::fs::read(path_a.join("ui-state.bin")).unwrap();
    let before_b = std::fs::read(path_b.join("ui-state.bin")).unwrap();
    let old = snapshot_request(&main, Some(NEWER_SNAPSHOT));
    let state = first.state::<AppState>();
    let other_state = second.state::<AppState>();
    let foreign = other_state.store.lock().await.take().unwrap();
    let original = state.store.lock().await.replace(foreign).unwrap();
    let original_target = state
        .native_text
        .ui_state
        .install_target(other_state.native_text.ui_state.original_target().unwrap())
        .unwrap();
    let blocked = close_request(&main, None, None);
    run(&queue).await;
    let blocked = snapshot_result(blocked.await.unwrap());
    assert_eq!(blocked["deferred"], true);
    assert_eq!(blocked["continuity_discardable"], false);
    assert!(blocked["continuity_revision"].is_null());
    assert_eq!(
        blocked["history_error"],
        "The original vault changed while closing."
    );
    drop(queue.lock().unwrap().pop());
    assert!(old.await.is_err());

    // Issue the revision through an actual IO failure on the original mount
    // after its key drain succeeds. Preserve the previously saved ciphertext.
    let foreign = state.store.lock().await.replace(original).unwrap();
    let foreign_target = state
        .native_text
        .ui_state
        .install_target(original_target.clone())
        .unwrap();
    let snapshot_path = path_a.join("ui-state.bin");
    let parked = path_a.join("saved-ui-before-close.bin");
    std::fs::rename(&snapshot_path, &parked).unwrap();
    std::fs::create_dir(&snapshot_path).unwrap();
    let close = close_request(&main, None, None);
    run(&queue).await;
    let close = snapshot_result(close.await.unwrap());
    assert_eq!(close["deferred"], true);
    assert_eq!(close["continuity_discardable"], true);
    assert!(close["continuity_error"].is_string());
    assert!(close["history_error"].is_null());
    let revision = close["continuity_revision"].as_str().unwrap();
    std::fs::remove_dir(&snapshot_path).unwrap();
    std::fs::rename(&parked, &snapshot_path).unwrap();

    let original = state.store.lock().await.replace(foreign).unwrap();
    drop(state.native_text.ui_state.install_target(foreign_target));
    let acknowledged = close_request(&main, None, Some(revision));
    run(&queue).await;
    let acknowledged = snapshot_result(acknowledged.await.unwrap());
    // The former snapshot-only close could finish here. With canonical
    // Sources, acknowledging DATA loss cannot certify a foreign owner pair.
    // Retain both original owners and leave both ciphertexts unchanged.
    assert_eq!(acknowledged["deferred"], true);
    assert_eq!(
        acknowledged["history_error"],
        "The original vault changed while closing."
    );
    assert!(
        state
            .native_text
            .ui_state
            .snapshots
            .close_decision(Some(revision))
            .acknowledged
    );
    assert!(state.native_text.sources.retains_original(&original));
    {
        let store = state.store.lock().await;
        let foreign = store.as_ref().expect("foreign facade retained");
        assert!(!state.native_text.sources.retains_original(foreign));
        assert!(other_state.native_text.sources.retains_original(foreign));
    }
    assert!(state.session_lock_requested.load(Ordering::SeqCst));
    assert_eq!(
        std::fs::read(path_a.join("ui-state.bin")).unwrap(),
        before_a
    );
    assert_eq!(
        std::fs::read(path_b.join("ui-state.bin")).unwrap(),
        before_b
    );
    let foreign = state.store.lock().await.replace(original).unwrap();
    *other_state.store.lock().await = Some(foreign);
    drop(state.native_text.ui_state.install_target(original_target));
    // The same acknowledged close can now retire the actual original pair.
    let retry = close_request(&main, None, Some(revision));
    run(&queue).await;
    let retried = snapshot_result(retry.await.unwrap());
    assert_eq!(retried["deferred"], false);
    assert!(retried["history_error"].is_null());
    assert!(state.store.lock().await.is_none());
    assert!(other_state.store.lock().await.is_some());
    assert_eq!(
        std::fs::read(path_a.join("ui-state.bin")).unwrap(),
        before_a
    );
    assert_eq!(
        std::fs::read(path_b.join("ui-state.bin")).unwrap(),
        before_b
    );
}
