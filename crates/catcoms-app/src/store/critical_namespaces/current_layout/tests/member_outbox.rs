//! Namespace5 (member outbox layout) declared through the Current3 opener, with no
//! governance4. Real files and the real vault; no member source, save or governance authority
//! is exercised or granted. Primary owns serial compiled validation; these are UNRUN here.
use super::super::super::member_outbox_layout::{
    codec as member_codec, MEMBER_OUTBOX_MARKER, MEMBER_OUTBOX_PARENT, MEMBER_OUTBOX_SEED,
};
use super::super::super::{catalog_envelope as outer, member_outbox_namespace_codec as member_ns};
use super::*;
use std::collections::BTreeMap;

/// The concrete member handler on its own independent pool (never the catalog pool).
pub(super) fn member_handler() -> V2MemberOutboxPolicy {
    V2MemberOutboxPolicy::new(
        CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap(),
        256,
        256 * 1024 * 1024,
        1 << 20,
        1,
        member_codec::PERSISTENT_MINIMUM,
    )
    .unwrap()
}

pub(super) fn member_policy(pool: &CriticalCatalogReadPool) -> V2CurrentLayoutPolicy {
    policy(pool).with_member_outbox(member_handler()).unwrap()
}

/// The Current3 opener with checkpoint-JOIN execution and WITHOUT the member handler: an
/// up-to-date binary configured for JOIN work but not for namespace5.
pub(super) fn execution_policy(pool: &CriticalCatalogReadPool) -> V2CurrentLayoutPolicy {
    V2CurrentLayoutPolicy::new(
        archive(pool)
            .with_checkpoint_join_execution(2, 10_000)
            .unwrap(),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * codec::MAX_DECLARATION_FRAME,
    )
    .unwrap()
}

/// The same JOIN-executing opener WITH the member handler, and no governance4 handler.
pub(super) fn member_execution_policy(pool: &CriticalCatalogReadPool) -> V2CurrentLayoutPolicy {
    execution_policy(pool)
        .with_member_outbox(member_handler())
        .unwrap()
}

/// A real checkpoint JOIN source: a separate founder vault and its owner, from which each
/// test mints one invite. It is the existing JOIN fixture's founder, never a member, save or
/// governance authority over the vault under test. Field order is Drop order, matching the
/// original single-function fixture: owner before its initialization, Store before its
/// directory.
pub(super) struct JoinSource {
    pub(super) owner: crate::store::OwnedOwnerServer<catcoms_rt::MemNetwork, ChaCha20Rng>,
    _founder: crate::store::OwnerFounderInitialization<catcoms_rt::MemNetwork, ChaCha20Rng>,
    pub(super) hub: Arc<catcoms_rt::Hub>,
    budget: crate::store::OwnerEvidenceBudget,
    pub(super) store: ServerStore,
    _dir: tempfile::TempDir,
}

pub(super) fn join_source(seed: u64, rng: &mut ChaCha20Rng) -> JoinSource {
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, PeerId};
    let dir = tempfile::tempdir().unwrap();
    let store = ServerStore::open(dir.path(), SECRET, rng)
        .unwrap()
        .into_critical_namespace_store(SECRET, rng)
        .unwrap();
    let budget = crate::store::OwnerEvidenceBudget::checkpoint(
        8,
        256 * 1024 * 1024,
        256 * 1024 * 1024,
        24 * 1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap();
    store.configure_owner_evidence_budget(budget).unwrap();
    let hub = Hub::new();
    let mut founder = store
        .prepare_owner_evidence_founder(
            hub.join(PeerId::from_u64(seed)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(seed),
            Box::new(ManualClock::new(1_000)),
            "actual checkpoint source",
            9,
        )
        .unwrap();
    let owner = founder.retry(&store, rng).unwrap();
    JoinSource {
        owner,
        _founder: founder,
        hub,
        budget,
        store,
        _dir: dir,
    }
}

/// A settled Current3 + namespace5 vault (no governance4) opened by the JOIN-executing member
/// opener, ready for an actual JOIN preparation from `source`.
pub(super) fn five_with_join_execution(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    source: &JoinSource,
    rng: &mut ChaCha20Rng,
) -> V2CurrentLayoutStore {
    settled_three(path, pool, rng);
    let mut opening =
        PreparedV2CurrentLayoutStore::open(path, SECRET, rng, member_execution_policy(pool))
            .unwrap();
    opening.retry(rng).unwrap();
    let ready = opening.into_current_layout_store().unwrap();
    let store = ready.checked_store().unwrap();
    store
        .configure_owner_evidence_budget(source.budget)
        .unwrap();
    store.validate_mounted_member_outbox_layout().unwrap();
    assert!(store.governance_layout.lock().unwrap().is_none());
    ready
}

/// The selected catalog's exact ID5 descriptor payload, whether governance4 is present, and
/// its number of JOIN rows. DATA read with the vault key; grants nothing.
fn catalog_facts(store: &ServerStore, raw: &[u8]) -> (Vec<u8>, bool, usize) {
    let key = store.keys.db_key().unwrap();
    let plain = Zeroizing::new(unseal(&key, &unframe(raw).unwrap()).unwrap());
    let parsed = store.parse_configured_join_catalog(&plain).unwrap();
    (
        parsed.member_outbox_payload.unwrap().to_vec(),
        parsed.governance.is_some(),
        parsed.rows.iter().flatten().count(),
    )
}

pub(super) fn settled_three(path: &Path, pool: &CriticalCatalogReadPool, rng: &mut ChaCha20Rng) {
    prepare_two(path, pool, rng);
    let mut owner = open(path, pool, rng);
    owner.retry(rng).unwrap();
    drop(owner.into_current_layout_store().unwrap());
}

/// Every fixed object this namespace or its neighbours own, byte for byte, plus whether the
/// member parent directory exists.
fn files(path: &Path) -> (BTreeMap<&'static str, Option<Vec<u8>>>, bool) {
    let objects = [
        Slot::Current.name(),
        Slot::Stage.name(),
        MEMBER_OUTBOX_SEED,
        "chat-owner-v2/member-outbox-layout.bin",
        "chat-owner-v2/current-layout.bin",
        "chat-owner-v2/root.bin",
        "chat-owner-v2/governance-layout.bin",
        crate::store::critical_namespaces::governance_layout::GOVERNANCE_SEED,
    ]
    .into_iter()
    .map(|name| (name, fs::read(path.join(name)).ok()))
    .collect();
    let parent = path
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(MEMBER_OUTBOX_PARENT)
        .is_dir();
    (objects, parent)
}

/// Resets the emulation even if an assertion inside the scope panics.
struct PreNamespace5(());
impl PreNamespace5 {
    fn on() -> Self {
        crate::store::critical_namespaces::member_outbox_layout::emulate_pre_namespace5_for_test(
            true,
        );
        Self(())
    }
}
impl Drop for PreNamespace5 {
    fn drop(&mut self) {
        crate::store::critical_namespaces::member_outbox_layout::emulate_pre_namespace5_for_test(
            false,
        );
    }
}

/// The default, Current3 and governance openers of a binary without namespace5 support all
/// refuse this vault before any cleaner.
fn refuses_as_pre_namespace5(path: &Path, pool: &CriticalCatalogReadPool, cut: u8) {
    use crate::store::critical_namespaces::governance_layout::{
        codec::PERSISTENT_MINIMUM, PreparedV2GovernanceLayoutStore, V2GovernanceLayoutPolicy,
    };
    let _emulated = PreNamespace5::on();
    assert!(
        ServerStore::open(path, SECRET, &mut NeverRng).is_err(),
        "cut {cut}"
    );
    assert!(
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, policy(pool)).is_err(),
        "cut {cut}"
    );
    let governance = V2GovernanceLayoutPolicy::new(policy(pool), PERSISTENT_MINIMUM).unwrap();
    assert!(
        PreparedV2GovernanceLayoutStore::open(path, SECRET, &mut NeverRng, governance).is_err(),
        "cut {cut}"
    );
}

