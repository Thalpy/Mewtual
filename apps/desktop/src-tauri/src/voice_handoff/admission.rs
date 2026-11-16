//! Clock-free final engine admission under the original native UI/selection/route fences.
//! No production opcode is enabled by this adapter alone.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::factory::Refused;
use super::ledger::{Purpose, RowId};
use super::requests::{Action, Kind, Request};
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
    let (selector, identity, engine_bytes) = {
        let request = Request::decode(
            incoming.operation(),
            incoming.plain().map_err(|_| VoiceError::Malformed)?,
        )?;
        let Kind::Admit {
            action,
            engine_bytes,
        } = request.kind
        else {
            return Err(VoiceError::Malformed.into());
        };
        (request.selector, action, engine_bytes)
    };
    let row = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let row = live.ledger.resolve(
            route.selected(),
            Purpose::Derive(selector),
            RowId::from_logical(incoming.logical())?,
        )?;
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
        row
    };
    super::factory::preflight_binding(context, route).await?;
    let context = context.upgrade().ok_or(Refused::Closed)?;
    super::factory::require_unlocked_now(&context, route.selected())?;
    let work = row.work()?;
    // Clock runs BEFORE acquiring even the async UI commit. A busy native fence is a
    // proven local, retryable refusal; no engine is admitted and no attempt is refunded.
    let permit = work
        .action()
        .with_engine_admission(engine_bytes as usize, |step| {
            final_engine_admission(&context, route, step)
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

/// Clock/capacity preparation precedes this entire sequence. Keep every final acquisition
/// nonblocking, in existing UI -> Coordinator -> Row -> Engine order. A retryable refusal
/// releases no admitted engine and never renews the original deadline or action identity.
pub(super) fn final_engine_admission(
    context: &super::context::LiveContext,
    route: &Route,
    step: catcoms_sync::voice_v2::crypto_actions::CryptoEngineAdmission<'_>,
) -> Result<u64, VoiceError> {
    let _commit = context
        .commit
        .clone()
        .try_lock_owned()
        .map_err(|_| VoiceError::Capacity)?;
    let authority = match context.authority.try_lock() {
        Ok(authority) => authority,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
    };
    super::factory::require_current_selection(context.gate(), &authority, route.selected())
        .map_err(|_| VoiceError::Unauthorized)?;
    route.try_with_open(|| step.commit())
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
        + std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<super::ledger::Status>()
        + std::mem::size_of::<Encoded>()
        + std::mem::size_of::<catcoms_sync::voice_v2::crypto_actions::CryptoEngineAdmission<'static>>(
        )
        // Original candidate owner and preflight lender exist outside the scoped callback.
        // The ticket borrows that owner; neither the credit nor a capacity-lock wait is
        // hidden behind its reference. Moving the same Credit is not a second allocation.
        + std::mem::size_of::<catcoms_sync::voice_v2::resources::ByteBudget>()
        + std::mem::size_of::<Option<catcoms_sync::voice_v2::resources::Credit>>()
        + 128 // nested synchronous final fences/argument wrappers
}

#[cfg(test)]
mod tests;
