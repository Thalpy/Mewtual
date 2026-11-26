//! Closed candidate constructor, dispatch and ownership tests. These fixtures
//! establish SDK behavior independently of Root and native source authority.
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

#[test]
fn private_text_remove_stages_only_the_exact_surviving_membership() {
    let founder = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&founder).unwrap();
    let survivor = MlsDevice::generate().unwrap();
    let target = MlsDevice::generate().unwrap();
    let other = MlsDevice::generate().unwrap();
    let packages = [&survivor, &target, &other].map(|device| device.key_package().unwrap());
    let (_, welcome, _) = group
        .group
        .add_members(founder.provider(), founder.signer(), &packages)
        .unwrap();
    group
        .group
        .merge_pending_commit(founder.provider())
        .unwrap();
    let welcome = welcome.tls_serialize_detached().unwrap();
    let joined = ServerGroup::join(&survivor, &welcome).unwrap();
    let survivor_bytes =
        crate::persist::snapshot_server_bounded(&survivor, &joined, RAW, RAW).unwrap();
    let (mut survivor, mut joined, old_payment) = parent(&survivor_bytes);
    let evicted = ServerGroup::join(&target, &welcome).unwrap();
    let evicted_bytes =
        crate::persist::snapshot_server_bounded(&target, &evicted, RAW, RAW).unwrap();
    let (target, evicted, _) = parent(&evicted_bytes);
    let before = joined.epoch_authenticator_id();
    let before_routing = Zeroizing::new(joined.routing_metadata_secret(&survivor).unwrap());
    let epoch = joined.epoch();
    let founder_bytes =
        crate::persist::snapshot_server_bounded(&founder, &group, RAW, RAW).unwrap();
    let (mut founder, mut group, founder_payment) = parent(&founder_bytes);
    let target_key = target.public_key_bytes().try_into().unwrap();
    let (payment, removal_payment, _) = admission(TextCurrentAddLimits::governed_text_v2());
    let mut removal =
        PreparedTextCurrentRemove::prepare(&founder, &group, target_key, payment).unwrap();
    assert!(removal.completed_data().is_none());
    assert!(removal.execute_private(&survivor, &joined).is_err());
    removal.execute_private(&founder, &group).unwrap();
    let previous = removal.completed_data().unwrap().commit_data().as_ptr();
    removal.execute_private(&founder, &group).unwrap();
    let removed = removal.completed_data().unwrap();
    assert_eq!(removed.commit_data().as_ptr(), previous);
    assert_eq!(group.epoch(), epoch);
    let packet = |subject| TextCurrentReceivedRemoveData {
        commit: removed.commit_data(),
        digest: *blake3::hash(removed.commit_data()).as_bytes(),
        epoch,
        committer: founder.public_key_bytes().try_into().unwrap(),
        subject,
    };
    let wrong = other.public_key_bytes().try_into().unwrap();
    let (payment, retained, _) = admission(TextCurrentAddLimits::governed_text_v2());
    assert!(PreparedTextCurrentReceivedAdd::prepare_remove(
        &survivor,
        &joined,
        packet(wrong),
        payment,
    )
    .is_err());
    let (payment, _, _) = admission(TextCurrentAddLimits::governed_text_v2());
    assert!(VerifiedTextCurrentEviction::verify_own_eviction(
        &survivor,
        &joined,
        packet(*removed.target_data()),
        payment,
    )
    .is_err());
    let mut corrupt = removed.commit_data().to_vec();
    *corrupt.last_mut().unwrap() ^= 1;
    let (payment, _, _) = admission(TextCurrentAddLimits::governed_text_v2());
    assert!(VerifiedTextCurrentEviction::verify_own_eviction(
        &target,
        &evicted,
        TextCurrentReceivedRemoveData {
            commit: &corrupt,
            digest: *blake3::hash(&corrupt).as_bytes(),
            ..packet(*removed.target_data())
        },
        payment,
    )
    .is_err());
    let (payment, evicted_payment, _) = admission(TextCurrentAddLimits::governed_text_v2());
    let eviction = VerifiedTextCurrentEviction::verify_own_eviction(
        &target,
        &evicted,
        packet(*removed.target_data()),
        payment,
    )
    .unwrap();
    assert_eq!(evicted_payment.load(Ordering::SeqCst), 0);
    assert_eq!(
        eviction.removed_data(),
        (epoch + 1, *blake3::hash(removed.commit_data()).as_bytes())
    );
    assert!(eviction.require_original_data(&survivor, &joined).is_err());
    eviction.require_original_data(&target, &evicted).unwrap();
    assert_eq!(evicted.epoch(), epoch);
    assert_eq!(retained.load(Ordering::SeqCst), 0);
    let (payment, _, _) = admission(TextCurrentAddLimits::governed_text_v2());
    assert!(PreparedTextCurrentReceivedAdd::prepare_remove(
        &target,
        &evicted,
        packet(*removed.target_data()),
        payment,
    )
    .is_err());
    let (payment, retained, _) = admission(TextCurrentAddLimits::governed_text_v2());
    let mut candidate = Some(
        PreparedTextCurrentReceivedAdd::prepare_remove(
            &survivor,
            &joined,
            packet(*removed.target_data()),
            payment,
        )
        .unwrap(),
    );
    assert_eq!(
        (joined.epoch(), joined.epoch_authenticator_id()),
        (epoch, before)
    );
    assert_eq!(
        candidate.as_ref().unwrap().selected_epoch_data(),
        removed.selected_epoch_data(),
    );
    let retired =
        PreparedTextCurrentReceivedAdd::prepare_install(&mut candidate, &mut survivor, &mut joined)
            .unwrap()
            .install();
    assert!(candidate.is_none());
    assert_eq!(joined.epoch(), epoch + 1);
    assert!(joined.member_device_ids().contains(&other.device_id()));
    assert!(!joined.member_device_ids().contains(&target.device_id()));
    let selected_routing = Zeroizing::new(joined.routing_metadata_secret(&survivor).unwrap());
    let expected = removed.selected_epoch_data();
    let mut removal = Some(removal);
    assert!(PreparedTextCurrentRemove::prepare_install_into(
        &mut removal,
        &mut survivor,
        &mut joined
    )
    .is_err());
    PreparedTextCurrentRemove::prepare_install_into(&mut removal, &mut founder, &mut group)
        .unwrap()
        .cancel();
    assert_eq!(group.epoch(), epoch);
    assert!(removal.is_some());
    let retired_founder =
        PreparedTextCurrentRemove::prepare_install_into(&mut removal, &mut founder, &mut group)
            .unwrap()
            .install();
    assert!(removal.is_none());
    assert_eq!((group.epoch(), group.epoch_authenticator_id()), expected);
    assert!(selected_routing.as_slice() == group.routing_metadata_secret(&founder).unwrap());
    assert!(selected_routing.as_slice() != before_routing.as_slice());
    assert_ne!(old_payment.load(Ordering::SeqCst), 0);
    drop((retired, joined, survivor));
    assert_eq!(old_payment.load(Ordering::SeqCst), 0);
    assert_eq!(retained.load(Ordering::SeqCst), 0);
    assert_ne!(founder_payment.load(Ordering::SeqCst), 0);
    drop(retired_founder);
    assert_eq!(founder_payment.load(Ordering::SeqCst), 0);
    assert_ne!(removal_payment.load(Ordering::SeqCst), 0);
    drop((founder, group));
    assert_eq!(removal_payment.load(Ordering::SeqCst), 0);
}

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
        routing_typed_bytes: 0,
        routing_work: 0,
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
