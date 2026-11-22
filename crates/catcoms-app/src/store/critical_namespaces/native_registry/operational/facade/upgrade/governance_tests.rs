//! Real-file constructor tests. These create no signer, Source, SDK or native
//! command credential: all assertions concern the original physical owner.
use super::*;
use crate::store::critical_namespaces::{
    governance_layout as governance,
    native_registry::preparation::tests::{current_policy, SECRET},
    pending_join_codec as catalog, Slot, ARCHIVE_BOOTSTRAP_ROOT,
};
use crate::store::{
    NativeIndexPolicy, NativeIndexUpgrade, NativeRegistryInstallation, NativeRegistryPolicy,
    NativeTextGovernedStorage, NativeTextJoinedStorage,
};
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use std::{fs, path::Path, sync::atomic::Ordering};

struct NeverRng;
impl CryptoRng for NeverRng {}
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("retained governance drew randomness")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("retained governance drew randomness")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("retained governance drew randomness")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("retained governance drew randomness")
    }
}

fn original(path: &Path, rng: &mut ChaCha20Rng) -> NativeTextVaultUpgrade {
    let first_policy = NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap();
    let mut first = NativeRegistryInstallation::prepare_fresh(
        path,
        SECRET,
        rng,
        current_policy(),
        first_policy,
    )
    .unwrap();
    first.retry().unwrap();
    let second_policy = NativeIndexPolicy::new(
        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
        NativeIndexPolicy::required_persistent_capacity(),
    )
    .unwrap();
    let mut second =
        NativeIndexUpgrade::prepare(first.into_native_registry().unwrap(), second_policy, rng)
            .unwrap();
    second.retry().unwrap();
    let ui = NativeUiInvocationBudget::new(
        CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap(),
    )
    .unwrap();
    let mut original = NativeTextVaultUpgrade::prepare(
        second.into_index_store().unwrap(),
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicBool::new(false)),
        ui,
    )
    .unwrap();
    original.retry(rng).unwrap();
    original
}

fn actual(plan: &NativeTextGovernanceUpgrade) -> &crate::store::ServerStore {
    let Some(Phase::Selected3(index)) = plan.original.as_ref().unwrap().phase.as_ref() else {
        panic!("whole selected3 receiver");
    };
    index.constructor_store_for_test().unwrap()
}
fn fault(plan: &NativeTextGovernanceUpgrade, point: u8) {
    actual(plan)
        .critical_namespace_fault
        .store(point, Ordering::SeqCst);
}
fn leaves(path: &Path) -> [Vec<u8>; 3] {
    [
        "native-registry.bin",
        "native-registry-layout.bin",
        "member-outbox-layout.bin",
    ]
    .map(|name| fs::read(path.join(ARCHIVE_BOOTSTRAP_ROOT).join(name)).unwrap())
}

#[test]
fn native_governance_same_owner_warm_constructor_and_cold_metadata_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc401);
    let original = original(dir.path(), &mut rng);
    let before = leaves(dir.path());
    let plan = original.prepare_governance().unwrap();
    assert!(!plan.receiver.is_complete());
    let mut plan = NativeTextGovernanceUpgrade::into_vault(plan).unwrap_err();
    plan.retry(&mut rng).unwrap();
    let store = actual(&plan);
    let raw = fs::read(dir.path().join(Slot::Current.name())).unwrap();
    let plain = crate::store::unseal(
        &store.keys.db_key().unwrap(),
        &crate::store::unframe(&raw).unwrap(),
    )
    .unwrap();
    let parsed = catalog::parse_governed_native_registry_layout(
        &plain,
        &store.chat_vault_domain().unwrap(),
        true,
    )
    .unwrap();
    assert!(parsed.governance_payload.is_some());
    assert!(parsed.member_outbox_payload.is_some());
    assert!(parsed.native_registry_payload.is_some());
    assert_eq!(before, leaves(dir.path()));
    let vault = plan.into_vault().unwrap();
    assert_eq!(vault.row_count_data().unwrap(), 0);
    assert!(vault
        .lookup_creation_data([1; 32], [2; 32])
        .unwrap()
        .is_none());
    drop(vault);
    let cold = NativeTextGovernedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
        .unwrap();
    assert_eq!(cold.row_count_data().unwrap(), 0);
    assert_eq!(before, leaves(dir.path()));
}

