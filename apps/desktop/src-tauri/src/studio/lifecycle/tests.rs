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

/// Every manual reason's wire name, pinned to design section 11's `OverlayManualReason` union.
///
/// A renderer will switch on these literals, and a misspelt one would reach it as an unknown reason
/// with no compile error on either side; the fixture test only checks that `manualReason` is a
/// string. The `expected` match has no catch-all, so a new reason fails to compile here until its
/// name is written down, and the names must stay distinct.
///
/// **One-sided today.** No frontend code consumes `manualReason` yet (the commands are unregistered
/// and section 11's TypeScript union is Agent 4's to apply), so this pins only the native side.
/// Whoever adds the renderer needs a matching test on the TypeScript side.
#[test]
fn every_manual_reason_crosses_under_its_section_11_name() {
    use catcoms_app::studio::types::{
        StudioOverlayEligibility as E, StudioOverlayManualReason as R,
    };
    let expected = |reason: R| match reason {
        R::NotReplayable => "notReplayable",
        R::Unconfirmed => "unconfirmed",
        R::NotCurrentAuthor => "notCurrentAuthor",
        R::SourceMissing => "sourceMissing",
        R::SourceUnreadable => "sourceUnreadable",
        R::Fault => "fault",
        R::SourceRewound => "sourceRewound",
        R::SourceNotClosing => "sourceNotClosing",
        R::SuccessorMissing => "successorMissing",
        R::ReceiptChanged => "receiptChanged",
        R::SourceReplaced => "sourceReplaced",
        R::SuccessorNotPristine => "successorNotPristine",
        R::ObjectMissing => "objectMissing",
        R::TenureUnknown => "tenureUnknown",
        R::TenureImported => "tenureImported",
        R::PreparedStuck => "preparedStuck",
    };
    let every = [
        R::NotReplayable,
        R::Unconfirmed,
        R::NotCurrentAuthor,
        R::SourceMissing,
        R::SourceUnreadable,
        R::Fault,
        R::SourceRewound,
        R::SourceNotClosing,
        R::SuccessorMissing,
        R::ReceiptChanged,
        R::SourceReplaced,
        R::SuccessorNotPristine,
        R::ObjectMissing,
        R::TenureUnknown,
        R::TenureImported,
        R::PreparedStuck,
    ];
    let mut names = std::collections::BTreeSet::new();
    for reason in every {
        assert_eq!(
            eligibility_fields(Some(E::Manual(reason))),
            (Value::from("manual"), Value::from(expected(reason))),
            "{reason:?}"
        );
        assert!(names.insert(expected(reason)), "two reasons share a name");
    }
    assert_eq!(
        eligibility_fields(Some(E::Transferable)),
        (Value::from("transferable"), Value::Null)
    );
    assert_eq!(eligibility_fields(None), (Value::Null, Value::Null));
}

#[test]
fn an_uncertain_outcome_is_marked_and_a_plain_refusal_is_not() {
    // Release can fail after the unlink, and such a caller must reconcile rather than resend. A
    // renderer that read that as an ordinary refusal would keep showing an archive that is gone.
    let uncertain = classified(Err(format!(
        "{}: syncing the parent directory",
        catcoms_app::UNCERTAIN_OUTCOME
    )))
    .unwrap_err();
    assert!(
        uncertain.starts_with("outcome=uncertain; "),
        "said: {uncertain}"
    );
    // A guard refusal cost nothing and must not be dressed up as one that might have landed.
    let refused = classified(Err("the branch changed since it was inspected".into())).unwrap_err();
    assert_eq!(refused, "the branch changed since it was inspected");
    assert!(classified(Ok(json!({"ok": true}))).is_ok());
}

