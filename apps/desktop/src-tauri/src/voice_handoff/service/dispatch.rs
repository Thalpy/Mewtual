//! Closed production action dispatch over the original paid HTTP service. A native Context
//! enables these adapters, never an HTTP flag or a renderer-supplied owner. Authentication
//! precedes Phase lookup and allocation; each adapter still resolves original authority and
//! performs its own final UI/Actor checks. This does not enable V2 call selection or RTC.
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
            .max(std::mem::size_of::<channel::sdp::Permit>())
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
    // A valid MAC alone must not spend counters for an impossible operation/class/length
    // or logical identity. The encrypted version/purpose is checked after AEAD by exactly
    // one typed decoder; this fixed header predicate never chooses a fallback grammar.
    if !requests::wire_shape(
        authenticated.operation(),
        authenticated.kind(),
        authenticated.logical(),
        authenticated.length(),
        route.session_id(),
    ) {
        return Err(Refused::Header);
    }
    if authenticated.operation().is_sdp() {
        return admit_sdp(authenticated, route, context);
    }
    let phase = match (authenticated.operation(), authenticated.kind()) {
        (Operation::Result, BodyKind::Data) => {
            let live = context.upgrade().ok_or(Refused::Binding)?;
            Some(
                live.ledger
                    .intake_phase(route.selected(), &authenticated)
                    .map_err(|_| Refused::Binding)?,
            )
        }
        // SDP has a distinct opaque permit and may never use the legacy Phase fallback.
        (Operation::Result, _) | (_, BodyKind::Data | BodyKind::Sdp) => {
            return Err(Refused::Binding)
        }
        (Operation::Status, BodyKind::Control) if authenticated.logical() != route.session_id() => {
            return Err(Refused::Binding)
        }
        (_, BodyKind::Control) => None,
    };
    authenticated
        .admit(phase.as_ref())
        .map_err(|_| Refused::Body)
}

/// The SDP grammar has its own original-row capacity permit. Never pass its large
/// bodies through optional legacy Phase admission. Fixed Control remains authenticated
/// bounded metadata: full encrypted ownership and current/cleanup authority are checked
/// by the selected adapter after opening, including valid history/Return after End.
fn admit_sdp(
    authenticated: channel::AuthenticatedRequest,
    route: &Route,
    context: &Context,
) -> Result<Incoming, Refused> {
    if authenticated.kind() == BodyKind::Control {
        return authenticated.admit(None).map_err(|_| Refused::Body);
    }
    let permit = {
        let live = context.upgrade().ok_or(Refused::Binding)?;
        let clock = {
            let authority = match live.authority.try_lock() {
                Ok(authority) => authority,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return Err(Refused::Capacity),
            };
            authority.runtime().clock()
        }; // No authority guard survives original Clock callbacks or ledger intake.
        match authenticated.operation() {
            Operation::SdpSealBegin => live.ledger.sdp_begin_intake(
                route.selected(),
                route.session().identity(),
                &authenticated,
                clock,
            ),
            Operation::SdpSealedResult | Operation::SdpOpenedResult => {
                live.ledger.sdp_result_intake(
                    route.selected(),
                    route.session().identity(),
                    &authenticated,
                    clock,
                )
            }
            _ => return Err(Refused::Binding),
        }
        .map_err(|_| Refused::Binding)?
    }; // No upgraded broad native context survives body allocation or any await.
    authenticated.admit_sdp(permit).map_err(|_| Refused::Body)
}

/// Explicit version selection, never decoder fallback. There is no fabricated browser
/// request for native production: Begin-v1 is historical only. A child factory still requires
/// its original admitted parent, normalized purpose and native UI/Actor fences.
pub(super) async fn respond(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, Refused> {
    if incoming.operation() == Operation::NegotiationCapacity {
        return crate::voice_handoff::negotiation_capacity::apply(context, route, incoming)
            .map_err(|_| Refused::Body);
    }
    if incoming.operation() == Operation::NegotiationAdmission {
        return crate::voice_handoff::negotiation_admission::apply(route, incoming)
            .map_err(|_| Refused::Body);
    }
    if incoming.operation() == Operation::Media {
        return crate::voice_handoff::media::apply(context, route, incoming)
            .await
            .map_err(|_| Refused::Body);
    }
    if incoming.operation().is_sdp() {
        // One explicit typed handler; no decoder fallback or success-as-permission.
        return match incoming.operation() {
            Operation::SdpInput => {
                crate::voice_handoff::sdp_input::pull(context, route, incoming).await
            }
            Operation::SdpEngineAdmit => {
                crate::voice_handoff::sdp_engine::admit(context, route, incoming).await
            }
            Operation::SdpSealedResult
            | Operation::SdpOpenedResult
            | Operation::SdpAuthenticationFailed => {
                crate::voice_handoff::sdp_result::apply(context, route, incoming).await
            }
            Operation::SdpDelivery => {
                crate::voice_handoff::sdp_delivery::apply(context, route, incoming).await
            }
            _ => crate::voice_handoff::sdp_http::apply(context, route, incoming).await,
        }
        .map_err(|_| Refused::Body);
    }
    if incoming.operation() == Operation::Status {
        return status(route, incoming);
    }
    if incoming.operation() == Operation::PcIntent {
        return crate::voice_handoff::pc::apply(context, route, incoming)
            .await
            .map_err(|_| Refused::Body);
    }
    if incoming.operation() == Operation::SdkCustody {
        return crate::voice_handoff::sdk::apply(context, route, incoming)
            .await
            .map_err(|_| Refused::Body);
    }
    if incoming.operation() == Operation::SdpPhase {
        return crate::voice_handoff::sdp::apply(context, route, incoming)
            .await
            .map_err(|_| Refused::Body);
    }
    if incoming.operation() == Operation::KeySession {
        return crate::voice_handoff::key_session::apply(context, route, incoming)
            .await
            .map_err(|_| Refused::Body);
    }
    if incoming.operation() == Operation::ChainTransition {
        return crate::voice_handoff::transition::apply(context, route, incoming)
            .await
            .map_err(|_| Refused::Body);
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
            #[cfg(test)]
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
                return reply.map_err(|_| Refused::Body);
            }
            input::pull(context, route, incoming).await
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
