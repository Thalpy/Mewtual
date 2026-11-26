//! One bounded receiver on the installed joined row, with guarded live
//! projection and archive selection under the same local permit.
use super::{connected::Source, *};
use catcoms_app::store::{
    NativeTextFounderDependencyExchange, NativeTextFounderFrameOutcome,
    NativeTextFounderMemberReceiver, NativeTextFounderReceived, NativeTextJoinedMemberLedger,
    NativeTextJoinedMemberReceiver, NativeTextJoinedReceivedOriginal, NativeTextJoinedRowSelection,
    NativeTextJoinedSelectionPreparation, NativeTextMemberPreparation, NativeUiInvocationMetadata,
    V2MemberSaveRefusal,
};
use catcoms_net::{NativeChatMemberFrame, NativeChatMemberGossipEvent};
use catcoms_replication::chat::{ActionId, ActionState, V2OrdinaryReceiveStatus};
use catcoms_rt::RequestCancellation;
use std::time::Duration;

const REOPEN_DELAY: Duration = Duration::from_secs(1);
const DEPENDENCY_TIMEOUT: Duration = Duration::from_secs(2);
const DEPENDENCIES_PER_FRAME: usize = 4;
const FOUNDER_CAPTURE_RETRY_DELAY: Duration = Duration::from_millis(25);
const FOUNDER_CAPTURE_ATTEMPTS: usize = 8;
const FOUNDER_RECEIVE_RETRY_MS: u64 = 2_000;

type FounderPreparation = NativeTextMemberPreparation<catcoms_net::MeshService, OsCryptoRng>;
type FounderReceiver = NativeTextFounderMemberReceiver<OsCryptoRng>;
type FounderReceiveResult = Result<NativeTextFounderFrameOutcome, V2MemberSaveRefusal>;

pub(super) async fn capture_founder_frame(
    state: &AppState,
    source: &Source,
) -> Option<(tokio::sync::OwnedMutexGuard<()>, FounderPreparation)> {
    let mut scope = source.negative();
    for attempt in 0..FOUNDER_CAPTURE_ATTEMPTS {
        let permit = source.member_work().foreground(scope.clone()).await?;
        if let Some(prepared) = source.capture(state).await {
            return Some((permit, prepared));
        }
        drop(permit);
        if attempt + 1 == FOUNDER_CAPTURE_ATTEMPTS {
            break;
        }
        tokio::select! {
            biased;
            _ = scope.cancelled() => return None,
            _ = SystemClock.sleep(FOUNDER_CAPTURE_RETRY_DELAY) => {}
        }
    }
    None
}

pub(super) async fn run_founder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: &Arc<Source>,
) {
    let mut scope = source.negative();
    let channel = source.channel();
    let mut receiver: Option<FounderReceiver> = None;
    loop {
        let state = app.state::<AppState>();
        let Some(mut original) = receiver.take() else {
            tokio::select! {
                biased;
                _ = scope.cancelled() => return,
                _ = SystemClock.sleep(REOPEN_DELAY) => {},
            }
            let local = source.member_work();
            let Some(permit) = local.try_background() else {
                continue;
            };
            let Some(preparation) = source.capture(&state).await else {
                continue;
            };
            let caller = scope.clone();
            let Ok(Ok(prepared)) = tauri::async_runtime::spawn_blocking(move || {
                let _permit = permit;
                preparation.prepare_founder_member_ingress(channel, caller)
            })
            .await
            else {
                continue;
            };
            receiver = prepared.open().await.ok();
            #[cfg(test)]
            if receiver.is_some() {
                state.native_text.founder_ingress_ready.notify_one();
            }
            continue;
        };
        let event = tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            event = original.next() => event,
        };
        match event {
            Some(NativeChatMemberGossipEvent::Frame(frame)) => {
                if let Some((original, action)) =
                    receive_founder_frame(&state, source, original, frame).await
                {
                    receiver = Some(original);
                    if let Some(action) = action {
                        source.notify_received(app, &state, Some(action)).await;
                    }
                }
            }
            Some(NativeChatMemberGossipEvent::RecoveryNeeded) => receiver = Some(original),
            None => {
                drop(original);
                source.notify_received(app, &state, None).await;
            }
        }
    }
}