#[test]
fn governed_joined_pin_catalog_inserts_only_six_over_actual_governed_native() {
    use crate::store::critical_namespaces::joined_pin::{
        declaration::{AuthenticatedSeed, DeclarationLeaves, PreparedDeclaration},
        namespace,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc409);
    let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
    plan.retry(&mut rng).unwrap();
    let store = actual(&plan);
    let selected = fs::read(dir.path().join(Slot::Current.name())).unwrap();
    let plain = crate::store::unseal(
        &store.keys.db_key().unwrap(),
        &crate::store::unframe(&selected).unwrap(),
    )
    .unwrap();
    let domain = store.chat_vault_domain().unwrap();
    let prior = catalog::parse_governed_native_registry_layout(&plain, &domain, true).unwrap();
    assert!(catalog::parse_joined_native_registry_layout(&plain, &domain, true).is_err());
    let successor = namespace::prepare_declaration(&plain, &domain).unwrap();
    namespace::verify_declaration(&plain, &successor, &domain).unwrap();
    assert!(catalog::parse_governed_native_registry_layout(&successor, &domain, true).is_err());
    assert!(namespace::prepare_declaration(&successor, &domain).is_err());
    let joined = catalog::parse_joined_native_registry_layout(&successor, &domain, true).unwrap();
    assert!(joined.joined_pin_payload.is_some());
    assert_eq!(joined.generation, prior.generation + 1);
    assert_eq!(joined.parent, Some(prior.digest));
    assert_eq!(joined.governance_payload, prior.governance_payload);
    assert_eq!(joined.member_outbox_payload, prior.member_outbox_payload);
    assert_eq!(
        joined.native_registry_payload,
        prior.native_registry_payload
    );
    assert!(catalog::successor(&prior.summary([0; 32]), &joined.summary([1; 32])).is_err());
    let key = store.keys.db_key().unwrap();
    let layout = prior.current_layout.unwrap();
    let governance_marker = fs::read(
        dir.path()
            .join(ARCHIVE_BOOTSTRAP_ROOT)
            .join(governance::GOVERNANCE_MARKER),
    )
    .unwrap();
    let [native_index, native_marker, outbox_marker] = leaves(dir.path());
    let original_leaves = DeclarationLeaves::from_frames(
        &governance_marker,
        &outbox_marker,
        &native_marker,
        &native_index,
    )
    .unwrap();
    let prepared =
        PreparedDeclaration::prepare(&key, &selected, layout, original_leaves, &mut rng).unwrap();
    let seed = AuthenticatedSeed::open(&key, &prepared.seed).unwrap();
    let reopened = seed.validate(&key, layout, original_leaves).unwrap();
    assert_eq!(reopened.original, selected);
    assert_eq!(reopened.candidate, prepared.candidate.as_slice());
    assert_eq!(reopened.initial_index, prepared.initial_index.as_slice());
    let reopened_plain =
        crate::store::unseal(&key, &crate::store::unframe(reopened.candidate).unwrap()).unwrap();
    assert_eq!(reopened_plain, successor.as_slice());
    let mut changed_index = native_index.clone();
    changed_index[0] ^= 1;
    let changed_leaves = DeclarationLeaves::from_frames(
        &governance_marker,
        &outbox_marker,
        &native_marker,
        &changed_index,
    )
    .unwrap();
    assert!(seed.validate(&key, layout, changed_leaves).is_err());
    let mut other_layout = layout;
    other_layout.root[0] ^= 1;
    assert!(seed.validate(&key, other_layout, original_leaves).is_err());
    assert!(AuthenticatedSeed::open(&key, &prepared.seed[..prepared.seed.len() - 1]).is_err());
    assert!(PreparedDeclaration::prepare(
        &key,
        &prepared.candidate,
        layout,
        original_leaves,
        &mut NeverRng,
    )
    .is_err());
    assert_eq!(
        fs::read(dir.path().join(Slot::Current.name())).unwrap(),
        selected
    );
}

