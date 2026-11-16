//! Resource/negative-lifetime unit tests. The watch fixtures construct NO admission, Saved
//! head, source or Current. Genuine registration acceptance also uses the owning caller's
//! physical SourceSaved/namespace/head/scope tests; this kernel alone cannot issue Current.
use super::*;

struct NegativeGeneration {
    slot: Slot,
    life: Arc<WriterLife>,
    observation: V2WriterObservation,
    _mount: watch::Sender<bool>,
    _scope: watch::Sender<bool>,
}
fn negative_generation(pool: &CriticalCatalogReadPool, key: u8) -> NegativeGeneration {
    negative_generation_with_notice(pool, key, None)
}
fn negative_generation_with_notice(
    pool: &CriticalCatalogReadPool,
    key: u8,
    drop_notice: Option<std::sync::mpsc::Sender<std::thread::ThreadId>>,
) -> NegativeGeneration {
    let fee = Arc::new(pool.try_reserve(WRITER_METADATA_FEE).unwrap());
    let (mount_owner, mount) = watch::channel(false);
    let (scope_owner, scope) = watch::channel(false);
    let (sender, receiver) = watch::channel(false);
    let life = Arc::new(WriterLife {
        identity: Arc::new(()),
        sender,
        drop_notice,
    });
    let cancellation = RequestCancellation::new(
        receiver,
        Some(Arc::new(ObservationAccounting { _fee: fee.clone() })),
    )
    .with_authority(Arc::new(AccountedAuthority {
        mount: RequestCancellation::new(mount, None),
        scope: RequestCancellation::new(scope, None),
        _fee: fee.clone(),
    }));
    NegativeGeneration {
        slot: Slot {
            key: [key; 32],
            owner: Arc::downgrade(&life),
            _fee: fee,
        },
        observation: V2WriterObservation {
            identity: life.identity.clone(),
            original_life: Arc::downgrade(&life),
            cancellation,
        },
        life,
        _mount: mount_owner,
        _scope: scope_owner,
    }
}

#[test]
fn installed_observation_requires_exact_live_generation_without_retaining_it() {
    let pool =
        CriticalCatalogReadPool::new(buffer_bytes().unwrap() + 3 * WRITER_METADATA_FEE).unwrap();
    let original = negative_generation(&pool, 41);
    let foreign = negative_generation(&pool, 41);
    let mut runtime = Runtime::default();
    assert!(runtime
        .require_installed_observation(&[41; 32], &original.observation)
        .is_err());
    let mut buffer = SlotBuffer::prepare(&pool).unwrap();
    buffer.slots.push(original.slot);
    runtime.buffer = Some(buffer);
    let count = Arc::strong_count(&original.life);
    let charged = pool.charged_for_test();
    let cloned = original.observation.clone();
    runtime
        .require_installed_observation(&[41; 32], &cloned)
        .unwrap();
    assert_eq!(Arc::strong_count(&original.life), count);
    assert!(runtime
        .require_installed_observation(&[42; 32], &cloned)
        .is_err());
    assert!(runtime
        .require_installed_observation(&[41; 32], &foreign.observation)
        .is_err());
    original.life.sender.send_replace(true);
    assert!(runtime
        .require_installed_observation(&[41; 32], &cloned)
        .is_err());
    assert!(runtime.require_no_live_writer(&[41; 32]).is_err());
    assert_eq!(runtime.buffer.as_ref().unwrap().slots.len(), 1);
    assert_eq!(pool.charged_for_test(), charged);
    drop(original.life);
    assert!(runtime
        .require_installed_observation(&[41; 32], &cloned)
        .is_err());
}

