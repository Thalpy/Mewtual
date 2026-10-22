use super::*;
use image::AnimationDecoder;
use std::cell::Cell;

fn golden() -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json"
    )))
    .unwrap();
    hex::decode(fixture["pixaHex"].as_str().unwrap()).unwrap()
}
fn repeated(count: usize, fps: u8) -> Vec<u8> {
    let bytes = golden();
    let p = catcoms_app::creative::validate_pixa(&bytes).unwrap();
    let frame = p.frames()[0];
    let tail = &bytes[7 + 4 + frame.len()..];
    let mut out = b"PIXA".to_vec();
    out.push(fps);
    out.extend((count as u16).to_le_bytes());
    for _ in 0..count {
        out.extend((frame.len() as u32).to_le_bytes());
        out.extend(frame);
    }
    out.extend(tail);
    out
}

#[test]
fn studio_gif_timing_palette_and_maximal_frame_count_are_bounded() {
    for fps in 1..=24 {
        let count = 25;
        let gif = encode_gif(&repeated(count, fps), GIF_MAX_BYTES, || false).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        assert_eq!(gif.last(), Some(&0x3b));
        let decoded = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&gif))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(decoded.len(), count);
        let mut elapsed = 0u32;
        for (i, frame) in decoded.iter().enumerate() {
            assert_eq!(frame.buffer().dimensions(), (192, 144));
            assert_eq!(
                frame.buffer().get_pixel(0, 0).0,
                [0x13, 0x12, 0x18, 255],
                "declared fallback palette"
            );
            let (num, den) = frame.delay().numer_denom_ms();
            assert_eq!(den, 1);
            elapsed += num;
            assert_eq!(elapsed, (((i + 1) * 100 / fps as usize) * 10) as u32);
        }
    }
    let gif = encode_gif(&repeated(999, 24), GIF_MAX_BYTES, || false).unwrap();
    assert_eq!(
        image::codecs::gif::GifDecoder::new(std::io::Cursor::new(gif))
            .unwrap()
            .into_frames()
            .count(),
        999
    );
}

#[test]
fn studio_gif_output_cap_including_drop_trailer_cancellation_and_audio_validation() {
    let input = golden();
    let gif = encode_gif(&input, GIF_MAX_BYTES, || false).unwrap();
    assert_eq!(encode_gif(&input, gif.len(), || false).unwrap(), gif);
    assert!(
        encode_gif(&input, gif.len() - 1, || false).is_err(),
        "the final trailer write must not silently overflow on Drop"
    );
    assert!(encode_gif(&input, 0, || false).is_err());
    assert!(encode_gif(&input, GIF_MAX_BYTES, || true).is_err());
    let checks = Cell::new(0);
    assert!(encode_gif(&repeated(3, 12), GIF_MAX_BYTES, || {
        checks.set(checks.get() + 1);
        checks.get() > 2
    })
    .is_err());
    let mut bad = input.clone();
    let at = bad.windows(5).position(|w| w == b"\"v\":1").unwrap();
    bad[at + 4] = b'2';
    assert!(
        encode_gif(&bad, GIF_MAX_BYTES, || false).is_err(),
        "ignored audio still validates fully"
    );
    assert!(decode_animation_input(
        &"A".repeat(catcoms_app::creative::MAX_BOUNDED_BLOB_BYTES.div_ceil(3) * 4 + 1)
    )
    .is_err());
    assert!(decode_animation_input("not base64").is_err());
}

#[test]
fn studio_gif_safe_download_never_overwrites_and_reports_reveal_failure() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(GIF_NAME), b"existing").unwrap();
    let gif = encode_gif(&golden(), GIF_MAX_BYTES, || false).unwrap();
    let abandoned = stage_gif(dir.path(), &gif, || false, || {}).unwrap();
    assert_eq!(std::fs::read(&abandoned.staging).unwrap(), gif);
    assert!(
        !abandoned.path.exists(),
        "the final name stays absent until publication"
    );
    drop(abandoned);
    let mut staged = stage_gif(dir.path(), &gif, || false, || {}).unwrap();
    staged.publish().unwrap();
    let saved = revealed(&staged.path, |_| Err("no desktop".into()));
    drop(staged);
    assert!(!saved.displayed);
    assert!(saved.warning.unwrap().contains("saved"));
    assert_eq!(
        std::fs::read(dir.path().join(GIF_NAME)).unwrap(),
        b"existing"
    );
    assert_eq!(std::fs::read(saved.path).unwrap(), gif);
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".part")));
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "an unpublished stage removes only its own staging file"
    );
}

