//! Namespace4 and namespace5 declared in either order on real files, and real JOIN
//! lifecycle writes preserving both descriptors exactly. Declaring ID5 grants nothing to
//! governance and needs nothing from it; no Source4, member source or save is exercised.
use super::super::super::member_outbox_layout::{
    codec as member_codec, MEMBER_OUTBOX_MARKER, MEMBER_OUTBOX_PARENT, MEMBER_OUTBOX_SEED,
};
use super::super::super::{catalog_envelope as outer, V2MemberOutboxPolicy};
use super::*;

fn with_member(
    policy: current_layout::V2CurrentLayoutPolicy,
) -> current_layout::V2CurrentLayoutPolicy {
    let member_pool = CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap();
    policy
        .with_member_outbox(
            V2MemberOutboxPolicy::new(
                member_pool,
                256,
                256 * 1024 * 1024,
                1 << 20,
                1,
                member_codec::PERSISTENT_MINIMUM,
            )
            .unwrap(),
        )
        .unwrap()
}
fn member_governance(pool: &CriticalCatalogReadPool) -> V2GovernanceLayoutPolicy {
    V2GovernanceLayoutPolicy::new(with_member(current_policy(pool)), codec::PERSISTENT_MINIMUM)
        .unwrap()
}

/// `(id, version, payload)` per catalog entry, in order.
type Entries = Vec<(u32, u32, Vec<u8>)>;

/// `(id, version, payload)` of the selected catalog, decrypted with the vault key.
fn catalog(key: &[u8; 32], domain: &[u8; 32], path: &Path) -> Entries {
    let raw = fs::read(path.join(Slot::Current.name())).unwrap();
    let plain = unseal(key, &unframe(&raw).unwrap()).unwrap();
    let parsed = outer::parse(&plain, domain, outer::MAX_ENVELOPE_PLAIN).unwrap();
    (0..parsed.count)
        .map(|index| {
            let entry = parsed.entry(index).unwrap();
            (entry.id, entry.version, entry.payload.to_vec())
        })
        .collect()
}
fn entry(entries: &[(u32, u32, Vec<u8>)], id: u32) -> Vec<u8> {
    entries.iter().find(|e| e.0 == id).unwrap().2.clone()
}
fn ids(entries: &[(u32, u32, Vec<u8>)]) -> Vec<u32> {
    entries.iter().map(|e| e.0).collect()
}
type Snapshot = (
    std::collections::BTreeMap<&'static str, Option<Vec<u8>>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    bool,
);
/// The governance fixture's objects plus every namespace5 artifact.
fn all_files(path: &Path) -> Snapshot {
    let root = path.join(ARCHIVE_BOOTSTRAP_ROOT);
    (
        files(path),
        fs::read(path.join(MEMBER_OUTBOX_SEED)).ok(),
        fs::read(root.join(MEMBER_OUTBOX_MARKER)).ok(),
        root.join(MEMBER_OUTBOX_PARENT).is_dir(),
    )
}

