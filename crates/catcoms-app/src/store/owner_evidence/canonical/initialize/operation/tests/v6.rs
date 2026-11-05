//! Actual supported EMPTY-catalog Store, native owner successor and retained client transport.
//! No metadata helper alone constructs supported-format, Current, Saved or membership authority.
use super::*;

#[tokio::test]
async fn known_v6_empty_store_reaches_actual_add_retry_selected_member_and_cold_gen2_restore() {
    let source_dir = tempfile::tempdir().unwrap();
    let member_dir = tempfile::tempdir().unwrap();
    let mut source_rng = ChaCha20Rng::seed_from_u64(0x6ad1);
    let mut member_rng = ChaCha20Rng::seed_from_u64(0x6ad2);
    let source = open_format(source_dir.path(), &mut source_rng, true);
    let destination = open_format(member_dir.path(), &mut member_rng, true);
    let catalog = std::fs::read(source_dir.path().join("chat-critical-namespaces.bin")).unwrap();
    let hub = Hub::new();
    let mut owner = founder(&source, &hub, 0x6ad1, &mut source_rng);
    let member = complete_actual_join(
        &mut owner,
        &source,
        &destination,
        &hub,
        0x6ad1,
        0x6ad2,
        &mut source_rng,
        &mut member_rng,
        Some(2),
    )
    .await;
    assert!(owner.is_live() && member.is_live());
    assert_eq!(
        source.chat_vault_format().unwrap(),
        catcoms_storage::VaultFormat::CriticalNamespaces
    );
    assert_eq!(
        destination.chat_vault_format().unwrap(),
        catcoms_storage::VaultFormat::CriticalNamespaces
    );
    let group = owner.group();
    let signer = *owner.founder();
    let request = owner.server.sync.owner_operation_request_id();
    let head = owner.current_head.clone();
    drop(member);
    drop(owner);
    drop(source);
    // Ordinary open authenticates the existing mandatory empty catalog before the positive
    // known-format helper is reachable; no consuming re-upgrade or empty substitution here.
    let source = ServerStore::open(source_dir.path(), SECRET, &mut source_rng).unwrap();
    source.configure_owner_evidence_budget(budget()).unwrap();
    assert_eq!(
        source.chat_vault_format().unwrap(),
        catcoms_storage::VaultFormat::CriticalNamespaces
    );
    let OwnerRestoreClassification::Current(prepared) = source
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("actual v6 first Add retains its original Current scope");
    };
    let mut reopened = prepared
        .restore_private(
            &source,
            Hub::new().join(PeerId::from_u64(0x6ad3)),
            ChaCha20Rng::seed_from_u64(0x6ad3),
            Box::new(ManualClock::new(1_000_000)),
            "supported v6 reopened",
        )
        .unwrap();
    let owner = reopened.retry(&source, &mut source_rng).unwrap();
    assert_eq!(owner.current_head, head);
    assert_eq!(owner.server.sync.owner_operation_request_id(), request);
    assert_eq!(owner.server.sync.member_count(), 2);
    assert_eq!(owner.server.sync.epoch(), 1);
    assert_eq!(
        std::fs::read(source_dir.path().join("chat-critical-namespaces.bin")).unwrap(),
        catalog
    );
}

#[test]
fn known_v6_positive_owner_gate_never_skips_unknown_catalog_before_older_stage_cleanup() {
    for (name, id) in [
        ("chat-critical-namespaces.bin", 1u32),
        ("chat-critical-namespaces.replace.tmp", 17),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0x6ad4);
        let original = ServerStore::open(directory.path(), SECRET, &mut rng)
            .unwrap()
            .into_owner_evidence_store(SECRET, &mut rng)
            .unwrap();
        let v4 = std::fs::read(directory.path().join("vault.bin")).unwrap();
        let store = original
            .into_critical_namespace_store(SECRET, &mut rng)
            .unwrap();
        store.configure_owner_evidence_budget(budget()).unwrap();
        let owner = founder(&store, &Hub::new(), 0x6ad4, &mut rng);
        assert!(owner.is_live());
        let identifier = key(&owner.group(), owner.founder());
        let canonical = directory
            .path()
            .join("owner-state")
            .join(format!("{}.bin", hex::encode(identifier)));
        let saved = std::fs::read(&canonical).unwrap();
        let mut e = Encoder::new();
        e.put_str("catcoms/critical-vault-namespaces/v1").unwrap();
        e.put_bytes(&store.chat_vault_domain().unwrap()).unwrap();
        e.put_u64(1);
        e.put_u32(1);
        e.put_u32(id);
        e.put_u32(1);
        e.put_bytes(b"unsupported retained namespace").unwrap();
        let unsupported = crate::store::frame(
            &seal(&store.keys.db_key().unwrap(), &e.finish(), &mut rng).unwrap(),
        );
        std::fs::write(directory.path().join(name), &unsupported).unwrap();
        let pins_stage = directory.path().join("chat-owner-pins.replace.tmp");
        std::fs::write(&pins_stage, b"must survive before old cleaner").unwrap();
        // An authentic older wrapper ALONE does not prevent the current reader seeing the
        // retained catalog. Removing the newer intent makes the actual older pin cleaner
        // reachable if constructor ordering were wrong; the catalog must refuse first.
        std::fs::write(directory.path().join("vault.bin"), &v4).unwrap();
        std::fs::remove_file(directory.path().join("chat-owner-operations.bin")).unwrap();
        drop(owner);
        drop(store);
        assert!(ServerStore::open(directory.path(), SECRET, &mut rng).is_err());
        assert_eq!(
            std::fs::read(directory.path().join(name)).unwrap(),
            unsupported
        );
        assert_eq!(
            std::fs::read(&pins_stage).unwrap(),
            b"must survive before old cleaner"
        );
        assert_eq!(std::fs::read(&canonical).unwrap(), saved);
        assert_eq!(
            std::fs::read(directory.path().join("vault.bin")).unwrap(),
            v4
        );
    }
}
