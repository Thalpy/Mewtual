//! Genuine public warm factory on one typed mount. TEST custody is not native authorization.
use super::*;
use crate::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    PreparedV2CurrentLayoutStore, V2CurrentLayoutPolicy, V2MemberOutboxPolicy,
};
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

// The genuine whole-chain path still requires the audited settlement/promotion
// directory profile, which currently refuses Linux. Keep that support gap
// explicit; enabling the Linux profile must make this exact positive case run.
#[cfg(windows)]
#[path = "founder_warm_native_tests.rs"]
mod native;

const SECRET: &[u8] = b"fresh whole original founder driver";
type Driver = PreparedFreshV2TextFounder<MemNetwork, ChaCha20Rng>;

fn pool() -> CriticalCatalogReadPool {
    CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap()
}
fn mount(
    path: &Path,
    shared: &CriticalCatalogReadPool,
    rng: &mut ChaCha20Rng,
    member: bool,
    join: bool,
) -> V2CurrentLayoutStore {
    drop(
        ServerStore::open(path, SECRET, rng)
            .unwrap()
            .into_critical_namespace_store(SECRET, rng)
            .unwrap(),
    );
    let mut archive = ArchiveBootstrapCatalogPolicy::new(shared.clone(), 4 * MAX_SEED_FRAMED)
        .unwrap()
        .with_bootstrap_source_data_read();
    if join {
        archive = archive.with_checkpoint_join_execution(2, 10_000).unwrap();
    }
    let mut policy = V2CurrentLayoutPolicy::new(
        archive, CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * crate::store::critical_namespaces::archive_current_namespace_codec::MAX_DECLARATION_FRAME,
    ).unwrap();
    if member {
        policy = policy.with_member_outbox(V2MemberOutboxPolicy::new(
            CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap(), 256,
            256 * 1024 * 1024, 1 << 20, 1,
            crate::store::critical_namespaces::member_outbox_layout::codec::PERSISTENT_MINIMUM,
        ).unwrap()).unwrap();
    }
    let mut opening = PreparedV2CurrentLayoutStore::open(path, SECRET, rng, policy).unwrap();
    opening.retry(rng).unwrap();
    opening.into_current_layout_store().unwrap()
}
fn policy() -> FreshV2TextFounderPolicy {
    FreshV2TextFounderPolicy::new(
        ArchiveFounderPolicy::new(
            FounderSnapshotLimits::new(
                8 * 1024 * 1024 + 64 * 1024,
                8 * 1024 * 1024,
                8 * 1024 * 1024,
            )
            .unwrap(),
            3,
            codec::PERSISTENT_CONTENT_BYTES,
        )
        .unwrap(),
        GenesisSeedBudget::new(
            8 * 1024 * 1024 + 64 * 1024,
            8 * 1024 * 1024,
            8 * 1024 * 1024,
            1024 * 1024,
            16 * 1024 * 1024,
        )
        .unwrap(),
        V2TextHostPolicy::new(1024 * 1024 * 1024, 1024 * 1024).unwrap(),
    )
    .unwrap()
}
fn inputs(seed: u64) -> ArchiveFounderInputs<MemNetwork, ChaCha20Rng> {
    ArchiveFounderInputs::new(
        Hub::new().join(PeerId::from_u64(seed)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(ManualClock::new(10)),
    )
}
fn entries() -> usize {
    crate::store::critical_namespaces::archive_bootstrap::founder::SDK_ENTRIES.with(|n| n.get())
}
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("fresh refusal/retry consumed RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("fresh refusal/retry consumed RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("fresh refusal/retry consumed RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("fresh refusal/retry consumed RNG")
    }
}
impl CryptoRng for NeverRng {}

#[test]
fn fresh_factory_requires_concrete_join_and_member_handlers_before_creation() {
    for (member, join) in [(false, true), (true, false)] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x34f1);
        let layout = mount(directory.path(), &shared, &mut rng, member, join);
        let original = inputs(0x34f1);
        let signer = original.device.public_key_bytes();
        let before = entries();
        let returned = layout
            .prepare_fresh_v2_text_founder([1; 32], "missing handler", policy(), original)
            .unwrap_err()
            .into_parts()
            .0;
        assert_eq!(returned.device.public_key_bytes(), signer);
        assert_eq!(entries(), before);
    }
}

