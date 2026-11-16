use super::*;
fn account() -> Account {
    Account::new(OwnerCheckpointRestoreLimits {
        max_typed_bytes: 1024 * 1024,
        max_work: 1024 * 1024,
    })
    .unwrap()
}
#[derive(serde::Serialize)]
struct Pair {
    public_key: KeyData,
    private_key: KeyData,
}
#[derive(serde::Serialize)]
struct KeyData {
    key: Bytes,
}
#[derive(serde::Serialize)]
struct Bytes {
    vec: Vec<u8>,
}
fn pair_data(byte: u8) -> Pair {
    Pair {
        public_key: KeyData {
            key: Bytes {
                vec: vec![byte; 32],
            },
        },
        private_key: KeyData {
            key: Bytes { vec: vec![23; 32] },
        },
    }
}
#[test]
fn epoch_key_extra_input_is_exact_nonempty_bounded_and_unique_before_decode() {
    for count in 1..=MAX_KEYS {
        let values = (0..count).map(|i| pair_data(i as u8)).collect::<Vec<_>>();
        let bytes = serde_json::to_vec(&values).unwrap();
        assert_eq!(check(&bytes, &mut account()).unwrap(), count);
    }
    for value in [
        vec![],
        (0..MAX_KEYS + 1).map(|i| pair_data(i as u8)).collect(),
        vec![pair_data(1), pair_data(1)],
    ] {
        assert!(check(&serde_json::to_vec(&value).unwrap(), &mut account()).is_err());
    }
    let mut value = pair_data(1);
    value.private_key.key.vec = vec![1];
    assert!(check(&serde_json::to_vec(&vec![value]).unwrap(), &mut account()).is_err());
    let mut bytes = serde_json::to_vec(&vec![pair_data(1)]).unwrap();
    let end = bytes.len() - 2;
    bytes.splice(end..end, b",\"unknown\":0".iter().copied());
    assert!(check(&bytes, &mut account()).is_err());
}
