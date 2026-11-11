//! Actual whole-owner founder-as-ordinary-member vertical: real settled warm/cold/repaired
//! owners, the real empty promotion, the pin-authenticated genesis anchor, the production
//! general verifier and plan issuer, and the node's own retained device. No fixture
//! authority, no founder Header/Append, no member outbox save and no network are involved.
//! Nonfounder sends, removal/rejoin A1/A2 and later governing prefixes need the separate
//! joined-member/governance source and are not claimed here.
use super::super::super::super::{
    arm_member_close_during_sign_for_test, arm_owner_cancel_during_sign_for_test,
    member_close_drained_during_sign_for_test, member_sign_dispatches_for_test, V2MemberDependency,
    V2MemberSignRequest,
};
use super::*;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::{ActionId, Body, SignedAction, MAX_PARENTS, MAX_TEXT_BYTES};

/// Field order is Drop order: the host (and the whole original owner it holds) first, then
/// the layout, then the temporary directory LAST.
struct MemberFixture {
    host: PreparedV2TextHost<MemNetwork, ChaCha20Rng>,
    target: std::path::PathBuf,
    signer: [u8; 32],
    admission: OwnerFactId,
    layout: V2CurrentLayoutStore,
    _directory: tempfile::TempDir,
}
impl MemberFixture {
    fn head(&self) -> Vec<u8> {
        std::fs::read(self.target.join("head.bin")).unwrap()
    }
    /// Sign one request end to end through the host and hand back its exact signed action.
    fn send(
        &mut self,
        token: [u8; 32],
        body: &Body,
        parents: &[ActionId],
        dependencies: &[V2MemberDependency<'_>],
    ) -> SignedAction {
        self.host
            .retry_sign_member_request(
                &self.layout,
                V2MemberSignRequest {
                    token,
                    channel: 7,
                    body,
                    parents,
                    dependencies,
                },
            )
            .unwrap();
        let signed = self.host.take_member_signed_original(token).unwrap();
        assert!(signed.action().verify());
        assert!(signed.admission() == self.admission);
        let action = signed.action().clone();
        assert!(self
            .host
            .finish_member_request_saved(token, action.id())
            .unwrap());
        action
    }
}

fn promoted(family: usize, number: u64) -> MemberFixture {
    let original = text_fixture(family, number);
    let (target, signer, admission) = {
        let owner = original.owner.borrow_current().unwrap();
        let data = owner.source().data();
        (
            owner.source().source().capture.target.to_path_buf(),
            data.context.spec_data().signer,
            data.admission.admission_id(),
        )
    };
    let TextFixture {
        owner,
        layout,
        _directory,
    } = original;
    let mut host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_original_owner(owner),
        host_policy(),
    );
    host.retry_empty_promotion(&layout, &mut ChaCha20Rng::seed_from_u64(number ^ 0x55))
        .unwrap();
    // The empty promotion leaves no founder WAL: the "no WAL" assertions below and the
    // member anchor's crash-left-WAL refusal both start from that proven state.
    assert!(!target.join("text.wal").exists());
    assert!(!target.join("text.wal.stage.v1").exists());
    MemberFixture {
        host,
        target,
        signer,
        admission,
        layout,
        _directory,
    }
}

fn create(text: &str, reply_to: Option<ActionId>) -> Body {
    Body::Create {
        text: text.to_owned(),
        reply_to,
    }
}

fn plain(token: [u8; 32], body: &Body) -> V2MemberSignRequest<'_> {
    V2MemberSignRequest {
        token,
        channel: 7,
        body,
        parents: &[],
        dependencies: &[],
    }
}

fn join_token(index: usize) -> [u8; 32] {
    let mut token = [0x60; 32];
    token[..8].copy_from_slice(&(index as u64).to_be_bytes());
    token
}

