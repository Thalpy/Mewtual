//! Fixed admitted-profile native entry. These commands never reinterpret an
//! Existing owner, and activation alone supplies no key/agreement/Ready/RTC grant.
use crate::voice_call_activation::Handle;
use crate::*;
use catcoms_sync::voice_v2::call_activation::ActivationRpcPermit;

pub(super) async fn prepare_inner(
    state: &AppState,
    controller: String,
    server: u64,
    room: String,
) -> Result<String, String> {
    collect_retired(state);
    let room_id = room.parse::<u128>().map_err(|_| "invalid voice room")?;
    if room_id.to_string() != room {
        return Err("invalid voice room".into());
    }
    let generation = unlocked_ui_session_generation(state).await?;
    let (actor, instance, group) = {
        let servers = state.servers.lock().await;
        let entry = servers
            .get(&server)
            .ok_or("voice server is no longer open")?;
        (entry.actor.clone(), entry.instance, entry.group_id.clone())
    };
    let context = actor.voice_v2_context(room_id).await?;
    let binding = voice_authority::Binding {
        generation,
        server,
        instance,
        group,
        scope: context.scope,
    };
    // voice_runtime itself reads Coordinator. Resolve it (and the sanctioned
    // clock callback) before taking that mutex; argument evaluation under the
    // final guard would recursively lock the same non-reentrant mutex.
    let now = voice_runtime(state).clock().monotonic_ms();
    let _commit = require_ui_session_generation(state, generation).await?;
    if !voice_binding_current(state, &binding).await {
        return Err("voice server changed".into());
    }
    let mut authority = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    check_voice_activation_generation(state, generation)?;
    authority.prepare(
        voice_authority::CandidateKind::AdmittedCall {
            controller,
            binding,
        },
        now,
    )
}

fn retained(state: &AppState, candidate: &str) -> Option<Handle> {
    state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admitted_activation(candidate)
}

pub(super) async fn activate_inner(
    state: &AppState,
    candidate: String,
) -> Result<VoiceActivation, String> {
    // Historical terminal recovery comes first, even after lock/End. This fixed
    // metadata cannot authorize a key or revive the call; every positive command
    // separately checks the original live activation and current UI generation.
    if let Some(handle) = retained(state, &candidate) {
        handle.drive_negative(state);
        return handle.reply().await.result();
    }
    let runtime = voice_runtime(state);
    let candidate_now = runtime.clock().monotonic_ms();
    let kind = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .candidate(&candidate, candidate_now)
        .ok_or("voice candidate expired")?;
    let voice_authority::CandidateKind::AdmittedCall {
        controller,
        binding,
    } = kind
    else {
        return Err("this voice candidate has a different protocol".into());
    };
    let actor = {
        let servers = state.servers.lock().await;
        servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .ok_or("voice server changed")?
            .actor
            .clone()
    };
    let payment = ActivationRpcPermit::reserve(&runtime.resources(), binding.scope)
        .map_err(|e| e.to_string())?;
    let pair = actor
        .prepare_admitted_voice_call(binding.scope.room, payment)
        .await
        .map_err(|e| e.to_string())?;
    let handle = Handle::new(
        candidate.clone(),
        controller,
        binding,
        pair,
        runtime.clock(),
    )
    .map_err(|e| e.to_string())?;
    // Declare old custody before the task: on unwind, the task releases its UI
    // guard before the returned old Owner can run a heavy final destructor.
    let retired;
    let commit = state.ui_session_commit.clone().lock_owned().await;
    state.ui_session_gate().require_unlocked().await?;
    check_voice_activation_generation(state, handle.binding().generation)?;
    if !voice_binding_current(state, handle.binding()).await {
        return Err("voice server changed".into());
    }
    // A second invoke may have won while this inert Actor pair was prepared.
    // It cannot consume a new nonce, overwrite the original or create a runner.
    if let Some(original) = retained(state, &candidate) {
        drop(commit);
        drop(handle);
        original.drive_negative(state);
        return original.reply().await.result();
    }
    let task = handle.prepare_task(state, commit); // RAII BEFORE native CAS
    let admission_now = runtime.clock().monotonic_ms();
    let admission = {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        check_voice_activation_generation(state, handle.binding().generation)?;
        let mut nonce = [0; 32];
        OsCryptoRng.fill_bytes(&mut nonce);
        check_voice_activation_generation(state, handle.binding().generation)?;
        authority.retain_admitted_activation(&handle)?;
        authority.activate_admitted(&handle, nonce, admission_now)
    };
    match admission {
        Ok(old) => {
            retired = old;
        }
        Err(error) => {
            // The separately retained original latches the same rejection on
            // unpolled cancellation. Never leave a reply waiting for a runner.
            drop(task);
            return if retained(state, &candidate).is_some() {
                handle.drive_negative(state);
                handle.reply().await.result()
            } else {
                Err(error)
            };
        }
    }
    drop(tokio::spawn(task.with_retired(retired).run()));
    handle.reply().await.result()
}

#[tauri::command]
pub(crate) async fn prepare_admitted_voice_owner(
    state: State<'_, AppState>,
    controller: String,
    server: u64,
    room: String,
) -> Result<String, String> {
    require_unlocked_session(&state).await?;
    prepare_inner(&state, controller, server, room).await
}

#[tauri::command]
pub(crate) async fn activate_admitted_voice_owner(
    state: State<'_, AppState>,
    candidate: String,
) -> Result<VoiceActivation, String> {
    // Recovery of exact historical metadata remains possible after locking.
    // The first positive admission inside activate_inner still requires unlock.
    activate_inner(&state, candidate).await
}

#[tauri::command]
pub(crate) async fn acknowledge_admitted_voice_activation(
    state: State<'_, AppState>,
    owner: String,
) -> Result<(), String> {
    acknowledge_inner(&state, &owner).await
}

pub(super) async fn acknowledge_inner(state: &AppState, owner: &str) -> Result<(), String> {
    let handle = retained(state, owner).ok_or("voice activation is unavailable")?;
    handle.reply().await.acknowledge();
    // Forget remains gated on actual original Root/task/SDK return. Acknowledging
    // an active call does not refund its owners or remove its native authority.
    let detached = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .forget_admitted_activation(&handle);
    drop(detached);
    collect_retired(state);
    Ok(())
}

/// Bounded negative maintenance. Run before taking UI/Coordinator guards: every
/// returned task/row/old owner is actually destroyed outside both. Acknowledging
/// an active call is normal; later prepare/End must revisit its actual return.
pub(crate) fn collect_retired(state: &AppState) {
    for _ in 0..voice_call_activation::MAX_ACTIVATIONS {
        let detached = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_one_retired_admitted_join_key();
        if let Some(detached) = detached {
            drop(detached);
        } else {
            break;
        }
    }
    for _ in 0..voice_call_activation::MAX_ACTIVATIONS {
        let detached = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .collect_one_completed_admitted_publication();
        if let Some(detached) = detached {
            drop(detached);
        } else {
            break;
        }
    }
    for _ in 0..voice_call_activation::MAX_ACTIVATIONS {
        let detached = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .collect_one_acknowledged_admitted_activation();
        if let Some(detached) = detached {
            drop(detached);
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests;
