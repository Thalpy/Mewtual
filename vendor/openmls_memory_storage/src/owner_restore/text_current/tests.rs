//! Pure bounded syntax/geometry tests. Synthetic rows here grant no SDK/source
//! authority; genuine actual20/50 producers are tested by the MLS wrapper family.
use super::*;
fn facts(epoch: u64) -> shape::Facts {
    let mut facts = shape::Facts {
        leaves: 64,
        parents: 63,
        secret_leaves: 64,
        secret_parents: 64,
        handshake: 64,
        application: 64,
        own_index: 63,
        secret_index: 63,
        secret_size: 127,
        epoch,
        resumption_max: 32,
        ..Default::default()
    };
    let mut slots = Vec::new();
    let mut cursor = 0usize;
    // Independent exact native add simulation pins the first-wrap special case.
    if epoch < 1000 {
        for e in 0..=epoch {
            if slots.len() < 32 {
                slots.push(e);
                cursor += 1;
            } else {
                cursor = (cursor + 1) % slots.len();
                slots[cursor] = e;
            }
        }
    } else {
        cursor = ((epoch - 31) % 32) as usize;
        slots = (0..32)
            .map(|i| epoch - ((cursor + 32 - i) % 32) as u64)
            .collect();
    }
    facts.resumption_cursor = cursor as u64;
    facts.resumption_count = slots.len();
    for (i, e) in slots.into_iter().enumerate() {
        facts.resumption_epochs[i] = Some(e);
    }
    facts
}
#[test]
fn text_ring_exact_first_wrap_and_arbitrary_u64_epoch_have_no_overflow_or_sort_assumption() {
    for e in [0, 1, 30, 31, 32, 62, 63, 64, 1000, u64::MAX - 1, u64::MAX] {
        let f = facts(e);
        f.finish().unwrap();
        let mut wrong = f;
        wrong.resumption_cursor += 1;
        assert!(wrong.finish().is_err());
        let mut wrong = f;
        wrong.resumption_epochs[0] = Some(e.wrapping_add(17));
        assert!(wrong.finish().is_err());
        let mut welcome = f;
        welcome.resumption_max = 0;
        welcome.resumption_count = 0;
        welcome.resumption_cursor = 0;
        welcome.resumption_epochs = [None; 32];
        welcome.finish().unwrap();
    }
    let mut invalid = facts(63);
    invalid.leaves = 65;
    assert!(invalid.finish().is_err());
}
#[test]
fn text_resumption_repeated_scan_clears_prior_slots_and_charges_the_same_account() {
    let mut f = facts(1);
    let mut a = Account::new(OwnerCheckpointRestoreLimits {
        max_typed_bytes: 1024 * 1024,
        max_work: usize::MAX,
    })
    .unwrap();
    for e in [64, 0] {
        let expected = facts(e);
        f.epoch = e;
        let entries: Vec<_> = expected
            .resumption_epochs
            .iter()
            .flatten()
            .map(|v| serde_json::json!([v,{"secret":{"value":{"vec":vec![0;32]}}}]))
            .collect();
        let raw=serde_json::to_vec(&serde_json::json!({"max_number_of_secrets":32,"cursor":expected.resumption_cursor,"resumption_psk":entries})).unwrap();
        shape::check(Row::Resumption, &raw, &[9; 16], &[10; 32], &mut f, &mut a).unwrap();
        f.finish().unwrap();
        assert_eq!(f.resumption_epochs, expected.resumption_epochs);
    }
    let before = a.work;
    assert!(shape::check(
        Row::State,
        b"\"FutureOperational\"",
        &[9; 16],
        &[10; 32],
        &mut f,
        &mut a
    )
    .is_err());
    assert!(a.work < before);
}

#[test]
fn sixtyfour_null_native_slots_charge_actual_option_element_growth_before_decode() {
    #[derive(serde::Deserialize)]
    struct Wide {
        #[serde(rename = "_secret")]
        _secret: [u8; 32],
    }
    let slot = std::mem::size_of::<Option<Wide>>();
    let requested = (4 + 8 + 16 + 32 + 64) * slot;
    let raw = format!(
        "[{}]",
        std::iter::repeat_n("null", 64)
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut exact = Account::new(OwnerCheckpointRestoreLimits {
        max_typed_bytes: requested,
        max_work: usize::MAX,
    })
    .unwrap();
    let decoded: Vec<Option<Wide>> =
        super::super::decode::from_slice(raw.as_bytes(), &mut exact).unwrap();
    assert_eq!(decoded.len(), 64);
    assert_eq!(exact.bytes, 0);
    let mut short = Account::new(OwnerCheckpointRestoreLimits {
        max_typed_bytes: requested - 1,
        max_work: usize::MAX,
    })
    .unwrap();
    assert!(
        super::super::decode::from_slice::<Vec<Option<Wide>>>(raw.as_bytes(), &mut short).is_err()
    );
}
