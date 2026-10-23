//! Private native-origin Derive production. No browser Header/Incoming is manufactured.
//! Its complete preparation/task/output has a separate original payment before any await,
//! because an actual browser Control request may coexist on this Session.
use super::*;
use crate::voice_handoff::channel::NativeHold;

// Unlike the physical factory's4KiB publication-only task, this standalone task also pays
// its entire preflight/UI preparation, concurrent with an independently occupied browser
// Control envelope. Actual v4 measurement is4,456B; the provisional4KiB contract refused.
// Reserve8KiB once, as native Begin does, within the SAME512KiB group/2MiB process pool.
// This reduces available ordinary capacity; it adds no pool, lane, row or admission limit.
const NATIVE_TASK_BYTES: usize = 8 * 1024;

/// Field order is a shutdown invariant: actual payment/Route ownership ends before the
/// native position publishes idle. Work/Actor/grant/commit retire before this custody moves
/// into completed output. An uncollected output still owns all three fields.
struct NativeCustody {
    _credit: Credit,
    route: Route,
    _native: NativeHold,
}
pub(in crate::voice_handoff) struct NativeCompletion {
    result: Result<RowId, Refused>,
    _custody: NativeCustody,
}
impl NativeCompletion {
    /// Bounded public row metadata only. The original ledger remains the authority for
    /// attached action/input/permit/result and exact retries. A Pending row is not input.
    pub(in crate::voice_handoff) fn result(&self) -> Result<RowId, Refused> {
        self.result
    }
}
struct Worker {
    context: Option<Context>,
    selector: PairSelector,
    custody: NativeCustody,
}

/// Observation only: dropping a waiter detaches the actual task. Do not expose Tokio's
/// abort/abort_handle APIs across this boundary: an Actor command already queued by that
/// task can outlive its awaiting future and still owns the original publication ordering.
pub(in crate::voice_handoff) struct NativeWaiter {
    task: tokio::task::JoinHandle<NativeCompletion>,
}
impl std::future::Future for NativeWaiter {
    type Output = Result<NativeCompletion, Refused>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.get_mut().task)
            .poll(cx)
            .map(|result| result.map_err(|_| Refused::Stopped))
    }
}

/// Trusted native work producer only; not an IPC or listener command. The actual canonical
/// Route supplies selected identity and shutdown discovery. A lost JoinHandle detaches the
/// paid task, including preflight queries and UI waiting. No copied Session ID, optional
/// PhysicalHold, replacement action or renderer drain flag can create this admission.
pub(in crate::voice_handoff) fn produce_derivation(
    context: &Context,
    route: &Route,
    selector: PairSelector,
) -> Result<NativeWaiter, Refused> {
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| Refused::Stopped)?;
    {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        // Same selected allocation alone is insufficient: a second Registry can have a
        // live Route too. Only this Context's canonical index is visible to its shutdown.
        live.channels
            .require_cleanup_route(route)
            .map_err(|_| Refused::Closed)?;
    }
    let selected = route.selected();
    let credit = selected.resources().charge(
        selected.binding().scope.group,
        NATIVE_TASK_BYTES,
        ResourceKind::Retained,
    )?;
    route.require_open().map_err(|_| Refused::Closed)?;
    let native = route.session().native_hold().map_err(|error| match error {
        super::super::channel::Refused::Capacity => Refused::Protocol(VoiceError::Capacity),
        _ => Refused::Closed,
    })?;
    // The native task, not the browser's Control envelope, pays ALL preflight/UI/Actor
    // future and completed-output storage. There is no second charge at final admission.
    let future = run_native(Worker {
        context: Some(context.clone()),
        selector,
        custody: NativeCustody {
            _credit: credit,
            route: route.clone(),
            _native: native,
        },
    });
    let bytes = layout(&future);
    #[cfg(test)]
    eprintln!(
        "native producer peak {bytes}; future {}; output {}",
        std::mem::size_of_val(&future),
        std::mem::size_of::<NativeCompletion>()
    );
    if bytes > NATIVE_TASK_BYTES {
        return Err(Refused::Protocol(VoiceError::Capacity));
    }
    Ok(NativeWaiter {
        task: runtime.spawn(future),
    })
}

fn layout<F: std::future::Future<Output = NativeCompletion>>(future: &F) -> usize {
    let admission = std::mem::size_of::<super::super::context::LiveContext>()
        + std::mem::size_of::<Begin>()
        + std::mem::size_of::<Registration>()
        + std::mem::size_of::<Result<catcoms_sync::voice_v2::Context, String>>()
        + 128;
    let publication = std::mem::size_of::<super::super::context::LiveContext>()
        + std::mem::size_of::<super::super::ledger::Status>()
        + 128;
    worker_layout(future)
        // Replace the shared helper's bare handle with the complete opaque observer.
        // The latter contains that handle today; future metadata must stay counted too.
        - std::mem::size_of::<tokio::task::JoinHandle<NativeCompletion>>()
        + std::mem::size_of::<NativeWaiter>()
        // Both synchronous stages execute inside the SAME already-paid outer future.
        // Its full future/output remains additive; only non-overlapping scoped stages
        // may share their declared maximum. Include actual preflight reply/Begin/output.
        + admission.max(publication)
}
async fn run_native(worker: Worker) -> NativeCompletion {
    // Complete preparation/publication in one paid state machine. Returning an intermediate
    // Effect enum embedded a second full registration/output graph in this outer future
    // throughout publication. No such copied admission result needs to survive this await.
    let result = execute(&worker).await;
    NativeCompletion {
        result,
        _custody: worker.custody,
    }
}
async fn execute(worker: &Worker) -> Result<RowId, Refused> {
    let context = worker.context.as_ref().ok_or(Refused::Closed)?;
    let route = &worker.custody.route;
    let selected = route.selected().clone();
    let commit = preflight_binding(context, route).await?;
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
        // Same final order as the physical factory: UI commit -> Coordinator -> Route.
        // Both entry paths must repeat these checks after preflight and the UI wait.
        let authority = context.authority.lock().unwrap_or_else(|e| e.into_inner());
        require_current_selection(context.gate(), &authority, &selected)?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    let grant = selected.grant()?;
    let registration = match context
        .ledger
        .begin(selected.clone(), Purpose::Derive(worker.selector))?
    {
        Begin::Existing(id) => return Ok(id),
        Begin::Factory(registration) => registration,
    };
    drop(context);
    // Only this task's weak context remains; no mounted Store/registry/Coordinator crosses
    // Actor publication. Retain the original selected owner and UI commit to the end of
    // this scope, after the shared exact-publication path has collected its Actor replies.
    let result = publish_derivation(registration, actor, grant, worker.selector).await;
    drop(commit);
    drop(selected);
    result
}

#[cfg(test)]
mod tests;
