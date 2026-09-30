//! The IPC contract for the lifecycle family: what a renderer must type, what it may not, and the
//! shape it gets back. Disposal semantics are the store's tests; these are the boundary's.
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

const HEX: &str = "aa00000000000000000000000000000000000000000000000000000000000011";

fn parse_mode(value: Value) -> Result<StudioDisposalRequestMode, String> {
    serde_json::from_value::<DisposalInput>(value)
        .map_err(|e| e.to_string())?
        .checked()
}

#[test]
fn releasing_needs_the_exact_literal_the_user_typed() {
    let token = StudioReleaseConfirmation::TOKEN;
    assert_eq!(token, "release-local-archive");
    release_request(HEX, token).expect("the exact literal releases");
    // A caller that built the string rather than echoing the user gets these near misses. Each is
    // a separate case because each is a different plausible bug: trimming, casing, padding, and
    // reaching for the *other* destructive token in this family.
    for wrong in [
        "",
        "release-local-archives",
        "release_local_archive",
        "Release-Local-Archive",
        " release-local-archive",
        "release-local-archive ",
        StudioDiscardConfirmation::TOKEN,
    ] {
        let error = release_request(HEX, wrong).expect_err("release accepted a non-literal");
        assert!(
            error.contains("release-local-archive"),
            "refusal must name the literal, said: {error}"
        );
    }
}

#[test]
fn the_two_destructive_tokens_are_not_interchangeable() {
    // They destroy different things, so a dialog wired to the wrong one must fail rather than
    // silently destroy the other. This is the guard, not a restatement of the constants.
    assert_ne!(
        StudioReleaseConfirmation::TOKEN,
        StudioDiscardConfirmation::TOKEN
    );
    assert!(StudioReleaseConfirmation::parse(StudioDiscardConfirmation::TOKEN).is_none());
    assert!(StudioDiscardConfirmation::parse(StudioReleaseConfirmation::TOKEN).is_none());
}

#[test]
fn a_release_refuses_the_confirmation_before_it_reads_the_archive_id() {
    // Both are wrong here. The caller is told about the thing it cannot fix by retrying.
    let error = release_request("not hex", "").expect_err("release accepted nothing at all");
    assert!(error.contains("release-local-archive"), "said: {error}");
}

#[test]
fn a_release_still_refuses_an_archive_id_it_cannot_read() {
    for wrong in [
        "",
        "abcd",
        &HEX[..63],
        &format!("{HEX}0"),
        &HEX.to_uppercase(),
    ] {
        let error = release_request(wrong, StudioReleaseConfirmation::TOKEN)
            .expect_err("release accepted a malformed archive id");
        assert!(error.contains("64 lowercase hex"), "said: {error}");
    }
}

#[test]
fn discarding_needs_the_exact_literal_and_preserving_needs_no_literal_at_all() {
    assert_eq!(StudioDiscardConfirmation::TOKEN, "destroy-local-draft");
    assert!(matches!(
        parse_mode(json!({"kind":"preserve"})).unwrap(),
        StudioDisposalRequestMode::Preserve
    ));
    assert!(matches!(
        parse_mode(json!({"kind":"discard","confirmation":"destroy-local-draft"})).unwrap(),
        StudioDisposalRequestMode::Discard(_)
    ));
    for wrong in [
        "",
        "destroy-local-drafts",
        "Destroy-Local-Draft",
        "preserve",
    ] {
        let error = parse_mode(json!({"kind":"discard","confirmation":wrong}))
            .expect_err("discard accepted a non-literal");
        assert!(error.contains("destroy-local-draft"), "said: {error}");
    }
}

#[test]
fn a_disposal_that_meant_to_preserve_and_lost_its_kind_is_refused_not_guessed() {
    // The whole point of the tagged representation. A payload that is missing, misspells or
    // half-fills its mode must never resolve to the destructive arm by default.
    for payload in [
        json!({}),
        json!({"kind":"discard"}),
        json!({"kind":"destroy","confirmation":"destroy-local-draft"}),
        json!({"kind":"preserve","confirmation":"destroy-local-draft"}),
        json!({"kind":"preserve","archive":HEX}),
        json!({"confirmation":"destroy-local-draft"}),
    ] {
        assert!(
            parse_mode(payload.clone()).is_err(),
            "a malformed mode resolved to something: {payload}"
        );
    }
}