#[test]
fn governed_joined_pin_stage_holds_its_first_claim_and_older_openers_preserve_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc40a);
    let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
    plan.retry(&mut rng).unwrap();
    let store = actual(&plan);
    let current = dir.path().join(Slot::Current.name());
    let claim = dir.path().join(governance::GOVERNANCE_SEED);
    let stage = dir.path().join(Slot::Stage.name());
    let selected = fs::read(&current).unwrap();
    let mut declaration = store.prepare_joined_pin_declaration(&mut rng).unwrap();
    assert!(!claim.exists() && !stage.exists());
    declaration.stage(store).unwrap();
    let first_claim = fs::read(&claim).unwrap();
    let first_stage = fs::read(&stage).unwrap();
    declaration.stage(store).unwrap();
    assert_eq!(fs::read(&claim).unwrap(), first_claim);
    assert_eq!(fs::read(&stage).unwrap(), first_stage);
    assert_eq!(fs::read(&current).unwrap(), selected);

    let original_stage = dir.path().join("held-joined-stage");
    fs::rename(&stage, &original_stage).unwrap();
    fs::write(&stage, &first_stage).unwrap();
    assert!(declaration.stage(store).is_err());
    assert_eq!(fs::read(&stage).unwrap(), first_stage);
    fs::remove_file(&stage).unwrap();
    fs::rename(&original_stage, &stage).unwrap();
    declaration.stage(store).unwrap();
    drop(declaration);
    drop(plan);
    assert!(
        NativeTextGovernedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
            .is_err()
    );
    assert_eq!(fs::read(&claim).unwrap(), first_claim);
    assert_eq!(fs::read(&stage).unwrap(), first_stage);
    assert_eq!(fs::read(&current).unwrap(), selected);

    // A positive baseline confirms the refusal came from declaration custody.
    fs::remove_file(&claim).unwrap();
    fs::remove_file(&stage).unwrap();
    fs::remove_file(
        dir.path()
            .join(crate::store::persistence::governance_layout::joined::INDEX_NAME),
    )
    .unwrap();
    let old = NativeTextGovernedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
        .unwrap();
    assert_eq!(old.row_count_data().unwrap(), 0);
}

