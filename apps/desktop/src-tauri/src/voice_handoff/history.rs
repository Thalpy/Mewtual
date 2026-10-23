//! Authenticated Begin(retry=false) existing-history recovery. It cannot create/rearm a
//! factory. Unknown remains refusal, never absence/drainage proof or permission to recompute.
//! Production serves this narrow read beside unchanged Status9; no effect opcode is enabled.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::factory::Refused;
use super::ledger::{Purpose, RowId, Status};
use super::replies;
use super::requests::{Kind, Request};
use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
use catcoms_sync::voice_v2::VoiceError;

/// The original physical request remains owned through metadata/reply collection. A lost
/// waiter cannot free its position while a completed response still owns that request.
pub(super) struct Response {
    result: Result<(RowId, Status), Refused>,
    incoming: Incoming,
}
impl Response {
    pub(super) fn new(incoming: Incoming, result: Result<(RowId, Status), Refused>) -> Self {
        Self { result, incoming }
    }
    /// Build the one authenticated Control reply on the original already-spent nonce.
    /// No data Phase, new task, grant upgrade or ordinary-pool payment is admitted here.
    pub(super) fn seal(self) -> Result<CompletedReply, channel::Refused> {
        let Self { result, incoming } = self;
        let encoded = match result {
            Ok((id, status)) => match replies::status(id, &status) {
                Ok(bytes) => Encoded::Status(bytes),
                Err(error) => Encoded::Refused(replies::refused(error.into())),
            },
            Err(error) => Encoded::Refused(replies::refused(error)),
        };
        let bytes = match &encoded {
            Encoded::Status(bytes) => bytes.as_slice(),
            Encoded::Refused(bytes) => bytes.as_slice(),
        };
        let mut reply = incoming.reply(BodyKind::Control, bytes.len(), None)?;
        reply.plain_mut().copy_from_slice(bytes);
        reply.seal()
    }
}
// Keep fixed Control bytes inline in the original payment. A Box here would introduce
// another allocation/lifetime path during cleanup while ordinary capacity may be full.
#[allow(clippy::large_enum_variant)]
enum Encoded {
    Status([u8; replies::STATUS_BYTES]),
    Refused([u8; 3]),
}

/// Zero logical header is canonical ONLY for this first unknown-row Begin. The returned
/// actual RowId is authenticated inside the response body; all later operations must name it.
/// The MAC alone authenticates no purpose: decrypt and bind the full selector to the SAME
/// native-selected grant allocation/executor/lease before reading any retained action.
pub(super) fn recover(context: &Context, route: &Route, incoming: Incoming) -> Response {
    let result = recover_original(context, route, &incoming);
    Response { result, incoming }
}
fn recover_original(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<(RowId, Status), Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::BeginDerive
        || incoming.kind() != BodyKind::Control
        || incoming.logical() != [0; 16]
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    if !matches!(request.kind, Kind::Begin { retry: false }) {
        // Authorized first admission/retry is a separate integration boundary. This lookup
        // must never accidentally call Ledger::begin when history is missing or idle.
        return Err(VoiceError::Unauthorized.into());
    }
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let row = context
        .ledger
        .lookup_original(route.selected(), Purpose::Derive(request.selector))
        .ok_or(VoiceError::Unauthorized)?;
    // Root history/Clock observations run outside all native guards through the existing
    // sole paid observer. Recover original terminal/settlement metadata beyond a stale cache.
    // Current Derive publishes Derived only: actor/authority refusal has NO terminal receipt.
    // Generic Rejected metadata is not evidence of such a Derive path. No first effect occurs.
    let status = row.reconcile()?;
    if status.engine.receipt.as_ref().is_some_and(|receipt| {
        !matches!(
            receipt.outcome,
            CryptoOutcome::Derived | CryptoOutcome::Rejected(_)
        )
    }) {
        return Err(VoiceError::Malformed.into());
    }
    Ok((row.id(), status))
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    // Conservative synchronous graph beyond the codec's existing physical state/scratch.
    // Count output, resolver snapshots, canonical response, digest state and argument guards.
    std::mem::size_of::<Response>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + 2 * std::mem::size_of::<Status>()
        + std::mem::size_of::<RowId>()
        + std::mem::size_of::<Encoded>()
        + std::mem::size_of::<sha2::Sha256>()
        + 64
}

#[cfg(test)]
mod tests;
