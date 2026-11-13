//! Actual opaque namespace7 owner for native boundary regressions. No SDK,
//! runtime row, permission bit, raw Store loan or injected ready state is used.
use super::*;
use catcoms_app::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    NativeIndexPolicy, NativeIndexUpgrade, NativeRegistryInstallation, NativeRegistryPolicy,
    NativeTextVault, NativeUiInvocationBudget, V2CurrentLayoutPolicy, V2MemberOutboxPolicy,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn actual_native_vault(path: &Path, state: &AppState, secret: &[u8]) -> NativeTextVault {
    let mut rng = ChaCha20Rng::seed_from_u64(0x7c01);
    let original = ServerStore::open(path, secret, &mut rng).unwrap();
    drop(
        original
            .into_critical_namespace_store(secret, &mut rng)
            .unwrap(),
    );
    // These are trusted logical limits, not allocations or free-space claims.
    // Deliberately separate archive, outbox, registry, index and UI accounts.
    let current = V2CurrentLayoutPolicy::new(
        ArchiveBootstrapCatalogPolicy::new(
            CriticalCatalogReadPool::new(1024 * 1024 * 1024).unwrap(),
            1024 * 1024 * 1024,
        )
        .unwrap()
        .with_bootstrap_source_data_read(),
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        1024 * 1024 * 1024,
    )
    .unwrap()
    .with_member_outbox(
        V2MemberOutboxPolicy::new(
            CriticalCatalogReadPool::new(64 * 1024 * 1024).unwrap(),
            256,
            256 * 1024 * 1024,
            1 << 20,
            1,
            1024 * 1024 * 1024,
        )
        .unwrap(),
    )
    .unwrap();
    let registry = NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap();
    let mut install =
        NativeRegistryInstallation::prepare(path, secret, &mut rng, current, registry).unwrap();
    install.retry().unwrap();
    let index_policy = NativeIndexPolicy::new(
        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
        NativeIndexPolicy::required_persistent_capacity(),
    )
    .unwrap();
    let mut upgrade = NativeIndexUpgrade::prepare(
        install.into_native_registry().unwrap(),
        index_policy,
        &mut rng,
    )
    .unwrap();
    upgrade.retry().unwrap();
    let index = upgrade.into_index_store().unwrap();
    let ui = NativeUiInvocationBudget::new(
        CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap(),
    )
    .unwrap();
    NativeTextVault::from_trusted_host(
        index,
        state.ui_session_generation.clone(),
        state.ui_authorized_generation.clone(),
        state.session_lock_requested.clone(),
        ui,
    )
    .unwrap()
}

#[tokio::test]
async fn mounted_native_vault_refuses_legacy_unlock_without_cold_fallback_or_owner_replacement() {
    let state = AppState::default();
    let lock = lock_session_outcome_inner(&state, None).await;
    let path = tempfile::tempdir().unwrap();
    let vault = actual_native_vault(path.path(), &state, b"native auth boundary");
    let before_count = vault.row_count_data().unwrap();
    let index = path
        .path()
        .join("chat-owner-v2")
        .join("native-registry.bin");
    let before_bytes = std::fs::read(&index).unwrap();
    *state.store.lock().await = Some(MountedVault::TextV2(vault));
    let before_authorized = state.ui_authorized_generation.load(Ordering::SeqCst);
    let error = unlock_authentication(
        &state,
        "native auth boundary".into(),
        lock.generation.clone(),
        None,
        |_, _, _, _| async { panic!("native owner reached legacy cold continuation") },
    )
    .await
    .err()
    .expect("unimplemented native route must explicitly refuse");
    assert_eq!(
        serde_json::to_value(error).unwrap()["code"],
        "VAULT.OPEN.REFUSED"
    );
    assert_eq!(
        state.ui_authorized_generation.load(Ordering::SeqCst),
        before_authorized
    );
    assert!(state.session_lock_requested.load(Ordering::SeqCst));
    assert!(!*state.session_resumable.lock().await);
    assert!(require_unlocked_session(&state).await.is_err());
    {
        let guard = state.store.lock().await;
        let owner = guard
            .as_ref()
            .unwrap()
            .as_text_v2()
            .expect("same native arm retained");
        assert_eq!(owner.row_count_data().unwrap(), before_count);
        assert_eq!(std::fs::read(&index).unwrap(), before_bytes);
        // Still the one original physical owner, not an owner silently moved
        // into or replaced by a legacy open after the refusal.
        assert!(ServerStore::open(path.path(), b"native auth boundary", &mut OsCryptoRng).is_err());
    }
    let owner = state.store.lock().await.take();
    drop(owner); // Close/notify/retire only after the native guard has ended.
}