#[test]
fn governed_joined_pin_cold_open_recovers_the_original_catalog_after_each_covered_cut() {
    use crate::store::persistence::governance_layout::joined::INDEX_NAME;
    use catcoms_replication::chat::join_bindings::GenesisAnchorBindingData;
    let absent_pin = GenesisAnchorBindingData {
        group: vec![176; 16],
        founder: [177; 32],
        profile: [178; 32],
        admission: [179; 32],
        tenure: [180; 32],
        header: [181; 32],
        state: [182; 32],
    };
    for cut in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc450 + cut);
        let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
        plan.retry(&mut rng).unwrap();
        let store = actual(&plan);
        let current = dir.path().join(Slot::Current.name());
        let claim = dir.path().join(governance::GOVERNANCE_SEED);
        let stage = dir.path().join(Slot::Stage.name());
        let index_path = dir.path().join(INDEX_NAME);
        let key = store.keys.db_key().unwrap();
        let domain = store.chat_vault_domain().unwrap();
        drop(plan);
        let mut prepared =
            NativeTextGovernedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
                .unwrap();
        prepared.stage_joined_pin_layout(&mut rng).unwrap();
        prepared.stage_joined_pin_layout(&mut NeverRng).unwrap();
        let expected = fs::read(&stage).unwrap();
        let expected_index = fs::read(&index_path).unwrap();
        let native = leaves(dir.path());
        drop(prepared);

        if cut == 0 {
            let held_stage = dir.path().join("original-stage-at-selection");
            let capture = stage.clone();
            let moved = held_stage.clone();
            let bytes = expected.clone();
            governance::set_stage_selection_hook_for_test(Box::new(move || {
                fs::rename(&capture, &moved).unwrap();
                fs::write(&capture, bytes).unwrap();
            }));
            let old_current = fs::read(&current).unwrap();
            let old_claim = fs::read(&claim).unwrap();
            assert!(NativeTextJoinedStorage::open(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy()
            )
            .is_err());
            assert_eq!(fs::read(&current).unwrap(), old_current);
            assert_eq!(fs::read(&claim).unwrap(), old_claim);
            assert_eq!(fs::read(&stage).unwrap(), expected);
            fs::remove_file(&stage).unwrap();
            fs::rename(&held_stage, &stage).unwrap();
        }
        if cut == 1 {
            // This models interruption during stage completion.
            fs::write(&stage, &expected[..expected.len() / 2]).unwrap();
            fs::write(&index_path, &expected_index[..expected_index.len() / 2]).unwrap();
        }
        if cut == 2 {
            // This models a lost return after rename and before seed retirement.
            fs::rename(&stage, &current).unwrap();
            fs::remove_file(&index_path).unwrap();
            let seed = fs::read(&claim).unwrap();
            assert!(NativeTextJoinedStorage::open(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy()
            )
            .is_err());
            assert_eq!(fs::read(&claim).unwrap(), seed);
            assert_eq!(fs::read(&current).unwrap(), expected);
            fs::write(&index_path, &expected_index).unwrap();
        }
        let cold =
            NativeTextJoinedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
                .unwrap_or_else(|error| panic!("joined declaration cut {cut}: {error}"));
        assert_eq!(cold.row_count_data().unwrap(), 0);
        assert_eq!(fs::read(&current).unwrap(), expected);
        assert_eq!(fs::read(&index_path).unwrap(), expected_index);
        assert_eq!(native, leaves(dir.path()));
        assert!(!claim.exists() && !stage.exists());
        let plain = crate::store::unseal(&key, &crate::store::unframe(&expected).unwrap()).unwrap();
        catalog::parse_joined_native_registry_layout(&plain, &domain, true).unwrap();
        assert!(cold.joined_pin_context_data(&absent_pin).unwrap().is_none());
        if cut == 0 {
            let displaced_index = dir.path().join("held-joined-pin-index");
            fs::rename(&index_path, &displaced_index).unwrap();
            fs::write(&index_path, &expected_index).unwrap();
            assert!(cold.joined_pin_context_data(&absent_pin).is_err());
            fs::remove_file(&index_path).unwrap();
            fs::rename(&displaced_index, &index_path).unwrap();
            assert!(cold.joined_pin_context_data(&absent_pin).unwrap().is_none());
        }
        drop(cold);
        assert!(NativeTextGovernedStorage::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy()
        )
        .is_err());
        let cold =
            NativeTextJoinedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
                .unwrap();
        assert_eq!(cold.row_count_data().unwrap(), 0);
    }
}

#[test]
fn governed_joined_pin_pending_replays_the_same_private_request_and_index_insertion() {
    use catcoms_crypto::DeviceKeypair;
    use catcoms_replication::chat::join_bindings::{
        GenesisAnchorBindingData, InviteBindingSpecData, SignedInviteBindingData,
    };
    use catcoms_sync::{DirectJoinLimits, PreparedV2JoinRequest};
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc498);
    let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
    plan.retry(&mut rng).unwrap();
    drop(plan);
    let mut original =
        NativeTextGovernedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
            .unwrap();
    original.stage_joined_pin_layout(&mut rng).unwrap();
    drop(original);
    let mut joined =
        NativeTextJoinedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy()).unwrap();
    assert!(joined.pending_join_data().unwrap().is_none());

    let founder = DeviceKeypair::from_seed(&[144; 32]);
    let anchor = GenesisAnchorBindingData {
        group: vec![145; 16],
        founder: founder.verifying_key().to_bytes(),
        profile: [146; 32],
        admission: [147; 32],
        tenure: [148; 32],
        header: [149; 32],
        state: [150; 32],
    };
    let invite = SignedInviteBindingData::sign_using(
        InviteBindingSpecData {
            anchor: anchor.clone(),
            inviter: anchor.founder,
            nonce: [151; 16],
            expires_at_ms: 800,
            policy: [152; 32],
            public_routing_plan: [153; 32],
        },
        |bytes| Ok(founder.sign(bytes)),
    )
    .unwrap();
    let prepare = || {
        PreparedV2JoinRequest::prepare(
            catcoms_mls::MlsDevice::generate().unwrap(),
            &anchor,
            &invite,
            [154; 32],
            700,
            DirectJoinLimits::new(512 * 1024, 2 * 1024 * 1024, 256 * 1024).unwrap(),
        )
        .unwrap()
    };
    let request = prepare();
    let expected = *blake3::hash(request.request_bytes_data()).as_bytes();
    joined.stage_first_join(request, &invite, &mut rng).unwrap();
    joined.retry_first_join().unwrap();
    assert!(joined.pending_join_data().is_err());
    assert!(joined
        .stage_first_join(prepare(), &invite, &mut NeverRng)
        .is_err());
    drop(joined);

    let joined =
        NativeTextJoinedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy()).unwrap();
    let (actual, context) = joined.pending_join_data().unwrap().unwrap();
    assert_eq!(actual, expected);
    assert_ne!(context.original_pending, [0; 32]);
    assert!(joined
        .joined_pin_context_data(&anchor)
        .unwrap()
        .is_some_and(|pin| pin == context));
    drop(joined);
    let joined =
        NativeTextJoinedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy()).unwrap();
    assert!(joined
        .pending_join_data()
        .unwrap()
        .is_some_and(|actual| actual == (expected, context)));
}

