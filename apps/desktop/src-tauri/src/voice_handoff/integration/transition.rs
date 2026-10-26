//! Authenticated Claim/Release against the actual six-flight, actor-committed Finish.
//! These tests reuse the permitted TCP graph, not a literal confirmation flag or a
//! reconstructed Root plan. They do not claim browser key lifetime or RTC acceptance.
use super::*;
use crate::voice_handoff::transition as native_transition;
use catcoms_sync::voice_v2::VoiceError;

pub(super) fn body(parent: &ledger::ResolvedRow, proof: Option<[u8; 32]>) -> Vec<u8> {
    let selector = parent.confirmation_selector().unwrap();
    let (serial, version, input_tag) = parent.status().unwrap().action.unwrap();
    let mut body = vec![3];
    body.extend_from_slice(selector.peer.as_bytes());
    body.extend_from_slice(&selector.join);
    body.extend_from_slice(&selector.key_ref);
    body.extend_from_slice(&selector.epoch.to_be_bytes());
    body.extend_from_slice(&serial.to_be_bytes());
    body.extend_from_slice(&version.to_be_bytes());
    body.extend_from_slice(&input_tag);
    body.extend_from_slice(&parent.confirmation_handshake().unwrap());
    body.push(u8::from(proof.is_some()));
    if let Some(proof) = proof {
        body.extend_from_slice(&proof);
    }
    body
}

async fn frame(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    body: &[u8],
) -> channel::CompletedReply {
    let header = (
        route.session().fixture_counts().1 + 1,
        10,
        0,
        parent.id().logical(),
    );
    let reply = native_transition::apply(
        &context::Context::new(state),
        route,
        opened_frame(public, route, header, body, None),
    )
    .await
    .unwrap();
    // Inspect every actual authenticated reply, even those whose receipt is deliberately
    // ignored below to model a lost browser response. No fixture status bypass is used.
    let _ = opened_reply(public, header, body, &reply);
    reply
}
async fn exchange(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    body: &[u8],
) -> Vec<u8> {
    let sequence = route.session().fixture_counts().1 + 1;
    let reply = frame(state, public, route, parent, body).await;
    opened_reply(
        public,
        (sequence, 10, 0, parent.id().logical()),
        body,
        &reply,
    )
}

pub(super) enum Cleanup {
    Live,
    SdkAdmissionRace,
    Pc(u8),
    Ended,
    Orphan,
}

