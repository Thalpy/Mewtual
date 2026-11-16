//! Deterministic local parser/invoice regressions. Actual SDK+App consumers have their own
//! genuine-source tests; these tests do not manufacture membership/current/payment authority.
use super::*;
use std::cell::Cell;
use std::collections::VecDeque;

fn account(bytes: usize) -> Account {
    Account::new(OwnerCheckpointRestoreLimits {
        max_typed_bytes: bytes,
        max_work: usize::MAX,
    })
    .unwrap()
}
#[test]
fn exact_checked_rawvec_growth_sum_includes_spare_capacity_and_reallocation_overlap() {
    for (count, element, fee) in [
        (0, 128, 0),
        (1, 1, 8),
        (9, 1, 24),
        (32, 1, 56),
        (1, 128, 512),
        (5, 128, 1536),
        (3, 1025, 7175),
    ] {
        let mut exact = account(fee + 1);
        exact.sequence(count, element).unwrap();
        assert_eq!(exact.bytes, 1);
        if fee != 0 {
            assert!(account(fee - 1).sequence(count, element).is_err());
        }
    }
    assert!(account(isize::MAX as usize)
        .sequence(usize::MAX, usize::MAX)
        .is_err());
    let mut zst = account(1);
    zst.sequence(100, 0).unwrap();
    assert_eq!(zst.bytes, 1);
    assert_eq!(zst.work, usize::MAX - 100);
}
#[test]
fn actual_counted_vec_and_vecdeque_debit_before_first_container_growth() {
    let mut exact = account(56);
    let bytes =
        b"[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32]";
    let values: Vec<u8> = decode::from_slice(bytes, &mut exact).unwrap();
    assert_eq!(values.len(), 32);
    assert_eq!(exact.bytes, 0);
    assert!(decode::from_slice::<Vec<u8>>(bytes, &mut account(55)).is_err());
    let fee = 12 * std::mem::size_of::<u64>(); // request4 then8, not merely logical length5.
    let mut exact = account(fee);
    let queue: VecDeque<u64> = decode::from_slice(b"[1,2,3,4,5]", &mut exact).unwrap();
    assert_eq!(queue.len(), 5);
    assert_eq!(exact.bytes, 0);
    assert!(decode::from_slice::<VecDeque<u64>>(b"[1,2,3,4,5]", &mut account(fee - 1)).is_err());
}
#[test]
fn nullable_private_layout_surrogates_pay_full_inline_slots_even_when_all_null() {
    #[derive(serde::Deserialize)]
    struct Wide {
        #[serde(rename = "_data")]
        _data: [u8; 32],
    }
    let slot = std::mem::size_of::<Option<Wide>>();
    assert!(slot > std::mem::size_of::<usize>());
    let fee = 4 * slot; // serde Vec starts at minimum4 for this actual element layout.
    let mut exact = account(fee);
    let values: Vec<Option<Wide>> = decode::from_slice(b"[null,null]", &mut exact).unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(exact.bytes, 0);
    assert!(
        decode::from_slice::<Vec<Option<Wide>>>(b"[null,null]", &mut account(fee - 1)).is_err()
    );
}
#[test]
fn fixed_inline_nonce_array_is_not_charged_as_a_vector_and_empty_lists_are_free() {
    let mut paid = account(1);
    let nonce: [u8; 12] = decode::from_slice(b"[0,0,0,0,0,0,0,0,0,0,0,0]", &mut paid).unwrap();
    assert_eq!(nonce, [0; 12]);
    assert_eq!(paid.bytes, 1);
    let empty: Vec<Vec<u8>> = decode::from_slice(b"[]", &mut paid).unwrap();
    assert!(empty.is_empty());
    assert_eq!(paid.bytes, 1);
}
std::thread_local! { static UNIT_VISITS: Cell<usize> = const { Cell::new(0) }; }
struct Unit;
impl<'de> serde::Deserialize<'de> for Unit {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        UNIT_VISITS.with(|visits| visits.set(visits.get() + 1));
        Ok(Unit)
    }
}
#[test]
fn nonempty_zst_or_in_place_seed_cannot_hide_an_existing_large_value() {
    UNIT_VISITS.with(|visits| visits.set(0));
    assert!(decode::from_slice::<Vec<Unit>>(b"[null]", &mut account(100)).is_err());
    assert_eq!(
        UNIT_VISITS.with(Cell::get),
        0,
        "refuse BEFORE invoking an uncharged child seed"
    );
}
#[test]
fn strict_shape_duplicate_escape_unknown_count_and_wrong_geometry_refuse() {
    let group = [7; 16];
    let signer = [8; 32];
    for bytes in [
        b"\"PendingCommit\"".as_slice(),
        b"\"Operati\\u006fnal\"",
        b"[\"Operational\"]",
        b"\"Operational\" true",
    ] {
        assert!(shape::check(
            Row::State,
            bytes,
            &group,
            &signer,
            &mut shape::Facts::default(),
            &mut account(100_000)
        )
        .is_err());
    }
    for bytes in [
        b"{\"max_number_of_secrets\":0,\"resumption_psk\":[],\"cursor\":0,\"cursor\":0}".as_slice(),
        b"{\"max_number_of_secrets\":0,\"resumption_psk\":[],\"cursor\":0,\"unknown\":0}",
        b"{\"max_number_of_secrets\":0,\"resumption_psk\":[null],\"cursor\":0}",
    ] {
        assert!(shape::check(
            Row::Resumption,
            bytes,
            &group,
            &signer,
            &mut shape::Facts::default(),
            &mut account(100_000)
        )
        .is_err());
    }
    let mut facts = shape::Facts {
        leaves: 2,
        parents: 1,
        secret_leaves: 2,
        secret_parents: 2,
        handshake: 2,
        application: 2,
        own_index: 1,
        secret_index: 1,
        secret_size: 3,
        epoch: 1,
        unmerged_max: None,
        ..shape::Facts::default()
    };
    facts.finish().unwrap();
    facts.secret_parents = 1;
    assert!(
        facts.finish().is_err(),
        "SDK SecretTree does NOT share public N-1 parent geometry"
    );
}
#[test]
fn failed_preflight_installs_cancelled_not_legacy_and_work_refuses_before_lookup() {
    let storage = MemoryStorage::default();
    assert!(storage.legacy_read_diagnostics().unwrap());
    assert!(storage
        .begin_owner_checkpoint_restore(
            &[7; 16],
            &[8; 32],
            OwnerCheckpointRestoreLimits {
                max_typed_bytes: 100_000,
                max_work: 10_000_000
            }
        )
        .is_err());
    assert!(storage.require_owner_read_label(b"GroupState").is_err());
    assert!(storage.require_unrestricted().is_err());
    assert!(
        !storage.legacy_read_diagnostics().unwrap(),
        "even cancelled policy never enters legacy backtrace/hex/logger branch"
    );
    assert_eq!(
        storage
            .owner_restore
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .phase
            .load(Ordering::Acquire),
        CANCELLED
    );
    let mut budget = account(1);
    budget.work = 512;
    assert!(budget.lookup(1, 1).is_err()); // Needs one full512-byte comparison+probe AND hash.
}