#[test]
fn native_governance_warm_faults_retry_exact_outputs_without_randomness() {
    for point in [240, 229, 231, 232, 233, 234, 235, 236, 237, 238, 239] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc410 + u64::from(point));
        let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
        let before = leaves(dir.path());
        fault(&plan, point);
        assert!(plan.retry(&mut rng).is_err(), "cut {point}");
        fault(&plan, 0);
        plan.retry(&mut NeverRng)
            .unwrap_or_else(|e| panic!("retry cut {point}: {e}"));
        assert_eq!(
            before,
            leaves(dir.path()),
            "native leaves changed at {point}"
        );
        let vault = plan.into_vault().unwrap();
        assert_eq!(vault.row_count_data().unwrap(), 0);
        drop(vault);
        let cold =
            NativeTextGovernedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
                .unwrap_or_else(|e| panic!("cold after {point}: {e}"));
        assert_eq!(cold.row_count_data().unwrap(), 0);
    }
}

#[test]
fn native_governance_cold_recovery_distinguishes_first_claim_from_complete_seed() {
    for point in [240, 229, 231, 232, 233, 234, 235, 236, 237, 238, 239] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc510 + u64::from(point));
        let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
        let before = leaves(dir.path());
        fault(&plan, point);
        assert!(plan.retry(&mut rng).is_err(), "cold cut {point}");
        drop(plan);
        // A first claim has no authenticated candidate; abort it and declare
        // fresh. A complete seed must replay its exact saved bytes and nonces.
        let cold = if point == 240 || point == 229 || point == 231 {
            NativeTextGovernedStorage::open(dir.path(), SECRET, &mut rng, current_policy())
        } else {
            NativeTextGovernedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
        }
        .unwrap_or_else(|e| panic!("cold cut {point}: {e}"));
        assert_eq!(cold.row_count_data().unwrap(), 0);
        assert_eq!(before, leaves(dir.path()));
        assert!(!dir.path().join(governance::GOVERNANCE_SEED).exists());
        assert!(!dir.path().join(Slot::Stage.name()).exists());
    }
}

#[test]
fn native_governance_warm_rejects_equal_native_file_replacement() {
    for name in ["native-registry.bin", "native-registry-layout.bin"] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc601);
        let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
        fault(&plan, 232); // Exact complete seed, before marker/output selection.
        assert!(plan.retry(&mut rng).is_err());
        let file = dir.path().join(ARCHIVE_BOOTSTRAP_ROOT).join(name);
        let bytes = fs::read(&file).unwrap();
        let displaced = tempfile::tempdir_in(dir.path().parent().unwrap()).unwrap();
        fs::rename(&file, displaced.path().join("retained-old")).unwrap();
        fs::write(&file, &bytes).unwrap();
        fault(&plan, 0);
        assert!(
            plan.retry(&mut NeverRng).is_err(),
            "equal replacement: {name}"
        );
        assert_eq!(fs::read(&file).unwrap(), bytes);
        assert!(dir.path().join(governance::GOVERNANCE_SEED).exists());
    }
}

