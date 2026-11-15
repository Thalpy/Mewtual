//! Plain native Create: retain the exact original from signing through actual
//! encrypted local IO. A local save does not establish transport or receipt.
use super::*;
use catcoms_app::store::{
    MemberSaveJobReport, MemberSaveNativeAdmission, NativeTextMemberFinalPreparation,
    NativeTextMemberPreparation, NativeTextMemberSaveJob, NativeUiInvocationMetadata,
    V2MemberSaveOutcome, V2MemberSaveRefusal, V2MemberSignRequest,
};
use catcoms_replication::chat::Body;
use tauri::ipc::InvokeResponseBody;
use tokio::sync::OwnedMutexGuard;

type Preparation = NativeTextMemberPreparation<catcoms_net::MeshService, OsCryptoRng>;
type FinalPreparation = NativeTextMemberFinalPreparation<catcoms_net::MeshService, OsCryptoRng>;
type SaveJob = NativeTextMemberSaveJob<catcoms_net::MeshService, OsCryptoRng>;
const BUSY: &str = "Chat is busy. Retry with the same message.";
const STALE: &str = "The chat session changed. Retry with the same message.";
const TEXT_LIMIT: usize = 8 * 1024;

struct Payment {
    _permit: OwnedSemaphorePermit,
    _credits: Vec<NativeUiInvocationMetadata>,
}
struct Intent {
    server: u64,
    channel: u128,
    token: [u8; 32],
    context: [u8; 32],
    text: String,
}
struct Invocation {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    selection: server_table::NativeSelection,
    instance: u64,
    scope: RequestCancellation,
    persist: Arc<Mutex<()>>,
    intent: Intent,
    payment: Arc<Payment>, // LAST through every blocking worker/result
}
fn parse(payload: &InvokeBody) -> Result<Intent, &'static str> {
    let InvokeBody::Json(value) = payload else {
        return Err("Invalid message request.");
    };
    let object = value.as_object().ok_or("Invalid message request.")?;
    if object.len() > 7
        || object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "server"
                    | "channel"
                    | "text"
                    | "replyTo"
                    | "retryToken"
                    | "expectedContext"
                    | "trace"
            )
        })
    {
        return Err("Invalid message request.");
    }
    let field = |key: &str, max| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| s.len() <= max)
            .ok_or("Invalid message request.")
    };
    let server = object
        .get("server")
        .and_then(serde_json::Value::as_u64)
        .ok_or("Invalid server.")?;
    let channel = field("channel", 39)?
        .parse()
        .map_err(|_| "Invalid channel.")?;
    let text = field("text", TEXT_LIMIT)?;
    if text.is_empty()
        || object
            .get("replyTo")
            .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
    {
        return Err("This chat currently supports plain messages only.");
    }
    let decode = |value: &str| {
        if value.len() != 64 {
            return Err("Keep the original message token and chat context.");
        }
        let mut bytes = [0; 32];
        hex::decode_to_slice(value, &mut bytes)
            .map_err(|_| "Invalid message token or chat context.")?;
        if bytes == [0; 32] {
            return Err("Invalid message token or chat context.");
        }
        Ok(bytes)
    };
    let token = decode(field("retryToken", 64)?)?;
    let context = decode(field("expectedContext", 64)?)?;
    if object
        .get("trace")
        .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| s.len() > 128))
    {
        return Err("Invalid message request.");
    }
    Ok(Intent {
        server,
        channel,
        token,
        context,
        text: text.to_owned(),
    })
}

pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("Chat is not available from this window.");
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let captured = (|| {
        let state = app.state::<AppState>();
        let generation = state.ui_session_generation.load(Ordering::SeqCst);
        if ui_session_closed(&state) || state.native_text.sources.is_closing_cached() {
            return Err(STALE);
        }
        let target = state.native_text.ui_state.original_target().ok_or(STALE)?;
        let permit = state
            .native_text
            .inflight
            .clone()
            .try_acquire_owned()
            .map_err(|_| BUSY)?;
        let first = state.native_text.ui_budget.prepare().map_err(|_| BUSY)?;
        let mut credits = Vec::with_capacity(8);
        credits.push(first);
        for _ in 1..8 {
            credits.push(state.native_text.ui_budget.prepare().map_err(|_| BUSY)?);
        }
        let payment = Arc::new(Payment {
            _permit: permit,
            _credits: credits,
        });
        let intent = parse(invoke.message.payload())?;
        let account = keep_session::current(&state, generation).ok_or(STALE)?;
        let (selection, instance, scope) = {
            // BEFORE scheduling: delayed work cannot select a replacement row.
            let table = state.servers.try_lock().map_err(|_| BUSY)?;
            let row = table.native(&intent.server).ok_or(STALE)?;
            let data = row.data();
            if data.request != intent.context
                || data.channel != intent.channel
                || !data.target.same_original(&target)
            {
                return Err(STALE);
            }
            (
                data.selection.clone(),
                data.instance,
                row.scope(account).ok_or(STALE)?,
            )
        };
        // This helper takes a mutex and may allocate. Never call it after final
        // Source preparation or while constructing the actual commit edge.
        let persist = persist_lock_for(&state, intent.server);
        Ok(Invocation {
            generation,
            target,
            selection,
            instance,
            scope,
            persist,
            intent,
            payment,
        })
    })();
    let captured = match captured {
        Ok(value) => value,
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let state = app.state::<AppState>();
        let result = execute(&state, &captured).await;
        #[cfg(test)]
        {
            let edge = state.native_text.text_reply_edge.lock().unwrap().clone();
            if let Some(edge) = edge {
                edge.wait_once().await;
            }
        }
        let _commit = state.ui_session_commit.lock().await;
        // Cancellation/cached target liveness cannot identify a still-live
        // original parked outside a replaced Store/table. Repeat the actual
        // installed-object checks through the real resolver call.
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        let original_mount = store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .is_some_and(|vault| captured.target.is_for_original_vault(vault));
        let original_row = table.native(&captured.intent.server).is_some_and(|row| {
            row.matches(&captured.selection, captured.instance, &captured.target)
                && row.data().request == captured.intent.context
                && row.data().channel == captured.intent.channel
        });
        if state.ui_session_generation.load(Ordering::SeqCst) != captured.generation
            || ui_session_closed(&state)
            || !captured.target.is_live_cached()
            || captured.scope.is_cancelled()
            || state.native_text.sources.is_closing_cached()
            || !original_mount
            || !original_row
        {
            resolver.reject(STALE);
        } else {
            match result {
                Ok(body) => resolver.resolve(InvokeResponseBody::Json(body)),
                Err(error) => resolver.reject(error),
            }
        }
    }));
    true
}

async fn capture(state: &AppState, invocation: &Invocation) -> Result<Preparation, String> {
    let policy = state.native_text.sources.policy();
    let mut budget = Some(policy.prepare_member().map_err(|_| BUSY)?);
    let mut ui =
        Some(founding::ui_capture(state, invocation.generation, &invocation.target).await?);
    let mut output = None;
    let result = {
        let _commit = require_ui_session_generation(state, invocation.generation).await?;
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        (|| {
            let vault = store
                .as_ref()
                .and_then(MountedVault::as_text_v2)
                .ok_or(STALE)?;
            let row = table.native(&invocation.intent.server).ok_or(STALE)?;
            if !invocation.target.is_for_original_vault(vault)
                || invocation.scope.is_cancelled()
                || !row.matches(
                    &invocation.selection,
                    invocation.instance,
                    &invocation.target,
                )
            {
                return Err(STALE);
            }
            state.native_text.sources.capture_member(
                &store,
                &mut ui,
                &invocation.selection,
                &mut budget,
                &mut output,
            )
        })()
    };
    result.map_err(str::to_owned)?;
    Ok(output.take().unwrap())
}