/// Real actor-committed Finish, framed admissions and original retained Root observers.
/// The four stages lose observation at Claim/key/storage/SDK boundaries. Closing intent
/// must permit exact history recovery while refusing every missing next admission; final
/// proof still waits for the actual copy/SDK/Claim/key obligations, without owner End.
pub(super) async fn fenced_custody_cycle(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    stage: u8,
    peers: &tokio::sync::Barrier,
) {
    assert!(stage <= 3);
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original native channel")
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    route.acknowledge_setup().unwrap();
    let context = context::Context::new(state);
    let clock = crate::voice_runtime(state).clock();
    let claim_body = body(parent, None);
    let capacity = replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
    let unauthorized = replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized));
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let reply = exchange(state, &public, &route, parent, &claim_body).await;
            if reply != capacity {
                break reply;
            }
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual original Claim did not admit");
    assert_eq!(&first[..2], &[3, 1]);
    let claim = first[182..214].try_into().unwrap();
    let key = if stage >= 1 {
        Some(super::key_session::adopt(state, &public, &route, parent, claim).await)
    } else {
        None
    };
    if stage >= 2 {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !parent.ready_test_complete() {
                clock.sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("actual Ready/core did not drain");
    }
    // Neither side may close its binding while its partner still needs Ready. Stages
    // 0/1 intentionally never wait for Ready, since one side has not adopted keys.
    peers.wait().await;
    let storage_body = key.map(|key| super::sdp::body(parent, key, None));
    let storage = if stage >= 2 {
        let prepared = super::sdp::exchange(
            state,
            &public,
            &route,
            parent,
            storage_body.as_ref().unwrap(),
        )
        .await;
        assert_eq!(&prepared[..2], &[5, 1]);
        Some(prepared[138..170].try_into().unwrap())
    } else {
        None
    };
    if stage == 2 {
        // Native-only race boundary: keep the actual admitted preparation/physical
        // owner while its original Plan closes. The peer stage3 covers framed SDK
        // receipt recovery and the independent retained-copy boundary below.
        super::sdk::fence_preparation_then_unwind(
            state,
            &public,
            &route,
            parent,
            storage_body.as_ref().unwrap(),
            storage.unwrap(),
            claim,
        )
        .await;
        crate::end_voice_owner_inner(state, selected.id()).await;
        return;
    }
    let sdk = if stage == 3 {
        Some(
            super::sdk::start(
                state,
                &public,
                &route,
                parent,
                storage_body.as_ref().unwrap(),
                storage.unwrap(),
            )
            .await,
        )
    } else {
        None
    };
    // This is an actual paid native owner, not a fixture busy flag. Source close, native
    // receipt recovery and final intent cannot release its original bytes or reservation.
    let copy = storage.map(|_| {
        parent
            .sdp_test_phase()
            .unwrap()
            .copy(8192)
            .unwrap()
            .adopt(vec![77; 8192])
            .unwrap()
    });
    let original_key = key.map(|_| parent.key_custody_for_test());
    let closing = plan_final::request(&context, &route, parent).await.unwrap();
    assert_eq!(closing.kind, ledger::FinalKind::Plan);
    assert!(
        closing.proof.is_none(),
        "intent cannot issue over unreleased original Claim"
    );
    assert!(plan_final::request(&context, &route, parent)
        .await
        .unwrap()
        .proof
        .is_none());
    if let Some(original) = &original_key {
        assert!(
            !original.is_current(),
            "fence must close the exact binding a parked worker owns"
        );
        assert!(matches!(
            original.prepare_sdp_handoff(),
            Err(VoiceError::Unauthorized)
        ));
    }
    let recovered = exchange(state, &public, &route, parent, &claim_body).await;
    assert_eq!(&recovered[..2], &[3, 2]);
    assert_eq!(&recovered[2..178], &first[2..178]);
    assert_eq!(&recovered[178..182], &[0; 4]);
    assert_eq!(&recovered[182..214], &claim);
    let (serial, version, input_tag) = parent.status().unwrap().action.unwrap();
    let action = requests::Action {
        serial,
        version,
        input_tag,
    };
    let handshake = parent.confirmation_handshake().unwrap();
    let key_reply = super::key_session::exchange(
        state,
        &public,
        &route,
        parent,
        &super::key_session::body(parent, claim, None),
    )
    .await;
    if let Some(key) = key {
        assert_eq!(&key_reply[..2], &[4, 2]);
        assert_eq!(&key_reply[202..234], &key);
        let request =
            requests::sdp::Request::decode(parent.id().logical(), storage_body.as_ref().unwrap())
                .unwrap();
        if storage.is_none() {
            assert!(
                matches!(
                    parent.recover_sdp(route.session().identity(), &request),
                    Err(VoiceError::Unauthorized)
                ),
                "frozen missing storage must refuse BEFORE Actor preflight"
            );
            assert_eq!(
                super::sdp::exchange(
                    state,
                    &public,
                    &route,
                    parent,
                    storage_body.as_ref().unwrap()
                )
                .await,
                unauthorized
            );
        }
        if let Some(proof) = storage {
            let mut sdk_body = storage_body.as_ref().unwrap()[..121].to_vec();
            sdk_body[0] = 6;
            sdk_body.extend_from_slice(&proof);
            sdk_body.push(0);
            let sdk_request =
                requests::sdk::Request::decode(parent.id().logical(), &sdk_body).unwrap();
            if sdk.is_none() {
                assert!(
                    matches!(
                        parent.recover_sdk(route.session().identity(), &sdk_request),
                        Err(VoiceError::Unauthorized)
                    ),
                    "frozen missing SDK must refuse BEFORE Actor/candidate construction"
                );
            }
            let reply = super::sdp::exchange(
                state,
                &public,
                &route,
                parent,
                &super::sdp::body(parent, key, Some(proof)),
            )
            .await;
            assert_eq!(&reply[..2], &[5, 2]);
            assert_eq!(copy.as_ref().unwrap().bytes()[0], 77);
            assert!(!parent.sdp_test_drained());
            assert!(plan_final::request(&context, &route, parent)
                .await
                .unwrap()
                .proof
                .is_none());
            drop(copy);
            let reply = super::sdp::exchange(
                state,
                &public,
                &route,
                parent,
                &super::sdp::body(parent, key, Some(proof)),
            )
            .await;
            assert_eq!(&reply[..2], &[5, 3]);
        }
        super::key_session::release(state, &public, &route, parent, claim, key, true).await;
        if let Some(sdk) = &sdk {
            super::sdk::release_after_key_release(state, &public, &route, parent, sdk).await;
        }
    } else {
        assert_eq!(key_reply, unauthorized);
        assert!(
            matches!(
                parent.recover_key_session(route.session().identity(), action, handshake, claim),
                Err(VoiceError::Unauthorized)
            ),
            "frozen missing key must refuse BEFORE Actor preflight"
        );
    }
    assert!(plan_final::request(&context, &route, parent)
        .await
        .unwrap()
        .proof
        .is_none());
    let released = exchange(state, &public, &route, parent, &body(parent, Some(claim))).await;
    assert_eq!(&released[..2], &[3, 3]);
    let final_receipt = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let receipt = plan_final::request(&context, &route, parent).await.unwrap();
            if receipt.proof.is_some() {
                break receipt;
            }
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual fenced plan did not drain after its original releases");
    parent
        .acknowledge_final(route.session().identity(), final_receipt.proof.unwrap())
        .unwrap();
    crate::end_voice_owner_inner(state, selected.id()).await;
}

