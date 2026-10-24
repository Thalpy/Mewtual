//! One paid native Ready attempt. No wait-for-Ready sleep or new retry/deadline lives here;
//! Root and the original KeyClaim retain exact progress when this attempt returns.
use super::*;
use crate::voice_handoff::ledger::{ReadyContinuation, ReadyProgress};
use catcoms_rt::Clock;

pub(super) fn layout_bytes() -> usize {
    std::mem::size_of::<crate::voice_handoff::context::LiveContext>()
        + std::mem::size_of::<ReadyContinuation>()
        + std::mem::size_of::<crate::voice_handoff::transition::Admission<'static>>()
        + 512 // fixed helper choices/lease, exact guard/return wrappers; no route Vec
}
pub(super) async fn execute(worker: &Worker, mut cursor: u8) -> Result<RowId, Refused> {
    let context = worker.context.as_ref().ok_or(Refused::Closed)?;
    let route = &worker.custody.route;
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
    let mono = clock.monotonic_ms();
    let continuation = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        step(
            "original continuation",
            live.ledger.ready_continuation(
                route.selected(),
                route.session().identity(),
                &mut cursor,
                mono,
            ),
        )?
    };
    let commit = step("native preflight", preflight_binding(context, route).await)?;
    let commit = commit.lock_owned().await;
    let (actor, grant) = step("original Actor", observation_actor(context, route))?;
    let progress = step(
        "before Actor",
        current(context, route, &commit, &continuation, clock.as_ref(), None),
    )?;
    if !progress.core_drained() {
        let drained = if continuation.low {
            step(
                "LOW prepay",
                actor
                    .prepay_voice_low_sdp(grant.clone(), continuation.handshake)
                    .await,
            )?;
            // Unauthorized can mean remote Ready has not arrived. Return the attempt;
            // the original finite hint remains pending, with no UI guard held while idle.
            step(
                "LOW extract",
                actor
                    .prepare_voice_low_sdp(grant.clone(), continuation.handshake)
                    .await,
            )?
        } else {
            step(
                "HIGH prepare",
                actor
                    .prepare_voice_high_ready_connected(grant.clone(), continuation.handshake)
                    .await,
            )?
        };
        if !drained {
            return step("actual old drainage", Err::<RowId, _>(VoiceError::Capacity));
        }
        step(
            "after core preparation",
            current(
                context,
                route,
                &commit,
                &continuation,
                clock.as_ref(),
                Some(ReadyProgress::CoreDrained),
            ),
        )?;
    }
    if !continuation.low {
        // The old native Workflow REALLY drained before any new64KiB handoff may pay.
        // Queue only the original Root cache; lost replies recover its same Source record.
        step(
            "HIGH Source queue",
            actor
                .queue_voice_high_ready(grant, continuation.handshake)
                .await,
        )?;
        step(
            "after Source queue",
            current(
                context,
                route,
                &commit,
                &continuation,
                clock.as_ref(),
                Some(ReadyProgress::HighQueued),
            ),
        )?;
    }
    Ok(continuation.parent.id())
}
fn current(
    context: &Context,
    route: &Route,
    commit: &tokio::sync::OwnedMutexGuard<()>,
    continuation: &ReadyContinuation,
    clock: &dyn Clock,
    update: Option<ReadyProgress>,
) -> Result<ReadyProgress, Refused> {
    // Clock callbacks precede every final guard, including post-Actor observations.
    let mono = clock.monotonic_ms();
    let live = context.upgrade().ok_or(Refused::Closed)?;
    require_unlocked_now(&live, route.selected())?;
    let authority = match live.authority.try_lock() {
        Ok(authority) => authority,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity.into()),
    };
    require_current_selection(live.gate(), &authority, route.selected())?;
    let admission =
        crate::voice_handoff::transition::Admission::new(&live, &authority, route, commit);
    route
        .try_with_open(|| continuation.progress(&admission, mono, update))
        .map_err(Into::into)
}

/// Redacted local stage observations distinguish capacity/admission refusals from a missing
/// remote Ready. They contain neither peer identifiers, packet bodies nor crypto material.
fn step<T, E: Into<Refused>>(stage: &'static str, result: Result<T, E>) -> Result<T, Refused> {
    result.map_err(|error| {
        let error = error.into();
        tracing::trace!(stage, ?error, "native Ready attempt returned");
        #[cfg(test)]
        eprintln!("native Ready {stage}: {error:?}");
        error
    })
}