#[test]
fn fresh_driver_one_byte_short_returns_original_inputs_before_sdk_or_invocation() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x34f2);
    let layout = mount(directory.path(), &shared, &mut rng, true, true);
    let before = shared.charged_for_test();
    let invoice = Driver::invoice().unwrap();
    let pressure = shared
        .try_reserve(shared.capacity() - before - invoice + 1)
        .unwrap();
    let original = inputs(0x34f2);
    let signer = original.device.public_key_bytes();
    let before_entries = entries();
    let returned = layout
        .prepare_fresh_v2_text_founder([2; 32], "one byte short", policy(), original)
        .unwrap_err()
        .into_parts()
        .0;
    assert_eq!(returned.device.public_key_bytes(), signer);
    assert_eq!(entries(), before_entries);
    drop(pressure);
    assert_eq!(shared.charged_for_test(), before);
    assert!(matches!(
        layout
            .prepare_fresh_v2_text_founder([2; 32], "one byte short", policy(), returned)
            .unwrap(),
        FreshV2TextFounderBegin::Original(_)
    ));
}

#[test]
fn fresh_driver_foreign_mount_with_same_pool_cannot_consume_original_creation() {
    let original_dir = tempfile::tempdir().unwrap();
    let foreign_dir = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x34f3);
    let original = mount(original_dir.path(), &shared, &mut rng, true, true);
    let foreign = mount(foreign_dir.path(), &shared, &mut rng, true, true);
    let FreshV2TextFounderBegin::Original(mut driver) = original
        .prepare_fresh_v2_text_founder([3; 32], "original mount", policy(), inputs(0x34f3))
        .unwrap()
    else {
        panic!("actual original")
    };
    let before = entries();
    assert!(driver.retry(&foreign, &mut NeverRng).is_err());
    assert_eq!(entries(), before);
    assert!(driver.creation.is_some());
    assert!(!driver.is_ready_data());
    driver.close_and_drain().unwrap();
    assert!(driver.retry(&original, &mut NeverRng).is_err());
    assert_eq!(entries(), before);
}

#[test]
fn fresh_incompatible_seed_caps_refuse_before_spending_invocation_or_sdk() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x34f8);
    let layout = mount(directory.path(), &shared, &mut rng, true, true);
    let matching = policy();
    let too_small = GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024 - 1,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert!(FreshV2TextFounderPolicy::new(matching.founder, too_small, matching.host).is_err());
    // NEGATIVE same-crate policy construction proves the actual entry also
    // checks compatibility; no untrusted constructor is available publicly.
    let bad = FreshV2TextFounderPolicy {
        seed: too_small,
        ..matching
    };
    let original = inputs(0x34f8);
    let signer = original.device.public_key_bytes();
    let before = entries();
    let returned = layout
        .prepare_fresh_v2_text_founder([8; 32], "incompatible caps", bad, original)
        .unwrap_err()
        .into_parts()
        .0;
    assert_eq!(returned.device.public_key_bytes(), signer);
    assert_eq!(entries(), before);
    assert!(matches!(
        layout
            .prepare_fresh_v2_text_founder([8; 32], "incompatible caps", matching, returned)
            .unwrap(),
        FreshV2TextFounderBegin::Original(_)
    ));
    assert_eq!(
        entries(),
        before,
        "matching policy admitted the same untouched invocation only"
    );
}

#[cfg(windows)]
#[test]
fn fresh_whole_warm_factory_saves_without_replacing_runtime_or_creator() {
    use crate::store::MemberSaveJobReport;
    use catcoms_replication::chat::Body;
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x34f4);
    let layout = mount(directory.path(), &shared, &mut rng, true, true);
    let original = inputs(0x34f4);
    let signer = original.device.public_key_bytes();
    let before = entries();
    let FreshV2TextFounderBegin::Original(mut driver) = layout
        .prepare_fresh_v2_text_founder([4; 32], "whole warm founder", policy(), original)
        .unwrap()
    else {
        panic!("actual original")
    };
    assert!(driver.binding_data().is_none());
    driver.retry(&layout, &mut rng).unwrap();
    assert_eq!(entries(), before + 1);
    let binding = driver.binding_data().unwrap();
    assert_eq!(binding.signer_data().as_slice(), signer.as_slice());
    assert!(!binding.group_data().is_empty());
    let source_id = *binding.source_data();
    let creator = driver
        .source
        .as_ref()
        .unwrap()
        .original
        .creator
        .original
        .observer();
    assert!(!creator.is_cancelled());
    assert!(driver
        .source
        .as_ref()
        .unwrap()
        .original
        .creator
        .server
        .is_none());
    assert!(matches!(
        driver.source.as_ref().unwrap().phase,
        SourcePhase::RuntimeTransferred
    ));
    driver.retry(&layout, &mut NeverRng).unwrap();
    assert_eq!(driver.binding_data().unwrap().source_data(), &source_id);
    assert_eq!(entries(), before + 1);

    let (session, receiver) = tokio::sync::watch::channel(false);
    let negative = RequestCancellation::new(receiver, None);
    let body = Body::Create {
        text: "genuine warm ordinary member".into(),
        reply_to: None,
    };
    let request = || V2MemberSignRequest {
        token: [41; 32],
        channel: 7,
        body: &body,
        parents: &[],
        dependencies: &[],
    };
    assert_eq!(
        driver
            .prepare_member_save(&layout, request(), &negative, &mut rng)
            .unwrap(),
        V2MemberSaveOutcome::Ready
    );
    // TEST custody only: no native row, credential or UI-session claim.
    let job = driver
        .admit_member_save(
            &layout,
            [41; 32],
            MemberSaveNativeAdmission::new((), negative.clone()),
        )
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert!(matches!(
        driver
            .prepare_member_save(&layout, request(), &negative, &mut NeverRng)
            .unwrap(),
        V2MemberSaveOutcome::Saved { .. }
    ));
    driver.close_and_drain().unwrap();
    assert!(creator.is_cancelled());
    assert!(!driver.is_ready_data());
    assert!(driver.retry(&layout, &mut NeverRng).is_err());
    drop(session);
    drop(driver);
    drop(layout);
    assert_eq!(shared.charged_for_test(), 0);
}