#[test]
fn native_governance_final_edges_reject_late_equal_replacement() {
    for marker_edge in [false, true] {
        for name in [
            "native-registry.bin",
            "native-registry-layout.bin",
            "current",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let displaced = tempfile::tempdir_in(dir.path().parent().unwrap()).unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0xc801);
            let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
            let file = if name == "current" {
                dir.path().join(Slot::Current.name())
            } else {
                dir.path().join(ARCHIVE_BOOTSTRAP_ROOT).join(name)
            };
            let observed = Arc::new(AtomicBool::new(false));
            let called = observed.clone();
            let moved = displaced.path().join("retained");
            let hook = Box::new(move || {
                let bytes = fs::read(&file).unwrap();
                fs::rename(&file, moved).unwrap();
                fs::write(&file, bytes).unwrap();
                called.store(true, Ordering::SeqCst);
            });
            if marker_edge {
                governance::set_marker_verification_hook_for_test(hook);
            } else {
                governance::set_stage_selection_hook_for_test(hook);
            }
            assert!(
                plan.retry(&mut rng).is_err(),
                "late edge {marker_edge}: {name}"
            );
            assert!(observed.load(Ordering::SeqCst));
            assert!(!plan.receiver.is_complete());
            assert!(plan.into_vault().is_err());
        }
    }
}

#[test]
fn native_governance_torn_cleanup_rejects_in_place_evidence_changes() {
    for rewrite_seed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc901);
        let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
        fault(&plan, 229); // Known first claim and empty fence; no seed AEAD yet.
        assert!(plan.retry(&mut rng).is_err());
        let key = zeroize::Zeroizing::new(blake3::derive_key(
            "catcoms/private-chat-v2-native-governance-declaration-key/v2",
            &actual(&plan).keys.db_key().unwrap(),
        ));
        let future = crate::store::frame(&catcoms_crypto::seal(&key, b"future", &mut rng).unwrap());
        drop(plan);
        let changed = dir.path().join(if rewrite_seed {
            governance::GOVERNANCE_SEED
        } else {
            Slot::Stage.name()
        });
        let bytes = if rewrite_seed { future } else { vec![1, 2, 3] };
        let replacement = bytes.clone();
        let changed_hook = changed.clone();
        let observed = Arc::new(AtomicBool::new(false));
        let called = observed.clone();
        governance::set_native_torn_hook_for_test(Box::new(move |_| {
            // Same file identity, changed contents. No rename or new child can
            // accidentally satisfy this regression's refusal.
            fs::write(changed_hook, replacement).unwrap();
            called.store(true, Ordering::SeqCst);
        }));
        assert!(NativeTextGovernedStorage::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
        )
        .is_err());
        assert!(observed.load(Ordering::SeqCst));
        assert_eq!(fs::read(changed).unwrap(), bytes);
        assert!(dir.path().join(governance::GOVERNANCE_SEED).exists());
        assert!(dir.path().join(Slot::Stage.name()).exists());
    }
}

#[test]
fn native_governance_drop_closes_original_before_receiver_and_keeps_outer_fee() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xca01);
    let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
    let ui = plan
        .original
        .as_ref()
        .unwrap()
        .ui
        .as_ref()
        .unwrap()
        .observe_mount();
    let Some(Phase::Selected3(index)) = plan.original.as_ref().unwrap().phase.as_ref() else {
        panic!("original selected3");
    };
    let handler = index.handler_for_test();
    let refunded = Arc::new(AtomicBool::new(false));
    plan._metadata.observe_drop_for_test(refunded.clone());
    fault(&plan, 232);
    assert!(plan.retry(&mut rng).is_err());
    let observed = Arc::new(AtomicBool::new(false));
    let called = observed.clone();
    let paid = refunded.clone();
    governance::set_declaration_tail_hook_for_test(Box::new(move || {
        assert!(
            !ui.is_open(),
            "original UI must close before declaration disposal"
        );
        assert!(
            !handler.is_open_cached(),
            "original native admission must close first"
        );
        assert!(
            !paid.load(Ordering::SeqCst),
            "receiver box still has its own payment"
        );
        called.store(true, Ordering::SeqCst);
    }));
    drop(plan);
    assert!(observed.load(Ordering::SeqCst));
    assert!(refunded.load(Ordering::SeqCst));
}