#[test]
fn studio_export_collision_never_replaces_or_cleans_up_another_local_file() {
    let dir = tempfile::tempdir().unwrap();
    let gif = encode_gif(&golden(), GIF_MAX_BYTES, || false).unwrap();
    for publish in [false, true] {
        let mut staged = stage_gif(dir.path(), &gif, || false, || {}).unwrap();
        let final_path = staged.path.clone();
        let staging_path = staged.staging.clone();
        std::fs::write(&final_path, b"local file created during export").unwrap();
        if publish {
            assert!(staged.publish().is_err());
        }
        drop(staged);
        assert_eq!(
            std::fs::read(final_path).unwrap(),
            b"local file created during export"
        );
        assert!(!staging_path.exists());
    }
}

#[test]
fn studio_export_link_is_committed_when_staging_cleanup_fails() {
    let dir = tempfile::tempdir().unwrap();
    let gif = encode_gif(&golden(), GIF_MAX_BYTES, || false).unwrap();
    let mut staged = stage_gif(dir.path(), &gif, || false, || {}).unwrap();
    let (path, staging) = (staged.path.clone(), staged.staging.clone());
    staged
        .publish_with_cleanup(|_, _| Err(std::io::ErrorKind::PermissionDenied.into()))
        .unwrap();
    assert!(staged.published);
    drop(staged);
    assert_eq!(std::fs::read(path).unwrap(), gif);
    assert!(
        staging.exists(),
        "Drop cannot reclassify a committed export as abandoned"
    );
}

async fn unlocked_state(vault: &Path) -> (AppState, u64) {
    let state = AppState::default();
    *state.store.lock().await =
        Some(ServerStore::open(vault, b"correct horse", &mut OsCryptoRng).unwrap());
    *state.session_resumable.lock().await = true;
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    (state, generation)
}

/// Register a real actor as server 12, as the export command would find it.
async fn register_server(
    state: &AppState,
    instance: u64,
) -> (ServerActor, impl Sized, tokio::task::JoinHandle<()>) {
    use catcoms_rt::{Hub, ManualClock};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    let server = Server::found(
        Hub::new().join(PeerId::from_u64(91)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(91),
        Box::new(ManualClock::new(1000)),
        "alice",
    )
    .unwrap();
    let group_id = server.group_id();
    let device_id = server.device_id();
    let (actor, events, task) = spawn(server);
    state.servers.lock().await.insert(
        12,
        ServerEntry {
            files: keep_session::EntryOwner::new(),
            actor: actor.clone(),
            instance,
            group_id,
            device_id,
            invite: None,
            name: "test".into(),
            bootstrap: vec![],
            bootstrap_owners: HashMap::new(),
            interface_routes: None,
            rendezvous: vec![],
            mesh: None,
            is_dm: false,
            switchboard: false,
            record_seq: 0,
            voice: None,
            persist: PersistCounters::default(),
        },
    );
    (actor, events, task)
}

#[tokio::test]
async fn studio_gif_stale_session_and_missing_server_never_write_or_reveal() {
    let vault = tempfile::tempdir().unwrap();
    let (state, generation) = unlocked_state(vault.path()).await;
    let dir = tempfile::tempdir().unwrap();
    let gif = encode_gif(&golden(), GIF_MAX_BYTES, || false).unwrap();
    let (_signal, rx) = watch::channel(false);
    let cancellation = RequestCancellation::new(rx, None);
    assert!(save_gif(
        &state,
        12,
        40,
        generation,
        dir.path().to_path_buf(),
        gif.clone(),
        &cancellation,
        || {},
        std::future::ready(()),
        |_| panic!("missing server must not reveal")
    )
    .await
    .is_err());
    let (actor, events, task) = register_server(&state, 41).await;
    assert!(save_gif(
        &state,
        12,
        40,
        generation,
        dir.path().to_path_buf(),
        gif.clone(),
        &cancellation,
        || {},
        std::future::ready(()),
        |_| panic!("replaced server must not reveal")
    )
    .await
    .is_err());
    let (_cancel_signal, cancel_rx) = watch::channel(true);
    let cancelled = RequestCancellation::new(cancel_rx, None);
    assert!(save_gif(
        &state,
        12,
        41,
        generation,
        dir.path().to_path_buf(),
        gif.clone(),
        &cancelled,
        || {},
        std::future::ready(()),
        |_| panic!("cancelled work must not reveal")
    )
    .await
    .is_err());
    let before_commit = async {
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
    };
    // The current instance, so the session fence after staging is what refuses publication.
    assert!(save_gif(
        &state,
        12,
        41,
        generation,
        dir.path().to_path_buf(),
        gif.clone(),
        &cancellation,
        || {},
        before_commit,
        |_| panic!("stale session must not reveal")
    )
    .await
    .is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert_eq!(
        unlocked_ui_session_generation(&state).await.unwrap_err(),
        "the vault is locked",
        "a changed generation is not an authenticated unlock"
    );
    {
        let _commit = state.ui_session_commit.lock().await;
        // This fixture already holds an unlocked vault. Explicitly model successful native
        // authentication before allowing fresh work; never restore authority to the old token.
        state.ui_authorized_generation.store(
            state.ui_session_generation.load(Ordering::Acquire),
            Ordering::Release,
        );
    }
    let current = unlocked_ui_session_generation(&state).await.unwrap();
    assert_ne!(current, generation);
    assert!(save_gif(
        &state,
        12,
        41,
        generation,
        dir.path().to_path_buf(),
        gif.clone(),
        &cancellation,
        || {},
        std::future::ready(()),
        |_| panic!("unlock must not revive the old generation")
    )
    .await
    .is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let result = save_gif(
        &state,
        12,
        41,
        current,
        dir.path().to_path_buf(),
        gif.clone(),
        &cancellation,
        || {},
        std::future::ready(()),
        |_| Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(result.path).unwrap(), gif);
    actor.shutdown().await;
    task.await.unwrap();
    drop(events);
}

fn staging_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        })
        .count()
}