/// Stop only AFTER the actual saved Current2 and checked warm Node exist. The
/// test flag grants no authority and does not fabricate a receipt or bridge.
#[cfg(windows)]
fn paused_current(seed: u64) -> (Driver, V2CurrentLayoutStore, ChaCha20Rng, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let layout = mount(directory.path(), &shared, &mut rng, true, true);
    let FreshV2TextFounderBegin::Original(mut driver) = layout
        .prepare_fresh_v2_text_founder(
            [51; 32],
            "actual paused warm Current",
            policy(),
            inputs(seed),
        )
        .unwrap()
    else {
        panic!("actual original")
    };
    driver.pause_after_current = true;
    assert!(driver.retry(&layout, &mut rng).is_err());
    // Unlike the deliberate outer pause this must PASS the actual Current gate.
    driver
        .current
        .as_mut()
        .unwrap()
        .retry(&layout, &mut NeverRng)
        .unwrap();
    let (old, current, combined) = driver
        .current
        .as_ref()
        .unwrap()
        .warm_negatives_for_test()
        .unwrap();
    assert!(!old.is_cancelled() && !current.is_cancelled() && !combined.is_cancelled());
    assert!(driver
        .current
        .as_ref()
        .unwrap()
        .warm_member_source_for_test()
        .is_ok());
    (driver, layout, rng, directory)
}

#[cfg(windows)]
#[test]
fn fresh_warm_bridge_original_revocation_leaves_current_live_but_closes_attachment() {
    let (mut driver, layout, _rng, _directory) = paused_current(0x34f5);
    let (old, current, combined) = driver
        .current
        .as_ref()
        .unwrap()
        .warm_negatives_for_test()
        .unwrap();
    driver.source.as_ref().unwrap().close();
    assert!(old.is_cancelled());
    assert!(
        !current.is_cancelled(),
        "independent Current authority really stayed live"
    );
    assert!(combined.is_cancelled());
    assert!(driver
        .current
        .as_ref()
        .unwrap()
        .warm_member_source_for_test()
        .is_err());
    driver.pause_after_current = false;
    assert!(driver.retry(&layout, &mut NeverRng).is_err());
    assert!(!driver.is_ready_data());
}

#[cfg(windows)]
#[test]
fn fresh_warm_bridge_current_revocation_leaves_original_live_but_closes_attachment() {
    let (mut driver, layout, _rng, _directory) = paused_current(0x34f6);
    let current_owner = driver.current.as_ref().unwrap();
    let (old, current, combined) = current_owner.warm_negatives_for_test().unwrap();
    current_owner.cancel_warm_current_for_test();
    assert!(
        !old.is_cancelled(),
        "actual original creator survived the row1/row2 transition"
    );
    assert!(current.is_cancelled());
    assert!(combined.is_cancelled());
    assert!(current_owner.warm_member_source_for_test().is_err());
    driver.pause_after_current = false;
    assert!(driver.retry(&layout, &mut NeverRng).is_err());
    assert!(!driver.is_ready_data());
}

#[cfg(windows)]
#[test]
fn fresh_warm_bridge_cached_identity_cannot_rebind_despite_live_same_labels() {
    let (mut driver, layout, _rng, _directory) = paused_current(0x34f7);
    let current_owner = driver.current.as_mut().unwrap();
    let (old, current, combined) = current_owner.warm_negatives_for_test().unwrap();
    // NEGATIVE corruption of bridge DATA after a genuine successful control.
    // It changes no labels or live owner and cannot issue a replacement Current.
    current_owner.corrupt_warm_identity_for_test();
    assert!(!old.is_cancelled() && !current.is_cancelled() && !combined.is_cancelled());
    assert!(current_owner.retry(&layout, &mut NeverRng).is_err());
    driver.pause_after_current = false;
    assert!(driver.retry(&layout, &mut NeverRng).is_err());
    assert!(!driver.is_ready_data());
}
