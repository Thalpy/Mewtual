//! Reconcile a joined row from its explicitly approved connected contacts.
//! Each peer gets at most one bounded page or original exchange per turn.
use super::*;
use catcoms_app::store::{
    NativeTextJoinedArchiveDependency, NativeTextJoinedArchiveReplay,
    NativeTextJoinedDependencyExchange, NativeTextJoinedDependencyResponse,
    NativeTextJoinedHistoryPage,
};
use catcoms_replication::chat::ActionId;
use catcoms_rt::TransportError;
use std::collections::VecDeque;
use std::time::Duration;

const TURN: Duration = Duration::from_millis(500);
const REPLAY_TURN: Duration = Duration::from_millis(100);
const REQUEST: Duration = Duration::from_secs(3);
const PAGES_PER_SLICE: u8 = 4;
const RETRY_MS: u64 = 5_000;
const RESCAN_MS: u64 = 30_000;
const REPLAY_MS: u64 = 15_000;
const MAX_ARCHIVE_SWEEP_PAGES: u16 = 256;
const MAX_ARCHIVE_CONTINUATION_FAILURES: u8 = 3;
const INITIAL_PAGE_RETRY_LIMIT: u8 = 3;
const MAX_ARCHIVE_ANCESTORS: usize = 8;
const MAX_ARCHIVE_ANCESTRY_REQUESTS: u8 = 24;
const ARCHIVE_ANCESTRY_MS: u64 = 45_000;
const EXACT_ENTRY_MS: u64 = 15_000;

struct ParkedOriginal {
    action: ActionId,
    work: ParkedWork,
    deadline: u64,
}

/// One paid lane either holds the exact wire response pending local
/// admission or a staged original. Busy cannot start another wire request.
enum ParkedWork {
    Reply {
        response: Box<NativeTextJoinedDependencyResponse>,
        expected: Option<NativeTextJoinedArchiveDependency>,
    },
    Replay(Box<NativeTextJoinedArchiveReplay>),
}

#[derive(Clone)]
enum ExactRequest {
    Offered(ActionId),
    Parent {
        proof: NativeTextJoinedArchiveDependency,
        holder_outbox: bool,
    },
}

impl ExactRequest {
    fn action(&self) -> ActionId {
        match self {
            Self::Offered(action) => *action,
            Self::Parent { proof, .. } => proof.parent_data(),
        }
    }
}

#[derive(Clone)]
struct PlannedOriginal {
    request: ExactRequest,
    expected: Option<NativeTextJoinedArchiveDependency>,
}

/// Only IDs and original digests are held between requests. Each suspended
/// child's paid stage is dropped before asking for its missing parent.
struct ArchiveAncestry {
    stack: Vec<PlannedOriginal>,
    requests: u8,
    deadline: u64,
}

impl ArchiveAncestry {
    fn new(action: ActionId) -> Self {
        let mut stack = Vec::with_capacity(MAX_ARCHIVE_ANCESTORS + 1);
        stack.push(PlannedOriginal {
            request: ExactRequest::Offered(action),
            expected: None,
        });
        Self {
            stack,
            requests: 0,
            deadline: SystemClock
                .monotonic_ms()
                .saturating_add(ARCHIVE_ANCESTRY_MS),
        }
    }

    fn note_missing(&mut self, proof: NativeTextJoinedArchiveDependency) -> bool {
        if self.stack.len() > MAX_ARCHIVE_ANCESTORS
            || self
                .stack
                .iter()
                .any(|step| step.request.action() == proof.parent_data())
        {
            return false;
        }
        self.stack
            .last_mut()
            .expect("active ancestry original")
            .expected = Some(proof.clone());
        self.stack.push(PlannedOriginal {
            request: ExactRequest::Parent {
                proof,
                holder_outbox: false,
            },
            expected: None,
        });
        true
    }
}

struct PeerSweep {
    peer: PeerId,
    archive: bool,
    page: Option<Arc<NativeTextJoinedHistoryPage>>,
    next_entry: usize,
    deferred: bool,
    pages: u8,
    total_pages: u16,
    continuation_failures: u8,
    initial_page_failures: u8,
    exact_retry: bool,
    exact_deadline: Option<u64>,
    ancestry: Option<ArchiveAncestry>,
    due: u64,
}

