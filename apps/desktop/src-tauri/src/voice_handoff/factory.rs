//! Native-owned derivation registration. No command/listener exposes this pilot yet. The
//! selected key, original actor and UI commit travel with actual work rather than an invoke
//! waiter. A lost reply must recover the same counted ledger purpose, never create new work.
use super::bootstrap::Route;
use super::channel::PhysicalHold;
use super::context::Context;
use super::ledger::{Begin, Purpose, Registration, RowId};
use crate::voice_authority::SelectedHandoffOwner;
use crate::{check_voice_binding_in, UiSessionGate};
use catcoms_app::{ServerActor, VoiceEndpointError};
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind};
use catcoms_sync::voice_v2::{CallGrant, VoiceError};
use std::sync::Arc;

const FACTORY_BYTES: usize = 4096;
// Pinned Tokio 1.53.3 task Cell Header/Core/Trailer (64-byte alignment, scheduler, hooks,
// stage discriminator and join waker), one sequential oneshot Inner<Result<Arc<Action>>>
// or Inner<Result<PairReceipt>>, its Arc counters and an actor-envelope copy. The actual
// future/output/handle are measured separately below. This allowance does not claim bounds
// on general allocator bookkeeping, thread stacks or the whole Tokio runtime. Re-audit on a
// Tokio task/oneshot layout or actor command change; the locked dependency is intentional.
// Re-audited for 1.53.3: Header's scheduled_at uses the zero-sized schedule-latency mock
// in our supported feature configuration. Enabling that instrumentation requires a new
// inventory; the separate measured future/output checks do not account for runtime hooks.
const TASK_AND_REPLY_BYTES: usize = 1024;
// Existing protected 96 KiB Workflow supplies this variable loan. No extra engine lane or
// group/process limit is introduced; framed input/result copies still debit this exact Phase.
const DERIVATION_HANDOFF_BYTES: usize = 16 * 1024;
mod confirmation;
// The inactive factories are compiled in production, but only the integration fixtures
// call them before the authenticated child dispatcher is connected.
#[cfg(test)]
pub(super) use confirmation::{ensure_confirmation_row, ensure_incoming_row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    Closed,
    Stopped,
    Protocol(VoiceError),
}
impl From<VoiceEndpointError> for Refused {
    fn from(error: VoiceEndpointError) -> Self {
        match error {
            VoiceEndpointError::Stopped => Self::Stopped,
            VoiceEndpointError::Protocol(error) => Self::Protocol(error),
        }
    }
}
impl From<VoiceError> for Refused {
    fn from(error: VoiceError) -> Self {
        Self::Protocol(error)
    }
}

/// Fixed completed-task output owns its payment until actual collection, including when the
/// invoking future disappeared. This is deliberately not a serialization/IPC custody proof.
struct Completion {
    result: Result<RowId, Refused>,
    _custody: FactoryCustody,
}
/// The physical position and task payment follow the same actual worker/output lifetime.
struct FactoryCustody {
    _physical: PhysicalHold,
    _credit: Credit,
}