/// A namespace5 declaration interrupted through the governance+member wrapper on a vault
/// that already holds 4: the governance gate must settle the member seed FIRST (its stage
/// owns the catalog slot), then let governance and the Current3 gate re-inspect. A first
/// seed that never authenticated (241 claim only, 240 claim + fence, 242 torn) is abandoned
/// with no RNG and declared afresh by the warm retry; every later cut finishes with no RNG.
#[test]
fn member_cuts_through_the_governance_wrapper_settle_before_governance_inspects() {
    let shared = pool();
    for (cut, complete) in [
        (240, false),
        (241, false),
        (242, false),
        (243, true),
        (245, true),
        (246, true),
        (247, true),
        (248, true),
        (249, true),
        (250, true),
        (255, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let mut rng = ChaCha20Rng::seed_from_u64(0x5560 + u64::from(cut));
        let (key, domain, after_four) = member_cut_over_four(path, &shared, cut, &mut rng);
        let snapshot = all_files(path);
        let mut cold = PreparedV2GovernanceLayoutStore::open(
            path,
            SECRET,
            &mut NeverRng,
            member_governance(&shared),
        )
        .unwrap_or_else(|error| panic!("cut {cut}: {error:?}"));
        if complete {
            cold.retry(&mut NeverRng).unwrap();
        } else {
            // Abandoned at open, with no RNG: no member object (seed, fence, marker, parent)
            // remains, and 4 and the selected catalog are untouched.
            let (objects, seed, marker, parent) = all_files(path);
            for name in [
                Slot::Current.name(),
                "chat-owner-v2/governance-layout.bin",
                "chat-owner-v2/current-layout.bin",
            ] {
                assert_eq!(objects[name], snapshot.0[name], "cut {cut}: {name}");
            }
            assert!(objects[Slot::Stage.name()].is_none(), "cut {cut}");
            assert!(seed.is_none() && marker.is_none() && !parent, "cut {cut}");
            cold.retry(&mut ChaCha20Rng::seed_from_u64(0x5600 + u64::from(cut)))
                .unwrap();
        }
        assert_settled_four_and_five(&cold, path, &key, &domain, &after_four, cut);
        drop(cold.into_governance_layout_store().unwrap());
    }
}

/// A vault holding settled 4, then a namespace5 declaration through the governance+member
/// wrapper cut at `cut`. Returns the key, domain and the catalog right after 4.
fn member_cut_over_four(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    cut: u8,
    rng: &mut ChaCha20Rng,
) -> ([u8; 32], [u8; 32], Entries) {
    settled_three(path, pool, rng);
    let mut four = open(path, pool, rng);
    four.retry(rng).unwrap();
    let key = private(&four).keys.db_key().unwrap();
    let domain = private(&four).chat_vault_domain().unwrap();
    drop(four.into_governance_layout_store().unwrap());
    let after_four = catalog(&key, &domain, path);
    let mut owner =
        PreparedV2GovernanceLayoutStore::open(path, SECRET, rng, member_governance(pool)).unwrap();
    fault(&owner, cut);
    assert!(owner.retry(rng).is_err(), "cut {cut}");
    drop(owner);
    (key, domain, after_four)
}

/// Both namespaces mounted, 3/4/5 in order with 4 byte-identical, and no seed left.
fn assert_settled_four_and_five(
    cold: &PreparedV2GovernanceLayoutStore,
    path: &Path,
    key: &[u8; 32],
    domain: &[u8; 32],
    after_four: &[(u32, u32, Vec<u8>)],
    cut: u8,
) {
    private(cold)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    private(cold).validate_mounted_governance_layout().unwrap();
    let both = catalog(key, domain, path);
    assert_eq!(&ids(&both)[ids(&both).len() - 3..], &[3, 4, 5], "cut {cut}");
    assert_eq!(entry(&both, 4), entry(after_four, 4), "cut {cut}");
    assert!(
        fs::read(path.join(MEMBER_OUTBOX_SEED)).is_err(),
        "cut {cut}"
    );
    assert!(fs::read(path.join(GOVERNANCE_SEED)).is_err(), "cut {cut}");
    assert!(
        fs::read(path.join(Slot::Stage.name())).is_err(),
        "cut {cut}"
    );
}

/// The cold first-torn disposal interrupted through the governance wrapper. Once the seed is
/// gone (228) the governance gate no longer runs the member inspector first; the Current3 gate
/// then sees no member artifact, and the warm retry declares 5.
#[test]
fn a_first_torn_disposal_through_the_governance_wrapper_completes_after_each_cold_cut() {
    use crate::store::critical_namespaces::member_outbox_layout::{
        arm_member_cold_fault_for_test, take_member_cold_fault_fired_for_test,
    };
    let shared = pool();
    for cold_cut in [226u8, 227, 228] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let mut rng = ChaCha20Rng::seed_from_u64(0x5570 + u64::from(cold_cut));
        let (key, domain, after_four) = member_cut_over_four(path, &shared, 242, &mut rng);
        arm_member_cold_fault_for_test(cold_cut);
        let refused = PreparedV2GovernanceLayoutStore::open(
            path,
            SECRET,
            &mut NeverRng,
            member_governance(&shared),
        );
        assert!(refused.is_err(), "cut {cold_cut}");
        drop(refused);
        assert!(
            take_member_cold_fault_fired_for_test(),
            "cut {cold_cut}: dead seam"
        );
        assert!(fs::read(path.join(Slot::Stage.name())).is_err());
        assert_eq!(
            path.join(MEMBER_OUTBOX_SEED).exists(),
            cold_cut != 228,
            "cut {cold_cut}"
        );
        let mut cold = PreparedV2GovernanceLayoutStore::open(
            path,
            SECRET,
            &mut NeverRng,
            member_governance(&shared),
        )
        .unwrap_or_else(|error| panic!("cut {cold_cut}: {error:?}"));
        assert!(!path.join(MEMBER_OUTBOX_SEED).exists());
        cold.retry(&mut rng).unwrap();
        assert_settled_four_and_five(&cold, path, &key, &domain, &after_four, cold_cut);
    }
}

