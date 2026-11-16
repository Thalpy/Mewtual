//! Scope half only: genuine Current + independently paid prepared cleanup. A unit callback
//! observes the real Durable role but does not fabricate a Saved cleanup or unlink anything.
//! Declare as a child of current::tests when the complete settlement consumer is paired.
//! The audited native fixed-folder cleanup supports Windows and Linux: three positive
//! tests run there; other platforms run an explicit no-effect refusal test, not those positives.
use super::*;
#[cfg(any(windows, target_os = "linux"))]
use crate::store::critical_namespaces::archive_current_codec::SCOPE_BUFFER_FEE;
use crate::store::persistence::v2_genesis_activation::settlement::PreparedGenesisSeedSettlement;
#[cfg(any(windows, target_os = "linux"))]
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
#[cfg(any(windows, target_os = "linux"))]
use std::cell::Cell;
use std::sync::atomic::Ordering;
use zeroize::Zeroizing;

#[cfg(any(windows, target_os = "linux"))]
struct CountRng<'rng> {
    inner: &'rng mut ChaCha20Rng,
    draws: usize,
}
#[cfg(any(windows, target_os = "linux"))]
impl RngCore for CountRng<'_> {
    fn next_u32(&mut self) -> u32 {
        self.draws += 1;
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.draws += 1;
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.draws += 1;
        self.inner.fill_bytes(dest);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.draws += 1;
        self.inner.try_fill_bytes(dest)
    }
}
#[cfg(any(windows, target_os = "linux"))]
impl CryptoRng for CountRng<'_> {}