impl PeerSweep {
    fn new(peer: PeerId, archive: bool) -> Self {
        Self {
            peer,
            archive,
            page: None,
            next_entry: 0,
            deferred: false,
            pages: 0,
            total_pages: 0,
            continuation_failures: 0,
            initial_page_failures: 0,
            exact_retry: false,
            exact_deadline: None,
            ancestry: None,
            due: 0,
        }
    }

    fn rescan_after(&mut self, delay: u64) {
        self.page = None;
        self.next_entry = 0;
        self.deferred = false;
        self.pages = 0;
        self.total_pages = 0;
        self.continuation_failures = 0;
        self.initial_page_failures = 0;
        self.exact_retry = false;
        self.exact_deadline = None;
        self.ancestry = None;
        self.due = SystemClock.monotonic_ms().saturating_add(delay);
    }

    fn finish_original(&mut self, covered: bool) {
        self.exact_retry = false;
        self.exact_deadline = None;
        if let Some(ancestry) = self.ancestry.as_mut().filter(|_| covered) {
            ancestry.stack.pop();
            if !ancestry.stack.is_empty() {
                return;
            }
        }
        self.ancestry = None;
        self.next_entry += 1;
        self.deferred |= !covered;
    }

    fn missing_original(&mut self, action: ActionId, proof: NativeTextJoinedArchiveDependency) {
        self.exact_retry = false;
        self.exact_deadline = None;
        let ancestry = self
            .ancestry
            .get_or_insert_with(|| ArchiveAncestry::new(action));
        if !ancestry.note_missing(proof) {
            self.finish_original(false);
        }
    }

    /// Retry only an authenticated empty archive result against the
    /// holder's separate own-outbox authority.
    fn holder_archive_empty(&mut self) {
        self.exact_retry = false;
        self.exact_deadline = None;
        match self
            .ancestry
            .as_mut()
            .and_then(|chain| chain.stack.last_mut())
            .map(|step| &mut step.request)
        {
            Some(ExactRequest::Parent { holder_outbox, .. }) if !*holder_outbox => {
                *holder_outbox = true;
            }
            _ => self.finish_original(false),
        }
    }
}

