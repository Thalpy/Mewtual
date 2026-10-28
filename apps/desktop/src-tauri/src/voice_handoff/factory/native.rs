//! Private native-origin Derive production. No browser Header/Incoming is manufactured.
//! Its complete preparation/task/output has a separate original payment before any await,
//! because an actual browser Control request may coexist on this Session.
use super::*;
use crate::voice_handoff::channel::NativeHold;
use catcoms_sync::voice_v2::crypto_actions::{CryptoActionSnapshot, CryptoActionSnapshotPermit};
use catcoms_sync::voice_v2::presence::PeerSnapshotPermit;

enum Selection {
    Exact(PairSelector),
    Discover {
        cursor: u8,
        permit: PeerSnapshotPermit,
    },
    Confirmation {
        cursor: u8,
        incoming: Option<CryptoActionSnapshotPermit>,
    },
    Ready {
        cursor: u8,
    },
    Consumed,
}

// Unlike the physical factory's4KiB publication-only task, this standalone task also pays
// its entire preflight/UI preparation, concurrent with an independently occupied browser
// Control envelope. The complete discovery/confirmation/reconciliation graph is measured
// by layout at admission and in tests; a publication-only4KiB inventory is insufficient.
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
    // Negative test evidence belongs to this actual worker/output, never the lossy
    // global diagnostic table. A discovery that returned before selecting a peer
    // cannot have entered execute_selected, Begin, or publication.
    #[cfg(test)]
    discovery_refusal: Option<u8>,
    _custody: NativeCustody,
}
impl NativeCompletion {
    /// Bounded public row metadata only. The original ledger remains the authority for
    /// attached action/input/permit/result and exact retries. A Pending row is not input.
    pub(in crate::voice_handoff) fn result(&self) -> Result<RowId, Refused> {
        self.result
    }

    #[cfg(test)]
    pub(in crate::voice_handoff) fn refused_before_selection(&self, cursor: u8) -> bool {
        self.discovery_refusal == Some(cursor)
            && self.result == Err(Refused::Protocol(VoiceError::Capacity))
    }
}
struct Worker {
    context: Option<Context>,
    selector: Selection,
    #[cfg(test)]
    discovery_refusal: Option<u8>,
    #[cfg(test)]
    fixture_late_contention: bool,
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
    spawn(context, route, Selection::Exact(selector))
}

/// One original native position includes the discovery RPC as well as publication. A parked
/// Actor can occupy only this channel's paid worker, never the Host's round-robin scheduler
/// or a replacement channel. The additional affine4KiB payment follows the actual peer
/// snapshot command/output even if its observing future disappears.
pub(in crate::voice_handoff) fn discover_derivation(
    context: &Context,
    route: &Route,
    cursor: u8,
) -> Result<NativeWaiter, Refused> {
    let selected = route.selected();
    let permit = PeerSnapshotPermit::new(&selected.resources(), selected.binding().scope)?;
    spawn(context, route, Selection::Discover { cursor, permit })
}

/// Native-owned continuation selection. An incoming turn prepays its actual snapshot RPC
/// before spawning, while a local turn scans only original retained publications/results.
/// Both share this channel's one native position; no browser request is fabricated.
pub(in crate::voice_handoff) fn produce_confirmation(
    context: &Context,
    route: &Route,
    cursor: u8,
    incoming: bool,
) -> Result<NativeWaiter, Refused> {
    let selected = route.selected();
    let incoming = if incoming {
        Some(CryptoActionSnapshotPermit::new(
            &selected.resources(),
            selected.binding().scope,
        )?)
    } else {
        None
    };
    spawn(context, route, Selection::Confirmation { cursor, incoming })
}

/// Advance the original KeyClaim's finite Ready intent using this channel's SAME native
/// position/payment. A busy/refused nudge leaves the inline hint for fair maintenance.
pub(in crate::voice_handoff) fn produce_ready(
    context: &Context,
    route: &Route,
    cursor: u8,
) -> Result<NativeWaiter, Refused> {
    if !route.native_ready() {
        return Err(Refused::Closed);
    }
    spawn(context, route, Selection::Ready { cursor })
}

fn spawn(context: &Context, route: &Route, selector: Selection) -> Result<NativeWaiter, Refused> {
    #[cfg(test)]
    let (kind, cursor) = diagnostic_selection(&selector);
    let result = spawn_inner(context, route, selector);
    #[cfg(test)]
    route
        .session()
        .fixture_native_admission(kind, cursor, result.as_ref().err().copied());
    result
}