std::thread_local! { static VARIANT_SEEDS: Cell<usize> = const { Cell::new(0) }; }
struct Name(String);
struct NameSeed;
impl<'de> serde::de::DeserializeSeed<'de> for NameSeed {
    type Value = String;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<String, D::Error> {
        VARIANT_SEEDS.with(|n| n.set(n.get() + 1));
        serde::Deserialize::deserialize(decoder)
    }
}
impl<'de> serde::Deserialize<'de> for Name {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct Enum;
        impl<'de> serde::de::Visitor<'de> for Enum {
            type Value = Name;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("Basic")
            }
            fn visit_enum<A: serde::de::EnumAccess<'de>>(self, data: A) -> Result<Name, A::Error> {
                use serde::de::VariantAccess as _;
                let (name, variant) = data.variant_seed(NameSeed)?;
                variant.unit_variant()?;
                Ok(Name(name))
            }
        }
        decoder.deserialize_enum("Name", &["Basic"], Enum)
    }
}
#[test]
fn real_variant_seed_copy_is_debited_before_visitor_even_with_borrowed_identifiers() {
    VARIANT_SEEDS.with(|n| n.set(0));
    assert!(decode::from_slice::<Name>(br#""Basic""#, &mut account(4)).is_err());
    assert_eq!(VARIANT_SEEDS.with(Cell::get), 0);
    let name: Name = decode::from_slice(br#""Basic""#, &mut account(5)).unwrap();
    assert_eq!(name.0, "Basic");
    assert_eq!(VARIANT_SEEDS.with(Cell::get), 1);
}

#[test]
fn a_legacy_writer_waiting_for_values_cannot_cross_new_policy_installation() {
    use std::sync::mpsc;
    use std::time::Duration;
    struct Release(Option<mpsc::Sender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }
    let storage = Arc::new(MemoryStorage::default());
    let (reached_tx, reached_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = storage.clone();
    let join = std::thread::spawn(move || {
        BEFORE_WRITER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                reached_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            }))
        });
        let result = worker.write::<1>(b"unrelated", b"row", b"true".to_vec());
        done_tx.send(result).unwrap();
    });
    let release = Release(Some(release_tx));
    reached_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(storage
        .begin_owner_checkpoint_restore(
            &[7; 16],
            &[8; 32],
            OwnerCheckpointRestoreLimits {
                max_typed_bytes: 100_000,
                max_work: 10_000_000
            }
        )
        .is_err());
    drop(release);
    assert!(done_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .is_err());
    join.join().unwrap();
    assert!(
        storage.values.read().unwrap().is_empty(),
        "no legacy write overtakes installation"
    );
}

#[path = "resumption_tests.rs"]
mod resumption_tests;
