use super::*;
#[path = "../../../../../../crates/catcoms-app/tests/support/studio_preview.rs"]
mod fixture;
use fixture::PreviewFixture;

async fn check_preview_conversion(expire_after_conversion: bool) {
    for art in [false, true] {
        let f = PreviewFixture::new(art).await;
        f.wait_ready().await;
        let state = AppState {
            store: f.store.clone(),
            ..Default::default()
        };
        *state.session_resumable.lock().await = true;
        install(&state, f.actor.clone(), f.group.clone(), f.device, 1).await;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        let converted = std::cell::Cell::new(false);
        let result = invoke_custody(
            &state,
            fixture::SERVER,
            InvokeRequest::Document(StudioRequest::Read { target: f.target }),
            |response| {
                let InvokeResponse::Document(Some(StudioRead::AwaitingTenureReceipt(preview))) =
                    response
                else {
                    panic!(
                        "native regression requires an actual actor-produced provisional response"
                    );
                };
                let delivery = preview.delivery();
                assert!(delivery.is_current());
                preview
                    .inspect(|id, projection| {
                        assert_eq!(id, f.epoch_id);
                        assert_eq!(projection, &f.expected);
                    })
                    .unwrap();
                let value = read_view(StudioRead::AwaitingTenureReceipt(preview))?;
                assert_eq!(
                    value["awaitingTenureReceipt"], true,
                    "preview trust-state flag"
                );
                assert_eq!(value["provisional"], true);
                assert_eq!(value["epochId"], format!("{:032x}", f.epoch_id));
                assert_eq!(value["epoch"], "1");
                assert_eq!(value["channel"], channel());
                assert!(value.get("phase").is_none());
                assert!(value.get("publication").is_none());
                // Compare the entire existing conflict-preserving representation, then require
                // concrete seed and signed-tail data so an empty fixture cannot satisfy it.
                assert_eq!(value["content"], projection_content(&f.expected));
                if art {
                    assert_eq!(
                        value["content"]["title"]["selected"]["value"],
                        fixture::TITLE
                    );
                    assert_eq!(
                        value["content"]["title"]["selected"]["source"]["author"],
                        f.author
                    );
                    assert_eq!(
                        value["content"]["timeline"],
                        json!([hex::encode(fixture::FRAME)])
                    );
                    let pixels = &value["content"]["frames"][hex::encode(fixture::FRAME)]["pixels"]
                        ["selected"]["value"];
                    assert_eq!(pixels["cid"], hex::encode(fixture::CID));
                    assert_eq!(pixels["bytes"], fixture::FRAME_BYTES);
                } else {
                    let object = &value["content"]["objects"][hex::encode(fixture::OBJECT)];
                    assert_eq!(object["title"]["selected"]["value"], fixture::TITLE);
                    assert_eq!(object["title"]["selected"]["source"]["author"], f.author);
                    assert_eq!(
                        object["expiry"]["selected"]["value"],
                        json!({"kind":"never"})
                    );
                }
                converted.set(true);
                assert!(
                    delivery.is_current(),
                    "conversion itself succeeded while valid"
                );
                if expire_after_conversion {
                    f.clock.advance_ms(60_000);
                }
                assert_eq!(delivery.is_current(), !expire_after_conversion);
                // No await or second request occurs here. Instance replacement is excluded by
                // the actual native guard; session fields and cancellation stay untouched.
                assert!(state.servers.try_lock().is_err());
                assert!(state.ui_session_commit.try_lock().is_err());
                assert!(!state.session_lock_requested.load(Ordering::Acquire));
                assert_eq!(
                    state.ui_session_generation.load(Ordering::Acquire),
                    generation
                );
                Ok(value)
            },
        )
        .await;
        assert!(
            converted.get(),
            "must reach successful provisional conversion"
        );
        if expire_after_conversion {
            assert_eq!(
                result.expect_err("expired preview escaped final native delivery fence"),
                "Studio response belongs to a locked or changed UI session",
                "expired preview must fail the final native delivery fence"
            );
        } else {
            assert_eq!(result.unwrap()["awaitingTenureReceipt"], true);
        }
        assert_eq!(state.servers.lock().await[&fixture::SERVER].instance, 1);
        f.shutdown().await;
    }
}

#[tokio::test]
async fn native_studio_preview_serializes_real_actor_content() {
    check_preview_conversion(false).await;
}

#[tokio::test]
async fn native_studio_preview_expiring_after_conversion_is_rejected() {
    check_preview_conversion(true).await;
}
