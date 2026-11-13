//! Real original MLS nodes/keys; these assert retained primitive behavior, not native authority.
use super::super::tests::{found, reserve};
use super::*;

fn counts(value: &RetainedFounderSigning) -> (usize, usize) {
    (value.profile.calls, value.header.calls)
}

#[test]
fn retained_bootstrap_each_postraw_refusal_reuses_both_exact_original_signatures() {
    for kind in [0, 1] {
        let node = found(0x3b01 + u64::from(kind));
        let mut signing = node.begin_owner_v2_founder_signing().unwrap();
        FAULT.with(|fault| fault.set(Some((kind, Fault::RefuseAfterCache))));
        assert!(node
            .prepare_owner_v2_founder_retained(&mut signing)
            .is_err());
        let profile = signing.profile.raw.unwrap();
        assert_eq!(counts(&signing), if kind == 0 { (1, 0) } else { (1, 1) });
        let prepared = node
            .prepare_owner_v2_founder_retained(&mut signing)
            .unwrap();
        assert_eq!(prepared.profile.signature(), &profile);
        assert_eq!(counts(&signing), (1, 1));
        let frozen = prepared.freeze(&node).unwrap();
        let expected = frozen
            .borrow_for_covering_save(&node)
            .unwrap()
            .header
            .to_vec();
        for _ in 0..3 {
            let again = node
                .prepare_owner_v2_founder_retained(&mut signing)
                .unwrap()
                .freeze(&node)
                .unwrap();
            assert_eq!(
                again.borrow_for_covering_save(&node).unwrap().header,
                expected
            );
        }
        assert_eq!(counts(&signing), (1, 1));
    }
}

#[test]
fn retained_bootstrap_entered_primitive_unwind_never_reenters_that_slot() {
    for kind in [0, 1] {
        let node = found(0x3b10 + u64::from(kind));
        let mut signing = node.begin_owner_v2_founder_signing().unwrap();
        FAULT.with(|fault| fault.set(Some((kind, Fault::UnwindBeforeReturn))));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = node.prepare_owner_v2_founder_retained(&mut signing);
        }))
        .is_err());
        let before = counts(&signing);
        assert_eq!(before, if kind == 0 { (1, 0) } else { (1, 1) });
        for _ in 0..3 {
            assert!(node
                .prepare_owner_v2_founder_retained(&mut signing)
                .is_err());
        }
        assert_eq!(counts(&signing), before);
        assert!(if kind == 0 {
            signing.profile.raw.is_none()
        } else {
            signing.header.raw.is_none()
        });
    }
}

#[test]
fn retained_bootstrap_foreign_node_refuses_after_profile_without_touching_either_key() {
    let node = found(0x3b20);
    let foreign = found(0x3b21);
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    FAULT.with(|fault| fault.set(Some((0, Fault::RefuseAfterCache))));
    assert!(node
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    assert!(foreign
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    assert_eq!(counts(&signing), (1, 0));
    node.prepare_owner_v2_founder_retained(&mut signing)
        .unwrap();
    assert_eq!(counts(&signing), (1, 1));
}

#[test]
fn retained_bootstrap_equal_scope_labels_do_not_adopt_a_replacement_reservation() {
    let mut node = found(0x3b30);
    let _original = reserve(&mut node);
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    FAULT.with(|fault| fault.set(Some((0, Fault::RefuseAfterCache))));
    assert!(node
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    node.local_chat_scope = Default::default();
    let _replacement = reserve(&mut node);
    assert!(node
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    assert_eq!(counts(&signing), (1, 0));
}

#[test]
fn retained_bootstrap_invalid_cached_signature_refuses_without_resigning() {
    let node = found(0x3b40);
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    FAULT.with(|fault| fault.set(Some((0, Fault::RefuseAfterCache))));
    assert!(node
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    signing.profile.raw.as_mut().unwrap()[0] ^= 1;
    for _ in 0..3 {
        assert!(node
            .prepare_owner_v2_founder_retained(&mut signing)
            .is_err());
    }
    assert_eq!(counts(&signing), (1, 0));
    assert!(signing.profile.raw.is_some());
}

#[test]
fn retained_bootstrap_does_not_change_the_stateless_library_contract() {
    let node = found(0x3b50);
    let first = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    let second = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    let retained = node
        .prepare_owner_v2_founder_retained(&mut signing)
        .unwrap()
        .freeze(&node)
        .unwrap();
    let one = first.borrow_for_covering_save(&node).unwrap();
    for other in [&second, &retained] {
        let two = other.borrow_for_covering_save(&node).unwrap();
        assert_eq!(one.profile, two.profile);
        assert_eq!(one.header, two.header);
        assert_eq!(one.state, two.state);
    }
    assert_eq!(counts(&signing), (1, 1));
}