/// Fee/permit tail survives actual blocking execution even if the async/IPC
/// observer disappears. Root values drop/restore BEFORE this native payment.
struct Worker<T> {
    value: T,
    _payment: Arc<Payment>,
}
impl<T> Worker<T> {
    fn run<O>(self, run: impl FnOnce(T) -> O) -> O {
        run(self.value)
    }
    fn map<O>(self, map: impl FnOnce(T) -> O) -> Worker<O> {
        let value = map(self.value);
        Worker {
            value,
            _payment: self._payment,
        }
    }
}

// The spawn closure must capture ONE whole work object. Separately captured
// plaintext/scope/Worker fields have unspecified closure destruction order.
// Field order covers both an unrun queue item and unwind during real signing.
struct PrepareWork<P> {
    preparation: P,
    text: InputText,
    scope: RequestCancellation,
    token: [u8; 32],
    channel: u128,
    _payment: Arc<Payment>,
}
struct InputText {
    value: String,
    #[cfg(test)]
    _notice: Option<tests::PaymentNotice>,
}
impl PrepareWork<Preparation> {
    fn run(self) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        let body = Body::Create {
            text: self.text.value,
            reply_to: None,
        };
        self.preparation.finish()?.prepare_member_save(
            V2MemberSignRequest {
                token: self.token,
                channel: self.channel,
                body: &body,
                parents: &[],
                dependencies: &[],
            },
            &self.scope,
        )
    }
}

async fn prepare(state: &AppState, invocation: &Invocation) -> Result<V2MemberSaveOutcome, String> {
    let preparation = capture(state, invocation).await?;
    let work = PrepareWork {
        preparation,
        text: InputText {
            value: invocation.intent.text.clone(),
            #[cfg(test)]
            _notice: None,
        },
        scope: invocation.scope.clone(),
        token: invocation.intent.token,
        channel: invocation.intent.channel,
        _payment: invocation.payment.clone(),
    };
    tauri::async_runtime::spawn_blocking(move || work.run())
        .await
        .map_err(|_| STALE)?
        .map_err(refusal)
}
fn refusal(reason: V2MemberSaveRefusal) -> String {
    match reason {
        V2MemberSaveRefusal::Retained { token } => {
            format!("Finish the earlier message first ({}).", hex::encode(token))
        }
        _ => format!(
            "The message was not confirmed ({reason:?}). Retry with the same message token."
        ),
    }
}