/// Consume the ORIGINAL native-selected allocation before any factory is called. A local
/// channel must pass its retained selection, never owner/executor strings from a body.
/// Matching public fields cannot adopt a replacement selection. The actual channel's closed
/// fence is repeated at final admission under the same commit which orders channel closure.
/// First effects run under the original UI commit. Lock may revoke the affine lease immediately,
/// but cannot report completion ahead of an already admitted actor turn. A subsequent unlock
/// never makes this generation current again. Historical metadata has separate ledger APIs.
/// An existing Pending row is returned as metadata only: inspect ledger status before asking
/// for input. Receiving RowId never asserts that an action is attached or a browser is ready.
pub(super) async fn ensure_derivation_row(
    context: &Context,
    route: &Route,
    selector: PairSelector,
    physical: PhysicalHold,
) -> Result<RowId, Refused> {
    if !physical.belongs_to(route.session()) {
        return Err(Refused::Closed);
    }
    let selected = route.selected().clone();
    let context = context.upgrade().ok_or(Refused::Closed)?;
    {
        let authority = context
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        require_current_selection(context.gate(), &authority, &selected)?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    check_voice_binding_in(&context.servers, selected.binding())
        .await
        .map_err(|_| Refused::Closed)?;
    // Owned equivalent of require_ui_session_generation: cancellation of the outer future
    // must not return this guard while an admitted actor request is still outstanding.
    let commit = context.commit.clone().lock_owned().await;
    context
        .gate()
        .require_unlocked()
        .await
        .map_err(|_| Refused::Closed)?;
    let actor = {
        let servers = context.servers.lock().await;
        let binding = selected.binding();
        servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .map(|entry| entry.actor.clone())
            .ok_or(Refused::Closed)?
    };
    {
        let authority = context
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        require_current_selection(context.gate(), &authority, &selected)?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    // Weak grant upgrade and all affine destruction occur outside Coordinator/ledger fences.
    let grant = selected.grant()?;
    let registration = match context
        .ledger
        .begin(selected.clone(), Purpose::Derive(selector))?
    {
        Begin::Existing(id) => return Ok(id),
        Begin::Factory(registration) => registration,
    };
    let credit = selected.resources().charge(
        selected.binding().scope.group,
        FACTORY_BYTES,
        ResourceKind::Retained,
    )?;
    // Do not let the detached task retain the whole registry, mounted store, Coordinator or
    // authorization fields. The original actor/grant and owned commit below are sufficient.
    // This drop is outside every native guard, including the final selection fence above.
    drop(context);
    let work = run(
        registration,
        selected,
        actor,
        grant,
        selector,
        commit,
        FactoryCustody {
            _physical: physical,
            _credit: credit,
        },
    );
    // Include the actual future, completion and task handles, not an optimistic named payload
    // size. Refusal before polling has performed no actor work and drops the unused ticket.
    if worker_layout(&work) > FACTORY_BYTES {
        return Err(Refused::Protocol(VoiceError::Capacity));
    }
    tokio::spawn(work)
        .await
        .map_err(|_| Refused::Stopped)?
        .result
}

/// Keep the atomic lock-registration fence inside the same Coordinator turn as selection.
/// A lock may publish its new generation before its revocation hook can acquire this mutex.
pub(super) fn require_current_selection(
    gate: UiSessionGate<'_>,
    authority: &crate::voice_authority::Coordinator,
    selected: &Arc<SelectedHandoffOwner>,
) -> Result<(), Refused> {
    gate.check_generation(selected.binding().generation)
        .map_err(|_| Refused::Closed)?;
    if !authority.is_current_handoff_owner(selected) {
        return Err(Refused::Closed);
    }
    Ok(())
}

fn worker_layout<F>(future: &F) -> usize {
    std::mem::size_of_val(future)
        + std::mem::size_of::<Completion>()
        + std::mem::size_of::<tokio::task::JoinHandle<Completion>>()
        + 2 * std::mem::size_of::<usize>()
        + std::mem::size_of::<catcoms_app::AppCommand>()
        + TASK_AND_REPLY_BYTES
}

/// One detached worker per admitted row, with no await between mark_handoff and the first
/// actor request. Every subsequent refusal retains that original purpose: Pair may already
/// exist even if Derive/attachment failed. There are no general ambiguous-attempt refunds.
async fn run(
    registration: Registration,
    _selected: Arc<SelectedHandoffOwner>,
    actor: ServerActor,
    grant: Arc<CallGrant>,
    selector: PairSelector,
    _commit: tokio::sync::OwnedMutexGuard<()>,
    custody: FactoryCustody,
) -> Completion {
    let result = async {
        if registration.has_publication()? {
            // An earlier actual Root reply already reached this counted Pending. A busy
            // previous handoff loan is a LOCAL attachment refusal, never permission to
            // repeat the Actor factory after that reply, a lost caller or later expiry.
            return registration
                .attach_published(DERIVATION_HANDOFF_BYTES)
                .map_err(Refused::from);
        }
        registration.mark_handoff()?;
        let pair = actor.prepare_voice_pair(grant.clone(), selector).await?;
        let action = actor
            .prepare_voice_derivation(grant, pair.handshake)
            .await?;
        registration
            .attach(action, pair, DERIVATION_HANDOFF_BYTES)
            .map_err(Refused::from)
    }
    .await;
    Completion {
        result,
        _custody: custody,
    }
}

#[cfg(test)]
pub(super) mod tests;