#[test]
fn native_governance_cached_negative_follows_actual_mounted_owner_without_locks() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xcb01);
    let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
    plan.retry(&mut rng).unwrap();
    let store = actual(&plan);
    let negative = store.capture_governance_cached_negative().unwrap();
    let pool = store
        .governance_layout_policy
        .as_ref()
        .unwrap()
        .current
        .archive
        .pending
        .pool
        .clone();
    pool.with_counter_locked_for_test(|| assert!(negative.is_live_cached()));
    let mounted = store.governance_layout.lock().unwrap().take().unwrap();
    let charged = pool.charged_for_test();
    drop(mounted);
    pool.with_counter_locked_for_test(|| assert!(!negative.is_live_cached()));
    // The closed metadata's exact reservation remains charged until every
    // cached tail drops. Revocation itself requires no IO/account lock.
    assert_eq!(charged, pool.charged_for_test());
    assert!(plan.into_vault().is_err());
}

#[test]
fn native_governance_proof_borrows_held_root_and_refuses_missing_mounted_owner() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xcb04);
    let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
    plan.retry(&mut rng).unwrap();
    let store = actual(&plan);
    let root = store.archive_bootstrap_root.lock().unwrap();
    let root = root.as_ref().unwrap();
    store
        .validate_mounted_governance_layout_under_archive_root(root)
        .unwrap();
    let mounted = store.governance_layout.lock().unwrap().take().unwrap();
    assert!(store
        .validate_mounted_governance_layout_under_archive_root(root)
        .is_err());
    *store.governance_layout.lock().unwrap() = Some(mounted);
    store
        .validate_mounted_governance_layout_under_archive_root(root)
        .unwrap();
}

#[test]
fn native_governance_steady_parser_rejects_original_catalog_without_namespace4() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xcc01);
    let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
    let old = fs::read(dir.path().join(Slot::Current.name())).unwrap();
    plan.retry(&mut rng).unwrap();
    let store = actual(&plan);
    let old_plain = crate::store::unseal(
        &store.keys.db_key().unwrap(),
        &crate::store::unframe(&old).unwrap(),
    )
    .unwrap();
    assert!(store.parse_configured_join_catalog(&old_plain).is_err());
    let current_handler = store
        .current_layout_policy
        .as_ref()
        .unwrap()
        .native_registry
        .as_ref()
        .unwrap();
    let governance_handler = store
        .governance_layout_policy
        .as_ref()
        .unwrap()
        .current
        .native_registry
        .as_ref()
        .unwrap();
    assert!(current_handler.is_governed());
    assert!(governance_handler.is_governed());
    assert!(current_handler
        .parse_layout(&old_plain, &store.chat_vault_domain().unwrap(), true)
        .is_err());
    assert!(governance_handler
        .parse_layout(&old_plain, &store.chat_vault_domain().unwrap(), true)
        .is_err());
}

#[test]
fn native_governance_consuming_warm_handoff_retains_evidence_through_last_reads() {
    let dir = tempfile::tempdir().unwrap();
    let displaced = tempfile::tempdir_in(dir.path().parent().unwrap()).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xcd01);
    let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
    plan.retry(&mut rng).unwrap();
    let file = dir
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join("native-registry.bin");
    let observed = Arc::new(AtomicBool::new(false));
    let called = observed.clone();
    governance::set_marker_verification_hook_for_test(Box::new(move || {
        let bytes = fs::read(&file).unwrap();
        fs::rename(&file, displaced.path().join("original")).unwrap();
        fs::write(file, bytes).unwrap();
        called.store(true, Ordering::SeqCst);
    }));
    assert!(plan.into_vault().is_err());
    assert!(observed.load(Ordering::SeqCst));
}

