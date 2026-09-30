//! The copy boundary: what a renderer must send, and what it gets back.
//!
//! The fixture's retained draft is the copy *source*, and the destination is a **second Flipnote in
//! the same group**. That is not an arbitrary choice: this fixture exists because its document went
//! Closing, which is why a local draft was retained at all, so the document itself is not Open and
//! cannot be its own destination here. The cross-document case is also the more interesting one,
//! since it is the path where the planner drops the logical-key equality and keeps everything else.
use super::super::tests::fixture::{self, InspectionFixture};
use super::*;

async fn state(f: &InspectionFixture) -> AppState {
    let state = AppState {
        store: f.store.clone(),
        ..Default::default()
    };
    *state.session_resumable.lock().await = true;
    super::super::tests::install(&state, f.actor.clone(), f.group.clone(), f.device, 1).await;
    state
}

fn destination(target: StudioTarget) -> Value {
    json!({"channel":u128::from_be_bytes(target.channel()).to_string(),
    "object":match target {
        StudioTarget::Index{..} => Value::Null,
        StudioTarget::Flipnote{object,..} => hex::encode(object).into(),
    }})
}

const HEX: &str = "aa00000000000000000000000000000000000000000000000000000000000011";
const ELSEWHERE: [u8; 16] = [0x5e; 16];

/// A second Flipnote in the same channel, created through the ordinary Save path so it is genuinely
/// Open rather than assembled by the test.
async fn open_destination(f: &InspectionFixture, state: &AppState) -> StudioTarget {
    super::super::invoke(
        state,
        fixture::SERVER,
        StudioRequest::Create {
            channel: f.target.channel(),
            object: ELSEWHERE,
            nonce: [0xc1; 16],
            title: "somewhere to copy into".into(),
            ts: 123,
        },
    )
    .await
    .expect("an ordinary create");
    StudioTarget::Flipnote {
        channel: f.target.channel(),
        object: ELSEWHERE,
    }
}

#[test]
fn a_destination_must_be_named_completely_and_exactly() {
    let whole = json!({"channel":"7","object":HEX[..32].to_string()});
    serde_json::from_value::<DestinationInput>(whole.clone())
        .unwrap()
        .checked()
        .expect("a complete destination");
    // The Index has no object, and that is a legitimate destination rather than a missing field.
    serde_json::from_value::<DestinationInput>(json!({"channel":"7"}))
        .unwrap()
        .checked()
        .expect("an Index destination names no object");
    for bad in [
        json!({}),
        json!({"object":HEX[..32].to_string()}),
        json!({"channel":"7","object":HEX[..32].to_string(),"server":1}),
        json!({"channel":"07"}),
        json!({"channel":"7","object":"nothex"}),
    ] {
        assert!(
            serde_json::from_value::<DestinationInput>(bad.clone())
                .map_err(|e| e.to_string())
                .and_then(|d| d.checked())
                .is_err(),
            "a malformed destination resolved to something: {bad}"
        );
    }
}

#[test]
fn an_apply_payload_must_carry_every_value_the_preview_gave_it() {
    let whole = json!({
        "destination": {"channel":"7"},
        "choice": {"kind":"title","value":HEX},
        "mode": "copy",
        "epochId": HEX[..32].to_string(),
        "expectedProjection": HEX,
        "nonce": HEX[..32].to_string(),
        "body": "x",
    });
    serde_json::from_value::<CopyApplyInput>(whole.clone())
        .unwrap()
        .checked()
        .expect("a complete apply");
    // Each field is a separate check at the actor, so a defaulted one silently stops checking.
    for missing in [
        "destination",
        "choice",
        "mode",
        "epochId",
        "expectedProjection",
        "nonce",
        "body",
    ] {
        let mut payload = whole.clone();
        payload.as_object_mut().unwrap().remove(missing);
        assert!(
            serde_json::from_value::<CopyApplyInput>(payload).is_err(),
            "an apply defaulted its {missing}"
        );
    }
    // A payload the command does not understand is refused rather than ignored: it is evidence the
    // renderer and this boundary disagree about the request.
    let mut extra = whole.clone();
    extra
        .as_object_mut()
        .unwrap()
        .insert("sourceOps".into(), json!([HEX]));
    assert!(serde_json::from_value::<CopyApplyInput>(extra).is_err());

    // The body bound is the IPC bound, checked before anything reaches the actor.
    let mut huge = whole;
    huge.as_object_mut()
        .unwrap()
        .insert("body".into(), "x".repeat(64 * 1024 + 1).into());
    let error = serde_json::from_value::<CopyApplyInput>(huge)
        .unwrap()
        .checked()
        .expect_err("an over-large body must not reach the actor");
    assert!(error.contains("64 KiB"), "said: {error}");
}

#[test]
fn a_copy_mode_is_one_of_two_literals() {
    assert!(matches!(mode("restore"), Ok(StudioRecoveryMode::Restore)));
    assert!(matches!(mode("copy"), Ok(StudioRecoveryMode::Copy)));
    for wrong in ["", "Restore", "COPY", "preserve", "restore "] {
        assert!(mode(wrong).is_err(), "accepted mode {wrong:?}");
    }
}