#[test]
fn a_disposal_payload_must_carry_every_value_the_user_was_shown() {
    let whole = json!({"branch":HEX,"content":HEX,"accepted":1,"mode":{"kind":"preserve"}});
    serde_json::from_value::<DisposalRequestInput>(whole.clone())
        .unwrap()
        .checked()
        .expect("a complete payload");
    // Dropping any one field is a refusal rather than a defaulted value, because each of the three
    // is a separate check and a defaulted one silently stops checking.
    for missing in ["branch", "content", "accepted", "mode"] {
        let mut payload = whole.clone();
        payload.as_object_mut().unwrap().remove(missing);
        assert!(
            serde_json::from_value::<DisposalRequestInput>(payload).is_err(),
            "a disposal defaulted its {missing}"
        );
    }
    // And a payload carrying something the command does not know about is refused rather than
    // quietly ignored: it is evidence the renderer and this boundary disagree about the request.
    let mut extra = whole;
    extra
        .as_object_mut()
        .unwrap()
        .insert("archive".into(), HEX.into());
    assert!(serde_json::from_value::<DisposalRequestInput>(extra).is_err());
}

#[tokio::test]
async fn lifecycle_classifies_a_live_branch_without_rebuilding_or_writing_it() {
    for art in [false, true] {
        let f = InspectionFixture::new(art).await;
        let state = state(&f).await;
        let before = f.records();
        let value = studio_overlay_lifecycle_for_test(&state, f.target)
            .await
            .unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["kind"], "overlayLifecycle");
        assert_eq!(
            value["channel"],
            u128::from_be_bytes(f.target.channel()).to_string()
        );
        match f.target {
            StudioTarget::Index { .. } => assert!(value["object"].is_null()),
            StudioTarget::Flipnote { object, .. } => {
                assert_eq!(value["object"], hex::encode(object));
            }
        }
        assert!(
            value["branch"].is_string(),
            "a live accepted branch has an identity"
        );
        // Without this a renderer cannot address a disposal at all: the store demands the branch's
        // content digest back and this view is the only place it is published.
        assert!(
            value["content"].is_string(),
            "a live branch must publish the content digest a disposal has to echo"
        );
        assert_ne!(
            value["content"], value["branch"],
            "identity and content are different checks and must not be the same value"
        );
        assert_eq!(value["generation"], "1", "the first minted generation");
        assert_eq!(value["accepted"], 1);
        assert!(value["archive"].is_null(), "nothing has been preserved yet");
        assert!(value["disposed"].is_null(), "nothing has been disposed yet");
        assert_eq!(value["transferred"], false);
        // Classification is the cheap step. If it ever writes, it stops being the thing a caller
        // can run before deciding whether to pay for an inspection.
        assert_eq!(f.records(), before, "classification wrote to the vault");
        f.shutdown().await;
    }
}

#[tokio::test]
async fn reading_an_archive_that_was_never_written_is_a_refusal_not_an_empty_view() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let error = recovery::invoke_control(
        &state,
        fixture::SERVER,
        f.target,
        Action::ReadOverlayArchive,
    )
    .await
    .expect_err("absent evidence rendered as a view");
    assert!(
        error.contains("no preserved draft archive"),
        "said: {error}"
    );
    f.shutdown().await;
}