#[test]
fn actual_three_family_founder_signs_successive_ordinary_sends_without_header_or_append() {
    for family in 0..3 {
        let mut f = promoted(family, 0x7fa00 + family as u64);
        let head = f.head();
        let dispatches = member_sign_dispatches_for_test();
        let mut encoded = Vec::new();
        for (index, token) in [[0x41u8; 32], [0x42; 32]].into_iter().enumerate() {
            let body = create(&format!("ordinary send {index}"), None);
            f.host
                .retry_sign_member_request(&f.layout, plain(token, &body))
                .unwrap();
            // Exact retries reuse the cached original: no draw, no key, the same bytes.
            f.host
                .retry_sign_member_request(&f.layout, plain(token, &body))
                .unwrap();
            assert_eq!(member_sign_dispatches_for_test(), dispatches + index + 1);
            let signed = f.host.take_member_signed_original(token).unwrap();
            assert!(signed.action().verify());
            assert_eq!(signed.action().signer(), &f.signer);
            assert!(signed.admission() == f.admission);
            // While lent, the same request is acknowledged without authoring again.
            f.host
                .retry_sign_member_request(&f.layout, plain(token, &body))
                .unwrap();
            assert!(f.host.take_member_signed_original(token).is_err());
            assert_eq!(member_sign_dispatches_for_test(), dispatches + index + 1);
            encoded.push(signed.action().encode());
            assert!(f
                .host
                .finish_member_request_saved(token, signed.action().id())
                .unwrap());
            // A finished token can never author again in this host lifetime.
            assert!(f
                .host
                .retry_sign_member_request(&f.layout, plain(token, &body))
                .is_err());
            // No founder certification: the selected O is untouched and no WAL exists.
            assert_eq!(f.head(), head);
            assert!(!f.target.join("text.wal").exists());
        }
        assert_ne!(encoded[0], encoded[1]);
        // The founder Create lane is excluded while the member lane exists.
        assert!(f.host.retry_token_lookup(&f.layout, [0x43; 32]).is_err());
        f.host.cancel_and_drain_admitted_promotion().unwrap();
        let body = create("after lock", None);
        assert!(f
            .host
            .retry_sign_member_request(&f.layout, plain([0x44; 32], &body))
            .is_err());
        assert_eq!(member_sign_dispatches_for_test(), dispatches + 2);
        assert_eq!(f.head(), head);
    }
}

#[test]
fn actual_reply_own_edit_delete_join_and_the_maximum_dependency_set_sign_in_any_order() {
    let mut f = promoted(0, 0x7fa10);
    let head = f.head();
    let admission = f.admission;
    let target = f.send([1; 32], &create("target", None), &[], &[]);
    let on_target = [V2MemberDependency {
        action: &target,
        admission,
    }];
    let reply = f.send(
        [2; 32],
        &create("reply", Some(target.id())),
        &[target.id()],
        &on_target,
    );
    assert!(
        matches!(reply.spec().body, Body::Create { reply_to: Some(t), .. } if t == target.id())
    );
    for (token, body) in [
        (
            [3; 32],
            Body::Edit {
                create: target.id(),
                text: "own edit".into(),
            },
        ),
        (
            [4; 32],
            Body::Join {
                create: target.id(),
            },
        ),
        (
            [5; 32],
            Body::Delete {
                create: target.id(),
            },
        ),
    ] {
        let action = f.send(token, &body, &[target.id()], &on_target);
        assert!(action.spec().body == body);
        assert!(action.spec().parents == vec![target.id()]);
    }
    // MAX_PARENTS Joins, then one Edit naming all of them: 64 distinct parents plus the
    // target is the maximum 65-dependency grammar. Parents and dependencies are supplied in
    // REVERSE order; the host canonicalises both.
    let mut joins = Vec::with_capacity(MAX_PARENTS);
    for index in 0..MAX_PARENTS {
        joins.push(f.send(
            join_token(index),
            &Body::Join {
                create: target.id(),
            },
            &[target.id()],
            &on_target,
        ));
    }
    let parents: Vec<ActionId> = joins.iter().map(SignedAction::id).rev().collect();
    let mut dependencies: Vec<V2MemberDependency<'_>> = joins
        .iter()
        .map(|action| V2MemberDependency { action, admission })
        .collect();
    dependencies.push(V2MemberDependency {
        action: &target,
        admission,
    });
    dependencies.reverse();
    let edit = f.send(
        [0x70; 32],
        &Body::Edit {
            create: target.id(),
            text: "x".repeat(MAX_TEXT_BYTES),
        },
        &parents,
        &dependencies,
    );
    assert_eq!(edit.spec().parents.len(), MAX_PARENTS);
    assert!(edit.spec().parents.windows(2).all(|pair| pair[0] < pair[1]));
    // A supplied duplicate (63 distinct + 1 repeat, 64 entries) dedups to 63 parents.
    let mut repeated: Vec<ActionId> = parents[..MAX_PARENTS - 1].to_vec();
    repeated.push(repeated[0]);
    let deduped: Vec<V2MemberDependency<'_>> = joins[1..]
        .iter()
        .map(|action| V2MemberDependency { action, admission })
        .chain(std::iter::once(V2MemberDependency {
            action: &target,
            admission,
        }))
        .collect();
    let smaller = f.send(
        [0x72; 32],
        &Body::Edit {
            create: target.id(),
            text: "deduplicated".into(),
        },
        &repeated,
        &deduped,
    );
    assert_eq!(smaller.spec().parents.len(), MAX_PARENTS - 1);
    // A full-size Create also signs.
    f.send(
        [0x71; 32],
        &create(&"y".repeat(MAX_TEXT_BYTES), None),
        &[],
        &[],
    );
    assert_eq!(f.head(), head);
}

