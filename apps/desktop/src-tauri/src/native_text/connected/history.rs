//! Reconcile a joined row from its explicitly approved connected contacts.
//! Each peer gets at most one bounded page or original exchange per turn.
use super::*;
use catcoms_app::store::{
    NativeTextJoinedArchiveReplay, NativeTextJoinedDependencyExchange, NativeTextJoinedHistoryPage,
};
use catcoms_replication::chat::ActionId;
use std::collections::VecDeque;
use std::time::Duration;

const TURN: Duration = Duration::from_millis(500);
const REQUEST: Duration = Duration::from_secs(3);
const PAGES_PER_SLICE: u8 = 4;
const RETRY_MS: u64 = 5_000;
const RESCAN_MS: u64 = 30_000;
const REPLAY_MS: u64 = 15_000;

struct ParkedOriginal {
    action: ActionId,
    replay: NativeTextJoinedArchiveReplay,
    deadline: u64,
}

struct PeerSweep {
    peer: PeerId,
    page: Option<Arc<NativeTextJoinedHistoryPage>>,
    next_entry: usize,
    deferred: bool,
    pages: u8,
    due: u64,
}

impl PeerSweep {
    fn new(peer: PeerId) -> Self {
        Self {
            peer,
            page: None,
            next_entry: 0,
            deferred: false,
            pages: 0,
            due: 0,
        }
    }