/// Resets the predecessor emulation even if an assertion inside the scope panics.
struct Predecessor(());
impl Predecessor {
    fn on() -> Self {
        crate::store::critical_namespaces::member_outbox_layout::emulate_pre_namespace5_for_test(
            true,
        );
        Self(())
    }
}
impl Drop for Predecessor {
    fn drop(&mut self) {
        crate::store::critical_namespaces::member_outbox_layout::emulate_pre_namespace5_for_test(
            false,
        );
    }
}

/// A predecessor's governance opener does not know the member seed name, so in the zero-byte
/// claim window it can begin, and be interrupted inside, a governance4 declaration. The head
/// then holds BOTH seeds:
/// - a COMPLETE governance seed (232-237): the claim is disposed without touching the
///   stage (governance's own), governance finishes cold, and the warm retry declares 5;
/// - governance already finished (238): an ordinary first-torn claim, disposed;
/// - only governance's empty fence (229): it looks like the claim's own fence and is unlinked.
///   This is the documented residual; it holds no bytes, and the retry declares 4 then 5;
/// - an empty or torn governance FIRST seed (230, 231): governance's own unrecovered state.
///   The claim is preserved and refused too, rather than disposed in front of that refusal.
#[test]
fn a_predecessor_governance_declaration_in_the_claim_window_never_deadlocks_the_member_seed() {
    let shared = pool();
    for (governance_cut, recovers) in [
        (229u8, true),
        (230, false),
        (231, false),
        (232, true),
        (233, true),
        (234, true),
        (235, true),
        (236, true),
        (237, true),
        (238, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let mut rng = ChaCha20Rng::seed_from_u64(0x55a0 + u64::from(governance_cut));
        settled_three(path, &shared, &mut rng);
        // The zero-byte claim alone (a namespace5 declaration cut at 241 leaves exactly this).
        fs::write(path.join(MEMBER_OUTBOX_SEED), b"").unwrap();
        let (key, domain) = {
            let _predecessor = Predecessor::on();
            let mut owner = open(path, &shared, &mut rng);
            let key = private(&owner).keys.db_key().unwrap();
            let domain = private(&owner).chat_vault_domain().unwrap();
            fault(&owner, governance_cut);
            assert!(owner.retry(&mut rng).is_err(), "cut {governance_cut}");
            (key, domain)
        };
        assert_eq!(fs::read(path.join(MEMBER_OUTBOX_SEED)).unwrap(), b"");
        let snapshot = all_files(path);
        let reopened = PreparedV2GovernanceLayoutStore::open(
            path,
            SECRET,
            &mut NeverRng,
            member_governance(&shared),
        );
        if !recovers {
            assert!(reopened.is_err(), "cut {governance_cut}");
            drop(reopened);
            assert_eq!(all_files(path), snapshot, "cut {governance_cut}");
            continue;
        }
        let mut cold = reopened.unwrap_or_else(|error| panic!("cut {governance_cut}: {error:?}"));
        assert!(
            !path.join(MEMBER_OUTBOX_SEED).exists(),
            "cut {governance_cut}"
        );
        cold.retry(&mut rng).unwrap();
        private(&cold).validate_mounted_governance_layout().unwrap();
        private(&cold)
            .validate_mounted_member_outbox_layout()
            .unwrap();
        drop(cold.into_governance_layout_store().unwrap());
        let both = catalog(&key, &domain, path);
        assert_eq!(
            &ids(&both)[ids(&both).len() - 3..],
            &[3, 4, 5],
            "cut {governance_cut}"
        );
        assert!(fs::read(path.join(GOVERNANCE_SEED)).is_err());
        assert!(fs::read(path.join(Slot::Stage.name())).is_err());
    }
}

/// Governance's own interrupted declaration over a vault that already holds 5: recovery
/// inserts 4 before 5 and keeps 5 byte for byte. Cuts 229-231 are governance's own
/// first-torn-seed states, preserved and refused (their recovery is not this module's).
#[test]
fn governance_cuts_over_an_existing_five_recover_and_keep_five_exactly() {
    let shared = pool();
    for (cut, finishes) in [
        (229, false),
        (230, false),
        (231, false),
        (232, true),
        (233, true),
        (234, true),
        (235, true),
        (236, true),
        (237, true),
        (238, true),
        (239, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let mut rng = ChaCha20Rng::seed_from_u64(0x5580 + u64::from(cut));
        settled_three(path, &shared, &mut rng);
        let mut five = current_layout::PreparedV2CurrentLayoutStore::open(
            path,
            SECRET,
            &mut rng,
            with_member(current_policy(&shared)),
        )
        .unwrap();
        five.retry(&mut rng).unwrap();
        let five = five.into_current_layout_store().unwrap();
        let key = five.checked_store().unwrap().keys.db_key().unwrap();
        let domain = five.checked_store().unwrap().chat_vault_domain().unwrap();
        drop(five);
        let after_five = catalog(&key, &domain, path);

        let mut owner = PreparedV2GovernanceLayoutStore::open(
            path,
            SECRET,
            &mut rng,
            member_governance(&shared),
        )
        .unwrap();
        fault(&owner, cut);
        assert!(owner.retry(&mut rng).is_err(), "cut {cut}");
        drop(owner);
        let snapshot = all_files(path);
        let reopened = PreparedV2GovernanceLayoutStore::open(
            path,
            SECRET,
            &mut NeverRng,
            member_governance(&shared),
        );
        if finishes {
            let mut cold = reopened.unwrap_or_else(|error| panic!("cut {cut}: {error:?}"));
            cold.retry(&mut NeverRng).unwrap();
            private(&cold).validate_mounted_governance_layout().unwrap();
            private(&cold)
                .validate_mounted_member_outbox_layout()
                .unwrap();
            drop(cold.into_governance_layout_store().unwrap());
            let both = catalog(&key, &domain, path);
            assert_eq!(&ids(&both)[ids(&both).len() - 3..], &[3, 4, 5], "cut {cut}");
            assert_eq!(entry(&both, 5), entry(&after_five, 5), "cut {cut}");
        } else {
            assert!(reopened.is_err(), "cut {cut}");
            assert_eq!(all_files(path), snapshot, "cut {cut}");
        }
    }
}

#[test]
fn declare_five_close_reopen_then_declare_four_inserts_four_before_five() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5541);
    settled_three(path, &shared, &mut rng);

    // Namespace5 through the Current3 opener only.
    let mut five = current_layout::PreparedV2CurrentLayoutStore::open(
        path,
        SECRET,
        &mut rng,
        with_member(current_policy(&shared)),
    )
    .unwrap();
    five.retry(&mut rng).unwrap();
    let five = five.into_current_layout_store().unwrap();
    let store = five.checked_store().unwrap();
    let key = store.keys.db_key().unwrap();
    let domain = store.chat_vault_domain().unwrap();
    assert!(store.governance_layout.lock().unwrap().is_none());
    drop(five);
    let after_five = catalog(&key, &domain, path);
    assert_eq!(&ids(&after_five)[ids(&after_five).len() - 2..], &[3, 5]);

    // A governance opener WITHOUT the member handler refuses the ID5 vault before cleanup.
    let snapshot = all_files(path);
    assert!(
        PreparedV2GovernanceLayoutStore::open(path, SECRET, &mut NeverRng, policy(&shared))
            .is_err()
    );
    assert_eq!(all_files(path), snapshot);

    // With both handlers: 4 is inserted BEFORE the existing 5, which is kept byte for byte.
    let mut owner =
        PreparedV2GovernanceLayoutStore::open(path, SECRET, &mut rng, member_governance(&shared))
            .unwrap();
    owner.retry(&mut rng).unwrap();
    private(&owner)
        .validate_mounted_governance_layout()
        .unwrap();
    private(&owner)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    drop(owner.into_governance_layout_store().unwrap());
    let both = catalog(&key, &domain, path);
    assert_eq!(&ids(&both)[ids(&both).len() - 3..], &[3, 4, 5]);
    assert_eq!(entry(&both, 5), entry(&after_five, 5));
    assert_eq!(entry(&both, 3), entry(&after_five, 3));

    // The Current3 opener (even with the member handler) cannot read governance4, and the
    // governance opener without the member handler cannot read namespace5.
    let snapshot = all_files(path);
    assert!(current_layout::PreparedV2CurrentLayoutStore::open(
        path,
        SECRET,
        &mut NeverRng,
        with_member(current_policy(&shared))
    )
    .is_err());
    assert!(
        PreparedV2GovernanceLayoutStore::open(path, SECRET, &mut NeverRng, policy(&shared))
            .is_err()
    );
    assert_eq!(all_files(path), snapshot);
    let mut cold = PreparedV2GovernanceLayoutStore::open(
        path,
        SECRET,
        &mut NeverRng,
        member_governance(&shared),
    )
    .unwrap();
    cold.retry(&mut NeverRng).unwrap();
    drop(cold.into_governance_layout_store().unwrap());
    assert_eq!(all_files(path), snapshot);
}

