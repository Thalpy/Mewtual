//! Authenticated operation31 only, on the separate admitted physical route.
use super::bootstrap::AdmittedRoute;
use super::channel::{BodyKind, CompletedReply, HeaderWorker, Incoming, Operation, Refused};
use super::context::Context;
use super::requests::admitted_admission::{self, Request};

pub(super) fn admit(
    worker: HeaderWorker,
    port: u16,
    origin: &str,
    route: &AdmittedRoute,
) -> Result<Incoming, Refused> {
    let authenticated = worker.authenticate(port, origin)?;
    // A valid MAC for another operation/profile never debits either counter.
    if authenticated.operation() != Operation::AdmittedAdmission
        || !admitted_admission::wire_shape(
            authenticated.kind(),
            authenticated.logical(),
            authenticated.length(),
            route.public().public().session,
        )
    {
        return Err(Refused::Header);
    }
    authenticated.admit(None)
}
pub(super) fn respond(
    context: &Context,
    route: &AdmittedRoute,
    incoming: Incoming,
) -> Result<CompletedReply, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::AdmittedAdmission
        || incoming.kind() != BodyKind::Control
        || incoming.logical() != route.public().public().session
    {
        return Err(Refused::Binding);
    }
    let request = Request::decode(incoming.plain()?).map_err(|_| Refused::Body)?;
    let bytes = route.admitted_control(context, request)?.encode();
    let mut reply = incoming.reply(BodyKind::Control, bytes.len(), None)?;
    reply.plain_mut().copy_from_slice(&bytes);
    reply.seal()
}
