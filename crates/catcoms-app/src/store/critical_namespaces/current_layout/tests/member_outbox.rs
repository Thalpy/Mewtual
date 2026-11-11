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
        (251, true),
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