async fn run_founder_receive<I: Send + 'static>(
    state: &AppState,
    source: &Source,
    mut receiver: FounderReceiver,
    mut input: I,
    run: impl Fn(FounderPreparation, &FounderReceiver, &I) -> FounderReceiveResult
        + Copy
        + Send
        + 'static,
) -> Option<(FounderReceiver, I, FounderReceiveResult)> {
    let deadline = SystemClock
        .monotonic_ms()
        .saturating_add(FOUNDER_RECEIVE_RETRY_MS);
    let mut scope = source.negative();
    loop {
        let Some((permit, preparation)) = capture_founder_frame(state, source).await else {
            return Some((receiver, input, Err(V2MemberSaveRefusal::Busy)));
        };
        let local = source.member_work().clone();
        let (next_receiver, next_input, result) = tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            let result = run(preparation, &receiver, &input);
            if let Ok(NativeTextFounderFrameOutcome::Retained(receipt)) = &result {
                note_founder_archive_change(&local, receipt);
            }
            (receiver, input, result)
        })
        .await
        .ok()?;
        receiver = next_receiver;
        input = next_input;
        if !matches!(result, Err(V2MemberSaveRefusal::Busy))
            || SystemClock.monotonic_ms() >= deadline
        {
            #[cfg(test)]
            if let Err(ref refusal) = result {
                eprintln!("native founder receive stopped: {refusal:?}");
            }
            return Some((receiver, input, result));
        }
        // The original input still owns its carrier scope. The local permit
        // and the refused physical preparation have both returned.
        #[cfg(test)]
        state.native_text.founder_receive_busy.notify_one();
        tokio::select! {
            biased;
            _ = scope.cancelled() => return None,
            _ = SystemClock.sleep(FOUNDER_CAPTURE_RETRY_DELAY) => {},
        }
    }
}

async fn receive_founder_frame(
    state: &AppState,
    source: &Arc<Source>,
    mut receiver: FounderReceiver,
    mut frame: NativeChatMemberFrame,
) -> Option<(FounderReceiver, Option<ActionId>)> {
    let mut remaining = DEPENDENCIES_PER_FRAME;
    loop {
        let (original, original_frame, next) = run_founder_receive(
            state,
            source,
            receiver,
            frame,
            move |prepared, receiver, frame| {
                if remaining > 0 {
                    prepared.receive_founder_member_with_recovery(receiver, frame)
                } else {
                    prepared
                        .receive_founder_member(receiver, frame)
                        .map(|received| {
                            received.map_or(
                                NativeTextFounderFrameOutcome::Unresolved,
                                NativeTextFounderFrameOutcome::Retained,
                            )
                        })
                }
            },
        )
        .await?;
        receiver = original;
        frame = original_frame;

        let exchange = match next {
            Ok(NativeTextFounderFrameOutcome::Retained(receipt)) => {
                return Some((receiver, Some(receipt.projection_target)))
            }
            Ok(NativeTextFounderFrameOutcome::NeedsDependency(exchange)) => exchange,
            _ => break,
        };
        let (original, recovered) =
            recover_founder_chain(state, source, receiver, exchange, &mut remaining).await?;
        receiver = original;
        if !recovered {
            break;
        }
    }
    Some((receiver, None))
}

async fn recover_founder_chain(
    state: &AppState,
    source: &Arc<Source>,
    mut receiver: FounderReceiver,
    first: NativeTextFounderDependencyExchange,
    remaining: &mut usize,
) -> Option<(FounderReceiver, bool)> {
    let mut next = Some(first);
    let mut waiting = Vec::with_capacity(*remaining);
    loop {
        if let Some(exchange) = next.take() {
            let Some(budget) = remaining.checked_sub(1) else {
                break;
            };
            *remaining = budget;
            let Ok(Ok(response)) =
                tokio::time::timeout(DEPENDENCY_TIMEOUT, exchange.receive()).await
            else {
                break;
            };
            waiting.push(response);
        }
        let Some(response) = waiting.pop() else {
            return Some((receiver, true));
        };
        let may_follow = *remaining > 0;
        let (original, response, result) = run_founder_receive(
            state,
            source,
            receiver,
            response,
            move |prepared, receiver, response| {
                prepared.recover_founder_dependency_with_followup(receiver, response, may_follow)
            },
        )
        .await?;
        receiver = original;
        match result {
            Ok(NativeTextFounderFrameOutcome::Retained(_)) => {}
            Ok(NativeTextFounderFrameOutcome::NeedsDependency(exchange)) => {
                waiting.push(response);
                next = Some(exchange);
            }
            _ => break,
        }
    }
    Some((receiver, false))
}

fn note_founder_archive_change(
    local: &server_table::NativeMemberWork,
    receipt: &NativeTextFounderReceived,
) {
    if receipt.action != receipt.projection_target {
        if let Some(settled) = receipt.archived {
            local.note_archive_change(receipt.action, receipt.projection_target, settled);
        }
    }
}

