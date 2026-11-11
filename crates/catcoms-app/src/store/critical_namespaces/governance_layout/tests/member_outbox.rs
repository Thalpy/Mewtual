//! Namespace4 and namespace5 declared in either order on real files, and real JOIN
//! lifecycle writes preserving both descriptors exactly. Declaring ID5 grants nothing to
//! governance and needs nothing from it; no Source4, member source or save is exercised.
use super::super::super::member_outbox_layout::{codec as member_codec, MEMBER_OUTBOX_PARENT};
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

/// `(id, version, payload)` of the selected catalog, decrypted with the vault key.
fn catalog(key: &[u8; 32], domain: &[u8; 32], path: &Path) -> Vec<(u32, u32, Vec<u8>)> {
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
    let snapshot = files(path);
    assert!(
        PreparedV2GovernanceLayoutStore::open(path, SECRET, &mut NeverRng, policy(&shared))
            .is_err()
    );
    assert_eq!(files(path), snapshot);

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
    let snapshot = files(path);
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
    assert_eq!(files(path), snapshot);
    let mut cold = PreparedV2GovernanceLayoutStore::open(
        path,
        SECRET,
        &mut NeverRng,
        member_governance(&shared),
    )
    .unwrap();
    cold.retry(&mut NeverRng).unwrap();
    drop(cold.into_governance_layout_store().unwrap());
    assert_eq!(files(path), snapshot);
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
