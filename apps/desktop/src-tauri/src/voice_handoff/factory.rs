//! Native-owned derivation registration. No command/listener exposes this pilot yet. The
//! selected key, original actor and UI commit travel with actual work rather than an invoke
//! waiter. A lost reply must recover the same counted ledger purpose, never create new work.
use super::ledger::{Begin, Purpose, Registration, RowId};
use crate::voice_authority::SelectedHandoffOwner;
use crate::{
    check_voice_activation_generation, check_voice_binding, require_unlocked_session, AppState,
};
use catcoms_app::{ServerActor, VoiceEndpointError};
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind};
use catcoms_sync::voice_v2::{CallGrant, VoiceError};
use std::sync::Arc;

const FACTORY_BYTES: usize = 4096;
// Pinned Tokio 1.52.3 task Cell Header/Core/Trailer (64-byte alignment, scheduler, hooks,
// stage discriminator and join waker), one sequential oneshot Inner<Result<Arc<Action>>>
// or Inner<Result<PairReceipt>>, its Arc counters and an actor-envelope copy. The actual
// future/output/handle are measured separately below. This allowance does not claim bounds
// on general allocator bookkeeping, thread stacks or the whole Tokio runtime. Re-audit on a
// Tokio task/oneshot layout or actor command change; the locked dependency is intentional.
const TASK_AND_REPLY_BYTES: usize = 1024;
// Existing protected 96 KiB Workflow supplies this variable loan. No extra engine lane or
// group/process limit is introduced; framed input/result copies still debit this exact Phase.
const DERIVATION_HANDOFF_BYTES: usize = 16 * 1024;

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
    _credit: Credit,
}

/// Resolve the actual selected generation before any native factory is called. All first
/// effects run under the original UI commit. Lock may revoke the affine lease immediately,
/// but cannot report completion ahead of an already admitted actor turn. A subsequent unlock
/// never makes this generation current again. Historical metadata has separate ledger APIs.
/// An existing Pending row is returned as metadata only: inspect ledger status before asking
/// for input. Receiving RowId never asserts that an action is attached or a browser is ready.
pub(crate) async fn ensure_derivation_row(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
    selector: PairSelector,
) -> Result<RowId, Refused> {
    let selected = state
        .voice_authority
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .selected_handoff_owner(owner, executor)
        .map_err(|_| Refused::Closed)?;
    check_voice_binding(state, selected.binding())
        .await
        .map_err(|_| Refused::Closed)?;
    // Owned equivalent of require_ui_session_generation: cancellation of the outer future
    // must not return this guard while an admitted actor request is still outstanding.
    let commit = state.ui_session_commit.clone().lock_owned().await;
    require_unlocked_session(state)
        .await
        .map_err(|_| Refused::Closed)?;
    let actor = {
        let servers = state.servers.lock().await;
        let binding = selected.binding();
        servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .map(|entry| entry.actor.clone())
            .ok_or(Refused::Closed)?
    };
    {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        require_current_selection(state, &mut authority, owner, executor, &selected)?;
    }
    // Weak grant upgrade and all affine destruction occur outside Coordinator/ledger fences.
    let grant = selected.grant()?;
    let registration = match state
        .voice_handoff
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
    let work = run(
        registration,
        selected,
        actor,
        grant,
        selector,
        commit,
        credit,
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
fn require_current_selection(
    state: &AppState,
    authority: &mut crate::voice_authority::Coordinator,
    owner: &str,
    executor: [u8; 32],
    selected: &Arc<SelectedHandoffOwner>,
) -> Result<(), Refused> {
    check_voice_activation_generation(state, selected.binding().generation)
        .map_err(|_| Refused::Closed)?;
    let current = authority
        .selected_handoff_owner(owner, executor)
        .map_err(|_| Refused::Closed)?;
    if !Arc::ptr_eq(selected, &current) {
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
    credit: Credit,
) -> Completion {
    let result = async {
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
        _credit: credit,
    }
}

#[cfg(test)]
mod tests;