#[test]
fn native_governance_cold_handoff_retains_observed_or_declared_original_leaves() {
    for already_governed in [false, true] {
        for name in ["native-registry.bin", "native-registry-layout.bin"] {
            let dir = tempfile::tempdir().unwrap();
            let displaced = tempfile::tempdir_in(dir.path().parent().unwrap()).unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0xce01);
            let original = original(dir.path(), &mut rng);
            if already_governed {
                let mut plan = original.prepare_governance().unwrap();
                plan.retry(&mut rng).unwrap();
                drop(plan.into_vault().unwrap());
            } else {
                drop(original);
            }
            let file = dir.path().join(ARCHIVE_BOOTSTRAP_ROOT).join(name);
            let observed = Arc::new(AtomicBool::new(false));
            let called = observed.clone();
            owning::at_edge(
                owning::Edge::BeforeColdExposure,
                Box::new(move || {
                    let bytes = fs::read(&file).unwrap();
                    fs::rename(&file, displaced.path().join("original")).unwrap();
                    fs::write(file, bytes).unwrap();
                    called.store(true, Ordering::SeqCst);
                }),
            );
            let result = if already_governed {
                NativeTextGovernedStorage::open(dir.path(), SECRET, &mut NeverRng, current_policy())
            } else {
                NativeTextGovernedStorage::open(dir.path(), SECRET, &mut rng, current_policy())
            };
            assert!(
                result.is_err(),
                "last cold handoff {already_governed}: {name}"
            );
            assert!(observed.load(Ordering::SeqCst));
        }
    }
}

#[test]
fn native_governance_cold_cleanup_cuts_repeat_fence_barrier_before_claim_retirement() {
    for point in [241, 242, 243, 244] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xcf01);
        let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
        fault(&plan, 229);
        assert!(plan.retry(&mut rng).is_err());
        let current = fs::read(dir.path().join(Slot::Current.name())).unwrap();
        let before = leaves(dir.path());
        drop(plan);
        let observed = Arc::new(AtomicBool::new(false));
        let called = observed.clone();
        governance::set_native_torn_hook_for_test(Box::new(move |store| {
            store
                .critical_namespace_fault
                .store(point, Ordering::SeqCst);
            called.store(true, Ordering::SeqCst);
        }));
        assert!(NativeTextGovernedStorage::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
        )
        .is_err());
        assert!(observed.load(Ordering::SeqCst), "cleanup cut {point}");
        assert!(!dir.path().join(Slot::Stage.name()).exists());
        assert_eq!(
            dir.path().join(governance::GOVERNANCE_SEED).exists(),
            point < 243
        );
        assert_eq!(
            fs::read(dir.path().join(Slot::Current.name())).unwrap(),
            current
        );
        assert_eq!(before, leaves(dir.path()));
        if point == 241 {
            // Stage is already absent, but the next attempt must still reach
            // the covering barrier before it can retire the original claim.
            governance::set_native_torn_hook_for_test(Box::new(|store| {
                store.critical_namespace_fault.store(242, Ordering::SeqCst);
            }));
            assert!(NativeTextGovernedStorage::open(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy(),
            )
            .is_err());
            assert!(dir.path().join(governance::GOVERNANCE_SEED).exists());
        }
        let cold = NativeTextGovernedStorage::open(dir.path(), SECRET, &mut rng, current_policy())
            .unwrap_or_else(|e| panic!("cleanup retry {point}: {e}"));
        assert_eq!(cold.row_count_data().unwrap(), 0);
        assert_eq!(before, leaves(dir.path()));
    }
}

#[test]
fn native_governance_authenticated_invalid_seed_is_preserved() {
    for invalid in [b"".as_slice(), b"future native governance v99".as_slice()] {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xc701);
        let mut plan = original(dir.path(), &mut rng).prepare_governance().unwrap();
        fault(&plan, 232);
        assert!(plan.retry(&mut rng).is_err());
        let key = zeroize::Zeroizing::new(blake3::derive_key(
            "catcoms/private-chat-v2-native-governance-declaration-key/v2",
            &actual(&plan).keys.db_key().unwrap(),
        ));
        let invalid = crate::store::frame(&catcoms_crypto::seal(&key, invalid, &mut rng).unwrap());
        drop(plan);
        let seed = dir.path().join(governance::GOVERNANCE_SEED);
        fs::write(&seed, &invalid).unwrap();
        let current = fs::read(dir.path().join(Slot::Current.name())).unwrap();
        assert!(NativeTextGovernedStorage::open(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
        )
        .is_err());
        assert_eq!(fs::read(seed).unwrap(), invalid);
        assert_eq!(
            fs::read(dir.path().join(Slot::Current.name())).unwrap(),
            current
        );
    }
}