/// An archive finish that fails after its write is uncertain, and only then.
///
/// The two post-write failures that produce no result at all - the store's own uncertain read-back
/// and the actor dropping the reply after the lease moved - must tell the caller to re-read. A
/// pre-write refusal must not be dressed up as one that might have landed, and an outcome that is
/// already classified passes through unchanged rather than being wrapped twice.
#[test]
fn an_archive_failure_is_uncertain_exactly_when_it_may_have_followed_the_write() {
    let store = format!(
        "{}: the draft archive was written but could not be read back",
        catcoms_app::UNCERTAIN_OUTCOME
    );
    for post_write in [
        store,
        catcoms_app::studio::CONTROL_REPLY_DROPPED.to_string(),
    ] {
        assert!(
            archive_failure(post_write.clone()).starts_with(super::super::ARCHIVE_MAYBE_WRITTEN),
            "a failure that may follow the write must be uncertain: {post_write}"
        );
    }
    let refused = "overlay inspection changed; refresh".to_string();
    assert_eq!(archive_failure(refused.clone()), refused);
    let classified = format!("{} (x)", super::super::UNDELIVERED_ARCHIVE);
    assert_eq!(archive_failure(classified.clone()), classified);
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
async fn lifecycle_classifies_a_live_branch_without_writing_to_the_vault() {
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
        let branch = &value["branch"];
        assert!(
            branch["branch"].is_string(),
            "a live accepted branch has an identity"
        );
        // Without this a renderer cannot address a disposal at all: the store demands the branch's
        // content digest back and this view is the only place it is published.
        assert!(
            branch["content"].is_string(),
            "a live branch must publish the content digest a disposal has to echo"
        );
        assert_ne!(
            branch["content"], branch["branch"],
            "identity and content are different checks and must not be the same value"
        );
        assert_eq!(branch["generation"], "1", "the first minted generation");
        assert_eq!(branch["accepted"], 1);
        assert_eq!(value["prepared"], false, "no transfer is staged");
        // P2: a live branch is always classified, with a reason exactly when it is manual.
        let eligibility = value["eligibility"]
            .as_str()
            .expect("a live branch must be classified");
        assert!(matches!(eligibility, "transferable" | "manual"));
        assert_eq!(
            value["manualReason"].is_string(),
            eligibility == "manual",
            "a reason exactly when manual, got {value}"
        );
        assert!(value["archive"].is_null(), "nothing has been preserved yet");
        assert!(value["disposed"].is_null(), "nothing has been disposed yet");
        assert_eq!(value["transferred"], false);
        // Classification is the cheap step. If it ever writes, it stops being the thing a caller
        // can run before deciding whether to pay for an inspection.
        assert_eq!(f.records(), before, "classification wrote to the vault");
        f.shutdown().await;
    }
}