    fn rescan_after(&mut self, delay: u64) {
        self.page = None;
        self.next_entry = 0;
        self.deferred = false;
        self.pages = 0;
        self.due = SystemClock.monotonic_ms().saturating_add(delay);
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
    loop {
        tokio::select! {
            biased;
            _ = scope.cancelled() => return,
            _ = SystemClock.sleep(TURN) => {}
        }
        let state = app.state::<AppState>();
        if let Some(target) = reconcile_archive_change(&state, source, &mut pending_cursor).await {
            source.notify_received(app, &state, Some(target)).await;
        }
        if let Some((mut sweep, parked)) = replay.take() {
            let mut parked = Some(parked);
            if let Some((covered, notice)) = resume_entry(&state, source, &mut parked).await {
                sweep.next_entry += 1;
                sweep.deferred |= !covered;
                peers.push_back(sweep);
                if let Some(target) = notice {
                    source.notify_received(app, &state, Some(target)).await;
                }
            } else if let Some(parked) = parked {
                replay = Some((sweep, parked));
            }
            // One paid continuation per row. It holds no native permit
            // between turns and yields its peer as soon as replay settles.
            continue;
        }
        let Some(candidates) = row.history_peer_candidates() else {
            continue;
        };
        peers.retain(|sweep| candidates.contains(&sweep.peer));
        for peer in candidates {
            if !peers.iter().any(|sweep| sweep.peer == peer) {
                peers.push_back(PeerSweep::new(peer));
            }
        }
        let now = SystemClock.monotonic_ms();
        let Some(index) = peers.iter().position(|sweep| sweep.due <= now) else {
            continue;
        };
        let mut sweep = peers.remove(index).expect("selected due history peer");
        let mut parked = None;
        let _ = turn(&state, source, &mut sweep, &mut parked).await;
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
    with_row(state, source, move |original, row, local, scope| {
        let selected = original
            .selected_archive_target_data(&row, channel, change.action, &scope)
            .ok()?;
        local.reconcile_archive_change(change, selected)
    })
    .await
    .flatten()
}

async fn turn(
    state: &AppState,
    source: &Arc<Source>,
    sweep: &mut PeerSweep,
    parked: &mut Option<ParkedOriginal>,
) -> Option<()> {
    let page = sweep.page.clone();
    if let Some(page) = &page {
        let Some(data) = page.data() else {
            sweep.rescan_after(RETRY_MS);
            return None;
        };
        if let Some(entry) = data.entries().get(sweep.next_entry) {
            let covered = fetch_entry(state, source, page.clone(), entry.action, parked).await?;
            sweep.next_entry += 1;
            // Preserve the obligation to revisit the whole frozen sweep.
            // Later IDs and other authors can progress while a parent is absent.
            sweep.deferred |= !covered;
            return None;
        }
    }
    if sweep.pages == PAGES_PER_SLICE {
        // Bound continuous scans even for a member advertising an inflated
        // end. Preserve the actual cursor for the next slice.
        sweep.pages = 0;
        sweep.due = SystemClock.monotonic_ms().saturating_add(RETRY_MS);
        return None;
    }
    let peer = sweep.peer;
    let channel = source.channel();
    let request = with_row(state, source, move |original, row, _, scope| match page {
        Some(page) => original.prepare_history_continuation(&row, &page, scope, &mut OsCryptoRng),
        None => original
            .prepare_history_exchange(&row, channel, peer, scope, &mut OsCryptoRng)
            .map(Some),
    })
    .await?;
    let exchange = match request {
        Ok(Some(exchange)) => exchange,
        Ok(None) => {
            sweep.rescan_after(if sweep.deferred { RETRY_MS } else { RESCAN_MS });
            return None;
        }
        Err(V2MemberSaveRefusal::Busy) => return None,
        Err(_) => {
            sweep.rescan_after(RETRY_MS);
            return None;
        }
    };
    let Ok(Ok(reply)) = tokio::time::timeout(REQUEST, exchange.receive()).await else {
        // Retain the exact earlier cursor on refused or lost replies. Slow
        // peers still yield their turn after each bounded network wait.
        sweep.due = SystemClock.monotonic_ms().saturating_add(RETRY_MS);
        return None;
    };
    let page = with_row(state, source, move |original, row, _, scope| {
        original.open_history_response(&row, reply, &scope)
    })
    .await?;
    match page {
        Ok(page) => {
            sweep.page = Some(Arc::new(page));
            sweep.next_entry = 0;
            sweep.pages += 1;
        }
        Err(V2MemberSaveRefusal::Busy) => {}
        Err(_) => sweep.rescan_after(RETRY_MS),
    }
    Some(())
}

enum Fetch {
    Covered,
    Request(NativeTextJoinedDependencyExchange),
}

async fn fetch_entry(
    state: &AppState,
    source: &Arc<Source>,
    page: Arc<NativeTextJoinedHistoryPage>,
    action: ActionId,
    parked: &mut Option<ParkedOriginal>,
) -> Option<bool> {
    let fetch = with_row(state, source, move |original, row, _, scope| {
        if original
            .selected_discovered_target(&row, &page, action, &scope)?
            .is_some()
        {
            return Ok(Fetch::Covered);
        }
        original
            .prepare_discovered_original(&row, &page, action, scope, &mut OsCryptoRng)
            .map(Fetch::Request)
    })
    .await?;
    let exchange = match fetch {
        Ok(Fetch::Covered) => return Some(true),
        Ok(Fetch::Request(exchange)) => exchange,
        Err(V2MemberSaveRefusal::Busy) => return None,
        Err(_) => return Some(false),
    };
    let Ok(Ok(reply)) = tokio::time::timeout(REQUEST, exchange.receive()).await else {
        return Some(false);
    };
    let stage = with_row(state, source, move |original, row, _, scope| {
        original.prepare_discovered_archive(&row, reply, scope)
    })
    .await?;
    let replay = match stage {
        Ok(Some(stage)) => stage,
        Err(V2MemberSaveRefusal::Busy) => return None,
        Ok(None) | Err(_) => return Some(false),
    };
    *parked = Some(ParkedOriginal {
        action,
        replay,
        deadline: SystemClock.monotonic_ms().saturating_add(REPLAY_MS),
    });
    None
}

async fn resume_entry(
    state: &AppState,
    source: &Arc<Source>,
    parked: &mut Option<ParkedOriginal>,
) -> Option<(bool, Option<ActionId>)> {
    if SystemClock.monotonic_ms() >= parked.as_ref()?.deadline {
        *parked = None;
        return Some((false, None));
    }
    // Acquire both before moving the paid continuation. Contention must
    // preserve it while foreground history or signing uses this row.
    let permit = source.member_work.try_background()?;
    let (original, _) = source.capture_joined(state).await?;
    let mut work = parked.take()?;
    let source = source.clone();
    let processing = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if SystemClock.monotonic_ms() >= work.deadline {
            return (work, Some((false, None)));
        }
        let scope = source.negative();
        let local = source.member_work();
        let target = work.replay.target_data();
        let action = work.action;
        let ready = original.advance_discovered_archive(&mut work.replay, &scope, &mut OsCryptoRng);
        match ready {
            Ok(false) | Err(V2MemberSaveRefusal::Busy) => return (work, None),
            Err(_) => return (work, Some((false, None))),
            Ok(true) => {}
        }
        if SystemClock.monotonic_ms() >= work.deadline {
            return (work, Some((false, None)));
        }
        // Reserve before any write. Even a Stale result can mean the disk
        // selected the original and the remote route then retired.
        if local.reserve_archive_change(action, target).is_none() {
            return (work, None);
        }
        let result =
            match original.select_discovered_archive(&work.replay, &scope, &mut OsCryptoRng) {
                Ok(Some(_)) => {
                    local.note_archive_change(action, target, true);
                    (true, Some(target))
                }
                _ => (false, Some(target)),
            };
        (work, Some(result))
    })
    .await;
    let Ok((work, result)) = processing else {
        return Some((false, None));
    };
    if result.is_none() {
        *parked = Some(work);
    }
    result
}

async fn with_row<T: Send + 'static>(
    state: &AppState,
    source: &Arc<Source>,
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
    let permit = source.member_work.try_background()?;
    let (original, row) = source.capture_joined(state).await?;
    let source = source.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        perform(original, row, source.member_work(), source.negative())
    })
    .await
    .ok()
}