/// A stalled `.part` write waits inside the staging worker, so unrelated commands still take the
/// UI-session guard and the server registry. Cancellation during the stall removes both files;
/// an uncancelled stall publishes once released and reveals with neither lock held.
#[tokio::test]
async fn studio_gif_stalled_staging_holds_no_lock_and_cancels_cleanly() {
    let vault = tempfile::tempdir().unwrap();
    let (state, generation) = unlocked_state(vault.path()).await;
    let (actor, events, task) = register_server(&state, 41).await;
    let dir = tempfile::tempdir().unwrap();
    let gif = encode_gif(&golden(), GIF_MAX_BYTES, || false).unwrap();
    for cancel in [true, false] {
        let (signal, rx) = watch::channel(false);
        let cancellation = RequestCancellation::new(rx, None);
        let (entered, stalled) = tokio::sync::oneshot::channel::<()>();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let mut entered = Some(entered);
        let stall = move || {
            if let Some(entered) = entered.take() {
                let _ = entered.send(());
                let _ = released.recv();
            }
        };
        let revealed_unlocked = Cell::new(false);
        let save = save_gif(
            &state,
            12,
            41,
            generation,
            dir.path().to_path_buf(),
            gif.clone(),
            &cancellation,
            stall,
            std::future::ready(()),
            |_| {
                assert!(!cancel, "cancelled work must not reveal");
                let session_free = state.ui_session_commit.try_lock().is_ok();
                let registry_free = state.servers.try_lock().is_ok();
                revealed_unlocked.set(session_free && registry_free);
                Ok(())
            },
        );
        let probe = async {
            stalled.await.unwrap();
            let unrelated = timeout(Duration::from_secs(10), async {
                drop(
                    require_ui_session_generation(&state, generation)
                        .await
                        .unwrap(),
                );
                drop(state.servers.lock().await);
            })
            .await;
            assert!(
                unrelated.is_ok(),
                "cancel={cancel}: a stalled GIF write must not hold either lock"
            );
            assert_eq!(staging_files(dir.path()), 1, "the stall is inside staging");
            if cancel {
                signal.send(true).unwrap();
            }
            release.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(save, probe);
        if cancel {
            assert_eq!(result.err().as_deref(), Some("request cancelled"));
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                0,
                "cancellation removes the staging file and the reserved name"
            );
        } else {
            let saved = result.unwrap();
            assert!(saved.displayed);
            assert!(
                revealed_unlocked.get(),
                "reveal runs after the critical section"
            );
            assert_eq!(std::fs::read(&saved.path).unwrap(), gif);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }
    actor.shutdown().await;
    task.await.unwrap();
    drop(events);
}