/// The whole preserving lifecycle through the boundary: archive, read it back, see it in the
/// classifier, dispose preserving it, and confirm the manifest names that archive.
///
/// Until `studio_overlay_archive` existed no test in this crate could reach any of this, because
/// nothing outside the store could create an archive. The archive read, the export, the release
/// and the preserving disposal were all unreachable success paths.
#[tokio::test]
async fn the_preserving_lifecycle_runs_end_to_end_through_the_boundary() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let live = studio_overlay_lifecycle_for_test(&state, f.target)
        .await
        .unwrap();
    let branch = live["branch"]["branch"].as_str().unwrap().to_owned();

    let written = archive(&state, f.target)
        .await
        .expect("archiving a live draft");
    assert_eq!(written["kind"], "overlayArchived");
    assert_eq!(written["preserved"], true);
    assert_eq!(
        written["branch"], branch,
        "the archive names the live branch"
    );
    assert_eq!(written["generation"], "1");
    assert_eq!(
        written["replayable"], true,
        "this branch replays, so the archive says so"
    );
    assert!(written["notReplayable"].is_null());
    assert_eq!(written["accepted"], 1);
    // Reading evidence is never authority, and the view says so rather than leaving a renderer to
    // infer non-authority from a missing field.
    assert_eq!(written["readOnly"], true);
    assert_eq!(written["authority"], false);
    assert_eq!(written["terminal"], true);
    assert!(
        written.get("provisional").is_none(),
        "an archive is finished evidence, not an unsettled save"
    );
    let id = written["archive"].as_str().unwrap().to_owned();

    // The read returns the same archive, and the classifier now reports it against its branch.
    let read = recovery::invoke_control(
        &state,
        fixture::SERVER,
        f.target,
        Action::ReadOverlayArchive,
    )
    .await
    .expect("the archive that was just written must read back");
    assert_eq!(read["kind"], "overlayArchive");
    assert_eq!(read["archive"], id);
    assert_eq!(read["branch"], branch);
    let classified = studio_overlay_lifecycle_for_test(&state, f.target)
        .await
        .unwrap();
    assert_eq!(classified["archive"]["archive"], id);
    assert_eq!(
        classified["archive"]["branch"], branch,
        "the classifier must say which branch the archive is evidence for"
    );

    // The export carries the canonical envelope, and it round-trips.
    let exported = invoke_archive(&state, fixture::SERVER, f.target, |archive, id, bytes| {
        super::response_value(Response::OverlayArchive {
            archive: Box::new(archive),
            id,
            physical_bytes: bytes,
        })
    })
    .await
    .unwrap();
    assert_eq!(exported["archive"], id);

    // And D4 accepts it: the Preserve arm is reachable from the renderer at last.
    let request = serde_json::from_value::<DisposalRequestInput>(json!({
        "branch": branch,
        "content": live["branch"]["content"],
        "accepted": 1,
        "mode": {"kind":"preserve"},
    }))
    .unwrap()
    .checked()
    .unwrap();
    let manifest = recovery::invoke_control(
        &state,
        fixture::SERVER,
        f.target,
        Action::DisposeOverlay(Box::new(request)),
    )
    .await
    .expect("a preserving disposal with a durable archive must succeed");
    assert_eq!(
        manifest["disposal"],
        json!({"mode":"preserved","archive":id}),
        "the manifest must name the archive that holds the bodies"
    );

    // The classifier now shows a disposal and an archive that are about the *same* branch, which
    // is the only reading under which "preserved" is true of the work the user just disposed of.
    let after = studio_overlay_lifecycle_for_test(&state, f.target)
        .await
        .unwrap();
    assert!(after["branch"].is_null());
    assert_eq!(after["disposed"]["mode"], "preserved");
    assert_eq!(after["disposed"]["branch"], branch);
    assert_eq!(after["archive"]["branch"], branch);
    f.shutdown().await;
}

/// Export writes nothing, and it produces the **same payload** the archive would.
///
/// That sharing is the design's requirement, not an economy: a draft that cannot be replayed must
/// still be exportable, and two serializers would drift, with the drifted one being what a user
/// reaches for when their work will not open. Comparing the bytes is the only thing that holds
/// them together.
#[tokio::test]
async fn exporting_writes_nothing_and_produces_the_payload_the_archive_would() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let before = f.records();

    let exported = two_visit(
        &state,
        f.target,
        Action::ExportOverlay,
        Action::FinishOverlayExport,
    )
    .await
    .expect("exporting a live draft");
    assert_eq!(exported["kind"], "overlayExport");
    assert_eq!(exported["preserved"], false);
    // The literal from design section 11, which Agent 4's UI-hooks row is written against.
    assert_eq!(exported["format"], "catcoms-studio-draft-v1");
    assert!(exported["bytes"].as_u64().unwrap() > 0);
    assert!(
        exported.get("physicalBytes").is_none(),
        "nothing was written, so there is no physical size to report"
    );
    assert_eq!(
        f.records(),
        before,
        "export is the one member of this family that changes nothing"
    );

    // Now archive, and demand byte equality with what export just handed out.
    let written = archive(&state, f.target).await.unwrap();
    assert_eq!(written["archive"], exported["archive"]);
    let stored = invoke_archive(&state, fixture::SERVER, f.target, |archive, id, bytes| {
        let value = super::with_payload(super::archive_value(&archive, id, bytes)?, &archive)?;
        Ok(value)
    })
    .await
    .unwrap();
    assert_eq!(
        stored["bytesB64"], exported["bytesB64"],
        "the live export and the stored archive must be the same bytes"
    );
    f.shutdown().await;
}

