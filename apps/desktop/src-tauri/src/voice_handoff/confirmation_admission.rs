//! Child engine admission under the same original native/Root fences.
//! No production opcode is enabled by this adapter alone.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::factory::Refused;
use super::requests::{confirmation::Request, Action, Kind};
use catcoms_sync::voice_v2::VoiceError;

#[allow(clippy::large_enum_variant)]
enum Encoded {
    Permit([u8; super::replies::PERMIT_BYTES]),
    History([u8; super::replies::STATUS_BYTES]),
}

#[allow(clippy::large_enum_variant)]
enum Initial {
    Historical([u8; super::replies::STATUS_BYTES]),
    Work(super::ledger::ResolvedRow, u32),
}

/// Resolve read-only history at construction, before the pending async frame exists. This
/// does not enqueue an Actor query, reserve Work or admit an engine until the returned
/// future is actually polled. The authenticated Incoming already owns its spent nonce.
pub(super) fn admit<'a>(
    context: &'a Context,
    route: &'a Route,
    incoming: Incoming,
) -> impl std::future::Future<Output = Result<CompletedReply, channel::Refused>> + 'a {
    // All decoder/reconciliation/Status owners end BEFORE construction of the returned
    // frame. Its captured Initial owns just serialized history or the same actual row.
    let initial = identify(context, route, &incoming);
    async move {
        let result = match initial {
            Ok(Initial::Historical(bytes)) => Ok(Encoded::History(bytes)),
            Ok(Initial::Work(row, engine_bytes)) => {
                prepare(context, route, row, engine_bytes).await
            }
            Err(error) => Err(error),
        };
        seal(incoming, result)
    }
}

fn seal(
    incoming: Incoming,
    result: Result<Encoded, Refused>,
) -> Result<CompletedReply, channel::Refused> {
    let refusal = result
        .as_ref()
        .err()
        .map(|error| super::replies::refused(*error));
    let bytes = match &result {
        Ok(Encoded::Permit(bytes)) => bytes.as_slice(),
        Ok(Encoded::History(bytes)) => bytes.as_slice(),
        Err(_) => refusal.as_ref().expect("encoded refusal").as_slice(),
    };
    let mut reply = incoming.reply(BodyKind::Control, bytes.len(), None)?;
    reply.plain_mut().copy_from_slice(bytes);
    reply.seal()
}

async fn prepare(
    context: &Context,
    route: &Route,
    row: super::ledger::ResolvedRow,
    engine_bytes: u32,
) -> Result<Encoded, Refused> {
    super::factory::preflight_binding(context, route).await?;
    let context = context.upgrade().ok_or(Refused::Closed)?;
    super::factory::require_unlocked_now(&context, route.selected())?;
    let work = row.work()?;
    // Clock runs BEFORE acquiring even the async UI commit. A busy native fence is a
    // proven local, retryable refusal; no engine is admitted and no attempt is refunded.
    let permit = work
        .action()
        .with_engine_admission(engine_bytes as usize, |step| {
            // Parent and child use the SAME nonblocking original-clock final fences.
            // A busy Coordinator/Route cannot carry a pre-wait observation across expiry.
            super::admission::final_engine_admission(&context, route, step)
        })?;
    // Return ONLY the actual commit result. A successful observation/callback is no permit.
    super::replies::admitted(
        row.id(),
        Action {
            serial: work.action().serial(),
            version: work.action().version(),
            input_tag: work.action().input_tag(),
        },
        engine_bytes as usize,
        permit,
    )
    .map(Encoded::Permit)
    .map_err(Into::into)
}

fn identify(context: &Context, route: &Route, incoming: &Incoming) -> Result<Initial, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::Admit
        || incoming.kind() != BodyKind::Control
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let (row, engine_bytes) = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let (row, identity, engine_bytes) = decode_admission(&live, route, incoming)?;
        {
            let status = row.reconcile()?;
            if status.action != Some((identity.serial, identity.version, identity.input_tag)) {
                return Err(VoiceError::Unauthorized.into());
            }
            if status.engine.receipt.is_some() || status.engine.settled || status.drained {
                return super::replies::status(row.id(), &status)
                    .map(Initial::Historical)
                    .map_err(Into::into);
            }
        }
        (row, engine_bytes)
    };
    Ok(Initial::Work(row, engine_bytes))
}

/// The full borrowing child decoder and transient parent resolver finish before any async
/// binding/UI wait. Only the actual resolved child and engine byte count cross that
/// boundary; parent/request metadata cannot become retained native/engine authority.
fn decode_admission(
    context: &super::context::LiveContext,
    route: &Route,
    incoming: &Incoming,
) -> Result<(super::ledger::ResolvedRow, Action, u32), Refused> {
    let request = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    let Kind::Admit {
        action,
        engine_bytes,
    } = request.kind
    else {
        return Err(VoiceError::Malformed.into());
    };
    Ok((
        super::confirmation_input::resolve(context, route, &request)?,
        action,
        engine_bytes,
    ))
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn future<F: std::future::Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    // identify finishes before even CONSTRUCTING the returned async frame, not merely
    // before its first suspension. Its original Incoming and whole output remain counted.
    // This is an application-owner inventory, not a claim about uninitialized stack/RSS.
    let identify = std::mem::size_of::<super::context::LiveContext>()
        + 2 * std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<super::ledger::Status>()
        + std::mem::size_of::<Initial>()
        + std::mem::size_of::<Action>()
        + super::ledger::reconciliation_layout_bytes()
        + super::replies::STATUS_BYTES
        + std::mem::size_of::<sha2::Sha256>()
        + std::mem::size_of::<Incoming>()
        + 128;
    let final_admission = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<super::ledger::Work>()
        + std::mem::size_of::<Encoded>()
        + std::mem::size_of::<catcoms_sync::voice_v2::crypto_actions::CryptoEngineAdmission<'static>>(
        )
        // Root prepares the original candidate/lender before the final Clock; count their
        // real owners too, even though the scoped final ticket only borrows the candidate.
        + std::mem::size_of::<catcoms_sync::voice_v2::resources::ByteBudget>()
        + std::mem::size_of::<Option<catcoms_sync::voice_v2::resources::Credit>>()
        + 128; // nested synchronous final fences/argument wrappers
    let serialization = std::mem::size_of::<Encoded>()
        + std::mem::size_of::<super::channel::PreparedReply>()
        + 3
        + 2 * std::mem::size_of::<usize>()
        + 128;
    // The whole measured returned future/output coexists with its final admission or
    // seal call frame. Do not max away that live outer storage during serialization.
    let bytes = identify.max(future(admit) + final_admission.max(serialization));
    eprintln!("child admission Control future/output {}; identify {identify}; final {final_admission}; serialize {serialization}; total {bytes}", future(admit));
    bytes
}
