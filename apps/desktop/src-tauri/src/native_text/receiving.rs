//! One bounded receiver on the installed joined row. Renderer history remains
//! on its separate own-receipt route until shared archival selection mounts.
use super::{connected::Source, *};
use catcoms_app::store::{
    NativeTextJoinedMemberReceiver, NativeTextJoinedReceivedOriginal, NativeTextJoinedRowSelection,
    NativeTextJoinedSelectionPreparation, NativeUiInvocationMetadata,
};
use catcoms_net::{NativeChatMemberFrame, NativeChatMemberGossipEvent};
use catcoms_replication::chat::{ActionId, ActionState, V2OrdinaryReceiveStatus};
use catcoms_rt::RequestCancellation;
use std::time::Duration;

const REOPEN_DELAY: Duration = Duration::from_secs(1);
const DEPENDENCY_TIMEOUT: Duration = Duration::from_secs(2);
const DEPENDENCIES_PER_FRAME: usize = 4;

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
    let Some(local) = source.member_work() else {
        return;
    };
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
        selected
            .receive_member_frame(slot.as_deref_mut()?, &frame, &scope)
            .ok()
    })
    .await
    else {
        return None;
    };
    let original = Arc::new(original);
    match status {
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid) => {
            let target = original.projection_target_data();
            archive_original(state, source, original).await;
            Some(target)
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
    let Some(local) = source.member_work() else {
        return false;
    };
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
    let candidate = original.clone();
    let result = with_row(state, source, move |selected, _, local, scope| {
        let mut slot = local.received.lock().ok()?;
        selected
            .project_received_member(slot.as_deref_mut()?, &candidate, &scope)
            .ok()
    })
    .await;
    if matches!(
        result,
        Some(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    ) {
        let target = original.projection_target_data();
        archive_original(state, source, original).await;
        return Some(target);
    }
    None
}

async fn archive_original(
    state: &AppState,
    source: &Arc<Source>,
    original: Arc<NativeTextJoinedReceivedOriginal>,
) {
    let _ = with_row(state, source, move |selected, _, local, scope| {
        let slot = local.received.lock().ok()?;
        selected
            .archive_received_member(slot.as_deref()?, &original, &scope, &mut OsCryptoRng)
            .ok()?;
        Some(())
    })
    .await;
}

async fn with_row<T: Send + 'static>(
    state: &AppState,
    source: &Arc<Source>,
    perform: impl FnOnce(
            NativeTextJoinedSelectionPreparation<OsCryptoRng>,
            NativeTextJoinedRowSelection,
            &server_table::JoinedMemberWork,
            RequestCancellation,
        ) -> Option<T>
        + Send
        + 'static,
) -> Option<T> {
    let local = source.member_work()?.clone();
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
