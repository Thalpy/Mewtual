//! Real JOIN v1 write, pause/resume, v2 migration and execution-arm paths.
//! The checkpoint fixture tests surrounding catalog preservation only; it is not
//! a V2 membership/source/signer grant and it does not activate the private4 API.
use super::*;
#[test]
fn actual_join_pause_resume_migration_and_first_arm_preserve_complete_namespace4_payload() {
    use crate::store::{CheckpointJoinSubmissionStatus, OwnerEvidenceBudget};
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, PeerId};
    let source_dir = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x3371);
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
            hub.join(PeerId::from_u64(0x3371)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0x3371),
            Box::new(ManualClock::new(1_000)),
            "actual checkpoint source",
            9,
        )
        .unwrap();
    let owner = founder.retry(&source, &mut rng).unwrap();
    let invite = owner
        .mint_owner_checkpoint_invite(&source, [71; 16], 100_000, vec![])
        .unwrap();
    settled_three(directory.path(), &shared, &mut rng);
    let execution = current_layout::V2CurrentLayoutPolicy::new(
        ArchiveBootstrapCatalogPolicy::new(shared.clone(), 4 * archive_bootstrap::MAX_SEED_FRAMED)
            .unwrap()
            .with_bootstrap_source_data_read()
            .with_checkpoint_join_execution(2, 10_000)
            .unwrap(),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        4 * archive_current_namespace_codec::MAX_DECLARATION_FRAME,
    )
    .unwrap();
    let governance_policy =
        V2GovernanceLayoutPolicy::new(execution, codec::PERSISTENT_MINIMUM).unwrap();
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
    let before = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let key = store.keys.db_key().unwrap();
    let plain = Zeroizing::new(unseal(&key, &unframe(&before).unwrap()).unwrap());
    let payload = store
        .parse_configured_join_catalog(&plain)
        .unwrap()
        .governance_payload
        .unwrap()
        .to_vec();
    let mut prepared = store
        .prepare_journalled_owner_checkpoint_join(
            &source,
            &owner,
            MlsDevice::generate().unwrap(),
            invite,
            &mut rng,
        )
        .unwrap();
    prepared.save_pending(store, &mut rng).unwrap();
    prepared.pause_pending(store, &mut rng).unwrap();
    prepared.resume_pending(store, &mut rng).unwrap();
    let activated = prepared
        .activate_original_transport(
            store,
            hub.join(PeerId::from_u64(0x3372)),
            PeerId::from_u64(0x3371),
        )
        .unwrap();
    let mut network = activated.into_network_execution(store).unwrap();
    assert_eq!(
        network
            .ensure_submission_armed(store, &mut rng, Arc::new(ManualClock::new(1_000)))
            .unwrap(),
        CheckpointJoinSubmissionStatus::Armed
    );
    let updated = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let updated_plain = Zeroizing::new(unseal(&key, &unframe(&updated).unwrap()).unwrap());
    let parsed = store.parse_configured_join_catalog(&updated_plain).unwrap();
    assert_eq!(parsed.governance_payload.unwrap(), payload);
    assert_eq!(
        parsed.join_version,
        Some(super::super::super::pending_join::execution_payload::VERSION)
    );
    let row = &parsed.rows.iter().flatten().next().unwrap().metadata;
    assert_eq!(row.execution.unwrap().limit, 2);
    assert_eq!(row.execution.unwrap().spent, 1);
    store.validate_mounted_governance_layout().unwrap();
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
    let raw = fs::read(directory.path().join(Slot::Current.name())).unwrap();
    let plain = unseal(&key, &unframe(&raw).unwrap()).unwrap();
    assert_eq!(
        private(&cold)
            .parse_configured_join_catalog(&plain)
            .unwrap()
            .governance_payload
            .unwrap(),
        payload
    );
    assert_eq!(
        fs::read(directory.path().join(Slot::Current.name())).unwrap(),
        updated
    );
}
