//! Restore only the retained standing consent for this exact joined row.
//! Dial submission and passive matching are separate and both recheck the
//! current source before installing a live approval.
use super::*;
use std::time::Duration;

const REFRESH: Duration = Duration::from_secs(15);
const PEER_TURN: Duration = Duration::from_millis(500);

pub(super) async fn restore_founder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: &Arc<Source>,
) {
    let mut scope = source.negative();
    let mut wait = PEER_TURN;
    loop {
        tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            _ = SystemClock.sleep(wait) => {}
        }
        wait = REFRESH;
        let state = app.state::<AppState>();
        let Some(local) = source.member_work().try_background() else {
            wait = PEER_TURN;
            continue;
        };
        let Some((prepared, selection)) = source.capture_joined(&state).await else {
            continue;
        };
        let caller = scope.clone();
        let Ok(Ok(Some(attempt))) = tauri::async_runtime::spawn_blocking(move || {
            let _local = local;
            prepared.prepare_saved_founder_reconnect(&selection, caller)
        })
        .await
        else {
            continue;
        };
        let _ = attempt.connect(&state.endpoint_dials).await;
    }
}

pub(super) async fn restore<R: tauri::Runtime>(app: &tauri::AppHandle<R>, source: &Arc<Source>) {
    let mut scope = source.negative();
    let mut wait = PEER_TURN;
    loop {
        tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            _ = SystemClock.sleep(wait) => {}
        }
        wait = REFRESH;
        let state = app.state::<AppState>();
        let Some(local) = source.member_work().try_background() else {
            wait = PEER_TURN;
            continue;
        };
        let Some((prepared, selection)) = source.capture_joined(&state).await else {
            continue;
        };
        let caller = scope.clone();
        let Ok(Ok(saved)) = tauri::async_runtime::spawn_blocking(move || {
            let _local = local;
            prepared.read_member_contact_approvals(&selection, &caller)
        })
        .await
        else {
            continue;
        };
        if saved.is_empty() {
            continue;
        }
        let saved = Arc::new(saved);
        for index in 0..saved.len() {
            tokio::select! {
                biased;
                _ = scope.cancelled() => return,
                _ = SystemClock.sleep(PEER_TURN) => {}
            }
            let Some(local) = source.member_work().try_background() else {
                continue;
            };
            let Some((prepared, selection)) = source.capture_joined(&state).await else {
                continue;
            };
            let caller = scope.clone();
            let saved = saved.clone();
            let Ok(Ok(Some(attempt))) = tauri::async_runtime::spawn_blocking(move || {
                let _local = local;
                prepared.prepare_saved_member_contact(&selection, &saved, index, caller)
            })
            .await
            else {
                continue;
            };
            let _ = attempt.connect(&state.endpoint_dials).await;
        }
    }
}
