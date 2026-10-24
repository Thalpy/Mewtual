//! Typed executor for the original Derive result. It owns the actual authenticated request
//! through actor publication and completed-output collection, including after socket loss.
//! The application-bound authenticated listener routes only its exact v1 Result grammar here.
use super::bootstrap::Route;
use super::channel::{Incoming, Operation};
use super::context::Context;
use super::custody::CopyCustody;
use super::factory::Refused;
use super::ledger::{Purpose, RowId, Status, Work};
use super::requests::{Kind, Request};
use catcoms_app::ServerActor;
use catcoms_sync::voice_v2::{CallGrant, VoiceError};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const TASK_BYTES: usize = 8 * 1024;
// Pinned Tokio task/oneshot/envelope overhead, in addition to concrete future/output/command
// layouts below. Incoming remains inline: count BOTH future and completed-output storage.
// Its ONE fixed scratch backing allocation stays explicitly paid/counted in Session control;
// moving the scratch handle cannot reserve duplicate4KiB arrays in each async state. This narrow
// empty-output executor does not also construct the separate4KiB seal/open actor body bridge.
const TASK_AND_REPLY_BYTES: usize = 1024;

/// Keep Work/task custody BEFORE Incoming in drop order. Its last physical Header must not
/// reopen the Session or wake shutdown while actual result-buffer/work owners still exist.
pub(super) struct Response {
    result: Result<Status, Refused>,
    work: Option<Work>,
    payment: Option<CopyCustody>,
    incoming: Incoming,
}
impl Response {
    /// The dispatcher collects the actual task output before building its one authenticated
    /// Control reply. No raw input escapes; the original Incoming still owns its Data buffer
    /// until reply construction drops it. Fixed status/error metadata is not a drainage proof.
    pub(super) fn into_parts(self) -> (Result<Status, Refused>, Incoming) {
        let Self {
            result,
            work,
            payment,
            incoming,
        } = self;
        drop(work);
        drop(payment);
        (result, incoming)
    }
}

