//! Real sealed JOIN source, including a cold mount with NO runtime task. Generic old admission
//! must not replace that source's basis. All tests authored UNRUN during the user pause.
use super::*;
use crate::store::{ChatAdmissionCapture, ChatAdmissionKey, ChatAdmissionPreparation};
use rand_core::{CryptoRng, RngCore};

struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("retained JOIN source reached generic RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("retained JOIN source reached generic RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("retained JOIN source reached generic RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("retained JOIN source reached generic RNG")
    }
}
impl CryptoRng for NeverRng {}
fn key(row: &codec::Metadata) -> ChatAdmissionKey {
    ChatAdmissionKey::new(row.group.as_slice(), row.signer).unwrap()
}
fn other_control(store: &ServerStore, seed: u8) {
    let key = ChatAdmissionKey::new(&[seed], [seed; 32]).unwrap();
    let capture = store.capture_chat_admission(key).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(seed as u64);
    let preparation = store
        .prepare_legacy_chat_admission(&capture, b"unrelated actual control", &mut rng)
        .unwrap();
    let mut called = false;
    let current = store
        .finish_chat_admission(&preparation, &mut rng, |_, _| {
            called = true;
            Ok(())
        })
        .unwrap()
        .into_binding();
    assert!(called);
    assert!(store
        .with_chat_admission_storage(&current, |_| Ok(()))
        .is_ok());
}
fn original_legacy(
    f: &Fixture,
    device: &MlsDevice,
    invite: &InviteToken,
) -> (ChatAdmissionCapture, ChatAdmissionPreparation) {
    let signer = device.public_key_bytes().try_into().unwrap();
    let capture = f
        .member()
        .capture_chat_admission(ChatAdmissionKey::new(&invite.group_id, signer).unwrap())
        .unwrap();
    let preparation = f
        .member()
        .prepare_legacy_chat_admission(
            &capture,
            b"original pre-source lineage",
            &mut ChaCha20Rng::seed_from_u64(0xfa01),
        )
        .unwrap();
    (capture, preparation)
}
fn same_device_source(
    f: &mut Fixture,
    device: MlsDevice,
    invite: InviteToken,
) -> JournalledCheckpointJoinPreparation {
    let mut source = f
        .member
        .as_deref()
        .unwrap()
        .prepare_journalled_owner_checkpoint_join(
            &f.source,
            &f.owner,
            device,
            invite,
            &mut f.member_rng,
        )
        .unwrap();
    source
        .save_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(0xfa02))
        .unwrap();
    source
}

#[test]
fn cold_paused_source_without_task_blocks_generic_fresh_and_legacy_before_rng() {
    let mut f = Fixture::new();
    let invite = f.invite(90);
    let mut source = f.begin(invite);
    source
        .save_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(0xfa03))
        .unwrap();
    source
        .pause_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(0xfa04))
        .unwrap();
    let identity = source.metadata.clone();
    let before = f.current_bytes();
    drop(source);
    f.reopen();
    assert!(f
        .member()
        .pending_join_catalog
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .networks
        .iter()
        .all(Option::is_none));
    let capture = f.member().capture_chat_admission(key(&identity)).unwrap();
    assert!(f
        .member()
        .prepare_legacy_chat_admission(&capture, b"must not replace stored source", &mut NeverRng)
        .is_err());
    assert!(f
        .member()
        .prepare_fresh_chat_admission(&capture, [9; 32], &mut NeverRng)
        .is_err());
    assert_eq!(f.current_bytes(), before);
    other_control(f.member(), 91);
    assert_eq!(
        f.current_bytes(),
        before,
        "other key control cannot alter the retained source"
    );
}

#[test]
fn preparation_captured_before_source_save_cannot_finish_or_resume_initializing_but_can_retire() {
    let mut f = Fixture::new();
    let invite = f.invite(92);
    let device = MlsDevice::generate().unwrap();
    let (old_capture, old) = original_legacy(&f, &device, &invite);
    let source = same_device_source(&mut f, device, invite);
    let before = f.current_bytes();
    assert!(
        f.member()
            .prepare_legacy_chat_admission(
                &old_capture,
                b"original pre-source lineage",
                &mut NeverRng
            )
            .is_err(),
        "issued retry must not precede persistent exclusion"
    );
    let mut called = false;
    assert!(f
        .member()
        .finish_chat_admission(&old, &mut NeverRng, |_, _| {
            called = true;
            Ok(())
        })
        .is_err());
    assert!(
        !called,
        "old covering callback must never run after source admission"
    );
    assert!(f
        .member()
        .resolve_chat_admission(&key(&source.metadata), old.nonce())
        .is_err());
    f.member()
        .retire_chat_preparation(&old, &mut ChaCha20Rng::seed_from_u64(0xfa05))
        .unwrap();
    assert_eq!(
        f.current_bytes(),
        before,
        "negative retirement preserves exact source DATA"
    );
    other_control(f.member(), 93);
}

