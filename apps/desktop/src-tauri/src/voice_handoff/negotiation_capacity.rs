//! Capacity-only authenticated control over the SAME original physical Session.
//! No renderer-supplied peer, role, profile, action, deadline or Ready hint exists.
use super::bootstrap::Route;
use super::channel::{BodyKind, CompletedReply, Incoming, Operation, Refused};
use super::context::Context;
use super::requests::negotiation_capacity::{Request, REPLY_BYTES};

pub(super) fn layout_bytes() -> usize {
    let reply = std::mem::size_of::<super::channel::PreparedReply>()
        + std::mem::size_of::<CompletedReply>()
        + REPLY_BYTES
        + 64;
    std::mem::size_of::<Incoming>()
        + std::mem::size_of::<Request>()
        + reply.max(super::bootstrap::admission_capacity_layout_bytes())
}

pub(super) fn apply(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::NegotiationCapacity
        || incoming.kind() != BodyKind::Control
        || incoming.logical() != route.session_id()
    {
        return Err(Refused::Binding);
    }
    let request =
        Request::decode(incoming.plain().map_err(|_| Refused::Body)?).map_err(|_| Refused::Body)?;
    let snapshot = route.admission_capacity(context, request)?.encode();
    let mut reply = incoming
        .reply(BodyKind::Control, snapshot.len(), None)
        .map_err(|_| Refused::Body)?;
    reply.plain_mut().copy_from_slice(&snapshot);
    reply.seal().map_err(|_| Refused::Body)
}
