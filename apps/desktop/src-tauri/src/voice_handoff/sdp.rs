//! Authenticated storage-phase admission on the original key session. Version5 creates
//! no SDK, PC, Ready, crypto action or network permission. Those have separate consumers.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{RowId, SdpReceipt};
use super::requests::sdp::{Kind, Request};
use catcoms_sync::voice_v2::VoiceError;

pub(super) const REPLY_BYTES: usize = 170;
pub(super) async fn apply(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let _physical = incoming.hold_position()?;
    let result = prepare(context, route, &incoming).await;
    let mut reply = incoming.reply(
        BodyKind::Control,
        if result.is_ok() { REPLY_BYTES } else { 3 },
        None,
    )?;
    match result {
        Ok(receipt) => encode_into(receipt, reply.plain_mut()),
        Err(error) => reply
            .plain_mut()
            .copy_from_slice(&super::replies::refused(error)),
    }
    reply.seal()
}
pub(super) fn current<T>(
    context: &Context,
    route: &Route,
    apply: impl FnOnce(&super::transition::Admission<'_>) -> Result<T, VoiceError>,
) -> Result<T, Refused> {
    let live = context.upgrade().ok_or(Refused::Closed)?;
    factory::require_unlocked_now(&live, route.selected())?;
    let commit = live
        .commit
        .clone()
        .try_lock_owned()
        .map_err(|_| VoiceError::Capacity)?;
    let authority = match live.authority.try_lock() {
        Ok(authority) => authority,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity.into()),
    };
    factory::require_current_selection(live.gate(), &authority, route.selected())?;
    let admission = super::transition::Admission::new(&live, &authority, route, &commit);
    route
        .try_with_open(|| apply(&admission))
        .map_err(Into::into)
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<SdpReceipt, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::SdpPhase
        || incoming.kind() != BodyKind::Control
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    let row = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        live.channels
            .require_cleanup_route(route)
            .map_err(|_| Refused::Closed)?;
        live.ledger
            .resolve_sdp_parent(route.selected(), RowId::from_logical(incoming.logical())?)?
    };
    if let Kind::Release(proof) = request.kind {
        return row
            .release_sdp(route.session().identity(), &request, proof)
            .map_err(Into::into);
    }
    if let Some(receipt) = row.recover_sdp(route.session().identity(), &request)? {
        return Ok(receipt);
    }
    factory::preflight_binding(context, route).await?;
    let Some(mut preparation) = current(context, route, |admission| {
        row.begin_sdp(admission, &request)
    })?
    else {
        return row
            .recover_sdp(route.session().identity(), &request)?
            .ok_or(VoiceError::Capacity.into());
    };
    // Fixed native intent already owns this preparation. End cannot report final drainage
    // while its actual reservation is being acquired, refused, attached or destroyed.
    preparation.prepare()?;
    let result = current(context, route, |admission| {
        row.attach_sdp(admission, &request, &mut preparation)
    });
    drop(preparation); // Actual worker completion, never the reply waiter, publishes drainage.
    result
}
fn encode_into(receipt: SdpReceipt, bytes: &mut [u8]) {
    assert_eq!(bytes.len(), REPLY_BYTES);
    let mut position = 0;
    let mut put = |value: &[u8]| {
        bytes[position..position + value.len()].copy_from_slice(value);
        position += value.len();
    };
    put(&[5, receipt.state]);
    put(&receipt.row.logical());
    put(&receipt.action.serial.to_be_bytes());
    put(&receipt.action.version.to_be_bytes());
    put(&receipt.action.input_tag);
    put(&receipt.handshake);
    put(&receipt.key);
    put(&receipt.sequence.to_be_bytes());
    put(&receipt.proof);
    assert_eq!(position, REPLY_BYTES);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_sdp_storage_replies_match_browser_vectors_without_effect_permission() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("requests/sdp-vectors.json")).unwrap();
        for (state, vector) in vectors["replies"].as_array().unwrap().iter().enumerate() {
            let receipt = SdpReceipt {
                row: RowId::from_logical(
                    hex::decode(vectors["parentRow"].as_str().unwrap())
                        .unwrap()
                        .try_into()
                        .unwrap(),
                )
                .unwrap(),
                action: super::super::requests::Action {
                    serial: 9_007_199_254_740_993,
                    version: u64::MAX,
                    input_tag: [17; 32],
                },
                handshake: [18; 32],
                key: [19; 32],
                sequence: u64::MAX,
                proof: [20; 32],
                state: state as u8,
            };
            let mut bytes = [0; REPLY_BYTES];
            encode_into(receipt, &mut bytes);
            assert_eq!(hex::encode(bytes), vector["body"]);
        }
    }
}
