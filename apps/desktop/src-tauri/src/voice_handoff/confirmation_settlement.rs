//! Inactive child receipt ACK, settlement and Return, including after End/lock.
//! A renderer digest is compared to the actual retained receipt; it cannot construct one.
use super::bootstrap::Route;
use super::channel::{BodyKind, Incoming, Operation};
use super::context::Context;
use super::factory::Refused;
use super::ledger::{RowId, Status};
use super::requests::{confirmation::Request, receipt_digest, Kind};
use catcoms_sync::voice_v2::VoiceError;

pub(super) fn apply(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> super::history::Response {
    let result = original(context, route, &incoming);
    super::history::Response::new(incoming, result)
}
fn original(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<(RowId, Status), Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.kind() != BodyKind::Control
        || !matches!(
            incoming.operation(),
            Operation::ReceiptAck | Operation::Settle | Operation::ClosePhase
        )
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    let (identity, digest) = match request.kind {
        Kind::ReceiptAck { action, digest } => (Some(action), Some(digest)),
        Kind::Settle { action } => (Some(action), None),
        Kind::ClosePhase {
            mode: super::custody::CloseMode::Return,
        } => (None, None),
        _ => return Err(VoiceError::Malformed.into()),
    };
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let row = super::confirmation_input::resolve(&context, route, &request)?;
    let status = row.reconcile()?;
    if identity.is_some_and(|identity| {
        status.action != Some((identity.serial, identity.version, identity.input_tag))
    }) {
        return Err(VoiceError::Unauthorized.into());
    }
    if identity.is_none() {
        if !matches!(status.pending, super::ledger::PendingStatus::NotPending) {
            // A lost factory/attachment reply can leave an original uncertain child. Its
            // exact Return closes ONLY local Pending preparation; otherwise it would block
            // parent Plan-final forever after End. This never claims Root absence, engine
            // settlement or Phase drainage. The same original physical position excludes
            // actual admitted factory/output work; close_pending rechecks its worker fence.
            return Ok((row.id(), row.close_pending()?));
        }
        row.close_phase(
            route.session().identity(),
            super::custody::CloseMode::Return,
        )?;
        let _ = row.try_compact()?;
        return Ok((row.id(), row.reconcile()?));
    }
    // Original selection/Session equality above is required. Current grants deliberately
    // are not: their revocation cannot strand cleanup or revive a fresh engine generation.
    if let Some(digest) = digest {
        let receipt = status
            .engine
            .receipt
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if receipt_digest(receipt) != digest {
            return Err(VoiceError::RetryMismatch.into());
        }
        row.acknowledge(receipt)?;
        Ok((row.id(), row.reconcile()?))
    } else {
        Ok((row.id(), row.settle()?))
    }
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    std::mem::size_of::<super::history::Response>()
        + std::mem::size_of::<super::context::LiveContext>()
        + 2 * std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + 3 * std::mem::size_of::<Status>()
        + std::mem::size_of::<super::custody::PhaseClosure>()
        + super::ledger::pending_close_layout_bytes()
        + std::mem::size_of::<sha2::Sha256>()
        + 128 // nested resolver/retirement/digest argument wrappers
}