/// A state with no CONTENT-BEARING member output (a zero-byte claim, or an abandoned
/// unauthenticated seed whose fence is already gone) is deliberately not fenced: the
/// predecessor's Current3 and governance openers open it, ignoring the unknown seed name, and
/// leave the seed byte for byte. Opening only; nothing is retried or settled here.
fn opens_as_pre_namespace5(path: &Path, pool: &CriticalCatalogReadPool, cut: u8) {
    use crate::store::critical_namespaces::governance_layout::{
        codec::PERSISTENT_MINIMUM, PreparedV2GovernanceLayoutStore, V2GovernanceLayoutPolicy,
    };
    let seed = fs::read(path.join(MEMBER_OUTBOX_SEED)).unwrap();
    let _emulated = PreNamespace5::on();
    let current = PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, policy(pool));
    assert!(current.is_ok(), "cut {cut}");
    drop(current);
    let governance = V2GovernanceLayoutPolicy::new(policy(pool), PERSISTENT_MINIMUM).unwrap();
    let governance = PreparedV2GovernanceLayoutStore::open(path, SECRET, &mut NeverRng, governance);
    assert!(governance.is_ok(), "cut {cut}");
    drop(governance);
    assert_eq!(
        fs::read(path.join(MEMBER_OUTBOX_SEED)).unwrap(),
        seed,
        "cut {cut}"
    );
}

/// After a cold open abandoned a first-torn declaration: no member object remains, and the
/// current catalog is exactly what it was. The warm retry must then declare afresh.
fn assert_abandoned(
    path: &Path,
    before: &(BTreeMap<&'static str, Option<Vec<u8>>>, bool),
    cut: u8,
) {
    let (objects, parent) = files(path);
    assert!(!parent, "cut {cut}");
    assert!(objects[MEMBER_OUTBOX_SEED].is_none(), "cut {cut}");
    assert!(objects[Slot::Stage.name()].is_none(), "cut {cut}");
    assert!(
        objects["chat-owner-v2/member-outbox-layout.bin"].is_none(),
        "cut {cut}"
    );
    assert_eq!(
        objects[Slot::Current.name()],
        before.0[Slot::Current.name()],
        "cut {cut}"
    );
}

/// A configured warm retry declares namespace5 and leaves only its settled objects.
fn assert_declared(cold: &PreparedV2CurrentLayoutStore, path: &Path, cut: u8) {
    private(cold)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    let (objects, parent) = files(path);
    assert!(parent, "cut {cut}");
    assert!(objects[MEMBER_OUTBOX_SEED].is_none(), "cut {cut}");
    assert!(objects[Slot::Stage.name()].is_none(), "cut {cut}");
    assert!(
        objects["chat-owner-v2/member-outbox-layout.bin"].is_some(),
        "cut {cut}"
    );
}

/// The warm Current3 declaration cut at `cut` and abandoned, on a fresh settled-3 vault.
/// Returns the vault database key, for negative fixtures that must seal with it.
pub(super) fn member_cut(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    cut: u8,
    rng: &mut ChaCha20Rng,
) -> [u8; 32] {
    settled_three(path, pool, rng);
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, rng, member_policy(pool)).unwrap();
    fault(&owner, cut);
    assert!(owner.retry(rng).is_err(), "cut {cut}");
    private(&owner).keys.db_key().unwrap()
}

pub(super) fn ids(key: &[u8; 32], domain: &[u8; 32], path: &Path) -> Vec<u32> {
    let raw = fs::read(path.join(Slot::Current.name())).unwrap();
    let plain = unseal(key, &unframe(&raw).unwrap()).unwrap();
    let parsed = outer::parse(&plain, domain, outer::MAX_ENVELOPE_PLAIN).unwrap();
    (0..parsed.count)
        .map(|index| parsed.entry(index).unwrap().id)
        .collect()
}

#[test]
fn member_same_opener_retries_after_seed_unlink_without_rng_or_retained_handles() {
    for cut in [250, 255] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x55e0 + u64::from(cut));
        settled_three(path, &shared, &mut rng);
        let mut owner =
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared))
                .unwrap();
        fault(&owner, cut);
        assert!(owner.retry(&mut rng).is_err(), "cut {cut}");
        let declaration = owner.member_declaration.as_ref().unwrap();
        assert!(declaration.seed_unlinked);
        assert!(declaration.seed_file.is_none());
        let candidate = declaration.candidate.to_vec();
        // `exists()` would hide Windows AccessDenied/delete-pending, the original bug.
        assert_eq!(
            fs::symlink_metadata(path.join(MEMBER_OUTBOX_SEED))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
        fault(&owner, 0);
        owner.retry(&mut NeverRng).unwrap();
        assert_eq!(
            fs::read(path.join(Slot::Current.name())).unwrap(),
            candidate
        );
        assert_declared(&owner, path, cut);
        drop(owner.into_current_layout_store().unwrap());
    }
}

#[test]
fn member_namespace_declares_on_current3_alone_and_unconfigured_openers_refuse_before_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5501);
    settled_three(path, &shared, &mut rng);
    let before = fs::read(path.join(Slot::Current.name())).unwrap();

    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared)).unwrap();
    owner.retry(&mut rng).unwrap();
    let store = private(&owner);
    let key = store.keys.db_key().unwrap();
    let domain = store.chat_vault_domain().unwrap();
    let after = fs::read(path.join(Slot::Current.name())).unwrap();
    let old_plain = unseal(&key, &unframe(&before).unwrap()).unwrap();
    let new_plain = unseal(&key, &unframe(&after).unwrap()).unwrap();
    let capability =
        member_ns::validate_declaration_pair_data(&old_plain, &new_plain, &domain).unwrap();
    assert_eq!(
        store
            .member_outbox_layout
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .capability,
        capability
    );
    // Namespace5 alone: no governance4 was declared, mounted or implied.
    assert!(store.governance_layout.lock().unwrap().is_none());
    assert!(store
        .parse_configured_join_catalog(&new_plain)
        .unwrap()
        .governance
        .is_none());
    let (objects, parent) = files(path);
    assert!(parent);
    assert!(objects[MEMBER_OUTBOX_SEED].is_none());
    assert!(objects[Slot::Stage.name()].is_none());
    assert!(objects["chat-owner-v2/member-outbox-layout.bin"].is_some());
    assert!(objects["chat-owner-v2/governance-layout.bin"].is_none());
    let (parent_path, _) = store.member_outbox_parent_data().unwrap();
    assert_eq!(
        parent_path,
        path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_PARENT)
    );
    drop(owner.into_current_layout_store().unwrap());
    let ordered = ids(&key, &domain, path);
    assert_eq!(&ordered[ordered.len() - 2..], &[3, 5]);

    // Openers without the concrete handler refuse BEFORE any cleanup or settlement.
    let snapshot = files(path);
    assert!(ServerStore::open(path, SECRET, &mut NeverRng).is_err());
    assert_eq!(files(path), snapshot);
    assert!(
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, policy(&shared)).is_err()
    );
    assert_eq!(files(path), snapshot);

    // The configured opener remounts cold with no RNG and no write.
    let mut cold =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared))
            .unwrap();
    cold.retry(&mut NeverRng).unwrap();
    private(&cold)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    drop(cold.into_current_layout_store().unwrap());
    assert_eq!(files(path), snapshot);
}

