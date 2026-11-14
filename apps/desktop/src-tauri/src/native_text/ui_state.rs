//! Native local UI-state IO. The task, not an IPC waiter, owns lane and Root
//! output through actual response publication. Lock/auth snapshot integration
//! joins the same lane separately; production selection remains disabled.
use super::*;
use catcoms_app::store::{
    CriticalCatalogReadPool, NativeUiInvocationMetadata, NativeUiStateBudget,
    NativeUiStatePreparation, NativeUiStateRead, NativeUiStateSaved,
};
use tauri::ipc::InvokeResponseBody;

pub(super) const EMPTY: &str = r#"{"version":1,"drafts":{},"readMarks":{}}"#;
pub(crate) struct Controller {
    pub(crate) lane: Arc<Mutex<()>>,
    pub(super) target: StdMutex<Option<Arc<NativeTextVaultAuthenticationCheck>>>,
    waiters: Arc<Semaphore>,
    budget: NativeUiStateBudget,
    #[cfg(test)]
    pub(super) output_drop: StdMutex<Option<Arc<AtomicBool>>>,
    #[cfg(test)]
    pub(super) after_io: StdMutex<Option<Arc<tests::AdmissionBarrier>>>,
    #[cfg(test)]
    pub(super) before_io: StdMutex<Option<Arc<tests::BlockingIoBarrier>>>,
    #[cfg(test)]
    pub(super) lock_waiting: tokio::sync::Notify,
    _metadata: NativeUiInvocationMetadata,
}
impl Controller {
    pub(super) fn new(metadata: NativeUiInvocationMetadata) -> Self {
        Self {
            lane: Arc::new(Mutex::new(())),
            target: StdMutex::new(None),
            waiters: Arc::new(Semaphore::new(4)),
            budget: NativeUiStateBudget::new(
                CriticalCatalogReadPool::new(NativeUiStateBudget::required_capacity())
                    .expect("fixed UI-state pool"),
            )
            .expect("fixed UI-state policy"),
            #[cfg(test)]
            output_drop: StdMutex::new(None),
            #[cfg(test)]
            after_io: StdMutex::new(None),
            #[cfg(test)]
            before_io: StdMutex::new(None),
            #[cfg(test)]
            lock_waiting: tokio::sync::Notify::new(),
            _metadata: metadata,
        }
    }
    /// Metadata only; never keeps the affine vault/physical session alive.
    pub(crate) fn original_target(&self) -> Option<Arc<NativeTextVaultAuthenticationCheck>> {
        self.target
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    /// Prepare outside native commit. Same-root reauthentication keeps the
    /// original canonical allocation, rather than rebinding pending references.
    pub(crate) fn canonical_target(
        &self,
        fresh: &Arc<NativeTextVaultAuthenticationCheck>,
    ) -> Arc<NativeTextVaultAuthenticationCheck> {
        let current = self.original_target();
        match current {
            Some(current) if current.is_live_cached() && current.same_original(fresh) => current,
            _ => fresh.clone(),
        }
    }
    /// Install a prebuilt allocation. The replaced whole check returns to an
    /// OUTER native variable for destruction after every commit/store guard.
    pub(crate) fn install_target(
        &self,
        target: Arc<NativeTextVaultAuthenticationCheck>,
    ) -> Option<Arc<NativeTextVaultAuthenticationCheck>> {
        self.target
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(target)
    }
    /// Same real lane acquisition in production and tests. The test receipt
    /// observes an actual Pending poll; it supplies no ownership or permission.
    pub(crate) async fn lock_lane(&self) -> tokio::sync::OwnedMutexGuard<()> {
        let acquire = self.lane.clone().lock_owned();
        #[cfg(test)]
        {
            let mut acquire = std::pin::pin!(acquire);
            std::future::poll_fn(|cx| {
                let polled = acquire.as_mut().poll(cx);
                if polled.is_pending() {
                    self.lock_waiting.notify_one();
                }
                polled
            })
            .await
        }
        #[cfg(not(test))]
        {
            acquire.await
        }
    }
}
enum Operation {
    Read,
    Save(String),
}
enum Output {
    Read(NativeUiStateRead),
    Saved(NativeUiStateSaved),
}
// Field order matters if the native observer is dropped while blocking IO
// continues: all real output/slot/payment custody retires BEFORE lane release.
struct Completed {
    result: Result<(RetainedOutput, Option<InvokeResponseBody>), String>,
    _lane: tokio::sync::OwnedMutexGuard<()>,
}
// Real Root output first; the observation runs only AFTER its actual slot,
// physical custody and fee have been destroyed. It supplies no permission.
struct RetainedOutput {
    value: Output,
    #[cfg(test)]
    _notice: Option<DropNotice>,
}
impl RetainedOutput {
    fn is_live(&self) -> bool {
        self.value.is_live()
    }
}
#[cfg(test)]
struct DropNotice {
    commit: Arc<Mutex<()>>,
    outside: Arc<AtomicBool>,
}
#[cfg(test)]
impl Drop for DropNotice {
    fn drop(&mut self) {
        self.outside
            .store(self.commit.try_lock().is_ok(), Ordering::SeqCst);
    }
}
impl Output {
    fn is_live(&self) -> bool {
        match self {
            Self::Read(value) => value.is_live_cached(),
            Self::Saved(value) => value.is_live_cached(),
        }
    }
}
fn parse(body: &InvokeBody, save: bool) -> Result<Operation, &'static str> {
    let InvokeBody::Json(value) = body else {
        return Err("Invalid UI state request.");
    };
    let object = value.as_object().ok_or("Invalid UI state request.")?;
    if !save {
        if !object.is_empty() {
            return Err("Invalid UI state request.");
        }
        return Ok(Operation::Read);
    }
    if object.len() != 1 {
        return Err("Invalid UI state request.");
    }
    let json = object
        .get("json")
        .and_then(serde_json::Value::as_str)
        .ok_or("Missing UI state.")?;
    if json.len() > MAX_UI_STATE_BYTES {
        return Err("UI state is too large.");
    }
    Ok(Operation::Save(json.to_owned()))
}
/// Only called by the trusted native-mode synchronous outer router. No renderer
/// flag selects this implementation. ACL/generation/capacity precede any copy.
pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    let save = invoke.message.command() == "save_ui_state";
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("UI state is not available from this window.");
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let captured = {
        let state = app.state::<AppState>();
        let generation = state.ui_session_generation.load(Ordering::SeqCst);
        let control = &state.native_text.ui_state;
        if state.ui_authorized_generation.load(Ordering::SeqCst) != generation
            || state.session_lock_requested.load(Ordering::SeqCst)
        {
            invoke.resolver.reject("Unlock the vault first.");
            return true;
        }
        control
            .waiters
            .clone()
            .try_acquire_owned()
            .map_err(|_| "UI state is busy. Try again.")
            .and_then(|permit| {
                control
                    .budget
                    .prepare()
                    .map_err(|_| "UI state is busy. Try again.")
                    .and_then(|credit| {
                        let target = control
                            .original_target()
                            .ok_or("The native vault is unavailable.")?;
                        parse(invoke.message.payload(), save)
                            .map(|op| (generation, permit, credit, op, target))
                    })
            })
    };
    let (generation, _permit, credit, operation, target) = match captured {
        Ok(value) => value,
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let _permit = _permit;
        let state = app.state::<AppState>();
        // No vault_mount here: authentication holds it while awaiting this lane.
        let lane = state.native_text.ui_state.lane.clone().lock_owned().await;
        let mut preparation: Option<NativeUiStatePreparation> = None;
        let admission = async {
            let _commit = require_ui_session_generation(&state, generation).await?;
            let store = state.store.lock().await;
            let vault = store
                .as_ref()
                .and_then(MountedVault::as_text_v2)
                .ok_or("The native vault is unavailable.")?;
            preparation = Some(vault.prepare_ui_state_for_original(credit, target));
            Ok::<_, String>(())
        }
        .await;
        if let Err(error) = admission {
            resolver.reject(error);
            return;
        }
        let preparation = preparation.expect("original admission captured one capsule");
        #[cfg(test)]
        let notice = state
            .native_text
            .ui_state
            .output_drop
            .lock()
            .unwrap()
            .clone()
            .map(|outside| DropNotice {
                commit: state.ui_session_commit.clone(),
                outside,
            });
        #[cfg(test)]
        let before_io = state.native_text.ui_state.before_io.lock().unwrap().clone();
        let mut done = tauri::async_runtime::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(edge) = before_io {
                edge.wait_once();
            }
            let result = (|| {
                let output = match operation {
                    Operation::Read => Output::Read(preparation.read().map_err(|e| e.to_string())?),
                    Operation::Save(json) => {
                        validate_ui_state_json(&json)?;
                        Output::Saved(
                            preparation
                                .save(json.as_bytes(), &mut OsCryptoRng)
                                .map_err(|e| e.to_string())?,
                        )
                    }
                };
                let serialized = match &output {
                    Output::Read(value) => {
                        let json = match value.bytes() {
                            None => EMPTY,
                            Some(bytes) => std::str::from_utf8(bytes)
                                .map_err(|_| "Saved UI state is not UTF-8.")?,
                        };
                        validate_ui_state_json(json)?;
                        serde_json::to_string(json).map_err(|e| e.to_string())?
                    }
                    Output::Saved(_) => "null".to_owned(),
                };
                let retained = RetainedOutput {
                    value: output,
                    #[cfg(test)]
                    _notice: notice,
                };
                Ok::<_, String>((retained, Some(InvokeResponseBody::Json(serialized))))
            })();
            Completed {
                result,
                _lane: lane,
            }
        })
        .await;
        let done = match done.as_mut() {
            Ok(done) => done,
            Err(_) => {
                resolver.reject("UI state operation failed.");
                return;
            }
        };
        #[cfg(test)]
        {
            let edge = state.native_text.ui_state.after_io.lock().unwrap().clone();
            if let Some(edge) = edge {
                edge.wait_once().await;
            }
        }
        // Retain all output/custody in this OUTER result. Actual resolver work
        // occurs under commit; serialization already finished on blocking pool.
        let _commit = state.ui_session_commit.lock().await;
        if state.ui_session_generation.load(Ordering::SeqCst) != generation
            || state.ui_authorized_generation.load(Ordering::SeqCst) != generation
            || state.session_lock_requested.load(Ordering::SeqCst)
        {
            resolver.reject("UI state request was superseded.");
            return;
        }
        match &done.result {
            Ok((output, _)) if !output.is_live() => {
                resolver.reject("The vault has closed.");
                return;
            }
            _ => {}
        }
        // Leave the paid output inside outer Completed even if the actual
        // responder panics. Only the already-serialized body moves here.
        match &mut done.result {
            Ok((_output, body)) => resolver.resolve(body.take().expect("one response")),
            Err(error) => resolver.reject(error.clone()),
        }
    }));
    true
}
const _: () = assert!(
    std::mem::size_of::<Controller>()
        + std::mem::size_of::<Operation>()
        + std::mem::size_of::<Output>()
        + 128 * std::mem::size_of::<usize>()
        < 4096
);