#[test]
fn sixty_five_supplied_parents_refuse_before_any_fee_draw_or_key() {
    let mut f = promoted(0, 0x7fa15);
    let target = f.send([1; 32], &create("target", None), &[], &[]);
    let dispatches = member_sign_dispatches_for_test();
    // 64 distinct entries plus one repeat: 65 as supplied. Bounded BEFORE dedup.
    let mut parents: Vec<ActionId> = (0..MAX_PARENTS)
        .map(|index| ActionId::from_bytes([index as u8; 32]))
        .collect();
    parents.push(parents[0]);
    let body = Body::Join {
        create: target.id(),
    };
    let request = V2MemberSignRequest {
        token: [2; 32],
        channel: 7,
        body: &body,
        parents: &parents,
        dependencies: &[],
    };
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, request)
        .is_err());
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
    // No request was created and no token retired: a valid request authors next.
    f.send([2; 32], &create("after a refused shape", None), &[], &[]);
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
}

#[test]
fn wrong_dependencies_admission_and_changed_requests_refuse_without_a_key_call() {
    let mut f = promoted(0, 0x7fa20);
    let admission = f.admission;
    let target = f.send([1; 32], &create("target", None), &[], &[]);
    let dispatches = member_sign_dispatches_for_test();
    let edit = Body::Edit {
        create: target.id(),
        text: "needs its target".into(),
    };
    // Missing dependency, unknown Admission, and an unused duplicate all refuse before the key.
    let unknown = OwnerFactId::from_bytes([9; 32]);
    let cases: [([u8; 32], Vec<V2MemberDependency<'_>>); 3] = [
        ([2; 32], vec![]),
        (
            [3; 32],
            vec![V2MemberDependency {
                action: &target,
                admission: unknown,
            }],
        ),
        (
            [4; 32],
            vec![
                V2MemberDependency {
                    action: &target,
                    admission,
                },
                V2MemberDependency {
                    action: &target,
                    admission,
                },
            ],
        ),
    ];
    for (token, dependencies) in &cases {
        assert!(f
            .host
            .retry_sign_member_request(
                &f.layout,
                V2MemberSignRequest {
                    token: *token,
                    channel: 7,
                    body: &edit,
                    parents: &[target.id()],
                    dependencies,
                },
            )
            .is_err());
        assert_eq!(member_sign_dispatches_for_test(), dispatches);
        // No completed original exists, so the request can be abandoned (token retired).
        f.host.abandon_member_request(*token).unwrap();
    }
    // A Create whose parents are not exactly its reply target is refused before any work.
    let reply = create("reply", Some(target.id()));
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([5; 32], &reply))
        .is_err());
    // Same token with changed text/channel is a conflict; the original survives.
    let body = create("original request", None);
    f.host
        .retry_sign_member_request(&f.layout, plain([6; 32], &body))
        .unwrap();
    let signed = member_sign_dispatches_for_test();
    let changed = create("changed request", None);
    let mut other_channel = plain([6; 32], &body);
    other_channel.channel = 8;
    for request in [
        plain([6; 32], &changed),
        other_channel,
        // One pending request per lane: another token waits.
        plain([7; 32], &body),
    ] {
        assert!(f
            .host
            .retry_sign_member_request(&f.layout, request)
            .is_err());
    }
    assert_eq!(member_sign_dispatches_for_test(), signed);
    let taken = f.host.take_member_signed_original([6; 32]).unwrap();
    assert!(taken.action().spec().body == body);
}

#[test]
fn founder_lane_state_excludes_the_member_lane_before_any_key_call() {
    let mut f = promoted(0, 0x7fa30);
    // A founder token lookup is the first step of every founder Create.
    f.host.retry_token_lookup(&f.layout, [0x51; 32]).unwrap();
    let dispatches = member_sign_dispatches_for_test();
    let body = create("member after founder lookup", None);
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([0x51; 32], &body))
        .is_err());
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
}