pub(super) async fn recover<R: tauri::Runtime>(app: &tauri::AppHandle<R>, source: &Arc<Source>) {
    let server_table::NativeSelection::Joined(row) = &source.selection else {
        return;
    };
    let mut scope = source.negative();
    let mut peers: VecDeque<PeerSweep> = VecDeque::new();
    let mut pending_cursor = 0;
    let mut replay: Option<(PeerSweep, ParkedOriginal)> = None;
    let mut catalogue_due = 0;
    let mut peer_due = 0;
    loop {
        // A checked original gets a short local turn while its selected
        // route is still live. A stalled admission still waits at most TURN
        // and a peer without outstanding originals uses the regular pace.
        let delay = if replay.is_some() { REPLAY_TURN } else { TURN };
        tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            _ = SystemClock.sleep(delay) => {}
        }
        let state = app.state::<AppState>();
        if let Some((mut sweep, parked)) = replay.take() {
            let mut parked = Some(parked);
            if let Some(outcome) = resume_entry(&state, source, &mut parked).await {
                let notice = match outcome {
                    ReplayOutcome::Missing(action, proof) => {
                        #[cfg(test)]
                        eprintln!("chat history worker found a missing exact ancestor");
                        sweep.missing_original(action, proof);
                        None
                    }
                    ReplayOutcome::Selected(target) => {
                        #[cfg(test)]
                        eprintln!("chat history worker selected an exact original");
                        sweep.finish_original(true);
                        Some(target)
                    }
                    ReplayOutcome::Empty => {
                        sweep.holder_archive_empty();
                        None
                    }
                    ReplayOutcome::Deferred(notice) => {
                        sweep.finish_original(false);
                        notice
                    }
                };
                peers.push_back(sweep);
                if let Some(target) = notice {
                    source.notify_received(app, &state, Some(target)).await;
                }
            } else if let Some(parked) = parked {
                replay = Some((sweep, parked));
            }
            // One paid continuation per row. Resume one batch and leave room
            // for inventory and reconciliation work while it is parked.
        }
        // No other foreground wait can outlive the active paid continuation.
        let parked_deadline = replay.as_ref().map(|(_, parked)| parked.deadline);
        if let Some(target) =
            reconcile_archive_change(&state, source, &mut pending_cursor, parked_deadline).await
        {
            source.notify_received(app, &state, Some(target)).await;
        }
        let Some(candidates) = row.history_peer_candidates() else {
            continue;
        };
        peers.retain(|sweep| {
            let approved = candidates.contains(&sweep.peer);
            #[cfg(test)]
            if !approved && sweep.page.is_some() {
                eprintln!("chat history worker dropped a page when its peer approval disappeared");
            }
            approved
        });
        let now = SystemClock.monotonic_ms();
        if now < peer_due {
            continue;
        }
        if !candidates.is_empty() && catalogue_due <= now {
            let channel = source.channel();
            let updated = with_row_until(
                &state,
                source,
                parked_deadline,
                move |original, row, _, scope| {
                    original.maintain_archive_relay_catalogue(&row, channel, &scope)
                },
            )
            .await;
            let delay = if matches!(updated, Some(Ok(false))) {
                0
            } else {
                RETRY_MS
            };
            catalogue_due = SystemClock.monotonic_ms().saturating_add(delay);
        }
        for peer in candidates {
            for archive in [false, true] {
                let parked = replay
                    .as_ref()
                    .is_some_and(|(sweep, _)| sweep.peer == peer && sweep.archive == archive);
                if !parked
                    && !peers
                        .iter()
                        .any(|sweep| sweep.peer == peer && sweep.archive == archive)
                {
                    peers.push_back(PeerSweep::new(peer, archive));
                }
            }
        }
        let now = SystemClock.monotonic_ms();
        let Some(index) = peers.iter().position(|sweep| sweep.due <= now) else {
            continue;
        };
        // Local replay can wake early without increasing peer request cadence.
        peer_due = now.saturating_add(TURN.as_millis() as u64);
        let mut sweep = peers.remove(index).expect("selected due history peer");
        let mut parked = None;
        let _ = turn(&state, source, &mut sweep, parked_deadline, &mut parked).await;
        if let Some(parked) = parked {
            replay = Some((sweep, parked));
        } else {
            peers.push_back(sweep);
        }
    }
}

async fn reconcile_archive_change(
    state: &AppState,
    source: &Arc<Source>,
    cursor: &mut usize,
    deadline: Option<u64>,
) -> Option<ActionId> {
    let pending = source.member_work().pending_archive_changes();
    // This point lookup may finish a durable WAL. It is scoped to the
    // installed local row, even if the peer that supplied the action left.
    let index = (0..pending.len())
        .map(|offset| (*cursor + offset) % pending.len())
        .find(|index| pending[*index].is_some())?;
    *cursor = (index + 1) % pending.len();
    let change = pending[index]?;
    let channel = source.channel();
    with_row_until(
        state,
        source,
        deadline,
        move |original, row, local, scope| {
            let selected = original
                .selected_archive_target_data(&row, channel, change.action, &scope)
                .ok()?;
            local.reconcile_archive_change(change, selected)
        },
    )
    .await
    .flatten()
}

