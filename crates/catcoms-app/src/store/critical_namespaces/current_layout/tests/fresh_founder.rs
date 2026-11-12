//! Start the real founder producer AFTER one configured Current mount is complete.
//! Older warm fixtures save the source before opening Current on a replacement mount.
//! This pins a prerequisite of the closed native bootstrap driver, not native authority.
use super::*;
use crate::store::{ArchiveFounderBegin, ArchiveFounderInputs, ArchiveFounderPolicy};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use catcoms_sync::owner_v2::FounderSnapshotLimits;

#[test]
fn source_saved_after_current_join_member_mount_stays_on_that_original_mount() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x33f1);
    initialize(directory.path(), &mut rng);
    let mut opening = PreparedV2CurrentLayoutStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        member_outbox::member_execution_policy(&shared),
    )
    .unwrap();
    opening.retry(&mut rng).unwrap();
    let layout = opening.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    store.validate_mounted_member_outbox_layout().unwrap();
    let mount = store.registry_mount();
    let original_mount = store.chat_mount.observer();
    let catalog = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    assert!(layout
        .read_saved_bootstrap_source_data(b"not-yet-created", [0; 32])
        .is_err());

    // This is the actual one-shot producer and retained device, not a supplied
    // snapshot, permission bit or second Store. The native adapter will need a
    // closed factory around these same stages rather than this private test loan.
    let device = MlsDevice::generate().unwrap();
    let signer = device.public_key_bytes().try_into().unwrap();
    let limits = FounderSnapshotLimits::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
    )
    .unwrap();
    let policy = ArchiveFounderPolicy::new(
        limits,
        3,
        archive_bootstrap::founder_codec::PERSISTENT_CONTENT_BYTES,
    )
    .unwrap();
    let inputs = ArchiveFounderInputs::new(
        Hub::new().join(PeerId::from_u64(0x33f1)),
        device,
        ChaCha20Rng::seed_from_u64(0x33f1),
        Box::new(ManualClock::new(10)),
    );
    let mut creator = match store
        .prepare_archive_founder([0xf1; 32], "one mounted founder", policy, inputs)
        .unwrap()
    {
        ArchiveFounderBegin::Original(creator) => creator,
        _ => panic!("this invocation must own the actual original"),
    };
    creator.advance(store, &mut rng).unwrap();
    let group = creator.group_data().unwrap().to_vec();
    let mut plan = creator.into_ram_plan(store).unwrap();
    plan.save_founding_intent(store, &mut rng).unwrap();
    let mut source = plan.into_source_preparation(store).unwrap();
    source.save_source_terminal(store, &mut rng).unwrap();
    // The retry reaches the producer's exact durable terminal. Any new sealing
    // or execution nonce would panic, so a successful read alone is insufficient.
    source.save_source_terminal(store, &mut NeverRng).unwrap();
    let read = layout
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    read.validate_original(store).unwrap();
    let spec = read.data().view_data().context.spec_data();
    assert_eq!(spec.group, group.as_slice());
    assert_eq!(spec.signer, signer);
    assert!(Arc::ptr_eq(&mount, &store.registry_mount()));
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        catalog,
        "source creation must preserve the configured namespace catalog bytes"
    );
    store.validate_mounted_member_outbox_layout().unwrap();

    // No physical reopen occurred above. Retiring this mount makes its paid
    // historical read unusable; merely retaining DATA cannot keep it current.
    assert!(!original_mount.is_cancelled());
    drop(source);
    drop(layout);
    assert!(
        original_mount.is_cancelled(),
        "retirement closes the actual original before any replacement mount opens"
    );
    let mut opening = PreparedV2CurrentLayoutStore::open(
        directory.path(),
        SECRET,
        &mut NeverRng,
        member_outbox::member_execution_policy(&shared),
    )
    .unwrap();
    opening.retry(&mut NeverRng).unwrap();
    let replacement = opening.into_current_layout_store().unwrap();
    assert!(read
        .validate_original(replacement.checked_store().unwrap())
        .is_err());
    replacement
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap()
        .validate_original(replacement.checked_store().unwrap())
        .unwrap();
}