pub(super) async fn serve_founder_dependencies<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: &Arc<Source>,
) {
    let mut scope = source.negative();
    let channel = source.channel();
    loop {
        tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            _ = SystemClock.sleep(REOPEN_DELAY) => {},
        }
        let state = app.state::<AppState>();
        let local = source.member_work();
        let Some(permit) = local.foreground(scope.clone()).await else {
            return;
        };
        let Some(preparation) = source.capture(&state).await else {
            continue;
        };
        let caller = scope.clone();
        let opened = tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            preparation.attach_founder_dependency_provider(channel, caller)
        })
        .await;
        let Ok(Ok(mut provider)) = opened else {
            continue;
        };
        while let Some(request) = provider.next().await {
            let Some(permit) = local.foreground(scope.clone()).await else {
                return;
            };
            let Some(preparation) = source.capture(&state).await else {
                continue;
            };
            let processed = tauri::async_runtime::spawn_blocking(move || {
                let _permit = permit;
                let _ = preparation.serve_founder_dependency_request(&provider, request);
                provider
            })
            .await;
            let Ok(next) = processed else {
                break;
            };
            provider = next;
        }
    }
}

pub(super) async fn open(
    state: &AppState,
    source: &Arc<Source>,
) -> Option<NativeTextJoinedMemberReceiver> {
    let channel = source.channel();
    let prepared = with_row(state, source, move |selected, row, _, scope| {
        selected.prepare_member_ingress(&row, channel, scope).ok()
    })
    .await?;
    prepared.open().await.ok()
}

pub(super) async fn run<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: &Arc<Source>,
    initial: Option<NativeTextJoinedMemberReceiver>,
    _payment: NativeUiInvocationMetadata,
) {
    let local = source.member_work();
    let mut scope = source.negative();
    let mut receiver = initial;
    loop {
        let state = app.state::<AppState>();
        let next = if let Some(ingress) = receiver.as_mut() {
            tokio::select! {
                biased;
                _ = scope.cancelled() => break,
                event = ingress.next() => event,
            }
        } else {
            tokio::select! {
                biased;
                _ = scope.cancelled() => break,
                _ = SystemClock.sleep(REOPEN_DELAY) => {}
            }
            receiver = open(&state, source).await;
            continue;
        };
        match next {
            Some(NativeChatMemberGossipEvent::Frame(frame)) => {
                let action = tokio::select! {
                    biased;
                    _ = scope.cancelled() => break,
                    action = receive(&state, source, frame) => action,
                };
                if let Some(action) = action {
                    source.notify_received(app, &state, Some(action)).await;
                }
            }
            Some(NativeChatMemberGossipEvent::RecoveryNeeded) => {}
            None => {
                receiver = None;
                let retired = local
                    .received
                    .try_lock()
                    .ok()
                    .and_then(|mut slot| slot.take());
                drop(retired);
                source.notify_received(app, &state, None).await;
            }
        }
    }
    let retired = local
        .received
        .try_lock()
        .ok()
        .and_then(|mut slot| slot.take());
    drop(retired);
}

async fn receive(
    state: &AppState,
    source: &Arc<Source>,
    frame: NativeChatMemberFrame,
) -> Option<ActionId> {
    if !prepare_ledger(state, source).await {
        return None;
    }
    let Some((original, status)) = with_row(state, source, move |selected, _, local, scope| {
        let mut slot = local.received.lock().ok()?;
        let ledger = slot.as_deref_mut()?;
        let (original, status) = selected.receive_member_frame(ledger, &frame, &scope).ok()?;
        if status == V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
            archive_original(&selected, ledger, &original, local, &scope);
        }
        Some((original, status))
    })
    .await
    else {
        return None;
    };
    let original = Arc::new(original);
    match status {
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid) => {
            Some(original.projection_target_data())
        }
        V2OrdinaryReceiveStatus::WaitingForDependencies(missing) => {
            recover_dependencies(state, source, &original, missing).await;
            let target = original.projection_target_data();
            (apply_and_archive(state, source, original).await?
                == V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
            .then_some(target)
        }
        V2OrdinaryReceiveStatus::Retained(_) => None,
    }
}

enum DependencyWork {
    Resolve(Arc<NativeTextJoinedReceivedOriginal>, ActionId),
    Retry(Arc<NativeTextJoinedReceivedOriginal>),
}

async fn recover_dependencies(
    state: &AppState,
    source: &Arc<Source>,
    child: &Arc<NativeTextJoinedReceivedOriginal>,
    missing: Vec<ActionId>,
) {
    let mut pending = Vec::with_capacity(2 * DEPENDENCIES_PER_FRAME);
    pending.extend(
        missing
            .into_iter()
            .take(DEPENDENCIES_PER_FRAME)
            .rev()
            .map(|id| DependencyWork::Resolve(child.clone(), id)),
    );
    let mut remaining = DEPENDENCIES_PER_FRAME;
    while let Some(work) = pending.pop() {
        match work {
            DependencyWork::Resolve(child, action) => {
                let Some(next) = remaining.checked_sub(1) else {
                    return;
                };
                remaining = next;
                let Some(parent) = recover_dependency(state, source, &child, action).await else {
                    continue;
                };
                let Some(V2OrdinaryReceiveStatus::WaitingForDependencies(missing)) =
                    apply_and_archive(state, source, parent.clone()).await
                else {
                    continue;
                };
                let queued = pending
                    .iter()
                    .filter(|work| matches!(work, DependencyWork::Resolve(..)))
                    .count();
                if missing.len() + queued <= remaining {
                    // This also keeps the original parent's reply scope held
                    // until its dependencies have returned and been checked.
                    pending.push(DependencyWork::Retry(parent.clone()));
                    pending.extend(
                        missing
                            .into_iter()
                            .rev()
                            .map(|id| DependencyWork::Resolve(parent.clone(), id)),
                    );
                }
            }
            DependencyWork::Retry(parent) => {
                let _ = apply_and_archive(state, source, parent).await;
            }
        }
    }
}

