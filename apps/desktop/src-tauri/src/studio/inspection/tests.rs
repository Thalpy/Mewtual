use super::*;
use catcoms_app as app;
#[path = "../../../../../../crates/catcoms-app/tests/support/studio_inspection.rs"]
mod fixture;
#[path = "../../../../../../crates/catcoms-app/tests/support/studio_inspection_shapes.rs"]
mod shapes;
use fixture::InspectionFixture;

async fn state(f: &InspectionFixture) -> AppState {
    let state = AppState {
        store: f.store.clone(),
        ..Default::default()
    };
    *state.session_resumable.lock().await = true;
    super::super::tests::install(&state, f.actor.clone(), f.group.clone(), f.device, 1).await;
    state
}
fn check(value: &Value, f: &InspectionFixture) {
    assert_eq!(value["v"], 1);
    assert_eq!(
        value["kind"], "local-draft",
        "real local draft classification"
    );
    assert_eq!(value["basis"], hex::encode(f.basis));
    assert_eq!(value["accepted"], 1);
    assert_eq!(
        value["channel"],
        u128::from_be_bytes(f.target.channel()).to_string()
    );
    assert_eq!(value["transferState"], "active");
    assert_eq!(value["readOnly"], true);
    assert_eq!(value["content"], projection_content(&f.expected));
    for field in [
        "epochId",
        "epoch",
        "phase",
        "publication",
        "provisional",
        "awaitingTenureReceipt",
    ] {
        assert!(
            value.get(field).is_none(),
            "local inspection invented {field}"
        );
    }
    let title = match f.target {
        StudioTarget::Index { .. } => {
            assert!(value["object"].is_null());
            &value["content"]["objects"][hex::encode(fixture::ELEMENT)]["title"]["selected"]
        }
        StudioTarget::Flipnote { object, .. } => {
            assert_eq!(value["object"], hex::encode(object));
            assert_eq!(
                value["content"]["timeline"],
                json!([hex::encode(fixture::ELEMENT)])
            );
            &value["content"]["title"]["selected"]
        }
    };
    assert_eq!(title["value"], fixture::TITLE);
    assert_eq!(title["source"]["author"], f.device.to_string());
}

#[tokio::test]
async fn native_studio_inspection_serializes_actual_drafts_without_writes() {
    for art in [false, true] {
        let f = InspectionFixture::new(art).await;
        let state = state(&f).await;
        let before = f.records();
        check(&read(&state, fixture::SERVER, f.target).await.unwrap(), &f);
        assert_eq!(f.records(), before);
        // Exercise the shared fixture's direct actor capture too: no synthetic native-only result.
        drop(f.capture().await);
        f.shutdown().await;
    }
}

#[tokio::test]
async fn native_studio_inspection_expiring_after_conversion_is_rejected() {
    for art in [false, true] {
        let f = InspectionFixture::new(art).await;
        let state = state(&f).await;
        let generation = unlocked_ui_session_generation(&state).await.unwrap();
        let converted = std::cell::Cell::new(false);
        let result = read_with(
            &state,
            fixture::SERVER,
            f.target,
            |_| {},
            |read| {
                let delivery = read.delivery();
                assert!(delivery.is_current());
                let value = view(read)?;
                check(&value, &f);
                converted.set(true);
                assert!(delivery.is_current(), "successful conversion while valid");
                f.clock.advance_ms(5000);
                assert!(!delivery.is_current());
                assert_eq!(
                    state.ui_session_generation.load(Ordering::Acquire),
                    generation
                );
                assert!(!state.session_lock_requested.load(Ordering::Acquire));
                assert!(state.servers.try_lock().is_err());
                assert!(state.ui_session_commit.try_lock().is_err());
                Ok(value)
            },
        )
        .await;
        assert!(
            converted.get(),
            "must reach actual successful inspection conversion"
        );
        assert_eq!(
            result.expect_err("expired inspection escaped final native delivery fence"),
            "Studio response belongs to a locked or changed UI session"
        );
        assert_eq!(state.servers.lock().await[&fixture::SERVER].instance, 1);
        f.shutdown().await;
    }
}