#[test]
fn a_crash_left_founder_wal_keeps_the_member_lane_closed_until_it_is_gone() {
    let mut f = promoted(0, 0x7fa35);
    let dispatches = member_sign_dispatches_for_test();
    let body = create("member beside an uncertain founder transaction", None);
    for name in ["text.wal", "text.wal.stage.v1"] {
        // An uncertain founder transaction (for example a retained A) may still recover.
        std::fs::write(f.target.join(name), b"crash-left founder work").unwrap();
        assert!(f
            .host
            .retry_sign_member_request(&f.layout, plain([0x52; 32], &body))
            .is_err());
        assert_eq!(member_sign_dispatches_for_test(), dispatches);
        std::fs::remove_file(f.target.join(name)).unwrap();
    }
    // With the founder work gone, the same request now authors (nothing was drawn or
    // retired by the refused attempts).
    f.host
        .retry_sign_member_request(&f.layout, plain([0x52; 32], &body))
        .unwrap();
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
}

#[test]
fn a_refused_unbound_lane_is_not_kept_so_the_founder_lane_stays_available() {
    let mut f = promoted(0, 0x7fa36);
    let dispatches = member_sign_dispatches_for_test();
    let body = create("refused beside founder work", None);
    std::fs::write(f.target.join("text.wal"), b"crash-left founder work").unwrap();
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([0x53; 32], &body))
        .is_err());
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
    std::fs::remove_file(f.target.join("text.wal")).unwrap();
    // No lane was installed: the founder lane's first step is still available here, so the
    // host is not wedged with neither lane able to proceed.
    f.host.retry_token_lookup(&f.layout, [0x54; 32]).unwrap();
    // And from now on the founder state excludes the member lane.
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([0x53; 32], &body))
        .is_err());
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
}

#[test]
fn a_malformed_first_request_creates_no_lane() {
    let mut f = promoted(0, 0x7fa37);
    let body = Body::Join {
        create: ActionId::from_bytes([1; 32]),
    };
    let parents: Vec<ActionId> = (0..=MAX_PARENTS)
        .map(|index| ActionId::from_bytes([index as u8; 32]))
        .collect();
    assert!(f
        .host
        .retry_sign_member_request(
            &f.layout,
            V2MemberSignRequest {
                token: [0x55; 32],
                channel: 7,
                body: &body,
                parents: &parents,
                dependencies: &[],
            },
        )
        .is_err());
    // The founder lane was not excluded by a lane the refused request would have created.
    f.host.retry_token_lookup(&f.layout, [0x56; 32]).unwrap();
}

#[test]
fn a_changed_selected_head_under_an_existing_lane_refuses_before_the_key() {
    let mut f = promoted(0, 0x7fa38);
    f.send([1; 32], &create("lane established", None), &[], &[]);
    let dispatches = member_sign_dispatches_for_test();
    let original_head = f.head();
    // The selected O is no longer the exact frame the anchor and verifier were built on.
    let mut changed = original_head.clone();
    changed.push(0);
    std::fs::write(f.target.join("head.bin"), &changed).unwrap();
    let body = create("after the head moved", None);
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([2; 32], &body))
        .is_err());
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
    // Restoring the exact frame lets the SAME pending request finish with one key call.
    std::fs::write(f.target.join("head.bin"), &original_head).unwrap();
    f.host
        .retry_sign_member_request(&f.layout, plain([2; 32], &body))
        .unwrap();
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
}

#[test]
fn close_during_the_admitted_key_operation_waits_for_its_reader_and_keeps_the_original() {
    let mut f = promoted(0, 0x7fa40);
    let dispatches = member_sign_dispatches_for_test();
    let body = create("close races an admitted signer", None);
    arm_member_close_during_sign_for_test();
    f.host
        .retry_sign_member_request(&f.layout, plain([0x61; 32], &body))
        .unwrap();
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
    // The close was requested while Core's original reader was held: it had NOT drained.
    assert_eq!(member_close_drained_during_sign_for_test(), Some(false));
    // The already admitted operation still produced its exact original.
    let signed = f.host.take_member_signed_original([0x61; 32]).unwrap();
    assert!(signed.action().verify());
    assert!(f
        .host
        .finish_member_request_saved([0x61; 32], signed.action().id())
        .unwrap());
    // No fresh signing after the close; the real drain now completes.
    let next = create("after close", None);
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([0x62; 32], &next))
        .is_err());
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
    f.host.cancel_and_drain_admitted_promotion().unwrap();
}

