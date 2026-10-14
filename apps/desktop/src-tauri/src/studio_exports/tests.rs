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
    let saved = write_gif(dir.path(), &gif, |_| Err("no desktop".into())).unwrap();
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
}

#[tokio::test]
async fn studio_gif_stale_session_and_missing_server_never_write_or_reveal() {
    let state = AppState::default();
    let vault = tempfile::tempdir().unwrap();
    *state.store.lock().await =
        Some(ServerStore::open(vault.path(), b"correct horse", &mut OsCryptoRng).unwrap());
    *state.session_resumable.lock().await = true;
    let generation = unlocked_ui_session_generation(&state).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let gif = encode_gif(&golden(), GIF_MAX_BYTES, || false).unwrap();
    let (_signal, rx) = watch::channel(false);
    let cancellation = RequestCancellation::new(rx, None);
    assert!(save_gif(
        &state,
        12,
        40,
        generation,
        dir.path(),
        &gif,
        &cancellation,
        std::future::ready(()),
        |_| panic!("missing server must not reveal")
    )
    .await
    .is_err());
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
            actor: actor.clone(),
            instance: 41,
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
            persist: PersistCounters::default(),
        },
    );
    assert!(save_gif(
        &state,
        12,
        40,
        generation,
        dir.path(),
        &gif,
        &cancellation,
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
        dir.path(),
        &gif,
        &cancelled,
        std::future::ready(()),
        |_| panic!("cancelled work must not reveal")
    )
    .await
    .is_err());
    let before_commit = async {
        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
    };
    assert!(save_gif(
        &state,
        12,
        40,
        generation,
        dir.path(),
        &gif,
        &cancellation,
        before_commit,
        |_| panic!("stale session must not reveal")
    )
    .await
    .is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let current = unlocked_ui_session_generation(&state).await.unwrap();
    let result = save_gif(
        &state,
        12,
        41,
        current,
        dir.path(),
        &gif,
        &cancellation,
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