async fn turn(
    state: &AppState,
    source: &Arc<Source>,
    sweep: &mut PeerSweep,
    parked_deadline: Option<u64>,
    parked: &mut Option<ParkedOriginal>,
) -> Option<()> {
    let page = sweep.page.clone();
    if let Some(page) = &page {
        if !page.is_current_cached() {
            #[cfg(test)]
            eprintln!(
                "chat history worker dropped an expired page (row, route, authority): {:?}",
                page.current_checks_for_test()
            );
            sweep.rescan_after(RETRY_MS);
            return None;
        }
        if let Some(action) = page.action_at(sweep.next_entry) {
            if parked_deadline.is_some() {
                return None;
            }
            let now = SystemClock.monotonic_ms();
            let mut deadline = *sweep
                .exact_deadline
                .get_or_insert_with(|| now.saturating_add(EXACT_ENTRY_MS));
            if now >= deadline {
                sweep.finish_original(false);
                return None;
            }
            let planned = if let Some(ancestry) = &mut sweep.ancestry {
                if ancestry.requests == MAX_ARCHIVE_ANCESTRY_REQUESTS || now >= ancestry.deadline {
                    sweep.finish_original(false);
                    return None;
                }
                ancestry.requests += 1;
                deadline = deadline.min(ancestry.deadline);
                ancestry.stack.last().expect("active original").clone()
            } else {
                PlannedOriginal {
                    request: ExactRequest::Offered(action),
                    expected: None,
                }
            };
            #[cfg(test)]
            eprintln!(
                "chat history worker preparing {} entry {}",
                if sweep.archive {
                    "archive"
                } else {
                    "own-outbox"
                },
                sweep.next_entry,
            );
            match fetch_entry(state, source, page.clone(), planned, Some(deadline), parked).await {
                Some(EntryResult::Covered) => sweep.finish_original(true),
                Some(EntryResult::Empty) => sweep.holder_archive_empty(),
                Some(EntryResult::Deferred) => sweep.finish_original(false),
                Some(EntryResult::TryAgain) if !sweep.exact_retry => {
                    sweep.exact_retry = true;
                    sweep.due = SystemClock
                        .monotonic_ms()
                        .saturating_add(TURN.as_millis() as u64);
                }
                Some(EntryResult::TryAgain) => sweep.finish_original(false),
                Some(EntryResult::LocallyBusy) => {
                    // The driver never owned this request, so the same
                    // wire retry is available when lower custody retires.
                    if let Some(ancestry) = &mut sweep.ancestry {
                        ancestry.requests -= 1;
                    }
                    sweep.due = now.saturating_add(TURN.as_millis() as u64);
                }
                None => {}
            }
            return None;
        }
    }
    if sweep.archive && sweep.total_pages == MAX_ARCHIVE_SWEEP_PAGES {
        sweep.rescan_after(if sweep.deferred { RETRY_MS } else { RESCAN_MS });
        return None;
    }
    if sweep.pages == PAGES_PER_SLICE {
        // Bound continuous scans even for a member advertising an inflated
        // end. Preserve the actual cursor for the next slice.
        sweep.pages = 0;
        sweep.due = SystemClock.monotonic_ms().saturating_add(RETRY_MS);
        return None;
    }
    let peer = sweep.peer;
    let archive = sweep.archive;
    let channel = source.channel();
    let request = with_row_until(
        state,
        source,
        parked_deadline,
        move |original, row, _, scope| match page {
            Some(page) => {
                original.prepare_history_continuation(&row, &page, scope, &mut OsCryptoRng)
            }
            None if archive => original
                .prepare_archive_history_exchange(&row, channel, peer, scope, &mut OsCryptoRng)
                .map(Some),
            None => original
                .prepare_history_exchange(&row, channel, peer, scope, &mut OsCryptoRng)
                .map(Some),
        },
    )
    .await?;
    let exchange = match request {
        Ok(Some(exchange)) => exchange,
        Ok(None) => {
            sweep.rescan_after(if sweep.deferred { RETRY_MS } else { RESCAN_MS });
            return None;
        }
        Err(V2MemberSaveRefusal::Busy) => return None,
        Err(failure) => {
            #[cfg(test)]
            eprintln!("chat history worker could not prepare page: {failure:?}");
            let _ = failure;
            sweep.rescan_after(RETRY_MS);
            return None;
        }
    };
    let request_time = time_remaining(parked_deadline, REQUEST);
    if request_time < REQUEST {
        // A carried reply near its own deadline must not give an unrelated
        // provider a partial response window and count it as a failure.
        return None;
    }
    let requested = tokio::time::timeout(request_time, exchange.receive()).await;
    if matches!(&requested, Ok(Err(TransportError::Busy))) {
        sweep.due = SystemClock
            .monotonic_ms()
            .saturating_add(TURN.as_millis() as u64);
        return None;
    }
    let Ok(Ok(reply)) = requested else {
        #[cfg(test)]
        eprintln!("chat history worker page request stopped: {requested:?}");
        // Retain the exact earlier cursor on refused or lost replies. Slow
        // peers still yield their turn after each bounded network wait.
        if sweep.archive && sweep.page.is_some() {
            sweep.continuation_failures += 1;
            if sweep.continuation_failures == MAX_ARCHIVE_CONTINUATION_FAILURES {
                // The holder may have expired the original sweep. After a
                // few exact retries start again with a fresh first request.
                sweep.rescan_after(RETRY_MS);
                return None;
            }
        }
        let delay = if sweep.total_pages == 0 {
            // An archive may still be building its catalogue. A member with
            // no local outbox also cannot prove an empty selected page.
            // Give both a few quick retries before the normal rescan.
            sweep.initial_page_failures =
                (sweep.initial_page_failures + 1).min(INITIAL_PAGE_RETRY_LIMIT);
            if sweep.initial_page_failures == INITIAL_PAGE_RETRY_LIMIT {
                RESCAN_MS
            } else {
                RETRY_MS
            }
        } else {
            RETRY_MS
        };
        sweep.due = SystemClock.monotonic_ms().saturating_add(delay);
        return None;
    };
    let page = with_row_until(
        state,
        source,
        parked_deadline,
        move |original, row, _, scope| original.open_history_response(&row, reply, &scope),
    )
    .await?;
    match page {
        Ok(page) => {
            #[cfg(test)]
            eprintln!(
                "chat history worker opened {} page with {} entries",
                if sweep.archive {
                    "an archive"
                } else {
                    "an own-outbox"
                },
                (0..8)
                    .filter(|index| page.action_at(*index).is_some())
                    .count()
            );
            sweep.page = Some(Arc::new(page));
            sweep.next_entry = 0;
            sweep.pages += 1;
            sweep.total_pages = sweep.total_pages.saturating_add(1);
            sweep.continuation_failures = 0;
            sweep.initial_page_failures = 0;
        }
        Err(V2MemberSaveRefusal::Busy) => {}
        Err(failure) => {
            #[cfg(test)]
            eprintln!("chat history worker refused page: {failure:?}");
            let _ = failure;
            sweep.rescan_after(RETRY_MS);
        }
    }
    Some(())
}

