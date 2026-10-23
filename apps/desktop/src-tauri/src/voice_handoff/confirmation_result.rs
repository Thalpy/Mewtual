//! Inactive typed executor for an original confirmation child result. It owns the actual authenticated request
//! through actor publication and completed-output collection, including after socket loss.
//! This effect executor is not exposed by the listener: ingress serves only Status and read-only history.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::custody::{CopyCustody, PublicationCustody};
use super::factory::Refused;
use super::ledger::{Status, Work};
use super::requests::{confirmation::Request, Action, Kind};
use crate::check_voice_binding_in;
use catcoms_app::{ServerActor, VoiceConfirmationBody};
use catcoms_sync::voice_v2::crypto_actions::CryptoOperation;
use catcoms_sync::voice_v2::{CallGrant, VoiceError};
use sha2::{Digest, Sha256};
use std::ops::Range;
use std::sync::Arc;

const TASK_BYTES: usize = 8 * 1024;
// Pinned Tokio task/oneshot/envelope overhead, in addition to concrete future/output/command
// layouts below. Incoming remains inline: count BOTH future and completed-output storage.
// Its ONE fixed scratch backing allocation stays explicitly paid/counted in Session control;
// moving the scratch handle cannot reserve duplicate4KiB arrays in each async state. This narrow
// child executor additionally borrows the SAME Phase for the separate4KiB actor body bridge;
// TASK_BYTES does not pay that bridge or its actual Data buffer twice.
const TASK_AND_REPLY_BYTES: usize = 1024;

/// Keep Work/task custody BEFORE Incoming in drop order. Its last physical Header must not
/// reopen the Session or wake shutdown while actual result-buffer/work owners still exist.
pub(super) struct Response {
    result: Result<Status, Refused>,
    work: Option<Work>,
    payment: Option<Arc<PublicationCustody>>,
    incoming: Incoming,
}
impl Response {
    /// The dispatcher collects the actual task output before building its one authenticated
    /// Control reply. Historical/refusal responses keep their original Data buffer; an
    /// admitted worker has transferred and collected it through the actual actor body.
    /// Incoming still owns the same physical header/response nonce. Fixed result metadata
    /// never proves that independent browser/Input/Phase owners have drained.
    pub(super) fn seal(self) -> Result<CompletedReply, channel::Refused> {
        let Self {
            result,
            work,
            payment,
            incoming,
        } = self;
        let result = result.and_then(|status| {
            let id = super::ledger::RowId::from_logical(incoming.logical())?;
            Ok((id, status))
        });
        // Keep the actual Work/publication payment through synchronous serialization too.
        // Historical/refusal replies have no task payment and use fixed physical Control.
        let reply = super::history::Response::new(incoming, result).seal();
        drop(work);
        drop(payment);
        reply
    }
}

