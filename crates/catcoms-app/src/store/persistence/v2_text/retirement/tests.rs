//! Inert plan/parser/accounting DATA controls; none constructs WD/WC or deletion permission.
use super::*;

fn roots() -> Vec<TextHeadIdData> {
    (1..=4)
        .map(|n| TextHeadIdData::from_bytes([n; 32]))
        .collect()
}
fn entries() -> Vec<RetirementEntryData> {
    (1..=41)
        .map(|n| RetirementEntryData {
            kind: match n {
                1..=11 => RetirementKindData::AcceptedIndex,
                12..=31 => RetirementKindData::TokenIndex,
                _ => RetirementKindData::PublicationIndex,
            },
            id: [n; 32],
            frame_bytes: 16470,
        })
        .chain([
            RetirementEntryData {
                kind: RetirementKindData::Intent,
                id: [42; 32],
                frame_bytes: 2849,
            },
            RetirementEntryData {
                kind: RetirementKindData::Signature,
                id: [43; 32],
                frame_bytes: 405,
            },
        ])
        .collect()
}
fn plan() -> RetirementPlanData {
    RetirementPlanData::from_data(
        TextContextIdData::from_bytes([90; 32]),
        TextTransactionIdData::from_bytes([91; 32]),
        TextHeadIdData::from_bytes([92; 32]),
        RetirementCarryData::from_data(&roots(), &entries()).unwrap(),
    )
    .unwrap()
}
#[test]
fn exact_maximum_plan_and_carry_are_canonical_and_bounded() {
    let plan = plan();
    let carry = plan.carry_data().encode_data().unwrap();
    assert_eq!(carry.len(), MAX_RETIREMENT_CARRY_BYTES);
    assert!(RetirementCarryData::decode_data(&carry).unwrap() == *plan.carry_data());
    let raw = plan.encode_data().unwrap();
    assert_eq!(raw.len(), MAX_RETIREMENT_PLAN_BYTES);
    let parsed = RetirementPlanData::decode_data(&raw).unwrap();
    assert!(parsed == plan);
    assert!(parsed.id_data().unwrap() == plan.id_data().unwrap());
    assert_eq!(format!("{parsed:?}"), "RetirementPlanData { .. }");
    assert_eq!(plan.carry_data().roots_data().len(), 4);
    assert_eq!(plan.carry_data().entries_data().len(), 43);
    for n in 0..raw.len() {
        assert!(RetirementPlanData::decode_data(&raw[..n]).is_err());
    }
    let mut bad = raw.to_vec();
    bad.push(0);
    assert!(RetirementPlanData::decode_data(&bad).is_err());
    assert!(RetirementPlanData::decode_data(&vec![0; MAX_RETIREMENT_PLAN_BYTES + 1]).is_err());
    for n in 0..carry.len() {
        assert!(RetirementCarryData::decode_data(&carry[..n]).is_err());
    }
    let mut bad = carry.to_vec();
    bad.push(0);
    assert!(RetirementCarryData::decode_data(&bad).is_err());
}
#[test]
fn count_length_kind_order_duplicate_and_size_refuse_before_copy() {
    let original = plan().carry_data().encode_data().unwrap();
    let packed_start = 1 + 4 * 36 + 2 + 4;
    for (at, replacement) in [
        (0, 0),
        (0, 5),
        (1 + 4 * 36 + 1, 44),
        (packed_start, 0),
        (packed_start, 6),
    ] {
        let mut bad = original.to_vec();
        bad[at] = replacement;
        assert!(RetirementCarryData::decode_data(&bad).is_err());
    }
    let mut bad = original.to_vec();
    bad[1 + 4 * 36 + 2..packed_start].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(RetirementCarryData::decode_data(&bad).is_err());
    let mut bad = original.to_vec();
    bad[packed_start + ENTRY_BYTES..packed_start + 2 * ENTRY_BYTES]
        .copy_from_slice(&original[packed_start..packed_start + ENTRY_BYTES]);
    assert!(RetirementCarryData::decode_data(&bad).is_err());
    for bad_size in [0u32, 86, 16471, u32::MAX] {
        let mut bad = original.to_vec();
        bad[packed_start + ENTRY_BYTES - 4..packed_start + ENTRY_BYTES]
            .copy_from_slice(&bad_size.to_be_bytes());
        assert!(RetirementCarryData::decode_data(&bad).is_err());
    }
    let mut values = entries();
    values[42].id = values[0].id;
    assert!(RetirementCarryData::from_data(&roots(), &values).is_err()); // even across kinds
    let mut values = entries();
    values.swap(0, 1);
    assert!(RetirementCarryData::from_data(&roots(), &values).is_err());
    let mut values = roots();
    values[1] = values[0];
    assert!(RetirementCarryData::from_data(&values, &entries()).is_err());
    let mut values = entries();
    values[42].frame_bytes = 404;
    assert!(RetirementCarryData::from_data(&roots(), &values).is_err());
}
#[test]
fn immutable_baseline_replay_cannot_double_credit_or_cross_head() {
    let plan = plan();
    let total = plan.carry_data().totals_data().unwrap();
    assert_eq!(
        total,
        RetiredTotalsData {
            bytes: 41 * 16470 + 2849 + 405,
            objects: 43
        }
    );
    let baseline = RetiredTotalsData {
        bytes: total.bytes + 1234,
        objects: total.objects + 10,
    };
    let expected = RetiredTotalsData {
        bytes: 1234,
        objects: 10,
    };
    for _ in 0..3 {
        assert_eq!(
            total
                .effective_baseline_data(&plan, plan.selected_data(), baseline)
                .unwrap(),
            expected
        );
    }
    assert!(total
        .effective_baseline_data(&plan, TextHeadIdData::from_bytes([93; 32]), baseline)
        .is_err());
    assert!(total
        .effective_baseline_data(
            &plan,
            plan.selected_data(),
            RetiredTotalsData {
                bytes: total.bytes - 1,
                ..baseline
            }
        )
        .is_err());
    assert!(total
        .effective_baseline_data(
            &plan,
            plan.selected_data(),
            RetiredTotalsData {
                objects: total.objects - 1,
                ..baseline
            }
        )
        .is_err());
    assert!(RetiredTotalsData {
        bytes: u128::MAX,
        objects: 43
    }
    .effective_baseline_data(&plan, plan.selected_data(), baseline)
    .is_err());
    assert!(RetiredTotalsData {
        bytes: total.bytes,
        objects: 44
    }
    .effective_baseline_data(&plan, plan.selected_data(), baseline)
    .is_err());
    assert!(plan.context_data() == TextContextIdData::from_bytes([90; 32]));
    assert!(plan.transaction_data() == TextTransactionIdData::from_bytes([91; 32]));
}
