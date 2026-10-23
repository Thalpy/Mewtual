//! Closed interoperability-fixture dispatch over the real paid HTTP service. This entire
//! module is cfg(test), selected only by a private Rust constructor. It cannot enable effects
//! in a shipping application. Header authentication, existing Phase lookup, original body
//! custody and all typed adapter authorization remain the same as the eventual live path.
use super::*;
use crate::voice_handoff::{
    admission, channel, confirmation_admission, confirmation_begin, confirmation_input,
    confirmation_result, confirmation_settlement, derivation_result, finalization, history, input,
    ledger, plan_final, requests, settlement,
};

/// The full service future already contains all awaited handler frames. Header admission
/// is synchronous, so also inventory its broad lookup context and immutable proof on top
/// of that future rather than assuming that an inline helper has no temporary ownership.
pub(super) fn admission_layout_bytes() -> usize {
    std::mem::size_of::<crate::voice_handoff::context::LiveContext>()
        + std::mem::size_of::<channel::AuthenticatedRequest>()
        + std::mem::size_of::<Option<Arc<crate::voice_handoff::custody::Phase>>>()
        + 128
}

/// Authenticate once before selecting an existing prepaid Data Phase. No unauthenticated
/// RowId can allocate a body, debit a counter or reach the native action index. The encrypted
/// full purpose is checked again by the typed adapter after AEAD opening.
pub(super) fn admit(
    worker: channel::HeaderWorker,
    port: u16,
    origin: &str,
    route: &Route,
    context: &Context,
) -> Result<Incoming, Refused> {
    let authenticated = worker
        .authenticate(port, origin)
        .map_err(|_| Refused::Header)?;
    let phase = match (authenticated.operation(), authenticated.kind()) {
        (Operation::Result, BodyKind::Data) => {
            let live = context.upgrade().ok_or(Refused::Binding)?;
            Some(
                live.ledger
                    .intake_phase(route.selected(), &authenticated)
                    .map_err(|_| Refused::Binding)?,
            )
        }
        (Operation::Result, _) | (_, BodyKind::Data) => return Err(Refused::Binding),
        (Operation::Status, BodyKind::Control) if authenticated.logical() != route.session_id() => {
            return Err(Refused::Binding)
        }
        (_, BodyKind::Control) => None,
    };
    authenticated
        .admit(phase.as_ref())
        .map_err(|_| Refused::Body)
}

/// Explicit version selection, never decoder fallback. There is no fabricated browser
/// request for native production: Begin-v1 is historical only. A child factory still requires
/// its original admitted parent, normalized purpose and native UI/Actor fences.
pub(super) async fn respond(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, Refused> {
    if incoming.operation() == Operation::Status {
        return status(route, incoming);
    }
    let version = incoming
        .plain()
        .map_err(|_| Refused::Body)?
        .first()
        .copied()
        .ok_or(Refused::Body)?;
    let operation = incoming.operation();
    let result = match (version, operation) {
        (1, Operation::BeginDerive) => history::recover(context, route, incoming).seal(),
        (1, Operation::Input) => {
            if route.session().fixture_take_input_pressure() {
                // Refuse one genuine authenticated Input at the real preflight boundary.
                // Do not retain a broad live context across the wait, fabricate bytes or
                // start action work. The original intake must survive this local refusal.
                let servers = {
                    let live = context.upgrade().ok_or(Refused::Binding)?;
                    live.servers.clone()
                };
                let held = servers.lock_owned().await;
                let reply = input::pull(context, route, incoming).await;
                drop(held);
                reply
            } else {
                input::pull(context, route, incoming).await
            }
        }
        (1, Operation::Admit) => admission::admit(context, route, incoming).await,
        (1, Operation::Result) => {
            let response = derivation_result::commit(context, route, incoming)
                .await
                .map_err(|_| Refused::Body)?;
            let (result, incoming) = response.into_parts();
            let id = ledger::RowId::from_logical(incoming.logical()).map_err(|_| Refused::Body)?;
            history::Response::new(incoming, result.map(|status| (id, status))).seal()
        }
        (1, Operation::ReceiptAck | Operation::Settle) => {
            settlement::apply(context, route, incoming).seal()
        }
        (1, Operation::ClosePhase) => match close_kind(&incoming)? {
            Close::Return => settlement::apply(context, route, incoming).seal(),
            Close::Plan => plan_final::apply(context, route, incoming).await,
            Close::Retire => finalization::respond(context, route, incoming).seal(),
        },
        (1, Operation::FinalAck) => finalization::respond(context, route, incoming).seal(),
        (2, Operation::BeginDerive) => confirmation_begin::begin(context, route, incoming).await,
        (2, Operation::Input) => confirmation_input::pull(context, route, incoming).await,
        (2, Operation::Admit) => confirmation_admission::admit(context, route, incoming).await,
        (2, Operation::Result) => confirmation_result::commit(context, route, incoming)
            .await
            .map_err(|_| Refused::Body)?
            .seal(),
        (2, Operation::ReceiptAck | Operation::Settle | Operation::ClosePhase) => {
            confirmation_settlement::apply(context, route, incoming).seal()
        }
        // No decoder retry, guessed role, synthetic receipt, replacement work or broad
        // catch-all adapter. Unsupported grammar closes this physical connection.
        _ => return Err(Refused::Body),
    };
    result.map_err(|_| Refused::Body)
}

enum Close {
    Return,
    Retire,
    Plan,
}
fn close_kind(incoming: &Incoming) -> Result<Close, Refused> {
    let request = requests::Request::decode(
        incoming.operation(),
        incoming.plain().map_err(|_| Refused::Body)?,
    )
    .map_err(|_| Refused::Body)?;
    match request.kind {
        requests::Kind::ClosePhase {
            mode: crate::voice_handoff::custody::CloseMode::Return,
        } => Ok(Close::Return),
        requests::Kind::ClosePhase {
            mode: crate::voice_handoff::custody::CloseMode::Retire,
        } => Ok(Close::Retire),
        requests::Kind::ClosePlan => Ok(Close::Plan),
        _ => Err(Refused::Body),
    }
}
