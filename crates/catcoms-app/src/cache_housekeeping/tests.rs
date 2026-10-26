//! Housekeeping against the real production store stack (Studio protection over Keep over the
//! crash-safe primary cache), driven through `cache_pump` exactly as the actor drives it.

use super::*;
use crate::store::ServerStore;
use crate::{write_file_entry, FileExpiry, Server};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_storage::FileRef;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Node = Server<MemNetwork, ChaCha20Rng>;

struct Fixture {
    _dir: tempfile::TempDir,
    store: ServerStore,
    server: Node,
    clock: ManualClock,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(3);
    let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
    let clock = ManualClock::new(1_000);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(1),
        Box::new(clock.clone()),
        "alice",
    )
    .unwrap();
    server.set_blob_store(store.blob_store(&hex::encode(server.group_id())).unwrap());
    server.sync.blob_cache_scan(true).unwrap();
    server.open_files().await.unwrap();
    server.open_profiles().await.unwrap();
    Fixture {
        _dir: dir,
        store,
        server,
        clock,
    }
}

/// List a file that is NOT this device's verified upload (no attestation), with its one chunk in
/// the cache, as a fetched file from another member would be. Returns its chunk CID.
async fn list_fetched(server: &mut Node, name: &str, bytes: &[u8]) -> Cid {
    let (file_ref, ciphertext): (FileRef, Vec<u8>) = server
        .sync
        .seal_file(bytes, "application/octet-stream")
        .unwrap();
    server.sync.put_blob(&ciphertext).unwrap();
    let chunk = file_ref.ciphertext_cid;
    let manifest = FileManifest {
        plaintext_cid: Cid::of(bytes),
        total_size: bytes.len() as u64,
        mime: "application/octet-stream".into(),
        chunks: vec![file_ref],
    }
    .encode();
    let name = name.to_string();
    server
        .sync
        .post(DocType::FileIndex, FILE_INDEX_DOC, |doc| {
            write_file_entry(
                doc,
                &name,
                "bob",
                "",
                &manifest,
                FileExpiry::Unrecorded,
                None,
            )
        })
        .await
        .unwrap();
    chunk
}

fn size_of(server: &Node, cid: &Cid) -> u64 {
    server.sync.blob_cache_index().unwrap().sizes()[cid]
}

#[tokio::test]
async fn the_oldest_listing_leaves_first_and_own_uploads_and_files_in_use_stay() {
    let mut f = fixture().await;
    let a = list_fetched(&mut f.server, "a", &[1; 3000]).await;
    let b = list_fetched(&mut f.server, "b", &[2; 3000]).await;
    let c = list_fetched(&mut f.server, "c", &[3; 3000]).await;
    let own = f
        .server
        .add_file("mine.bin", "application/octet-stream", "", &[4; 3000])
        .await
        .unwrap();
    let usage = f.server.sync.blob_cache_usage().unwrap().bytes;
    // Room for everything but `a`: the oldest listing goes, nothing else.
    f.server
        .set_cache_limit(CacheLimit::Bytes(usage - size_of(&f.server, &a)));
    let pump = f.server.cache_pump();
    assert_eq!(pump.deleted, 1);
    assert!(!f.server.sync.has_blob(&a));
    assert!(f.server.sync.has_blob(&b) && f.server.sync.has_blob(&c));

    // `b` is on screen now; squeezing out one more file takes `c` instead, the next unpinned.
    f.server.note_cache_use(Cid::of(&[2; 3000]), [b]);
    let usage = f.server.sync.blob_cache_usage().unwrap().bytes;
    f.server
        .set_cache_limit(CacheLimit::Bytes(usage - size_of(&f.server, &b)));
    f.clock.advance_ms(PASS_INTERVAL_MS);
    f.server.cache_pump();
    assert!(f.server.sync.has_blob(&b), "in use");
    assert!(!f.server.sync.has_blob(&c));

    // With only the own upload and an in-use file left, nothing else can go: held by pins.
    f.server.set_cache_limit(CacheLimit::Bytes(1));
    f.clock.advance_ms(PASS_INTERVAL_MS);
    let pump = f.server.cache_pump();
    assert_eq!(pump.deleted, 0);
    assert!(
        f.server.file_available(&own),
        "this device's own upload is never evicted"
    );
    let status = f.server.cache_status();
    assert!(status.over_limit && status.held_by_pins);
    assert_eq!(
        status.leaving_soon_files, 0,
        "nothing pinned is ever counted as leaving"
    );

    // Once `b` is no longer in use it can leave.
    f.clock.advance_ms(IN_USE_MS + 1);
    f.server.cache_pump();
    assert!(!f.server.sync.has_blob(&b));
    assert!(f.server.file_available(&own));
}

#[tokio::test]
async fn nothing_is_evicted_before_a_limit_is_known_or_when_the_setting_is_unreadable() {
    let mut f = fixture().await;
    let a = list_fetched(&mut f.server, "a", &[1; 3000]).await;
    assert_eq!(f.server.cache_status().paused, Some(CachePause::NotReady));
    assert_eq!(f.server.cache_pump().deleted, 0);
    f.server.set_cache_limit(CacheLimit::Unreadable);
    assert_eq!(
        f.server.cache_status().paused,
        Some(CachePause::SettingUnreadable)
    );
    assert_eq!(f.server.cache_pump().deleted, 0);
    assert!(f.server.sync.has_blob(&a));
}

