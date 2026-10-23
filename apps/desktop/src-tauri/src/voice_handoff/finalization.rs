//! Inactive exact original-Session terminal closure/issuance/ACK adapter. This is metadata
//! cleanup, not engine admission, browser drainage evidence or a normal Actor/WebView flow.
//! Production's whitelist remains unchanged until the complete action contracts are ready.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::custody::CloseMode;
use super::factory::Refused;
use super::ledger::{FinalReceipt, Purpose, RowId};
use super::replies;
use super::requests::{Kind, Request};
use catcoms_sync::voice_v2::VoiceError;

/// Physical custody continues through response sealing and actual output collection. A lost
/// response leaves original intent/issuance/ACK intact; no new task or variable owner exists.
pub(super) struct Response {
    result: Result<FinalReceipt, Refused>,
    incoming: Incoming,
}
impl Response {
    pub(super) fn seal(self) -> Result<CompletedReply, channel::Refused> {
        let Self { result, incoming } = self;
        let mut bytes = [0; replies::FINAL_BYTES];
        let encoded =
            result.and_then(|receipt| replies::final_receipt(receipt).map_err(Into::into));
        let length = match encoded {
            Ok(encoded) => {
                bytes = encoded;
                bytes.len()
            }
            Err(error) => {
                bytes[..3].copy_from_slice(&replies::refused(error));
                3
            }
        };
        let mut reply = incoming.reply(BodyKind::Control, length, None)?;
        reply.plain_mut().copy_from_slice(&bytes[..length]);
        reply.seal()
    }
}

pub(super) fn respond(context: &Context, route: &Route, incoming: Incoming) -> Response {
    let result = respond_original(context, route, &incoming);
    Response { result, incoming }
}
fn respond_original(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<FinalReceipt, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.kind() != BodyKind::Control
        || !matches!(
            incoming.operation(),
            Operation::ClosePhase | Operation::FinalAck
        )
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let row = context.ledger.resolve(
        route.selected(),
        Purpose::Derive(request.selector),
        RowId::from_logical(incoming.logical())?,
    )?;
    match request.kind {
        Kind::ClosePhase {
            mode: CloseMode::Retire,
        } => {
            // A prior ordinary Return may have lost its reply while an actual copy was
            // still alive. Visit bounded compaction BEFORE checking immutable close mode:
            // once that copy drains, an ordinary row must be able to become local history.
            // This never upgrades Return to Retire or supplies its missing Root witness.
            // The Retire below still refuses, and independent channel preflight decides
            // whether that ordinary history permits closing this original Session.
            let _ = row.try_compact()?;
            row.close_phase(route.session().identity(), CloseMode::Retire)?;
            // No listener wait for copies/Root snapshot: one original paid driver retries
            // Pending. Do not take Work or Input here; that would wait on its own custody.
            let _ = row.try_compact()?;
            row.issue_final(route.session().identity())
                .map_err(Into::into)
        }
        Kind::FinalAck { proof } => row
            .acknowledge_final(route.session().identity(), proof)
            .map_err(Into::into),
        _ => Err(VoiceError::Unauthorized.into()),
    }
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    std::mem::size_of::<Response>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<super::custody::PhaseClosure>()
        + 2 * std::mem::size_of::<FinalReceipt>()
        + 2 * std::mem::size_of::<super::ledger::Status>()
        + 2 * replies::FINAL_BYTES
        + std::mem::size_of::<sha2::Sha256>()
        + 128 // nested resolver/observer/argument wrappers
}

#[cfg(test)]
mod tests;
