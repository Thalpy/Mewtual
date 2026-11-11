//! Namespace5 (member outbox layout) declared through the Current3 opener, with no
//! governance4. Real files and the real vault; no member source, save or governance authority
//! is exercised or granted. Primary owns serial compiled validation; these are UNRUN here.
use super::super::super::member_outbox_layout::{
    codec as member_codec, MEMBER_OUTBOX_MARKER, MEMBER_OUTBOX_PARENT, MEMBER_OUTBOX_SEED,
};
use super::super::super::{catalog_envelope as outer, member_outbox_namespace_codec as member_ns};
use super::*;
use std::collections::BTreeMap;

fn member_policy(pool: &CriticalCatalogReadPool) -> V2CurrentLayoutPolicy {
    let member_pool = CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap();
    policy(pool)
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

fn settled_three(path: &Path, pool: &CriticalCatalogReadPool, rng: &mut ChaCha20Rng) {
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

fn ids(key: &[u8; 32], domain: &[u8; 32], path: &Path) -> Vec<u32> {
    let raw = fs::read(path.join(Slot::Current.name())).unwrap();
    let plain = unseal(key, &unframe(&raw).unwrap()).unwrap();
    let parsed = outer::parse(&plain, domain, outer::MAX_ENVELOPE_PLAIN).unwrap();
    (0..parsed.count)
        .map(|index| parsed.entry(index).unwrap().id)
        .collect()
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
fn interrupted_member_declarations_finish_from_a_complete_seed_or_stay_preserved() {
    let shared = pool();
    // (cut, a configured cold reopen finishes it). Cuts 240-242 stop before a complete
    // seed exists: the first-torn-seed state is preserved and refused (not repaired here).
    for (cut, finishes) in [
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
        settled_three(path, &shared, &mut rng);
        let mut owner =
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, member_policy(&shared))
                .unwrap();
        fault(&owner, cut);
        assert!(owner.retry(&mut rng).is_err(), "cut {cut}");
        drop(owner);
        let snapshot = files(path);

        // An opener without the handler refuses every interrupted state and changes nothing.
        assert!(
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, policy(&shared))
                .is_err(),
            "cut {cut}"
        );
        assert_eq!(files(path), snapshot, "cut {cut}");
        // So does a binary that predates namespace5 entirely (it does not even know the
        // member names): it is fenced only by the stage slot and its closed grammar.
        refuses_as_pre_namespace5(path, &shared, cut);
        assert_eq!(files(path), snapshot, "cut {cut}");

        let reopened =
            PreparedV2CurrentLayoutStore::open(path, SECRET, &mut NeverRng, member_policy(&shared));
        if finishes {
            let mut cold = reopened.unwrap_or_else(|error| panic!("cut {cut}: {error:?}"));
            cold.retry(&mut NeverRng).unwrap();
            private(&cold)
                .validate_mounted_member_outbox_layout()
                .unwrap();
            drop(cold.into_current_layout_store().unwrap());
            let (objects, parent) = files(path);
            assert!(parent, "cut {cut}");
            assert!(objects[MEMBER_OUTBOX_SEED].is_none(), "cut {cut}");
            assert!(objects[Slot::Stage.name()].is_none(), "cut {cut}");
            assert!(
                objects["chat-owner-v2/member-outbox-layout.bin"].is_some(),
                "cut {cut}"
            );
        } else {
            assert!(reopened.is_err(), "cut {cut}");
            assert_eq!(files(path), snapshot, "cut {cut}");
        }
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
