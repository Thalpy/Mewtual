//! Actual Tauri command dispatch, JSON argument decoding and reply serialization
//! over the real issuer/Actor/Root fixture. No production setup/run, native window,
//! browser SDK or audible/physical RTC acceptance is claimed. MockContext has no
//! app ACL: tests/command_acl.rs separately exercises the REAL RuntimeAuthority.
use super::*;
use catcoms_rt::{Clock, SystemClock};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;
use tauri::Manager;

const POINT: &str = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
// Deterministic public2G, computed with Node's existing prime256v1 implementation
// from fixed test scalar2, and validated by the native parser below. This is NOT
// the same bytes as POINT, so an invalid-input freeze would break the later call.
const DIFFERENT_POINT: &str = "047cf27b188d034f7e8a52380304b51ac3c08969e277f21b35a60b48fc4766997807775510db8ed040293d9ac69f7430dbba7dade63ce982299e04b79d227873d1";

async fn bounded<T>(future: impl Future<Output = T>) -> Result<T, String> {
    tokio::select! {
        result = future => Ok(result),
        _ = SystemClock.sleep(Duration::from_secs(30)) => Err("native Tauri IPC fixture exceeded its bound".into()),
    }
}

/// Keep actual task ownership through the timeout path. A timed-out task is
/// aborted and JOINED, never silently detached by dropping its JoinHandle. If
/// cancellation itself cannot return, report that failed join; do not certify
/// graceful protocol/Root retirement from an observed timeout or abort flag.
async fn join_actual(task: &mut tokio::task::JoinHandle<()>, label: &str) -> Result<(), String> {
    match bounded(&mut *task).await {
        Ok(result) => result.map_err(|error| format!("{label} returned an error: {error}")),
        Err(timeout) => {
            task.abort();
            let retirement = match bounded(&mut *task).await {
                Ok(Ok(())) => "actual task returned after cancellation".to_string(),
                Ok(Err(error)) => format!("actual task joined after cancellation: {error}"),
                Err(error) => format!("actual task did NOT join after cancellation: {error}"),
            };
            Err(format!("{label}: {timeout}; {retirement}"))
        }
    }
}

enum Reply {
    Resolved(Value),
    Rejected(Value),
}

struct IpcFixture {
    // MockRuntime's UI objects are created, invoked and destroyed only on this
    // current-thread test executor. Native command futures use Tauri's normal
    // async runtime and borrow the SAME managed AppState, not a second fixture.
    app: Option<tauri::App<tauri::test::MockRuntime>>,
    webview: Option<tauri::WebviewWindow<tauri::test::MockRuntime>>,
    actor: ServerActor,
    worker: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
    controller: String,
    clock: Arc<ManualClock>,
    // LAST: application/store/actor holders must leave before the actual vault
    // temp directory is destroyed, including a fixture-construction error.
    _root: tempfile::TempDir,
}

impl IpcFixture {
    async fn new() -> Result<Self, String> {
        let clock = Arc::new(ManualClock::new(1000));
        let Fixture {
            state,
            _root,
            actor,
            mut worker,
            mut drain,
            controller,
        } = Fixture::with_voice_clock(clock.clone()).await;
        let app = tauri::test::mock_builder()
            .manage(state)
            .invoke_handler(tauri::generate_handler![
                crate::voice_admitted_call::prepare_admitted_voice_owner,
                crate::voice_admitted_call::activate_admitted_voice_owner,
                crate::voice_admitted_call::acknowledge_admitted_voice_activation,
                crate::voice_admitted_key::admit_admitted_voice_join_key,
                crate::voice_admitted_key::publish_admitted_voice_join_key,
                crate::voice_admitted_key::acknowledge_admitted_voice_key_publication,
                crate::voice_admitted_key::settle_admitted_voice_join_key,
                crate::voice_join_key::admit_voice_join_key,
                crate::voice_join_key::publish_voice_join_key,
                crate::end_voice_owner,
                crate::lock_session,
            ])
            .build(tauri::test::mock_context(tauri::test::noop_assets()));
        let app = match app {
            Ok(app) => app,
            Err(error) => {
                // No test helper thread or visible native app exists. Still join
                // the actual fixture actor before returning construction failure.
                let stopped = bounded(actor.shutdown()).await;
                if stopped.is_err() {
                    worker.abort();
                }
                let worker = join_actual(&mut worker, "fixture actor").await;
                let drain = join_actual(&mut drain, "fixture event drain").await;
                drop(_root);
                stopped?;
                worker?;
                drain?;
                return Err(error.to_string());
            }
        };
        let mut fixture = Self {
            app: Some(app),
            webview: None,
            actor,
            worker,
            drain,
            controller,
            clock,
            _root,
        };
        let webview = tauri::WebviewWindowBuilder::new(
            fixture.app.as_ref().expect("original mock app"),
            "main",
            Default::default(),
        )
        .build();
        match webview {
            Ok(webview) => fixture.webview = Some(webview),
            Err(error) => {
                let message = error.to_string();
                let _ = fixture.close().await;
                return Err(message);
            }
        }
        Ok(fixture)
    }