/// Resolve the original native row from the decrypted request, never from browser identity
/// labels alone. The caller must supply an already opened frame and its actual native Route.
/// Historical recovery admits no Work, ADDITIONAL Phase contribution, task or body allocation.
/// It still owns the original Data body already admitted and paid by the physical channel.
pub(super) async fn commit(
    context: &Context,
    route: &Route,
    mut incoming: Incoming,
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
        Ok(Prepared::Work(mut admitted)) => {
            let physical = incoming.hold_position().map_err(|_| Refused::Closed)?;
            // Already paid by the original 8KiB task contribution. The body and worker
            // share this SAME allocation; neither can release the ordering guard alone.
            let publication = Arc::new(PublicationCustody::new(
                admitted.commit.take().expect("original admitted UI commit"),
                physical,
                admitted.payment.take().expect("original task contribution"),
            ));
            // Range and actual child/action were checked before this irreversible move.
            // A local metadata refusal restores the SAME buffer and makes no actor request.
            let body = match incoming.result_body_with_publication(
                admitted.work.phase(),
                admitted.output.clone(),
                Some(publication.clone()),
            ) {
                Ok(body) => body,
                Err(error) => {
                    drop(admitted); // all unstarted work/commit before original physical reply
                    return Ok(Response {
                        result: Err(Refused::Protocol(match error {
                            super::channel::Refused::Capacity => VoiceError::Capacity,
                            _ => VoiceError::Malformed,
                        })),
                        work: None,
                        payment: None,
                        incoming,
                    });
                }
            };
            let task = run(Worker {
                body: Some(body),
                publication: Some(publication),
                admitted,
                incoming,
            });
            #[cfg(test)]
            eprintln!(
                "confirmation result peak {}; future {}; response {}; admitted {}; command {}",
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
    output: Range<usize>,
    commit: Option<tokio::sync::OwnedMutexGuard<()>>,
    payment: Option<CopyCustody>,
}
/// Declaration order applies even on unwind/runtime teardown: the actual work, grant/commit
/// and paid task contribution drop BEFORE Incoming's Header can clear its physical position.
struct Worker {
    // Drop actual transferred bytes before the worker payment and physical header.
    body: Option<VoiceConfirmationBody>,
    admitted: Admitted,
    publication: Option<Arc<PublicationCustody>>,
    incoming: Incoming,
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<Prepared, Refused> {
    let context = context.upgrade().ok_or(Refused::Closed)?;
    let (original, identity, tag, output_range) = decode_result(&context, route, incoming)?;
    {
        // Drop the history snapshot before first-effect awaits. Its dynamic receipt owners
        // are not needed for admission and must not coexist with the later worker graph.
        let status = original.reconcile()?;
        if status.action != Some((identity.serial, identity.version, identity.input_tag)) {
            return Err(Refused::Protocol(VoiceError::Unauthorized));
        }
        if let Some(receipt) = &status.engine.receipt {
            if receipt.output_tag != tag {
                return Err(Refused::Protocol(VoiceError::RetryMismatch));
            }
            // History is past metadata, with no current grant/keys/RTC authority.
            return Ok(Prepared::Historical(status));
        }
    }
    let selected = route.selected();
    {
        let authority = context
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        super::factory::require_current_selection(context.gate(), &authority, selected)?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    check_voice_binding_in(&context.servers, selected.binding())
        .await
        .map_err(|_| Refused::Closed)?;
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
        super::factory::require_current_selection(context.gate(), &authority, selected)?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    let grant = selected.grant()?;
    let work = original.work()?;
    let output = &incoming.plain().map_err(|_| VoiceError::Malformed)?[output_range.clone()];
    if !matches!(
        work.action().operation(),
        CryptoOperation::EncryptConfirmation | CryptoOperation::DecryptConfirmation
    ) || work.action().output_tag(output)? != tag
    {
        return Err(VoiceError::RetryMismatch.into());
    }
    let payment = work.phase().copy(TASK_BYTES)?;
    // Never detach the broad mounted-store/Coordinator/native context. The original actor,
    // affine grant, commit and paid work are sufficient; Root rechecks authority in its turn.
    drop(context);
    Ok(Prepared::Work(Admitted {
        work,
        actor,
        grant,
        tag,
        output: output_range,
        commit: Some(commit),
        payment: Some(payment),
    }))
}

/// The full borrowing decoder and transient parent handle end BEFORE the first suspension.
/// No plaintext copy or request descriptor remains live alongside actor-body construction.
fn decode_result(
    context: &super::context::LiveContext,
    route: &Route,
    incoming: &Incoming,
) -> Result<(super::ledger::ResolvedRow, Action, [u8; 32], Range<usize>), Refused> {
    // A different Session's successfully opened frame is not this Route's authority.
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::Result
        || incoming.kind() != BodyKind::Data
    {
        return Err(Refused::Protocol(VoiceError::Unauthorized));
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
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
    if tag != <[u8; 32]>::from(Sha256::digest(output)) {
        return Err(Refused::Protocol(VoiceError::RetryMismatch));
    }
    let original = super::confirmation_input::resolve(context, route, &request)?;
    // The canonical result is the exact trailing slice; no pointer arithmetic or copy.
    let end = incoming.plain().map_err(|_| VoiceError::Malformed)?.len();
    let output_range = end - output.len()..end;
    Ok((original, identity, tag, output_range))
}
fn worker_layout<F>(future: &F) -> usize {
    worker_layout_bytes(std::mem::size_of_val(future))
}
fn worker_layout_bytes(future: usize) -> usize {
    future
        + std::mem::size_of::<Response>()
        + std::mem::size_of::<Admitted>()
        + std::mem::size_of::<tokio::task::JoinHandle<Response>>()
        + std::mem::size_of::<catcoms_app::AppCommand>()
        + std::mem::size_of::<PublicationCustody>()
        + 2 * std::mem::size_of::<usize>()
        + TASK_AND_REPLY_BYTES
}

/// Concrete Control portion only. The detached worker, its shared publication allocation,
/// and actor body bridge are independently paid by8KiB+4KiB of the ORIGINAL16KiB Data Phase.
/// They must not be hidden here as an uncharged Box or additional ordinary-pool reservation.
#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn future<F: std::future::Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let decode = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<Request<'static>>()
        + 2 * std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<sha2::Sha256>()
        + 32
        + std::mem::size_of::<Action>()
        + std::mem::size_of::<Range<usize>>()
        + 128;
    let validate = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<Status>()
        + std::mem::size_of::<Admitted>()
        + std::mem::size_of::<sha2::Sha256>()
        + 32
        + 128;
    // The borrowing decoder returns before any await; validation finishes before the worker
    // is spawned, and Response::seal runs only after actual task/output collection. Include
    // the complete synchronous reply graph explicitly, even without a worker payment.
    future(commit)
        + decode
            .max(validate)
            .max(super::history::serialization_layout_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concrete_worker_and_publication_fit_the_original_task_contribution() {
        fn size<F: std::future::Future>(_: impl FnOnce(Worker) -> F) -> usize {
            worker_layout_bytes(std::mem::size_of::<F>())
        }
        let bytes = size(run);
        assert!(bytes <= TASK_BYTES, "result worker requires {bytes}");
        // Worst admitted Data body already includes its AEAD tag. Actor metadata pays its
        // bounded ResultOwner/Bytes/BodyState/flight/mailbox/completion graph; Publication's
        // pointee and Arc header are separately counted in worker_layout_bytes above.
        let total = bytes + catcoms_app::VOICE_CONFIRMATION_BODY_METADATA_BYTES + 2064;
        assert!(
            total <= 16 * 1024,
            "result worker/body/frame requires {total}"
        );
    }

    #[tokio::test]
    async fn renderer_result_labels_cannot_create_a_native_child_or_admit_actor_work() {
        use super::super::derivation_result::tests::Case;
        use super::super::service::tests::opened_frame;
        let case = Case::unadmitted().await;
        let foreign = Case::unadmitted().await;
        let mut row = case.id.logical();
        row[15] += 1; // canonical child-shaped header, with NO actual registered child
        let mut body = vec![2];
        body.extend_from_slice(case.selector.peer.as_bytes());
        body.extend_from_slice(&case.selector.join);
        body.extend_from_slice(&case.selector.key_ref);
        body.extend_from_slice(&case.selector.epoch.to_be_bytes());
        body.extend_from_slice(&case.id.logical());
        body.push(0); // Start(None)
        body.extend_from_slice(&case.action.serial().to_be_bytes());
        body.extend_from_slice(&case.action.version().to_be_bytes());
        body.extend_from_slice(&case.action.input_tag());
        body.extend_from_slice(&<[u8; 32]>::from(Sha256::digest([9; 16])));
        body.extend_from_slice(&16u16.to_be_bytes());
        body.extend_from_slice(&[9; 16]);
        let phase = case.row().work().unwrap().phase().clone();
        let before = case.selected.resources().counts();
        for (index, offset) in [
            None,
            Some(0),
            Some(1),
            Some(105),
            Some(121),
            Some(170),
            Some(204),
        ]
        .into_iter()
        .enumerate()
        {
            let mut bytes = body.clone();
            if let Some(offset) = offset {
                bytes[offset] ^= 1;
            }
            let incoming = opened_frame(
                &case.public,
                &case.route,
                (index as u64 + 1, 3, 1, row),
                &bytes,
                Some(&phase),
            );
            let response = commit(&case.context, &case.route, incoming).await.unwrap();
            assert!(response.result.is_err());
            assert!(response.work.is_none() && response.payment.is_none());
            assert_eq!(case.selected.resources().counts(), before);
            assert!(case.native.state.ui_session_commit.try_lock().is_ok());
            drop(response);
        }
        let foreign_phase = foreign.row().work().unwrap().phase().clone();
        let incoming = opened_frame(
            &foreign.public,
            &foreign.route,
            (1, 3, 1, row),
            &body,
            Some(&foreign_phase),
        );
        let response = commit(&case.context, &case.route, incoming).await.unwrap();
        assert!(matches!(
            response.result,
            Err(Refused::Protocol(VoiceError::Unauthorized))
        ));
        assert!(response.work.is_none() && response.payment.is_none());
        drop(response);
        assert!(case.action.terminal_status().receipt.is_none());
        case.retire();
        foreign.retire();
        case.native.close().await;
        foreign.native.close().await;
    }
}
async fn run(mut worker: Worker) -> Response {
    // This independently spawned task survives loss of the socket/invoke waiter. Its exact
    // original UI commit remains owned until the actor publication AND body/reply collection.
    // No engine/replay allowance is refunded on an ambiguous handoff or stopped actor.
    let result = {
        let body = worker.body.as_ref().expect("actual transferred body");
        let submitted = match worker.admitted.work.action().operation() {
            CryptoOperation::EncryptConfirmation => {
                worker
                    .admitted
                    .actor
                    .submit_voice_confirmation_result(
                        worker.admitted.grant.clone(),
                        worker.admitted.work.action().clone(),
                        body,
                        worker.admitted.tag,
                    )
                    .await
            }
            CryptoOperation::DecryptConfirmation => {
                worker
                    .admitted
                    .actor
                    .commit_voice_confirmation_result(
                        worker.admitted.grant.clone(),
                        worker.admitted.work.action().clone(),
                        body,
                        worker.admitted.tag,
                    )
                    .await
            }
            CryptoOperation::Derive => {
                unreachable!("child operation checked before ownership transfer")
            }
        };
        match submitted {
            Ok(completed) => match completed.result() {
                Ok(_) => worker.admitted.work.refresh().map_err(Refused::from),
                Err(error) => Err(error.into()),
            },
            Err(error) => Err(error.into()),
        }
    }; // the ACTUAL completed actor reply is collected before releasing the commit
       // Original body/copy owners first, never a drainage inference.
    drop(worker.body.take());
    // Keep the actual shared payment/hold through completed worker-output collection too.
    // A queued actor body independently retains it if THIS task is cancelled before now.
    let payment = worker.publication.take();
    let Worker {
        admitted: Admitted { work, .. },
        incoming,
        ..
    } = worker;
    Response {
        result,
        work: Some(work),
        payment,
        incoming,
    }
}
