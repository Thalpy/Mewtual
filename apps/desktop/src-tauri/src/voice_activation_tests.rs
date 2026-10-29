//! Actual native bridge/actor barriers. `admitted_ipc` also drives Tauri JSON; none exercise WebRTC.
use super::*;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

mod admitted_ipc;
mod join_key;

pub(crate) struct Fixture {
    pub(crate) state: AppState,
    _root: tempfile::TempDir,
    pub(crate) actor: ServerActor,
    worker: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
    controller: String,
}
impl Fixture {
    pub(crate) async fn new() -> Self {
        Self::from_state(AppState::default()).await
    }
    /// Native admission regressions control the SAME clock used by the selected owner
    /// and its actual Root action, without replacing a runtime after publication.
    pub(crate) async fn with_voice_clock(clock: Arc<dyn catcoms_rt::Clock>) -> Self {
        Self::from_state(AppState {
            voice_authority: Arc::new(StdMutex::new(voice_authority::Coordinator::with_runtime(
                catcoms_sync::voice_v2::runtime::Runtime::new(clock),
            ))),
            ..AppState::default()
        })
        .await
    }
    async fn from_state(state: AppState) -> Self {
        let root = tempfile::tempdir().unwrap();
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"test", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(1)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(71),
            Box::new(ManualClock::new(1000)),
            "member",
        )
        .unwrap();
        let group = server.group_id();
        let device = server.device_id();
        let (actor, mut events, worker) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        state.servers.lock().await.insert(
            7,
            tests::snapshot_test_entry(1, actor.clone(), group, device),
        );
        let generation = state.ui_session_generation.load(Ordering::Acquire);
        finalize_unlock_session(&state, generation, false)
            .await
            .unwrap();
        let controller = prepare_voice_controller_inner(&state).await.unwrap();
        activate_voice_owner_inner(&state, controller.clone())
            .await
            .unwrap();
        Self {
            state,
            _root: root,
            actor,
            worker,
            drain,
            controller,
        }
    }
    pub(crate) async fn candidate(&self) -> String {
        prepare_voice_owner_inner(
            &self.state,
            self.controller.clone(),
            7,
            channel_id("general").to_string(),
        )
        .await
        .unwrap()
    }
    pub(crate) async fn close(self) {
        self.actor.shutdown().await;
        self.worker.await.unwrap();
        self.drain.await.unwrap();
    }
}

fn same_projection(first: &VoiceActivation, second: &VoiceActivation) {
    assert_eq!(first.owner, second.owner);
    assert_eq!(first.group, second.group);
    assert_eq!(first.join, second.join);
}

#[tokio::test]
async fn accepted_activation_reply_loss_recovers_original_join_and_controller() {
    let f = Fixture::new().await;
    let candidate = f.candidate().await;
    let first = activate_voice_owner_inner(&f.state, candidate.clone())
        .await
        .unwrap();
    let lease = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .owner(&candidate)
        .unwrap()
        .lease;
    // Treat the first reply as lost after the actual native installation. No candidate remains.
    let recovered = activate_voice_owner_inner(&f.state, candidate.clone())
        .await
        .unwrap();
    same_projection(&first, &recovered);
    let controller = activate_voice_owner_inner(&f.state, f.controller.clone())
        .await
        .unwrap();
    assert!(controller.group.is_none() && controller.join.is_none());
    let current = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .owner(&candidate)
        .unwrap();
    assert!(current.lease.is_same(&lease) && lease.is_live());
    f.close().await;
}

#[tokio::test]
async fn concurrent_activation_replies_share_the_first_admitted_join() {
    let f = Fixture::new().await;
    let candidate = f.candidate().await;
    let boundary = f.state.ui_session_commit.lock().await;
    let (first, second, ()) = tokio::join!(biased;
        activate_voice_owner_inner(&f.state, candidate.clone()),
        activate_voice_owner_inner(&f.state, candidate.clone()),
        async {
            // Both real grant queries precede this actor command. Both early candidate lookups
            // happened before either final admission; the second must recover at the tail.
            f.actor.member_count().await;
            drop(boundary);
        }
    );
    let first = first.unwrap();
    let second = second.unwrap();
    same_projection(&first, &second);
    let recovered = activate_voice_owner_inner(&f.state, candidate)
        .await
        .unwrap();
    same_projection(&first, &recovered);
    f.close().await;
}

#[tokio::test]
async fn accepted_activation_end_before_final_admission_cannot_revive_owner() {
    let f = Fixture::new().await;
    let candidate = f.candidate().await;
    activate_voice_owner_inner(&f.state, candidate.clone())
        .await
        .unwrap();
    let boundary = f.state.ui_session_commit.lock().await;
    let (result, ()) = tokio::join!(biased;
        activate_voice_owner_inner(&f.state, candidate.clone()),
        async {
            f.actor.member_count().await;
            f.state.voice_authority.lock().unwrap().end(&candidate);
            drop(boundary);
        }
    );
    assert!(result.is_err());
    assert!(activate_voice_owner_inner(&f.state, candidate)
        .await
        .is_err());
    f.close().await;
}