#[test]
fn interrupted_member_declarations_finish_from_a_complete_seed_or_abandon_a_first_torn_one() {
    let shared = pool();
    // (cut, a complete authenticated seed exists). Cuts 241 (claim only), 240 (claim + fence)
    // and 242 (torn seed + fence) stop before one: the cold open abandons that declaration
    // with no RNG, and the warm retry declares afresh. Every later cut finishes the SAME
    // declaration with no RNG.
    for (cut, complete) in [
        (240, false),
        (241, false),
        (242, false),
        (243, true),
        (244, true),
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
        let mut rng = ChaCha20Rng::seed_from_u64(0x5510 + u64::from(cut));
        member_cut(path, &shared, cut, &mut rng);
        let snapshot = files(path);
        assert!(
            snapshot.0[MEMBER_OUTBOX_SEED].is_some() || complete,
            "cut {cut}"
        );
        // The claim always precedes the fence: a fence without a claim never exists.
        assert!(
            snapshot.0[Slot::Stage.name()].is_none() || snapshot.0[MEMBER_OUTBOX_SEED].is_some(),
            "cut {cut}"
        );

        // An opener without the handler refuses every interrupted state and changes nothing.
        assert!(
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, policy(&shared))
                .is_err(),
            "cut {cut}"
        );
        assert_eq!(files(path), snapshot, "cut {cut}");
        // A binary that predates namespace5 does not know the member names: it is fenced by
        // the stage slot and its closed grammar from every content-bearing output, and only
        // the zero-byte claim (241, no fence yet) is deliberately left unfenced.
        if cut == 241 {
            assert_eq!(snapshot.0[MEMBER_OUTBOX_SEED].as_deref(), Some(&[][..]));
            opens_as_pre_namespace5(path, &shared, cut);
        } else {
            refuses_as_pre_namespace5(path, &shared, cut);
        }
        assert_eq!(files(path), snapshot, "cut {cut}");

        let mut cold =
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared))
                .unwrap_or_else(|error| panic!("cut {cut}: {error:?}"));
        if complete {
            cold.retry(&mut NeverRng).unwrap();
        } else {
            assert_abandoned(path, &snapshot, cut);
            cold.retry(&mut ChaCha20Rng::seed_from_u64(0x5600 + u64::from(cut)))
                .unwrap();
        }
        assert_declared(&cold, path, cut);
        drop(cold.into_current_layout_store().unwrap());
    }
}

/// The cold first-torn disposal itself interrupted, at each of its cuts (226 after the fence
/// unlink, 227 after its barrier, 228 after the seed unlink). Every intermediate is first-torn
/// again (or already abandoned) on the next open; none is ambiguous or stuck.
#[test]
fn an_interrupted_first_torn_disposal_completes_on_the_next_open() {
    use crate::store::critical_namespaces::member_outbox_layout::{
        arm_member_cold_fault_for_test, take_member_cold_fault_fired_for_test,
    };
    let shared = pool();
    for cold_cut in [226u8, 227, 228] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let mut rng = ChaCha20Rng::seed_from_u64(0x5620 + u64::from(cold_cut));
        member_cut(path, &shared, 242, &mut rng);
        let before = files(path);
        let torn = before.0[MEMBER_OUTBOX_SEED].clone().unwrap();
        assert!(!torn.is_empty());
        arm_member_cold_fault_for_test(cold_cut);
        let refused =
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared));
        assert!(refused.is_err(), "cut {cold_cut}");
        drop(refused);
        assert!(
            take_member_cold_fault_fired_for_test(),
            "cut {cold_cut}: dead seam"
        );
        let (objects, _) = files(path);
        assert!(objects[Slot::Stage.name()].is_none(), "cut {cold_cut}");
        if cold_cut == 228 {
            assert!(objects[MEMBER_OUTBOX_SEED].is_none(), "cut {cold_cut}");
        } else {
            // The unfenced intermediate: the torn seed, untouched, with no fence.
            assert_eq!(objects[MEMBER_OUTBOX_SEED], Some(torn.clone()));
            opens_as_pre_namespace5(path, &shared, cold_cut);
        }
        let mut cold =
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared))
                .unwrap_or_else(|error| panic!("cut {cold_cut}: {error:?}"));
        assert!(
            !take_member_cold_fault_fired_for_test(),
            "one-shot seam fired again"
        );
        assert_abandoned(path, &before, cold_cut);
        cold.retry(&mut rng).unwrap();
        assert_declared(&cold, path, cold_cut);
        drop(cold.into_current_layout_store().unwrap());
    }
}

/// A removed name that is not provably gone (the Windows delete-pending case, reported by the
/// regression seam) refuses BEFORE the seed is touched: the seed stays byte for byte, so the
/// next open is first-torn again and completes.
#[test]
fn a_stage_name_not_proven_gone_refuses_before_the_seed_is_touched() {
    use crate::store::critical_namespaces::member_outbox_layout::{
        report_name_present_once_for_test, take_name_reported_present_for_test,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5630);
    member_cut(path, &shared, 242, &mut rng);
    let before = files(path);
    assert!(before.0[Slot::Stage.name()].is_some());
    report_name_present_once_for_test();
    let refused =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared));
    assert!(refused.is_err());
    drop(refused);
    // The seam was consumed by the stage's proof, and the seed was never reached.
    assert!(!take_name_reported_present_for_test());
    assert_eq!(
        files(path).0[MEMBER_OUTBOX_SEED],
        before.0[MEMBER_OUTBOX_SEED]
    );
    let mut cold =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared))
            .unwrap();
    assert_abandoned(path, &before, 242);
    cold.retry(&mut rng).unwrap();
    assert_declared(&cold, path, 242);
}