enum Fetch {
    Covered,
    Request(NativeTextJoinedDependencyExchange),
}

enum EntryResult {
    Covered,
    Empty,
    Deferred,
    TryAgain,
    LocallyBusy,
}

async fn fetch_entry(
    state: &AppState,
    source: &Arc<Source>,
    page: Arc<NativeTextJoinedHistoryPage>,
    planned: PlannedOriginal,
    ancestry_deadline: Option<u64>,
    parked: &mut Option<ParkedOriginal>,
) -> Option<EntryResult> {
    let action = planned.request.action();
    let request = planned.request.clone();
    let channel = source.channel();
    let check_local = planned.expected.is_none();
    let fetch = with_row_until(
        state,
        source,
        ancestry_deadline,
        move |original, row, _, scope| {
            if !page.is_current_cached() {
                return Err(V2MemberSaveRefusal::Stale);
            }
            let local = if !check_local {
                None
            } else if matches!(&request, ExactRequest::Offered(_)) {
                original.selected_discovered_target(&row, &page, action, &scope)?
            } else {
                original.selected_archive_target_data(&row, channel, action, &scope)?
            };
            if local.is_some() && page.is_current_cached() {
                return Ok(Fetch::Covered);
            }
            match request {
                ExactRequest::Offered(action) => original.prepare_discovered_original(
                    &row,
                    &page,
                    action,
                    scope,
                    &mut OsCryptoRng,
                ),
                ExactRequest::Parent {
                    proof,
                    holder_outbox,
                } => original.prepare_archive_parent_request(
                    &row,
                    &page,
                    &proof,
                    holder_outbox,
                    scope,
                    &mut OsCryptoRng,
                ),
            }
            .map(Fetch::Request)
        },
    )
    .await?;
    let exchange = match fetch {
        Ok(Fetch::Covered) => return Some(EntryResult::Covered),
        Ok(Fetch::Request(exchange)) => exchange,
        Err(V2MemberSaveRefusal::Busy) => return None,
        Err(failure) => {
            #[cfg(test)]
            eprintln!("chat history worker could not prepare exact fetch: {failure:?}");
            let _ = failure;
            return Some(EntryResult::Deferred);
        }
    };
    let request_time = time_remaining(ancestry_deadline, REQUEST);
    if request_time.is_zero() {
        return Some(EntryResult::Deferred);
    }
    let requested = tokio::time::timeout(request_time, exchange.receive()).await;
    if matches!(&requested, Ok(Err(TransportError::Busy))) {
        return Some(EntryResult::LocallyBusy);
    }
    #[cfg(test)]
    let probe = state
        .native_text
        .join_history_reply_probe
        .lock()
        .unwrap()
        .clone()
        .filter(|probe| probe.action == action);
    #[cfg(test)]
    if let Some(probe) = &probe {
        probe.attempts.fetch_add(1, Ordering::SeqCst);
    }
    let Ok(Ok(reply)) = requested else {
        #[cfg(test)]
        eprintln!("chat history worker exact request stopped: {requested:?}");
        return Some(EntryResult::TryAgain);
    };
    let deadline = SystemClock.monotonic_ms().saturating_add(REPLAY_MS);
    *parked = Some(ParkedOriginal {
        action,
        work: ParkedWork::Reply {
            response: Box::new(reply),
            expected: planned.expected,
        },
        deadline: ancestry_deadline.map_or(deadline, |earlier| earlier.min(deadline)),
    });
    #[cfg(test)]
    if let Some(probe) = probe {
        probe.edge.wait_once().await;
    }
    // Preserve the usual fast path. If local admission is busy this leaves
    // the paid reply parked instead of asking the peer for the same bytes.
    match resume_entry(state, source, parked).await {
        Some(ReplayOutcome::Empty) => Some(EntryResult::Empty),
        Some(ReplayOutcome::Deferred(_)) => Some(EntryResult::Deferred),
        Some(ReplayOutcome::Missing(..) | ReplayOutcome::Selected(..)) => {
            unreachable!("opening a reply cannot select an archive record")
        }
        None => None,
    }
}

