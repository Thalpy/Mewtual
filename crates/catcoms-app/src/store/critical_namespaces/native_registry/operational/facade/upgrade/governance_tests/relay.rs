//! Actual configured Current4/ID8/ID13 ownership. These are storage tests:
//! signed action fixture DATA does not stand in for a native member verifier.
use super::*;
use crate::store::persistence::{
    v2_member_outbox::member_outbox_key,
    v2_shared_history::{
        PreparedSharedHistoryRecord, SharedHistoryRelayPageError, SharedHistoryScope,
    },
};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId, owner_header::OwnerTenureId, ActionSpec, BasisId, Body, GrantId,
    Realm, SignedAction,
};

fn mounted(path: &Path, rng: &mut ChaCha20Rng) -> NativeTextJoinedStorage {
    let mut plan = original(path, rng).prepare_governance().unwrap();
    plan.retry(rng).unwrap();
    let mut upgrade = plan.prepare_joined_vault().unwrap();
    upgrade.retry_with_indexed_history(rng).unwrap();
    drop(upgrade.into_vault().ok().unwrap());
    NativeTextJoinedStorage::open_for_indexed_native_startup(
        path,
        SECRET,
        &mut NeverRng,
        current_policy(),
    )
    .unwrap()
}

fn scope(store: &crate::store::ServerStore) -> SharedHistoryScope {
    SharedHistoryScope {
        vault: store.chat_vault_domain().unwrap(),
        group: [0x64; 16],
        profile: OwnerFactId::from_bytes([0x65; 32]),
        tenure: OwnerTenureId::from_bytes([0x66; 32]),
        creation: [0x67; 32],
        channel: 17,
    }
}

fn save(
    store: &crate::store::ServerStore,
    nonce: u8,
    rng: &mut ChaCha20Rng,
) -> catcoms_replication::chat::ActionId {
    let scope = scope(store);
    let action = SignedAction::sign(
        &catcoms_crypto::DeviceKeypair::from_seed(&[0x68; 32]),
        ActionSpec {
            realm: Realm::new(&scope.group, scope.channel).unwrap(),
            basis: BasisId::from_bytes([0x69; 32]),
            grant: GrantId::from_bytes([0x70; 32]),
            nonce: [nonce; 16],
            presented_ms: u64::from(nonce),
            parents: vec![],
            body: Body::Create {
                text: format!("mounted paged relay {nonce}"),
                reply_to: None,
            },
        },
    )
    .unwrap();
    let key = member_outbox_key(store).unwrap();
    let record = PreparedSharedHistoryRecord::prepare(
        &scope,
        &key,
        OwnerFactId::from_bytes([0x71; 32]),
        &action,
        &action.encode(),
        rng,
    )
    .unwrap();
    let manifest = record.manifest.id();
    store
        .save_shared_history_record(
            scope,
            action.id(),
            manifest,
            &record.encode_bundle().unwrap(),
            rng,
        )
        .unwrap();
    action.id()
}

