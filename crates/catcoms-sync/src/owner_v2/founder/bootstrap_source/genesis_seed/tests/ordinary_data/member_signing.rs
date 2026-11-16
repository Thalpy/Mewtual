//! The founder's actual closed genesis node acting as an ORDINARY member, against the real
//! production general verifier and plan issuer. HOST trust for that verifier is the explicit
//! test selection in `governance_fixture`; the App host derives it from the original pin.
//! These pin that no founder Header/Append signature or Create producer stage is used.
use super::super::super::ordinary_member::after_member_sign_for_test;
use super::*;
use crate::owner_v2::member::governance_fixture::{create_draft, governance};
use crate::owner_v2::member::key_calls_for_test;
use crate::owner_v2::{V2MemberDraftInputs, V2MemberSignRefusal};

#[test]
fn founder_node_signs_successive_ordinary_creates_without_header_append_or_create_lane() {
    let (mut node, _reservation, _opened, _count) = prepared_node(0xd100);
    let before = snapshot(&node);
    let d = node.data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    let signer = node.ordinary_member_signer_data().unwrap();
    assert_eq!(signer, node.data().profile.spec().founder);
    assert_eq!(signer, node.data().admission.spec().subject);
    let genesis_header = g.verifier.replay_data().header().id();
    append::reset_for_test();
    create::reset_for_test();
    let calls = key_calls_for_test();
    let mut signed = Vec::new();
    for (index, text) in ["first ordinary send", "second ordinary send"]
        .into_iter()
        .enumerate()
    {
        let mut slot = V2MemberDraftInputs::new();
        let drawn = node.draw_ordinary_draft_inputs(&mut slot).unwrap();
        let draft = create_draft(&g, &g.founder, signer, drawn, text);
        let mut plan = g
            .verifier
            .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
            .unwrap();
        let original = plan.observe_original();
        let guard = plan.admit_signing(&draft, &signer).unwrap();
        drop(node.sign_ordinary_admitted(&slot, guard).unwrap());
        let action = plan.complete_cached_raw64_data(&original, &draft).unwrap();
        assert!(action.verify());
        assert_eq!(action.signer(), &signer);
        assert_eq!(key_calls_for_test(), calls + index + 1);
        signed.push(action);
    }
    assert!(signed[0].id() != signed[1].id());
    // Successive sends needed no governing Header acceptance: the founder Append signer,
    // the founder Create producer and the selected governing Header are all untouched.
    assert_eq!(append::signs_for_test(), 0);
    assert_eq!(create::entries_for_test(), [0; 5]);
    assert!(g.verifier.replay_data().header().id() == genesis_header);
    assert!(snapshot(&node).as_slice() == before.as_slice());
}

#[test]
fn founder_cancellation_after_the_cached_raw_keeps_the_original_and_refuses_new_signing() {
    let (mut node, reservation, _opened, _count) = prepared_node(0xd110);
    let d = node.data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    let signer = node.ordinary_member_signer_data().unwrap();
    let mut slot = V2MemberDraftInputs::new();
    let drawn = node.draw_ordinary_draft_inputs(&mut slot).unwrap();
    let draft = create_draft(&g, &g.founder, signer, drawn, "cancelled after raw");
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
        .unwrap();
    let original = plan.observe_original();
    let owner = reservation.owner.clone();
    // The real scope reservation is revoked AFTER the raw is cached, inside the seam.
    after_member_sign_for_test(Box::new(move || {
        owner.send_replace(true);
    }));
    let calls = key_calls_for_test();
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    let guard = node.sign_ordinary_admitted(&slot, guard).unwrap();
    assert!(guard.cached_raw64_data().is_some());
    drop(guard);
    assert_eq!(key_calls_for_test(), calls + 1);
    // Every later positive seam refuses on the revoked source, and nothing is drawn...
    assert_eq!(
        node.ordinary_member_signer_data(),
        Err(V2MemberSignRefusal::Source)
    );
    let mut later = V2MemberDraftInputs::new();
    assert_eq!(
        node.draw_ordinary_draft_inputs(&mut later),
        Err(V2MemberSignRefusal::Source)
    );
    assert!(later.drawn_data().is_none());
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    assert_eq!(
        node.sign_ordinary_admitted(&slot, guard).unwrap_err(),
        V2MemberSignRefusal::Source
    );
    assert_eq!(key_calls_for_test(), calls + 1);
    // ...while the exact original raw still completes as historical DATA.
    let action = plan.complete_cached_raw64_data(&original, &draft).unwrap();
    assert!(action.verify());
    assert!(action.id() == draft.action_id_data());
}

#[test]
fn founder_draw_is_once_only_through_the_retained_runtime_seams() {
    let (mut node, _reservation, _opened, _count) = prepared_node(0xd120);
    // Fork the node's actual retained RNG so the exact consumed stream can be checked.
    let mut fork = node._parts.body._rng.clone();
    let mut slot = V2MemberDraftInputs::new();
    let first = node.draw_ordinary_draft_inputs(&mut slot).unwrap();
    let mut nonce = [0; 16];
    fork.fill_bytes(&mut nonce);
    assert_eq!(first.1, nonce);
    assert_eq!(node.draw_ordinary_draft_inputs(&mut slot).unwrap(), first);
    let mut expected = [0; 16];
    fork.fill_bytes(&mut expected);
    let mut actual = [0; 16];
    node._parts.body._rng.fill_bytes(&mut actual);
    assert_eq!(actual, expected);
}

#[test]
fn another_founder_node_cannot_answer_for_this_founders_request() {
    let (mut node, _reservation, _opened, _count) = prepared_node(0xd130);
    let (mut other, _other_reservation, _other_opened, _other_count) = prepared_node(0xd131);
    let d = node.data();
    let g = governance(d.profile, d.admission, d.header, d.state);
    let signer = node.ordinary_member_signer_data().unwrap();
    assert_ne!(signer, other.ordinary_member_signer_data().unwrap());
    let mut slot = V2MemberDraftInputs::new();
    let drawn = node.draw_ordinary_draft_inputs(&mut slot).unwrap();
    // The slot is bound to this seed: another node cannot redraw into it.
    assert_eq!(
        other.draw_ordinary_draft_inputs(&mut slot),
        Err(V2MemberSignRefusal::Source)
    );
    let draft = create_draft(&g, &g.founder, signer, drawn, "only this founder");
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
        .unwrap();
    let original = plan.observe_original();
    let calls = key_calls_for_test();
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    assert_eq!(
        other.sign_ordinary_admitted(&slot, guard).unwrap_err(),
        V2MemberSignRefusal::Source
    );
    assert_eq!(key_calls_for_test(), calls);
    // The refused attempt did not consume the one key entry.
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    drop(node.sign_ordinary_admitted(&slot, guard).unwrap());
    assert!(plan
        .complete_cached_raw64_data(&original, &draft)
        .unwrap()
        .verify());
}