/// Resolve the original native row from the decrypted request, never from browser identity
/// labels alone. The caller must supply an already opened frame and its actual native Route.
/// Historical recovery admits no Work, ADDITIONAL Phase contribution, task or body allocation.
/// It still owns the original Data body already admitted and paid by the physical channel.
pub(super) async fn commit(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<Response, Refused> {
    let prepared = prepare(context, route, &incoming).await;
    match prepared {
        Err(error) => Ok(Response {
            result: Err(error),
            work: None,
            payment: None,
            incoming,
        }),
        Ok(Prepared::Historical(status)) => Ok(Response {
            result: Ok(status),
            work: None,
            payment: None,
            incoming,
        }),
        Ok(Prepared::Work(admitted)) => {
            let task = run(Worker { admitted, incoming });
            #[cfg(test)]
            eprintln!(
                "derivation result peak {}; future {}; response {}; admitted {}; command {}",
                worker_layout(&task),
                std::mem::size_of_val(&task),
                std::mem::size_of::<Response>(),
                std::mem::size_of::<Admitted>(),
                std::mem::size_of::<catcoms_app::AppCommand>()
            );
            if worker_layout(&task) > TASK_BYTES {
                // Proven local refusal: future was never polled and no actor request exists.
                // Dropping it releases original work/payment/request, never refunds a Root
                // publication or reports that an already admitted browser engine drained.
                return Err(Refused::Protocol(VoiceError::Capacity));
            }
            tokio::spawn(task).await.map_err(|_| Refused::Stopped)
        }
    }
}
enum Prepared {
    Historical(Status),
    Work(Admitted),
}
// This value stays inline in the paid physical operation until its original task contribution
// is reserved. No ordinary-pool allocation is required to publish an already admitted result.
struct Admitted {
    work: Work,
    actor: ServerActor,
    grant: Arc<CallGrant>,
    tag: [u8; 32],
    commit: Option<tokio::sync::OwnedMutexGuard<()>>,
    payment: CopyCustody,
}
/// Declaration order applies even on unwind/runtime teardown: the actual work, grant/commit
/// and paid task contribution drop BEFORE Incoming's Header can clear its physical position.
struct Worker {
    admitted: Admitted,
    incoming: Incoming,
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<Prepared, Refused> {
    // A different Session's successfully opened frame is not this Route's authority.
    if !incoming.belongs_to(route.session()) || incoming.operation() != Operation::Result {
        return Err(Refused::Protocol(VoiceError::Unauthorized));
    }
    let request = Request::decode(
        incoming.operation(),
        incoming
            .plain()
            .map_err(|_| Refused::Protocol(VoiceError::Malformed))?,
    )?;
    let Kind::Result {
        action: identity,
        tag,
        output,
    } = request.kind
    else {
        return Err(Refused::Protocol(VoiceError::Malformed));
    };
    if !output.is_empty() || tag != <[u8; 32]>::from(Sha256::digest([])) {
        return Err(Refused::Protocol(VoiceError::RetryMismatch));
    }
    let original = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let original = live.ledger.resolve(
            route.selected(),
            Purpose::Derive(request.selector),
            RowId::from_logical(incoming.logical())?,
        )?;
        let status = original.reconcile()?;
        if status.action != Some((identity.serial, identity.version, identity.input_tag)) {
            return Err(Refused::Protocol(VoiceError::Unauthorized));
        }
        if let Some(receipt) = &status.engine.receipt {
            if receipt.output_tag != tag {
                return Err(Refused::Protocol(VoiceError::RetryMismatch));
            }
            // History resolves without upgrading the old grant or current UI authorization.
            // This supplies no keys, readiness in a replacement browser, or fresh RTC authority.
            return Ok(Prepared::Historical(status));
        }
        original
    };
    let selected = route.selected();
    let commit = super::factory::preflight_binding(context, route).await?;
    let commit = commit.lock_owned().await;
    let context = context.upgrade().ok_or(Refused::Closed)?;
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
        super::factory::require_current_selection(context.gate(), &authority, selected)?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    let grant = selected.grant()?;
    let work = original.work()?;
    let payment = work.phase().copy(TASK_BYTES)?;
    // Never detach the broad mounted-store/Coordinator/native context. The original actor,
    // affine grant, commit and paid work are sufficient; Root rechecks authority in its turn.
    drop(context);
    Ok(Prepared::Work(Admitted {
        work,
        actor,
        grant,
        tag,
        commit: Some(commit),
        payment,
    }))
}
fn worker_layout<F>(future: &F) -> usize {
    std::mem::size_of_val(future)
        + std::mem::size_of::<Response>()
        + std::mem::size_of::<Admitted>()
        + std::mem::size_of::<tokio::task::JoinHandle<Response>>()
        + std::mem::size_of::<catcoms_app::AppCommand>()
        + TASK_AND_REPLY_BYTES
}
async fn run(mut worker: Worker) -> Response {
    // No cancellation point between final admission/task spawn and the owned actor request.
    // The outer waiter may disappear; this task still owns the original UI commit and input.
    let result = match worker
        .admitted
        .actor
        .commit_voice_derivation_result(
            worker.admitted.grant.clone(),
            worker.admitted.work.action().clone(),
            worker.admitted.tag,
        )
        .await
    {
        Ok(_) => worker.admitted.work.refresh().map_err(Refused::from),
        Err(error) => Err(Refused::from(error)),
    };
    drop(worker.admitted.commit.take()); // actual actor publication completed before lock finishes.
    let Worker {
        admitted: Admitted { work, payment, .. },
        incoming,
    } = worker;
    Response {
        result,
        work: Some(work),
        payment: Some(payment),
        incoming,
    }
}

#[cfg(test)]
pub(super) mod tests;