#[test]
fn old_observation_cannot_select_same_key_replacement_and_pays_its_own_weak_tail() {
    let pool =
        CriticalCatalogReadPool::new(buffer_bytes().unwrap() + 2 * WRITER_METADATA_FEE).unwrap();
    let old = negative_generation(&pool, 43);
    let next = negative_generation(&pool, 43);
    let mut buffer = SlotBuffer::prepare(&pool).unwrap();
    buffer.slots.push(old.slot);
    let mut runtime = Runtime {
        buffer: Some(buffer),
    };
    drop(old.life);
    assert!(runtime
        .require_installed_observation(&[43; 32], &old.observation)
        .is_err());
    // This is a negative registry fixture, not a fabricated successful install/Current.
    let retired = std::mem::replace(&mut runtime.buffer.as_mut().unwrap().slots[0], next.slot);
    drop(retired);
    assert!(runtime
        .require_installed_observation(&[43; 32], &old.observation)
        .is_err());
    runtime
        .require_installed_observation(&[43; 32], &next.observation)
        .unwrap();
    drop(next.life);
    drop(runtime);
    assert_eq!(pool.charged_for_test(), 2 * WRITER_METADATA_FEE);
    assert_eq!(old.observation.original_life.strong_count(), 0);
    drop(next.observation);
    assert_eq!(pool.charged_for_test(), WRITER_METADATA_FEE);
    drop(old.observation);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn installed_probe_does_not_take_last_owner_cleanup_from_its_original_thread() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let pool = CriticalCatalogReadPool::new(buffer_bytes().unwrap() + WRITER_METADATA_FEE).unwrap();
    let (notice, dropped_on) = channel();
    let original = negative_generation_with_notice(&pool, 44, Some(notice));
    let mut buffer = SlotBuffer::prepare(&pool).unwrap();
    buffer.slots.push(original.slot);
    let canonical = Arc::new(std::sync::Mutex::new(Runtime {
        buffer: Some(buffer),
    }));
    let (release, released) = channel();
    let (finished, complete) = channel();
    let life = original.life;
    let worker = std::thread::spawn(move || {
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        drop(life);
        // Destructor notice alone can arrive before watch fields drain. Complete means
        // the native sender has actually dropped, so cancellation is deterministically ready.
        finished.send(()).unwrap();
    });
    let original_thread = worker.thread().id();
    let guard = canonical.lock().unwrap();
    BEFORE_INSTALLED_FINAL_CHECK.with(|slot| {
        assert!(slot
            .replace(Some(Box::new(move || {
                release.send(()).unwrap();
                complete.recv_timeout(Duration::from_secs(10)).unwrap();
                assert_eq!(
                    dropped_on.recv_timeout(Duration::from_secs(10)).unwrap(),
                    original_thread
                );
            })))
            .is_none());
    });
    assert!(guard
        .require_installed_observation(&[44; 32], &original.observation)
        .is_err());
    guard.require_no_live_writer(&[44; 32]).unwrap();
    drop(guard);
    worker.join().unwrap();
    drop(canonical);
    assert_eq!(pool.charged_for_test(), WRITER_METADATA_FEE);
    drop(original.observation);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn old_observation_keeps_its_own_generation_paid_after_slot_replacement() {
    let pool = CriticalCatalogReadPool::new(4 * WRITER_METADATA_FEE).unwrap();
    let old = negative_generation(&pool, 1);
    let old_copy = old.observation.clone();
    let original_life = Arc::downgrade(&old.life);
    assert!(!old.observation.is_cancelled());
    drop(old.life);
    assert!(original_life.upgrade().is_none());
    drop(original_life); // test instrumentation must not pin an unpaid old WriterLife body
    assert!(old.observation.is_cancelled());
    drop(old.slot);
    // The latest slot can now refer to another instance. Its fee cannot pay old observers.
    let next = negative_generation(&pool, 1);
    assert_eq!(pool.charged_for_test(), 2 * WRITER_METADATA_FEE);
    drop(old.observation);
    assert_eq!(pool.charged_for_test(), 2 * WRITER_METADATA_FEE);
    drop(old_copy);
    assert_eq!(pool.charged_for_test(), WRITER_METADATA_FEE);
    assert!(!next.observation.is_cancelled());
    drop(next);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn dead_writer_arc_body_stays_paid_until_slot_weak_drains() {
    let pool = CriticalCatalogReadPool::new(WRITER_METADATA_FEE).unwrap();
    let owner = negative_generation(&pool, 2);
    let slot = owner.slot;
    let life = Arc::downgrade(&owner.life);
    drop(owner.life);
    drop(owner.observation);
    assert!(life.upgrade().is_none());
    drop(life);
    // Runtime still holds the original Weak allocation. Its fee must outlive that Weak.
    assert_eq!(pool.charged_for_test(), WRITER_METADATA_FEE);
    drop(slot);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn cancelled_strong_original_is_still_present_until_its_actual_drain() {
    let pool = CriticalCatalogReadPool::new(2 * WRITER_METADATA_FEE).unwrap();
    let owner = negative_generation(&pool, 3);
    let next = negative_generation(&pool, 3);
    owner.life.sender.send_replace(true);
    assert!(owner.observation.is_cancelled());
    // Exercise the SAME classifier invoked by real installation/installed proof. A queued
    // original cleanup excludes the new instance even though its signal is cancelled.
    assert!(classify_original_owner(&owner.slot, &next.life).is_err());
    assert_eq!(
        classify_original_owner(&owner.slot, &owner.life).unwrap(),
        ExistingOwner::SameOriginal
    );
    drop(owner.life);
    assert_eq!(
        classify_original_owner(&owner.slot, &next.life).unwrap(),
        ExistingOwner::Absent
    );
}

#[test]
fn foreign_classification_never_takes_final_sender_cleanup_from_the_original_drop_thread() {
    let required = buffer_bytes().unwrap();
    let pool = CriticalCatalogReadPool::new(required + 2 * WRITER_METADATA_FEE).unwrap();
    let (notice, dropped_on) = std::sync::mpsc::channel();
    let old = negative_generation_with_notice(&pool, 6, Some(notice));
    let proposed = negative_generation(&pool, 6);
    let mut buffer = SlotBuffer::prepare(&pool).unwrap();
    buffer.slots.push(old.slot);
    let canonical = std::sync::Mutex::new(Runtime {
        buffer: Some(buffer),
    });
    let (release, released) = std::sync::mpsc::channel();
    let (drained, completed) = std::sync::mpsc::channel();
    let original = old.life;
    let worker = std::thread::spawn(move || {
        released
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        drop(original);
        drained.send(()).unwrap();
    });
    let original_drop_thread = worker.thread().id();
    BEFORE_FOREIGN_REFUSAL.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            release.send(()).unwrap();
            completed
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
        }));
    });
    let held_canonical = canonical.lock().unwrap();
    let slot = &held_canonical.buffer.as_ref().unwrap().slots[0];
    assert!(classify_original_owner(slot, &proposed.life).is_err());
    // With a temporary foreign upgrade, the original's external Drop would finish but the
    // classifier would perform final WriterLife/Sender cleanup on THIS locked thread.
    assert_eq!(
        dropped_on
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap(),
        original_drop_thread
    );
    assert_eq!(
        classify_original_owner(slot, &proposed.life).unwrap(),
        ExistingOwner::Absent
    );
    drop(held_canonical);
    worker.join().unwrap();
    drop(old.observation);
    drop(canonical);
    drop(proposed);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn canonical_absence_refuses_cancelled_strong_writer_and_preserves_its_inert_fee() {
    let required = buffer_bytes().unwrap();
    let pool = CriticalCatalogReadPool::new(required + WRITER_METADATA_FEE).unwrap();
    let owner = negative_generation(&pool, 7);
    let mut buffer = SlotBuffer::prepare(&pool).unwrap();
    buffer.slots.push(owner.slot);
    let runtime = Runtime {
        buffer: Some(buffer),
    };
    assert!(runtime.require_no_live_writer(&[7; 32]).is_err());
    owner.life.sender.send_replace(true);
    assert!(owner.observation.is_cancelled());
    assert!(runtime.require_no_live_writer(&[7; 32]).is_err());
    assert!(runtime.require_no_live_writer(&[8; 32]).is_ok());
    drop(owner.life);
    assert!(runtime.require_no_live_writer(&[7; 32]).is_ok());
    drop(owner.observation);
    assert_eq!(pool.charged_for_test(), required + WRITER_METADATA_FEE);
    drop(runtime);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn original_cancellation_clone_retains_accounting_without_retaining_sender_or_store() {
    let pool = CriticalCatalogReadPool::new(WRITER_METADATA_FEE).unwrap();
    let owner = negative_generation(&pool, 4);
    let cancellation = owner.observation.cancellation();
    let life = Arc::downgrade(&owner.life);
    drop(owner);
    assert!(life.upgrade().is_none());
    assert!(cancellation.is_cancelled());
    assert_eq!(pool.charged_for_test(), WRITER_METADATA_FEE);
    drop(life);
    drop(cancellation);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn fixed_slot_buffer_is_paid_before_allocation_and_keeps_its_original_pool() {
    BUFFER_ALLOCATIONS.with(|count| count.set(0));
    let required = buffer_bytes().unwrap();
    let short = CriticalCatalogReadPool::new(required - 1).unwrap();
    assert!(SlotBuffer::prepare(&short).is_err());
    assert_eq!(BUFFER_ALLOCATIONS.with(|count| count.get()), 0);
    assert_eq!(short.charged_for_test(), 0);
    let pool = CriticalCatalogReadPool::new(required).unwrap();
    let witness = pool.clone();
    let buffer = SlotBuffer::prepare(&pool).unwrap();
    assert_eq!(BUFFER_ALLOCATIONS.with(|count| count.get()), 1);
    assert_eq!(buffer.slots.capacity(), MAX_WRITERS);
    assert!(buffer.slots.is_empty());
    assert_eq!(pool.charged_for_test(), required);
    drop(pool);
    assert_eq!(witness.charged_for_test(), required);
    drop(buffer);
    assert_eq!(witness.charged_for_test(), 0);
    check_writer_invoice().unwrap();
}

#[derive(Debug)]
struct RefuseReplacementEffect;
#[async_trait::async_trait]
impl RequestAuthority for RefuseReplacementEffect {
    fn authorized(&self) -> bool {
        false
    }
    async fn revoked(&self) {}
}

#[test]
fn replacing_effect_authority_keeps_original_accounting_without_granting_permission() {
    let pool = CriticalCatalogReadPool::new(WRITER_METADATA_FEE).unwrap();
    let owner = negative_generation(&pool, 5);
    let lower = owner
        .observation
        .cancellation()
        .with_authority(Arc::new(RefuseReplacementEffect));
    assert!(lower.is_cancelled());
    assert!(lower.keepalive().is_some());
    drop(owner);
    // The old effect authority is gone. Its SAME generation's keepalive still pays lower
    // ownership; an authority-only fee would refund here while its signal remained retained.
    assert_eq!(pool.charged_for_test(), WRITER_METADATA_FEE);
    drop(lower);
    assert_eq!(pool.charged_for_test(), 0);
}