#[tokio::test]
async fn native_studio_inspection_original_session_request_and_instance_span_both_visits() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    for mode in 0..3 {
        let reached = std::cell::Cell::new(false);
        let result = read_with(
            &state,
            fixture::SERVER,
            f.target,
            |original| {
                assert!(state.store.try_lock().is_ok());
                assert!(state.ui_session_commit.try_lock().is_ok());
                assert!(original.view_request.as_ref().unwrap().is_current());
                assert_eq!(
                    state.ui_session_generation.load(Ordering::Acquire),
                    original.generation
                );
                reached.set(true);
                match mode {
                    0 => {
                        state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
                    }
                    1 => {
                        drop(requests::ViewRequest::begin(
                            &state,
                            fixture::SERVER,
                            f.target,
                        ));
                    }
                    _ => {
                        state
                            .servers
                            .try_lock()
                            .unwrap()
                            .get_mut(&fixture::SERVER)
                            .unwrap()
                            .instance += 1;
                    }
                }
            },
            |_| panic!("obsolete first inspection was legitimized by second custody visit"),
        )
        .await;
        assert!(
            reached.get(),
            "must rebuild before changing original operation context"
        );
        assert!(result.is_err());
    }
    check(&read(&state, fixture::SERVER, f.target).await.unwrap(), &f);
    f.shutdown().await;
}

#[test]
fn native_studio_inspection_output_limit_counts_encoded_utf8_and_escapes() {
    let value = json!({"title":"\u{0000}\n雪🖊\"\\"});
    let actual = serde_json::to_vec(&value).unwrap().len();
    check_view_size(&value, actual).unwrap();
    assert!(check_view_size(&value, actual - 1).is_err());
    // Exactly the production output ceiling and one byte over it. The counting writer never
    // needs a second 32-MiB serialized buffer; JSON string quotes count toward the limit.
    let at_limit = Value::String("x".repeat(MAX_STUDIO_VIEW_BYTES - 2));
    assert!(bounded_view(at_limit).is_ok());
    let over_limit = Value::String("x".repeat(MAX_STUDIO_VIEW_BYTES - 1));
    assert!(
        bounded_view(over_limit).is_err(),
        "oversized native output escaped byte cap"
    );
}

#[tokio::test]
async fn native_studio_inspection_session_and_request_changed_after_conversion_are_rejected() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    for session in [false, true] {
        let converted = std::cell::Cell::new(false);
        let result = read_with(
            &state,
            fixture::SERVER,
            f.target,
            |_| {},
            |read| {
                let delivery = read.delivery();
                let value = view(read)?;
                check(&value, &f);
                converted.set(true);
                assert!(delivery.is_current());
                if session {
                    state.session_lock_requested.store(true, Ordering::Release);
                } else {
                    drop(requests::ViewRequest::begin(
                        &state,
                        fixture::SERVER,
                        f.target,
                    ));
                }
                assert!(
                    delivery.is_current(),
                    "delivery expiry must not mask native context rejection"
                );
                assert!(state.servers.try_lock().is_err());
                assert!(state.ui_session_commit.try_lock().is_err());
                Ok(value)
            },
        )
        .await;
        assert!(converted.get());
        assert_eq!(
            result.unwrap_err(),
            if session {
                "Studio response belongs to a locked or changed UI session"
            } else {
                "Studio view request was superseded; refresh"
            }
        );
        state.session_lock_requested.store(false, Ordering::Release);
    }
    f.shutdown().await;
}

#[test]
fn native_studio_inspection_maximal_typed_content_preserves_all_retained_conflicts() {
    for target in [
        StudioTarget::Index { channel: [7; 16] },
        StudioTarget::Flipnote {
            channel: [7; 16],
            object: [9; 16],
        },
    ] {
        let projection = shapes::maximal(b"maximal-inspection-codec", target);
        let value = bounded_view(projection_content(&projection)).unwrap();
        match &projection {
            StudioProjection::Index(p) => {
                assert_eq!(value["objects"].as_object().unwrap().len(), 64);
                for (id, entry) in &p.objects {
                    let actual = &value["objects"][hex::encode(id)];
                    assert_eq!(
                        actual["title"]["conflicts"].as_array().unwrap().len(),
                        entry.title.conflicts.len()
                    );
                    assert_eq!(
                        actual["expiry"]["conflicts"].as_array().unwrap().len(),
                        entry.expiry.conflicts.len()
                    );
                }
            }
            StudioProjection::Flipnote(p) => {
                assert_eq!(value["timeline"].as_array().unwrap().len(), 999);
                for (id, entry) in &p.frames {
                    let actual = &value["frames"][hex::encode(id)];
                    assert_eq!(
                        actual["insertions"].as_array().unwrap().len(),
                        entry.insertions.len()
                    );
                    assert_eq!(
                        actual["pixels"]["conflicts"].as_array().unwrap().len(),
                        entry.pixels.conflicts.len()
                    );
                }
            }
        }
        let encoded = serde_json::to_vec(&value).unwrap();
        check_view_size(&value, encoded.len()).unwrap();
        assert!(check_view_size(&value, encoded.len() - 1).is_err());
    }
}
