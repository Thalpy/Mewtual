//! Authenticated original-session retention of the existing browser key domain. This
//! separate opcode neither changes finite Claim/Release nor enables SDP/RTC operations.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{KeySessionReceipt, Purpose, RowId};
use super::requests::key_session::{Kind, Request};
use catcoms_sync::voice_v2::VoiceError;

pub(super) const REPLY_BYTES: usize = 234;
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
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<KeySessionReceipt, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::KeySession
        || incoming.kind() != BodyKind::Control
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let (row, clock, action, handshake, claim, kind) = {
        let request = Request::decode(
            incoming.logical(),
            incoming.plain().map_err(|_| VoiceError::Malformed)?,
        )?;
        let live = context.upgrade().ok_or(Refused::Closed)?;
        live.channels
            .require_cleanup_route(route)
            .map_err(|_| Refused::Closed)?;
        let row = live.ledger.resolve(
            route.selected(),
            Purpose::Derive(request.selector),
            RowId::from_logical(incoming.logical())?,
        )?;
        let clock = live
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .runtime()
            .clock();
        (
            row,
            clock,
            request.action,
            request.handshake,
            request.claim,
            request.kind,
        )
    };
    if let Kind::Release(proof) = kind {
        return row
            .release_key_session(route.session().identity(), action, handshake, claim, proof)
            .map_err(Into::into);
    }
    // This exact historical lookup creates nothing. In particular it remains available
    // when an accepted adoption's reply was lost beyond old Finish expiry or owner End.
    if let Some(receipt) =
        row.recover_key_session(route.session().identity(), action, handshake, claim)?
    {
        return Ok(receipt);
    }
    factory::preflight_binding(context, route).await?;
    let live = context.upgrade().ok_or(Refused::Closed)?;
    factory::require_unlocked_now(&live, route.selected())?;
    let mono = clock.monotonic_ms();
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
        .try_with_open(|| row.adopt_key_session(&admission, action, handshake, claim, mono))
        .map_err(Into::into)
}
fn encode_into(receipt: KeySessionReceipt, bytes: &mut [u8]) {
    assert_eq!(bytes.len(), REPLY_BYTES, "original paid key custody reply");
    let mut position = 0;
    let mut put = |value: &[u8]| {
        bytes[position..position + value.len()].copy_from_slice(value);
        position += value.len();
    };
    put(&[4, receipt.state]);
    put(&receipt.row.logical());
    put(&receipt.action.serial.to_be_bytes());
    put(&receipt.action.version.to_be_bytes());
    put(&receipt.action.input_tag);
    put(&receipt.handshake);
    put(&receipt.epoch.to_be_bytes());
    put(&receipt.message);
    put(&receipt.hash);
    put(&receipt.claim);
    put(&receipt.receipt);
    assert_eq!(
        position, REPLY_BYTES,
        "fixed retention-only key custody layout"
    );
}
#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn future<F: std::future::Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    future(apply)
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request>()
        + std::mem::size_of::<KeySessionReceipt>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + std::mem::size_of::<sha2::Sha256>()
        + 3
        + 256
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn established_keys_reply_vectors_preserve_native_identity_without_a_clock_domain_extension() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("requests/key-session-vectors.json")).unwrap();
        let row = RowId::from_logical(
            hex::decode(vectors["parentRow"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        for (index, vector) in vectors["replies"].as_array().unwrap().iter().enumerate() {
            let receipt = KeySessionReceipt {
                row,
                action: super::super::requests::Action {
                    serial: 9_007_199_254_740_993,
                    version: u64::MAX,
                    input_tag: [0x44; 32],
                },
                handshake: [0x55; 32],
                epoch: u64::MAX,
                message: [0x77; 32],
                hash: [0x88; 32],
                claim: [0x66; 32],
                receipt: [0x99; 32],
                state: (index + 1) as u8,
            };
            let mut body = [0; REPLY_BYTES];
            encode_into(receipt, &mut body);
            assert_eq!(hex::encode(body), vector["body"]);
        }
    }
}
