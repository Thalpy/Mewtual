//! Original paid SDP Begin/Resolve/History/ACK/settlement/Return handlers. These are staged private
//! adapters: the production Header gate still refuses ALL SDP action operations until
//! Input/engine/result/delivery/ACK/settlement and the private SDK vertical are coherent.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, PhysicalHold};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{self, Purpose, ResolvedRow, SdpBegin, SdpHistory};
use super::requests::sdp_action::{self, Request};
use super::sdp_submission::{Payload, Preparation};
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
use catcoms_sync::voice_v2::VoiceError;

// Inline snapshot storage is included in the complete paid Control graph.
#[allow(clippy::large_enum_variant)]
enum Reply {
    History(SdpHistory),
    Missing {
        parent: ledger::RowId,
        purpose: NativeRowPurpose,
        closed: bool,
    },
}
/// One physical request always survives its row/worker/body. A queued factory transfers
/// independent physical/UI custody into the original paid worker; cancellation of this
/// handler neither aborts Actor work nor proves no publication.
pub(super) async fn apply(
    context: &Context,
    route: &Route,
    mut incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let physical = incoming.hold_position()?;
    let result = prepare(context, route, &mut incoming, physical).await;
    seal(incoming, result)
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &mut Incoming,
    physical: PhysicalHold,
) -> Result<Reply, Refused> {
    if !incoming.belongs_to(route.session()) {
        return Err(VoiceError::Unauthorized.into());
    }
    let ledger = resolve_context(context, route)?;
    if let Some(reply) = inspect(&ledger, route, incoming)? {
        return Ok(reply);
    }
    preflight(context, route).await?;
    let waiter = match start_begin(context, route, &ledger, incoming, physical)? {
        Started::Immediate(reply) => return Ok(reply),
        Started::Task(waiter) => waiter,
    };
    finish(route, waiter.await)
}
/// A read-only preflight keeps only the actual admitted query receiver across await.
/// Full mailbox refusal precedes query admission and is explicitly retryable. Cleanup
/// inspection never enters this path; final publication still repeats native/Root fences.
async fn preflight(context: &Context, route: &Route) -> Result<(), Refused> {
    let pending = queue_preflight(context, route)?;
    let observed = pending.complete().await.map_err(|_| Refused::Closed)?;
    if observed.scope != route.selected().binding().scope {
        return Err(Refused::Closed);
    }
    let live = context.upgrade().ok_or(Refused::Closed)?;
    let authority = live
        .authority
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    factory::require_current_selection(live.gate(), &authority, route.selected())?;
    route.require_open().map_err(|_| Refused::Closed)
}
/// Separate synchronous frame: the largest AppCommand envelope exists only during its
/// nonblocking send, not inside the paid Control future while waiting for Actor progress.
fn queue_preflight(
    context: &Context,
    route: &Route,
) -> Result<catcoms_app::VoiceContextPending, Refused> {
    let actor = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        {
            let authority = live
                .authority
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            factory::require_current_selection(live.gate(), &authority, route.selected())?;
            route.require_open().map_err(|_| Refused::Closed)?;
        }
        let servers = live.servers.try_lock().map_err(|_| VoiceError::Capacity)?;
        let binding = route.selected().binding();
        servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .map(|entry| entry.actor.clone())
            .ok_or(Refused::Closed)?
    };
    actor
        .try_queue_voice_v2_context(route.selected().binding().scope.room)
        .map_err(|error| match error {
            catcoms_app::VoiceEndpointError::Stopped => Refused::Stopped,
            catcoms_app::VoiceEndpointError::Protocol(error) => Refused::Protocol(error),
        })
}
/// Completion owns the actual publication UI guard. Consume that owner before history
/// invokes original Root Clock callbacks or any further native UI admission.
fn finish(
    route: &Route,
    completion: Result<super::sdp_submission::Completion, Refused>,
) -> Result<Reply, Refused> {
    let row: ResolvedRow = completion?.into_result()?;
    Ok(Reply::History(row.sdp_history(route.session().identity())?))
}
/// Serialization never overlaps borrowed decoding/admission or history observation.
fn seal(
    incoming: Incoming,
    result: Result<Reply, Refused>,
) -> Result<CompletedReply, channel::Refused> {
    let length = match &result {
        Ok(Reply::History(_)) => super::sdp_replies::HISTORY_BYTES,
        Ok(Reply::Missing { .. }) => super::sdp_replies::MISSING_BYTES,
        Err(_) => 4,
    };
    let mut output = incoming.reply(BodyKind::Control, length, None)?;
    match result {
        Ok(Reply::History(snapshot)) => match super::sdp_replies::history(&snapshot) {
            Ok(bytes) => output.plain_mut().copy_from_slice(&bytes),
            Err(_) => return Err(channel::Refused::Binding), // no invented success on inconsistent native history
        },
        Ok(Reply::Missing {
            parent,
            purpose,
            closed,
        }) => output
            .plain_mut()
            .copy_from_slice(&super::sdp_replies::missing(parent, purpose, closed)),
        Err(error) => output
            .plain_mut()
            .copy_from_slice(&super::sdp_replies::refused(error)),
    }
    output.seal()
}
/// Canonical route lookup follows the existing UI->Registry order, then releases every
/// guard BEFORE Root Clock/Engine observation or any await. Cleanup needs provenance,
/// not an unlocked/current lease; End must not hide original finite history.
fn resolve_context(
    context: &Context,
    route: &Route,
) -> Result<std::sync::Arc<ledger::Ledger>, Refused> {
    let live = context.upgrade().ok_or(Refused::Closed)?;
    let _commit = live
        .commit
        .clone()
        .try_lock_owned()
        .map_err(|_| VoiceError::Capacity)?;
    live.channels
        .require_cleanup_route(route)
        .map_err(|_| Refused::Closed)?;
    Ok(live.ledger.clone())
}
/// Extract the exact original runtime clock under the short native guard, then sample
/// it outside every UI/Coordinator/Index/State guard. Never retain the broad live context
/// or this callback owner into final admission or a detached worker.
fn observe_time(context: &Context) -> Result<(u64, u64), Refused> {
    let clock = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let clock = live
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .runtime()
            .clock();
        clock
    };
    Ok((clock.now_ms(), clock.monotonic_ms()))
}
fn inspect(
    ledger: &std::sync::Arc<ledger::Ledger>,
    route: &Route,
    incoming: &Incoming,
) -> Result<Option<Reply>, Refused> {
    let request = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    match request {
        Request::Resolve {
            original,
            purpose,
            close,
        } => {
            let parent = ledger.resolve_sdp_parent(route.selected(), original.parent)?;
            let row = if close {
                ledger.close_sdp_original(
                    &parent,
                    route.selected(),
                    route.session().identity(),
                    &original,
                    purpose,
                )?
            } else {
                ledger.lookup_sdp_action(
                    &parent,
                    route.selected(),
                    route.session().identity(),
                    &original,
                    purpose,
                )?
            };
            row.map_or(
                Ok(Some(Reply::Missing {
                    parent: parent.id(),
                    purpose,
                    closed: close,
                })),
                |row| {
                    if close {
                        row.try_compact_sdp()?;
                    }
                    Ok(Some(Reply::History(
                        row.sdp_history(route.session().identity())?,
                    )))
                },
            )
        }
        Request::Existing(request) => {
            let row = ledger.resolve(
                route.selected(),
                Purpose::Sdp {
                    parent: request.parent,
                    purpose: request.purpose,
                },
                request.row,
            )?;
            match request.kind {
                sdp_action::Kind::History => {}
                sdp_action::Kind::Return => {
                    row.close_sdp_action(route.session().identity())?;
                    row.try_compact_sdp()?;
                }
                sdp_action::Kind::Ack { action, digest } => {
                    cleanup_action(&row, route.session().identity(), action, Some(digest))?;
                }
                sdp_action::Kind::Settle(action) => {
                    cleanup_action(&row, route.session().identity(), action, None)?;
                }
                _ => return Err(VoiceError::Malformed.into()),
            }
            Ok(Some(Reply::History(
                row.sdp_history(route.session().identity())?,
            )))
        }
        Request::Begin(request) => {
            let parent = ledger.resolve_sdp_parent(route.selected(), request.parent)?;
            let purpose = if request.plaintext.is_some() {
                NativeRowPurpose::Seal
            } else {
                NativeRowPurpose::Open
            };
            if let Some(row) = ledger.lookup_sdp_action(
                &parent,
                route.selected(),
                route.session().identity(),
                &request,
                purpose,
            )? {
                if !row.check_sdp_begin(&parent, route.session().identity(), &request)? {
                    return Ok(Some(Reply::History(
                        row.sdp_history(route.session().identity())?,
                    )));
                }
            }
            Ok(None)
        }
    }
}

