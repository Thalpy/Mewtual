//! Head side of the real predecessor-binary gate.
//!
//! The focused CI lane (`.github/scripts/check-chat-v2-member-save.py`) runs, in order:
//! 1. [`generate_namespace5_predecessor_fixtures`]: writes vault directories and a
//!    `manifest.txt` (one fixture name per line) under `MEWTUAL_NAMESPACE5_FIXTURES`;
//! 2. `.github/predecessor/namespace5_gate.rs` inside an unmodified `git worktree` of
//!    `ce402364` (the last commit before namespace5): it owns each fixture's expected
//!    predecessor verdicts, and writes the trees its openers produce from the may-open
//!    fixtures under `after/`;
//! 3. [`verify_namespace5_predecessor_round_trip`]: the head reopens those trees with the
//!    member handler and must abandon the claim or torn seed, then declare namespace5.
//!
//! Both cases are ignored and live outside every focused cohort filter (`member_outbox`,
//! `v2_member_outbox`, `member_save`), so they never enter an exact count. They assert only
//! that each fixture reached its intended state and that the round trip recovers; the
//! predecessor's verdicts belong to the harness. Vaults use this test module's `SECRET`, and
//! the harness repeats it.
use super::member_outbox::{
    five_with_join_execution, ids, join_source, member_cut, member_policy, settled_three,
};
use super::*;
use crate::store::critical_namespaces::catalog_envelope as outer;
use crate::store::critical_namespaces::governance_layout::{
    codec::PERSISTENT_MINIMUM as GOVERNANCE_MINIMUM, PreparedV2GovernanceLayoutStore,
    V2GovernanceLayoutPolicy, GOVERNANCE_MARKER, GOVERNANCE_SEED,
};
use crate::store::critical_namespaces::member_outbox_layout::{
    arm_member_cold_fault_for_test, take_member_cold_fault_fired_for_test, MEMBER_OUTBOX_MARKER,
    MEMBER_OUTBOX_PARENT, MEMBER_OUTBOX_SEED,
};
use std::path::PathBuf;

/// Pinned in the generator, the harness and the CI script, so a changed set is deliberate.
const EXPECTED_FIXTURES: usize = 23;
/// The may-open fixtures, each carried through the predecessor's Current3 and governance
/// openers in full (open, retry, ready store) and kept for the head's round trip.
const ROUND_TRIP: [&str; 4] = [
    "cut241-current3",
    "cut241-governance",
    "d227-current3",
    "d227-governance",
];

fn fixture_root() -> PathBuf {
    PathBuf::from(
        std::env::var_os("MEWTUAL_NAMESPACE5_FIXTURES")
            .expect("MEWTUAL_NAMESPACE5_FIXTURES must name the fixture root"),
    )
}

/// The vault database key and chat vault domain, for reading the selected catalog.
fn keys(store: &ServerStore) -> ([u8; 32], [u8; 32]) {
    (
        store.keys.db_key().unwrap(),
        store.chat_vault_domain().unwrap(),
    )
}

/// The exact payload of catalog entry `id` in the selected catalog.
fn entry_payload(key: &[u8; 32], domain: &[u8; 32], path: &Path, id: u32) -> Vec<u8> {
    let raw = fs::read(path.join(Slot::Current.name())).unwrap();
    let plain = unseal(key, &unframe(&raw).unwrap()).unwrap();
    let parsed = outer::parse(&plain, domain, outer::MAX_ENVELOPE_PLAIN).unwrap();
    (0..parsed.count)
        .map(|index| parsed.entry(index).unwrap())
        .find(|entry| entry.id == id)
        .unwrap()
        .payload
        .to_vec()
}

fn governance(pool: &CriticalCatalogReadPool, member: bool) -> V2GovernanceLayoutPolicy {
    let current = if member {
        member_policy(pool)
    } else {
        policy(pool)
    };
    V2GovernanceLayoutPolicy::new(current, GOVERNANCE_MINIMUM).unwrap()
}

/// One governance-opener retry: declares 4 when absent and, with the member handler, then 5
/// when absent (inserting 4 before an existing 5).
fn governance_retry(
    path: &Path,
    pool: &CriticalCatalogReadPool,
    member: bool,
    rng: &mut ChaCha20Rng,
) {
    let mut owner =
        PreparedV2GovernanceLayoutStore::open(path, SECRET, rng, governance(pool, member)).unwrap();
    owner.retry(rng).unwrap();
    drop(owner.into_governance_layout_store().unwrap());
}

fn declare_five(path: &Path, pool: &CriticalCatalogReadPool, rng: &mut ChaCha20Rng) {
    let mut owner =
        PreparedV2CurrentLayoutStore::open(path, SECRET, rng, member_policy(pool)).unwrap();
    owner.retry(rng).unwrap();
    drop(owner.into_current_layout_store().unwrap());
}

