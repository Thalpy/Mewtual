//! Inactive child engine admission under the same original native/Root fences.
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

pub(super) async fn admit(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let result = prepare(context, route, &incoming).await;
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
    incoming: &Incoming,
) -> Result<Encoded, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::Admit
        || incoming.kind() != BodyKind::Control
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let (row, identity, engine_bytes) = decode_admission(&context, route, incoming)?;
    {
        let status = row.reconcile()?;
        if status.action != Some((identity.serial, identity.version, identity.input_tag)) {
            return Err(VoiceError::Unauthorized.into());
        }
        if status.engine.receipt.is_some() || status.engine.settled || status.drained {
            return super::replies::status(row.id(), &status)
                .map(Encoded::History)
                .map_err(Into::into);
        }
    }
    crate::check_voice_binding_in(&context.servers, route.selected().binding())
        .await
        .map_err(|_| Refused::Closed)?;
    context
        .gate()
        .require_unlocked()
        .await
        .map_err(|_| Refused::Closed)?;
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

/// The full borrowing child decoder and transient parent resolver finish before any async
/// binding/UI wait. Only the actual resolved child and fixed action identity cross that
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
    future(admit)
        + std::mem::size_of::<super::context::LiveContext>()
        + 2 * std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<super::ledger::Status>()
        + std::mem::size_of::<Encoded>()
        + std::mem::size_of::<catcoms_sync::voice_v2::crypto_actions::CryptoEngineAdmission<'static>>(
        )
        // Root prepares the original candidate/lender before the final Clock; count their
        // real owners too, even though the scoped final ticket only borrows the candidate.
        + std::mem::size_of::<catcoms_sync::voice_v2::resources::ByteBudget>()
        + std::mem::size_of::<Option<catcoms_sync::voice_v2::resources::Credit>>()
        + 128 // nested synchronous final fences/argument wrappers
}
