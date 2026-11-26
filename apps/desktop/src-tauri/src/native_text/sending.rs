//! Native ordinary actions retain the exact original from signing through actual
//! encrypted local IO. The local receipt precedes any joined outbox attempt.
use super::*;
use catcoms_app::store::{
    MemberSaveJobReport, MemberSaveNativeAdmission, NativeTextJoinedMemberFinalPreparation,
    NativeTextJoinedMemberSaveJob, NativeTextJoinedRowSelection,
    NativeTextJoinedSelectionPreparation, NativeTextMemberFinalPreparation,
    NativeTextMemberPreparation, NativeTextMemberSaveJob, NativeUiInvocationMetadata,
    V2MemberSaveOutcome, V2MemberSaveRefusal, V2MemberSignRequest,
};
use catcoms_replication::chat::{ActionId, Body};
use tauri::ipc::InvokeResponseBody;
use tokio::sync::OwnedMutexGuard;

type FounderPreparation = NativeTextMemberPreparation<catcoms_net::MeshService, OsCryptoRng>;
type FounderFinal = NativeTextMemberFinalPreparation<catcoms_net::MeshService, OsCryptoRng>;
type FounderJob = NativeTextMemberSaveJob<catcoms_net::MeshService, OsCryptoRng>;

enum Preparation {
    Founder(FounderPreparation),
    Joined(
        NativeTextJoinedSelectionPreparation<OsCryptoRng>,
        NativeTextJoinedRowSelection,
    ),
}

enum FinalPreparation {
    Founder(FounderFinal),
    Joined(Option<NativeTextJoinedMemberFinalPreparation<OsCryptoRng>>),
}

enum SaveJob {
    Founder(FounderJob),
    Joined(NativeTextJoinedMemberSaveJob<OsCryptoRng>),
}

impl SaveJob {
    fn run(self) -> MemberSaveJobReport {
        match self {
            Self::Founder(job) => job.run(),
            Self::Joined(job) => job.run(),
        }
    }
}
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
    body: Body,
    parent: Option<ActionId>,
}
struct Invocation {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    selection: server_table::NativeSelection,
    instance: u64,
    scope: RequestCancellation,
    persist: Arc<Mutex<()>>,
    member_work: Arc<server_table::NativeMemberWork>,
    intent: Intent,
    payment: Arc<Payment>, // LAST through every blocking worker/result
}
#[derive(Clone, Copy)]
enum Kind {
    Create,
    Edit,
    Delete,
}

fn parse(payload: &InvokeBody, kind: Kind) -> Result<Intent, &'static str> {
    let InvokeBody::Json(value) = payload else {
        return Err("Invalid message request.");
    };
    let object = value.as_object().ok_or("Invalid message request.")?;
    let specific: &[&str] = match kind {
        Kind::Create => &["text", "replyTo"],
        Kind::Edit => &["text", "messageId", "expectedRevision"],
        Kind::Delete => &["messageId", "expectedRevision"],
    };
    if object.len() > 8
        || object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "server" | "channel" | "retryToken" | "expectedContext" | "trace"
            ) && !specific.contains(&key.as_str())
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
    let text = || {
        let text = field("text", TEXT_LIMIT)?;
        if text.is_empty() {
            return Err("Enter a message.");
        }
        Ok(text.to_owned())
    };
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
    let decode_action = |value: &str| {
        decode(value)
            .map(ActionId::from_bytes)
            .map_err(|_| "Invalid message action.")
    };
    if object
        .get("trace")
        .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| s.len() > 128))
    {
        return Err("Invalid message request.");
    }
    let (body, parent) = match kind {
        Kind::Create => {
            let reply_to = match object.get("replyTo") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(value)) if value.is_empty() => None,
                Some(serde_json::Value::String(value)) => Some(decode_action(value)?),
                _ => return Err("Invalid reply target."),
            };
            (
                Body::Create {
                    text: text()?,
                    reply_to,
                },
                reply_to,
            )
        }
        Kind::Edit | Kind::Delete => {
            let create = decode_action(field("messageId", 64)?)?;
            let revision = decode_action(field("expectedRevision", 64)?)?;
            let body = match kind {
                Kind::Edit => Body::Edit {
                    create,
                    text: text()?,
                },
                _ => Body::Delete { create },
            };
            (body, Some(revision))
        }
    };
    Ok(Intent {
        server,
        channel,
        token,
        context,
        body,
        parent,
    })
}

pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    let kind = match invoke.message.command() {
        "send_message" => Kind::Create,
        "edit_message" => Kind::Edit,
        "delete_message" => Kind::Delete,
        _ => return false,
    };
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
        let intent = parse(invoke.message.payload(), kind)?;
        let account = keep_session::current(&state, generation).ok_or(STALE)?;
        let (selection, instance, scope, member_work) = {
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
                data.member_work.clone(),
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
            member_work,
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
        let wake = {
            let _commit = state.ui_session_commit.lock().await;
            // A parked original can stay live after the Store/table has been
            // replaced. Reprove the installed objects through the resolver.
            let store = state.store.lock().await;
            let table = state.servers.lock().await;
            let original_mount = store
                .as_ref()
                .and_then(MountedVault::as_text_v2)
                .is_some_and(|vault| captured.target.is_for_original_vault(vault));
            let original_row = table.native(&captured.intent.server).filter(|row| {
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
                || original_row.is_none()
            {
                resolver.reject(STALE);
                None
            } else {
                match result {
                    Ok(reply) => {
                        resolver.resolve(InvokeResponseBody::Json(reply.body));
                        original_row
                            .filter(|_| reply.saved_local)
                            .map(|row| row.data().member_work.clone())
                    }
                    Err(error) => {
                        resolver.reject(error);
                        None
                    }
                }
            }
        };
        if let Some(work) = wake {
            work.publication_wake.notify_one();
        }
    }));
    true
}

async fn capture(state: &AppState, invocation: &Invocation) -> Result<Preparation, String> {
    let policy = state.native_text.sources.policy();
    let mut budget = if invocation.selection.founder().is_some() {
        Some(policy.prepare_member().map_err(|_| BUSY)?)
    } else {
        None
    };
    let mut ui =
        Some(founding::ui_capture(state, invocation.generation, &invocation.target).await?);
    let mut founder = None;
    let mut joined = None;
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
            match &invocation.selection {
                server_table::NativeSelection::Founder(_) => {
                    state.native_text.sources.capture_member(
                        &store,
                        &mut ui,
                        &invocation.selection,
                        &mut budget,
                        &mut founder,
                    )
                }
                server_table::NativeSelection::Joined(selection) => state
                    .native_text
                    .sources
                    .capture_joined_member(&store, &mut ui, selection, &mut joined),
            }
        })()
    };
    result.map_err(str::to_owned)?;
    match (&invocation.selection, founder, joined) {
        (server_table::NativeSelection::Founder(_), Some(original), _) => {
            Ok(Preparation::Founder(original))
        }
        (server_table::NativeSelection::Joined(row), _, Some(original)) => {
            Ok(Preparation::Joined(original, row.clone()))
        }
        _ => Err(STALE.to_owned()),
    }
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
    input: ActionInput,
    scope: RequestCancellation,
    token: [u8; 32],
    channel: u128,
    _payment: Arc<Payment>,
}
struct ActionInput {
    body: Body,
    parent: Option<ActionId>,
    #[cfg(test)]
    _notice: Option<tests::PaymentNotice>,
}
impl PrepareWork<Preparation> {
    fn run(self) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        let request = V2MemberSignRequest {
            token: self.token,
            channel: self.channel,
            body: &self.input.body,
            parents: self.input.parent.as_slice(),
            dependencies: &[],
        };
        match self.preparation {
            Preparation::Founder(original) => {
                original.finish()?.prepare_member_save(request, &self.scope)
            }
            Preparation::Joined(original, row) => {
                if original.is_capture_busy_cached() {
                    return Err(V2MemberSaveRefusal::Busy);
                }
                original.prepare_member_save(&row, request, &self.scope, &mut OsCryptoRng)
            }
        }
    }
}

