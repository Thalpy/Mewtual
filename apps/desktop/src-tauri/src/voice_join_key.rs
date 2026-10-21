//! Selected V2 bootstrap only. Fixed public metadata crosses ordinary IPC; no private key,
//! confirmation plaintext, SDP or ICE does. Byte-bearing action IPC remains a separate boundary.
use super::*;
use voice_authority::{JoinKeyAdmission, JoinKeyPublication, PublicationPlan};

fn executor(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("invalid voice key executor".into());
    }
    hex::decode(value)
        .map_err(|_| "invalid voice key executor".to_string())?
        .try_into()
        .map_err(|_| "invalid voice key executor".into())
}

pub(super) async fn admit_inner(
    state: &AppState,
    owner: &str,
    executor_id: &str,
) -> Result<JoinKeyAdmission, String> {
    let executor = executor(executor_id)?;
    let binding = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .owner(owner)
        .ok_or("voice owner changed")?
        .binding;
    check_voice_binding(state, &binding).await?;
    let _commit = require_ui_session_generation(state, binding.generation).await?;
    if !voice_binding_current(state, &binding).await {
        return Err("voice server changed".into());
    }
    let mut authority = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Lock registration publishes its atomic generation before it can acquire Coordinator.
    check_voice_activation_generation(state, binding.generation)?;
    authority.admit_join_key(owner, executor)
}

#[tauri::command]
pub(crate) async fn admit_voice_join_key(
    state: State<'_, AppState>,
    owner: String,
    executor: String,
) -> Result<JoinKeyAdmission, String> {
    require_unlocked_session(&state).await?;
    admit_inner(&state, &owner, &executor).await
}

pub(super) async fn publish_inner(
    state: &AppState,
    owner: &str,
    executor_id: &str,
    permit: &str,
    point: [u8; 65],
) -> Result<JoinKeyPublication, String> {
    let executor = executor(executor_id)?;
    let binding = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .owner(owner)
        .ok_or("voice owner changed")?
        .binding;
    check_voice_binding(state, &binding).await?;
    // Owned equivalent of require_ui_session_generation. This guard travels with the admitted
    // actor work; canceling the invoke must never let lock complete ahead of that publication.
    let commit = state.ui_session_commit.clone().lock_owned().await;
    require_unlocked_session(state).await?;
    let actor = {
        let servers = state.servers.lock().await;
        let entry = servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .ok_or("voice server changed")?;
        entry.actor.clone()
    };
    let plan = {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        check_voice_activation_generation(state, binding.generation)?;
        authority.prepare_join_key_publication(owner, executor, permit, point)?
    };
    match plan {
        PublicationPlan::Accepted(reply) => Ok(reply),
        PublicationPlan::Start(mut work) => {
            let authority = state.voice_authority.clone();
            // There is no await between claiming the bounded row and transferring it to this
            // native task. At most one publication exists per row; commit serializes actor work.
            tokio::spawn(async move {
                let result = actor
                    .start_voice_endpoint(work.grant(), work.join(), work.point())
                    .await;
                let reply = authority
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .finish_join_key_publication(&mut work, result.is_ok());
                // Affine grant cleanup precedes returning commit and happens outside Coordinator.
                drop(work);
                drop(commit);
                result.map_err(|e| e.to_string())?;
                reply
            })
            .await
            .map_err(|_| "voice key publication worker stopped".to_string())?
        }
    }
}

#[tauri::command]
pub(crate) async fn publish_voice_join_key(
    state: State<'_, AppState>,
    owner: String,
    executor: String,
    permit: String,
    public_key: String,
) -> Result<JoinKeyPublication, String> {
    require_unlocked_session(&state).await?;
    if public_key.len() != 130
        || !public_key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("invalid voice public key".into());
    }
    let point: [u8; 65] = hex::decode(public_key)
        .map_err(|_| "invalid voice public key")?
        .try_into()
        .map_err(|_| "invalid voice public key")?;
    publish_inner(&state, &owner, &executor, &permit, point).await
}

/// Exact cleanup is permitted while locked. It cannot create engine/network authority and
/// confirms completion only after this generation's actual native publication has drained.
#[tauri::command]
pub(crate) async fn settle_voice_join_key(
    state: State<'_, AppState>,
    owner: String,
    executor_id: String,
) -> Result<(), String> {
    let executor = executor(&executor_id)?;
    settle_inner(&state, &owner, executor).await
}

pub(super) async fn settle_inner(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<(), String> {
    let pending = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .settle_join_key(owner, executor)?;
    if let Some(mut drained) = pending {
        // One retained state notification, independent of an IPC waiter's lifetime. No timer,
        // repeated actor request or speculative engine-credit refund is needed to await it.
        while !*drained.borrow_and_update() {
            drained
                .changed()
                .await
                .map_err(|_| "voice key settlement owner stopped")?;
        }
    }
    Ok(())
}