enum ReplayOutcome {
    Missing(ActionId, NativeTextJoinedArchiveDependency),
    Selected(ActionId),
    Empty,
    Deferred(Option<ActionId>),
}

async fn resume_entry(
    state: &AppState,
    source: &Arc<Source>,
    parked: &mut Option<ParkedOriginal>,
) -> Option<ReplayOutcome> {
    let original = parked.as_ref()?;
    let remaining = original.deadline.saturating_sub(SystemClock.monotonic_ms());
    if remaining == 0
        || matches!(&original.work, ParkedWork::Reply { response, .. } if !response.is_current_cached())
    {
        *parked = None;
        return Some(ReplayOutcome::Deferred(None));
    }
    // Acquire both before moving the paid continuation. Contention must
    // preserve it while foreground history or signing uses this row.
    let admitted = tokio::time::timeout(Duration::from_millis(remaining).min(TURN), async {
        let permit = source.member_work.foreground(source.negative()).await?;
        let (original, row) = source.capture_joined(state).await?;
        Some((permit, original, row))
    })
    .await;
    let (permit, original, row) = match admitted {
        Ok(value) => value?,
        Err(_) if remaining <= TURN.as_millis() as u64 => {
            *parked = None;
            return Some(ReplayOutcome::Deferred(None));
        }
        Err(_) => {
            #[cfg(test)]
            if matches!(&original.work, ParkedWork::Reply { .. }) {
                state.native_text.join_history_staging_wait.notify_one();
            }
            return None;
        }
    };
    let mut work = parked.take()?;
    let source = source.clone();
    let processing = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if SystemClock.monotonic_ms() >= work.deadline {
            return (work, Some(ReplayOutcome::Deferred(None)));
        }
        let scope = source.negative();
        let local = source.member_work();
        if let ParkedWork::Reply { response, expected } = &work.work {
            let replay =
                match original.prepare_parked_discovered_archive(&row, response, scope.clone()) {
                    Ok(Some(replay)) => replay,
                    Ok(None) => return (work, Some(ReplayOutcome::Empty)),
                    Err(V2MemberSaveRefusal::Busy) => return (work, None),
                    Err(failure) => {
                        #[cfg(test)]
                        eprintln!(
                            "chat history worker could not stage exact original: {failure:?}"
                        );
                        let _ = failure;
                        return (work, Some(ReplayOutcome::Deferred(None)));
                    }
                };
            if expected
                .as_ref()
                .is_some_and(|proof| !proof.matches_child(&replay))
            {
                return (work, Some(ReplayOutcome::Deferred(None)));
            }
            // The wire buffer retires now. The stage owns a negative of that
            // exact route and the invocation needed until archive selection.
            work.work = ParkedWork::Replay(Box::new(replay));
            return (work, None);
        }
        let ParkedWork::Replay(replay) = &mut work.work else {
            unreachable!("the exact response was staged");
        };
        let target = replay.target_data();
        let action = work.action;
        let ready = original.advance_discovered_archive(replay, &scope, &mut OsCryptoRng);
        match ready {
            Ok(false) | Err(V2MemberSaveRefusal::Busy) => return (work, None),
            Err(V2MemberSaveRefusal::Unavailable) => {
                let missing = replay
                    .missing_archive_dependency()
                    .map(|proof| ReplayOutcome::Missing(action, proof));
                return (work, Some(missing.unwrap_or(ReplayOutcome::Deferred(None))));
            }
            Err(_) => return (work, Some(ReplayOutcome::Deferred(None))),
            Ok(true) => {}
        }
        if SystemClock.monotonic_ms() >= work.deadline {
            return (work, Some(ReplayOutcome::Deferred(None)));
        }
        // Reserve before any write. Even a Stale result can mean the disk
        // selected the original and the remote route then retired.
        if local.reserve_archive_change(action, target).is_none() {
            // Yield this exact entry so the next turn can reconcile one of
            // the existing debts and make room for the later sweep.
            return (work, Some(ReplayOutcome::Deferred(None)));
        }
        let result = match original.select_discovered_archive(replay, &scope, &mut OsCryptoRng) {
            Ok(Some(_)) => {
                local.note_archive_change(action, target, true);
                ReplayOutcome::Selected(target)
            }
            other => {
                #[cfg(test)]
                eprintln!(
                    "chat history worker selection was incomplete: {:?}",
                    other.as_ref().err()
                );
                let _ = other;
                ReplayOutcome::Deferred(Some(target))
            }
        };
        (work, Some(result))
    })
    .await;
    let Ok((work, result)) = processing else {
        return Some(ReplayOutcome::Deferred(None));
    };
    if result.is_none() {
        *parked = Some(work);
    }
    result
}