/// Everything the first-torn role must NOT treat as torn scratch: each state is preserved byte
/// for byte (recursive names and contents) and refused by the configured opener.
#[test]
fn first_torn_disposal_preserves_and_refuses_everything_it_cannot_attribute() {
    let shared = pool();
    // Each arranges a negative fixture over the first-torn cut-242 state (a torn seed and the
    // empty fence, with the original current catalog), given the vault database key.
    type Arrange = fn(&Path, &[u8; 32]);
    let cases: [(&str, Arrange); 6] = [
        // An AUTHENTICATED seed (sealed under the seed key) with invalid content: possibly a
        // future version's seed, never torn scratch.
        ("authenticated invalid seed", |path, key| {
            let sealed = member_codec::seal_seed_plain_for_test(
                key,
                b"not a v1 namespace5 declaration seed",
                &mut ChaCha20Rng::seed_from_u64(0x5641),
            );
            fs::write(path.join(MEMBER_OUTBOX_SEED), sealed.as_slice()).unwrap();
        }),
        // Anything a complete seed precedes: a marker,
        ("torn seed beside a marker", |path, _| {
            fs::write(
                path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_MARKER),
                [0x5a; 64],
            )
            .unwrap();
        }),
        // a parent directory,
        ("torn seed beside a parent", |path, _| {
            fs::create_dir(path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_PARENT)).unwrap();
        }),
        // or candidate bytes in the stage (a prefix: not an authenticated ordinary catalog).
        ("torn seed beside a partial stage", |path, _| {
            fs::write(path.join(Slot::Stage.name()), [0x5b; 17]).unwrap();
        }),
        // A seed that is not a real file.
        ("seed directory", |path, _| {
            fs::remove_file(path.join(MEMBER_OUTBOX_SEED)).unwrap();
            fs::create_dir(path.join(MEMBER_OUTBOX_SEED)).unwrap();
        }),
        // Another namespace's declaration seed.
        ("beside a Current3 seed", |path, _| {
            fs::write(
                path.join(CURRENT_DECLARATION_SEED),
                b"another declaration's seed",
            )
            .unwrap();
        }),
    ];
    for (index, (name, arrange)) in cases.into_iter().enumerate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let mut rng = ChaCha20Rng::seed_from_u64(0x5640 + index as u64);
        let key = member_cut(path, &shared, 242, &mut rng);
        arrange(path, &key);
        let before = recursive_fixture_files(path);
        let refused =
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared));
        assert!(refused.is_err(), "{name}");
        drop(refused);
        assert_eq!(recursive_fixture_files(path), before, "{name}");
    }

    // A used namespace that lost its marker and parent: the authenticated current catalog
    // still names ID5, so an unauthenticated seed beside it is not first-torn scratch.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5650);
    settled_three(path, &shared, &mut rng);
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared)).unwrap();
    owner.retry(&mut rng).unwrap();
    drop(owner.into_current_layout_store().unwrap());
    let root = path.join(ARCHIVE_BOOTSTRAP_ROOT);
    fs::remove_file(root.join(MEMBER_OUTBOX_MARKER)).unwrap();
    fs::remove_dir(root.join(MEMBER_OUTBOX_PARENT)).unwrap();
    fs::write(path.join(MEMBER_OUTBOX_SEED), [0x5c; 33]).unwrap();
    let before = recursive_fixture_files(path);
    let refused =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared));
    assert!(refused.is_err());
    drop(refused);
    assert_eq!(recursive_fixture_files(path), before);
}

/// A predecessor binary opens the unfenced claim window (cut 241) and runs an actual JOIN
/// write that is interrupted. A COMPLETE stage there is an authenticated ordinary successor
/// that does not name ID5, so it can never be this declaration's: the head disposes only the
/// claim, leaves the stage to the cold JOIN settlement, then declares namespace5 afresh. A
/// partial stage is refused and preserved, as the JOIN policy already requires. A ZERO-length
/// JOIN stage (fault 5) is the documented residual: beside the claim it is indistinguishable
/// from the claim's own fence and is unlinked with it. It holds no bytes; the current catalog
/// is unchanged, and the retry declares namespace5 over it with no JOIN row.
#[test]
fn a_predecessor_join_stage_in_the_claim_window_settles_and_only_the_claim_is_disposed() {
    use catcoms_mls::MlsDevice;
    for fault_at in [1u8, 4, 5] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let shared = pool();
        let seed = 0x5660 + u64::from(fault_at);
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let source = join_source(seed, &mut rng);
        let invite = source
            .owner
            .mint_owner_checkpoint_invite(&source.store, [76; 16], 100_000, vec![])
            .unwrap();
        member_cut(path, &shared, 241, &mut rng);
        let original = fs::read(path.join(Slot::Current.name())).unwrap();
        {
            // The predecessor (emulated: it does not know the member names) runs the JOIN.
            let _emulated = PreNamespace5::on();
            let mut opening = PreparedV2CurrentLayoutStore::open(
                path,
                SECRET,
                &mut rng,
                execution_policy(&shared),
            )
            .unwrap();
            opening.retry(&mut rng).unwrap();
            let ready = opening.into_current_layout_store().unwrap();
            let store = ready.checked_store().unwrap();
            store
                .configure_owner_evidence_budget(source.budget)
                .unwrap();
            let mut prepared = store
                .prepare_journalled_owner_checkpoint_join(
                    &source.store,
                    &source.owner,
                    MlsDevice::generate().unwrap(),
                    invite,
                    &mut rng,
                )
                .unwrap();
            store
                .critical_namespace_fault
                .store(fault_at, Ordering::SeqCst);
            assert!(prepared.save_pending(store, &mut rng).is_err());
            store.critical_namespace_fault.store(0, Ordering::SeqCst);
        }
        let stage = fs::read(path.join(Slot::Stage.name())).unwrap();
        assert_eq!(fs::read(path.join(MEMBER_OUTBOX_SEED)).unwrap(), b"");
        assert_eq!(fs::read(path.join(Slot::Current.name())).unwrap(), original);
        let before = recursive_fixture_files(path);
        let reopened = PreparedV2CurrentLayoutStore::open(
            path,
            SECRET,
            &mut NeverRng,
            member_execution_policy(&shared),
        );
        if fault_at == 4 {
            assert!(reopened.is_err());
            drop(reopened);
            assert_eq!(recursive_fixture_files(path), before);
            continue;
        }
        let mut cold = reopened.unwrap();
        assert!(!path.join(MEMBER_OUTBOX_SEED).exists());
        assert!(!path.join(Slot::Stage.name()).exists());
        if fault_at == 5 {
            assert!(stage.is_empty());
            assert_eq!(fs::read(path.join(Slot::Current.name())).unwrap(), original);
        } else {
            // The predecessor's exact stage is the selected catalog.
            assert_eq!(fs::read(path.join(Slot::Current.name())).unwrap(), stage);
        }
        cold.retry(&mut rng).unwrap();
        assert_declared(&cold, path, fault_at);
        let ready = cold.into_current_layout_store().unwrap();
        let store = ready.checked_store().unwrap();
        let (_, four, rows) =
            catalog_facts(store, &fs::read(path.join(Slot::Current.name())).unwrap());
        assert!(!four);
        assert_eq!(rows, usize::from(fault_at == 1));
    }
}

/// Fixture-only complete inventory: include directory names as well as file contents, so
/// refusing recovery cannot hide a newly created stage or a changed nested sentinel.
fn recursive_fixture_files(path: &Path) -> BTreeMap<std::path::PathBuf, Option<Vec<u8>>> {
    fn visit(
        base: &Path,
        directory: &Path,
        entries: &mut BTreeMap<std::path::PathBuf, Option<Vec<u8>>>,
    ) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(base).unwrap().to_path_buf();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                entries.insert(relative, None);
                visit(base, &path, entries);
            } else {
                assert!(kind.is_file(), "fixture must contain only regular files");
                entries.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(path, path, &mut entries);
    entries
}

#[test]
fn cold_complete_member_seed_with_nonempty_parent_refuses_before_creating_a_stage() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5540);
    settled_three(path, &shared, &mut rng);
    let original = fs::read(path.join(Slot::Current.name())).unwrap();
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared)).unwrap();
    // This cut leaves a genuine complete authenticated seed and marker, with the
    // original catalog still selected and the legacy refusal fence still empty.
    fault(&owner, 245);
    assert!(owner.retry(&mut rng).is_err());
    let key = private(&owner).keys.db_key().unwrap();
    let domain = private(&owner).chat_vault_domain().unwrap();
    let seed = fs::read(path.join(MEMBER_OUTBOX_SEED)).unwrap();
    let plain = member_codec::open_seed(&key, &seed).unwrap();
    let saved = member_codec::parse_seed(&key, &domain, &plain).unwrap();
    assert_eq!(saved.original, original.as_slice());
    assert_eq!(fs::read(path.join(Slot::Current.name())).unwrap(), original);
    let marker = fs::read(path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_MARKER)).unwrap();
    assert_eq!(saved.marker, marker.as_slice());
    drop(owner);

    assert!(fs::read(path.join(Slot::Stage.name())).unwrap().is_empty());
    fs::remove_file(path.join(Slot::Stage.name())).unwrap();
    let parent = path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_PARENT);
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("sentinel.bin"), b"preserve this existing child").unwrap();
    let before = recursive_fixture_files(path);

    // A complete seed cannot justify replay over an already used parent. In particular,
    // refuse before recreating the missing fixed stage, not only at the later write edge.
    let cold =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared));
    assert!(cold.is_err());
    drop(cold);
    assert_eq!(recursive_fixture_files(path), before);
    assert!(!path.join(Slot::Stage.name()).exists());
}

