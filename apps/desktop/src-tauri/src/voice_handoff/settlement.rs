//! Original-session receipt ACK and actual action settlement, including after End/lock.
//! A renderer digest is compared to the actual retained receipt; it cannot construct one.
use super::bootstrap::Route;
use super::channel::{BodyKind, Incoming, Operation};
use super::context::Context;
use super::factory::Refused;
use super::ledger::{Purpose, RowId, Status};
use super::requests::{receipt_digest, Kind, Request};
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
    let row = context.ledger.resolve(
        route.selected(),
        Purpose::Derive(request.selector),
        RowId::from_logical(incoming.logical())?,
    )?;
    let status = row.reconcile()?;
    if identity.is_some_and(|identity| {
        status.action != Some((identity.serial, identity.version, identity.input_tag))
    }) {
        return Err(VoiceError::Unauthorized.into());
    }
    if identity.is_none() {
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
        + std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + 3 * std::mem::size_of::<Status>()
        + std::mem::size_of::<super::custody::PhaseClosure>()
        + std::mem::size_of::<sha2::Sha256>()
        + 128 // nested resolver/retirement/digest argument wrappers
}

#[cfg(test)]
mod tests;