async fn retry_preparation<T, F>(
    invocation: &Invocation,
    #[cfg(test)] state: &AppState,
    attempt: impl Fn() -> F,
) -> Result<T, String>
where
    F: Future<Output = Result<Result<T, V2MemberSaveRefusal>, String>> + Send,
{
    let deadline = SystemClock.monotonic_ms().saturating_add(2_000);
    let mut scope = invocation.scope.clone();
    loop {
        match attempt().await? {
            Err(V2MemberSaveRefusal::Busy) if SystemClock.monotonic_ms() < deadline => {
                #[cfg(test)]
                state.native_text.send_preparation_busy.notify_one();
                // The failed preparation is gone before waiting. The command
                // still owns its original row, action and retry token.
                tokio::select! {
                    biased;
                    _ = scope.cancelled() => return Err(STALE.into()),
                    _ = SystemClock.sleep(Duration::from_millis(25)) => {},
                }
            }
            result => return result.map_err(refusal),
        }
    }
}

async fn prepare(
    state: &AppState,
    invocation: &Invocation,
    local: Arc<OwnedMutexGuard<()>>,
) -> Result<V2MemberSaveOutcome, String> {
    retry_preparation(
        invocation,
        #[cfg(test)]
        state,
        || async {
            let preparation = capture(state, invocation).await?;
            let work = PrepareWork {
                preparation,
                input: ActionInput {
                    body: invocation.intent.body.clone(),
                    parent: invocation.intent.parent,
                    #[cfg(test)]
                    _notice: None,
                },
                scope: invocation.scope.clone(),
                token: invocation.intent.token,
                channel: invocation.intent.channel,
                _payment: invocation.payment.clone(),
            };
            let local = local.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _local = local;
                work.run()
            })
            .await
            .map_err(|_| STALE.to_owned())
        },
    )
    .await
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
    job: Option<Result<SaveJob, SaveAdmissionRefusal>>,
    _payment: Arc<Payment>,
}
enum SaveAdmissionRefusal {
    Founder(
        Box<
            catcoms_app::store::NativeTextMemberAdmissionFailure<
                catcoms_net::MeshService,
                OsCryptoRng,
            >,
        >,
    ),
    Joined {
        reason: V2MemberSaveRefusal,
        _native: MemberSaveNativeAdmission,
        _original: Option<NativeTextJoinedMemberFinalPreparation<OsCryptoRng>>,
    },
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
        admitted.job = Some(
            if let Some(FinalPreparation::Joined(original)) = attempt.preparation.as_mut() {
                match NativeTextJoinedMemberFinalPreparation::try_admit_member_save(
                    original,
                    &invocation.scope,
                    native,
                ) {
                    Ok(job) => Ok(SaveJob::Joined(job)),
                    Err((native, reason)) => Err(SaveAdmissionRefusal::Joined {
                        reason,
                        _native: native,
                        _original: original.take(),
                    }),
                }
            } else {
                let Some(FinalPreparation::Founder(founder)) = attempt.preparation.take() else {
                    unreachable!("checked founder final preparation")
                };
                founder
                    .try_admit(native)
                    .map(SaveJob::Founder)
                    .map_err(SaveAdmissionRefusal::Founder)
            },
        );
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
            let reason = match &failure {
                SaveAdmissionRefusal::Founder(original) => original.refusal(),
                SaveAdmissionRefusal::Joined { reason, .. } => *reason,
            };
            drop(failure);
            Err(refusal(reason))
        }
    }
}
struct Reply {
    body: String,
    saved_local: bool,
}

