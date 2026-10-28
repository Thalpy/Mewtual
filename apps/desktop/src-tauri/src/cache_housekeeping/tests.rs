//! Attach, rejoin, leave-cleanup and sweep behaviour of the desktop side, against a real vault
//! store. Studio-reference retention itself is covered in `catcoms-app` (it needs Studio saves).

use super::*;
use catcoms_app::TracedEvent;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn store_in(dir: &std::path::Path) -> ServerStore {
    ServerStore::open(
        dir,
        b"cache housekeeping",
        &mut ChaCha20Rng::seed_from_u64(7),
    )
    .unwrap()
}

const GROUP: [u8; 32] = [0x5a; 32];

/// Real sealed setting and actor; only event draining is controlled by these regressions.
struct LimitWriteCase {
    root: tempfile::TempDir,
    state: Arc<AppState>,
    actor: ServerActor,
    group: Vec<u8>,
    task: tokio::task::JoinHandle<()>,
    events: Option<tokio::sync::mpsc::Receiver<TracedEvent>>,
    drain: Option<tokio::task::JoinHandle<()>>,
}

impl LimitWriteCase {
    async fn new(hold_events: bool) -> Self {
        use catcoms_rt::{Hub, ManualClock};
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::default());
        *state.store.lock().await = Some(store_in(root.path()));
        *state.session_resumable.lock().await = true;
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(55_001)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(55_001),
            Box::new(ManualClock::new(1_000)),
            "cache write",
        )
        .unwrap();
        let group = server.group_id();
        let device = server.device_id();
        let (actor, events, task) = spawn(server);
        state.servers.lock().await.insert(
            1,
            crate::tests::snapshot_test_entry(55_001, actor.clone(), group.clone(), device),
        );
        let mut case = Self {
            root,
            state,
            actor,
            group,
            task,
            events: Some(events),
            drain: None,
        };
        if !hold_events {
            case.start_drain();
        }
        case.actor
            .set_cache_limit(CacheLimit::Unlimited)
            .await
            .unwrap();
        case.state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .save_cache_setting(
                1,
                &case.group,
                &CacheSetting {
                    limit: None,
                    origin: CacheSettingOrigin::User,
                },
                &mut OsCryptoRng,
            )
            .unwrap();
        case
    }

    fn start_drain(&mut self) {
        if let Some(mut events) = self.events.take() {
            self.drain = Some(tokio::spawn(async move {
                while events.recv().await.is_some() {}
            }));
        }
    }

    fn sealed(&self) -> Option<Vec<u8>> {
        std::fs::read(self.root.path().join("servers/1.storage")).ok()
    }

    async fn finish(mut self) {
        self.start_drain();
        self.state.servers.lock().await.clear();
        self.actor.shutdown().await;
        self.task.await.unwrap();
        self.drain.take().unwrap().await.unwrap();
    }
}

#[tokio::test]
async fn cache_limit_queued_before_lock_cannot_write_or_revive_after_unlock() {
    for unlock_again in [false, true] {
        let case = LimitWriteCase::new(false).await;
        let before = case.sealed();
        let serial = case.state.cache.limit_writes.lock().await;
        let mut write = Box::pin(set_cache_limit_in_session(&case.state, 1, Some(1 << 30)));
        assert!(
            still_pending(write.as_mut()).await,
            "actual setter is queued"
        );
        lock_session_inner(&case.state, None).await.unwrap();
        if unlock_again {
            let generation = case.state.ui_session_generation.load(Ordering::Acquire);
            finalize_unlock_session(&case.state, generation, false)
                .await
                .unwrap();
            assert!(!ui_session_closed(&case.state));
        }
        drop(serial);
        assert!(write.await.is_err());
        assert_eq!(
            case.sealed(),
            before,
            "old request must not rewrite sealed settings"
        );
        assert!(case.actor.cache_status().await.unwrap().unlimited);
        case.finish().await;
    }
}

