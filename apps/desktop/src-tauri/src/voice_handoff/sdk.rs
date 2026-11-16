//! Authenticated SDK custody only. This obligation precedes future SDK permission, and
//! remains independently released after native storage closes or its HTTP Session ends.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation, PhysicalHold};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{RowId, SdkReceipt};
use super::requests::sdk::{Kind, Request};
use catcoms_sync::voice_v2::VoiceError;
pub(super) const REPLY_BYTES: usize = 210;
pub(super) async fn apply(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let physical = incoming.hold_position()?;
    let result = prepare(context, route, &incoming, &physical).await;
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
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
    physical: &PhysicalHold,
) -> Result<SdkReceipt, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::SdkCustody
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
    if let Kind::Release { generation, proof } = request.kind {
        return row
            .release_sdk(route.session().identity(), &request, generation, proof)
            .map_err(Into::into);
    }
    if let Some(receipt) = row.recover_sdk(route.session().identity(), &request)? {
        return Ok(receipt);
    }
    factory::preflight_binding(context, route).await?;
    let Some(mut preparation) = super::sdp::current(context, route, |admission| {
        row.begin_sdk(admission, physical, route.session(), &request)
    })?
    else {
        return row
            .recover_sdk(route.session().identity(), &request)?
            .ok_or(VoiceError::Capacity.into());
    };
    preparation.allocate()?;
    let result = super::sdp::current(context, route, |admission| {
        row.attach_sdk(admission, &request, &mut preparation)
    });
    drop(preparation);
    result
}
fn encode_into(receipt: SdkReceipt, bytes: &mut [u8]) {
    assert_eq!(bytes.len(), REPLY_BYTES);
    let mut position = 0;
    let mut put = |value: &[u8]| {
        bytes[position..position + value.len()].copy_from_slice(value);
        position += value.len();
    };
    put(&[6, receipt.state]);
    put(&receipt.row.logical());
    put(&receipt.action.serial.to_be_bytes());
    put(&receipt.action.version.to_be_bytes());
    put(&receipt.action.input_tag);
    put(&receipt.handshake);
    put(&receipt.key);
    put(&receipt.sequence.to_be_bytes());
    put(&receipt.storage);
    put(&receipt.generation.to_be_bytes());
    put(&receipt.proof);
    assert_eq!(position, REPLY_BYTES);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_sdk_custody_replies_pin_original_storage_and_generation_without_rtc_authority() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("requests/sdk-vectors.json")).unwrap();
        for (index, vector) in vectors["replies"].as_array().unwrap().iter().enumerate() {
            let receipt = SdkReceipt {
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
                storage: [20; 32],
                generation: u64::MAX,
                proof: [21; 32],
                state: index as u8 + 1,
            };
            let mut body = [0; REPLY_BYTES];
            encode_into(receipt, &mut body);
            assert_eq!(hex::encode(body), vector["body"]);
        }
    }
}