/// Release destroys the archive, and only when named exactly.
#[tokio::test]
async fn releasing_destroys_only_the_archive_it_was_shown() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let written = archive(&state, f.target).await.unwrap();
    let id = written["archive"].as_str().unwrap().to_owned();

    // A confirmation cannot be spent on an archive the user never saw.
    let stale = release(&state, f.target, HEX, StudioReleaseConfirmation::TOKEN)
        .await
        .expect_err("a release naming another archive must be refused");
    assert!(!stale.is_empty());
    assert!(
        recovery::invoke_control(
            &state,
            fixture::SERVER,
            f.target,
            Action::ReadOverlayArchive
        )
        .await
        .is_ok(),
        "a refused release must leave the archive intact"
    );

    let released = release(&state, f.target, &id, StudioReleaseConfirmation::TOKEN)
        .await
        .expect("releasing the archive that was read");
    assert_eq!(released["kind"], "overlayArchiveReleased");
    // Both budgets are closed behind this, so the renderer must not assume it may write again.
    assert_eq!(released["reconcileRequired"], true);
    assert!(
        recovery::invoke_control(
            &state,
            fixture::SERVER,
            f.target,
            Action::ReadOverlayArchive
        )
        .await
        .is_err(),
        "the archive must be gone"
    );
    assert!(
        studio_overlay_lifecycle_for_test(&state, f.target)
            .await
            .unwrap()["archive"]
            .is_null(),
        "the classifier must stop reporting a released archive"
    );
    f.shutdown().await;
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
    let branch = live["branch"]["branch"].as_str().unwrap().to_owned();
    let content = live["branch"]["content"].as_str().unwrap().to_owned();
    let before = f.records();

    // **The first three are sent as `discard`, deliberately.** As `preserve` they are all refused
    // by D4 for having no archive, whichever D3 check is deleted, and the test passes while
    // proving nothing: a review verified exactly that by deleting the `branch` and `accepted`
    // checks and watching every assertion here still hold. A confirmed discard has nothing left to
    // stop it, so deleting the guard a case names really does destroy the branch, and the survival
    // assertion below really does catch it.
    //
    // Each expected message is asserted too. Three guards that all refuse is not the same as three
    // guards that each refuse for its own reason, and only the second is what D3 claims.
    for (reason, says, payload) in [
        (
            "another generation",
            "names another branch generation",
            json!({"branch":HEX,"content":content,"accepted":1,
                "mode":{"kind":"discard","confirmation":"destroy-local-draft"}}),
        ),
        (
            "content changed under the dialog",
            "the branch changed since it was inspected",
            json!({"branch":branch,"content":HEX,"accepted":1,
                "mode":{"kind":"discard","confirmation":"destroy-local-draft"}}),
        ),
        (
            "size disagreement",
            "disagrees with the branch's accepted count",
            json!({"branch":branch,"content":content,"accepted":99,
                "mode":{"kind":"discard","confirmation":"destroy-local-draft"}}),
        ),
        // D4, which only the preserving arm has: destroying the bodies while claiming they were
        // kept is the one refusal that has to happen even when everything else agrees.
        (
            "preserving with nothing preserved",
            "archive",
            json!({"branch":branch,"content":content,"accepted":1,"mode":{"kind":"preserve"}}),
        ),
    ] {
        let request = serde_json::from_value::<DisposalRequestInput>(payload)
            .expect("a well-formed payload")
            .checked()
            .expect("a well-formed payload");
        let error = recovery::invoke_control(
            &state,
            fixture::SERVER,
            f.target,
            Action::DisposeOverlay(Box::new(request)),
        )
        .await
        .expect_err(reason);
        assert!(
            error.contains(says),
            "the refusal for {reason} must name {says}, said: {error}"
        );
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
    let branch = live["branch"]["branch"].as_str().unwrap().to_owned();

    let request = serde_json::from_value::<DisposalRequestInput>(json!({
        "branch": branch,
        "content": live["branch"]["content"],
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
    assert_eq!(after["disposed"]["mode"], "discarded");
    assert_eq!(
        after["disposed"]["branch"], branch,
        "a terminal record must say which branch it ended"
    );
    assert_eq!(after["disposed"]["generation"], "1");
    assert!(after["archive"].is_null());
    f.shutdown().await;
}

/// An archive whose result cannot be delivered is UNCERTAIN, not refused, because the write already
/// happened.
///
/// Delivering archive results through the inspection fence made this reachable: the handoff can
/// expire while native converts. Every withheld result used to read as a plain refusal, which for
/// an archive would tell a user nothing was preserved while the record sits on disk. Here the
/// handoff expires mid-conversion, and the caller must be told to re-read, and the archive must
/// really be there.
#[tokio::test]
async fn an_archive_whose_result_expires_before_delivery_is_uncertain_and_is_on_disk() {
    let f = InspectionFixture::new(true).await;
    let state = state(&f).await;
    let target = f.target;
    let context = InvokeContext::new(&state, fixture::SERVER, Some(target))
        .await
        .unwrap();
    let job = invoke_with_context(
        &state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target,
            action: Action::ArchiveOverlay,
        }),
        |response| match response {
            InvokeResponse::Control(Response::OverlayPreparation(job)) => Ok(job),
            _ => Err("mismatched archive response".into()),
        },
    )
    .await
    .unwrap();
    let prepared = job.rebuild_for_archive().await.unwrap();
    let withheld = invoke_with_context(
        &state,
        &context,
        InvokeRequest::Control(StudioControlRequest {
            target,
            action: Action::FinishOverlayArchive(Box::new(prepared)),
        }),
        |response| {
            // The handoff expires while native is converting.
            f.clock.advance_ms(5_000);
            match response {
                InvokeResponse::Control(response) => response_value(response),
                _ => Err("mismatched archive response".into()),
            }
        },
    )
    .await
    .expect_err("an expired archive result must not be delivered");
    assert!(
        withheld.starts_with(super::super::UNDELIVERED_ARCHIVE),
        "a written archive must be reported uncertain, said: {withheld}"
    );

    let after = studio_overlay_lifecycle_for_test(&state, target)
        .await
        .unwrap();
    assert!(
        after["archive"].is_object(),
        "the write the caller was told is uncertain must really have happened"
    );
    f.shutdown().await;
}

/// The real two-visit body: the detached rebuild and both custody visits are the production ones,
/// and only the `State` wrapper the `#[tauri::command]` needs is absent.
async fn two_visit(
    state: &AppState,
    target: StudioTarget,
    begin: Action,
    finish: impl FnOnce(Box<StudioPreparedInspection>) -> Action,
) -> Result<Value, String> {
    two_visit_archive(state, fixture::SERVER, target, begin, finish, "test").await
}
async fn archive(state: &AppState, target: StudioTarget) -> Result<Value, String> {
    two_visit(
        state,
        target,
        Action::ArchiveOverlay,
        Action::FinishOverlayArchive,
    )
    .await
}

async fn release(
    state: &AppState,
    target: StudioTarget,
    id: &str,
    confirmation: &str,
) -> Result<Value, String> {
    let request = release_request(id, confirmation)?;
    recovery::invoke_control(
        state,
        fixture::SERVER,
        target,
        Action::ReleaseOverlayArchive(Box::new(request)),
    )
    .await
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