#[tokio::test]
async fn cache_limit_rechecks_lock_after_waiting_for_the_sealed_store() {
    let case = LimitWriteCase::new(false).await;
    let before = case.sealed();
    let servers = case.state.servers.lock().await;
    let mut write = Box::pin(set_cache_limit_in_session(&case.state, 1, Some(1 << 30)));
    assert!(still_pending(write.as_mut()).await);
    let store = case.state.store.lock().await;
    drop(servers);
    assert!(still_pending(write.as_mut()).await);
    let mut next_store = Box::pin(case.state.store.lock());
    assert!(still_pending(next_store.as_mut()).await);
    drop(store);
    // FIFO Store admission lets the final UI authorization finish, then holds the
    // actual sealed write behind next_store. Lock registers while commit is owned.
    assert!(still_pending(write.as_mut()).await);
    let store = next_store.await;
    assert!(case.state.ui_session_commit.try_lock().is_err());
    let mut lock = Box::pin(lock_session_inner(&case.state, None));
    assert!(still_pending(lock.as_mut()).await);
    drop(store);
    assert!(write.await.is_err());
    lock.await.unwrap();
    assert_eq!(case.sealed(), before);
    assert!(case.actor.cache_status().await.unwrap().unlimited);
    case.finish().await;
}

#[tokio::test]
async fn queued_cache_limit_cannot_select_a_replacement_server() {
    let case = LimitWriteCase::new(false).await;
    let before = case.sealed();
    let serial = case.state.cache.limit_writes.lock().await;
    let mut write = Box::pin(set_cache_limit_in_session(&case.state, 1, Some(1 << 30)));
    assert!(still_pending(write.as_mut()).await);
    let (replacement, task, drain) = crate::leaving::tests::running(55_002);
    let replacement_actor = replacement.actor.clone();
    let replacement_group = replacement.group_id.clone();
    replacement_actor
        .set_cache_limit(CacheLimit::Unlimited)
        .await
        .unwrap();
    case.state.servers.lock().await.insert(1, replacement);
    drop(serial);
    assert!(write.await.is_err());
    assert_eq!(
        case.sealed(),
        before,
        "replacement's settings are not written by old UI work"
    );
    assert!(replacement_actor.cache_status().await.unwrap().unlimited);
    assert_ne!(replacement_group, case.group);
    case.state.servers.lock().await.clear();
    replacement_actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
    case.finish().await;
}

#[tokio::test]
async fn cache_limit_cannot_recreate_a_record_after_committed_leave() {
    let case = LimitWriteCase::new(false).await;
    let serial = case.state.cache.limit_writes.lock().await;
    let mut write = Box::pin(set_cache_limit_in_session(&case.state, 1, Some(1 << 30)));
    assert!(still_pending(write.as_mut()).await);
    let stopped = crate::leaving::stop_server(&case.state, 1, &SystemClock)
        .await
        .unwrap()
        .unwrap();
    // The same order and real retirement/store operations used by leave_server. Store is
    // deliberately held until the old command has resumed, exposing the former late write.
    let persist = persist_lock_for(&case.state, 1).lock_owned().await;
    let generation = case.state.ui_session_generation.load(Ordering::Acquire);
    let session = require_ui_session_generation(&case.state, generation)
        .await
        .unwrap();
    let store = case.state.store.lock().await;
    drop(serial);
    assert!(still_pending(write.as_mut()).await);
    let removed = stopped
        .remove(&mut *case.state.servers.lock().await)
        .unwrap();
    store.as_ref().unwrap().remove_server(1).unwrap();
    drop((removed, store, session, persist));
    assert!(write.await.is_err());
    assert!(
        case.sealed().is_none(),
        "a committed leave must stay removed"
    );
    case.finish().await;
}

#[tokio::test]
async fn admitted_cache_limit_outlives_its_waiter_and_precedes_actual_lock() {
    let mut case = LimitWriteCase::new(true).await;
    let mut sequence = 0;
    while case.events.as_ref().unwrap().capacity() > 0 {
        case.actor
            .set_server_name(format!("cache barrier {sequence}"))
            .await
            .unwrap();
        sequence += 1;
    }
    let actor = case.actor.clone();
    let mut barrier =
        Box::pin(async move { actor.set_server_name("last cache barrier".into()).await });
    assert!(still_pending(barrier.as_mut()).await);
    let before = case.sealed();
    let (entered, started) = tokio::sync::oneshot::channel();
    *case.state.cache.limit_apply_entered.lock().unwrap() = Some(entered);
    let mut write = Box::pin(set_cache_limit_in_session(&case.state, 1, Some(1 << 30)));
    assert!(still_pending(write.as_mut()).await);
    assert_ne!(
        case.sealed(),
        before,
        "final admission has written the actual setting"
    );
    drop(write); // Only the waiting command disappears; actual apply owns all commit fences.
                 // Observe the worker itself, not merely spawn: otherwise an unpolled worker
                 // could retain the guards accidentally and make an early-drop mutant pass.
    started.await.unwrap();
    let state = case.state.clone();
    let mut lock = Box::pin(lock_session_inner(&state, None));
    assert!(
        still_pending(lock.as_mut()).await,
        "lock waits for admitted actor application"
    );
    assert!(case.state.cache.limit_writes.try_lock().is_err());
    assert!(persist_lock_for(&case.state, 1).try_lock_owned().is_err());
    case.start_drain();
    lock.await.unwrap();
    barrier.await.unwrap();
    let status = case.actor.cache_status().await.unwrap();
    assert_eq!(status.limit_bytes, Some(1 << 30));
    assert!(!status.unlimited);
    let stored = case
        .state
        .store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_cache_setting(1, &case.group);
    assert_eq!(
        stored,
        CacheSettingLoad::Found(CacheSetting {
            limit: Some(1 << 30),
            origin: CacheSettingOrigin::User,
        })
    );
    case.finish().await;
}