fn time_remaining(deadline: Option<u64>, maximum: Duration) -> Duration {
    deadline.map_or(maximum, |end| {
        Duration::from_millis(end.saturating_sub(SystemClock.monotonic_ms())).min(maximum)
    })
}

async fn with_row_until<T: Send + 'static>(
    state: &AppState,
    source: &Arc<Source>,
    deadline: Option<u64>,
    perform: impl FnOnce(
            NativeTextJoinedSelectionPreparation<OsCryptoRng>,
            NativeTextJoinedRowSelection,
            &server_table::NativeMemberWork,
            RequestCancellation,
        ) -> T
        + Send
        + 'static,
) -> Option<T> {
    // A blocking cut retains the original row, native permit, and fees until
    // the actual work leaves, even if lock drops the async observer earlier.
    let admit = async {
        let permit = source.member_work.foreground(source.negative()).await?;
        let Some((original, row)) = source.capture_joined(state).await else {
            #[cfg(test)]
            eprintln!("chat history worker could not capture its original row");
            return None;
        };
        Some((permit, original, row))
    };
    let (permit, original, row) = if deadline.is_some() {
        let wait = time_remaining(deadline, TURN);
        if wait.is_zero() {
            return None;
        }
        tokio::time::timeout(wait, admit).await.ok()??
    } else {
        admit.await?
    };
    let source = source.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if time_remaining(deadline, TURN).is_zero() {
            return None;
        }
        Some(perform(
            original,
            row,
            source.member_work(),
            source.negative(),
        ))
    })
    .await
    .ok()
    .flatten()
}