async fn execute(state: &AppState, invocation: &Invocation) -> Result<Reply, String> {
    let local = Arc::new(
        invocation
            .member_work
            .foreground(invocation.scope.clone())
            .await
            .ok_or(STALE)?,
    );
    let mut outcome = prepare(state, invocation, local.clone()).await?;
    if outcome == V2MemberSaveOutcome::Ready {
        let token = invocation.intent.token;
        let preparation = retry_preparation(
            invocation,
            #[cfg(test)]
            state,
            || async {
                let preparation = capture(state, invocation).await?;
                let scope = invocation.scope.clone();
                let worker = Worker {
                    value: preparation,
                    _payment: invocation.payment.clone(),
                };
                let local = local.clone();
                let prepared = tauri::async_runtime::spawn_blocking(move || {
                    let _local = local;
                    worker.map(|value| match value {
                        Preparation::Founder(original) => original
                            .finish_admission(token)
                            .map(FinalPreparation::Founder),
                        Preparation::Joined(original, row) => {
                            if original.is_capture_busy_cached() {
                                return Err(V2MemberSaveRefusal::Busy);
                            }
                            original
                                .finish_member_admission(row, token, &scope)
                                .map(|ready| FinalPreparation::Joined(Some(ready)))
                        }
                    })
                })
                .await
                .map_err(|_| STALE.to_owned())?;
                Ok(prepared.value)
            },
        )
        .await?;
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
        let local_during_save = local.clone();
        let report = tauri::async_runtime::spawn_blocking(move || {
            let _local = local_during_save;
            #[cfg(test)]
            if let Some(edge) = io_edge {
                edge.wait_once();
            }
            job.run(|value| value.run())
        })
        .await
        .map_err(|_| STALE)?;
        if report == MemberSaveJobReport::Uncertain {
            return Ok(Reply {
                body: serde_json::json!({"backend":"text-v2","disposition":"uncertain","retry_token":hex::encode(token)}).to_string(),
                saved_local: false,
            });
        }
        outcome = prepare(state, invocation, local).await?; // exact saved receipt
    }
    match outcome {
        V2MemberSaveOutcome::Saved{action,ordinal}=>Ok(Reply {
            body: serde_json::json!({"backend":"text-v2","disposition":"saved-local",
                "message_id":hex::encode(action.as_bytes()),"ordinal":ordinal.to_string(),"retry_token":hex::encode(invocation.intent.token)}).to_string(),
            saved_local: true,
        }),
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
                input: ActionInput {
                    body: Body::Create {
                        text: "paid plaintext".into(),
                        reply_to: None,
                    },
                    parent: None,
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
    fn native_text_send_parser_requires_exact_token_context_and_bounded_reply() {
        let base = serde_json::json!({"server":1,"channel":"7","text":"hello","replyTo":null,
            "retryToken":"01".repeat(32),"expectedContext":"02".repeat(32)});
        let accepted = parse(&InvokeBody::Json(base.clone()), Kind::Create).unwrap();
        assert_eq!(accepted.token, [1; 32]);
        assert_eq!(accepted.context, [2; 32]);
        assert!(accepted.parent.is_none());
        let mut reply = base.clone();
        reply["replyTo"] = serde_json::json!("03".repeat(32));
        assert_eq!(
            parse(&InvokeBody::Json(reply), Kind::Create)
                .unwrap()
                .parent,
            Some(ActionId::from_bytes([3; 32])),
        );
        for (key, value) in [
            ("retryToken", serde_json::json!("01".repeat(16))),
            ("expectedContext", serde_json::json!("00".repeat(32))),
            ("channel", serde_json::json!("x")),
            ("text", serde_json::json!("x".repeat(TEXT_LIMIT + 1))),
            ("text", serde_json::json!("")),
            ("replyTo", serde_json::json!("00".repeat(32))),
            ("replyTo", serde_json::json!("01".repeat(33))),
            ("replyTo", serde_json::json!([1])),
            ("trace", serde_json::json!("x".repeat(129))),
            ("protocol", serde_json::json!("legacy")),
        ] {
            let mut changed = base.clone();
            changed[key] = value;
            assert!(
                parse(&InvokeBody::Json(changed), Kind::Create).is_err(),
                "{key}"
            );
        }
        let mut edit = base;
        edit.as_object_mut().unwrap().remove("replyTo");
        edit["messageId"] = serde_json::json!("04".repeat(32));
        edit["expectedRevision"] = serde_json::json!("05".repeat(32));
        let parsed = parse(&InvokeBody::Json(edit.clone()), Kind::Edit).unwrap();
        assert!(
            matches!(parsed.body, Body::Edit { create, .. } if create == ActionId::from_bytes([4; 32]))
        );
        assert_eq!(parsed.parent, Some(ActionId::from_bytes([5; 32])));
        assert!(parse(&InvokeBody::Json(edit.clone()), Kind::Delete).is_err());
        edit.as_object_mut().unwrap().remove("text");
        assert!(matches!(
            parse(&InvokeBody::Json(edit.clone()), Kind::Delete)
                .unwrap()
                .body,
            Body::Delete { .. }
        ));
        edit.as_object_mut().unwrap().remove("expectedRevision");
        assert!(parse(&InvokeBody::Json(edit), Kind::Delete).is_err());
    }
}
