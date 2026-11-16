//! Authenticated read-only child recovery. Renderer metadata never constructs a
//! parent, action or incoming token. Unknown history is refusal, not permission to recompute.
use super::bootstrap::Route;
use super::channel::{Incoming, Operation};
use super::context::Context;
use super::factory::Refused;
use super::history::Response;
use super::ledger::{Purpose, RowId, Status};
use super::requests::{confirmation::Request, Kind};
use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
use catcoms_sync::voice_v2::VoiceError;

/// Keep the same original physical request through authenticated reply collection. This
/// read cannot reserve a child, retry an Actor factory, admit an engine or borrow a Phase.
pub(super) fn recover(context: &Context, route: &Route, incoming: Incoming) -> Response {
    let result = recover_original(context, route, &incoming);
    Response::new(incoming, result)
}
fn recover_original(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<(RowId, Status), Refused> {
    if !incoming.belongs_to(route.session()) || incoming.operation() != Operation::BeginDerive {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    if !matches!(request.kind, Kind::Begin { retry: false }) {
        return Err(VoiceError::Unauthorized.into());
    }
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let parent = context.ledger.resolve(
        route.selected(),
        Purpose::Derive(request.selector),
        request.parent,
    )?;
    let child = context
        .ledger
        .lookup_child(route.selected(), &parent, request.purpose, None)?;
    // The sole paid observer may call Root/Clock. No native Index/State guard survives
    // lookup_child, including after End, ordinary Return or separate Plan-final intent.
    let status = child.reconcile()?;
    if status.engine.receipt.as_ref().is_some_and(|receipt| {
        !matches!(
            receipt.outcome,
            CryptoOutcome::Sealed { .. } | CryptoOutcome::Opened | CryptoOutcome::Rejected(_)
        )
    }) {
        return Err(VoiceError::Malformed.into());
    }
    Ok((child.id(), status))
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    // Separate from v1 so its paid handler graph does not grow. Count both original row
    // handles, the borrowing child request, fixed reply/status, resolver and digest locals.
    std::mem::size_of::<Response>()
        + std::mem::size_of::<super::context::LiveContext>()
        + 2 * std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + 2 * std::mem::size_of::<Status>()
        + std::mem::size_of::<RowId>()
        + super::replies::STATUS_BYTES
        + std::mem::size_of::<sha2::Sha256>()
        + 64
}

#[cfg(test)]
mod tests;