#[test]
fn settled_member_layout_without_a_seed_remounts_a_nonempty_parent_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5541);
    settled_three(path, &shared, &mut rng);
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared)).unwrap();
    owner.retry(&mut rng).unwrap();
    let (parent, _) = private(&owner).member_outbox_parent_data().unwrap();
    drop(owner.into_current_layout_store().unwrap());
    assert!(!path.join(MEMBER_OUTBOX_SEED).exists());
    assert!(!path.join(Slot::Stage.name()).exists());

    // Sentinel contents test declaration/mount preservation only; they are not a fabricated
    // authenticated outbox, request receipt or member-save authority.
    let child = parent.join("retained-fixture-child");
    fs::create_dir(&child).unwrap();
    fs::write(
        child.join("sentinel.bin"),
        b"keep retained context contents",
    )
    .unwrap();
    let before = recursive_fixture_files(path);

    let mut cold =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared))
            .unwrap();
    cold.retry(&mut NeverRng).unwrap();
    private(&cold)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    drop(cold.into_current_layout_store().unwrap());
    assert_eq!(recursive_fixture_files(path), before);
}

#[test]
fn an_id2_vault_reaches_three_then_five_in_one_member_configured_retry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5520);
    // ID2 only: no ID3, no member artifact. The configured member handler must not refuse
    // the first Current3 declaration; it declares 5 serially after 3.
    prepare_two(path, &shared, &mut rng);
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared)).unwrap();
    owner.retry(&mut rng).unwrap();
    let key = private(&owner).keys.db_key().unwrap();
    let domain = private(&owner).chat_vault_domain().unwrap();
    private(&owner).validate_mounted_current_layout().unwrap();
    private(&owner)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    drop(owner.into_current_layout_store().unwrap());
    let ordered = ids(&key, &domain, path);
    assert_eq!(&ordered[ordered.len() - 2..], &[3, 5]);
    assert!(!ordered.contains(&4));
}

#[test]
fn member_policy_must_use_an_independent_pool() {
    let shared = pool();
    let same = V2MemberOutboxPolicy::new(
        shared.clone(),
        256,
        256 * 1024 * 1024,
        1 << 20,
        1,
        member_codec::PERSISTENT_MINIMUM,
    )
    .unwrap();
    assert!(policy(&shared).with_member_outbox(same).is_err());
    // A pool too small for every simultaneous per-context invoice refuses at configuration.
    let small = CriticalCatalogReadPool::new(1024 * 1024).unwrap();
    assert!(V2MemberOutboxPolicy::new(
        small,
        256,
        256 * 1024 * 1024,
        1 << 20,
        1,
        member_codec::PERSISTENT_MINIMUM
    )
    .is_err());
}

#[test]
fn orphan_member_artifacts_without_a_declaration_are_preserved_and_refused() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5530);
    settled_three(path, &shared, &mut rng);
    // A parent directory with no seed and no ID5 is not a namespace to adopt.
    fs::create_dir(path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_PARENT)).unwrap();
    let snapshot = files(path);
    assert!(PreparedV2CurrentLayoutStore::open(
        path,
        SECRET,
        &mut NeverRng,
        member_policy(&shared)
    )
    .is_err());
    assert!(
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, policy(&shared)).is_err()
    );
    assert_eq!(files(path), snapshot);
    // So is a stray marker.
    fs::remove_dir(path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_PARENT)).unwrap();
    fs::write(
        path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_MARKER),
        [0x5a; 64],
    )
    .unwrap();
    let snapshot = files(path);
    assert!(PreparedV2CurrentLayoutStore::open(
        path,
        SECRET,
        &mut NeverRng,
        member_policy(&shared)
    )
    .is_err());
    assert_eq!(files(path), snapshot);
}

#[test]
fn the_store_member_outbox_mount_installs_once_from_the_declared_parent_and_bounds_contexts() {
    use crate::store::persistence::v2_member_outbox::{
        member_outbox_key, MemberOutboxContextData, MemberOutboxContextId, MemberOutboxContextSpec,
    };
    use catcoms_replication::chat::owner_facts::OwnerFactId;
    use catcoms_replication::chat::owner_header::OwnerTenureId;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5550);
    settled_three(path, &shared, &mut rng);
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared)).unwrap();
    owner.retry(&mut rng).unwrap();
    let layout = owner.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    // Not installed until first use; the install re-proves the declared parent. Later calls
    // (and the no-IO final-edge accessor) return the SAME table.
    assert!(store.installed_member_outbox_mount().unwrap().is_none());
    let mount = store.member_outbox_mount().unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &mount,
        &store.member_outbox_mount().unwrap()
    ));
    assert!(std::sync::Arc::ptr_eq(
        &mount,
        &store.installed_member_outbox_mount().unwrap().unwrap()
    ));
    // A real owner opens under the declared parent, in the namespace its context names.
    let layout_data = mount.capability().current_layout_data();
    let context = MemberOutboxContextData::from_spec(MemberOutboxContextSpec {
        vault: layout_data.vault,
        layout: layout_data.root,
        pin: [3; 32],
        profile: OwnerFactId::from_bytes([4; 32]),
        tenure: OwnerTenureId::from_bytes([5; 32]),
        group_nonce: [6; 32],
        device: [7; 32],
        group: b"member mount regression group",
    })
    .unwrap();
    let id = context.id();
    let key = member_outbox_key(store).unwrap();
    let cell = mount.cell(id).unwrap();
    assert!(std::sync::Arc::ptr_eq(&cell, &mount.cell(id).unwrap()));
    let outbox = mount.open_owner(store, context, &key, &mut rng).unwrap();
    assert!(outbox.context_id() == id);
    let parent = path.join(ARCHIVE_BOOTSTRAP_ROOT).join(MEMBER_OUTBOX_PARENT);
    let namespaces: Vec<String> = fs::read_dir(&parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(namespaces.len(), 1);
    assert!(namespaces[0].starts_with("mo-") && namespaces[0].len() == 67);
    // The configured policy bounds contexts per mount (one here): another context refuses.
    assert!(mount
        .cell(MemberOutboxContextId::from_bytes([9; 32]))
        .is_err());
    drop(outbox);
    drop(cell);
    drop(mount);
    drop(layout);
}