#[test]
fn attach_migrates_a_missing_setting_once_and_honours_saved_and_damaged_ones() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    let (_, limit) = prepare_blob_store(&store, Some(3), &GROUP).unwrap();
    assert_eq!(limit, CacheLimit::Bytes(cache_plan::DEFAULT_CACHE_LIMIT));
    assert_eq!(
        store.load_cache_setting(3, &GROUP),
        CacheSettingLoad::Found(CacheSetting {
            limit: Some(cache_plan::DEFAULT_CACHE_LIMIT),
            origin: CacheSettingOrigin::Migrated,
        }),
        "the migrated limit is saved, so it never shrinks on a later launch"
    );
    // A user choice survives the next attach unchanged.
    store
        .save_cache_setting(
            3,
            &GROUP,
            &CacheSetting {
                limit: None,
                origin: CacheSettingOrigin::User,
            },
            &mut OsCryptoRng,
        )
        .unwrap();
    assert_eq!(
        prepare_blob_store(&store, Some(3), &GROUP).unwrap().1,
        CacheLimit::Unlimited
    );
    // A damaged record keeps eviction off instead of falling back to a default.
    std::fs::write(
        store.backup_source_dir().join("servers").join("3.storage"),
        b"damaged",
    )
    .unwrap();
    assert_eq!(
        prepare_blob_store(&store, Some(3), &GROUP).unwrap().1,
        CacheLimit::Unreadable
    );
    // A server being founded or joined has no number yet: its limit waits for registration.
    assert_eq!(
        prepare_blob_store(&store, None, &GROUP).unwrap().1,
        CacheLimit::Pending
    );
}

#[test]
fn a_rejoin_over_a_marker_keeps_the_cache_but_finishes_removing_keep() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    let key = hex::encode(GROUP);
    let cached = store
        .blob_store(&key)
        .unwrap()
        .put(b"group content")
        .unwrap();
    let namespace = store.cache_namespace_dir(&key).unwrap();
    let stale = namespace.join("kept").join("transfers-v2").join("d-1");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("current"), [0; 16]).unwrap();
    assert!(mark_leaving(&store, &GROUP).unwrap());

    let (blobs, _) = prepare_blob_store(&store, Some(4), &GROUP).unwrap();
    assert!(
        blobs.has(&cached),
        "the group's own cache is kept for the rejoin"
    );
    assert!(
        !stale.exists(),
        "the Keep copies the user confirmed deleting are gone"
    );
    assert!(!storage_cache::has_leaving_marker(&namespace));
    assert!(blobs
        .kept_files()
        .error
        .is_none_or(|e| !e.contains("needs attention")));
}

#[tokio::test]
async fn leave_cleanup_empties_the_namespace_under_a_claim_and_reports_freed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    let key = hex::encode(GROUP);
    store.blob_store(&key).unwrap().put(&[1; 4000]).unwrap();
    let namespace = store.cache_namespace_dir(&key).unwrap();
    assert!(mark_leaving(&store, &GROUP).unwrap());
    let state = AppState::default();
    *state.store.lock().await = Some(store);

    let claim = state.cache.claim(&GROUP).unwrap();
    assert!(state.cache.claim(&GROUP).is_none(), "one cleanup per group");
    let report = clean_namespace(&state, &claim, None, false).await;
    assert_eq!(report.failed, 0);
    assert!(report.cleaned);
    assert!(report.freed_bytes >= 4000);
    assert!(!namespace.exists(), "nothing pinned, so the namespace went");
    drop(claim);
    assert!(!state.cache.claimed(&GROUP));
}