    fn state(&self) -> tauri::State<'_, AppState> {
        self.app
            .as_ref()
            .expect("original mock app")
            .state::<AppState>()
    }

    async fn invoke(&self, command: &str, args: Value) -> Result<Reply, String> {
        let (send, receive) = tokio::sync::oneshot::channel();
        let webview: &tauri::Webview<tauri::test::MockRuntime> = self
            .webview
            .as_ref()
            .expect("original mock webview")
            .as_ref();
        // This is the SAME public on_message path get_ipc_response uses. Its
        // std::sync::mpsc::recv is unbounded, so await an owned async callback
        // instead: a missing reply cannot strand a test-side blocking thread.
        webview.clone().on_message(
            tauri::webview::InvokeRequest {
                cmd: command.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: if cfg!(any(windows, target_os = "android")) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .map_err(|error| format!("test origin: {error}"))?,
                body: tauri::ipc::InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
            Box::new(move |_webview, _command, response, _success, _error| {
                // Caller timeout is not a native cancellation/ACK. A late reply
                // merely returns its actual response to this already-gone waiter.
                let _ = send.send(response);
            }),
        );
        match bounded(receive)
            .await?
            .map_err(|_| "native IPC responder disappeared")?
        {
            tauri::ipc::InvokeResponse::Ok(tauri::ipc::InvokeResponseBody::Json(body)) => {
                serde_json::from_str(&body)
                    .map(Reply::Resolved)
                    .map_err(|error| error.to_string())
            }
            tauri::ipc::InvokeResponse::Ok(tauri::ipc::InvokeResponseBody::Raw(_)) => {
                Err("public key command returned raw bytes instead of JSON".into())
            }
            tauri::ipc::InvokeResponse::Err(error) => Ok(Reply::Rejected(error.0)),
        }
    }

    async fn resolved(&self, command: &str, args: Value) -> Result<Value, String> {
        match self.invoke(command, args).await? {
            Reply::Resolved(value) => Ok(value),
            Reply::Rejected(error) => Err(format!("{command} rejected: {error}")),
        }
    }

    async fn rejected(&self, command: &str, args: Value) -> Result<Value, String> {
        match self.invoke(command, args).await? {
            Reply::Rejected(value) => Ok(value),
            Reply::Resolved(value) => Err(format!("{command} unexpectedly resolved: {value}")),
        }
    }

    async fn close(mut self) -> Result<(), String> {
        // Ordinary request/assertion failures still get teardown. Revocation
        // closes original gates, never invents SDK/Work/Root-return certificates.
        self.state()
            .voice_authority
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .revoke_all();
        self.clock.advance_ms(250);
        let stopped = bounded(self.actor.shutdown()).await;
        if stopped.is_err() {
            self.worker.abort();
        }
        let worker = join_actual(&mut self.worker, "fixture actor").await;
        let drain = join_actual(&mut self.drain, "fixture event drain").await;
        // After actual Root destruction, a late original negative worker must
        // observe its real residence-return marker rather than need a new Actor.
        self.clock.advance_ms(250);
        if let Some(webview) = self.webview.take() {
            let _ = webview.close();
            drop(webview);
        }
        drop(self.app.take());
        stopped?;
        worker?;
        drain?;
        Ok(())
    }
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn string(value: &Value, field: &str) -> Result<String, String> {
    value[field]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("missing JSON string {field}: {value}"))
}