/// Exact original executor cleanup, not new SDK/RTC authority. The authenticated SAME
/// Session's physical position excludes queued/native output work; the browser additionally
/// owes a join of its real SDK and last Result RPC before sending23. Native cannot infer
/// that join from a receipt, funding snapshot, closed PC, timeout or lost reply.
///
/// Select the registered allocation under native provenance fences, then release every
/// native guard before Root clocks/Engine/credit destructors. These trusted historical
/// seams intentionally remain usable after End/lock, without a current grant or Actor.
fn cleanup_action(
    row: &ResolvedRow,
    session: &std::sync::Arc<super::channel::SessionIdentity>,
    action: super::requests::Action,
    digest: Option<[u8; 32]>,
) -> Result<(), VoiceError> {
    let original = row.sdp_result_action(session, action)?;
    if let Some(digest) = digest {
        let status = original.terminal_status();
        let receipt = status.receipt.as_ref().ok_or(VoiceError::Unauthorized)?;
        if super::requests::receipt_digest(receipt) != digest {
            return Err(VoiceError::RetryMismatch);
        }
        original.acknowledge_result(receipt)
    } else {
        // Root serializes this irreversible fence with admission/publication. In-flight
        // preparation keeps its own credit until actual retirement. An uncertain result
        // never refunds an engine; only this original executor's settlement statement does.
        original.settle_engine();
        Ok(())
    }
}
// This synchronous result is measured in the Control graph before its sole waiter parks.
#[allow(clippy::large_enum_variant)]
enum Started {
    Immediate(Reply),
    Task(super::sdp_submission::Waiter),
}
/// All borrowed decoding, native admission and paid worker construction stay in a
/// synchronous frame. Only its opaque waiter crosses await; the Control future never
/// retains another copy of the Candidate/Registration/Preparation graph while parked.
fn start_begin(
    context: &Context,
    route: &Route,
    ledger: &std::sync::Arc<ledger::Ledger>,
    incoming: &mut Incoming,
    physical: PhysicalHold,
) -> Result<Started, Refused> {
    let (registration, intent, funding, delivery, seal) = {
        let Request::Begin(request) = Request::decode(
            incoming.operation(),
            incoming.kind(),
            incoming.logical(),
            incoming.plain().map_err(|_| VoiceError::Malformed)?,
        )?
        else {
            return Err(VoiceError::Malformed.into());
        };
        let parent = ledger.resolve_sdp_parent(route.selected(), request.parent)?;
        let seal = request.plaintext.is_some();
        let purpose = if seal {
            NativeRowPurpose::Seal
        } else {
            NativeRowPurpose::Open
        };
        let existing = ledger.lookup_sdp_action(
            &parent,
            route.selected(),
            route.session().identity(),
            &request,
            purpose,
        )?;
        if let Some(row) = &existing {
            if !row.check_sdp_begin(&parent, route.session().identity(), &request)? {
                return Ok(Started::Immediate(Reply::History(
                    row.sdp_history(route.session().identity())?,
                )));
            }
        }
        let candidate = parent.sdp_action_candidate(route.session().identity(), &request)?;
        let (wall, mono) = observe_time(context)?;
        let delivery = if seal && !candidate.intent().local_is_low() {
            // A premature HIGH Answer must not install/poison its one canonical
            // Seal row. Only a CURRENT original PC may wait for the same applied
            // accepted Open. Terminal rejection is not a transient absence.
            super::sdp::current(context, route, |admission| {
                parent.require_sdp_publication_current(admission, &candidate, wall, mono)
            })?;
            let opened = ledger
                .lookup_sdp_action(
                    &parent,
                    route.selected(),
                    route.session().identity(),
                    &request,
                    NativeRowPurpose::Open,
                )?
                .ok_or(VoiceError::Capacity)?;
            let history = opened.sdp_history(route.session().identity())?;
            // Root historical lookup invokes original Clock callbacks. Recheck
            // native currency AFTER them before classifying an unapplied result
            // as a temporary wait on the original HIGH PC.
            let (current_wall, current_mono) = observe_time(context)?;
            super::sdp::current(context, route, |admission| {
                parent.require_sdp_publication_current(
                    admission,
                    &candidate,
                    current_wall,
                    current_mono,
                )
            })?;
            if matches!(
                history.status().engine.receipt.as_ref().map(|r| r.outcome),
                Some(CryptoOutcome::Rejected(_))
            ) {
                return Err(VoiceError::Unauthorized.into());
            }
            Some(
                history
                    .delivery()
                    .filter(|delivery| delivery.was_applied())
                    .cloned()
                    .ok_or(VoiceError::Capacity)?,
            )
        } else {
            None
        };
        // No callback-bearing history work or retained history destructor may
        // intervene between this fresh time observation and try-only admission.
        // The earlier prerequisite sample is never an installation permission.
        let (wall, mono) = observe_time(context)?;
        let registration = if let Some(row) = existing {
            let resumed = super::sdp::current(context, route, |admission| {
                row.resume_sdp_action(&parent, admission, &candidate, wall, mono)
            })?;
            match resumed {
                Some(registration) => registration,
                None => {
                    return Ok(Started::Immediate(Reply::History(
                        row.sdp_history(route.session().identity())?,
                    )))
                }
            }
        } else {
            match super::sdp::current(context, route, |admission| {
                ledger.begin_sdp_action(
                    &parent,
                    route.selected(),
                    admission,
                    &candidate,
                    wall,
                    mono,
                )
            })? {
                SdpBegin::Factory(registration) => registration,
                SdpBegin::Existing(row) => {
                    return Ok(Started::Immediate(Reply::History(
                        row.sdp_history(route.session().identity())?,
                    )))
                }
            }
        };
        let intent = candidate.intent().clone();
        let funding = candidate.funding().clone();
        (registration, intent, funding, delivery, seal)
    };
    let payload = if seal {
        Payload::Seal {
            body: incoming
                .sdp_body(intent, &funding)
                .map_err(|error| match error {
                    channel::Refused::Capacity => VoiceError::Capacity,
                    channel::Refused::Bounds => VoiceError::Bounds,
                    channel::Refused::Body | channel::Refused::Header => VoiceError::Malformed,
                    _ => VoiceError::Unauthorized,
                })?,
            delivery,
        }
    } else {
        Payload::Open
    };
    let preparation = Preparation::new(registration, physical, route.session(), payload)?;
    Ok(Started::Task(preparation.launch(context, route)?))
}
/// Complete inline Control adapter graph, composed with the canonical physical
/// Session/codec/IO inventory in channel::tests. Persistent rows and detached workers
/// retain their own original payments; this accounts for the real outer future/output
/// and the largest synchronous stage that can run inside it.
#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn measured<F: std::future::Future>(
        _: fn(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        eprintln!(
            "SDP actual future {}, output {}",
            std::mem::size_of::<F>(),
            std::mem::size_of::<F::Output>()
        );
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let current = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, crate::voice_authority::Coordinator>>(
        )
        + 3 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 128; // try-only guard/argument/time/closure scalars
               // These synchronous CALLER locals coexist with their nested helper. Candidate,
               // Registration, SubmissionOwners and their exact same original funding/Outcome
               // owners are already in the canonical fixed-row fee proof, not a second payment.
    let inspect = std::mem::size_of::<Request<'static>>()
        + 3 * std::mem::size_of::<ResolvedRow>()
        + 128
        + super::ledger::sdp_history_layout_bytes()
            .max(super::ledger::sdp_compaction_layout_bytes())
            .max(cleanup_layout_bytes());
    let start_caller = std::mem::size_of::<Request<'static>>()
        + 4 * std::mem::size_of::<ResolvedRow>()
        + std::mem::size_of::<Option<std::sync::Arc<catcoms_sync::voice_v2::sdp::SdpDelivery>>>()
        + 6 * std::mem::size_of::<u64>()
        + 96;
    // The Open Snapshot is produced by history (its return allocation is included there),
    // then remains beside current-admission. It is not already live during that history
    // invocation. Serialization starts only after all inspect/start/observer frames retire.
    let start = start_caller
        + super::ledger::sdp_history_layout_bytes()
            .max(std::mem::size_of::<SdpHistory>() + current);
    let query = catcoms_app::voice_context_queue_bytes()
        + std::mem::size_of::<catcoms_app::ServerActor>()
        + 64;
    let transfer = std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<catcoms_app::VoiceSdpBody>()
        + std::mem::size_of::<sha2::Sha256>()
        + 256;
    // Preparation's construction/launch/worker/output graph is independently covered by
    // its SAME original TaskCopy from its first fallible allocation. Only remaining caller
    // arguments surround it here; no task or fixed-row owner is counted as newly funded.
    let construction_caller = 128;
    let nested = inspect
        .max(start)
        .max(query)
        .max(transfer)
        .max(construction_caller)
        .max(super::sdp_replies::serialization_layout_bytes());
    let bytes = measured(apply) + nested;
    eprintln!(
        "SDP HTTP future/output {}; nested {nested}; total {bytes}",
        measured(apply)
    );
    bytes
}

/// ACK/settlement coexist with the inspect caller, not later history/serialization.
/// Persistent registered Action/receipt owners keep their original fees. Count the actual
/// handle, fixed status, SHA256/digest and Root Engine/argument wrappers explicitly here.
#[cfg(test)]
fn cleanup_layout_bytes() -> usize {
    std::mem::size_of::<std::sync::Arc<catcoms_sync::voice_v2::crypto_actions::CryptoAction>>()
        + std::mem::size_of::<catcoms_sync::voice_v2::crypto_actions::CryptoResultStatus>()
        + std::mem::size_of::<sha2::Sha256>()
        + 2 * std::mem::size_of::<[u8; 32]>()
        + std::mem::size_of::<Option<[u8; 32]>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 128
}
