//! Real merged candidate and genuine Welcome recipient. These prove private
//! cryptographic/resource behavior, not native or Root publication authority.
use super::*;
use catcoms_rt::{rng::RngError, CryptoRng, RngCore};
use catcoms_wire::Decoder;
use rand_chacha::{rand_core::SeedableRng, ChaCha20Rng};

struct CountRng {
    inner: ChaCha20Rng,
    draws: usize,
}
impl CountRng {
    fn new() -> Self {
        Self {
            inner: ChaCha20Rng::seed_from_u64(0xd801),
            draws: 0,
        }
    }
}
impl RngCore for CountRng {
    fn next_u32(&mut self) -> u32 {
        panic!("seal must request its exact nonce")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("seal must request its exact nonce")
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        assert_eq!(bytes.len(), 24);
        self.draws += 1;
        self.inner.fill_bytes(bytes);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), RngError> {
        self.fill_bytes(bytes);
        Ok(())
    }
}
impl CryptoRng for CountRng {}
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("no new randomness")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("no new randomness")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("no new randomness")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), RngError> {
        panic!("no new randomness")
    }
}
impl CryptoRng for NeverRng {}

fn routed(
    d: &MlsDevice,
    g: &ServerGroup,
    plain: &[u8],
) -> (MlsDevice, PreparedTextCurrentAdd, Arc<AtomicUsize>) {
    let (peer, frame, subject, reference) = request(g.group_id_ref());
    let mut caps = limits();
    caps.routing_typed_bytes = TEXT_ADD_ROUTING_TYPED_BYTES;
    caps.routing_work = TEXT_ADD_ROUTING_WORK;
    let (paid, used, _) = admission(caps);
    let mut paid = Some(paid);
    let mut cell = None;
    PreparedTextCurrentAdd::prepare_with_routing_into(
        d,
        g,
        input(&frame, subject, reference),
        plain,
        &mut paid,
        &mut cell,
    )
    .unwrap();
    assert!(paid.is_none());
    (peer, cell.unwrap(), used)
}

fn blob(frame: &[u8]) -> catcoms_crypto::SealedBlob {
    let mut d = Decoder::new(frame);
    let nonce = d.get_bytes().unwrap().try_into().unwrap();
    let ciphertext = d.get_bytes().unwrap().to_vec();
    d.finish().unwrap();
    catcoms_crypto::SealedBlob { nonce, ciphertext }
}

#[test]
fn actual_joiner_opens_same_candidate_transfer_and_old_epoch_or_group_cannot() {
    let (d, g, _old_fee) = parent(&source(1, false));
    let plain = Zeroizing::new(vec![0x59; MAX_TEXT_ADD_ROUTING_PLAIN_BYTES]);
    let old_key = Zeroizing::new(g.routing_transfer_key(&d).unwrap());
    let (peer, mut cell, used) = routed(&d, &g, &plain);
    let mut rng = CountRng::new();
    cell.execute_private_with_routing(&mut rng).unwrap();
    assert_eq!(rng.draws, 1);
    let result = cell.completed_result_data().unwrap();
    let joined = ServerGroup::join(&peer, result.welcome_data()).unwrap();
    let key = Zeroizing::new(joined.routing_transfer_key(&peer).unwrap());
    let sealed = blob(result.sealed_routing_transfer_data().unwrap());
    let opened = Zeroizing::new(catcoms_crypto::unseal(&key, &sealed).unwrap());
    assert!(opened.as_slice() == plain.as_slice());
    assert!(catcoms_crypto::unseal(&old_key, &sealed).is_err());
    let foreign = MlsDevice::generate().unwrap();
    let other = ServerGroup::create(&foreign).unwrap();
    let wrong = Zeroizing::new(other.routing_transfer_key(&foreign).unwrap());
    assert!(catcoms_crypto::unseal(&wrong, &sealed).is_err());
    cell.execute_private_with_routing(&mut NeverRng).unwrap();
    assert_eq!(rng.draws, 1);
    assert!(used.load(Ordering::SeqCst) > 0);
    drop(cell);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn routing_constructors_and_executors_refuse_cross_mode_before_work() {
    let (d, g, _old_fee) = parent(&source(1, false));
    let (_peer, mut cell, _used) = routed(&d, &g, b"original frozen routing");
    assert!(cell.execute_private().is_err());
    assert_eq!(cell.faults.sdk_dispatches, 0);
    let mut rng = CountRng::new();
    cell.execute_private_with_routing(&mut rng).unwrap();
    let frame = cell.package_frame.clone();
    let subject = cell.subject;
    let reference = cell.reference;
    let mut slot = Some(cell);
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut None,
        &mut slot,
    )
    .is_err());
    assert!(PreparedTextCurrentAdd::prepare_with_routing_into(
        &d,
        &g,
        input(&frame, subject, reference),
        b"changed",
        &mut None,
        &mut slot,
    )
    .is_err());
    PreparedTextCurrentAdd::prepare_with_routing_into(
        &d,
        &g,
        input(&frame, subject, reference),
        b"original frozen routing",
        &mut None,
        &mut slot,
    )
    .unwrap();
    let (_peer, mut plain_cell, _fee, _, _, _) = prepared(&d, &g);
    assert!(plain_cell
        .execute_private_with_routing(&mut NeverRng)
        .is_err());
    assert_eq!(plain_cell.faults.sdk_dispatches, 0);
    plain_cell.execute_private().unwrap();
    assert!(plain_cell
        .completed_result_data()
        .unwrap()
        .sealed_routing_transfer_data()
        .is_err());
}

