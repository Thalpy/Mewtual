//! Read-only projection of the actor's authenticated communication policy.
use super::*;
use catcoms_app::GroupMode;

pub(super) fn mode_name(mode: GroupMode) -> &'static str {
    match mode {
        GroupMode::LegacyUnverified => "legacy_unverified",
        GroupMode::PeerToPeer => "peer_to_peer",
        GroupMode::Dedicated => "dedicated",
    }
}

/// A preview proves the inviter signed this declaration. MLS governance is verified only when
/// the Welcome and sealed policy transfer arrive; this must not confer member route authority.
pub(super) fn declared_mode(invite: &catcoms_mls::InviteToken) -> &'static str {
    mode_name(
        invite
            .policy
            .as_ref()
            .map_or(GroupMode::LegacyUnverified, |policy| policy.mode()),
    )
}

#[tauri::command]
pub(super) async fn group_communication_mode(
    state: State<'_, AppState>,
    server: u64,
) -> Result<&'static str, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let (actor, instance) = actor_instance_of(&state, server)
        .await
        .map_err(|failure| failure.message())?;
    let mode = actor.group_mode().await?;
    let _session = require_ui_session_generation(&state, generation).await?;
    let servers = state.servers.lock().await;
    if servers
        .get(&server)
        .is_none_or(|entry| entry.instance != instance)
    {
        return Err("The group changed while reading its communication policy.".into());
    }
    Ok(mode_name(mode))
}
