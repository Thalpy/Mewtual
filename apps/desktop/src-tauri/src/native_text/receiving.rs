//! One bounded receiver on the installed joined row, with guarded live
//! projection and archive selection under the same local permit.
use super::{connected::Source, *};
use catcoms_app::store::{
    NativeTextFounderMemberReceiver, NativeTextJoinedMemberLedger, NativeTextJoinedMemberReceiver,
    NativeTextJoinedReceivedOriginal, NativeTextJoinedRowSelection,
    NativeTextJoinedSelectionPreparation, NativeUiInvocationMetadata,
};
use catcoms_net::{NativeChatMemberFrame, NativeChatMemberGossipEvent};
use catcoms_replication::chat::{ActionId, ActionState, V2OrdinaryReceiveStatus};
use catcoms_rt::RequestCancellation;
use std::time::Duration;

const REOPEN_DELAY: Duration = Duration::from_secs(1);
const DEPENDENCY_TIMEOUT: Duration = Duration::from_secs(2);
const DEPENDENCIES_PER_FRAME: usize = 4;

pub(super) async fn run_founder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: &Arc<Source>,
) {
    let mut scope = source.negative();
    let channel = source.channel();
    let mut receiver: Option<NativeTextFounderMemberReceiver<OsCryptoRng>> = None;
    loop {
        let state = app.state::<AppState>();
        let Some(mut original) = receiver.take() else {
            tokio::select! {
                biased;
                _ = scope.cancelled() => return,
                _ = tokio::time::sleep(REOPEN_DELAY) => {},
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
            continue;
        };
        let event = tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            event = original.next() => event,
        };
        match event {
            Some(NativeChatMemberGossipEvent::Frame(frame)) => {
                let Some(permit) = source.member_work().foreground(scope.clone()).await else {
                    return;
                };
                let Some(preparation) = source.capture(&state).await else {
                    receiver = Some(original);
                    continue;
                };
                let output = tauri::async_runtime::spawn_blocking(move || {
                    let _permit = permit;
                    let action = preparation
                        .receive_founder_member(&original, &frame)
                        .ok()
                        .flatten();
                    (original, action)
                })
                .await;
                if let Ok((original, action)) = output {
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
                _ = tokio::time::sleep(REOPEN_DELAY) => {}
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
            for action in missing.into_iter().take(DEPENDENCIES_PER_FRAME) {
                recover_dependency(state, source, &original, action).await;
            }
            apply_and_archive(state, source, original).await
        }
        V2OrdinaryReceiveStatus::Retained(_) => None,
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
) {
    let local_result = with_row(state, source, move |selected, _, local, scope| {
        let mut slot = local.received.lock().ok()?;
        selected
            .replay_saved_member_dependency(slot.as_deref_mut()?, action, &scope, &mut OsCryptoRng)
            .ok()
    })
    .await;
    // Consult the carrier when this exact original is absent from the local
    // outbox. A local refusal leaves the child waiting.
    if !matches!(local_result, Some(None)) {
        return;
    }
    let original = child.clone();
    let Some(exchange) = with_row(state, source, move |selected, _, _, scope| {
        selected
            .prepare_received_dependency_exchange(&original, action, scope, &mut OsCryptoRng)
            .ok()
    })
    .await
    else {
        return;
    };
    let Ok(Ok(response)) = tokio::time::timeout(DEPENDENCY_TIMEOUT, exchange.receive()).await
    else {
        return;
    };
    let Some(original) = with_row(state, source, move |selected, row, _, scope| {
        selected
            .open_dependency_response(&row, response, &scope)
            .ok()
            .flatten()
    })
    .await
    else {
        return;
    };
    let _ = apply_and_archive(state, source, Arc::new(original)).await;
}

async fn apply_and_archive(
    state: &AppState,
    source: &Arc<Source>,
    original: Arc<NativeTextJoinedReceivedOriginal>,
) -> Option<ActionId> {
    with_row(state, source, move |selected, _, local, scope| {
        let mut slot = local.received.lock().ok()?;
        let ledger = slot.as_deref_mut()?;
        let status = selected
            .project_received_member(ledger, &original, &scope)
            .ok()?;
        if status != V2OrdinaryReceiveStatus::Retained(ActionState::Valid) {
            return None;
        }
        archive_original(&selected, ledger, &original, local, &scope);
        Some(original.projection_target_data())
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
