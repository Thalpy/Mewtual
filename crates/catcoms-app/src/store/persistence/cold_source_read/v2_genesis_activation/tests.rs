//! Genuine SourceSaved/new paid read/SDK capsule -> actual prepaid candidate encodes.
//! Physical namespace/seed/head/scope coverage additionally lives in the owning caller's
//! tests; candidate-only success cannot establish application or native acceptance.
use super::super::tests::fixture;
use super::*;
use catcoms_rt::{CryptoRng, RngCore};
use rand_chacha::ChaCha20Rng;

fn budget() -> GenesisSeedBudget {
    GenesisSeedBudget::new(
        8 * 1024 * 1024 + 64 * 1024,
        8 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        16 * 1024 * 1024,
    )
    .unwrap()
}
struct NeverRng;
impl RngCore for NeverRng {
    fn next_u32(&mut self) -> u32 {
        panic!("unexpected RNG before actual metadata payment")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("unexpected RNG before actual metadata payment")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("unexpected RNG before actual metadata payment")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("unexpected RNG before actual metadata payment")
    }
}
impl CryptoRng for NeverRng {}

#[test]
fn actual_source_and_original_app_name_produce_exact_paid_pin_head_seed_candidates() {
    let mut f = fixture(0xda01);
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let source = f.store.prepare_genesis_seed(read, budget()).unwrap();
    let baseline = f.pool.charged_for_test();
    let prepared =
        PreparedV2GenesisActivation::prepare(&f.store, &source.borrow_source_data(), &mut f.rng)
            .unwrap();
    assert_eq!(
        f.pool.charged_for_test(),
        baseline + codec::ACTIVATION_METADATA_FEE
    );
    let candidates = &prepared.candidates;
    assert_eq!(candidates.expected.context, *source.data().context);
    assert_eq!(candidates.expected.source_id, source.data().source);
    assert_eq!(candidates.expected.debit_display_name, "actual cold source");
    let key = Zeroizing::new(f.store.keys.db_key().unwrap());
    let plain = codec::open_activation_seed_data(&key, &candidates.seed).unwrap();
    let opened = codec::parse_activation_seed_data(&key, &candidates.expected, &plain).unwrap();
    assert_eq!(opened.pin_frame, candidates.pin.as_slice());
    assert_eq!(opened.head_frame, candidates.head.as_slice());
    let public = source.data();
    opened
        .head
        .require_original_public_data(
            &opened.pin,
            &codec::GenesisPublicData {
                profile: public.profile,
                admission: public.admission,
                header: public.header,
                image: public.state,
            },
        )
        .unwrap();
    assert!(
        prepared.seed_file.is_none() && prepared.pin_file.is_none() && prepared.head_file.is_none()
    );
    assert!(prepared.saved_seed.is_none() && prepared.saved_head.is_none());
    assert_eq!(
        format!("{prepared:?}"),
        "PreparedV2GenesisActivation { .. }"
    );
    drop(prepared);
    assert_eq!(f.pool.charged_for_test(), baseline);
}

#[test]
fn one_byte_short_metadata_refuses_before_rng_and_preserves_same_source() {
    let mut f = fixture(0xda02);
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let source = f.store.prepare_genesis_seed(read, budget()).unwrap();
    let original = source.source().observe_original();
    let baseline = f.pool.charged_for_test();
    let pressure = f
        .pool
        .try_reserve(1024 * 1024 * 1024 - baseline - codec::ACTIVATION_METADATA_FEE + 1)
        .unwrap();
    assert!(PreparedV2GenesisActivation::prepare(
        &f.store,
        &source.borrow_source_data(),
        &mut NeverRng
    )
    .is_err());
    original.require_same_original(source.source()).unwrap();
    source.validate_original(&f.store).unwrap();
    drop(pressure);
    assert_eq!(f.pool.charged_for_test(), baseline);
    let prepared =
        PreparedV2GenesisActivation::prepare(&f.store, &source.borrow_source_data(), &mut f.rng)
            .unwrap();
    prepared
        .candidates
        .retains_original(&f.store, &source.borrow_source_data())
        .unwrap();
}

#[test]
fn new_genuine_paid_capture_of_same_ciphertext_cannot_replace_original_activation_input() {
    let mut f = fixture(0xda03);
    let read_a = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let a = f.store.prepare_genesis_seed(read_a, budget()).unwrap();
    let prepared =
        PreparedV2GenesisActivation::prepare(&f.store, &a.borrow_source_data(), &mut f.rng)
            .unwrap();
    let read_b = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let b = f.store.prepare_genesis_seed(read_b, budget()).unwrap();
    assert_eq!(a.data().source, b.data().source);
    assert_eq!(a.data().context, b.data().context);
    assert!(prepared
        .candidates
        .retains_original(&f.store, &b.borrow_source_data())
        .is_err());
    prepared
        .candidates
        .retains_original(&f.store, &a.borrow_source_data())
        .unwrap();
}

struct CancelOnThirdSeal<'a> {
    rng: &'a mut ChaCha20Rng,
    revoker: crate::store::ChatMountRevoker,
    fills: usize,
}
impl RngCore for CancelOnThirdSeal<'_> {
    fn next_u32(&mut self) -> u32 {
        self.rng.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.rng.fill_bytes(bytes);
        self.fills += 1;
        if self.fills == 3 {
            self.revoker.revoke();
        }
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(bytes);
        Ok(())
    }
}
impl CryptoRng for CancelOnThirdSeal<'_> {}

#[test]
fn final_rng_callback_cancellation_refuses_candidate_handoff_and_refunds_only_metadata() {
    let mut f = fixture(0xda04);
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let source = f.store.prepare_genesis_seed(read, budget()).unwrap();
    let baseline = f.pool.charged_for_test();
    let mut rng = CancelOnThirdSeal {
        revoker: f.store.chat_mount_revoker(),
        rng: &mut f.rng,
        fills: 0,
    };
    assert!(
        PreparedV2GenesisActivation::prepare(&f.store, &source.borrow_source_data(), &mut rng)
            .is_err()
    );
    assert_eq!(rng.fills, 3);
    assert_eq!(f.pool.charged_for_test(), baseline);
    assert!(source.validate_original(&f.store).is_err());
}

#[test]
fn candidate_payment_survives_store_drop_but_cannot_keep_original_source_alive() {
    let mut f = fixture(0xda05);
    let read = f
        .store
        .read_saved_bootstrap_source_data(&f.group, f.signer)
        .unwrap();
    let source = f.store.prepare_genesis_seed(read, budget()).unwrap();
    let prepared =
        PreparedV2GenesisActivation::prepare(&f.store, &source.borrow_source_data(), &mut f.rng)
            .unwrap();
    let crate::store::persistence::cold_source_read::tests::Fixture { store, pool, .. } = f;
    drop(store);
    drop(source);
    assert_eq!(pool.charged_for_test(), codec::ACTIVATION_METADATA_FEE);
    assert!(prepared
        .candidates
        .original_read
        .identity
        .upgrade()
        .is_none());
    drop(prepared);
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn fixed_path_and_layout_invoice_do_not_depend_on_growth_or_untrusted_caps() {
    validate_invoice().unwrap();
    let base = PathBuf::from("a".repeat(MAX_PATH));
    for name in [SEED, PIN, HEAD, STAGE] {
        let path = child(&base, name).unwrap();
        assert!(path.capacity() <= MAX_PATH + 256);
        assert_eq!(path.file_name().unwrap(), name);
    }
    assert!(child(&PathBuf::from("a".repeat(MAX_PATH + 256)), SEED).is_err());
}
