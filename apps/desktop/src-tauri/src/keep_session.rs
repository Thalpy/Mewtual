//! Unlock ownership for requested file reads, background Keep and source service. Independent of voice policy and
//! frontend views. The lock generation is checked synchronously at actual lower admission;
//! the watch also wakes suspended jobs after retirement. New unlocks mint new tokens.
use super::*;
use catcoms_rt::RequestAuthority;

#[derive(Debug)]
struct Authority {
    generation: Arc<AtomicU64>,
    expected: u64,
    signal: watch::Receiver<bool>,
}
#[async_trait::async_trait]
impl RequestAuthority for Authority {
    fn authorized(&self) -> bool {
        self.generation.load(Ordering::Acquire) == self.expected
            && !*self.signal.borrow()
            && self.signal.has_changed().is_ok()
    }
    async fn revoked(&self) {
        let mut signal = self.signal.clone();
        while self.authorized() {
            if signal.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Exact native registry-row lifetime, independent of account unlock and actor/RPC handles.
/// The row alone owns this publisher; observers cannot keep it live after removal/replacement.
/// Explicit retirement under the registry guard precedes returning a removed row across awaits.
#[derive(Debug)]
pub(super) struct EntryOwner(watch::Sender<bool>);
impl EntryOwner {
    pub(super) fn new() -> Self {
        Self(watch::channel(false).0)
    }
    pub(super) fn retire(&self) {
        self.0.send_replace(true);
    }

    /// Preserve the FULL account cancellation authority. with_authority replaces its old field;
    /// assigning an entry-only gate there would silently lose the synchronous lock generation.
    /// Each unlock can compose a new account observer, but a retired row is never reactivated.
    pub(super) fn scope(&self, account: RequestCancellation) -> RequestCancellation {
        let entry = self.0.subscribe();
        RequestCancellation::new(entry.clone(), None)
            .with_authority(Arc::new(EntryAuthority { entry, account }))
    }
}
impl Drop for EntryOwner {
    fn drop(&mut self) {
        self.retire();
    }
}

#[derive(Debug)]
struct EntryAuthority {
    entry: watch::Receiver<bool>,
    account: RequestCancellation,
}
#[async_trait::async_trait]
impl RequestAuthority for EntryAuthority {
    fn authorized(&self) -> bool {
        !*self.entry.borrow() && self.entry.has_changed().is_ok() && !self.account.is_cancelled()
    }
    async fn revoked(&self) {
        let mut entry = self.entry.clone();
        let mut account = self.account.clone();
        while self.authorized() {
            tokio::select! {
                _ = entry.changed() => {},
                _ = account.cancelled() => return,
            }
        }
    }
}

/// Called only at authenticated commit/registration. The mutex orders token creation against
/// synchronous lock retirement; the generation prevents a thread paused before retirement
/// from admitting an old request in the meantime. Duplicate same-session unlocks reuse custody.
pub(super) fn current(state: &AppState, expected: u64) -> Option<RequestCancellation> {
    let mut slot = state.keep_session.lock().unwrap_or_else(|e| e.into_inner());
    if ui_session_closed(state) || state.ui_session_generation.load(Ordering::Acquire) != expected {
        return None;
    }
    if slot
        .as_ref()
        .is_none_or(|(generation, _)| *generation != expected)
    {
        *slot = Some((expected, watch::channel(false).0));
    }
    let signal = slot.as_ref()?.1.subscribe();
    Some(
        RequestCancellation::new(signal.clone(), None).with_authority(Arc::new(Authority {
            generation: state.ui_session_generation.clone(),
            expected,
            signal,
        })),
    )
}

/// Capture before the first authorization await. Waiting behind lock/unlock cannot adopt the
/// new generation as authority for a native invocation that already started in the old one.
pub(super) async fn invocation_generation(state: &AppState) -> Result<u64, String> {
    let expected = state.ui_session_generation.load(Ordering::Acquire);
    authorize_invocation(state, expected).await
}

/// Synchronous native entry points capture `expected` BEFORE spawning an async task. Even an
/// entirely unpolled old task must fail after lock/unlock, rather than sample the fresh session.
pub(super) async fn authorize_invocation(state: &AppState, expected: u64) -> Result<u64, String> {
    let _commit = require_ui_session_generation(state, expected).await?;
    Ok(expected)
}

/// Capture under the exact registry-row guard before queueing actor I/O. The returned observer
/// retains neither row nor publisher; replacement/lock remains irreversible while commands wait.
pub(super) async fn for_actor(
    state: &AppState,
    server: u64,
    actor: &ServerActor,
    expected: u64,
) -> Option<RequestCancellation> {
    let account = current(state, expected)?;
    let servers = state.servers.lock().await;
    let entry = servers.get(&server)?;
    if !entry.actor.same_instance(actor) || account.is_cancelled() {
        return None;
    }
    Some(entry.files.scope(account))
}

#[derive(Debug)]
struct ReadAuthority {
    native: RequestCancellation,
    caller: RequestCancellation,
}
#[async_trait::async_trait]
impl RequestAuthority for ReadAuthority {
    fn authorized(&self) -> bool {
        !self.native.is_cancelled() && !self.caller.is_cancelled()
    }
    async fn revoked(&self) {
        let mut native = self.native.clone();
        let mut caller = self.caller.clone();
        if !self.authorized() {
            return;
        }
        tokio::select! { _ = native.cancelled() => {}, _ = caller.cancelled() => {} }
    }
}
/// Add original native scope without losing the caller's cancellation OR lower keepalive.
/// with_authority replaces its field, so the complete original caller is retained inside.
pub(super) fn bind_read(
    native: RequestCancellation,
    caller: RequestCancellation,
) -> RequestCancellation {
    caller
        .clone()
        .with_authority(Arc::new(ReadAuthority { native, caller }))
}

/// Serialize one synchronous plaintext publication with lock, retaining the invocation's
/// exact registry-row observer. A fresh account unlock or replacement row cannot authorize it.
pub(super) async fn read_commit<'a>(
    state: &'a AppState,
    generation: u64,
    origin: &RequestCancellation,
) -> Result<tokio::sync::MutexGuard<'a, ()>, String> {
    let commit = require_ui_session_generation(state, generation).await?;
    if origin.is_cancelled() {
        return Err("The file read is no longer active.".into());
    }
    Ok(commit)
}

/// Progress contains file metadata too. The synchronous emitter must observe the original read
/// immediately before publication, including retirement during a preceding disk/CPU step.
pub(super) fn publish_progress(origin: &RequestCancellation, publish: impl FnOnce()) {
    if !origin.is_cancelled() {
        publish();
    }
}

/// Must run before the first lock/close await. Dropping the sole sender permanently revokes
/// every captured receiver; no later unlocked session can change that channel back to live.
pub(super) fn revoke(state: &AppState) {
    state
        .keep_session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keep_session_lock_generation_revokes_before_cleanup_and_unlock_never_revives_old_work()
    {
        let state = AppState::default();
        let old = current(&state, 0).unwrap();
        assert!(!old.is_cancelled());
        let duplicate = current(&state, 0).unwrap();
        let generation = register_ui_lock_request(&state);
        assert!(old.is_cancelled());
        assert!(duplicate.is_cancelled());
        assert!(current(&state, generation).is_none());
        let mut waiting = old.clone();
        waiting.cancelled().await;
        state
            .ui_authorized_generation
            .store(generation, Ordering::Release);
        state.session_lock_requested.store(false, Ordering::Release);
        let fresh = current(&state, generation).unwrap();
        assert!(!fresh.is_cancelled());
        assert!(
            old.is_cancelled(),
            "fresh session cannot revive an old lower admission token"
        );
        revoke(&state);
        assert!(
            fresh.is_cancelled(),
            "owner disappearance is permanently closed"
        );
    }

    #[test]
    fn keep_session_generation_alone_refuses_even_before_watch_retirement() {
        let state = AppState::default();
        let held = current(&state, 0).unwrap();
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        assert!(
            held.is_cancelled(),
            "actual driver gate does not depend on a propagation task"
        );
        assert!(current(&state, 0).is_none());
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;

    #[tokio::test]
    async fn exact_entry_and_account_generation_each_retire_composed_work_permanently() {
        let state = AppState::default();
        let entry = EntryOwner::new();
        let account = current(&state, 0).unwrap();
        let old = entry.scope(account.clone());
        assert!(!old.is_cancelled());
        entry.retire();
        assert!(old.is_cancelled());
        let mut waiting = old.clone();
        waiting.cancelled().await;
        assert!(
            entry.scope(account.clone()).is_cancelled(),
            "fresh account capture cannot revive retired row"
        );
        let replacement = EntryOwner::new();
        let fresh = replacement.scope(account.clone());
        assert!(!fresh.is_cancelled());
        assert!(old.is_cancelled());
        // Generation check remains synchronous before the account publisher is retired.
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        assert!(fresh.is_cancelled());
        assert!(
            account.keepalive().is_none(),
            "these are observing scopes, not lower accounting"
        );
        drop(replacement);
        assert!(fresh.is_cancelled());
    }

    #[tokio::test]
    async fn entry_drop_wakes_scope_even_while_actor_or_account_observers_remain() {
        let state = AppState::default();
        let account = current(&state, 0).unwrap();
        let entry = EntryOwner::new();
        let mut scope = entry.scope(account.clone());
        drop(entry);
        assert!(
            !account.is_cancelled(),
            "removing one server does not lock the account"
        );
        scope.cancelled().await;
        assert!(scope.is_cancelled());
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;
    use std::future::{poll_fn, Future};

    #[tokio::test]
    async fn invocation_waiting_for_commit_cannot_adopt_an_intervening_unlock() {
        let state = AppState::default();
        let directory = tempfile::tempdir().unwrap();
        *state.store.lock().await =
            Some(ServerStore::open(directory.path(), b"read test", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let held = state.ui_session_commit.lock().await;
        let mut old = Box::pin(invocation_generation(&state));
        poll_fn(|cx| {
            assert!(old.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let fresh_generation = register_ui_lock_request(&state);
        // Complete a new authenticated unlock while the original invoke still waits. The real
        // unlocked store is essential: otherwise either implementation would fail as locked.
        state
            .ui_authorized_generation
            .store(fresh_generation, Ordering::Release);
        state.session_lock_requested.store(false, Ordering::Release);
        drop(held);
        assert!(old.await.is_err());
        assert_eq!(
            invocation_generation(&state).await.unwrap(),
            fresh_generation
        );
    }

    #[tokio::test]
    async fn entirely_unpolled_media_request_cannot_adopt_a_new_unlock() {
        let state = AppState::default();
        let directory = tempfile::tempdir().unwrap();
        *state.store.lock().await =
            Some(ServerStore::open(directory.path(), b"read test", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let captured = state.ui_session_generation.load(Ordering::Acquire);
        let path = format!("/1/{}", "a".repeat(64));
        let request = serve_media(&state, &path, None, captured);
        // Do NOT poll the future until an intervening authenticated unlock completed.
        let fresh = register_ui_lock_request(&state);
        state
            .ui_authorized_generation
            .store(fresh, Ordering::Release);
        state.session_lock_requested.store(false, Ordering::Release);
        let (scope, response) = request.await;
        assert!(scope.is_none());
        assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
        assert!(response.body().is_empty());
        assert_eq!(authorize_invocation(&state, fresh).await.unwrap(), fresh);
    }

    #[tokio::test]
    async fn composed_read_preserves_every_original_cancellation_and_exact_lower_custody() {
        for edge in 0..4 {
            let state = AppState::default();
            let entry = EntryOwner::new();
            let caller_owner = EntryOwner::new();
            let (signal, receiver) = watch::channel(false);
            let custody: catcoms_rt::SharedRequestKeepalive = Arc::new("original request");
            let caller = RequestCancellation::new(receiver, Some(custody.clone()));
            // Include an independent authority in addition to the caller's signal/keepalive.
            let caller = caller.with_authority(Arc::new(EntryAuthority {
                entry: caller_owner.0.subscribe(),
                account: current(&state, 0).unwrap(),
            }));
            let native = entry.scope(current(&state, 0).unwrap());
            let mut read = bind_read(native, caller);
            assert!(!read.is_cancelled());
            let publications = std::cell::Cell::new(0);
            publish_progress(&read, || publications.set(publications.get() + 1));
            assert_eq!(publications.get(), 1);
            assert!(Arc::ptr_eq(&custody, &read.keepalive().unwrap()));
            match edge {
                0 => {
                    signal.send_replace(true);
                }
                1 => caller_owner.retire(),
                2 => {
                    state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
                }
                _ => entry.retire(),
            }
            assert!(read.is_cancelled(), "edge {edge}");
            publish_progress(&read, || publications.set(publications.get() + 1));
            assert_eq!(
                publications.get(),
                1,
                "retired metadata must not be emitted"
            );
            read.cancelled().await;
            assert!(Arc::ptr_eq(&custody, &read.keepalive().unwrap()));
        }
    }

    #[tokio::test]
    async fn retired_original_entry_cannot_publish_media_or_cache_in_the_same_account() {
        let state = AppState::default();
        let directory = tempfile::tempdir().unwrap();
        *state.store.lock().await =
            Some(ServerStore::open(directory.path(), b"read test", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let entry = EntryOwner::new();
        let origin = entry.scope(current(&state, 0).unwrap());
        entry.retire();
        assert!(
            !current(&state, 0).unwrap().is_cancelled(),
            "account remains unlocked"
        );
        let response = http::Response::builder()
            .status(200)
            .body(b"plaintext".to_vec())
            .unwrap();
        publish_media_response(&state, Some(0), Some(&origin), response, |response| {
            assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
            assert!(response.body().is_empty());
        })
        .await;
        assert!(
            !media_cache_put_for_read(
                &state,
                0,
                Some(&origin),
                MediaChunk {
                    server: 1,
                    cid: "cid".into(),
                    manifest_version: [1; 32],
                    index: 0,
                    bytes: Arc::new(b"plaintext".to_vec()),
                }
            )
            .await
        );
        assert!(state.media_cache.lock().await.is_empty());
        assert!(media_head_put_for_read(
            &state,
            0,
            Some(&origin),
            MediaHead {
                server: 1,
                cid: "cid".into(),
                manifest_version: [1; 32],
                total_size: 9,
                mime: "audio/wav".into(),
            }
        )
        .await
        .is_none());
        assert!(state.media_heads.lock().await.is_empty());
    }

    #[tokio::test]
    async fn saved_read_tail_refuses_retirement_before_inspection_reveal_and_success() {
        for edge in 0..4 {
            let state = AppState::default();
            let directory = tempfile::tempdir().unwrap();
            *state.store.lock().await =
                Some(ServerStore::open(directory.path(), b"read test", &mut OsCryptoRng).unwrap());
            *state.session_resumable.lock().await = true;
            let path = directory.path().join("already-published.bin");
            std::fs::write(&path, b"verified file").unwrap();
            let entry = EntryOwner::new();
            let origin = entry.scope(current(&state, 0).unwrap());
            let inspected = std::cell::Cell::new(false);
            let revealed = std::cell::Cell::new(false);
            if edge == 0 {
                entry.retire();
            }
            assert!(finish_saved_read(
                &state,
                0,
                &origin,
                &path,
                "audio/wav",
                |_, _| {
                    inspected.set(true);
                    if edge == 1 {
                        entry.retire();
                    }
                    if edge == 3 {
                        register_ui_lock_request(&state);
                    }
                    None
                },
                |_| {
                    revealed.set(true);
                    if edge == 2 {
                        entry.retire();
                    }
                    Ok(())
                }
            )
            .await
            .is_err());
            assert_eq!(inspected.get(), edge != 0);
            assert_eq!(revealed.get(), edge == 2);
            assert_eq!(
                std::fs::read(&path).unwrap(),
                b"verified file",
                "publication is not rolled back"
            );
        }
    }

    #[tokio::test]
    async fn a_recovered_save_returns_its_path_only_if_still_authorized_after_cleanup() {
        // Review U-M4: the preflight found an already linked, verified file. Cleanup is blocked
        // while Lock, an entry retirement or an actor/Root change completes; the stale response
        // must be refused and the published file kept.
        for edge in 0..4 {
            let state = AppState::default();
            let directory = tempfile::tempdir().unwrap();
            *state.store.lock().await =
                Some(ServerStore::open(directory.path(), b"read test", &mut OsCryptoRng).unwrap());
            *state.session_resumable.lock().await = true;
            let path = directory.path().join("recovered.bin");
            std::fs::write(&path, b"verified file").unwrap();
            let entry = EntryOwner::new();
            let origin = entry.scope(current(&state, 0).unwrap());
            let actor_current = std::cell::Cell::new(true);
            let (cleanup_done, cleanup_wait) = tokio::sync::oneshot::channel::<bool>();
            let mut response = Box::pin(private_downloads::finish_recovered_save(
                &state,
                0,
                &origin,
                &path,
                || actor_current.get(),
                |_| Ok(()),
                async {
                    match cleanup_wait.await {
                        Ok(true) => Ok(()),
                        _ => Err("cleanup failed".to_string()),
                    }
                },
            ));
            std::future::poll_fn(|cx| {
                assert!(response.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            match edge {
                0 => {
                    register_ui_lock_request(&state);
                }
                1 => entry.retire(),
                2 => actor_current.set(false),
                _ => {} // still authorized; the cleanup itself fails
            }
            cleanup_done.send(edge != 3).unwrap();
            let outcome = response.await;
            if edge == 3 {
                let saved = outcome.expect("a live session still gets its saved file");
                assert!(
                    saved.warning.is_some(),
                    "a failed cleanup is reported, not discarded"
                );
            } else {
                assert!(
                    outcome.is_err(),
                    "edge {edge}: no path into a retired invocation"
                );
            }
            assert_eq!(std::fs::read(&path).unwrap(), b"verified file");
        }
    }

    #[tokio::test]
    async fn private_download_cleanup_wait_cannot_return_success_into_a_retired_invocation() {
        for edge in 0..3 {
            let state = AppState::default();
            let directory = tempfile::tempdir().unwrap();
            *state.store.lock().await =
                Some(ServerStore::open(directory.path(), b"read test", &mut OsCryptoRng).unwrap());
            *state.session_resumable.lock().await = true;
            let path = directory.path().join("already-published.bin");
            std::fs::write(&path, b"verified file").unwrap();
            let entry = EntryOwner::new();
            let origin = entry.scope(current(&state, 0).unwrap());
            let actor_current = std::cell::Cell::new(true);
            let (cleanup_done, cleanup_wait) = tokio::sync::oneshot::channel();
            let mut response = Box::pin(finish_download_response(
                &state,
                0,
                &origin,
                SavedFileResult {
                    path: path.to_string_lossy().into_owned(),
                    displayed: true,
                    warning: None,
                    content_validation: None,
                },
                || actor_current.get(),
                async { cleanup_wait.await.map_err(|_| "cleanup stopped".into()) },
            ));
            std::future::poll_fn(|cx| {
                assert!(response.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            match edge {
                0 => {
                    register_ui_lock_request(&state);
                }
                1 => entry.retire(),
                _ => actor_current.set(false),
            }
            cleanup_done.send(()).unwrap();
            assert!(response.await.is_err());
            assert_eq!(
                std::fs::read(&path).unwrap(),
                b"verified file",
                "response refusal never unpublishes an already saved file"
            );
        }
    }

    #[tokio::test]
    async fn original_actor_row_cannot_be_replaced_under_a_queued_read() {
        let state = AppState::default();
        let (old, old_task, old_drain) = leaving::tests::running(941);
        let actor = old.actor.clone();
        state.servers.lock().await.insert(1, old);
        let original = for_actor(&state, 1, &actor, 0).await.unwrap();
        let (new, new_task, new_drain) = leaving::tests::running(942);
        let replacement_actor = new.actor.clone();
        drop(state.servers.lock().await.insert(1, new));
        assert!(original.is_cancelled());
        assert!(for_actor(&state, 1, &actor, 0).await.is_none());
        let replacement = for_actor(&state, 1, &replacement_actor, 0).await.unwrap();
        assert!(!replacement.is_cancelled());
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        assert!(replacement.is_cancelled());
        actor.shutdown().await;
        replacement_actor.shutdown().await;
        old_task.await.unwrap();
        old_drain.await.unwrap();
        new_task.await.unwrap();
        new_drain.await.unwrap();
    }
}