#[test]
fn declare_four_then_five_appends_five_and_keeps_four_exactly() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5542);
    settled_three(path, &shared, &mut rng);
    let mut four = open(path, &shared, &mut rng);
    four.retry(&mut rng).unwrap();
    let key = private(&four).keys.db_key().unwrap();
    let domain = private(&four).chat_vault_domain().unwrap();
    assert!(private(&four)
        .member_outbox_layout
        .lock()
        .unwrap()
        .is_none());
    drop(four.into_governance_layout_store().unwrap());
    let after_four = catalog(&key, &domain, path);
    assert!(!path
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(MEMBER_OUTBOX_PARENT)
        .exists());

    let mut owner =
        PreparedV2GovernanceLayoutStore::open(path, SECRET, &mut rng, member_governance(&shared))
            .unwrap();
    owner.retry(&mut rng).unwrap();
    private(&owner)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    drop(owner.into_governance_layout_store().unwrap());
    let both = catalog(&key, &domain, path);
    assert_eq!(&ids(&both)[ids(&both).len() - 3..], &[3, 4, 5]);
    assert_eq!(entry(&both, 4), entry(&after_four, 4));
    assert!(path
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(MEMBER_OUTBOX_PARENT)
        .is_dir());
}

#[test]
fn actual_join_lifecycle_preserves_both_four_and_five_exactly() {
    use crate::store::{CheckpointJoinSubmissionStatus, OwnerEvidenceBudget};
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, PeerId};
    let source_dir = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5543);
    let source = ServerStore::open(source_dir.path(), SECRET, &mut rng)
        .unwrap()
        .into_critical_namespace_store(SECRET, &mut rng)
        .unwrap();
    let budget = OwnerEvidenceBudget::checkpoint(
        8,
        256 * 1024 * 1024,
        256 * 1024 * 1024,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap();
    source.configure_owner_evidence_budget(budget).unwrap();
    let hub = Hub::new();
    let mut founder = source
        .prepare_owner_evidence_founder(
            hub.join(PeerId::from_u64(0x5543)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0x5543),
            Box::new(ManualClock::new(1_000)),
            "actual checkpoint source",
            9,
        )
        .unwrap();
    let owner = founder.retry(&source, &mut rng).unwrap();
    let invite = owner
        .mint_owner_checkpoint_invite(&source, [72; 16], 100_000, vec![])
        .unwrap();
    settled_three(directory.path(), &shared, &mut rng);
    let execution = with_member(
        current_layout::V2CurrentLayoutPolicy::new(
            ArchiveBootstrapCatalogPolicy::new(
                shared.clone(),
                4 * archive_bootstrap::MAX_SEED_FRAMED,
            )
            .unwrap()
            .with_bootstrap_source_data_read()
            .with_checkpoint_join_execution(2, 10_000)
            .unwrap(),
            CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
            4 * archive_current_namespace_codec::MAX_DECLARATION_FRAME,
        )
        .unwrap(),
    );
    let governance_policy =
        V2GovernanceLayoutPolicy::new(execution, codec::PERSISTENT_MINIMUM).unwrap();
    // One retry declares 4 and then 5.
    let mut opening = PreparedV2GovernanceLayoutStore::open(
        directory.path(),
        SECRET,
        &mut rng,
        governance_policy.clone(),
    )
    .unwrap();
    opening.retry(&mut rng).unwrap();
    let ready = opening.into_governance_layout_store().unwrap();
    let store = &ready.store; // private child-test access only
    store.configure_owner_evidence_budget(budget).unwrap();
    let key = store.keys.db_key().unwrap();
    let before = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let plain = Zeroizing::new(unseal(&key, &unframe(&before).unwrap()).unwrap());
    let parsed = store.parse_configured_join_catalog(&plain).unwrap();
    let governance_payload = parsed.governance_payload.unwrap().to_vec();
    let member_payload = parsed.member_outbox_payload.unwrap().to_vec();

    let mut prepared = store
        .prepare_journalled_owner_checkpoint_join(
            &source,
            &owner,
            MlsDevice::generate().unwrap(),
            invite,
            &mut rng,
        )
        .unwrap();
    let preserved = |raw: &[u8]| {
        let plain = Zeroizing::new(unseal(&key, &unframe(raw).unwrap()).unwrap());
        let parsed = store.parse_configured_join_catalog(&plain).unwrap();
        (
            parsed.governance_payload.unwrap() == governance_payload.as_slice(),
            parsed.member_outbox_payload.unwrap() == member_payload.as_slice(),
        )
    };
    let current = || fs::read(directory.path().join(Slot::Current.name())).unwrap();
    prepared.save_pending(store, &mut rng).unwrap();
    assert_eq!(preserved(&current()), (true, true), "save");
    prepared.pause_pending(store, &mut rng).unwrap();
    assert_eq!(preserved(&current()), (true, true), "pause");
    prepared.resume_pending(store, &mut rng).unwrap();
    assert_eq!(preserved(&current()), (true, true), "resume");
    let activated = prepared
        .activate_original_transport(
            store,
            hub.join(PeerId::from_u64(0x5544)),
            PeerId::from_u64(0x5543),
        )
        .unwrap();
    let mut network = activated.into_network_execution(store).unwrap();
    assert_eq!(
        network
            .ensure_submission_armed(store, &mut rng, Arc::new(ManualClock::new(1_000)))
            .unwrap(),
        CheckpointJoinSubmissionStatus::Armed
    );
    let updated = current();
    assert_eq!(preserved(&updated), (true, true), "migration and arm");
    let updated_plain = Zeroizing::new(unseal(&key, &unframe(&updated).unwrap()).unwrap());
    assert_eq!(
        store
            .parse_configured_join_catalog(&updated_plain)
            .unwrap()
            .join_version,
        Some(super::super::super::pending_join::execution_payload::VERSION)
    );
    store.validate_mounted_governance_layout().unwrap();
    store.validate_mounted_member_outbox_layout().unwrap();
    drop(network);
    drop(ready);

    let mut cold = PreparedV2GovernanceLayoutStore::open(
        directory.path(),
        SECRET,
        &mut NeverRng,
        governance_policy,
    )
    .unwrap();
    cold.retry(&mut NeverRng).unwrap();
    private(&cold)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    assert_eq!(current(), updated);
}