/// Review H1, as production reaches it: the group's server is running when the user joins the
/// same group again, so the join attaches a second handle (which cannot get the Keep lease), and
/// then the running server is left. Once its handle drops, neither that leave's cleanup nor the
/// sweep may empty the namespace under the live joiner, and the marker stays for a later retry.
/// When the joiner is gone too, the sweep finishes the leave.
#[tokio::test]
async fn cleanup_never_empties_a_namespace_a_live_join_still_holds() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    let key = hex::encode(GROUP);
    let (running, _) = prepare_blob_store(&store, Some(1), &GROUP).unwrap();
    let (mut joiner, _) = prepare_blob_store(&store, None, &GROUP).unwrap();
    let cached = joiner.put(&[2; 4000]).unwrap();
    let namespace = store.cache_namespace_dir(&key).unwrap();
    // `leave_server`: claim, marker, then the stopped server's handle drops.
    let state = AppState::default();
    let claim = state.cache.claim(&GROUP).unwrap();
    assert!(mark_leaving(&store, &GROUP).unwrap());
    drop(running);
    *state.store.lock().await = Some(store);
    let report = clean_namespace(&state, &claim, None, false).await;
    assert_eq!(report.freed_bytes, 0);
    assert!(report.failed > 0, "reported as not finished");
    assert!(joiner.has(&cached), "the live joiner's cache survives");
    assert!(
        storage_cache::has_leaving_marker(&namespace),
        "the marker stays: the holder could also be the leaver's own handle"
    );
    drop(claim);
    // Not registered yet, so the sweep does try it, and meets the same open handle.
    sweep_leaving(&state, None).await;
    assert!(joiner.has(&cached), "the sweep leaves it alone too");
    assert!(storage_cache::has_leaving_marker(&namespace));
    drop(joiner);
    sweep_leaving(&state, None).await;
    assert!(
        !namespace.exists(),
        "with every handle gone the leave finishes"
    );
}

/// A rejoin over a marker while another handle still has the namespace open must not remove that
/// handle's live scratch or the marker: Keep stays off for the new attach and the next attach
/// retries.
#[test]
fn a_rejoin_over_a_marker_leaves_a_namespace_another_handle_holds() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    let key = hex::encode(GROUP);
    let mut open = store.blob_store(&key).unwrap();
    let staged = open.put_staged(b"an upload being staged").unwrap();
    let namespace = store.cache_namespace_dir(&key).unwrap();
    let confirmed = namespace.join("kept").join("transfers-v2").join("d-1");
    std::fs::create_dir_all(&confirmed).unwrap();
    assert!(mark_leaving(&store, &GROUP).unwrap());
    let (rejoined, _) = prepare_blob_store(&store, Some(2), &GROUP).unwrap();
    assert!(
        open.promote_staged(&staged).unwrap(),
        "the open handle's staging survived"
    );
    assert!(
        confirmed.exists(),
        "kept/ is not touched under an open handle"
    );
    assert!(storage_cache::has_leaving_marker(&namespace));
    drop((open, rejoined));
    // With nothing else open, the next attach finishes it.
    let (_again, _) = prepare_blob_store(&store, Some(2), &GROUP).unwrap();
    assert!(!storage_cache::has_leaving_marker(&namespace));
    assert!(namespace.join(staged.to_hex()).exists(), "the cache stays");
}

/// Poll `future` once: whether it is still waiting.
async fn still_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) -> bool {
    std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx).is_pending())).await
}

/// `leave_server` itself cannot run here (it needs a Tauri `State`), so pin the order in its
/// source: the claim comes before the stop (see `claim_for_leave` for why).
#[test]
fn leave_server_claims_the_group_before_it_stops_the_server() {
    let source = include_str!("../lib.rs");
    let body = &source[source.find("async fn leave_server(").expect("leave_server")..];
    let body = &body[..body.find("\n}\n").expect("end of leave_server")];
    let claim = body.find("claim_for_leave(").expect("leave_server claims");
    let stop = body.find("stop_server(").expect("leave_server stops");
    assert!(
        claim < stop,
        "the claim must be taken while the server runs"
    );
}

