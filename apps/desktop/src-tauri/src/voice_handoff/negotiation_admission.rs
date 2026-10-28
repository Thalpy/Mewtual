//! Authenticated, sticky local protocol selection. This first boundary prevents
//! old automatic producers racing future canonical Root admission. A pinned reply
//! is only mode selection: no Root preparation, native row or Ready is exposed here.
use super::bootstrap::Route;
use super::channel::{BodyKind, CompletedReply, Incoming, Operation, Refused};
use super::requests::negotiation_admission::Request;

/// Synchronous frames are not part of the enclosing service future's inline
/// size. Include decoding/State observation and reply construction in the SAME
/// unchanged12KiB Control graph, with no charge borrowed from future Root work.
pub(super) fn layout_bytes() -> usize {
    let reply = std::mem::size_of::<super::channel::PreparedReply>()
        + std::mem::size_of::<CompletedReply>()
        + super::requests::negotiation_admission::REPLY_BYTES
        + 64;
    std::mem::size_of::<Incoming>()
        + std::mem::size_of::<Request>()
        + reply.max(super::bootstrap::admission_mode_layout_bytes())
}

pub(super) fn apply(route: &Route, incoming: Incoming) -> Result<CompletedReply, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::NegotiationAdmission
        || incoming.kind() != BodyKind::Control
        || incoming.logical() != route.session_id()
    {
        return Err(Refused::Binding);
    }
    let request =
        Request::decode(incoming.plain().map_err(|_| Refused::Body)?).map_err(|_| Refused::Body)?;
    let snapshot = route.admission_mode(request)?;
    let mut prepared = incoming
        .reply(BodyKind::Control, snapshot.len(), None)
        .map_err(|_| Refused::Body)?;
    prepared.plain_mut().copy_from_slice(&snapshot);
    prepared.seal().map_err(|_| Refused::Body)
}