struct Custody {
    persist: Option<OwnedMutexGuard<()>>,
    commit: Option<OwnedMutexGuard<()>>,
    _payment: Arc<Payment>,
}
// FIRST actual native custody; source preparation follows it on every early
// return/unwind. Guard refusal never destroys Source work under UI commit.
struct Attempt {
    custody: Option<Box<Custody>>,
    preparation: Option<FinalPreparation>,
    _payment: Arc<Payment>,
}
struct Admitted {
    job: Option<
        Result<
            SaveJob,
            Box<
                catcoms_app::store::NativeTextMemberAdmissionFailure<
                    catcoms_net::MeshService,
                    OsCryptoRng,
                >,
            >,
        >,
    >,
    _payment: Arc<Payment>,
}
fn admit(
    state: &AppState,
    invocation: &Invocation,
    preparation: FinalPreparation,
) -> Result<Worker<SaveJob>, String> {
    let mut attempt = Attempt {
        custody: Some(Box::new(Custody {
            persist: None,
            commit: None,
            _payment: invocation.payment.clone(),
        })),
        preparation: Some(preparation),
        _payment: invocation.payment.clone(),
    };
    let mut admitted = Admitted {
        job: None,
        _payment: invocation.payment.clone(),
    };
    let result = (|| {
        let custody = attempt.custody.as_mut().unwrap();
        custody.persist = Some(
            invocation
                .persist
                .clone()
                .try_lock_owned()
                .map_err(|_| BUSY)?,
        );
        custody.commit = Some(
            state
                .ui_session_commit
                .clone()
                .try_lock_owned()
                .map_err(|_| BUSY)?,
        );
        let store = state.store.try_lock().map_err(|_| BUSY)?;
        let table = state.servers.try_lock().map_err(|_| BUSY)?;
        let vault = store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .ok_or(STALE)?;
        let row = table.native(&invocation.intent.server).ok_or(STALE)?;
        if state.ui_session_generation.load(Ordering::SeqCst) != invocation.generation
            || ui_session_closed(state)
            || state.native_text.sources.is_closing_cached()
            || !invocation.target.is_for_original_vault(vault)
            || !row.matches(
                &invocation.selection,
                invocation.instance,
                &invocation.target,
            )
            || invocation.scope.is_cancelled()
        {
            return Err(STALE);
        }
        let native = MemberSaveNativeAdmission::from_boxed_custody(
            attempt.custody.take().unwrap(),
            invocation.scope.clone(),
        );
        admitted.job = Some(attempt.preparation.take().unwrap().try_admit(native));
        Ok(())
    })(); // Store/table guards have left before either whole owning result.
    drop(attempt); // native guards BEFORE any unentered Source preparation
    result.map_err(str::to_owned)?;
    match admitted.job.take().unwrap() {
        Ok(job) => Ok(Worker {
            value: job,
            _payment: invocation.payment.clone(),
        }),
        Err(failure) => {
            let reason = failure.refusal();
            drop(failure);
            Err(refusal(reason))
        }
    }
}
async fn execute(state: &AppState, invocation: &Invocation) -> Result<String, String> {
    let mut outcome = prepare(state, invocation).await?;
    if outcome == V2MemberSaveOutcome::Ready {
        let preparation = capture(state, invocation).await?;
        let token = invocation.intent.token;
        let worker = Worker {
            value: preparation,
            _payment: invocation.payment.clone(),
        };
        let prepared = tauri::async_runtime::spawn_blocking(move || {
            worker.map(|value| value.finish_admission(token))
        })
        .await
        .map_err(|_| STALE)?;
        let preparation = prepared.value.map_err(refusal)?;
        #[cfg(test)]
        {
            let edge = state.native_text.send_final_edge.lock().unwrap().clone();
            if let Some(edge) = edge {
                edge.wait_once().await;
            }
        }
        // TRY only: preparation owns counted physical Source work. Awaiting
        // commit here would deadlock whole-close, which drains that same work.
        let job = admit(state, invocation, preparation)?;
        #[cfg(test)]
        let io_edge = state.native_text.send_io_edge.lock().unwrap().clone();
        let report = tauri::async_runtime::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(edge) = io_edge {
                edge.wait_once();
            }
            job.run(|value| value.run())
        })
        .await
        .map_err(|_| STALE)?;
        if report == MemberSaveJobReport::Uncertain {
            return Ok(serde_json::json!({"backend":"text-v2","disposition":"uncertain","retry_token":hex::encode(token)}).to_string());
        }
        outcome = prepare(state, invocation).await?; // recover exact receipt without re-signing
    }
    match outcome {
        V2MemberSaveOutcome::Saved{action,ordinal}=>Ok(serde_json::json!({"backend":"text-v2","disposition":"saved-local",
            "message_id":hex::encode(action.as_bytes()),"ordinal":ordinal.to_string(),"retry_token":hex::encode(invocation.intent.token)}).to_string()),
        V2MemberSaveOutcome::Ready=>Err("The message outcome is still being checked. Retry with the same message token.".into()),
    }
}
const _: () = assert!(
    std::mem::size_of::<Invocation>() + std::mem::size_of::<Attempt>() + 3 * TEXT_LIMIT + 4096
        <= 8 * 4096
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Weak};

    pub(super) struct PaymentNotice {
        payment: Weak<Payment>,
        observed: Arc<AtomicUsize>,
        covered: Arc<AtomicBool>,
    }
    impl Drop for PaymentNotice {
        fn drop(&mut self) {
            if self.payment.upgrade().is_none() {
                self.covered.store(false, Ordering::SeqCst);
            }
            self.observed.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn payment() -> Arc<Payment> {
        let budget = NativeUiInvocationBudget::new(
            catcoms_app::store::CriticalCatalogReadPool::new(
                NativeUiInvocationBudget::required_capacity(),
            )
            .unwrap(),
        )
        .unwrap();
        Arc::new(Payment {
            _permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
            _credits: vec![budget.prepare().unwrap()],
        })
    }
    #[test]
    fn native_text_prepare_work_unrun_and_unwind_keep_real_input_payment_last() {
        // This checks the ACTUAL work wrapper/closure ownership, not native
        // authentication. The separate router cases execute genuine signing.
        for unwind in [false, true] {
            let payment = payment();
            let weak = Arc::downgrade(&payment);
            let observed = Arc::new(AtomicUsize::new(0));
            let covered = Arc::new(AtomicBool::new(true));
            let notice = || PaymentNotice {
                payment: weak.clone(),
                observed: observed.clone(),
                covered: covered.clone(),
            };
            let (_sender, signal) = watch::channel(false);
            let work = PrepareWork {
                preparation: notice(),
                text: InputText {
                    value: "paid plaintext".into(),
                    _notice: Some(notice()),
                },
                scope: RequestCancellation::new(signal, None),
                token: [1; 32],
                channel: 7,
                _payment: payment,
            };
            if unwind {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    let _work = work;
                    panic!("actual work-wrapper unwind");
                }));
                assert!(result.is_err());
            } else {
                drop(move || drop(work)); // actual unrun whole-object capture
            }
            assert_eq!(observed.load(Ordering::SeqCst), 2);
            assert!(
                covered.load(Ordering::SeqCst),
                "native data outlived its actual payment"
            );
            assert!(weak.upgrade().is_none());
        }
    }
    #[test]
    fn native_text_worker_map_keeps_payment_through_queued_and_returned_owners() {
        for run in [false, true] {
            let payment = payment();
            let weak = Arc::downgrade(&payment);
            let observed = Arc::new(AtomicUsize::new(0));
            let covered = Arc::new(AtomicBool::new(true));
            let worker = Worker {
                value: PaymentNotice {
                    payment: weak.clone(),
                    observed: observed.clone(),
                    covered: covered.clone(),
                },
                _payment: payment,
            };
            let queued = move || worker.map(|value| value);
            if run {
                drop(queued());
            } else {
                drop(queued);
            }
            assert_eq!(observed.load(Ordering::SeqCst), 1);
            assert!(
                covered.load(Ordering::SeqCst),
                "native data outlived its actual payment"
            );
            assert!(weak.upgrade().is_none());
        }
    }
    #[test]
    fn native_text_send_parser_requires_exact_token_context_and_plain_bounded_body() {
        let base = serde_json::json!({"server":1,"channel":"7","text":"hello","replyTo":null,
            "retryToken":"01".repeat(32),"expectedContext":"02".repeat(32)});
        let accepted = parse(&InvokeBody::Json(base.clone())).unwrap();
        assert_eq!(accepted.token, [1; 32]);
        assert_eq!(accepted.context, [2; 32]);
        for (key, value) in [
            ("retryToken", serde_json::json!("01".repeat(16))),
            ("expectedContext", serde_json::json!("00".repeat(32))),
            ("channel", serde_json::json!("x")),
            ("text", serde_json::json!("x".repeat(TEXT_LIMIT + 1))),
            ("text", serde_json::json!("")),
            ("replyTo", serde_json::json!("01".repeat(32))),
            ("trace", serde_json::json!("x".repeat(129))),
            ("protocol", serde_json::json!("legacy")),
        ] {
            let mut changed = base.clone();
            changed[key] = value;
            assert!(parse(&InvokeBody::Json(changed)).is_err(), "{key}");
        }
    }
}