/// The actual JOIN lifecycle on a namespace5 vault with NO governance4: save, pause, resume,
/// migration/initial grant and the first arm each re-encode the whole catalog. Activation
/// supplies the network owner; `ensure_initial_grant` performs the migration. Every step
/// must carry the ID5 descriptor byte for byte, never introduce 4, and
/// leave the mounted marker/parent valid. A cold remount then writes nothing.
#[test]
fn actual_join_lifecycle_on_current3_preserves_five_exactly_without_four() {
    use crate::store::CheckpointJoinSubmissionStatus;
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{ManualClock, PeerId};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5570);
    let source = join_source(0x5570, &mut rng);
    let invite = source
        .owner
        .mint_owner_checkpoint_invite(&source.store, [73; 16], 100_000, vec![])
        .unwrap();
    let ready = five_with_join_execution(path, &shared, &source, &mut rng);
    let store = ready.checked_store().unwrap();
    let key = store.keys.db_key().unwrap();
    let domain = store.chat_vault_domain().unwrap();
    let current = || fs::read(path.join(Slot::Current.name())).unwrap();
    let (five, four, rows) = catalog_facts(store, &current());
    assert!(!four);
    assert_eq!(rows, 0);
    let declared = ids(&key, &domain, path);
    assert_eq!(&declared[declared.len() - 2..], &[3, 5]);
    // The first saved row adds the JOIN entry itself (ID1, sorted first); every other entry,
    // and their order, must stay exactly as declared through the whole lifecycle.
    assert!(!declared.contains(&catalog::JOIN_ID));
    let mut joined = vec![catalog::JOIN_ID];
    joined.extend_from_slice(&declared);

    let mut prepared = store
        .prepare_journalled_owner_checkpoint_join(
            &source.store,
            &source.owner,
            MlsDevice::generate().unwrap(),
            invite,
            &mut rng,
        )
        .unwrap();
    let check = |step: &str| {
        let (payload, four, rows) = catalog_facts(store, &current());
        assert_eq!(payload, five, "{step}: ID5 descriptor changed");
        assert!(!four, "{step}: governance4 appeared");
        assert_eq!(rows, 1, "{step}");
        assert_eq!(
            ids(&key, &domain, path),
            joined,
            "{step}: entry set or order"
        );
        store.validate_mounted_member_outbox_layout().unwrap();
        assert!(store.governance_layout.lock().unwrap().is_none(), "{step}");
    };
    prepared.save_pending(store, &mut rng).unwrap();
    check("save");
    prepared.pause_pending(store, &mut rng).unwrap();
    check("pause");
    prepared.resume_pending(store, &mut rng).unwrap();
    check("resume");
    let activated = prepared
        .activate_original_transport(
            store,
            source.hub.join(PeerId::from_u64(0x5571)),
            PeerId::from_u64(0x5570),
        )
        .unwrap();
    let mut network = activated.into_network_execution(store).unwrap();
    network.ensure_initial_grant(store, &mut rng).unwrap();
    check("migration and initial grant");
    assert_eq!(
        network
            .ensure_submission_armed(store, &mut rng, Arc::new(ManualClock::new(1_000)))
            .unwrap(),
        CheckpointJoinSubmissionStatus::Armed
    );
    check("first arm");
    let updated = current();
    let updated_plain = Zeroizing::new(unseal(&key, &unframe(&updated).unwrap()).unwrap());
    assert_eq!(
        store
            .parse_configured_join_catalog(&updated_plain)
            .unwrap()
            .join_version,
        Some(super::super::super::pending_join::execution_payload::VERSION)
    );
    drop(network);
    drop(ready);

    // The same JOIN-executing opener WITHOUT the member handler refuses before any cleaner.
    let snapshot = files(path);
    assert!(PreparedV2CurrentLayoutStore::open(
        path,
        SECRET,
        &mut NeverRng,
        execution_policy(&shared)
    )
    .is_err());
    assert_eq!(files(path), snapshot);
    // With it, a cold remount draws no RNG and writes nothing.
    let mut cold = PreparedV2CurrentLayoutStore::open(
        path,
        SECRET,
        &mut NeverRng,
        member_execution_policy(&shared),
    )
    .unwrap();
    cold.retry(&mut NeverRng).unwrap();
    private(&cold)
        .validate_mounted_member_outbox_layout()
        .unwrap();
    drop(cold.into_current_layout_store().unwrap());
    assert_eq!(current(), updated);
    assert_eq!(files(path), snapshot);
}

/// Real saved JOIN and execution transitions after native7 installation. A codec-only test
/// cannot establish that the native pre-cleaner and the actual writer agree on preservation.
#[test]
fn native_registry_actual_join_lifecycle_preserves_seven_and_cold_reopens() {
    use crate::store::{
        CheckpointJoinSubmissionStatus, NativeRegistryInstallation, NativeRegistryPolicy,
        NativeRegistryStore,
    };
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{ManualClock, PeerId};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x77b0);
    let source = join_source(0x77b0, &mut rng);
    let invite = source
        .owner
        .mint_owner_checkpoint_invite(&source.store, [77; 16], 100_000, vec![])
        .unwrap();
    drop(five_with_join_execution(path, &shared, &source, &mut rng));
    let native_policy = || {
        NativeRegistryPolicy::new(
            CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
            NativeRegistryPolicy::required_persistent_capacity(),
        )
        .unwrap()
    };
    let mut installation = NativeRegistryInstallation::prepare(
        path,
        SECRET,
        &mut rng,
        member_execution_policy(&shared),
        native_policy(),
    )
    .unwrap();
    installation.retry().unwrap();
    let native = installation.into_native_registry().unwrap();
    let store = native.actual_store_for_test().unwrap();
    // This is a newly opened physical mount; the old fixture's trusted runtime budget
    // was deliberately not persisted or inherited through decoded registry DATA.
    store
        .configure_owner_evidence_budget(source.budget)
        .unwrap();
    let key = store.keys.db_key().unwrap();
    let domain = store.chat_vault_domain().unwrap();
    let namespaces = || {
        let raw = fs::read(path.join(Slot::Current.name())).unwrap();
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw).unwrap()).unwrap());
        let parsed = store.parse_configured_join_catalog(&plain).unwrap();
        (
            parsed.member_outbox_payload.unwrap().to_vec(),
            parsed.native_registry_payload.unwrap().to_vec(),
            parsed.count,
        )
    };
    let (five, seven, count) = namespaces();
    assert_eq!(count, 0);
    let check = || {
        let (next_five, next_seven, count) = namespaces();
        assert_eq!(next_five, five);
        assert_eq!(next_seven, seven);
        assert_eq!(count, 1);
        assert!(!ids(&key, &domain, path).contains(&4));
        store.validate_mounted_member_outbox_layout().unwrap();
    };
    let mut joined = store
        .prepare_journalled_owner_checkpoint_join(
            &source.store,
            &source.owner,
            MlsDevice::generate().unwrap(),
            invite,
            &mut rng,
        )
        .unwrap();
    joined.save_pending(store, &mut rng).unwrap();
    check();
    joined.pause_pending(store, &mut rng).unwrap();
    check();
    joined.resume_pending(store, &mut rng).unwrap();
    check();
    let activated = joined
        .activate_original_transport(
            store,
            source.hub.join(PeerId::from_u64(0x77b1)),
            PeerId::from_u64(0x77b0),
        )
        .unwrap();
    let mut network = activated.into_network_execution(store).unwrap();
    network.ensure_initial_grant(store, &mut rng).unwrap();
    check();
    assert_eq!(
        network
            .ensure_submission_armed(store, &mut rng, Arc::new(ManualClock::new(1_000)))
            .unwrap(),
        CheckpointJoinSubmissionStatus::Armed
    );
    check();
    drop(network);
    drop(native);
    let before = files(path);
    let cold = NativeRegistryStore::open(
        path,
        SECRET,
        &mut NeverRng,
        member_execution_policy(&shared),
        native_policy(),
    )
    .unwrap();
    assert_eq!(cold.row_count_data().unwrap(), 0);
    assert_eq!(files(path), before);
    let raw = fs::read(path.join(Slot::Current.name())).unwrap();
    let plain = Zeroizing::new(unseal(&key, &unframe(&raw).unwrap()).unwrap());
    let parsed = cold
        .actual_store_for_test()
        .unwrap()
        .parse_configured_join_catalog(&plain)
        .unwrap();
    assert_eq!(parsed.native_registry_payload.unwrap(), seven);
    assert_eq!(parsed.member_outbox_payload.unwrap(), five);
}