#[test]
fn blob_and_frame_return_loss_never_export_seal_or_draw_again() {
    for cut in [TestCut::RoutingBlobCached, TestCut::RoutingFrameCached] {
        let (d, g, _old_fee) = parent(&source(1, false));
        let (peer, mut cell, _used) = routed(&d, &g, b"one admitted private transfer");
        cell.faults.fail_at = Some(cut);
        let mut rng = CountRng::new();
        assert!(cell.execute_private_with_routing(&mut rng).is_err());
        assert_eq!(rng.draws, 1);
        assert!(cell.completed_result_data().is_err());
        assert_eq!(cell.faults.routing_exports, 1);
        cell.execute_private_with_routing(&mut NeverRng).unwrap();
        assert_eq!(cell.faults.routing_exports, 1);
        let result = cell.completed_result_data().unwrap();
        let sealed = blob(result.sealed_routing_transfer_data().unwrap());
        let joined = ServerGroup::join(&peer, result.welcome_data()).unwrap();
        let key = Zeroizing::new(joined.routing_transfer_key(&peer).unwrap());
        assert_eq!(
            catcoms_crypto::unseal(&key, &sealed).unwrap(),
            b"one admitted private transfer"
        );
    }
}

#[test]
fn routing_rng_unwind_is_terminal_and_keeps_candidate_and_fee() {
    let (d, g, _old_fee) = parent(&source(1, false));
    let (_peer, mut cell, used) = routed(&d, &g, b"uncertain nonce entry");
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cell.execute_private_with_routing(&mut NeverRng)
    }))
    .is_err());
    // This returns a typed error BEFORE reaching NeverRng again.
    assert!(cell.execute_private_with_routing(&mut NeverRng).is_err());
    assert!(cell.completed_result_data().is_err());
    assert_eq!(g.epoch(), 0);
    assert!(used.load(Ordering::SeqCst) > 0);
    drop(cell);
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn routing_credit_shortfalls_and_oversize_refuse_before_preparation() {
    for (typed, work) in [
        (TEXT_ADD_ROUTING_TYPED_BYTES - 1, TEXT_ADD_ROUTING_WORK),
        (TEXT_ADD_ROUTING_TYPED_BYTES, TEXT_ADD_ROUTING_WORK - 1),
        (0, TEXT_ADD_ROUTING_WORK),
        (TEXT_ADD_ROUTING_TYPED_BYTES, 0),
    ] {
        let mut caps = limits();
        caps.routing_typed_bytes = typed;
        caps.routing_work = work;
        let bytes = caps.buffer_reservation_bytes().unwrap()
            + TextCurrentAddAdmission::metadata_layout_bytes::<Fee>().unwrap();
        let used = Arc::new(AtomicUsize::new(bytes));
        let calls = Arc::new(AtomicUsize::new(0));
        assert!(TextCurrentAddAdmission::new(Fee {
            used: used.clone(),
            calls: calls.clone(),
            bytes,
            limits: caps,
        })
        .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
    let (d, g, _old_fee) = parent(&source(1, false));
    let (_peer, frame, subject, reference) = request(g.group_id_ref());
    let mut caps = limits();
    caps.routing_typed_bytes = TEXT_ADD_ROUTING_TYPED_BYTES;
    caps.routing_work = TEXT_ADD_ROUTING_WORK;
    let (paid, _used, _) = admission(caps);
    let mut paid = Some(paid);
    let mut cell = None;
    // A routing-funded operation cannot silently enter the old mode.
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut paid,
        &mut cell,
    )
    .is_err());
    let over = Zeroizing::new(vec![7; MAX_TEXT_ADD_ROUTING_PLAIN_BYTES + 1]);
    assert!(PreparedTextCurrentAdd::prepare_with_routing_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &over,
        &mut paid,
        &mut cell,
    )
    .is_err());
    assert!(cell.is_none() && paid.is_some());
    // Both refusals precede preparation entry, so the SAME fee still enters
    // once with bounded valid input. No entered SDK work was refunded.
    PreparedTextCurrentAdd::prepare_with_routing_into(
        &d,
        &g,
        input(&frame, subject, reference),
        b"fixed",
        &mut paid,
        &mut cell,
    )
    .unwrap();
    assert_eq!(cell.as_ref().unwrap().faults.sdk_dispatches, 0);
}

