//! Inactive original-Pending closure adapter. It terminates ONLY local native preparation,
//! not an uncertain Root pair/action or full Workflow. Production serves only Status/read-only history.
//! The retained broker must first drain actual SDK/RPC work before issuing ClosePhase; frame
//! authentication cannot prove that protocol precondition on its own.
use super::bootstrap::Route;
use super::channel::{Incoming, Operation};
use super::context::Context;
use super::custody::CloseMode;
use super::factory::Refused;
use super::ledger::{Purpose, RowId, Status};
use super::requests::{Kind, Request};
use catcoms_sync::voice_v2::VoiceError;

/// Keep the affine actual request through result collection and its one authenticated reply.
/// A lost reply leaves the same native closed tombstone; a new frame cannot rearm its factory.
pub(super) struct Response {
    result: Result<Status, Refused>,
    incoming: Incoming,
}
impl Response {
    pub(super) fn into_parts(self) -> (Result<Status, Refused>, Incoming) {
        (self.result, self.incoming)
    }
}

/// No first-effect grant upgrade, Clock, factory, new task, variable loan or ordinary charge.
/// The original physical position excludes any admitted factory/completed output still owning
/// this Session. It is acquired by the codec before this function, never inferred from IDs.
pub(super) fn close(context: &Context, route: &Route, incoming: Incoming) -> Response {
    let result = close_original(context, route, &incoming);
    Response { result, incoming }
}
fn close_original(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<Status, Refused> {
    if !incoming.belongs_to(route.session()) || incoming.operation() != Operation::ClosePhase {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    if !matches!(
        request.kind,
        Kind::ClosePhase {
            mode: CloseMode::Return
        }
    ) {
        // A possibly published Pair does not give this local Pending a capability to close
        // Root's Workflow. In particular it must never issue a whole-plan final proof.
        return Err(VoiceError::Unauthorized.into());
    }
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let row = context.ledger.resolve(
        route.selected(),
        Purpose::Derive(request.selector),
        RowId::from_logical(incoming.logical())?,
    )?;
    // All context/Row owners subsequently drop outside native Index/State/Coordinator/UI
    // guards. The ledger releases actual local preparation before publishing ClosedPending.
    row.close_pending().map_err(Into::into)
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    // Conservative synchronous graph beyond the codec's already counted Incoming. Include
    // caller output AND nested resolver/retirement temporaries, not merely a named status.
    // Original fixed header backing remains separately counted in the codec control guard.
    std::mem::size_of::<Response>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + 2 * std::mem::size_of::<Status>()
        + std::mem::size_of::<RowId>()
        + super::ledger::pending_close_layout_bytes()
        + 64 // borrowed argument/guard wrappers
}

#[cfg(test)]
mod tests;