pub(super) async fn custody_cycle(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    cleanup: Cleanup,
    peer_ready: &tokio::sync::Barrier,
) {
    let end_before_recovery = !matches!(
        cleanup,
        Cleanup::Live | Cleanup::SdkAdmissionRace | Cleanup::Pc(_)
    );
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original channel")
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    // This post-authentication fixture previously kept automatic production dormant. Its
    // setup has actually completed; allow the same native scheduler as a real browser ACK.
    route.acknowledge_setup().unwrap();
    let context = context::Context::new(state);
    let claim = body(parent, None);

    // Existing guards, not a test bool: contention before final admission installs nothing.
    let commit = state.ui_session_commit.clone().lock_owned().await;
    assert_eq!(
        exchange(state, &public, &route, parent, &claim).await,
        replies::refused(factory::Refused::Protocol(VoiceError::Capacity))
    );
    drop(commit);
    for offset in [1, 33, 65, 104, 105, 113, 152, 184] {
        let mut wrong = claim.clone();
        wrong[offset] ^= 1;
        let reply = exchange(state, &public, &route, parent, &wrong).await;
        assert_eq!(
            reply,
            replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized)),
            "changed selector/action/handshake at {offset} must not claim original keys"
        );
    }
    let clock = crate::voice_runtime(state).clock();
    // Actor preflight may be transiently busy with real mesh housekeeping. Only its exact
    // Capacity is retryable under this single fixed observer bound; no Root time is renewed.
    let claim_started = clock.monotonic_ms();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let reply = exchange(state, &public, &route, parent, &claim).await;
            if reply == replies::refused(factory::Refused::Protocol(VoiceError::Capacity)) {
                clock.sleep(std::time::Duration::from_millis(10)).await;
                continue;
            }
            break reply;
        }
    })
    .await
    .expect("original Claim admission stalled");
    assert_eq!(first.len(), native_transition::REPLY_BYTES);
    assert_eq!(&first[..2], &[3, 1]);
    assert_eq!(&first[2..18], &parent.id().logical());
    assert!(u32::from_be_bytes(first[178..182].try_into().unwrap()) > 0);
    // Use the earlier observation plus the actual ORIGINAL remaining Claim time. Both
    // foreign setup attempts share this conservative deadline; no retry renews it.
    let setup_deadline = claim_started
        .saturating_add(u32::from_be_bytes(first[178..182].try_into().unwrap()) as u64);
    let receipt: [u8; 32] = first[182..214].try_into().unwrap();
    let key_proof = super::key_session::adopt(state, &public, &route, parent, receipt).await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !parent.ready_test_complete() {
            clock.sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "automatic Ready/core stalled: (LOW, progress, released, current)={:?}; resources={:?}",
            parent.ready_test_observation(),
            selected.resources().counts()
        )
    });
    // Both native Ready consumers must finish before either endpoint intentionally releases
    // its key in this cleanup fixture. No synthetic Ready/CoreDrained flag is installed.
    peer_ready.wait().await;
    let (sdk, retired_sdp) = super::sdp::exercise(
        state,
        &public,
        &route,
        parent,
        key_proof,
        !matches!(cleanup, Cleanup::Ended),
        matches!(cleanup, Cleanup::SdkAdmissionRace),
        if let Cleanup::Pc(case) = cleanup {
            Some(case)
        } else {
            None
        },
    )
    .await;
    if matches!(cleanup, Cleanup::Pc(_)) {
        // Both live-peer PC scenarios finish before either endpoint intentionally Ends.
        // The earlier barrier established Ready only; it was not SDP/PC completion.
        peer_ready.wait().await;
    }
    assert!(
        matches!(parent.retire_plan(), Err(VoiceError::Capacity)),
        "browser key custody must precede normal whole-plan retirement"
    );
    // Early Plan intent is now an irreversible fence, not a non-mutating readiness
    // query. Keep this live scenario open; proof issuance still requires an intent.
    assert!(matches!(
        parent.issue_final(route.session().identity()),
        Err(VoiceError::Unauthorized)
    ));

    // A distinct actual Session allocation may have the same public hash/keys. It cannot
    // recover or release the original claim. Check the row boundary independently of the
    // canonical Registry gate, which also rejects this foreign channel in production.
    let foreign = bootstrap::Registry::default();
    let (_foreign_output, foreign_route) = bootstrap::fixture_installed_when_available(
        &foreign,
        selected.clone(),
        clock.as_ref(),
        setup_deadline,
    )
    .await;
    foreign_route.acknowledge_setup().unwrap();
    let (serial, version, input_tag) = parent.status().unwrap().action.unwrap();
    let action = requests::Action {
        serial,
        version,
        input_tag,
    };
    let handshake = parent.confirmation_handshake().unwrap();
    super::key_session::foreign_refuses(parent, &foreign_route, receipt, key_proof);
    if let Some(sdk) = &sdk {
        super::sdk::foreign_refuses(parent, &foreign_route, sdk);
    }
    let mono = clock.monotonic_ms();
    assert!(matches!(
        parent.recover_transition(foreign_route.session().identity(), action, handshake, mono),
        Err(VoiceError::Unauthorized)
    ));
    assert!(matches!(
        parent.release_transition(
            foreign_route.session().identity(),
            action,
            handshake,
            receipt,
            mono
        ),
        Err(VoiceError::Unauthorized)
    ));

    assert!(
        !parent.transition_test_state().3,
        "first Claim irreversibly freezes new children"
    );
    bootstrap::fixture_retire(&foreign, selected);
    drop((_foreign_output, foreign_route));
    foreign.collect_retired(&state.voice_handoff);
    assert!(
        !parent.transition_test_state().0,
        "foreign physical collection cannot cover original Claim"
    );
    // Construct the independent actual Session while this selection is still live.
    // A dead selection correctly refuses bootstrap; cleanup never revives it for tests.
    let final_foreign = bootstrap::Registry::default();
    let (_final_foreign_output, final_foreign_route) = bootstrap::fixture_installed_when_available(
        &final_foreign,
        selected.clone(),
        clock.as_ref(),
        setup_deadline,
    )
    .await;
    if end_before_recovery {
        // The first reply has been physically collected, but treat its browser receipt as
        // lost. End + lock must preserve same-session recovery and exact Release cleanup.
        crate::end_voice_owner_inner(state, selected.id()).await;
        crate::register_ui_lock_request(state);
    }
    let recovered = exchange(state, &public, &route, parent, &claim).await;
    assert_eq!(
        &recovered[2..178],
        &first[2..178],
        "original identity and first deadline never change"
    );
    assert_eq!(&recovered[182..214], &receipt);
    if end_before_recovery {
        assert_eq!(&recovered[..2], &[3, 2]);
        assert_eq!(&recovered[178..182], &[0; 4]);
    } else {
        assert_eq!(&recovered[..2], &[3, 1]);
        assert!(
            u32::from_be_bytes(recovered[178..182].try_into().unwrap())
                <= u32::from_be_bytes(first[178..182].try_into().unwrap())
        );
    }
    if matches!(cleanup, Cleanup::Orphan) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !parent.transition_test_state().1 {
                clock.sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("actual Root workflow did not drain after End");
        assert!(matches!(parent.retire_plan(), Err(VoiceError::Capacity)));
        state.voice_channels.stop_and_wait_for_physical().await;
        bootstrap::fixture_retire(&state.voice_channels, selected);
        drop((public, route));
        state.voice_channels.collect_retired(&state.voice_handoff);
        assert_eq!(
            parent.transition_test_state(),
            (true, true, false, false),
            "physical GC evidence must not fabricate Release or child authority"
        );
        assert!(
            state
                .voice_handoff
                .lookup_original(
                    selected,
                    ledger::Purpose::Derive(parent.confirmation_selector().unwrap())
                )
                .is_some(),
            "actual retained ResolvedRow still prevents orphan collection"
        );
        if let Some(sdk) = sdk {
            super::sdk::release_after_physical_collection(parent, sdk);
        }
        return;
    }
    let mut wrong = receipt;
    wrong[0] ^= 1;
    assert_eq!(
        exchange(state, &public, &route, parent, &body(parent, Some(wrong))).await,
        replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized))
    );
    // Exercise both release orders. Neither individual release can cover the other's
    // separately latched obligation; the old finite Claim never changes identity.
    if end_before_recovery {
        super::key_session::release(state, &public, &route, parent, receipt, key_proof, true).await;
        assert!(matches!(parent.retire_plan(), Err(VoiceError::Capacity)));
    }
    let release = body(parent, Some(receipt));
    for _ in 0..2 {
        // lost Release reply does not create a new obligation or deadline
        let released = exchange(state, &public, &route, parent, &release).await;
        assert_eq!(&released[..2], &[3, 3]);
        assert_eq!(&released[178..182], &[0; 4]);
        assert_eq!(&released[182..214], &receipt);
    }
    if !end_before_recovery {
        assert!(
            matches!(parent.retire_plan(), Err(VoiceError::Capacity)),
            "old Release alone cannot cover established key custody"
        );
        assert!(matches!(
            parent.issue_final(route.session().identity()),
            Err(VoiceError::Unauthorized)
        ));
        super::key_session::release(state, &public, &route, parent, receipt, key_proof, false)
            .await;
    }
    if let Some(sdk) = &sdk {
        super::sdk::release_after_key_release(state, &public, &route, parent, sdk).await;
    }
    let closed = exchange(state, &public, &route, parent, &claim).await;
    assert_eq!(&closed[..2], &[3, 3], "released Claim can never reopen");
    plan_final::assert_foreign_session_after_release(
        &context,
        &route,
        parent,
        &final_foreign_route,
    )
    .await;
    if !end_before_recovery {
        crate::end_voice_owner_inner(state, selected.id()).await;
    }
    // Release proves browser key custody ended, not that a real Source/Lower retired.
    // Keep the same final intent while its actual Root retirement witness drains.
    let final_receipt = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let receipt = plan_final::request(&context, &route, parent).await.unwrap();
            if receipt.proof.is_some() {
                break receipt;
            }
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original Root retirement did not drain after End");
    assert_eq!(final_receipt.kind, ledger::FinalKind::Plan);
    parent
        .acknowledge_final(route.session().identity(), final_receipt.proof.unwrap())
        .unwrap();
    if let Some(probe) = retired_sdp {
        // This is deliberately AFTER actual Key/Claim/SDK/PC release, End, Root
        // retirement, Plan compaction and the original final acknowledgement.
        probe
            .after_plan_retirement(state, &public, &route, parent)
            .await;
    }
}
