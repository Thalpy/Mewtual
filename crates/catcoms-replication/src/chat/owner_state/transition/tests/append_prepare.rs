//! Independent existing received-state fixtures pin pre-sign/post-sign math equivalence.
//! DATA only: real signatures do not select a governing prefix or certify accepted absence.
//! Declared beside transition tests; actual Root storage selection remains a separate gate.
use super::*;

#[test]
fn unsigned_append_matches_existing_complete_image_and_uses_new_actual_header_tip() {
    let (key, base) = genesis();
    let admission = base.slots().next().unwrap().image.admission;
    let (body, expected, image) = append(&base, &key, admission, 90);
    let original_image = base.image().to_vec();
    let original_header = base.header().clone();
    let plan = base.prepare_append_data(&body).unwrap();
    assert!(plan.image_data() == image.as_slice());
    assert!(plan.header_spec_data() == expected.spec());
    let signed =
        SignedOwnerHeader::sign_using(plan.header_spec_data().clone(), |bytes| Ok(key.sign(bytes)))
            .unwrap();
    let step = base
        .derive_successor(&signed, ReplayBody::Append(&body), plan.image_data())
        .unwrap();
    assert!(plan.signing_bytes_data() == signed.semantic_bytes());
    assert_eq!(step.next.slot(admission).unwrap().tip_header, signed.id());
    assert!(base.image() == original_image.as_slice() && base.header() == &original_header);
    assert_eq!(base.slot(admission).unwrap().image.accepted_count, 0);
    assert_eq!(step.next.slot(admission).unwrap().image.accepted_count, 1);
}

#[test]
fn interleaved_other_admission_keeps_its_own_tip_and_wrong_global_tip_refuses_presign() {
    let (key, genesis) = genesis();
    let a = genesis.slots().next().unwrap().image.admission;
    let (b_body, b_add, b_image) = add(&genesis, &key, [91; 32], 92);
    let both = genesis
        .derive_successor(&b_add, ReplayBody::Admission(&b_body), &b_image)
        .unwrap()
        .next;
    let (a_body, a_header, a_image) = append(&both, &key, a, 93);
    let after_a = both
        .derive_successor(&a_header, ReplayBody::Append(&a_body), &a_image)
        .unwrap()
        .next;
    let (b_append, b_header, b_image) = append(&after_a, &key, b_body.id(), 94);
    let after_b = after_a
        .derive_successor(&b_header, ReplayBody::Append(&b_append), &b_image)
        .unwrap()
        .next;
    let before = after_b.image().to_vec();
    let (a_next, expected, image) = append(&after_b, &key, a, 95);
    assert_eq!(a_next.spec().previous_tip, a_header.id());
    assert_ne!(a_next.spec().previous_tip, after_b.header().id());
    let mut wrong = a_next.spec().clone();
    wrong.previous_tip = after_b.header().id();
    assert!(after_b
        .prepare_append_data(&AppendBody::new(wrong).unwrap())
        .is_err());
    let mut foreign = a_next.spec().clone();
    foreign.group = b"another-group".to_vec();
    assert!(after_b
        .prepare_append_data(&AppendBody::new(foreign).unwrap())
        .is_err());
    let plan = after_b.prepare_append_data(&a_next).unwrap();
    assert!(plan.image_data() == image.as_slice());
    assert!(plan.header_spec_data() == expected.spec());
    let signed =
        SignedOwnerHeader::sign_using(plan.header_spec_data().clone(), |bytes| Ok(key.sign(bytes)))
            .unwrap();
    let next = after_b
        .derive_successor(&signed, ReplayBody::Append(&a_next), plan.image_data())
        .unwrap()
        .next;
    assert_eq!(next.slot(b_body.id()).unwrap().tip_header, b_header.id());
    assert!(next.slot(b_body.id()).unwrap().image == after_b.slot(b_body.id()).unwrap().image);
    assert_eq!(next.slot(a).unwrap().tip_header, signed.id());
    assert!(after_b.image() == before.as_slice());
}
