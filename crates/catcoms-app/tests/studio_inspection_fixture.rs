use catcoms_app as app;
#[path = "support/studio_inspection.rs"]
mod fixture;
#[path = "support/studio_inspection_shapes.rs"]
mod shapes;
use app::studio::{StudioControlAction, StudioControlResponse};

#[test]
fn studio_inspection_maximal_typed_seed_shapes() {
    use app::studio::types::StudioTarget;
    for target in [
        StudioTarget::Index { channel: [7; 16] },
        StudioTarget::Flipnote {
            channel: [7; 16],
            object: [9; 16],
        },
    ] {
        shapes::maximal(b"maximal-inspection-codec", target);
    }
}

#[tokio::test]
async fn native_inspection_fixture_obtains_real_actor_index_and_flipnote_drafts() {
    for art in [false, true] {
        let f = fixture::InspectionFixture::new(art).await;
        assert!(!f.group.is_empty());
        assert!(!f.device.to_string().is_empty());
        let before = f.records();
        let prepared = f.capture().await.rebuild().await.unwrap();
        let StudioControlResponse::OverlayInspection(read) = f
            .control(StudioControlAction::FinishOverlayInspection(Box::new(
                prepared,
            )))
            .await
            .unwrap()
        else {
            panic!("expected real actor inspection")
        };
        read.inspect(|target, prepared, draft| {
            assert_eq!(target, f.target);
            assert!(!prepared);
            let draft = draft.unwrap();
            assert_eq!(draft.basis(), f.basis);
            assert_eq!(draft.accepted(), 1);
            assert_eq!(draft.projection(), &f.expected);
        })
        .unwrap();
        assert_eq!(f.records(), before, "inspection wrote durable records");
        let delivery = read.delivery();
        assert!(delivery.is_current());
        f.clock.advance_ms(5000);
        assert!(!delivery.is_current());
        drop(delivery);
        drop(read);
        f.shutdown().await;
    }
}