/// A genuine index2 mount must preserve its explicit namespace version through
/// saved JOIN, pause/resume, execution migration/arming and a real cold opener.
#[test]
fn native_index2_actual_join_lifecycle_preserves_version_and_cold_reopens() {
    use crate::store::{
        CheckpointJoinSubmissionStatus, NativeIndexPolicy, NativeIndexStore, NativeIndexUpgrade,
        NativeRegistryInstallation, NativeRegistryPolicy,
    };
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{ManualClock, PeerId};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x77b0);
    let source = join_source(0x77b0, &mut rng);
    let invite = source
        .owner
        .mint_owner_checkpoint_invite(&source.store, [77; 16], 100_000, vec![])
        .unwrap();
    drop(five_with_join_execution(path, &shared, &source, &mut rng));
    let native_policy = || {
        NativeRegistryPolicy::new(
            CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
            NativeRegistryPolicy::required_persistent_capacity(),
        )
        .unwrap()
    };
    let index_policy = || {
        NativeIndexPolicy::new(
            CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
            NativeIndexPolicy::required_persistent_capacity(),
        )
        .unwrap()
    };
    let mut installation = NativeRegistryInstallation::prepare(
        path,
        SECRET,
        &mut rng,
        member_execution_policy(&shared),
        native_policy(),
    )
    .unwrap();
    installation.retry().unwrap();
    let mut upgrade = NativeIndexUpgrade::prepare(
        installation.into_native_registry().unwrap(),
        index_policy(),
        &mut rng,
    )
    .unwrap();
    upgrade.retry().unwrap();
    let native = upgrade.into_index_store().unwrap();
    let store = native.actual_store_for_test().unwrap();
    // This is a newly opened physical mount; the old fixture's trusted runtime budget
    // was deliberately not persisted or inherited through decoded registry DATA.
    store
        .configure_owner_evidence_budget(source.budget)
        .unwrap();
    let key = store.keys.db_key().unwrap();
    let domain = store.chat_vault_domain().unwrap();
    let namespaces = || {
        let raw = fs::read(path.join(Slot::Current.name())).unwrap();
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw).unwrap()).unwrap());
        let parsed = store.parse_configured_join_catalog(&plain).unwrap();
        assert_eq!(parsed.native_registry_version, Some(2));
        (
            parsed.member_outbox_payload.unwrap().to_vec(),
            parsed.native_registry_payload.unwrap().to_vec(),
            parsed.count,
        )
    };
    let (five, seven, count) = namespaces();
    assert_eq!(count, 0);
    let check = || {
        let (next_five, next_seven, count) = namespaces();
        assert_eq!(next_five, five);
        assert_eq!(next_seven, seven);
        assert_eq!(count, 1);
        assert!(!ids(&key, &domain, path).contains(&4));
        store.validate_mounted_member_outbox_layout().unwrap();
    };
    let mut joined = store
        .prepare_journalled_owner_checkpoint_join(
            &source.store,
            &source.owner,
            MlsDevice::generate().unwrap(),
            invite,
            &mut rng,
        )
        .unwrap();
    joined.save_pending(store, &mut rng).unwrap();
    check();
    joined.pause_pending(store, &mut rng).unwrap();
    check();
    joined.resume_pending(store, &mut rng).unwrap();
    check();
    let activated = joined
        .activate_original_transport(
            store,
            source.hub.join(PeerId::from_u64(0x77b1)),
            PeerId::from_u64(0x77b0),
        )
        .unwrap();
    let mut network = activated.into_network_execution(store).unwrap();
    network.ensure_initial_grant(store, &mut rng).unwrap();
    check();
    assert_eq!(
        network
            .ensure_submission_armed(store, &mut rng, Arc::new(ManualClock::new(1_000)))
            .unwrap(),
        CheckpointJoinSubmissionStatus::Armed
    );
    check();
    drop(network);
    drop(native);
    let before = files(path);
    let cold = NativeIndexStore::open(
        path,
        SECRET,
        &mut NeverRng,
        member_execution_policy(&shared),
        index_policy(),
    )
    .unwrap();
    assert_eq!(cold.row_count_data().unwrap(), 0);
    assert_eq!(files(path), before);
    let raw = fs::read(path.join(Slot::Current.name())).unwrap();
    let plain = Zeroizing::new(unseal(&key, &unframe(&raw).unwrap()).unwrap());
    let parsed = cold
        .actual_store_for_test()
        .unwrap()
        .parse_configured_join_catalog(&plain)
        .unwrap();
    assert_eq!(parsed.native_registry_version, Some(2));
    assert_eq!(parsed.native_registry_payload.unwrap(), seven);
    assert_eq!(parsed.member_outbox_payload.unwrap(), five);
}

/// A paused row is retained JOIN work, not a vacant startup catalog. The native
/// preparation must preserve both active and paused genuine sources unchanged.
#[test]
fn native_registry_preparation_refuses_real_active_and_paused_join_rows() {
    use crate::store::{NativeRegistryPreparationPolicy, PreparedNativeRegistryMigration};
    use catcoms_mls::MlsDevice;
    for paused in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x7790 + u64::from(paused));
        let source = join_source(0x7790, &mut rng);
        let invite = source
            .owner
            .mint_owner_checkpoint_invite(&source.store, [74; 16], 100_000, vec![])
            .unwrap();
        let ready = five_with_join_execution(path, &shared, &source, &mut rng);
        let store = ready.checked_store().unwrap();
        let mut joined = store
            .prepare_journalled_owner_checkpoint_join(
                &source.store,
                &source.owner,
                MlsDevice::generate().unwrap(),
                invite,
                &mut rng,
            )
            .unwrap();
        joined.save_pending(store, &mut rng).unwrap();
        if paused {
            joined.pause_pending(store, &mut rng).unwrap();
        }
        let current = fs::read(path.join(Slot::Current.name())).unwrap();
        assert_eq!(catalog_facts(store, &current).2, 1);
        drop(joined);
        drop(ready);
        let native = NativeRegistryPreparationPolicy::new(
            CriticalCatalogReadPool::new(NativeRegistryPreparationPolicy::required_capacity())
                .unwrap(),
        )
        .unwrap();
        assert!(PreparedNativeRegistryMigration::open(
            path,
            SECRET,
            &mut NeverRng,
            member_execution_policy(&shared),
            native
        )
        .is_err());
        assert_eq!(fs::read(path.join(Slot::Current.name())).unwrap(), current);
        assert!(!path
            .join("chat-critical-namespaces.native-registry.seed")
            .exists());
    }
}