#[tokio::test]
async fn accepted_activation_entry_replacement_before_final_admission_is_refused() {
    let f = Fixture::new().await;
    let candidate = f.candidate().await;
    activate_voice_owner_inner(&f.state, candidate.clone())
        .await
        .unwrap();
    let boundary = f.state.ui_session_commit.lock().await;
    let (result, ()) = tokio::join!(biased;
        activate_voice_owner_inner(&f.state, candidate.clone()),
        async {
            f.actor.member_count().await;
            // Even a replacement retaining the same group/actor cannot adopt an old entry id.
            f.state.servers.lock().await.get_mut(&7).unwrap().instance = 2;
            drop(boundary);
        }
    );
    assert!(result.is_err());
    assert!(activate_voice_owner_inner(&f.state, candidate)
        .await
        .is_err());
    f.close().await;
}

#[tokio::test]
async fn refused_pending_activation_after_grant_preserves_existing_owner() {
    let f = Fixture::new().await;
    let original = f.candidate().await;
    let first = activate_voice_owner_inner(&f.state, original.clone())
        .await
        .unwrap();
    let pending = f.candidate().await;
    let boundary = f.state.ui_session_commit.lock().await;
    let (result, ()) = tokio::join!(biased;
        activate_voice_owner_inner(&f.state, pending.clone()),
        async {
            f.actor.member_count().await;
            f.state.voice_authority.lock().unwrap().end(&pending);
            drop(boundary);
        }
    );
    assert!(result.is_err());
    let recovered = activate_voice_owner_inner(&f.state, original)
        .await
        .unwrap();
    same_projection(&first, &recovered);
    assert!(f.state.voice_authority.try_lock().is_ok());
    assert!(f.state.ui_session_commit.try_lock().is_ok());
    f.close().await;
}

#[tokio::test]
async fn accepted_activation_actual_actor_stop_before_final_admission_is_refused() {
    let f = Fixture::new().await;
    let candidate = f.candidate().await;
    activate_voice_owner_inner(&f.state, candidate.clone())
        .await
        .unwrap();
    let lease = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .owner(&candidate)
        .unwrap()
        .lease;
    let boundary = f.state.ui_session_commit.lock().await;
    let (result, ()) = tokio::join!(biased;
        activate_voice_owner_inner(&f.state, candidate.clone()),
        async {
            f.actor.member_count().await;
            f.actor.stop_and_wait().await.unwrap();
            assert!(!lease.is_live(), "actual Root teardown closes its original child");
            drop(boundary);
        }
    );
    assert!(result.is_err());
    assert!(activate_voice_owner_inner(&f.state, candidate)
        .await
        .is_err());
    f.close().await;
}

#[tokio::test]
async fn accepted_activation_lock_then_unlock_never_revives_old_join() {
    let f = Fixture::new().await;
    let candidate = f.candidate().await;
    activate_voice_owner_inner(&f.state, candidate.clone())
        .await
        .unwrap();
    let lease = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .owner(&candidate)
        .unwrap()
        .lease;
    let boundary = f.state.ui_session_commit.lock().await;
    let (result, ()) = tokio::join!(biased;
        activate_voice_owner_inner(&f.state, candidate.clone()),
        async {
            f.actor.member_count().await;
            let mut locking = Box::pin(lock_session_inner(&f.state, None));
            std::future::poll_fn(|cx| {
                assert!(locking.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            }).await;
            assert!(!lease.is_live(), "lock revokes before the held UI commit can finish");
            drop(boundary);
            locking.await.unwrap();
        }
    );
    assert!(result.is_err());
    let generation = f.state.ui_session_generation.load(Ordering::Acquire);
    finalize_unlock_session(&f.state, generation, false)
        .await
        .unwrap();
    assert!(activate_voice_owner_inner(&f.state, candidate)
        .await
        .is_err());
    assert!(activate_voice_owner_inner(&f.state, f.controller.clone())
        .await
        .is_err());
    assert!(!lease.is_live());
    f.close().await;
}

#[tokio::test]
async fn accepted_activation_generation_fence_precedes_delayed_revocation_hook() {
    let f = Fixture::new().await;
    let candidate = f.candidate().await;
    activate_voice_owner_inner(&f.state, candidate.clone())
        .await
        .unwrap();
    let generation = f.state.ui_session_generation.load(Ordering::Acquire);
    {
        let authority = f.state.voice_authority.lock().unwrap();
        let accepted = authority.accepted_activation(&candidate).unwrap();
        let lease = accepted.owner().unwrap().lease.clone();
        check_voice_activation_generation(&f.state, generation).unwrap();
        // This is the actual first step of lock, deliberately separated from revoke_all: a
        // thread can publish its generation and be descheduled before it reaches Coordinator.
        let registered = register_ui_lock_request(&f.state);
        assert_ne!(registered, generation);
        assert!(
            lease.is_live(),
            "the later voice revocation hook has not run"
        );
        authority.validate_accepted_activation(&accepted).unwrap();
        assert!(
            check_voice_activation_generation(&f.state, generation).is_err(),
            "the pure generation check must refuse even while original actor owners are live"
        );
    }
    // Complete a real lock after releasing Coordinator; no lock-hook deadlock is introduced.
    lock_session_inner(&f.state, None).await.unwrap();
    f.close().await;
}