#[test]
fn two_adds_keep_supplied_l0_secret_but_use_their_own_welcome_epoch_key() {
    let (mut d, mut g, _old_fee) = parent(&source(1, false));
    let original_l0 = Zeroizing::new(g.routing_metadata_secret(&d).unwrap());
    // Routing provenance belongs to the Sync producer; here the input is a
    // genuine original secret, frozen unchanged through two private Adds.
    let plain = Zeroizing::new(original_l0.as_slice().to_vec());
    let mut rng = CountRng::new();
    let mut prior_key: Option<Zeroizing<[u8; 32]>> = None;
    let mut charges = Vec::new();
    for epoch in 1..=2 {
        let (peer, mut cell, used) = routed(&d, &g, &plain);
        cell.execute_private_with_routing(&mut rng).unwrap();
        let result = cell.completed_result_data().unwrap();
        let joined = ServerGroup::join(&peer, result.welcome_data()).unwrap();
        assert_eq!(joined.epoch(), epoch);
        let wrap = Zeroizing::new(joined.routing_transfer_key(&peer).unwrap());
        let sealed = blob(result.sealed_routing_transfer_data().unwrap());
        let opened = Zeroizing::new(catcoms_crypto::unseal(&wrap, &sealed).unwrap());
        assert!(opened.as_slice() == original_l0.as_slice());
        if let Some(old) = &prior_key {
            assert!(catcoms_crypto::unseal(old, &sealed).is_err());
        }
        assert!(joined.routing_metadata_secret(&peer).unwrap() != *original_l0);
        prior_key = Some(wrap);
        charges.push(used);
        let mut slot = Some(cell);
        let install =
            PreparedTextCurrentAdd::prepare_install_into(&mut slot, &mut d, &mut g).unwrap();
        let retired = install.install();
        drop(retired);
        assert_eq!(g.epoch(), epoch);
    }
    assert_eq!(rng.draws, 2);
    assert_eq!(charges[0].load(Ordering::SeqCst), 0);
    assert!(charges[1].load(Ordering::SeqCst) > 0);
    drop(d);
    drop(g);
    assert_eq!(charges[1].load(Ordering::SeqCst), 0);
}

#[test]
fn signed_credential_expectation_refuses_before_secret_snapshot_or_restore() {
    let (d, g, _old_fee) = parent(&source(1, false));
    let (peer, frame, subject, reference) = request(g.group_id_ref());
    let credential = *peer
        .retained_key_package_identity(&frame)
        .unwrap()
        .credential_digest();
    let mut wrong = credential;
    wrong[0] ^= 1;
    let (paid, _used, _) = admission(limits());
    let mut paid = Some(paid);
    let mut cell = None;
    let before = crate::persist::snapshot_encoding_entries_for_test();
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference).with_expected_credential_data(wrong),
        &mut paid,
        &mut cell,
    )
    .is_err());
    assert_eq!(crate::persist::snapshot_encoding_entries_for_test(), before);
    assert!(paid.is_some() && cell.is_none());
    // The failed public check spent its one preparation. Another independent
    // fee is required; changing the request cannot reset that debit.
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference).with_expected_credential_data(credential),
        &mut paid,
        &mut cell,
    )
    .is_err());
    assert_eq!(crate::persist::snapshot_encoding_entries_for_test(), before);
    drop(paid);
    let (fresh, _used, _) = admission(limits());
    paid = Some(fresh);
    PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference).with_expected_credential_data(credential),
        &mut paid,
        &mut cell,
    )
    .unwrap();
    assert!(crate::persist::snapshot_encoding_entries_for_test() > before);
    // An existing bound Cell cannot be retried through the weaker old input.
    assert!(PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut None,
        &mut cell,
    )
    .is_err());
    cell.as_mut().unwrap().execute_private().unwrap();
    let result = cell.as_ref().unwrap().completed_result_data().unwrap();
    assert_eq!(
        result.membership_change_data().credential_digest(),
        Some(credential)
    );
    result
        .require_request_data(
            input(&frame, subject, reference).with_expected_credential_data(credential),
        )
        .unwrap();
    assert!(result
        .require_request_data(
            input(&frame, subject, reference).with_expected_credential_data(wrong),
        )
        .is_err());
    assert!(result
        .require_request_data(input(&frame, subject, reference))
        .is_err());

    let (legacy, _used, _) = admission(limits());
    let mut legacy = Some(legacy);
    let mut legacy_cell = None;
    PreparedTextCurrentAdd::prepare_into(
        &d,
        &g,
        input(&frame, subject, reference),
        &mut legacy,
        &mut legacy_cell,
    )
    .unwrap();
    legacy_cell.as_mut().unwrap().execute_private().unwrap();
    let legacy_result = legacy_cell
        .as_ref()
        .unwrap()
        .completed_result_data()
        .unwrap();
    legacy_result
        .require_request_data(input(&frame, subject, reference))
        .unwrap();
    assert!(legacy_result
        .require_request_data(
            input(&frame, subject, reference).with_expected_credential_data(credential),
        )
        .is_err());
}