/// Re-review of 251ca05c: `leave_server` claims the group while the server still runs, before the
/// stop drops its Keep lease, so no join can attach (with a healthy Keep whose copies a kept
/// marker would later wipe) between the stop and the marker. A concurrent leave of the same server
/// waits and then finds it gone. Two alias rows left together run one after the other, so the last
/// one claims (re-review of 62d7cbdc: before, each saw the other and neither marked or cleaned).
#[tokio::test]
async fn a_leave_claims_the_group_before_the_server_stops() {
    let state = AppState::default();
    let (entry, task, drain) = crate::leaving::tests::running(951);
    let group = entry.group_id.clone();
    let actor = entry.actor.clone();
    state.servers.lock().await.insert(1, entry);
    let claim = claim_for_leave(&state, 1)
        .await
        .expect("a registered server with no alias is claimed");
    assert!(state.cache.claimed(&group));
    let join = state.cache.wait_unclaimed(&group);
    tokio::pin!(join);
    assert!(
        still_pending(join.as_mut()).await,
        "a join of the same group cannot attach until the leave is done"
    );
    let second = claim_for_leave(&state, 1);
    tokio::pin!(second);
    assert!(still_pending(second.as_mut()).await);
    state.servers.lock().await.remove(&1); // the first leave removed the row
    drop(claim);
    assert!(
        second.await.is_none(),
        "the second leave finds nothing to claim"
    );
    join.await;
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();

    let (two, two_task, two_drain) = crate::leaving::tests::running(952);
    let (mut three, three_task, three_drain) = crate::leaving::tests::running(953);
    three.group_id = two.group_id.clone();
    let shared = two.group_id.clone();
    let actors = [two.actor.clone(), three.actor.clone()];
    state.servers.lock().await.insert(2, two);
    state.servers.lock().await.insert(3, three);
    let first = claim_for_leave(&state, 2)
        .await
        .expect("an alias row is claimed too");
    let last = claim_for_leave(&state, 3);
    tokio::pin!(last);
    assert!(
        still_pending(last.as_mut()).await,
        "the other alias's leave waits"
    );
    // `leave_server` for row 2: removed, an alias remains, so its claim goes unused.
    state.servers.lock().await.remove(&2);
    drop(first);
    let last = last
        .await
        .expect("the last alias left claims while it still runs");
    assert!(state.cache.claimed(&shared));
    drop(last);
    for actor in actors {
        actor.shutdown().await;
    }
    for handle in [two_task, two_drain, three_task, three_drain] {
        handle.await.unwrap();
    }
}

#[test]
fn adoption_never_migrates_without_a_size_limited_cache() {
    use catcoms_app::cache_housekeeping::CachePause;
    let status = |paused| CacheStatus {
        paused,
        ..CacheStatus::default()
    };
    assert_eq!(adoption_scan(&status(Some(CachePause::NoCache))), None);
    assert_eq!(
        adoption_scan(&status(Some(CachePause::ScanIncomplete))),
        Some(false)
    );
    assert_eq!(
        adoption_scan(&status(Some(CachePause::NotReady))),
        Some(true)
    );
}

#[tokio::test]
async fn an_attach_waits_while_a_cleanup_holds_the_group() {
    let state = AppState::default();
    let claim = state.cache.claim(&GROUP).unwrap();
    let waiting = state.cache.wait_unclaimed(&GROUP);
    tokio::pin!(waiting);
    let pending = std::future::poll_fn(|cx| {
        std::task::Poll::Ready(std::future::Future::poll(waiting.as_mut(), cx).is_pending())
    })
    .await;
    assert!(pending, "no attach while the namespace is being emptied");
    drop(claim);
    waiting.await;
}

#[tokio::test]
async fn the_sweep_finishes_marked_namespaces_but_skips_claimed_ones() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    let left = [0x11u8; 32];
    let busy = [0x22u8; 32];
    for group in [left, busy] {
        store
            .blob_store(&hex::encode(group))
            .unwrap()
            .put(&group)
            .unwrap();
        assert!(mark_leaving(&store, &group).unwrap());
    }
    let busy_dir = store.cache_namespace_dir(&hex::encode(busy)).unwrap();
    let left_dir = store.cache_namespace_dir(&hex::encode(left)).unwrap();
    let state = AppState::default();
    *state.store.lock().await = Some(store);
    let held = state.cache.claim(&busy).unwrap();
    sweep_leaving(&state, None).await;
    assert!(!left_dir.exists(), "an interrupted leave is finished");
    assert!(
        storage_cache::has_leaving_marker(&busy_dir),
        "a claimed namespace is left to its owner"
    );
    drop(held);
}
