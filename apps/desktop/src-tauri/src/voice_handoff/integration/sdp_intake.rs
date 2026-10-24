//! Real original PC/Ready/Phase capacity, with independently authenticated loopback bytes.
//! This exercises intake only: unsupported service dispatch stays closed and no Action is
//! issued. Fixture Vecs are not claims about production browser custody or audible RTC.
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use catcoms_sync::voice_v2::resources::{ResourceKind, GROUP_BYTES};
use catcoms_sync::voice_v2::VoiceError;
use service::tests::{authenticated_header, signed_frame};

pub(super) fn exercise(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    pc_frame: &[u8],
) {
    let phase = parent.sdp_test_phase().unwrap();
    let clock = crate::voice_runtime(state).clock();
    let resources = route.selected().resources();
    let group = route.selected().binding().scope.group;
    let before = route.session().fixture_counts();
    let mut plain = pc_frame[..193].to_vec();
    plain[0] = 8;
    plain.extend_from_slice(&parent.pc_test_proof());
    plain.extend_from_slice(&8192u16.to_be_bytes());
    plain.resize(227 + 8192, 42);
    let mut header = (before.1 + 1, 15, 2, parent.id().logical());
    let wire = signed_frame(public, header, &plain);
    let mut wrong_parent = header;
    wrong_parent.3[15] ^= 1;
    let wrong_wire = signed_frame(public, wrong_parent, &plain);
    let wrong = authenticated_header(route, &wrong_wire);
    let charged = resources.counts();
    assert!(matches!(
        state.voice_handoff.sdp_begin_intake(
            route.selected(),
            route.session().identity(),
            &wrong,
            clock.clone()
        ),
        Err(VoiceError::Unauthorized)
    ));
    assert_eq!(resources.counts(), charged);
    drop(wrong);
    // An actual independently authenticated foreign Session must refuse BEFORE this
    // original index/capacity is visited, even for equal group/slot/public logical labels.
    let foreign = channel::tests::FixtureChannel::new(resources.clone(), group);
    let foreign_wire = foreign.wire(
        parent.id().logical(),
        1,
        channel::Operation::SdpSealBegin,
        channel::BodyKind::Sdp,
        &plain[..230],
    );
    let foreign_header = foreign.authenticate(&foreign_wire).unwrap();
    let charged = resources.counts();
    assert!(matches!(
        state.voice_handoff.sdp_begin_intake(
            route.selected(),
            route.session().identity(),
            &foreign_header,
            clock.clone()
        ),
        Err(VoiceError::Unauthorized)
    ));
    assert_eq!(resources.counts(), charged);
    drop(foreign_header);
    drop(foreign);

    // The permit belongs to this exact physical allocation, not merely the same signed
    // header bytes. It keeps the original position busy if its header owner disappears.
    let auth = authenticated_header(route, &wire);
    let permit = state
        .voice_handoff
        .sdp_begin_intake(
            route.selected(),
            route.session().identity(),
            &auth,
            clock.clone(),
        )
        .unwrap();
    drop(auth);
    assert_eq!(route.session().fixture_counts(), (true, before.1, before.2));
    assert!(matches!(
        route.session().header(),
        Err(channel::Refused::Capacity)
    ));
    drop(permit);
    assert_eq!(route.session().fixture_counts(), before);

    let auth = authenticated_header(route, &wire);
    let permit = state
        .voice_handoff
        .sdp_begin_intake(
            route.selected(),
            route.session().identity(),
            &auth,
            clock.clone(),
        )
        .unwrap();
    let funding = phase.native_history(NativeRowPurpose::Seal).unwrap();
    assert_eq!(funding.in_flight(), 0);
    let mut slots = Vec::new();
    while let Ok(copy) = phase.copy(1) {
        slots.push(copy);
        assert!(slots.len() <= 16);
    }
    assert_eq!(
        slots.len(),
        15,
        "the actual outstanding PC completion owns the sixteenth Copy"
    );
    assert!(matches!(
        auth.admit_sdp(permit),
        Err(channel::Refused::Capacity)
    ));
    assert_eq!(
        route.session().fixture_counts(),
        before,
        "capacity does not debit either nonce"
    );
    assert_eq!(funding.in_flight(), 0);
    drop(slots);
    // The same immutable request may retry after a proven local capacity refusal. Full
    // ordinary group bytes cannot block the already prepaid fee, Buffer or RPC allowance.
    let held = resources
        .charge(
            group,
            GROUP_BYTES - resources.group_counts(group).unwrap().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let auth = authenticated_header(route, &wire);
    let permit = state
        .voice_handoff
        .sdp_begin_intake(
            route.selected(),
            route.session().identity(),
            &auth,
            clock.clone(),
        )
        .unwrap();
    let mut incoming = auth.admit_sdp(permit).unwrap();
    assert_eq!(funding.in_flight(), 1);
    let tail = incoming.remaining_mut().len();
    incoming
        .remaining_mut()
        .copy_from_slice(&wire[wire.len() - tail..]);
    incoming.advance(tail).unwrap();
    assert_eq!(incoming.open().unwrap(), plain);
    assert!(matches!(
        requests::sdp_action::Request::decode(
            incoming.operation(),
            incoming.kind(),
            incoming.logical(),
            incoming.plain().unwrap()
        ),
        Ok(requests::sdp_action::Request::Begin(_))
    ));
    assert_eq!(
        route.session().fixture_counts(),
        (true, header.0, before.2 + 1)
    );
    drop(incoming);
    assert_eq!(funding.in_flight(), 0);
    assert!(!funding.is_collected());
    drop(held);
    assert!(Arc::ptr_eq(
        &funding,
        &phase.native_history(NativeRowPurpose::Seal).unwrap()
    ));
    // Correct Header MAC, invalid body AEAD: one permanent physical debit and no Row or
    // purpose closure. A later genuine request still binds the SAME original funding.
    header.0 += 1;
    let mut bad = signed_frame(public, header, &plain);
    *bad.last_mut().unwrap() ^= 1;
    let auth = authenticated_header(route, &bad);
    let permit = state
        .voice_handoff
        .sdp_begin_intake(route.selected(), route.session().identity(), &auth, clock)
        .unwrap();
    let mut incoming = auth.admit_sdp(permit).unwrap();
    let tail = incoming.remaining_mut().len();
    incoming
        .remaining_mut()
        .copy_from_slice(&bad[bad.len() - tail..]);
    incoming.advance(tail).unwrap();
    assert_eq!(incoming.open(), Err(channel::Refused::Body));
    drop(incoming);
    assert_eq!(
        route.session().fixture_counts(),
        (false, header.0, before.2 + 2)
    );
    assert_eq!(funding.in_flight(), 0);
    assert!(!funding.is_collected());
    assert!(phase.effect().is_live());
}
