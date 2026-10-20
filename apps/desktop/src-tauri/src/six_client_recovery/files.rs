//! File recovery through actual TCP, native actor ownership and the sealed retained store.
use super::*;

async fn native_read(running: &Running) -> RequestCancellation {
    let generation = keep_session::invocation_generation(&running.state)
        .await
        .unwrap();
    keep_session::for_actor(&running.state, 1, &running.actor, generation)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_keep_uses_member_bridge_without_a_direct_edge_and_survives_solo_native_reopen() {
    bridged_saved_copy(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn private_download_bridges_without_becoming_a_source_and_exports_after_solo_native_reopen() {
    bridged_saved_copy(true).await;
}

/// Both purposes traverse the same production actor worker but retain distinct storage/source
/// contracts. Test the actual native attachment on both sides rather than assuming a trait
/// forwarding wrapper preserves private Download support.
async fn bridged_saved_copy(private: bool) {
    let mut scenario = Scenario::new(false);
    scenario.admit_prefix_with_files(3, true).await;
    // A=1 and C=2 are forbidden at the actual libp2p behaviour, throughout admission and read.
    // B=0 is a server member, with no file or call UI opened and no ciphertext copy.
    let bytes: Vec<_> = (0..150_000).map(|i| (i % 251) as u8).collect();
    let cid = bounded(
        "source publishes file",
        scenario.online(1).actor.add_file(
            "bridged.bin".into(),
            "application/octet-stream".into(),
            String::new(),
            bytes.clone(),
        ),
    )
    .await
    .unwrap();
    let raw = hex::decode(&cid).unwrap();
    bounded("file directory reaches requester through B", async {
        loop {
            if scenario.online(2).actor.files().await.iter().any(|entry| {
                catcoms_storage::FileManifest::decode_or_legacy(&entry.file_ref)
                    .is_ok_and(|m| hex::encode(m.plaintext_cid.as_bytes()) == cid)
            }) {
                break;
            }
            SystemClock.sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert_eq!(
        scenario
            .online(0)
            .actor
            .file_download_plan(raw.clone())
            .await,
        Some((1, bytes.len() as u64)),
        "B has the exact single-chunk manifest"
    );
    assert!(!scenario.online(0).actor.file_available(raw.clone()).await);
    let origin = native_read(scenario.online(2)).await;
    let download_id = if private {
        let ready = bounded(
            "native private download fetches encrypted pages through B",
            scenario
                .online(2)
                .actor
                .acquire_download(raw.clone(), None, origin),
        )
        .await
        .unwrap();
        assert!(ready.is_current());
        Some(ready.id())
    } else {
        bounded(
            "native Keep fetches encrypted pages through B",
            scenario
                .online(2)
                .actor
                .keep_file(raw.clone(), Some(origin)),
        )
        .await
        .unwrap();
        None
    };
    let kept = scenario.online(2).actor.kept_files().await;
    let expected_plan = if let Some(id) = download_id {
        assert!(kept.files.is_empty() && kept.transfers.is_empty());
        assert!(!scenario.online(2).actor.file_available(raw.clone()).await);
        let saved = scenario.online(2).actor.downloads().await.unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].checked && saved[0].id == id);
        saved[0].state.plan.clone()
    } else {
        assert_eq!(kept.files.len(), 1);
        assert!(kept.files[0].checked);
        kept.files[0].plan.clone()
    };
    assert_eq!(
        scenario
            .online(0)
            .actor
            .file_download_plan(raw.clone())
            .await,
        Some((1, bytes.len() as u64))
    );
    assert!(
        !scenario.online(0).actor.file_available(raw.clone()).await,
        "forwarding must not silently retain a copy at B"
    );
    {
        let trace = scenario.rules.trace.lock().unwrap();
        assert!(
            trace.received_kinds.get(&(0, 2, 29)).copied().unwrap_or(0) >= 3,
            "actual requester/helper pages must reach B: {:?}",
            trace.received_kinds
        );
        assert!(
            trace.received_kinds.get(&(1, 0, 30)).copied().unwrap_or(0) >= 3,
            "actual helper/source pages must reach A: {:?}",
            trace.received_kinds
        );
    }
    scenario.rules.assert_graph();
    scenario.close(&[0, 1, 2]).await;
    scenario.reopen_in(&[2], false).await;
    let origin = native_read(scenario.online(2)).await;
    let actual = if let Some(id) = download_id {
        let saved = scenario.online(2).actor.downloads().await.unwrap();
        assert!(
            !saved[0].checked && !saved[0].state.running,
            "restart never auto-exports plaintext"
        );
        assert_eq!(saved[0].state.plan, expected_plan);
        let slot = private_downloads::export_slot().unwrap();
        let ready = bounded(
            "solo reopened private Download rechecks encrypted bytes",
            scenario
                .online(2)
                .actor
                .acquire_download(raw.clone(), Some(id), origin.clone()),
        )
        .await
        .unwrap()
        .with_export_custody(slot);
        assert!(!scenario.online(2).actor.file_available(raw.clone()).await);
        let destination = tempfile::tempdir().unwrap();
        let mut source = PrivateExportSource {
            running: scenario.online(2),
            ready,
            origin,
            generation: 0,
        };
        let target = source.ready.cid();
        let path = bounded(
            "native private export streams to a fresh destination",
            stream_download_to_disk(
                destination.path(),
                "bridged.bin",
                source.ready.chunks(),
                source.ready.total_size(),
                target.as_bytes(),
                &mut source,
            ),
        )
        .await
        .unwrap();
        let actual = std::fs::read(path).unwrap();
        source
            .running
            .actor
            .finish_download_export(source.ready.clone())
            .await
            .unwrap();
        assert!(source.running.actor.downloads().await.unwrap().is_empty());
        actual
    } else {
        bounded(
            "solo reopened Keep verifies saved bytes",
            scenario
                .online(2)
                .actor
                .keep_file(raw.clone(), Some(origin.clone())),
        )
        .await
        .unwrap();
        let reopened = scenario.online(2).actor.kept_files().await;
        assert_eq!(reopened.files[0].plan, expected_plan);
        assert!(reopened.files[0].checked);
        let (actual, _) = bounded(
            "solo reopened file reads locally",
            scenario
                .online(2)
                .actor
                .fetch_file_chunk_cancellable(raw, 0, Some(origin)),
        )
        .await
        .unwrap();
        actual
    };
    assert_eq!(actual, bytes);
    scenario.rules.assert_graph();
    scenario.close(&[2]).await;
}

/// Production stream/publication seam driven by actual native-owned private chunks. Only the
/// AppHandle progress sink is omitted; no plaintext or wrapped reference enters a mock IPC UI.
struct PrivateExportSource<'a> {
    running: &'a Running,
    ready: catcoms_app::DownloadReady,
    origin: RequestCancellation,
    generation: u64,
}
impl SaveSource for PrivateExportSource<'_> {
    async fn chunk(&mut self, index: usize) -> Result<(Vec<u8>, Option<String>), String> {
        self.running
            .actor
            .read_download_chunk(self.ready.clone(), index, self.origin.clone())
            .await
            .map(|chunk| (chunk.into_bytes(), None))
    }
    async fn still_unlocked(&self) -> Result<(), String> {
        drop(keep_session::read_commit(&self.running.state, self.generation, &self.origin).await?);
        if !self.ready.is_current() {
            return Err("Download retired.".into());
        }
        Ok(())
    }
    async fn publish_verified(&self, staging: &Path, final_path: &Path) -> Result<(), String> {
        let _commit =
            keep_session::read_commit(&self.running.state, self.generation, &self.origin).await?;
        if !self.ready.is_current() {
            return Err("Download retired.".into());
        }
        publish_staged_download(staging, final_path)
    }
    fn progress(&self, _: usize, _: u64, _: u64, _: Option<String>) {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_bridge_keep_preserves_spend_on_native_reopen_and_recovers_through_d() {
    interrupted_saved_bridge(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_private_download_retains_spend_and_resumes_through_d_after_native_restart() {
    interrupted_saved_bridge(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_keep_survives_bridge_restart_without_a_second_user_request() {
    active_bridge_restart(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_private_download_survives_bridge_restart_without_manual_resume() {
    active_bridge_restart(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_private_download_recovers_when_b_returns_after_the_first_reconnect_admission() {
    active_bridge_restart(true, true).await;
}

/// Retire the actual B connection after its first signed response, while C's original native
/// invocation is still active. Reopening B must recover that same intent; a new Resume call
/// would hide the exact production stranding bug this scenario is meant to exercise.
async fn active_bridge_restart(private: bool, later_return: bool) {
    let mut scenario = Scenario::new(false);
    scenario.admit_prefix_with_files(3, true).await;
    // Consume the ordinary startup discovery pass before the restart. Depending on incidental
    // setup speed let the old test pass by borrowing its first tick; recovery must also work
    // just AFTER that tick, while the next periodic pass is still 45–75 seconds away.
    bounded(
        "initial native discovery completes before active bridge read",
        async {
            while (0..3).any(|index| scenario.online(index).passes.load(Ordering::Acquire) == 0) {
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        },
    )
    .await;
    let bytes: Vec<_> = (0..150_000).map(|i| (i % 233) as u8).collect();
    let cid = scenario
        .online(1)
        .actor
        .add_file(
            "bridge-restart.bin".into(),
            "application/octet-stream".into(),
            String::new(),
            bytes.clone(),
        )
        .await
        .unwrap();
    let raw = hex::decode(cid).unwrap();
    bounded(
        "file manifest reaches C before active bridge restart",
        async {
            while scenario
                .online(2)
                .actor
                .file_download_plan(raw.clone())
                .await
                .is_none()
            {
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        },
    )
    .await;
    let actor = scenario.online(2).actor.clone();
    let origin = native_read(scenario.online(2)).await;
    let (entered, held) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *scenario.rules.file_reply.lock().unwrap() = Some(transport::HeldFileReply {
        entered,
        release: resume,
    });
    let request = raw.clone();
    let keeping = tokio::spawn(async move {
        if private {
            actor
                .acquire_download(request, None, origin)
                .await
                .map(Some)
        } else {
            actor.keep_file(request, Some(origin)).await.map(|_| None)
        }
    });
    bounded(
        "first real signed bridge page held during connection replacement",
        held,
    )
    .await
    .unwrap();
    let original = saved_state(private, scenario.online(2)).await;
    assert!(original.running && original.verified_chunks == 0);
    let dial_count = || {
        scenario
            .rules
            .trace
            .lock()
            .unwrap()
            .submitted_peer_dials
            .get(&(2, 0))
            .copied()
            .unwrap_or(0)
    };
    let dials_before = dial_count();
    scenario.close(&[0]).await;
    if later_return {
        bounded(
            "C spends its first actual background reconnect opportunity while B is closed",
            async {
                while scenario
                    .rules
                    .trace
                    .lock()
                    .unwrap()
                    .submitted_peer_dials
                    .get(&(2, 0))
                    .copied()
                    .unwrap_or(0)
                    <= dials_before
                {
                    SystemClock.sleep(Duration::from_millis(20)).await;
                }
            },
        )
        .await;
        // Keep the listener absent beyond the confirmed local admission. Submission is not
        // terminal failure: the original driver-owned attempt may still succeed when B returns.
        // Either that path or a later scheduled opportunity may recover this same user request.
        SystemClock.sleep(Duration::from_secs(1)).await;
        assert!(!scenario
            .online(2)
            .actor
            .member_routes()
            .await
            .iter()
            .any(|row| row.peer_id == Some(*scenario.rules.peers[0].as_bytes()) && row.connected));
    }
    scenario.reopen_in(&[0], false).await;
    assert_eq!(
        scenario.online(0).actor.members().await.len(),
        3,
        "this scenario retains its original three-member roster"
    );
    let _ = release.send(()); // the canceled old observer may already have dropped its receiver
    let ready = tokio::select! {
        ready = keeping => ready,
        _ = SystemClock.sleep(Duration::from_secs(30)) => {
            let mut observations = Vec::new();
            for index in 0..3 {
                observations.push(scenario.recipient_diagnostic(
                    index, channel_id("general"), &History::new(),
                ).await);
            }
            let received = scenario.rules.trace.lock().unwrap().received_kinds.clone();
            panic!("original active read did not recover through restarted B: {observations:?}; received kinds={received:?}");
        }
    }
        .unwrap()
        .unwrap();
    if let Some(ready) = ready {
        assert!(ready.is_current());
        assert_eq!(ready.id().as_bytes(), &original.transfer);
        let done = saved_state(true, scenario.online(2)).await;
        assert_eq!(done.intent, original.intent);
        assert_eq!(done.grant, original.grant);
        assert!(
            done.remaining.requests <= original.remaining.requests,
            "bridge replacement must never refill spent durable work"
        );
        let ready = ready.with_export_custody(private_downloads::export_slot().unwrap());
        let actual = scenario
            .online(2)
            .actor
            .read_download_chunk(ready, 0, native_read(scenario.online(2)).await)
            .await
            .unwrap()
            .into_bytes();
        assert_eq!(
            actual, bytes,
            "the original private invocation recovered the exact file"
        );
    } else {
        let kept = scenario.online(2).actor.kept_files().await;
        assert_eq!(kept.files.len(), 1);
        assert!(kept.files[0].checked);
        assert_eq!(kept.files[0].plan, original.plan);
        let (chunk, _) = scenario
            .online(2)
            .actor
            .fetch_file_chunk(raw.clone(), 0)
            .await
            .unwrap();
        assert_eq!(chunk, bytes);
    }
    assert!(
        !scenario.online(0).actor.file_available(raw).await,
        "B forwards ciphertext without gaining a retained copy"
    );
    assert!(
        scenario
            .rules
            .trace
            .lock()
            .unwrap()
            .received_kinds
            .get(&(0, 2, 29))
            .copied()
            .unwrap_or(0)
            >= 2,
        "the original read must submit a fresh bridge request"
    );
    scenario.rules.assert_graph();
    scenario.close(&[0, 1, 2]).await;
}

async fn saved_state(private: bool, running: &Running) -> catcoms_storage::kept::KeptTransfer {
    if private {
        running.actor.downloads().await.unwrap().remove(0).state
    } else {
        running.actor.kept_files().await.transfers.remove(0)
    }
}
async fn saved_pause(
    private: bool,
    running: &Running,
    raw: Vec<u8>,
    state: &catcoms_storage::kept::KeptTransfer,
    paused: bool,
) {
    if private {
        running
            .actor
            .set_download_paused(
                catcoms_storage::kept::DownloadId::from_bytes(state.transfer),
                state.intent,
                paused,
            )
            .await
            .unwrap();
    } else {
        running
            .actor
            .set_kept_paused(raw, state.transfer, state.intent, paused)
            .await
            .unwrap();
    }
}
async fn interrupted_saved_bridge(private: bool) {
    let mut scenario = Scenario::new(false);
    scenario.admit_prefix_with_files(6, true).await;
    scenario.close(&[4, 5]).await;
    let bytes: Vec<_> = (0..150_000).map(|i| (i % 239) as u8).collect();
    let cid = bounded(
        "source publishes interrupted file",
        scenario.online(1).actor.add_file(
            "resume.bin".into(),
            "application/octet-stream".into(),
            String::new(),
            bytes.clone(),
        ),
    )
    .await
    .unwrap();
    let raw = hex::decode(&cid).unwrap();
    bounded("interrupted file directory discovery", async {
        loop {
            if scenario
                .online(2)
                .actor
                .file_download_plan(raw.clone())
                .await
                .is_some()
            {
                break;
            }
            SystemClock.sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let actor = scenario.online(2).actor.clone();
    let native = native_read(scenario.online(2)).await;
    let (caller, receiver) = watch::channel(false);
    let origin = keep_session::bind_read(native, RequestCancellation::new(receiver, None));
    // Hold the actual first helper response at C. Unlike a fixed delay this remains pending
    // under a slow CI scheduler until the original durable run has been paused.
    let (entered, held) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *scenario.rules.file_reply.lock().unwrap() = Some(transport::HeldFileReply {
        entered,
        release: resume,
    });
    let request = raw.clone();
    let keeping = tokio::spawn(async move {
        if private {
            actor
                .acquire_download(request, None, origin)
                .await
                .map(|_| ())
        } else {
            actor.keep_file(request, Some(origin)).await
        }
    });
    bounded("actual first helper response held before pause", held)
        .await
        .unwrap();
    let before = saved_state(private, scenario.online(2)).await;
    assert!(before.running);
    saved_pause(private, scenario.online(2), raw.clone(), &before, true).await;
    // Cancellation may already have discarded the held observer. Neither outcome refunds
    // its durable debit or permits this old result to mutate the paused transfer.
    let _ = release.send(());
    caller.send_replace(true);
    assert!(bounded("paused original native Keep returns", keeping)
        .await
        .unwrap()
        .is_err());
    let paused = saved_state(private, scenario.online(2)).await;
    assert!(paused.user_paused && !paused.running);
    assert_eq!(
        paused.verified_chunks, 0,
        "an unfinished ciphertext chunk is not durable progress"
    );
    assert_eq!(paused.grant, before.grant);
    assert!(paused.remaining.requests <= before.remaining.requests);
    // Pin the actual replacement prerequisite under contention. This observes ordinary native
    // authenticated capture of BOTH D→A and D→C; it never seeds/dials a test-only route or
    // changes the existing reopened-recovery deadline.
    let _original_endpoints = voice::wait_for_saved_bridge_endpoints(&scenario).await;
    scenario.close(&[0, 2]).await;
    scenario.reopen(&[2]).await;
    let d_before = {
        let trace = scenario.rules.trace.lock().unwrap();
        (
            trace.received_kinds.get(&(3, 2, 29)).copied().unwrap_or(0),
            trace.received_kinds.get(&(1, 3, 30)).copied().unwrap_or(0),
        )
    };
    let recovered = saved_state(private, scenario.online(2)).await;
    assert_eq!(recovered.plan, paused.plan);
    assert_eq!(recovered.transfer, paused.transfer);
    assert_eq!(recovered.intent, paused.intent);
    assert_eq!(recovered.grant, paused.grant);
    assert_eq!(
        recovered.remaining, paused.remaining,
        "native restart supplies no new allowance"
    );
    assert!(recovered.user_paused && !recovered.running);
    if private {
        // Explicit Resume has its existing finite read deadline. Wait only for ordinary native
        // reconnection to D, without dialing or creating a test route, before simulating the
        // user's new Resume click. Startup itself must NOT begin private download work.
        assert!(
            !scenario.online(2).actor.downloads().await.unwrap()[0]
                .state
                .running
        );
        let d = fingerprint(&scenario.clients[3].identity.as_ref().unwrap().1);
        periodic_bounded(
            "ordinary native connected-member snapshot includes D",
            async {
                while !scenario.online(2).actor.online_members().await.contains(&d) {
                    SystemClock.sleep(Duration::from_millis(50)).await;
                }
            },
        )
        .await;
        // This is only a scheduling precondition. Actual authenticated kind29/kind30 receipt
        // and exact exported bytes below remain the bridge/service acceptance oracles.
    }
    saved_pause(private, scenario.online(2), raw.clone(), &recovered, false).await;
    // No synthetic dial or forwarded FileRef. D's ordinary native reconnect/discovery and
    // the resumed durable intent must supply the replacement route and fresh paid grants.
    let ready = if private {
        let origin = native_read(scenario.online(2)).await;
        let id = catcoms_storage::kept::DownloadId::from_bytes(recovered.transfer);
        Some(
            periodic_bounded(
                "replacement D serves explicit private Resume",
                scenario
                    .online(2)
                    .actor
                    .acquire_download(raw.clone(), Some(id), origin),
            )
            .await
            .unwrap()
            .with_export_custody(private_downloads::export_slot().unwrap()),
        )
    } else {
        periodic_bounded("replacement D serves resumed native Keep", async {
            loop {
                if scenario
                    .online(2)
                    .actor
                    .kept_files()
                    .await
                    .files
                    .iter()
                    .any(|f| f.checked)
                {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        None
    };
    let after = saved_state(private, scenario.online(2)).await;
    assert_eq!(after.transfer, recovered.transfer);
    assert_eq!(after.grant, recovered.grant);
    assert!(after.remaining.requests < recovered.remaining.requests);
    let origin = native_read(scenario.online(2)).await;
    let actual = if let Some(ready) = ready {
        assert!(!scenario.online(2).actor.file_available(raw.clone()).await);
        assert!(scenario.online(2).actor.kept_files().await.files.is_empty());
        bounded(
            "private resumed bytes remain under original native custody",
            scenario
                .online(2)
                .actor
                .read_download_chunk(ready, 0, origin),
        )
        .await
        .unwrap()
        .into_bytes()
    } else {
        let (actual, _) = bounded(
            "recovered native file reads verified bytes",
            scenario
                .online(2)
                .actor
                .fetch_file_chunk_cancellable(raw, 0, Some(origin)),
        )
        .await
        .unwrap();
        actual
    };
    assert_eq!(actual, bytes);
    {
        let trace = scenario.rules.trace.lock().unwrap();
        assert!(
            trace.received_kinds.get(&(3, 2, 29)).copied().unwrap_or(0) >= d_before.0 + 3,
            "replacement D received the resumed page grants: {:?}",
            trace.received_kinds
        );
        assert!(
            trace.received_kinds.get(&(1, 3, 30)).copied().unwrap_or(0) >= d_before.1 + 3,
            "D actually requested A's encrypted pages: {:?}",
            trace.received_kinds
        );
    }
    scenario.rules.assert_graph();
    scenario.close(&[1, 2, 3]).await;
}