#[test]
fn preexisting_current_and_issued_dirty_retry_cannot_overtake_source_but_negative_control_survives()
{
    let mut f = Fixture::new();
    let invite = f.invite(94);
    let device = MlsDevice::generate().unwrap();
    let (old_capture, old) = original_legacy(&f, &device, &invite);
    let mut rng = ChaCha20Rng::seed_from_u64(0xfa06);
    let binding = f
        .member()
        .finish_chat_admission(&old, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    // Another actual row supplies a real dirty negative-save refusal, without poking private
    // admission internals. Its repair must remain independent from this JOIN reservation.
    let other_key = ChatAdmissionKey::new(&[95], [95; 32]).unwrap();
    let other_capture = f.member().capture_chat_admission(other_key).unwrap();
    let other_prepared = f
        .member()
        .prepare_legacy_chat_admission(&other_capture, b"other real row", &mut rng)
        .unwrap();
    let other = f
        .member()
        .finish_chat_admission(&other_prepared, &mut rng, |_, _| Ok(()))
        .unwrap()
        .into_binding();
    let source = same_device_source(&mut f, device, invite);
    let before = f.current_bytes();
    assert!(f
        .member()
        .resolve_chat_admission(&key(&source.metadata), old.nonce())
        .is_err());
    let mut called = false;
    assert!(f
        .member()
        .with_chat_admission_storage(&binding, |_| {
            called = true;
            Ok(())
        })
        .is_err());
    assert!(!called);
    assert!(f
        .member()
        .finish_chat_admission(&old, &mut NeverRng, |_, _| panic!(
            "AlreadyCurrent source must not save"
        ))
        .is_err());
    f.member()
        .chat_fence_pre_rename_failure
        .store(true, Ordering::SeqCst);
    assert!(f.member().retire_chat_admission(&other, &mut rng).is_err());
    assert!(
        f.member()
            .prepare_legacy_chat_admission(
                &old_capture,
                b"original pre-source lineage",
                &mut NeverRng
            )
            .is_err(),
        "old issued retry cannot flush dirty inventory/RNG before source check"
    );
    f.member()
        .chat_fence_pre_rename_failure
        .store(false, Ordering::SeqCst);
    f.member()
        .repair_chat_admission_inventory(&mut rng)
        .unwrap();
    assert!(
        f.member()
            .with_original_chat_store(&binding, |_| Ok(()))
            .is_ok(),
        "same-original negative repair remains allowed"
    );
    f.member()
        .retire_chat_admission(&binding, &mut rng)
        .unwrap();
    assert_eq!(f.current_bytes(), before);
    other_control(f.member(), 96);
}

#[test]
fn store_admitted_unwritten_source_candidate_reserves_key_before_current_file_contains_it() {
    let mut f = Fixture::new();
    let invite = f.invite(97);
    let mut source = f.begin(invite);
    let identity = source.metadata.clone();
    let old = f.current_bytes();
    f.member()
        .critical_namespace_fault
        .store(3, Ordering::SeqCst);
    assert!(source
        .save_pending(f.member(), &mut ChaCha20Rng::seed_from_u64(0xfa07))
        .is_err());
    let candidate = {
        let mounted = f.member().pending_join_catalog.lock().unwrap();
        mounted
            .as_ref()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .bytes
            .to_vec()
    };
    assert_eq!(f.current_bytes(), old);
    drop(source);
    let capture = f.member().capture_chat_admission(key(&identity)).unwrap();
    assert!(f
        .member()
        .prepare_legacy_chat_admission(&capture, b"candidate is durable source debt", &mut NeverRng)
        .is_err());
    assert!(f
        .member()
        .prepare_fresh_chat_admission(&capture, [9; 32], &mut NeverRng)
        .is_err());
    other_control(f.member(), 98);
    f.member()
        .critical_namespace_fault
        .store(0, Ordering::SeqCst);
    f.member().retry_pending_join_catalog().unwrap();
    assert_eq!(f.current_bytes(), candidate);
}
