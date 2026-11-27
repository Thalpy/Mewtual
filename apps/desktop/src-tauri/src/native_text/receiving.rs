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
const FOUNDER_ARCHIVE_RECONCILE_MS: u64 = 1_000;

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
    let mut next_archive_check = SystemClock.monotonic_ms();
    let mut archive_cursor = 0;
    loop {
        let state = app.state::<AppState>();
        let now = SystemClock.monotonic_ms();
        if now >= next_archive_check {
            next_archive_check = now.saturating_add(FOUNDER_ARCHIVE_RECONCILE_MS);
            if let Some(target) =
                reconcile_founder_archive_change(&state, source, &mut archive_cursor).await
            {
                source.notify_received(app, &state, Some(target)).await;
            }
        }
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
            _ = SystemClock.sleep(Duration::from_millis(
                next_archive_check.saturating_sub(SystemClock.monotonic_ms()),
            )) => {
                receiver = Some(original);
                continue;
            }
            event = original.next() => event,
        };
        match event {
            Some(NativeChatMemberGossipEvent::Frame(frame)) => {
                if let Some((original, action)) =
                    receive_founder_frame(app, &state, source, original, frame).await
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

async fn reconcile_founder_archive_change(
    state: &AppState,
    source: &Source,
    cursor: &mut usize,
) -> Option<ActionId> {
    let local = source.member_work().clone();
    let pending = local.pending_archive_changes();
    let index = (0..pending.len())
        .map(|offset| (*cursor + offset) % pending.len())
        .find(|index| pending[*index].is_some())?;
    let permit = local.try_background()?;
    let preparation = source.capture(state).await?;
    let change = pending[index]?;
    *cursor = (index + 1) % pending.len();
    let channel = source.channel();
    let caller = source.negative();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let selected = preparation.selected_founder_archive_target(channel, change.action, &caller);
        if caller.is_cancelled() {
            return None;
        }
        selected
            .ok()
            .and_then(|target| local.reconcile_archive_change(change, target))
            .or_else(|| local.take_archive_invalidation(change))
    })
    .await
    .ok()
    .flatten()
}

async fn run_founder_receive<I: Send + 'static>(
    state: &AppState,
    source: &Source,
    mut receiver: FounderReceiver,
    mut input: I,
    run: impl Fn(
            FounderPreparation,
            &FounderReceiver,
            &I,
            &server_table::NativeMemberWork,
        ) -> FounderReceiveResult
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
            let result = run(preparation, &receiver, &input, &local);
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

async fn receive_founder_frame<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
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
            move |prepared, receiver, frame, local| {
                if remaining > 0 {
                    prepared.receive_founder_member_with_recovery(receiver, frame, local)
                } else {
                    prepared
                        .receive_founder_member(receiver, frame, local)
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
                // The notice invalidates cached text. A fresh read still
                // requires the exact archive selection for mounted rows.
                return Some((receiver, Some(receipt.projection_target)));
            }
            Ok(NativeTextFounderFrameOutcome::NeedsDependency(exchange)) => exchange,
            _ => break,
        };
        let (original, recovered) =
            recover_founder_chain(app, state, source, receiver, exchange, &mut remaining).await?;
        receiver = original;
        if !recovered {
            break;
        }
    }
    Some((receiver, None))
}