/// An ordinary JOIN write interrupted on a namespace5 vault (no governance4), then a cold open.
/// The JOIN writer's own cuts: 1 leaves a complete synced stage before the rename (the cold
/// JOIN settlement promotes it), 2 fails the directory barrier AFTER the rename (no stage; the
/// candidate is already selected), 4 leaves a 17-byte prefix and 5 a zero-length stage (both
/// preserved and refused by the JOIN policy). The member inspector runs FIRST and must neither
/// refuse a complete stage that carries ID5 exactly, nor act on (or clean) a partial one.
#[test]
fn an_interrupted_join_stage_on_a_five_vault_settles_cold_or_stays_preserved() {
    use catcoms_mls::MlsDevice;
    for fault in [1u8, 2, 4, 5] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let shared = pool();
        let seed = 0x5580 + u64::from(fault);
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let source = join_source(seed, &mut rng);
        let invite = source
            .owner
            .mint_owner_checkpoint_invite(&source.store, [74; 16], 100_000, vec![])
            .unwrap();
        let ready = five_with_join_execution(path, &shared, &source, &mut rng);
        let store = ready.checked_store().unwrap();
        let original = fs::read(path.join(Slot::Current.name())).unwrap();
        let (five, _, rows) = catalog_facts(store, &original);
        let mut prepared = store
            .prepare_journalled_owner_checkpoint_join(
                &source.store,
                &source.owner,
                MlsDevice::generate().unwrap(),
                invite,
                &mut rng,
            )
            .unwrap();
        store
            .critical_namespace_fault
            .store(fault, Ordering::SeqCst);
        assert!(
            prepared.save_pending(store, &mut rng).is_err(),
            "fault {fault}"
        );
        store.critical_namespace_fault.store(0, Ordering::SeqCst);
        drop(prepared);
        drop(ready);
        let stage_path = path.join(Slot::Stage.name());
        let stage = fs::read(&stage_path).ok();
        let selected = fs::read(path.join(Slot::Current.name())).unwrap();
        let complete = fault == 1 || fault == 2;
        match fault {
            1 => assert!(stage.as_ref().is_some_and(|s| !s.is_empty()) && selected == original),
            2 => assert!(stage.is_none() && selected != original),
            4 => assert!(stage.as_ref().is_some_and(|s| s.len() == 17) && selected == original),
            _ => assert!(stage.as_ref().is_some_and(Vec::is_empty) && selected == original),
        }
        // The exact bytes a settled cold open must select: the complete stage, or the already
        // renamed candidate.
        let candidate = stage.clone().filter(|_| fault == 1).unwrap_or(selected);
        let snapshot = files(path);

        // No opener without the handler acts on the stage: it refuses before any cleaner.
        assert!(
            PreparedV2CurrentLayoutStore::open(
                path,
                SECRET,
                &mut NeverRng,
                execution_policy(&shared)
            )
            .is_err(),
            "fault {fault}"
        );
        assert_eq!(files(path), snapshot, "fault {fault}");
        refuses_as_pre_namespace5(path, &shared, fault);
        assert_eq!(files(path), snapshot, "fault {fault}");

        let reopened = PreparedV2CurrentLayoutStore::open(
            path,
            SECRET,
            &mut NeverRng,
            member_execution_policy(&shared),
        );
        if complete {
            let mut cold = reopened.unwrap_or_else(|error| panic!("fault {fault}: {error:?}"));
            cold.retry(&mut NeverRng).unwrap();
            let ready = cold.into_current_layout_store().unwrap();
            let store = ready.checked_store().unwrap();
            store.validate_mounted_member_outbox_layout().unwrap();
            assert!(!stage_path.exists(), "fault {fault}");
            let promoted = fs::read(path.join(Slot::Current.name())).unwrap();
            assert_eq!(
                promoted, candidate,
                "fault {fault}: the exact candidate is selected"
            );
            let (payload, four, settled) = catalog_facts(store, &promoted);
            assert_eq!(payload, five, "fault {fault}");
            assert!(!four, "fault {fault}");
            assert_eq!(settled, rows + 1, "fault {fault}");
            drop(ready);
        } else {
            assert!(reopened.is_err(), "fault {fault}");
            drop(reopened);
            assert_eq!(files(path), snapshot, "fault {fault}");
        }
    }
}

/// A complete, AUTHENTICATED stage (sealed with the vault key) that is otherwise an ordinary
/// successor but drops ID5, or carries a changed ID5 descriptor, is not this namespace's
/// successor: the member-configured opener preserves it and refuses. Negative fixtures only.
#[test]
fn an_authenticated_stage_that_drops_or_changes_five_is_preserved_and_refused() {
    use catcoms_wire::Encoder;
    for change in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let shared = pool();
        let mut rng = ChaCha20Rng::seed_from_u64(0x5590 + u64::from(change));
        settled_three(path, &shared, &mut rng);
        let mut owner =
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared))
                .unwrap();
        owner.retry(&mut rng).unwrap();
        let store = private(&owner);
        let key = store.keys.db_key().unwrap();
        let domain = store.chat_vault_domain().unwrap();
        drop(owner.into_current_layout_store().unwrap());

        let raw = fs::read(path.join(Slot::Current.name())).unwrap();
        let plain = unseal(&key, &unframe(&raw).unwrap()).unwrap();
        let parsed = outer::parse(&plain, &domain, outer::MAX_ENVELOPE_PLAIN).unwrap();
        let mut entries: Vec<(u32, u32, Vec<u8>)> = (0..parsed.count)
            .map(|index| {
                let entry = parsed.entry(index).unwrap();
                (entry.id, entry.version, entry.payload.to_vec())
            })
            .collect();
        let encode = |generation: u64, parent: &[u8; 32], entries: &[(u32, u32, Vec<u8>)]| {
            let mut encoder = Encoder::new();
            encoder.put_str(outer::TAG_V2).unwrap();
            encoder.put_bytes(&domain).unwrap();
            encoder.put_u64(generation);
            encoder.put_bytes(parent).unwrap();
            encoder.put_u32(entries.len() as u32);
            for (id, version, payload) in entries {
                encoder.put_u32(*id).put_u32(*version);
                encoder.put_bytes(payload).unwrap();
            }
            encoder.finish()
        };
        // Positive control: this hand encoder reproduces the actual selected catalog byte for
        // byte, so the refusals below come from the changed ID5 entry, not from bad framing.
        assert_eq!(
            encode(parsed.generation, &parsed.parent.unwrap(), &entries),
            plain
        );
        if change {
            let five = entries
                .iter_mut()
                .find(|entry| entry.0 == member_ns::NAMESPACE_ID)
                .unwrap();
            let last = five.2.len() - 1;
            five.2[last] ^= 1;
        } else {
            entries.retain(|entry| entry.0 != member_ns::NAMESPACE_ID);
        }
        let successor = encode(
            parsed.generation + 1,
            &outer::catalog_digest(&plain),
            &entries,
        );
        let sealed = catcoms_crypto::keystore::seal(&key, &successor, &mut rng).unwrap();
        fs::write(path.join(Slot::Stage.name()), crate::store::frame(&sealed)).unwrap();
        let snapshot = files(path);
        assert!(
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared))
                .is_err(),
            "change {change}"
        );
        assert_eq!(files(path), snapshot, "change {change}");
        assert!(
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, policy(&shared))
                .is_err(),
            "change {change}"
        );
        assert_eq!(files(path), snapshot, "change {change}");
    }
}

#[test]
fn an_opener_without_the_member_handler_installs_no_member_outbox_mount() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5551);
    settled_three(path, &shared, &mut rng);
    let mut owner = open(path, &shared, &mut rng);
    owner.retry(&mut rng).unwrap();
    let layout = owner.into_current_layout_store().unwrap();
    let store = layout.checked_store().unwrap();
    assert!(store.member_outbox_mount().is_err());
    assert!(store.installed_member_outbox_mount().unwrap().is_none());
    assert!(!path
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(MEMBER_OUTBOX_PARENT)
        .exists());
}
