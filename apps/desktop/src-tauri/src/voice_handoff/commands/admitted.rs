//! Fixed public metadata for the admitted physical bootstrap ONLY. There is no
//! operation31/Ready/RTC producer or old selected-key conversion in these entries.
use super::*;

pub(super) async fn setup_admitted_inner(
    state: &AppState,
    host: &Host,
    owner: String,
    executor: String,
    rng: &mut impl CryptoRngCore,
) -> Result<SetupReply, String> {
    let id = identity(&owner, &executor)?;
    if owner.capacity() > 128 || executor.capacity() > 64 {
        return Err(REFUSED.into());
    }
    let port = host.port().map_err(|_| REFUSED.to_string())?;
    let status = bootstrap::ensure_admitted(state, &owner, id, rng)
        .await
        .map_err(|_| REFUSED.to_string())?;
    let projection = state
        .voice_channels
        .project_admitted(&owner, id)
        .map_err(|_| REFUSED.to_string())?;
    SetupReply::new(status, owner, executor, port, projection)
}

pub(super) async fn settle_admitted_inner(
    state: &AppState,
    owner: String,
    executor: String,
) -> Result<(), String> {
    let id = identity(&owner, &executor)?;
    drained(bootstrap::settle_admitted_setup(state, &owner, id).await)
}
pub(super) async fn collect_admitted_inner(
    state: &AppState,
    owner: String,
    executor: String,
) -> Result<(), String> {
    let id = identity(&owner, &executor)?;
    drained(bootstrap::collect_admitted_channel(state, &owner, id).await)
}
