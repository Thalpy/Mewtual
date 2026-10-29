//! Actual admitted activation/key/Session, never an Existing grant conversion.
use super::*;
use crate::voice_authority::AdmittedPublishedKey;
use crate::voice_handoff::channel::SessionIdentity;
use crate::voice_handoff::ledger::{AdmittedParentClaim, Ledger};
use catcoms_sync::voice_v2::resources::{ResourceKind, GROUP_BYTES};
use catcoms_sync::voice_v2::VoiceError;

async fn physical() -> (
    Fixture,
    Arc<ManualClock>,
    Handle,
    AdmittedPublishedKey,
    Arc<SessionIdentity>,
) {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let output = ready(
        bounded(ensure_admitted(
            &f.state,
            handle.id(),
            [9; 32],
            &mut Rng::new(),
        ))
        .await
        .unwrap(),
    );
    let route = f
        .state
        .voice_channels
        .admitted_route(output.public().slot)
        .unwrap();
    let selected = route.selected().clone();
    let identity = route.session().identity().clone();
    drop(route);
    drop(output);
    (f, clock, handle, selected, identity)
}
async fn cleanup(f: Fixture, clock: Arc<ManualClock>, handle: Handle) {
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    end(&f, &clock, &handle).await;
    bounded(f.close()).await;
}

#[tokio::test]
async fn admitted_native_parent_keeps_exact_physical_provenance_and_two_original_fees() {
    let (f, clock, handle, selected, identity) = bounded(physical()).await;
    let ledger = f.state.voice_handoff.clone();
    let resources = selected.resources();
    let before = resources.counts();
    let claim = ledger
        .reserve_admitted_parent(selected.clone(), identity.clone(), 7)
        .unwrap();
    assert!(claim.matches(&selected, &identity, 7));
    assert!(claim.belongs_to(&ledger));
    assert!(!claim.belongs_to(&Arc::new(Ledger::default())));
    for generation in [0, 6, 8] {
        assert!(!claim.matches(&selected, &identity, generation));
    }
    assert!(!claim.matches(&selected, &Arc::new(SessionIdentity(identity.0)), 7));
    assert_eq!(resources.counts(), (before.0 + 2304 + 2048, before.1));
    let id = claim.fixture_id();
    assert!(ledger.fixture_admitted_claim_has_only_reserved_stamp(id));
    let graph = std::mem::size_of::<AdmittedParentClaim>()
        + AdmittedParentClaim::preparation_layout_bytes();
    eprintln!("complete admitted native parent graph {graph}/2304");
    assert!(graph <= 2304);
    drop(claim);
    assert_eq!(resources.counts(), before);
    assert!(!ledger.fixture_parent_reserved(id));
    drop(selected);
    drop(identity);
    cleanup(f, clock, handle).await;
}

#[tokio::test]
async fn admitted_and_existing_parent_claims_share_all_real_32_allocator_positions() {
    let (f, clock, handle, selected, identity) = bounded(physical()).await;
    let ledger = f.state.voice_handoff.clone();
    let before = selected.resources().counts();
    let mut claims = Vec::new();
    let mut existing = Vec::new();
    for generation in 1..=16 {
        claims.push(
            ledger
                .reserve_admitted_parent(selected.clone(), identity.clone(), generation)
                .unwrap(),
        );
        existing.push(crate::voice_handoff::ledger::fixture_existing_claim(
            &ledger,
        ));
    }
    let full = selected.resources().counts();
    let id = claims[0].fixture_id();
    crate::voice_handoff::ledger::fixture_existing_allocators_are_excluded(&ledger, id);
    assert!(matches!(
        ledger.reserve_admitted_parent(selected.clone(), identity.clone(), 17),
        Err(VoiceError::Capacity)
    ));
    assert_eq!(selected.resources().counts(), full);
    drop(claims.remove(0));
    let next = ledger
        .reserve_admitted_parent(selected.clone(), identity.clone(), 17)
        .unwrap();
    let old = id.logical();
    let new = next.fixture_id().logical();
    assert_eq!(&old[..8], &new[..8]);
    assert!(
        u64::from_be_bytes(new[8..].try_into().unwrap())
            > u64::from_be_bytes(old[8..].try_into().unwrap())
    );
    assert!(
        !ledger.fixture_parent_reserved(id),
        "an old stamp cannot name its successor"
    );
    assert!(ledger.fixture_admitted_claim_has_only_reserved_stamp(next.fixture_id()));
    drop(next);
    drop(claims);
    drop(existing);
    assert_eq!(selected.resources().counts(), before);
    drop(selected);
    drop(identity);
    cleanup(f, clock, handle).await;
}

#[tokio::test]
async fn admitted_native_parent_partial_capacity_refusal_spends_no_serial_and_keeps_original_key() {
    let (f, clock, handle, selected, identity) = bounded(physical()).await;
    let ledger = f.state.voice_handoff.clone();
    let resources = selected.resources();
    let group = selected.binding().scope.group;
    let before = resources.counts();
    assert!(matches!(
        ledger.reserve_admitted_parent(selected.clone(), identity.clone(), 0),
        Err(VoiceError::Unauthorized)
    ));
    assert_eq!(resources.counts(), before);
    for allowance in [0, 2304, 2304 + 2047] {
        let charged = resources.group_counts(group).unwrap().0;
        let pressure = resources
            .charge(
                group,
                GROUP_BYTES - charged - allowance,
                ResourceKind::Retained,
            )
            .unwrap();
        let full = resources.counts();
        assert!(matches!(
            ledger.reserve_admitted_parent(selected.clone(), identity.clone(), 1),
            Err(VoiceError::Capacity)
        ));
        assert_eq!(
            resources.counts(),
            full,
            "both speculative fees return on refusal"
        );
        drop(pressure);
        assert_eq!(resources.counts(), before);
    }
    let claim = ledger
        .reserve_admitted_parent(selected.clone(), identity.clone(), 1)
        .unwrap();
    assert_eq!(&claim.fixture_id().logical()[8..], &1_u64.to_be_bytes());
    assert!(claim.matches(&selected, &identity, 1));
    drop(claim);
    assert_eq!(resources.counts(), before);
    drop(selected);
    drop(identity);
    cleanup(f, clock, handle).await;
}

#[tokio::test]
async fn admitted_native_parent_end_does_not_refund_an_actual_claim_and_unwind_returns_its_stamp() {
    struct Unwind {
        _claim: AdmittedParentClaim,
    }
    impl Drop for Unwind {
        fn drop(&mut self) {
            panic!("controlled admitted native parent owner unwind");
        }
    }
    let (f, clock, handle, selected, identity) = bounded(physical()).await;
    let ledger = f.state.voice_handoff.clone();
    let resources = selected.resources();
    let claim = ledger
        .reserve_admitted_parent(selected.clone(), identity.clone(), 1)
        .unwrap();
    let id = claim.fixture_id();
    end(&f, &clock, &handle).await;
    assert!(!claim.matches(&selected, &identity, 1));
    assert!(ledger.fixture_admitted_claim_has_only_reserved_stamp(id));
    let retained = resources.counts();
    assert!(matches!(
        ledger.reserve_admitted_parent(selected.clone(), identity.clone(), 2),
        Err(VoiceError::Unauthorized)
    ));
    assert_eq!(resources.counts(), retained);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(Unwind {
            _claim: claim
        })))
        .is_err()
    );
    assert_eq!(resources.counts(), (retained.0 - 2304 - 2048, retained.1));
    assert!(!ledger.fixture_parent_reserved(id));
    drop(selected);
    drop(identity);
    cleanup(f, clock, handle).await;
}