#[test]
fn a_late_source_negative_after_the_cached_raw_still_completes_without_resigning() {
    for family in 0..3 {
        let mut f = promoted(family, 0x7fa50 + family as u64);
        let dispatches = member_sign_dispatches_for_test();
        let body = create("late negative after raw", None);
        // The whole original owner is revoked right after the raw is cached, so the
        // Current callback's final source check fails AFTER the key operation.
        arm_owner_cancel_during_sign_for_test();
        assert!(f
            .host
            .retry_sign_member_request(&f.layout, plain([0x71; 32], &body))
            .is_err());
        assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
        // Exact retry after the revocation: pure DATA completion, no key and no draw.
        f.host
            .retry_sign_member_request(&f.layout, plain([0x71; 32], &body))
            .unwrap();
        assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
        let signed = f.host.take_member_signed_original([0x71; 32]).unwrap();
        assert!(signed.action().verify());
        assert!(signed.admission() == f.admission);
        // Nothing new can be authored after the revocation.
        assert!(f
            .host
            .finish_member_request_saved([0x71; 32], signed.action().id())
            .unwrap());
        let next = create("after the revocation", None);
        assert!(f
            .host
            .retry_sign_member_request(&f.layout, plain([0x72; 32], &next))
            .is_err());
        assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
    }
}

#[test]
fn restore_accepts_only_the_exact_original_it_lent() {
    let mut f = promoted(0, 0x7fa60);
    let first_body = create("first", None);
    f.host
        .retry_sign_member_request(&f.layout, plain([0x81; 32], &first_body))
        .unwrap();
    let first = f.host.take_member_signed_original([0x81; 32]).unwrap();
    let first_bytes = first.action().encode();
    let first_id = first.action().id();
    // Wrong token: the value comes back whole.
    let (first, _) = f
        .host
        .restore_member_signed_original([0x99; 32], first)
        .unwrap_err();
    // Right token: restored, and the next take yields the same exact original.
    f.host
        .restore_member_signed_original([0x81; 32], first)
        .unwrap();
    let first = f.host.take_member_signed_original([0x81; 32]).unwrap();
    assert_eq!(first.action().encode(), first_bytes);
    assert!(f
        .host
        .finish_member_request_saved([0x81; 32], first_id)
        .unwrap());
    let second_body = create("second", None);
    f.host
        .retry_sign_member_request(&f.layout, plain([0x82; 32], &second_body))
        .unwrap();
    let second = f.host.take_member_signed_original([0x82; 32]).unwrap();
    // A different (earlier) original cannot be cached under the second request.
    let (first, _) = f
        .host
        .restore_member_signed_original([0x82; 32], first)
        .unwrap_err();
    assert_eq!(first.action().encode(), first_bytes);
    f.host
        .restore_member_signed_original([0x82; 32], second)
        .unwrap();
    // A completed original the outbox permanently refused can be abandoned: it never left
    // this process, so nothing is published, and the lane is not wedged.
    f.host.abandon_member_request([0x82; 32]).unwrap();
    assert!(f.host.take_member_signed_original([0x82; 32]).is_err());
    f.send([0x83; 32], &create("after abandonment", None), &[], &[]);
}

#[test]
fn a_lost_lent_original_is_forgotten_only_explicitly_and_the_lane_continues() {
    let mut f = promoted(0, 0x7fa70);
    let body = create("lent then lost", None);
    f.host
        .retry_sign_member_request(&f.layout, plain([0x91; 32], &body))
        .unwrap();
    // The outbox preparation holding the lent value is dropped (or unwinds).
    drop(f.host.take_member_signed_original([0x91; 32]).unwrap());
    // Lent: neither abandonment nor another token can proceed.
    assert!(f.host.abandon_member_request([0x91; 32]).is_err());
    let next = create("next request", None);
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([0x92; 32], &next))
        .is_err());
    // After the outbox lookup proved Absent, the caller forgets it explicitly.
    f.host.forget_lent_member_request([0x91; 32]).unwrap();
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([0x91; 32], &body))
        .is_err());
    f.send([0x92; 32], &next, &[], &[]);
}

#[test]
fn already_saved_naming_an_earlier_original_is_reported_and_retires_the_token() {
    let mut f = promoted(0, 0x7fa80);
    let earlier = f.send([0xa1; 32], &create("an earlier original", None), &[], &[]);
    let body = create("signed again before a restart", None);
    f.host
        .retry_sign_member_request(&f.layout, plain([0xa2; 32], &body))
        .unwrap();
    let lent = f.host.take_member_signed_original([0xa2; 32]).unwrap();
    assert!(lent.action().id() != earlier.id());
    // AlreadySaved's receipt names a different ActionId: the lent copy must be dropped by
    // the caller and never published; the host reports the difference and retires the token.
    assert!(!f
        .host
        .finish_member_request_saved([0xa2; 32], earlier.id())
        .unwrap());
    drop(lent);
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain([0xa2; 32], &body))
        .is_err());
}