async fn prepare_ledger(state: &AppState, source: &Arc<Source>) -> bool {
    let local = source.member_work();
    if local.received.try_lock().is_ok_and(|slot| {
        slot.as_ref()
            .is_some_and(|ledger| ledger.is_current_cached())
    }) {
        return true;
    }
    let channel = source.channel();
    with_row(state, source, move |selected, row, local, scope| {
        let mut slot = local.received.lock().ok()?;
        *slot = None;
        let ledger = selected.prepare_member_ledger(&row, channel, scope).ok()?;
        *slot = Some(Box::new(ledger));
        Some(())
    })
    .await
    .is_some()
}

async fn recover_dependency(
    state: &AppState,
    source: &Arc<Source>,
    child: &Arc<NativeTextJoinedReceivedOriginal>,
    action: ActionId,
) -> Option<Arc<NativeTextJoinedReceivedOriginal>> {
    let local_result = with_row(state, source, move |selected, _, local, scope| {
        let mut slot = local.received.lock().ok()?;
        selected
            .replay_saved_member_dependency(slot.as_deref_mut()?, action, &scope, &mut OsCryptoRng)
            .ok()
    })
    .await;
    // Consult the carrier when this exact original is absent from selected
    // local storage. A local refusal leaves the child waiting.
    if !matches!(local_result, Some(None)) {
        return None;
    }
    let original = child.clone();
    let exchange = with_row(state, source, move |selected, _, _, scope| {
        selected
            .prepare_received_dependency_exchange(&original, action, scope, &mut OsCryptoRng)
            .ok()
    })
    .await?;
    let response = tokio::time::timeout(DEPENDENCY_TIMEOUT, exchange.receive())
        .await
        .ok()?
        .ok()?;
    with_row(state, source, move |selected, row, _, scope| {
        selected
            .open_dependency_response(&row, response, &scope)
            .ok()
            .flatten()
    })
    .await
    .map(Arc::new)
}

async fn apply_and_archive(
    state: &AppState,
    source: &Arc<Source>,
    original: Arc<NativeTextJoinedReceivedOriginal>,
) -> Option<V2OrdinaryReceiveStatus> {
    with_row(state, source, move |selected, _, local, scope| {
        let mut slot = local.received.lock().ok()?;
        let ledger = slot.as_deref_mut()?;
        let status = selected
            .project_received_member(ledger, &original, &scope)
            .ok()?;
        if status == V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
            archive_original(&selected, ledger, &original, local, &scope);
        }
        Some(status)
    })
    .await
}

fn archive_original(
    selected: &NativeTextJoinedSelectionPreparation<OsCryptoRng>,
    ledger: &NativeTextJoinedMemberLedger,
    original: &NativeTextJoinedReceivedOriginal,
    local: &server_table::NativeMemberWork,
    scope: &RequestCancellation,
) {
    let result = selected.archive_received_member(ledger, original, scope, &mut OsCryptoRng);
    let action = original.action_id_data();
    let target = original.projection_target_data();
    if action != target {
        match result {
            Ok(Some(_)) => local.note_archive_change(action, target, true),
            Err(_) => local.note_archive_change(action, target, false),
            Ok(None) => {}
        }
    }
}

async fn with_row<T: Send + 'static>(
    state: &AppState,
    source: &Arc<Source>,
    perform: impl FnOnce(
            NativeTextJoinedSelectionPreparation<OsCryptoRng>,
            NativeTextJoinedRowSelection,
            &server_table::NativeMemberWork,
            RequestCancellation,
        ) -> Option<T>
        + Send
        + 'static,
) -> Option<T> {
    let local = source.member_work().clone();
    let scope = source.negative();
    let guard = local.foreground(scope.clone()).await?;
    let (original, selected) = source.capture_joined(state).await?;
    let source = source.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _payment = source;
        let local = local;
        let _guard = guard;
        perform(original, selected, &local, scope)
    })
    .await
    .ok()
    .flatten()
}