#[test]
fn indexed_archive_relay_mounted_empty_and_quiet_root_before_optional_compatibility_completion() {
    let temp = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe540);
    let vault = mounted(temp.path(), &mut rng);
    let store = vault.actual_store_for_test().unwrap();
    let scope = scope(store);
    assert!(store.prepare_shared_history_relay(scope).unwrap());
    let empty = store
        .selected_shared_history_relay_page(scope, None)
        .unwrap();
    assert_eq!(empty.relay_entries_data().len(), 0);
    assert!(!empty.has_more_data());
    assert!(matches!(
        store.selected_shared_history_relay_page(scope, Some(&empty)),
        Err(SharedHistoryRelayPageError::Stale)
    ));
    drop(empty);
    let mut expected = (1..=17)
        .map(|nonce| save(store, nonce, &mut rng))
        .collect::<Vec<_>>();
    expected.sort_unstable();
    let mut page = store
        .selected_shared_history_relay_page(scope, None)
        .unwrap();
    let mut found = Vec::new();
    let mut lengths = Vec::new();
    loop {
        lengths.push(page.relay_entries_data().len());
        found.extend(page.relay_entries_data().map(|entry| entry.action));
        store
            .require_selected_shared_history_relay_page(scope, &page)
            .unwrap();
        // The actual old body-read path is retained on each selected window.
        let entries = page.relay_entries_data().collect::<Vec<_>>();
        let mut checked = Vec::new();
        store
            .visit_selected_shared_history_relay_page(
                scope,
                &entries,
                || Ok(()),
                |entry, original| {
                    assert_eq!(entry.action, original.id());
                    checked.push(original.id());
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            checked,
            entries.iter().map(|entry| entry.action).collect::<Vec<_>>()
        );
        store
            .require_selected_shared_history_relay_page(scope, &page)
            .unwrap();
        if !page.has_more_data() {
            break;
        }
        page = store
            .selected_shared_history_relay_page(scope, Some(&page))
            .unwrap();
    }
    assert_eq!(lengths, [8, 8, 1]);
    assert_eq!(found, expected);
    // Only this explicit compatibility call now creates its separately paid
    // old capped index. Ordinary pages above needed no catalogue, and remain
    // usable while >8 originals leave healthy review work for another turn.
    assert!(!store.advance_shared_history_relay_index(scope).unwrap());
    let independent = store
        .selected_shared_history_relay_page(scope, None)
        .unwrap();
    assert_eq!(
        independent
            .relay_entries_data()
            .map(|entry| entry.action)
            .collect::<Vec<_>>(),
        expected[..8]
    );
    let mut compatibility_turns = 1;
    while !store.advance_shared_history_relay_index(scope).unwrap() {
        compatibility_turns += 1;
        assert!(compatibility_turns < 3);
    }
    assert_eq!(compatibility_turns + 1, 3);
    store
        .require_selected_shared_history_relay_page(scope, &independent)
        .unwrap();
}

#[test]
fn indexed_archive_relay_mounted_foreign_scope_new_selection_and_equal_head_replacement_refuse() {
    use crate::store::persistence::{
        governance_layout::joined::shared_history::DIRECTORY_NAME,
        v2_shared_history::INDEXED_HEAD_NAME,
    };
    let temp = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe541);
    let vault = mounted(temp.path(), &mut rng);
    let foreign = mounted(other.path(), &mut rng);
    let store = vault.actual_store_for_test().unwrap();
    let other_store = foreign.actual_store_for_test().unwrap();
    let scope = scope(store);
    for nonce in 1..=9 {
        save(store, nonce, &mut rng);
    }
    let first = store
        .selected_shared_history_relay_page(scope, None)
        .unwrap();
    assert!(store
        .require_selected_shared_history_relay_page(
            SharedHistoryScope {
                channel: 18,
                ..scope
            },
            &first
        )
        .is_err());
    assert!(other_store
        .require_selected_shared_history_relay_page(self::scope(other_store), &first)
        .is_err());
    save(store, 10, &mut rng);
    assert!(matches!(
        store.selected_shared_history_relay_page(scope, Some(&first)),
        Err(SharedHistoryRelayPageError::Stale)
    ));
    assert!(matches!(
        store.require_selected_shared_history_relay_page(scope, &first),
        Err(SharedHistoryRelayPageError::Stale)
    ));
    let fresh = store
        .selected_shared_history_relay_page(scope, None)
        .unwrap();
    let head = temp.path().join(DIRECTORY_NAME).join(INDEXED_HEAD_NAME);
    let bytes = fs::read(&head).unwrap();
    let parked = temp.path().join("actual-relay-selected-head");
    fs::rename(&head, &parked).unwrap();
    fs::write(&head, &bytes).unwrap();
    assert!(store
        .require_selected_shared_history_relay_page(scope, &fresh)
        .is_err());
    assert!(store
        .selected_shared_history_relay_page(scope, Some(&fresh))
        .is_err());
    fs::remove_file(&head).unwrap();
    fs::rename(&parked, &head).unwrap();
    store
        .require_selected_shared_history_relay_page(scope, &fresh)
        .unwrap();
    assert!(!store
        .selected_shared_history_relay_page(scope, Some(&fresh))
        .unwrap()
        .has_more_data());
}
