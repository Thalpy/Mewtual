//! Constructor/fork tests only. No private Add SDK dispatch, Current, Source,
//! governance or publication role is supplied by these resource-only fixtures.
use super::*;
use crate::config::CIPHERSUITE;
use crate::persist::text_current_add::{TextCurrentAddLimits, TextCurrentAddReservation};
use crate::persist::{
    restore_server_text_current_bounded, TextCurrentSdkGraphReservation,
    TextCurrentSdkRestoreAdmission, TextCurrentSdkRestoreLimits,
};
use openmls_memory_storage::owner_restore::TextAddOperationLimits;
use openmls_traits::OpenMlsProvider;
use std::sync::atomic::{AtomicUsize, Ordering};
const RAW: usize = 2 * 1024 * 1024;
const TYPED: usize = 2 * 1024 * 1024;
const WORK: usize = 2 * 1024 * 1024 * 1024 - 1;
const NOW: u64 = 1_800_000_000_000;

struct OldFee(Arc<AtomicUsize>);
impl TextCurrentSdkGraphReservation for OldFee {
    fn limits(&self) -> TextCurrentSdkRestoreLimits {
        TextCurrentSdkRestoreLimits {
            max_typed_bytes: TYPED,
            max_work: WORK,
        }
    }
}
impl Drop for OldFee {
    fn drop(&mut self) {
        assert_eq!(self.0.fetch_sub(TYPED, Ordering::SeqCst), TYPED);
    }
}
struct Fee {
    used: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    bytes: usize,
    limits: TextCurrentAddLimits,
}
impl TextCurrentAddReservation for Fee {
    fn limits(&self) -> TextCurrentAddLimits {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.limits
    }
}
impl Drop for Fee {
    fn drop(&mut self) {
        assert_eq!(
            self.used.fetch_sub(self.bytes, Ordering::SeqCst),
            self.bytes
        );
    }
}
fn limits() -> TextCurrentAddLimits {
    TextCurrentAddLimits {
        max_snapshot_bytes: 4 * RAW,
        max_provider_backing_bytes: RAW,
        max_provider_payload_bytes: 8 * RAW,
        prepare_work: WORK,
        package_typed_bytes: 1024 * 1024,
        package_work: WORK,
        restore_typed_bytes: TYPED,
        restore_work: WORK,
        operation: TextAddOperationLimits {
            max_typed_bytes: TYPED,
            max_serialized_bytes: 16 * RAW,
            max_provider_bytes: 16 * RAW,
            max_work: WORK,
        },
        sdk_typed_bytes: 32 * RAW,
        sdk_work: WORK,
        output_bytes: 4 * RAW,
    }
}
fn admission(
    limits: TextCurrentAddLimits,
) -> (TextCurrentAddAdmission, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let bytes = limits.buffer_reservation_bytes().unwrap()
        + TextCurrentAddAdmission::metadata_layout_bytes::<Fee>().unwrap();
    let used = Arc::new(AtomicUsize::new(bytes));
    let calls = Arc::new(AtomicUsize::new(0));
    let paid = TextCurrentAddAdmission::new(Fee {
        used: used.clone(),
        calls: calls.clone(),
        bytes,
        limits,
    })
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    (paid, used, calls)
}

fn source(members: usize, remove_interior: bool) -> Zeroizing<Vec<u8>> {
    let owner = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let mut interior = None;
    for i in 1..members {
        let peer = MlsDevice::generate().unwrap();
        if i == 3 {
            interior = Some(peer.device_id());
        }
        group
            .add_member(&owner, peer.key_package().unwrap())
            .unwrap();
    }
    if remove_interior {
        group.remove_member(&owner, &interior.unwrap()).unwrap();
    }
    crate::persist::snapshot_server_bounded(&owner, &group, RAW, RAW).unwrap()
}
fn parent(bytes: &[u8]) -> (MlsDevice, ServerGroup, Arc<AtomicUsize>) {
    let used = Arc::new(AtomicUsize::new(TYPED));
    let (d, g) = restore_server_text_current_bounded(
        bytes,
        RAW,
        RAW,
        TextCurrentSdkRestoreAdmission::new(OldFee(used.clone())).unwrap(),
    )
    .unwrap();
    (d, g, used)
}
fn request(group: &[u8]) -> (MlsDevice, Vec<u8>, [u8; 32], [u8; 32]) {
    let peer = MlsDevice::generate().unwrap();
    let credential = CredentialWithKey {
        credential: BasicCredential::new(
            crate::MembershipCredential {
                device_id: peer.device_id(),
                group_id: group.to_vec(),
                invite_nonce: [29; 16],
            }
            .encode(),
        )
        .into(),
        signature_key: peer.public_key_ref().into(),
    };
    let kp = KeyPackage::builder()
        .leaf_node_capabilities(crate::config::capabilities())
        .key_package_lifetime(Lifetime::init(
            NOW / 1000 - 60,
            NOW / 1000 + 60 * 24 * 60 * 60,
        ))
        .build(CIPHERSUITE, peer.provider(), peer.signer(), credential)
        .unwrap();
    let frame = crate::serialize_key_package(kp.key_package()).unwrap();
    let reference = peer.key_package_reference(kp.key_package()).unwrap();
    let subject = peer.public_key_ref().try_into().unwrap();
    (peer, frame, subject, reference)
}
fn input(frame: &[u8], subject: [u8; 32], reference: [u8; 32]) -> TextCurrentAddRequestData<'_> {
    TextCurrentAddRequestData::from_untrusted_data(frame, subject, [29; 16], reference, NOW)
}