#[tokio::test]
async fn a_live_branch_survives_every_refused_disposal() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let live = studio_overlay_lifecycle_for_test(&state, f.target)
        .await
        .unwrap();
    let branch = live["branch"].as_str().unwrap().to_owned();
    let content = live["content"].as_str().unwrap().to_owned();
    let before = f.records();

    // Naming another branch, disagreeing about content, and disagreeing about size are three
    // separate refusals, and none of them may cost the branch anything. The fourth is D4: a
    // preserving disposal with nothing preserved, which would otherwise lose the bodies while
    // claiming they were kept.
    for (reason, payload) in [
        (
            "another generation",
            json!({"branch":HEX,"content":content,"accepted":1,"mode":{"kind":"preserve"}}),
        ),
        (
            "content changed under the dialog",
            json!({"branch":branch,"content":HEX,"accepted":1,"mode":{"kind":"preserve"}}),
        ),
        (
            "size disagreement",
            json!({"branch":branch,"content":content,"accepted":99,"mode":{"kind":"preserve"}}),
        ),
        (
            "preserving with nothing preserved",
            json!({"branch":branch,"content":content,"accepted":1,"mode":{"kind":"preserve"}}),
        ),
    ] {
        let request = serde_json::from_value::<DisposalRequestInput>(payload)
            .expect("a well-formed payload")
            .checked()
            .expect("a well-formed payload");
        recovery::invoke_control(
            &state,
            fixture::SERVER,
            f.target,
            Action::DisposeOverlay(Box::new(request)),
        )
        .await
        .expect_err(reason);
        assert_eq!(f.records(), before, "a refused disposal wrote: {reason}");
        assert_eq!(
            studio_overlay_lifecycle_for_test(&state, f.target)
                .await
                .unwrap(),
            live,
            "a refused disposal changed the branch: {reason}"
        );
    }
    f.shutdown().await;
}

#[tokio::test]
async fn a_confirmed_discard_ends_the_branch_and_says_so_terminally() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let live = studio_overlay_lifecycle_for_test(&state, f.target)
        .await
        .unwrap();
    let branch = live["branch"].as_str().unwrap().to_owned();

    let request = serde_json::from_value::<DisposalRequestInput>(json!({
        "branch": branch,
        "content": live["content"],
        "accepted": 1,
        "mode": {"kind":"discard","confirmation":"destroy-local-draft"},
    }))
    .expect("the payload a renderer sends")
    .checked()
    .unwrap();
    let manifest = recovery::invoke_control(
        &state,
        fixture::SERVER,
        f.target,
        Action::DisposeOverlay(Box::new(request)),
    )
    .await
    .expect("a fully specified, confirmed discard");

    assert_eq!(manifest["v"], 1);
    assert_eq!(manifest["kind"], "overlayDisposed");
    assert_eq!(manifest["branch"], branch);
    assert_eq!(manifest["generation"], "1");
    assert_eq!(manifest["accepted"], 1);
    assert_eq!(manifest["disposal"], json!({"mode":"discarded"}));
    assert_eq!(manifest["terminal"], true);
    assert_eq!(manifest["author"], hex::encode(f.device.as_bytes()));
    assert_eq!(manifest["provenance"], json!({"kind":"closing"}));
    assert_eq!(manifest["basis"], hex::encode(f.basis));
    assert!(
        manifest["disposal"].get("archive").is_none(),
        "a discard must not point at bodies that do not exist"
    );

    // The terminal manifest is retained, so the branch is gone but the event is not.
    let after = studio_overlay_lifecycle_for_test(&state, f.target)
        .await
        .unwrap();
    assert!(
        after["branch"].is_null(),
        "the branch outlived its disposal"
    );
    assert_eq!(after["accepted"], 0);
    assert_eq!(after["disposed"], json!({"mode":"discarded"}));
    assert!(after["archive"].is_null());
    f.shutdown().await;
}

/// The command bodies take `State<'_, AppState>`, which a unit test cannot mint. This is the same
/// body, one layer in, so the tests above exercise the real target resolution and dispatch rather
/// than a hand-rolled request.
async fn studio_overlay_lifecycle_for_test(
    state: &AppState,
    target: StudioTarget,
) -> Result<Value, String> {
    recovery::invoke_control(state, fixture::SERVER, target, Action::OverlayLifecycle).await
}
