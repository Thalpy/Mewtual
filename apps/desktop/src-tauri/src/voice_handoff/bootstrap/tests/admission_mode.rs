use super::*;

use crate::voice_handoff::requests::negotiation_admission::{Kind, Request};

async fn route(f: &Fixture) -> Route {
    let (owner, _) = select(f, point()).await;
    let public = ready(
        ensure(&f.state, &owner, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    f.state.voice_channels.route(public.public.slot).unwrap()
}
fn request(kind: Kind, generation: u64) -> Request {
    Request { kind, generation }
}

#[tokio::test]
async fn admission_pre_ack_and_pinned_routes_refuse_actual_pilot_factories_without_new_rows() {
    use crate::voice_handoff::factory::{self, native};
    let f = Fixture::new().await;
    let route = route(&f).await;
    let context = crate::voice_handoff::context::Context::new(&f.state);
    let selector = catcoms_sync::voice_v2::presence::PairSelector {
        peer: catcoms_app::DeviceId::from_bytes([91; 32]),
        join: [8; 32],
        key_ref: [7; 32],
        epoch: 0,
    };
    for pinned in [false, true] {
        if pinned {
            route.admission_mode(request(Kind::Register, 1)).unwrap();
            route.acknowledge_setup().unwrap();
        }
        let before = route.selected().resources().counts();
        assert_eq!(
            native::discover_derivation(&context, &route, 0).err(),
            Some(factory::Refused::Closed)
        );
        assert_eq!(
            native::produce_derivation(&context, &route, selector).err(),
            Some(factory::Refused::Closed)
        );
        let result = factory::ensure_derivation_row(
            &context,
            &route,
            selector,
            factory::tests::physical(&route),
        )
        .await;
        assert_eq!(result, Err(factory::Refused::Closed));
        assert!(!f
            .state
            .voice_handoff
            .derivation_known(route.selected(), selector));
        assert_eq!(route.selected().resources().counts(), before);
        // No detached native worker retained this physical Session's only position.
        drop(route.session().native_hold().unwrap());
    }
    f.close().await;
}

#[tokio::test]
async fn admission_pin_before_ack_is_sticky_across_reply_loss_cancel_and_late_ack() {
    for first in [Kind::Register, Kind::Cancel] {
        let f = Fixture::new().await;
        let route = route(&f).await;
        let before = route.selected().resources().counts();
        assert!(!route.native_ready());
        assert_eq!(route.require_pilot_open(), Err(Refused::Binding));
        let pinned = route.admission_mode(request(first, 1)).unwrap();
        assert_eq!(pinned[1], if first == Kind::Register { 1 } else { 2 });
        // Lost pin reply: exact retry returns the SAME state and allocates nothing.
        assert_eq!(route.admission_mode(request(first, 1)).unwrap(), pinned);
        route.acknowledge_setup().unwrap();
        assert!(!route.native_ready());
        assert_eq!(route.require_pilot_open(), Err(Refused::Binding));
        assert!(
            route.require_open().is_ok(),
            "cleanup/ordinary channel routing stays available"
        );
        let cancelled = route.admission_mode(request(Kind::Cancel, 1)).unwrap();
        assert_eq!(cancelled[1], 2);
        assert_eq!(
            route.admission_mode(request(Kind::Register, 1)).unwrap(),
            cancelled
        );
        assert_eq!(
            route.admission_mode(request(Kind::Observe, 1)).unwrap(),
            cancelled
        );
        // Setup ACK returns its OWN original setup credit; pinning creates no owner.
        assert!(route.selected().resources().counts().0 < before.0);
        f.close().await;
    }
}

#[tokio::test]
async fn admission_ack_before_first_pin_refuses_upgrade_without_mutating_pilot() {
    let f = Fixture::new().await;
    let route = route(&f).await;
    assert_eq!(
        route.admission_mode(request(Kind::Observe, 1)).unwrap()[1],
        0
    );
    route.acknowledge_setup().unwrap();
    let before = route.selected().resources().counts();
    for kind in [Kind::Register, Kind::Cancel] {
        assert_eq!(
            route.admission_mode(request(kind, 1)),
            Err(Refused::Binding)
        );
        assert_eq!(
            route.admission_mode(request(Kind::Observe, 1)).unwrap()[1],
            0
        );
        assert!(route.native_ready() && route.require_pilot_open().is_ok());
    }
    assert_eq!(route.selected().resources().counts(), before);
    f.close().await;
}

#[tokio::test]
async fn admission_invalid_ordinals_cannot_select_mode_or_poison_a_valid_pin() {
    let f = Fixture::new().await;
    let route = route(&f).await;
    for ordinal in [0, 2, 1 << 53, u64::MAX] {
        for kind in [Kind::Register, Kind::Observe, Kind::Cancel] {
            assert_eq!(
                route.admission_mode(request(kind, ordinal)),
                Err(Refused::Bounds)
            );
        }
    }
    assert_eq!(
        route.admission_mode(request(Kind::Observe, 1)).unwrap()[1],
        0
    );
    let original = route.admission_mode(request(Kind::Register, 1)).unwrap();
    assert_eq!(
        route.admission_mode(request(Kind::Cancel, u64::MAX)),
        Err(Refused::Bounds)
    );
    assert_eq!(
        route.admission_mode(request(Kind::Observe, 1)).unwrap(),
        original
    );
    f.close().await;
}