async fn admitted_scenario(f: &IpcFixture) -> Result<(), String> {
    let candidate = f
        .resolved(
            "prepare_admitted_voice_owner",
            json!({
                "controller": f.controller, "server": 7, "room": channel_id("general").to_string(),
            }),
        )
        .await?
        .as_str()
        .ok_or("candidate was not a JSON string")?
        .to_owned();
    let activated = f
        .resolved(
            "activate_admitted_voice_owner",
            json!({ "candidate": candidate }),
        )
        .await?;
    require(
        activated["owner"] == candidate,
        "activation returned another owner",
    )?;
    let join = string(&activated, "join")?;
    f.resolved(
        "acknowledge_admitted_voice_activation",
        json!({ "owner": candidate }),
    )
    .await?;
    let executor = hex::encode([9; 32]);
    // A real command-body rejection, not a missing-handler/parser false pass.
    let error = f
        .rejected(
            "admit_voice_join_key",
            json!({ "owner": candidate, "executor": executor }),
        )
        .await?;
    require(
        error.as_str() == Some("voice owner changed"),
        "legacy key admission did not reach its profile fence",
    )?;
    let error = f.rejected("publish_voice_join_key", json!({
        "owner": candidate, "executor": executor, "permit": "not-admitted", "publicKey": POINT,
    })).await?;
    require(
        error.as_str() == Some("voice owner changed"),
        "legacy publication did not reach its profile fence",
    )?;
    let admission = f
        .resolved(
            "admit_admitted_voice_join_key",
            json!({ "owner": candidate, "executor": executor }),
        )
        .await?;
    let admission_keys: BTreeSet<_> = admission
        .as_object()
        .ok_or("admission was not JSON object")?
        .keys()
        .map(String::as_str)
        .collect();
    require(
        admission_keys == BTreeSet::from(["kind", "owner", "executor", "permit", "join"]),
        "admission changed its five-field JSON shape",
    )?;
    require(
        admission["kind"] == "admitted"
            && admission["owner"] == candidate
            && admission["executor"] == executor
            && admission["join"] == join,
        "native key admission changed the original join",
    )?;
    let permit = string(&admission, "permit")?;
    require(
        !permit.is_empty() && permit.len() <= 128,
        "admission returned an invalid permit bound",
    )?;
    let different = hex::decode(DIFFERENT_POINT).map_err(|error| error.to_string())?;
    require(
        p256::PublicKey::from_sec1_bytes(&different).is_ok(),
        "different public test point is not on the actual native curve",
    )?;
    // Actual camelCase input reaches the production parser; uppercase is rejected
    // before point freeze, so the following valid request retains its original permit.
    let malformed = f.rejected("publish_admitted_voice_join_key", json!({
        "owner": candidate, "executor": executor, "permit": permit, "publicKey": DIFFERENT_POINT.to_ascii_uppercase(),
    })).await?;
    require(
        malformed.as_str() == Some("invalid voice public key"),
        "invalid public point was not rejected by the actual command",
    )?;
    let args =
        json!({ "owner": candidate, "executor": executor, "permit": permit, "publicKey": POINT });
    let first = f
        .resolved("publish_admitted_voice_join_key", args.clone())
        .await?;
    let keys: BTreeSet<_> = first
        .as_object()
        .ok_or("publication was not JSON object")?
        .keys()
        .map(String::as_str)
        .collect();
    require(
        keys == BTreeSet::from(["kind", "owner", "executor", "permit", "join", "keyRef"]),
        "publication must be a flat fixed-metadata tagged object",
    )?;
    require(
        first["kind"] == "accepted"
            && first["owner"] == candidate
            && first["executor"] == executor
            && first["permit"] == permit
            && first["join"] == join,
        "publication changed its admitted identity",
    )?;
    let point: [u8; 65] = hex::decode(POINT)
        .map_err(|error| error.to_string())?
        .try_into()
        .map_err(|_| "fixture point width")?;
    require(
        first["keyRef"] == hex::encode(catcoms_sync::voice_v2::key_reference(&point)),
        "Root publication returned another key reference",
    )?;
    // Treat the first actual reply as lost. No SDK/private-key input or fake Root
    // receipt is supplied; a second JSON invocation recovers the canonical fact.
    require(
        f.resolved("publish_admitted_voice_join_key", args.clone())
            .await?
            == first,
        "lost publication reply did not recover exactly",
    )?;
    let original = f
        .state()
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(&candidate)
        .ok_or("original activation missing")?;
    f.resolved("end_voice_owner", json!({ "owner": candidate }))
        .await?;
    f.clock.advance_ms(250);
    bounded(original.wait_execution_return()).await?;
    let locked = f
        .resolved("lock_session", json!({ "uiStateJson": null }))
        .await?;
    require(
        locked
            .as_object()
            .is_some_and(|object| object.len() == 1 && object.contains_key("continuity_error")),
        "lock outcome changed its actual JSON shape",
    )?;
    require(
        locked["continuity_error"].is_null(),
        "real fixture lock could not save continuity",
    )?;
    require(
        f.state()
            .ui_session_gate()
            .require_unlocked()
            .await
            .is_err(),
        "lock did not close the native command boundary",
    )?;
    require(
        f.resolved("publish_admitted_voice_join_key", args).await? == first,
        "End/lock lost the actual historical publication",
    )?;
    let cleanup = json!({ "owner": candidate, "executorId": executor });
    f.resolved(
        "acknowledge_admitted_voice_key_publication",
        cleanup.clone(),
    )
    .await?;
    // Lost ACK reply recovery occurs BEFORE SDK settlement; the actual native
    // engine charge prevents its canonical terminal Cell being removed early.
    f.resolved(
        "acknowledge_admitted_voice_key_publication",
        cleanup.clone(),
    )
    .await?;
    f.resolved("settle_admitted_voice_join_key", cleanup)
        .await?;
    bounded(async {
        loop {
            crate::voice_admitted_call::collect_retired(&f.state());
            if f.state()
                .voice_authority
                .lock()
                .unwrap()
                .admitted_activation(&candidate)
                .is_none()
            {
                return;
            }
            // Harness-only clock wait: this observes actual return predicates,
            // never manufactures a returned/collected boolean or protocol expiry.
            SystemClock.sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    require(
        original.collectible(),
        "original Work/Root/task custody has not actually returned",
    )?;
    drop(original);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn admitted_commands_cross_real_tauri_json_dispatch_and_recover_locked_history() {
    let fixture = IpcFixture::new().await.expect("mock IPC fixture");
    // Keep fixture ownership outside the bounded scenario. Every ordinary IPC or
    // assertion-as-Result failure is followed by actual teardown before panicking.
    let scenario = bounded(admitted_scenario(&fixture)).await;
    let teardown = fixture.close().await;
    scenario
        .expect("scenario watchdog")
        .expect("actual Tauri admitted IPC scenario");
    teardown.expect("actual actor/mock app teardown");
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_fixture_task_is_actually_joined_and_never_reported_as_graceful_success() {
    struct ActualDrop(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for ActualDrop {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Release);
        }
    }
    let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let original = ActualDrop(returned.clone());
    let (send, started) = tokio::sync::oneshot::channel();
    let mut task = tokio::spawn(async move {
        let _original = original;
        let _ = send.send(());
        std::future::pending::<()>().await;
    });
    bounded(started).await.unwrap().unwrap();
    task.abort();
    let result = join_actual(&mut task, "cancelled fixture task").await;
    assert!(
        result.is_err(),
        "cancellation is not graceful actor/Root shutdown"
    );
    assert!(task.is_finished());
    // This witnesses only this helper task's actual destructor. It is NOT a
    // native Work, SDK, Root-residence or protocol retirement certificate.
    assert!(returned.load(std::sync::atomic::Ordering::Acquire));
}
