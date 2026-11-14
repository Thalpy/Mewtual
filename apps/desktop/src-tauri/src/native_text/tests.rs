//! Actual Tauri ACL/router, real selected3 vault and AppState finalization.
//! Scheduling barriers never replace a secret, owner or native generation.
use super::*;
use tauri::test::MockRuntime;
use tokio::sync::{oneshot, Notify};

const SECRET: &str = "native text fixture secret";
type Queue = Arc<StdMutex<Vec<Task>>>;
type Reply = oneshot::Receiver<tauri::ipc::InvokeResponse>;

pub(super) struct AdmissionBarrier {
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
    pub(super) async fn wait_once(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.arrived.notify_one();
            self.release.notified().await;
        }
    }
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
async fn run(queue: &Queue) {
    let task = queue
        .lock()
        .unwrap()
        .pop()
        .expect("actual router scheduled one task");
    task.await;
}
fn success(reply: tauri::ipc::InvokeResponse) {
    let tauri::ipc::InvokeResponse::Ok(body) = reply else {
        panic!("actual unlock refused");
    };
    assert_eq!(
        body.deserialize::<serde_json::Value>().unwrap(),
        serde_json::json!([])
    );
}
fn refused(reply: tauri::ipc::InvokeResponse) {
    assert!(matches!(reply, tauri::ipc::InvokeResponse::Err(_)));
}
async fn assert_locked(state: &AppState) {
    assert!(require_unlocked_session(state).await.is_err());
    assert!(!*state.session_resumable.lock().await);
    assert!(state.session_lock_requested.load(Ordering::SeqCst));
}
fn index(path: &Path) -> Vec<u8> {
    std::fs::read(path.join("chat-owner-v2/native-registry.bin")).unwrap()
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
async fn native_text_existing_directory_is_preserved_and_never_tried_as_another_format() {
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