async fn recover_founder_chain<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
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
            move |prepared, receiver, response, local| {
                prepared
                    .recover_founder_dependency_with_followup(receiver, response, may_follow, local)
            },
        )
        .await?;
        receiver = original;
        match result {
            Ok(NativeTextFounderFrameOutcome::Retained(receipt)) => {
                source
                    .notify_received(app, state, Some(receipt.projection_target))
                    .await;
            }
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
    if receipt.archived != Some(false) {
        local.settle_archive_change(receipt.action, receipt.projection_target);
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
                    action = receive(app, &state, source, frame) => action,
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

async fn receive<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    source: &Arc<Source>,
    frame: NativeChatMemberFrame,
) -> Option<ActionId> {
    if !prepare_ledger(state, source).await {
        return None;
    }
    let channel = source.channel();
    let Some((original, status)) = with_row(state, source, move |selected, row, local, scope| {
        let mut slot = local.received.lock().ok()?;
        let ledger = slot.as_deref_mut()?;
        let original = selected
            .open_member_frame(&row, channel, &frame, &scope)
            .ok()?;
        let status = apply_original(&selected, ledger, &original, local, &scope)?;
        Some((original, status))
    })
    .await
    else {
        return None;
    };
    let original = Arc::new(original);
    match status {
        JoinedApply::Ordinary(V2OrdinaryReceiveStatus::WaitingForDependencies(missing)) => {
            recover_dependencies(app, state, source, &original, missing).await;
            let target = original.projection_target_data();
            apply_and_archive(state, source, original)
                .await?
                .invalidation(target)
        }
        status => status.invalidation(original.projection_target_data()),
    }
}

enum DependencyWork {
    Resolve(Arc<NativeTextJoinedReceivedOriginal>, ActionId),
    Retry(Arc<NativeTextJoinedReceivedOriginal>),
}

async fn recover_dependencies<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
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
                let Some(status) = apply_and_archive(state, source, parent.clone()).await else {
                    continue;
                };
                let JoinedApply::Ordinary(V2OrdinaryReceiveStatus::WaitingForDependencies(missing)) =
                    status
                else {
                    if let Some(target) = status.invalidation(parent.projection_target_data()) {
                        source.notify_received(app, state, Some(target)).await;
                    }
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
                let target = parent.projection_target_data();
                if let Some(target) = apply_and_archive(state, source, parent)
                    .await
                    .and_then(|status| status.invalidation(target))
                {
                    source.notify_received(app, state, Some(target)).await;
                }
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

enum JoinedApply {
    Ordinary(V2OrdinaryReceiveStatus),
    ArchiveUnconfirmed,
}

impl JoinedApply {
    fn invalidation(self, target: ActionId) -> Option<ActionId> {
        // Both outcomes invalidate old text. ArchiveUnconfirmed leaves the
        // exact reservation in place so targeted reads cannot publish it.
        matches!(
            self,
            Self::Ordinary(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
                | Self::ArchiveUnconfirmed
        )
        .then_some(target)
    }
}

async fn apply_and_archive(
    state: &AppState,
    source: &Arc<Source>,
    original: Arc<NativeTextJoinedReceivedOriginal>,
) -> Option<JoinedApply> {
    with_row(state, source, move |selected, _, local, scope| {
        let mut slot = local.received.lock().ok()?;
        let ledger = slot.as_deref_mut()?;
        apply_original(&selected, ledger, &original, local, &scope)
    })
    .await
}

fn apply_original(
    selected: &NativeTextJoinedSelectionPreparation<OsCryptoRng>,
    ledger: &mut NativeTextJoinedMemberLedger,
    original: &NativeTextJoinedReceivedOriginal,
    local: &server_table::NativeMemberWork,
    scope: &RequestCancellation,
) -> Option<JoinedApply> {
    let action = original.action_id_data();
    let target = original.projection_target_data();
    let newly_reserved = local.reserve_archive_change(action, target)?;
    let status = selected.stage_received_member(ledger, original, scope).ok();
    if status != Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid)) {
        if newly_reserved {
            local.settle_archive_change(action, target);
        }
        return status.map(JoinedApply::Ordinary);
    }
    match selected.archive_received_member(ledger, original, scope, &mut OsCryptoRng) {
        Ok(None) => {
            // Only a layout with no archive can publish from its live ledger.
            local.settle_archive_change(action, target);
            selected
                .project_received_member(ledger, original, scope)
                .ok()
                .map(JoinedApply::Ordinary)
        }
        Ok(Some(_)) => {
            local.settle_archive_change(action, target);
            status.map(JoinedApply::Ordinary)
        }
        Err(_) => Some(JoinedApply::ArchiveUnconfirmed),
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
