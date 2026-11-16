use super::*;
fn limits() -> TextAddOperationLimits {
    TextAddOperationLimits {
        max_typed_bytes: 1024 * 1024,
        max_serialized_bytes: 64 * 1024,
        max_provider_bytes: 1024 * 1024,
        max_work: 128 * 1024 * 1024,
    }
}
#[test]
fn exact_serializer_and_key_charge_before_owned_allocation() {
    let mut a = OperationAccount::new(limits()).unwrap();
    a.write_entry(512).unwrap();
    let expected = serde_json::to_vec(&[1u8, 2, 255]).unwrap();
    let bytes = a.encode(&[1u8, 2, 255], 512).unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(bytes.capacity(), bytes.len());
    a.serialized = bytes.len() - 1;
    assert!(a.encode(&[1u8, 2, 255], 512).is_err());
    let mut key = StackKey::new();
    key.push(b"EpochKeyPairs").unwrap();
    key.json(&[7u8; 16]).unwrap();
    key.json(&u64::MAX).unwrap();
    key.json(&63u32).unwrap();
    key.push(&[0, 1]).unwrap();
    let mut expected = b"EpochKeyPairs".to_vec();
    serde_json::to_writer(&mut expected, &[7u8; 16]).unwrap();
    serde_json::to_writer(&mut expected, &u64::MAX).unwrap();
    serde_json::to_writer(&mut expected, &63u32).unwrap();
    expected.extend_from_slice(&[0, 1]);
    assert_eq!(key.bytes(), expected);
    assert!(key.push(&[0; 512]).is_err());
}
#[test]
fn backing_upper_bound_covers_actual_empty_and_preallocated_candidate_tables() {
    for rows in [0usize, 1, 13, 20, 64, 128, 4096] {
        let mut map: StorageMap = StorageMap::with_capacity_and_hasher(rows, Default::default());
        assert!(map.allocation_size() <= backing(rows).unwrap());
        for i in 0..rows {
            map.insert(i.to_be_bytes().to_vec(), vec![]);
        }
        let bound = backing(rows + MAX_PROVIDER_WRITES).unwrap();
        map.try_reserve(MAX_PROVIDER_WRITES).unwrap();
        assert!(map.allocation_size() <= bound);
        let allocated = map.allocation_size();
        // Deletes/tombstones do not invalidate the separately reserved allowance
        // for EVERY remaining potential insertion in this one-use operation.
        for i in 0..rows {
            map.remove(i.to_be_bytes().as_slice());
        }
        for i in rows..rows + MAX_PROVIDER_WRITES {
            map.insert(i.to_be_bytes().to_vec(), vec![]);
            assert_eq!(map.allocation_size(), allocated);
        }
    }
    assert!(backing(usize::MAX).is_none());
}
#[test]
fn exhausted_write_and_work_accounts_never_reopen() {
    let mut a = OperationAccount::new(limits()).unwrap();
    for _ in 0..MAX_PROVIDER_WRITES {
        a.write_entry(0).unwrap();
    }
    assert!(a.write_entry(0).is_err());
    assert!(a.write_entry(0).is_err());
    let mut low = limits();
    low.max_work = 1;
    assert!(OperationAccount::new(low).unwrap().key_entry().is_err());
}

#[test]
fn serializer_temporary_debt_is_reserved_before_any_serialize_call() {
    use std::cell::Cell;
    struct Observe<'a>(&'a Cell<usize>);
    impl serde::Serialize for Observe<'_> {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            self.0.set(self.0.get() + 1);
            s.serialize_u8(7)
        }
    }
    let calls = Cell::new(0);
    let fixed = OperationAccount::fixed_layout_bytes().unwrap();
    let mut short = limits();
    short.max_typed_bytes = fixed + SERIALIZE_TEMPORARY_REQUESTS - 1;
    let mut a = OperationAccount::new(short).unwrap();
    a.write_entry(32).unwrap();
    assert!(a.encode(&Observe(&calls), 32).is_err());
    assert_eq!(
        calls.get(),
        0,
        "even first serializer/converter must not run"
    );
    let mut exact = limits();
    exact.max_typed_bytes = fixed + SERIALIZE_TEMPORARY_REQUESTS;
    let mut a = OperationAccount::new(exact).unwrap();
    a.write_entry(32).unwrap();
    assert_eq!(a.encode(&Observe(&calls), 32).unwrap(), b"7");
    assert_eq!(calls.get(), 2);
    assert_eq!(a.typed.bytes, 0);
}