/// A settled namespace5 vault whose JOIN catalog the actual JOIN producer has written:
/// `fault` 0 runs the whole lifecycle to the first arm; 1 leaves a complete stage, 4 a prefix.
fn join_state(path: &Path, pool: &CriticalCatalogReadPool, fault: u8, seed: u64) {
    use crate::store::CheckpointJoinSubmissionStatus;
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{ManualClock, PeerId};
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let source = join_source(seed, &mut rng);
    let invite = source
        .owner
        .mint_owner_checkpoint_invite(&source.store, [75; 16], 100_000, vec![])
        .unwrap();
    let ready = five_with_join_execution(path, pool, &source, &mut rng);
    let store = ready.checked_store().unwrap();
    let mut prepared = store
        .prepare_journalled_owner_checkpoint_join(
            &source.store,
            &source.owner,
            MlsDevice::generate().unwrap(),
            invite,
            &mut rng,
        )
        .unwrap();
    if fault != 0 {
        store
            .critical_namespace_fault
            .store(fault, Ordering::SeqCst);
        assert!(prepared.save_pending(store, &mut rng).is_err());
        store.critical_namespace_fault.store(0, Ordering::SeqCst);
        return;
    }
    prepared.save_pending(store, &mut rng).unwrap();
    prepared.pause_pending(store, &mut rng).unwrap();
    prepared.resume_pending(store, &mut rng).unwrap();
    let activated = prepared
        .activate_original_transport(
            store,
            source.hub.join(PeerId::from_u64(seed + 1)),
            PeerId::from_u64(seed),
        )
        .unwrap();
    let mut network = activated.into_network_execution(store).unwrap();
    assert_eq!(
        network
            .ensure_submission_armed(store, &mut rng, Arc::new(ManualClock::new(1_000)))
            .unwrap(),
        CheckpointJoinSubmissionStatus::Armed
    );
}

#[test]
#[ignore = "writes predecessor-gate vaults; run by the focused CI lane"]
fn generate_namespace5_predecessor_fixtures() {
    let root = fixture_root();
    fs::create_dir_all(&root).unwrap();
    assert!(
        fs::read_dir(&root).unwrap().next().is_none(),
        "the fixture root must start empty"
    );
    let shared = pool();
    let mut manifest = String::new();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5e00);
    let mut fixture = |name: &str| {
        let path = root.join(name);
        fs::create_dir(&path).unwrap();
        manifest.push_str(name);
        manifest.push('\n');
        path
    };

    // Controls: head-written vaults WITHOUT namespace5 that the predecessor must open, so no
    // refusal below can be vacuous (a copy, path, passphrase or format problem fails here).
    let path = fixture("c3");
    settled_three(&path, &shared, &mut rng);
    let path = fixture("c34");
    settled_three(&path, &shared, &mut rng);
    governance_retry(&path, &shared, false, &mut rng);

    // Settled namespace5, alone and with governance4 in either declaration order, and a USED
    // namespace (its parent holds a context directory).
    let path = fixture("s35");
    settled_three(&path, &shared, &mut rng);
    declare_five(&path, &shared, &mut rng);
    let path = fixture("s345");
    settled_three(&path, &shared, &mut rng);
    governance_retry(&path, &shared, false, &mut rng);
    governance_retry(&path, &shared, true, &mut rng);
    let path = fixture("s354");
    settled_three(&path, &shared, &mut rng);
    declare_five(&path, &shared, &mut rng);
    governance_retry(&path, &shared, true, &mut rng);
    let path = fixture("s35-used");
    settled_three(&path, &shared, &mut rng);
    declare_five(&path, &shared, &mut rng);
    let context = path
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(MEMBER_OUTBOX_PARENT)
        .join("mo-fixture-context");
    fs::create_dir(&context).unwrap();
    fs::write(context.join("sentinel.bin"), b"used namespace contents").unwrap();

    // Every warm declaration cut on the Current3 opener. The claim-only cut (241: a zero-byte
    // reservation at the seed name, no fence yet) is the one with no content-bearing output.
    for cut in [240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 255] {
        let path = fixture(&format!("cut{cut}"));
        member_cut(&path, &shared, cut, &mut rng);
        let stage = path.join(Slot::Stage.name());
        match cut {
            241 => {
                assert_eq!(fs::read(path.join(MEMBER_OUTBOX_SEED)).unwrap(), b"");
                assert!(!stage.exists());
            }
            240 | 242 => assert_eq!(fs::read(&stage).unwrap(), b""),
            _ => {}
        }
    }

    // The cold first-torn disposal interrupted after its fence unlink and barrier (227): the
    // torn seed with no fence, deliberately unfenced.
    let path = fixture("d227");
    member_cut(&path, &shared, 242, &mut rng);
    arm_member_cold_fault_for_test(227);
    assert!(PreparedV2CurrentLayoutStore::open(
        &path,
        SECRET,
        &mut NeverRng,
        member_policy(&shared)
    )
    .is_err());
    assert!(take_member_cold_fault_fired_for_test());
    assert!(!path.join(Slot::Stage.name()).exists());
    assert!(!fs::read(path.join(MEMBER_OUTBOX_SEED)).unwrap().is_empty());

    // Verify by breaking it: cut 243 with its fence REMOVED. The predecessor must then open it,
    // proving that cut 243 is refused because of the fence, not for an unrelated reason.
    let path = fixture("cut243-unfenced");
    member_cut(&path, &shared, 243, &mut rng);
    fs::remove_file(path.join(Slot::Stage.name())).unwrap();

    // The actual JOIN producer on a namespace5 vault: the whole lifecycle to the first arm,
    // an interrupted complete stage and an interrupted 17-byte stage.
    join_state(&fixture("join-armed"), &shared, 0, 0x5e10);
    join_state(&fixture("join-stage"), &shared, 1, 0x5e20);
    join_state(&fixture("join-prefix"), &shared, 4, 0x5e30);

    let count = manifest.lines().count();
    assert_eq!(count, EXPECTED_FIXTURES);
    fs::write(root.join("manifest.txt"), &manifest).unwrap();
    println!("namespace5 predecessor fixtures: {count} written");
}