#[cfg(test)]
fn diagnostic_selection(selection: &Selection) -> (u8, u8) {
    match selection {
        Selection::Exact(_) => (0, u8::MAX),
        Selection::Discover { cursor, .. } => (1, *cursor),
        Selection::Confirmation { cursor, incoming } => {
            (if incoming.is_some() { 3 } else { 2 }, *cursor)
        }
        Selection::Ready { cursor } => (4, *cursor),
        Selection::Consumed => (5, u8::MAX),
    }
}

fn spawn_inner(
    context: &Context,
    route: &Route,
    selector: Selection,
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
    route.require_pilot_open().map_err(|_| Refused::Closed)?;
    let native = route.session().native_hold().map_err(|error| match error {
        super::super::channel::Refused::Capacity => Refused::Protocol(VoiceError::Capacity),
        _ => Refused::Closed,
    })?;
    // The native task, not the browser's Control envelope, pays ALL preflight/UI/Actor
    // future and completed-output storage. There is no second charge at final admission.
    let future = run_native(Worker {
        context: Some(context.clone()),
        selector,
        #[cfg(test)]
        discovery_refusal: None,
        #[cfg(test)]
        fixture_late_contention: false,
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
    let discovery = std::mem::size_of::<super::super::context::LiveContext>()
        + std::mem::size_of::<catcoms_sync::voice_v2::presence::PeerSnapshot>()
        + std::mem::size_of::<
            Result<(u64, &[catcoms_sync::voice_v2::presence::FreshPeer]), VoiceError>,
        >()
        + std::mem::size_of::<PairSelector>()
        + std::mem::size_of::<Option<super::super::ledger::ResolvedRow>>()
        + std::mem::size_of::<Result<Arc<CallGrant>, String>>()
        + 128; // native resolver/guard and argument wrappers, not the separately-paid Vec
    let confirmation_observation = std::mem::size_of::<super::super::context::LiveContext>()
        + std::mem::size_of::<CryptoActionSnapshot>()
        + std::mem::size_of::<super::super::ledger::Continuation>()
        + 128;
    let confirmation_admission = std::mem::size_of::<super::super::context::LiveContext>()
        + std::mem::size_of::<super::super::ledger::Continuation>()
        + std::mem::size_of::<Begin>()
        + std::mem::size_of::<Registration>()
        + 256; // bounded Index/parent/child, native wrappers and trigger observation
               // Respond reconciliation runs synchronously before its Begin/Registration exists.
               // Pay its actual Root snapshot/merge graph independently; neither a later value's
               // storage nor generic guard slack can substitute for this live observer inventory.
    let confirmation_reconciliation = std::mem::size_of::<super::super::context::LiveContext>()
        + std::mem::size_of::<super::super::ledger::Continuation>()
        + 2 * std::mem::size_of::<super::super::ledger::ResolvedRow>()
        + std::mem::size_of::<RowId>()
        + super::super::ledger::reconciliation_layout_bytes()
        + 128;
    worker_layout(future)
        // Replace the shared helper's bare handle with the complete opaque observer.
        // The latter contains that handle today; future metadata must stay counted too.
        - std::mem::size_of::<tokio::task::JoinHandle<NativeCompletion>>()
        + std::mem::size_of::<NativeWaiter>()
        // Both synchronous stages execute inside the SAME already-paid outer future.
        // Its full future/output remains additive; only non-overlapping scoped stages
        // may share their declared maximum. Include actual preflight reply/Begin/output.
        + admission.max(publication).max(discovery)
            .max(confirmation_observation).max(confirmation_admission)
            .max(confirmation_reconciliation)
            .max(ready::layout_bytes())
}
async fn run_native(mut worker: Worker) -> NativeCompletion {
    #[cfg(test)]
    worker
        .custody
        .route
        .session()
        .fixture_native_started(diagnostic_selection(&worker.selector).0);
    // Complete preparation/publication in one paid state machine. Returning an intermediate
    // Effect enum embedded a second full registration/output graph in this outer future
    // throughout publication. No such copied admission result needs to survive this await.
    let result = execute(&mut worker).await;
    #[cfg(test)]
    worker
        .custody
        .route
        .session()
        .fixture_native_finished(result.as_ref().err().copied());
    NativeCompletion {
        result,
        #[cfg(test)]
        discovery_refusal: worker.discovery_refusal,
        _custody: worker.custody,
    }
}
async fn execute(worker: &mut Worker) -> Result<RowId, Refused> {
    let selector = match std::mem::replace(&mut worker.selector, Selection::Consumed) {
        Selection::Exact(selector) => selector,
        Selection::Discover { cursor, permit } => {
            let result = discover(worker, cursor, permit).await;
            #[cfg(test)]
            if result == Err(Refused::Protocol(VoiceError::Capacity)) {
                worker.discovery_refusal = Some(cursor);
            }
            result?
        }
        Selection::Confirmation { cursor, incoming } => {
            return execute_confirmation(worker, cursor, incoming).await;
        }
        Selection::Ready { cursor } => return ready::execute(worker, cursor).await,
        Selection::Consumed => return Err(Refused::Closed),
    };
    #[cfg(test)]
    worker
        .custody
        .route
        .session()
        .fixture_native_selector(selector);
    execute_selected(worker, selector).await
}

async fn execute_confirmation(
    worker: &Worker,
    mut cursor: u8,
    incoming: Option<CryptoActionSnapshotPermit>,
) -> Result<RowId, Refused> {
    let context = worker.context.as_ref().ok_or(Refused::Closed)?;
    let route = &worker.custody.route;
    let selected = route.selected().clone();
    let continuation = if let Some(permit) = incoming {
        let (actor, grant) = observation_actor(context, route)?;
        let snapshot = actor.voice_confirmation_actions(grant, permit).await?;
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let mut found = None;
        for action in snapshot.result()? {
            if let Some(continuation) = live.ledger.incoming_confirmation(&selected, action) {
                found = Some(continuation);
                break;
            }
        }
        // Destroy the Vec/RPC owner before a child can borrow the chain-wide Phase loan.
        drop(snapshot);
        found
    } else {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        live.ledger
            .confirmation_continuation(&selected, &mut cursor)
    }
    .ok_or(VoiceError::Capacity)?;
    let commit = preflight_binding(context, route).await?;
    let commit = commit.lock_owned().await;
    let live = context.upgrade().ok_or(Refused::Closed)?;
    live.gate()
        .require_unlocked()
        .await
        .map_err(|_| Refused::Closed)?;
    let actor = {
        let servers = live.servers.lock().await;
        let binding = selected.binding();
        servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .map(|entry| entry.actor.clone())
            .ok_or(Refused::Closed)?
    };
    {
        // Same final ordering as the physical factory. No earlier scan or original
        // allocation can authorize first publication after End, lock, or replacement.
        let authority = live
            .authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        require_current_selection(live.gate(), &authority, &selected)?;
        route.require_pilot_open().map_err(|_| Refused::Closed)?;
    }
    let grant = selected.grant()?;
    let handshake = continuation.parent.confirmation_handshake()?;
    let (begin, trigger) = if let Some(child) = &continuation.known {
        (
            live.ledger.resume_known_child(
                selected.clone(),
                &continuation.parent,
                child,
                continuation.request,
            )?,
            None,
        )
    } else {
        live.ledger
            .begin_child(selected.clone(), &continuation.parent, continuation.request)?
    };
    let registration = match begin {
        Begin::Existing(id) => return Ok(id),
        Begin::Factory(registration) => registration,
    };
    drop(live);
    let result = super::confirmation::publish_confirmation(
        registration,
        actor,
        grant,
        handshake,
        continuation.request,
        continuation.incoming.or(trigger),
    )
    .await;
    drop(commit);
    drop(selected);
    result
}

/// Resolve only the genuine original Actor/grant for a non-effect observation. Do not
/// hold an upgraded mounted store, server map, or Coordinator over the Actor RPC.
pub(in crate::voice_handoff) fn observation_actor(
    context: &Context,
    route: &Route,
) -> Result<(ServerActor, Arc<CallGrant>), Refused> {
    let selected = route.selected();
    let live = context.upgrade().ok_or(Refused::Closed)?;
    if !live.try_unlocked()? {
        return Err(Refused::Closed);
    }
    let servers = live.servers.try_lock().map_err(|_| VoiceError::Capacity)?;
    let binding = selected.binding();
    let actor = servers
        .get(&binding.server)
        .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
        .map(|entry| entry.actor.clone())
        .ok_or(Refused::Closed)?;
    Ok((actor, selected.grant()?))
}

async fn discover(
    worker: &Worker,
    cursor: u8,
    permit: PeerSnapshotPermit,
) -> Result<PairSelector, Refused> {
    let context = worker.context.as_ref().ok_or(Refused::Closed)?;
    let route = &worker.custody.route;
    let selected = route.selected();
    let (actor, grant) = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        #[cfg(test)]
        route.session().fixture_native_stage(7);
        if !live.try_unlocked()? {
            return Err(Refused::Closed);
        }
        #[cfg(test)]
        route.session().fixture_native_stage(8);
        if !live.ledger.automatic_parent_available(selected)? {
            return Err(VoiceError::Capacity.into());
        }
        #[cfg(test)]
        route.session().fixture_native_stage(9);
        let servers = live.servers.try_lock().map_err(|_| VoiceError::Capacity)?;
        let binding = selected.binding();
        let actor = servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .map(|entry| entry.actor.clone())
            .ok_or(Refused::Closed)?;
        (actor, selected.grant()?)
    }; // No upgraded mounted Store, server map or Coordinator crosses the Actor wait.
    #[cfg(test)]
    route.session().fixture_native_stage(2);
    let snapshot = actor.voice_participant_peer_snapshot(grant, permit).await?;
    let (epoch, peers) = snapshot.result()?;
    let live = context.upgrade().ok_or(Refused::Closed)?;
    // Already-known purposes are handled by the scheduler's separate Pending turn. Never
    // replace an expired original selector with this observation's later epoch or proof.
    for offset in 0..peers.len() {
        let peer = &peers[(usize::from(cursor) + offset) % peers.len()];
        if let catcoms_sync::voice_v2::Body::Announcement { key, .. } = peer.announcement().body() {
            let selector = PairSelector {
                peer: peer.announcement().issuer(),
                join: key.join,
                key_ref: key.key_ref,
                epoch,
            };
            if !live.ledger.derivation_known(selected, selector) {
                // Only immutable fixed metadata leaves this scope. The actual Vec/proof
                // observation and permit drop before pair/Phase/ledger publication begins.
                return Ok(selector);
            }
        }
    }
    Err(VoiceError::Unauthorized.into())
}

async fn execute_selected(worker: &Worker, selector: PairSelector) -> Result<RowId, Refused> {
    let context = worker.context.as_ref().ok_or(Refused::Closed)?;
    let route = &worker.custody.route;
    let selected = route.selected().clone();
    #[cfg(test)]
    route.session().fixture_native_stage(3);
    let commit = preflight_binding(context, route).await?;
    #[cfg(test)]
    route.session().fixture_native_stage(4);
    let commit = commit.lock_owned().await;
    let context = context.upgrade().ok_or(Refused::Closed)?;
    #[cfg(test)]
    route.session().fixture_native_stage(10);
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
        #[cfg(test)]
        route.session().fixture_native_stage(11);
        let authority = context.authority.lock().unwrap_or_else(|e| e.into_inner());
        require_current_selection(context.gate(), &authority, &selected)?;
        route.require_pilot_open().map_err(|_| Refused::Closed)?;
    }
    let grant = selected.grant()?;
    // Recheck AFTER the Actor/UI waits, before any new counted Pending or Root
    // publication. Competing native/browser factories share this same UI commit.
    // Classify exact known work first: do not strand its original attachment/retry.
    let known = context.ledger.derivation_known(&selected, selector);
    // Negative-only fixture pressure must come AFTER the blocking known lookup.
    // With !known, the very next production check is the nonblocking Index check;
    // this guard can only cause Capacity, then drops before Begin. In particular
    // never hold Index through a known-work path or any blocking Index acquisition.
    let available = known || {
        #[cfg(test)]
        let _pressure = if worker.fixture_late_contention {
            Some(context.ledger.fixture_try_hold_index()?)
        } else {
            None
        };
        // Keep the test guard in this synchronous block: no MutexGuard can
        // survive into Begin or an asynchronous publication future.
        context.ledger.automatic_parent_available(&selected)?
    };
    if !available {
        return Err(VoiceError::Capacity.into());
    }
    #[cfg(test)]
    route.session().fixture_native_stage(12);
    let registration = match context
        .ledger
        .begin(selected.clone(), Purpose::Derive(selector))?
    {
        Begin::Existing(id) => return Ok(id),
        Begin::Factory(registration) => registration,
    };
    drop(context);
    // Only this task's weak context remains; no mounted Store/registry/Coordinator crosses
    // Actor publication. Retain the original selected owner and UI commit to the end of
    // this scope, after the shared exact-publication path has collected its Actor replies.
    #[cfg(test)]
    route.session().fixture_native_stage(5);
    let result = publish_derivation(registration, actor, grant, selector).await;
    drop(commit);
    drop(selected);
    result
}

mod ready;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(in crate::voice_handoff) use tests::prove_discovery_late_refusal;
