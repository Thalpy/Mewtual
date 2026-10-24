//! Authenticated original-parent key custody. No SDP or RTC authority is issued here.
//! Existing Claim recovery and Release remain available after End/lock; first Claim alone
//! needs current original UI/selection admission. No renderer time or Finish evidence enters.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation, SessionIdentity};
use super::context::{Context, LiveContext};
use super::factory::{self, Refused};
#[cfg(test)]
use super::ledger::ResolvedRow;
use super::ledger::{Ledger, Purpose, RowId, TransitionReceipt};
use super::requests::transition::{Kind, Request};
use crate::voice_authority::{Coordinator, SelectedHandoffOwner};
use catcoms_sync::voice_v2::VoiceError;
use std::sync::Arc;

pub(super) const REPLY_BYTES: usize = 214;
/// Private construction borrows the ACTUAL current UI -> Coordinator -> Route fence.
/// It cannot outlive those guards or be supplied by a renderer/boolean/test mutex.
pub(super) struct Admission<'a> {
    context: &'a LiveContext,
    authority: &'a Coordinator,
    route: &'a Route,
    _commit: &'a tokio::sync::OwnedMutexGuard<()>,
}
impl Admission<'_> {
    pub(super) fn ledger(&self) -> &Arc<Ledger> {
        &self.context.ledger
    }
    pub(super) fn selected(&self) -> &SelectedHandoffOwner {
        self.route.selected()
    }
    pub(super) fn session(&self) -> &Arc<SessionIdentity> {
        self.route.session().identity()
    }
    pub(super) fn current(&self) -> Result<(), VoiceError> {
        factory::require_current_selection(
            self.context.gate(),
            self.authority,
            self.route.selected(),
        )
        .map_err(|_| VoiceError::Unauthorized)
    }
}

pub(super) async fn apply(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let _physical = incoming.hold_position()?;
    let result = prepare(context, route, &incoming).await;
    // Serialize directly into the original paid reply. Two intermediate 214-byte
    // arrays would exceed this operation's unchanged 12 KiB Control envelope.
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
) -> Result<TransitionReceipt, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::ChainTransition
        || incoming.kind() != BodyKind::Control
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let (row, clock, action, handshake, kind) = {
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
        // Lookup/Coordinator waiting precedes native Clock sampling. No broad context or
        // mutex guard crosses the later asynchronous first-Claim preflight.
        let clock = live
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .runtime()
            .clock();
        // The 104-byte selector is lookup-only. Do not retain a duplicate in the
        // asynchronous frame after exact resolution has bound the original row.
        (row, clock, request.action, request.handshake, request.kind)
    };
    let mono = clock.monotonic_ms();
    if let Kind::Release(proof) = kind {
        return row
            .release_transition(route.session().identity(), action, handshake, proof, mono)
            .map_err(Into::into);
    }
    // Recover BEFORE requiring a live grant. A lost initial reply followed by End must
    // recover its original receipt with zero remaining time, so Release cannot deadlock.
    if let Some(receipt) =
        row.recover_transition(route.session().identity(), action, handshake, mono)?
    {
        return Ok(receipt);
    }
    factory::preflight_binding(context, route).await?;
    let live = context.upgrade().ok_or(Refused::Closed)?;
    factory::require_unlocked_now(&live, route.selected())?;
    // All acquisitions after this native sample use try-locks. Known local contention is
    // a retryable refusal, never a new deadline or an ambiguous attempt refund.
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
    let admission = Admission {
        context: &live,
        authority: &authority,
        route,
        _commit: &commit,
    };
    let receipt =
        route.try_with_open(|| row.claim_transition(&admission, action, handshake, mono))?;
    Ok(receipt)
}
fn encode_into(receipt: TransitionReceipt, bytes: &mut [u8]) {
    assert_eq!(bytes.len(), REPLY_BYTES, "original fixed reply buffer");
    let mut position = 0;
    let mut put = |value: &[u8]| {
        bytes[position..position + value.len()].copy_from_slice(value);
        position += value.len();
    };
    put(&[3, receipt.state]);
    put(&receipt.row.logical());
    put(&receipt.action.serial.to_be_bytes());
    put(&receipt.action.version.to_be_bytes());
    put(&receipt.action.input_tag);
    put(&receipt.handshake);
    put(&receipt.epoch.to_be_bytes());
    put(&receipt.message);
    put(&receipt.hash);
    put(&receipt.until.to_be_bytes());
    put(&receipt.remaining.to_be_bytes());
    put(&receipt.receipt);
    assert_eq!(position, REPLY_BYTES, "fixed custody reply layout");
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn future<F: std::future::Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    future(apply)
        + std::mem::size_of::<LiveContext>()
        + std::mem::size_of::<ResolvedRow>()
        + std::mem::size_of::<Request>()
        + std::mem::size_of::<TransitionReceipt>()
        + std::mem::size_of::<Admission<'static>>()
        + std::mem::size_of::<sha2::Sha256>()
        + 3 // fixed refusal; successful encoding writes directly into the original reply
        + 256
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transition_reply_vectors_preserve_original_identity_and_u64s_with_no_current_grant_in_history(
    ) {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("requests/transition/vectors.json")).unwrap();
        for case in vectors["replies"].as_array().unwrap() {
            let expected = &case["expected"];
            let fixed = |value: &serde_json::Value| -> [u8; 32] {
                hex::decode(value.as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap()
            };
            let receipt = TransitionReceipt {
                row: RowId::from_logical(
                    hex::decode(expected["parentRow"].as_str().unwrap())
                        .unwrap()
                        .try_into()
                        .unwrap(),
                )
                .unwrap(),
                action: super::super::requests::Action {
                    serial: expected["identity"]["action"]
                        .as_str()
                        .unwrap()
                        .parse()
                        .unwrap(),
                    version: expected["identity"]["version"]
                        .as_str()
                        .unwrap()
                        .parse()
                        .unwrap(),
                    input_tag: fixed(&expected["identity"]["inputTag"]),
                },
                handshake: fixed(&expected["handshake"]),
                epoch: expected["purpose"]["epoch"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
                message: [0x66; 32],
                hash: [0x88; 32],
                until: case["until"].as_str().unwrap().parse().unwrap(),
                remaining: case["remaining"].as_u64().unwrap() as u32,
                receipt: [0x99; 32],
                state: case["state"].as_u64().unwrap() as u8,
            };
            let mut bytes = [0; REPLY_BYTES];
            encode_into(receipt, &mut bytes);
            assert_eq!(hex::encode(bytes), case["body"]);
        }
    }
}