#[tokio::test]
async fn unknown_studio_protection_pauses_eviction_and_asks_for_a_scan() {
    let mut f = fixture().await;
    let a = list_fetched(&mut f.server, "a", &[1; 3000]).await;
    // A damaged Studio record makes the complete reference scan fail: protection is unknown.
    std::fs::write(
        f.store
            .backup_source_dir()
            .join("servers")
            .join(format!("{}.studio-epoch", "ab".repeat(32))),
        b"broken",
    )
    .unwrap();
    assert!(f.store.creative_pinned_cids().is_err());
    f.server.set_cache_limit(CacheLimit::Bytes(1));
    let pump = f.server.cache_pump();
    assert_eq!(pump.deleted, 0);
    assert!(pump.studio_scan, "the desktop's reference service is asked");
    assert_eq!(
        f.server.cache_status().paused,
        Some(CachePause::StudioUnknown)
    );
    assert!(f.server.sync.has_blob(&a));
    // Asked at most once per interval, however often the actor turns.
    assert!(!f.server.cache_pump().studio_scan);
}

#[tokio::test]
async fn a_missing_index_document_pauses_rather_than_reading_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(3);
    let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
    let mut server: Node = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(1),
        Box::new(ManualClock::new(1_000)),
        "alice",
    )
    .unwrap();
    server.set_blob_store(store.blob_store(&hex::encode(server.group_id())).unwrap());
    let held = server
        .sync
        .put_blob(b"cached before the index opened")
        .unwrap();
    server.sync.blob_cache_scan(true).unwrap();
    server.set_cache_limit(CacheLimit::Bytes(1));
    assert_eq!(server.cache_pump().deleted, 0);
    assert_eq!(
        server.cache_status().paused,
        Some(CachePause::IndexUnreadable)
    );
    assert!(
        server.sync.has_blob(&held),
        "an unreadable index never makes blobs unclaimed"
    );
}

#[tokio::test]
async fn unclaimed_blobs_from_before_the_mount_go_first_but_profile_images_stay() {
    let mut f = fixture().await;
    // Blobs present before this mount: one no row names, and this device's avatar.
    let orphan = f.server.sync.put_blob(&[9; 2000]).unwrap();
    let avatar = vec![7u8; 2000];
    let profile = crate::Profile {
        name: "alice".into(),
        avatar: avatar.clone(),
        ..Default::default()
    };
    f.server.set_profile(profile).await.unwrap();
    let avatar_cid = Cid::of(&avatar);
    let listed = list_fetched(&mut f.server, "listed", &[1; 2000]).await;
    // Re-scan as a new mount would: everything present now is in the mount set.
    f.server.sync.blob_cache_scan(true).unwrap();
    let usage = f.server.sync.blob_cache_usage().unwrap().bytes;
    f.server
        .set_cache_limit(CacheLimit::Bytes(usage - size_of(&f.server, &orphan)));
    assert_eq!(f.server.cache_pump().deleted, 1);
    assert!(!f.server.sync.has_blob(&orphan), "unclaimed goes first");
    assert!(f.server.sync.has_blob(&avatar_cid) && f.server.sync.has_blob(&listed));
    // A blob written during this mount is never unclaimed, even when nothing names it.
    let fresh = f.server.sync.put_blob(&[8; 2000]).unwrap();
    f.server.set_cache_limit(CacheLimit::Bytes(1));
    f.clock.advance_ms(PASS_INTERVAL_MS);
    f.server.cache_pump();
    assert!(f.server.sync.has_blob(&fresh));
    assert!(
        f.server.sync.has_blob(&avatar_cid),
        "profile images are never unclaimed"
    );
    assert!(
        !f.server.sync.has_blob(&listed),
        "the listed file went instead"
    );
}

#[test]
fn the_setting_record_round_trips_binds_its_group_and_refuses_bad_values() {
    let setting = CacheSetting {
        limit: Some(cache_plan::DEFAULT_CACHE_LIMIT),
        origin: CacheSettingOrigin::Migrated,
    };
    let encoded = setting.encode_for(b"group");
    assert_eq!(
        CacheSetting::decode(&encoded),
        Some((b"group".to_vec(), setting))
    );
    let unlimited = CacheSetting {
        limit: None,
        origin: CacheSettingOrigin::User,
    };
    assert_eq!(
        CacheSetting::decode(&unlimited.encode_for(b"g")).map(|(_, s)| s.as_limit()),
        Some(CacheLimit::Unlimited)
    );
    // Out-of-range limits, trailing bytes and unknown versions are unreadable, never clamped.
    let tiny = CacheSetting {
        limit: Some(1),
        origin: CacheSettingOrigin::User,
    };
    assert_eq!(CacheSetting::decode(&tiny.encode_for(b"g")), None);
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert_eq!(CacheSetting::decode(&trailing), None);
    let mut version = encoded;
    version[0] = 2;
    assert_eq!(CacheSetting::decode(&version), None);
}

#[test]
fn the_store_reads_missing_foreign_found_and_damaged_settings_distinctly() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(4);
    let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
    assert_eq!(
        store.load_cache_setting(7, b"group"),
        CacheSettingLoad::Missing
    );
    let setting = CacheSetting {
        limit: Some(2 * cache_plan::MIN_CACHE_LIMIT),
        origin: CacheSettingOrigin::User,
    };
    store
        .save_cache_setting(7, b"group", &setting, &mut rng)
        .unwrap();
    assert_eq!(
        store.load_cache_setting(7, b"group"),
        CacheSettingLoad::Found(setting)
    );
    assert_eq!(
        store.load_cache_setting(7, b"other group"),
        CacheSettingLoad::Missing,
        "a record for another group never applies"
    );
    std::fs::write(
        store.backup_source_dir().join("servers").join("7.storage"),
        b"damaged beyond the seal",
    )
    .unwrap();
    assert_eq!(
        store.load_cache_setting(7, b"group"),
        CacheSettingLoad::Unreadable
    );
    store.remove_server(7).unwrap();
    assert_eq!(
        store.load_cache_setting(7, b"group"),
        CacheSettingLoad::Missing
    );
}