#[test]
fn real20_and50_member_parent_forks_keep_both_public_mutation_fences_and_original_bytes() {
    for count in [20, 50] {
        let source = source(count, false);
        let (d, mut g, old_used) = parent(&source);
        let before = crate::snapshot_server(&d, &g).unwrap();
        let epoch = g.epoch();
        let authenticator = g.epoch_authenticator_id();
        let (_peer, frame, subject, reference) = request(g.group_id_ref());
        let (paid, used, calls) = admission(limits());
        let mut paid = Some(paid);
        let mut cell = None;
        PreparedTextCurrentAdd::prepare_into(
            &d,
            &g,
            input(&frame, subject, reference),
            &mut paid,
            &mut cell,
        )
        .unwrap();
        assert!(paid.is_none());
        let cell = cell.as_mut().unwrap();
        assert_eq!(cell.original_epoch_data(), epoch);
        assert!(!cell.is_complete_data());
        assert!(cell.outputs.is_none() && cell.snapshot.is_none());
        assert!(matches!(
            g.create_application_message(&d, b"closed"),
            Err(MlsError::Internal(_))
        ));
        assert!(matches!(
            cell.graph
                .group
                .create_application_message(&cell.graph.device, b"closed"),
            Err(MlsError::Internal(_))
        ));
        assert!(matches!(d.key_package(), Err(MlsError::Internal(_))));
        assert!(matches!(
            cell.graph.device.key_package(),
            Err(MlsError::Internal(_))
        ));
        assert_eq!(g.epoch(), epoch);
        assert_eq!(g.epoch_authenticator_id(), authenticator);
        assert!(crate::snapshot_server(&d, &g).unwrap().as_slice() == before.as_slice());
        assert!(!Arc::ptr_eq(
            d.sdk_graph_lease.as_ref().unwrap(),
            cell.graph.device.sdk_graph_lease.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            cell.graph.device.sdk_graph_lease.as_ref().unwrap(),
            cell.graph.group.sdk_graph_lease.as_ref().unwrap()
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(used.load(Ordering::SeqCst) > 0 && old_used.load(Ordering::SeqCst) == TYPED);
    }
}

#[test]
fn cross_pair_and_package_refusal_preserve_inputs_without_resetting_spent_preparation_credit() {
    let source = source(3, false);
    let (d, g, _old) = parent(&source);
    let (_other_d, other_g, _other_old) = parent(&source);
    let (_peer, frame, subject, reference) = request(g.group_id_ref());
    let before = crate::snapshot_server(&d, &g).unwrap();
    let (paid, used, _) = admission(limits());
    let mut paid = Some(paid);
    let mut cell = None;
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &other_g,
        input(&frame, subject, reference),
        &mut paid,
        &mut cell
    )
    .is_err());
    assert!(paid.is_some() && cell.is_none());
    let mut wrong = reference;
    wrong[0] ^= 1;
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, wrong),
        &mut paid,
        &mut cell
    )
    .is_err());
    assert!(paid.is_some() && cell.is_none());
    assert!(crate::snapshot_server(&d, &g).unwrap().as_slice() == before.as_slice());
    // The wrong reference was discovered only after real typed decoding and
    // crypto. Its cumulative debit stays spent despite the preserved input.
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut paid,
        &mut cell
    )
    .is_err());
    assert!(paid.is_some() && cell.is_none());
    drop(paid);
    assert_eq!(used.load(Ordering::SeqCst), 0);
    // All partial locals have drained. A NEW independent debit can prepare the
    // SAME unchanged source and valid original request; no SDK Add ran above.
    let (fresh, used, _) = admission(limits());
    let mut paid = Some(fresh);
    PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut paid,
        &mut cell,
    )
    .unwrap();
    assert!(paid.is_none() && cell.is_some());
    PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut paid,
        &mut cell,
    )
    .unwrap();
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, wrong),
        &mut paid,
        &mut cell
    )
    .is_err());
    drop(cell);
    assert_eq!(used.load(Ordering::SeqCst), 0);
    let mut short = limits();
    short.package_typed_bytes = 1;
    let (short, short_used, _) = admission(short);
    let mut short = Some(short);
    let mut cell = None;
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut short,
        &mut cell
    )
    .is_err());
    assert!(short.is_some() && cell.is_none());
    drop(short);
    assert_eq!(short_used.load(Ordering::SeqCst), 0);
}

#[test]
fn real64_slots_full_refuses_blank_reuse_prepares_and_fee_last_survives_both_parent_drop_orders() {
    for remove in [false, true] {
        let bytes = source(64, remove);
        let (d, g, old_used) = parent(&bytes);
        let (_peer, frame, subject, reference) = request(g.group_id_ref());
        let (paid, used, _) = admission(limits());
        let mut paid = Some(paid);
        let mut cell = None;
        let result = PreparedTextCurrentAdd::prepare_into(
            &d,
            &g,
            input(&frame, subject, reference),
            &mut paid,
            &mut cell,
        );
        if !remove {
            assert!(result.is_err() && paid.is_some() && cell.is_none());
            drop(d);
            assert_eq!(old_used.load(Ordering::SeqCst), TYPED);
            drop(g);
            assert_eq!(old_used.load(Ordering::SeqCst), 0);
            drop(paid);
            assert_eq!(used.load(Ordering::SeqCst), 0);
        } else {
            result.unwrap();
            assert!(paid.is_none());
            drop(g);
            assert_eq!(old_used.load(Ordering::SeqCst), TYPED);
            drop(d);
            assert_eq!(old_used.load(Ordering::SeqCst), TYPED);
            assert!(used.load(Ordering::SeqCst) > 0);
            // The original accounting Arc is strong, but holds no SDK graph.
            // Candidate payloads and both graph halves drain before either fee.
            drop(cell);
            assert_eq!(used.load(Ordering::SeqCst), 0);
            assert_eq!(old_used.load(Ordering::SeqCst), 0);
        }
    }
}

mod worker;
