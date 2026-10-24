//! Separate whole-plan final delivery after ordinary action Returns. No wire operation or
//! production dispatch is enabled here. Authentication/full-purpose/RowId resolution must
//! precede this trusted adapter; this gate orders new intent against normal channel closure.
use super::bootstrap::Route;
use super::channel::{
    self, BodyKind, CompletedReply, Incoming, Operation, Session, SessionIdentity,
};
use super::context::Context;
use super::factory::Refused;
use super::ledger::{FinalReceipt, Ledger, Purpose, ResolvedRow, RowId};
use super::requests::{Kind, Request};
use crate::voice_authority::SelectedHandoffOwner;
use catcoms_sync::voice_v2::VoiceError;
use std::sync::Arc;

/// Private construction retains the ACTUAL application UI commit through intent installation
/// and Root retirement. A random mutex/renderer flag cannot serve as this cleanup admission.
/// All retained metadata is covered by the original Control operation; no task is spawned.
pub(super) struct Admission {
    ledger: Arc<Ledger>,
    selected: Arc<SelectedHandoffOwner>,
    // Before intent is installed there may be NO Row paying for this identity yet.
    // Retain the original paid Session through admission, including forced-shutdown races.
    session: Arc<Session>,
    _commit: tokio::sync::OwnedMutexGuard<()>,
}
impl Admission {
    pub(super) fn ledger(&self) -> &Arc<Ledger> {
        &self.ledger
    }
    pub(super) fn selected(&self) -> &SelectedHandoffOwner {
        &self.selected
    }
    pub(super) fn session(&self) -> &Arc<SessionIdentity> {
        self.session.identity()
    }
}

pub(super) async fn admit(context: &Context, route: &Route) -> Result<Admission, Refused> {
    // Only this original UI fence may survive a parked wait. Holding LiveContext here
    // would pin the mounted store/registries through lock or application teardown even
    // though no final intent has been admitted. Cleanup is deliberately grant-independent.
    let commit = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        live.commit.clone()
    };
    let commit = commit.lock_owned().await;
    let context = context.upgrade().ok_or(Refused::Closed)?;
    // Cleanup must remain available after End/lock. The exact original canonical registry
    // allocation/session must still be established and unclosed; no grant is upgraded.
    context
        .channels
        .require_cleanup_route(route)
        .map_err(|_| Refused::Closed)?;
    Ok(Admission {
        ledger: context.ledger.clone(),
        selected: route.selected().clone(),
        session: route.session().clone(),
        _commit: commit,
    })
}

/// Future authenticated child adapter entry. Root may remain Pending while original inputs
/// or Lower owners survive. A retry uses the same intent and publisher, without taking Work.
pub(super) async fn request(
    context: &Context,
    route: &Route,
    row: &ResolvedRow,
) -> Result<FinalReceipt, Refused> {
    let admission = admit(context, route).await?;
    row.request_plan_final(&admission)?;
    row.issue_final(admission.session()).map_err(Into::into)
}

/// Authenticated Control adapter. Keep the original physical position through
/// await, synchronous Root retirement, serialization and actual output collection. Socket
/// loss may abandon this future before intent, but can never relabel a committed Return.
pub(super) async fn apply(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let _physical = incoming.hold_position()?;
    let result = prepare(context, route, &incoming)
        .await
        .and_then(|receipt| super::replies::final_receipt(receipt).map_err(Into::into));
    let refusal = result
        .as_ref()
        .err()
        .map(|error| super::replies::refused(*error));
    let bytes = match &result {
        Ok(bytes) => bytes.as_slice(),
        Err(_) => refusal.as_ref().expect("encoded fixed refusal").as_slice(),
    };
    let mut reply = incoming.reply(BodyKind::Control, bytes.len(), None)?;
    reply.plain_mut().copy_from_slice(bytes);
    reply.seal()
}

async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<FinalReceipt, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::ClosePhase
        || incoming.kind() != BodyKind::Control
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let selector = {
        let decoded = Request::decode(
            incoming.operation(),
            incoming.plain().map_err(|_| VoiceError::Malformed)?,
        )?;
        if !matches!(decoded.kind, Kind::ClosePlan) {
            return Err(VoiceError::Unauthorized.into());
        }
        decoded.selector
    };
    let row = {
        // Broad resolver state and borrowed decoded body do not cross the UI-commit await.
        let context = context.upgrade().ok_or(Refused::Closed)?;
        context.ledger.resolve(
            route.selected(),
            Purpose::Derive(selector),
            RowId::from_logical(incoming.logical())?,
        )?
    };
    request(context, route, &row).await
}

/// Complete mutually-exclusive Control adapter graph. The actual compiler-generated
/// futures are measured without polling, so nested await state (including the owned UI
/// commit) cannot silently escape the original 12KiB physical-position inventory.
#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    tests::future_layout_bytes()
        + std::mem::size_of::<Admission>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<ResolvedRow>()
        + super::ledger::retirement_layout_bytes()
        + 2 * std::mem::size_of::<FinalReceipt>()
        + 2 * std::mem::size_of::<super::ledger::Status>()
        + 2 * super::replies::FINAL_BYTES
        + std::mem::size_of::<sha2::Sha256>()
        + 256 // synchronous resolver, exact proof hashing and response wrappers
}

/// Exercise the row's independent original-Session check even if a future canonical
/// Registry implementation accidentally admits another actual channel allocation.
#[cfg(test)]
pub(super) async fn assert_foreign_session_after_release(
    context: &Context,
    route: &Route,
    row: &ResolvedRow,
    foreign: &Route,
) {
    let mut admission = admit(context, route).await.unwrap();
    admission.session = foreign.session().clone();
    assert!(matches!(
        row.request_plan_final(&admission),
        Err(catcoms_sync::voice_v2::VoiceError::Unauthorized)
    ));
}

#[cfg(test)]
mod tests;
