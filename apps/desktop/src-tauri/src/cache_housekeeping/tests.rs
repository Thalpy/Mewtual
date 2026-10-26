//! Attach, rejoin, leave-cleanup and sweep behaviour of the desktop side, against a real vault
//! store. Studio-reference retention itself is covered in `catcoms-app` (it needs Studio saves).

use super::*;
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
    assert!(mark_leaving(&store, &GROUP).unwrap());
    let (rejoined, _) = prepare_blob_store(&store, Some(2), &GROUP).unwrap();
    assert!(
        open.promote_staged(&staged).unwrap(),
        "the open handle's staging survived"
    );
    assert!(storage_cache::has_leaving_marker(&namespace));
    drop((open, rejoined));
    // With nothing else open, the next attach finishes it.
    let (_again, _) = prepare_blob_store(&store, Some(2), &GROUP).unwrap();
    assert!(!storage_cache::has_leaving_marker(&namespace));
    assert!(namespace.join(staged.to_hex()).exists(), "the cache stays");
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