/// The head reopens every tree the predecessor's full Current3 and governance runs produced
/// from a may-open fixture. A predecessor may have opened, retried and even declared 4 over
/// the claim or torn seed; the head must still abandon that seed (no RNG at open) and declare
/// namespace5 afresh, preserving whatever the predecessor settled.
#[test]
#[ignore = "reopens predecessor-gate outputs; run by the focused CI lane"]
fn verify_namespace5_predecessor_round_trip() {
    let after = fixture_root().join("after");
    let mut found: Vec<String> = fs::read_dir(&after)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    let mut expected: Vec<String> = ROUND_TRIP.iter().map(ToString::to_string).collect();
    expected.sort();
    assert_eq!(found, expected, "the predecessor round-trip set changed");
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5e40);
    for name in ROUND_TRIP {
        let path = after.join(name);
        assert!(path.join(MEMBER_OUTBOX_SEED).exists(), "{name}: seed lost");
        if name.ends_with("-current3") {
            let mut cold = PreparedV2CurrentLayoutStore::open(
                &path,
                SECRET,
                &mut NeverRng,
                member_policy(&shared),
            )
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            assert!(!path.join(MEMBER_OUTBOX_SEED).exists(), "{name}");
            assert!(!path.join(Slot::Stage.name()).exists(), "{name}");
            let (key, domain) = keys(private(&cold));
            // The predecessor left a settled Current3 catalog without 4 or 5.
            let before = ids(&key, &domain, &path);
            assert_eq!(before.last(), Some(&3), "{name}: {before:?}");
            cold.retry(&mut rng).unwrap();
            private(&cold)
                .validate_mounted_member_outbox_layout()
                .unwrap();
            let mut declared = before;
            declared.push(5);
            assert_eq!(ids(&key, &domain, &path), declared, "{name}");
            drop(cold.into_current_layout_store().unwrap());
        } else {
            // The predecessor's governance retry declared 4 over the claim or torn seed.
            assert!(
                path.join(ARCHIVE_BOOTSTRAP_ROOT)
                    .join(GOVERNANCE_MARKER)
                    .is_file(),
                "{name}: the predecessor did not declare 4"
            );
            let mut cold = PreparedV2GovernanceLayoutStore::open(
                &path,
                SECRET,
                &mut NeverRng,
                governance(&shared, true),
            )
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            assert!(!path.join(MEMBER_OUTBOX_SEED).exists(), "{name}");
            let (key, domain) = keys(cold.store_for_test());
            // The predecessor's settled 3 then 4; its exact ID4 entry must survive the retry.
            let before = ids(&key, &domain, &path);
            assert_eq!(&before[before.len() - 2..], &[3, 4], "{name}: {before:?}");
            let four = entry_payload(&key, &domain, &path, 4);
            cold.retry(&mut rng).unwrap();
            cold.store_for_test()
                .validate_mounted_governance_layout()
                .unwrap();
            cold.store_for_test()
                .validate_mounted_member_outbox_layout()
                .unwrap();
            let mut declared = before;
            declared.push(5);
            assert_eq!(ids(&key, &domain, &path), declared, "{name}");
            assert_eq!(entry_payload(&key, &domain, &path, 4), four, "{name}");
            drop(cold.into_governance_layout_store().unwrap());
            assert!(!path.join(GOVERNANCE_SEED).exists(), "{name}");
        }
        let root = path.join(ARCHIVE_BOOTSTRAP_ROOT);
        assert!(root.join(MEMBER_OUTBOX_MARKER).is_file(), "{name}");
        assert!(root.join(MEMBER_OUTBOX_PARENT).is_dir(), "{name}");
        assert!(!path.join(MEMBER_OUTBOX_SEED).exists(), "{name}");
        assert!(!path.join(Slot::Stage.name()).exists(), "{name}");
    }
    println!(
        "namespace5 predecessor round trip: {} verified",
        ROUND_TRIP.len()
    );
}
