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
                (
                    state.ui_session_generation.clone(),
                    state.ui_authorized_generation.clone(),
                    state.session_lock_requested.clone(),
                ),
                state.native_text.ui_budget.clone(),
                state.native_text.sources.policy(),
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
    let closing =
        tokio::spawn(
            async move { sources::close(&handle.state::<AppState>(), &SystemClock).await },
        );
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
    let close = close_request(&main, None, None);
    run(&queue).await;
    let close = snapshot_result(close.await.unwrap());
    assert_eq!(close["deferred"], true);
    assert_eq!(close["continuity_discardable"], true);
    assert!(close["continuity_error"].is_string());
    drop(queue.lock().unwrap().pop());
    assert!(old.await.is_err());
    let acknowledged = close_request(&main, None, close["continuity_revision"].as_str());
    run(&queue).await;
    let acknowledged = snapshot_result(acknowledged.await.unwrap());
    // The former snapshot-only close could finish here. With canonical
    // Sources, acknowledging DATA loss cannot certify a foreign owner pair.
    // Keep both the original Source owner and the foreign facade untouched.
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
            .close_decision(close["continuity_revision"].as_str())
            .acknowledged
    );
    assert!(state.native_text.sources.retains_original(&original));
    {
        let store = state.store.lock().await;
        let foreign = store.as_ref().expect("foreign facade retained");
        assert!(!state.native_text.sources.retains_original(foreign));
        assert!(other_state.native_text.sources.retains_original(foreign));
    }
    assert!(state.native_text.sources.is_closing_cached());
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
    let retry = close_request(&main, None, close["continuity_revision"].as_str());
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