/// The preview view names both ends and states plainly that a copy is not preservation.
#[tokio::test]
async fn a_copy_preview_proposes_a_body_and_says_it_preserves_nothing() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let into = open_destination(&f, &state).await;
    let before = f.records();

    let value = preview(
        &state,
        f.target,
        destination(into),
        json!({"kind":"title","value": title_op(&f).await}),
        "copy",
    )
    .await
    .expect("a cross-document copy of the draft's own title");

    assert_eq!(value["v"], 1);
    assert_eq!(value["kind"], "overlayCopyPreview");
    assert_eq!(value["disposition"], "ready");
    assert!(value["body"].is_string(), "a Ready plan proposes a body");
    assert_ne!(
        value["source"], value["destination"],
        "a cross-document copy names two documents"
    );
    assert_eq!(value["destination"], destination(into));
    // The one claim this view exists to refuse. A count of copied items never establishes that a
    // branch was preserved, so the view states it rather than leaving it to be inferred.
    assert_eq!(value["preservesBranch"], false);
    assert_eq!(
        value["sourceOps"].as_array().unwrap().len(),
        1,
        "a title copy resolves exactly one source operation"
    );
    assert!(value["expectedProjection"].is_string());
    assert!(value["provisional"].as_bool().unwrap());
    // Previewing is a proposal. It must cost the vault nothing.
    assert_eq!(f.records(), before, "a copy preview wrote to the vault");
    f.shutdown().await;
}

/// A preview whose echo has gone stale is refused at apply, and the vault is untouched.
#[tokio::test]
async fn an_apply_with_a_stale_echo_is_refused_and_writes_nothing() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let into = open_destination(&f, &state).await;
    let value = preview(
        &state,
        f.target,
        destination(into),
        json!({"kind":"title","value": title_op(&f).await}),
        "copy",
    )
    .await
    .unwrap();
    let before = f.records();

    // Every one of these is a value the actor re-derives rather than trusts.
    for (reason, field, wrong) in [
        ("another epoch", "epochId", json!(HEX[..32].to_string())),
        ("another projection", "expectedProjection", json!(HEX)),
        ("another body", "body", json!("not what was proposed")),
    ] {
        let mut payload = json!({
            "destination": destination(into),
            "choice": {"kind":"title","value": title_op(&f).await},
            "mode": "copy",
            "epochId": value["epochId"],
            "expectedProjection": value["expectedProjection"],
            "nonce": HEX[..32].to_string(),
            "body": value["body"],
        });
        payload.as_object_mut().unwrap().insert(field.into(), wrong);
        let edit = serde_json::from_value::<CopyApplyInput>(payload)
            .expect("a well-formed payload")
            .checked()
            .expect("a well-formed payload");
        recovery::invoke_control(
            &state,
            fixture::SERVER,
            f.target,
            Action::ApplyOverlayCopy(Box::new(edit)),
        )
        .await
        .expect_err(reason);
        assert_eq!(f.records(), before, "a refused apply wrote: {reason}");
    }
    f.shutdown().await;
}

/// The op id of the draft's own title, which is what a title copy has to name.
async fn title_op(f: &InspectionFixture) -> String {
    let prepared = f.capture().await.rebuild().await.unwrap();
    let catcoms_app::studio::StudioControlResponse::OverlayInspection(read) = f
        .control(
            catcoms_app::studio::StudioControlAction::FinishOverlayInspection(Box::new(prepared)),
        )
        .await
        .unwrap()
    else {
        panic!("not an inspection")
    };
    let mut id = None;
    read.inspect(|_, _, draft| {
        let catcoms_app::studio::types::StudioProjection::Flipnote(p) = draft.unwrap().projection()
        else {
            panic!("the art fixture projects a Flipnote")
        };
        id = Some(hex::encode(p.title.as_ref().unwrap().selected.source.op_id));
    })
    .unwrap();
    id.unwrap()
}

/// The real two-visit preview, one layer in from the `#[tauri::command]` wrapper.
async fn preview(
    state: &AppState,
    source: StudioTarget,
    destination: Value,
    choice: Value,
    mode: &str,
) -> Result<Value, String> {
    let choice = StudioOverlayCopyChoice {
        destination: serde_json::from_value::<DestinationInput>(destination)
            .map_err(|e| e.to_string())?
            .checked()?,
        item: serde_json::from_value::<ChoiceInput>(choice)
            .map_err(|e| e.to_string())?
            .checked()?,
        mode: self::mode(mode)?,
    };
    let context = InvokeContext::new(state, fixture::SERVER, Some(source)).await?;
    let job = invoke_with_context(
        state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target: source,
            action: Action::PrepareOverlayCopy(Box::new(choice)),
        }),
        |response| match response {
            InvokeResponse::Control(Response::OverlayCopyPreparation(job)) => Ok(job),
            _ => Err("mismatched overlay copy response".into()),
        },
    )
    .await?;
    let prepared = job.plan().await.map_err(|e| e.to_string())?;
    invoke_with_context(
        state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target: source,
            action: Action::FinishOverlayCopyPreview(Box::new(prepared)),
        }),
        |response| match response {
            InvokeResponse::Control(Response::OverlayCopyPreview(preview)) => {
                preview_value(&preview)
            }
            _ => Err("mismatched overlay copy response".into()),
        },
    )
    .await
}