/// Compare the COMPLETE actual serialized inventory without logging its nonce/key rows.
/// The test alone opens its disk ciphertext; it never decodes/adopts an owner from it.
fn inventory(store: &ServerStore) -> Zeroizing<Vec<u8>> {
    let raw = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
    let sealed = crate::store::unframe(&raw).unwrap();
    Zeroizing::new(catcoms_crypto::unseal(&store.keys.db_key().unwrap(), &sealed).unwrap())
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn even_clean_scope_barrier_precedes_cleanup_and_retains_exact_owner_on_both_write_faults() {
    for after_rename in [false, true] {
        let mut f = fixture(0xef01 + u64::from(after_rename));
        f.preparation.retry(&f.layout, &mut f.rng).unwrap();
        let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
        let store = f.layout.checked_store().unwrap();
        let owner = current.settlement_owner_ref().unwrap();
        let mut prepared = PreparedGenesisSeedSettlement::prepare(store, &owner).unwrap();
        let before = inventory(store);
        let nonce = current.scope.nonce();
        let identity = current.scope.owner_identity();
        let source = current.data().source;
        let original_task = current.task.observe_original();
        let original_writer = current.writer.observe_original();
        let baseline = f.pool.charged_for_test();
        let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
        assert_eq!(f.pool.charged_for_test(), baseline + SCOPE_BUFFER_FEE);
        let callbacks = Cell::new(0);
        if after_rename {
            store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
        } else {
            store
                .chat_fence_pre_rename_failure
                .store(true, Ordering::SeqCst);
        }
        // The actual Current is clean at entry. A dirty-only shortcut would enter the
        // callback and leave this REAL physical fault armed, instead of failing the write.
        assert!(store
            .with_durable_genesis_current2_settlement(
                &owner,
                &mut prepared,
                &buffers,
                &mut f.rng,
                |_, _| {
                    callbacks.set(callbacks.get() + 1);
                    Ok(())
                },
            )
            .is_err());
        assert_eq!(callbacks.get(), 0);
        assert!(inventory(store).as_slice() == before.as_slice());
        assert!(current.validate_original(&f.layout).is_err());
        assert_eq!(nonce, current.scope.nonce());
        assert!(Arc::ptr_eq(&identity, &current.scope.owner_identity()));
        assert_eq!(source, current.data().source);
        original_task.require_same_original(&current.task).unwrap();
        current
            .writer
            .require_same_observation(&original_writer)
            .unwrap();

        // Settle the earlier authorized dirty write with the SAME loaded inventory. This
        // maintenance result is not the NEW even-clean covering barrier below.
        store
            .resync_v2_current_scope_inventory(&buffers, &mut f.rng)
            .unwrap();
        let mut counted = CountRng {
            inner: &mut f.rng,
            draws: 0,
        };
        store
            .with_durable_genesis_current2_settlement(
                &owner,
                &mut prepared,
                &buffers,
                &mut counted,
                |durable, _| {
                    durable.validate_source_only()?;
                    callbacks.set(callbacks.get() + 1);
                    Ok(())
                },
            )
            .unwrap();
        assert!(counted.draws > 0);
        assert_eq!(callbacks.get(), 1);
        assert!(inventory(store).as_slice() == before.as_slice());
        assert_eq!(nonce, current.scope.nonce());
        assert!(Arc::ptr_eq(&identity, &current.scope.owner_identity()));
        original_task.require_same_original(&current.task).unwrap();
        current
            .writer
            .require_same_observation(&original_writer)
            .unwrap();
        drop(buffers); // actual refund OUTSIDE admission/namespace/Root writers
        assert_eq!(f.pool.charged_for_test(), baseline);
        // Unit returned no cleanup proof. The actual named activation seed remains and the
        // original strict seed-present Current proof still succeeds after refund.
        current.validate_original(&f.layout).unwrap();
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn covering_scope_credit_is_original_account_bound_and_cancelled_owner_cannot_enter() {
    let mut f = fixture(0xef10);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    let store = f.layout.checked_store().unwrap();
    let owner = current.settlement_owner_ref().unwrap();
    let mut prepared = PreparedGenesisSeedSettlement::prepare(store, &owner).unwrap();
    let before = inventory(store);
    let original_owner = current.scope.owner_identity();
    let original_nonce = current.scope.nonce();
    let short = f.pool.try_reserve(SCOPE_BUFFER_FEE - 1).unwrap();
    assert!(store
        .with_durable_genesis_current2_settlement(
            &owner,
            &mut prepared,
            &short,
            &mut NeverRng,
            |_, _| -> Result<(), AppError> { panic!("short credit reached callback") },
        )
        .is_err());
    drop(short);
    let foreign_pool =
        critical_namespaces::read_pool::CriticalCatalogReadPool::new(CAPACITY).unwrap();
    let foreign = foreign_pool.try_reserve(SCOPE_BUFFER_FEE).unwrap();
    assert!(store
        .with_durable_genesis_current2_settlement(
            &owner,
            &mut prepared,
            &foreign,
            &mut NeverRng,
            |_, _| -> Result<(), AppError> { panic!("foreign credit reached callback") },
        )
        .is_err());
    drop(foreign);
    assert_eq!(foreign_pool.charged_for_test(), 0);
    assert!(inventory(store).as_slice() == before.as_slice());
    current.validate_original(&f.layout).unwrap();

    let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
    let original_observer = current.scope.reservation_observer();
    // Cancellation AFTER the real write/full pre-callback proof still cannot produce a
    // positive result. The callback returns only unit; it cannot override the original
    // negative or fabricate a Saved cleanup. A later retry also refuses before RNG.
    assert!(store
        .with_durable_genesis_current2_settlement(
            &owner,
            &mut prepared,
            &buffers,
            &mut f.rng,
            |durable, _| {
                durable.validate_source_only()?;
                current.scope.cancel_original();
                Ok(())
            },
        )
        .is_err());
    assert!(original_observer.is_cancelled());
    assert!(store
        .with_durable_genesis_current2_settlement(
            &owner,
            &mut prepared,
            &buffers,
            &mut NeverRng,
            |_, _| -> Result<(), AppError> { panic!("cancelled owner reached callback") },
        )
        .is_err());
    assert!(inventory(store).as_slice() == before.as_slice());
    assert_eq!(original_nonce, current.scope.nonce());
    assert!(Arc::ptr_eq(
        &original_owner,
        &current.scope.owner_identity()
    ));
    assert!(original_observer.is_cancelled());
    drop(buffers);
    assert!(current.validate_original(&f.layout).is_err());
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn even_clean_covering_write_preserves_all_unrelated_rows_and_their_original_owners() {
    use crate::store::ChatAdmissionKey;
    let mut f = fixture(0xef20);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    let store = f.layout.checked_store().unwrap();
    let capture = store
        .capture_chat_admission(
            ChatAdmissionKey::new(b"settlement unrelated group", [0x73; 32]).unwrap(),
        )
        .unwrap();
    let other = store
        .prepare_legacy_chat_admission(&capture, b"original unrelated row", &mut f.rng)
        .unwrap();
    let other_nonce = other.nonce();
    let other_owner = other.owner_identity();
    let before = inventory(store);
    let owner = current.settlement_owner_ref().unwrap();
    let mut prepared = PreparedGenesisSeedSettlement::prepare(store, &owner).unwrap();
    let buffers = store.reserve_v2_genesis_scope_buffers().unwrap();
    let mut counted = CountRng {
        inner: &mut f.rng,
        draws: 0,
    };
    store
        .with_durable_genesis_current2_settlement(
            &owner,
            &mut prepared,
            &buffers,
            &mut counted,
            |durable, _| {
                durable.validate_source_only()?;
                assert!(!durable.read_admission().owner_observer().is_cancelled());
                Ok(())
            },
        )
        .unwrap();
    assert!(counted.draws > 0);
    assert!(inventory(store).as_slice() == before.as_slice());
    drop(buffers);
    let same = store
        .prepare_legacy_chat_admission(&capture, b"original unrelated row", &mut NeverRng)
        .unwrap();
    assert_eq!(same.nonce(), other_nonce);
    assert!(Arc::ptr_eq(&same.owner_identity(), &other_owner));
    assert!(!same.reservation_observer().is_cancelled());
    current.validate_original(&f.layout).unwrap();
}

#[cfg(not(any(windows, target_os = "linux")))]
#[test]
fn unaudited_native_cleanup_profile_refuses_without_scope_effect_or_seed_loss() {
    let mut f = fixture(0xef30);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    let store = f.layout.checked_store().unwrap();
    let owner = current.settlement_owner_ref().unwrap();
    let before = inventory(store);
    let charged = f.pool.charged_for_test();
    store
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    // The closed physical profile refuses before Scope can obtain an actual Prepared
    // owner to pass into its covering callback. No fake preparation replaces this refusal.
    assert!(PreparedGenesisSeedSettlement::prepare(store, &owner).is_err());
    assert!(store.chat_fence_pre_rename_failure.load(Ordering::SeqCst));
    assert!(inventory(store).as_slice() == before.as_slice());
    assert_eq!(charged, f.pool.charged_for_test());
    // The old supported seed-present path is unchanged and its named seed is preserved.
    current.validate_original(&f.layout).unwrap();
}
