//! Group lifetime races use real signed action fixtures and deterministic channel barriers.
//! They do not supply the still-absent production root/governing issuer or native/UI adapter.
use super::*;

fn ready_channel(
    f: &Fixture,
    group: &ProjectionGroupView,
) -> (ChatLedger, ProjectionView, SignedAction) {
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view_in(group).unwrap();
    let create = f.create(0, "original channel body");
    f.retain(&mut ledger, &create);
    ledger
        .admit_projection(&f.token(&create, &view, None), 1)
        .unwrap();
    (ledger, view, create)
}

#[test]
fn group_close_before_publication_cancels_two_channels_and_original_captured_results() {
    let a = Fixture::new();
    let mut b = Fixture::new();
    b.realm = Realm::new(a.realm.group(), a.realm.channel() + 1).unwrap();
    let mut group = ProjectionGroup::new(a.realm.group()).unwrap();
    let view = group.begin_view();
    let (mut left, left_view, left_create) = ready_channel(&a, &view);
    let (right, _right_view, right_create) = ready_channel(&b, &view);
    let left_reply = left.guarded_projection(left_create.id(), 1).unwrap();
    let right_reply = right.guarded_projection(right_create.id(), 1).unwrap();
    let right_heads = right
        .projectable_causal_heads(right_create.id(), 1)
        .unwrap();
    let revoker = view.revoker(); // captured before the simulated actor/body wait
    let mut closed = revoker.close();
    assert!(closed.try_complete());
    assert_eq!(left_reply.with_publication(|_| ()), Err(ChatError::Scope));
    assert_eq!(right_reply.with_publication(|_| ()), Err(ChatError::Scope));
    assert_eq!(right_heads.with_publication(|_| ()), Err(ChatError::Scope));
    assert!(left
        .admit_projection(&a.token(&left_create, &left_view, None), 1)
        .is_err());
    assert!(matches!(
        result(&left, left_create.id()),
        GuardedProjection::Held { .. }
    ));
    assert!(matches!(
        result(&right, right_create.id()),
        GuardedProjection::Held { .. }
    ));
    assert_eq!(
        left.projection(left_create.id(), 1)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("original channel body"),
        "group closure preserves retained audit history"
    );
}

#[test]
fn group_close_and_nested_replacements_wait_for_original_admitted_channel_reply() {
    let f = Fixture::new();
    let mut group = ProjectionGroup::new(f.realm.group()).unwrap();
    let first = group.begin_view();
    let (mut ledger, _channel, create) = ready_channel(&f, &first);
    let reply = ledger.guarded_projection(create.id(), 1).unwrap();
    let (reached, release, done, worker) = hold_publication(reply);
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut closed = first.revoker().close();
    assert!(!closed.try_complete());
    let mut middle = group.begin_view();
    assert!(!middle.try_ready());
    let mut last = group.begin_view();
    assert!(
        !last.try_ready(),
        "a pending intermediate view cannot hide first-view readers"
    );
    assert!(matches!(
        ledger.begin_projection_view_in(&last),
        Err(ChatError::Scope)
    ));
    drop(release);
    assert!(done.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap();
    assert!(closed.try_complete());
    assert!(
        !middle.try_ready(),
        "superseded original owner stays cancelled"
    );
    assert!(last.try_ready());
    let mut channel = ledger.begin_projection_view_in(&last).unwrap();
    assert!(channel.try_ready());
    ledger
        .admit_projection(&f.token(&create, &channel, None), 1)
        .unwrap();
    visible(
        &ledger,
        create.id(),
        "original channel body",
        ProjectionCertainty::Proven,
    );
}

#[test]
fn group_close_waits_for_actual_capsule_installation_already_admitted_under_original_gates() {
    let f = Fixture::new();
    let mut group = ProjectionGroup::new(f.realm.group()).unwrap();
    let view = group.begin_view();
    let mut ledger = f.ledger();
    let channel = ledger.begin_projection_view_in(&view).unwrap();
    let create = f.create(0, "body");
    let delete = f.action(
        1,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    f.retain(&mut ledger, &create);
    f.retain(&mut ledger, &delete);
    ledger
        .admit_projection(&f.token(&delete, &channel, None), 1)
        .unwrap();
    let token = f.token(&create, &channel, None);
    let (reached_tx, reached_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *channel.life.mutation_hook.lock().unwrap() = Some(Box::new(move || {
        reached_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    }));
    let release = ReleaseOnDrop(Some(release_tx));
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        ledger.admit_projection(&token, 2).unwrap();
        done_tx.send(ledger).unwrap();
    });
    reached_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut closed = view.revoker().close();
    assert!(
        !closed.try_complete(),
        "close cannot ACK ahead of admitted positive mutation"
    );
    drop(release);
    let ledger = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    assert!(ledger.entries[&delete.id()].prior_delete.is_some());
    assert!(closed.try_complete());
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Held { .. }
    ));
    // The completed mutation's Delete custody still suppresses after replacing the closed
    // group/view. Group closure does not retroactively discard the original capsule.
    let mut next = group.begin_view();
    assert!(next.try_ready());
    let mut ledger = ledger;
    let mut next_channel = ledger.begin_projection_view_in(&next).unwrap();
    assert!(next_channel.try_ready());
    ledger
        .admit_projection(&f.token(&create, &next_channel, None), 2)
        .unwrap();
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::IncompleteSuppressed { .. }
    ));
}

#[test]
fn group_close_orders_both_before_and_after_exact_negative_capsule_resolution() {
    let f = Fixture::new();
    for current_delete in [false, true] {
        for close_first in [true, false] {
            let mut group = ProjectionGroup::new(f.realm.group()).unwrap();
            let first = group.begin_view();
            let (mut ledger, old_owner, create) = ready_channel(&f, &first);
            let delete = f.action(
                1,
                Body::Delete {
                    create: create.id(),
                },
                vec![create.id()],
            );
            f.retain(&mut ledger, &delete);
            ledger
                .admit_projection(&f.token(&delete, &old_owner, None), 1)
                .unwrap();
            let mut next = group.begin_view();
            assert!(next.try_ready());
            let mut next_channel = ledger.begin_projection_view_in(&next).unwrap();
            assert!(next_channel.try_ready());
            ledger
                .admit_projection(&f.token(&create, &next_channel, None), 2)
                .unwrap();
            if current_delete {
                ledger
                    .admit_projection(&f.token(&delete, &next_channel, None), 1)
                    .unwrap();
            }
            let prior = ledger.entries[&delete.id()].prior_delete.clone().unwrap();
            let current_source = ledger.entries[&delete.id()]
                .projection
                .as_ref()
                .unwrap()
                .source
                .clone();
            let resolution = f.resolution(&ledger, delete.id(), &next_channel);
            let revision = next_channel.life.revision.load(Ordering::Acquire);
            if close_first {
                let mut closed = next.revoker().close();
                assert!(closed.try_complete());
                assert_eq!(
                    ledger.resolve_projection_delete(&resolution),
                    Err(ChatError::Scope)
                );
                assert!(ledger.entries[&delete.id()]
                    .prior_delete
                    .as_ref()
                    .unwrap()
                    .same(&prior));
                assert_eq!(next_channel.life.revision.load(Ordering::Acquire), revision);
            } else {
                let (reached_tx, reached_rx) = std::sync::mpsc::channel();
                let (release_tx, release_rx) = std::sync::mpsc::channel();
                *next_channel.life.mutation_hook.lock().unwrap() = Some(Box::new(move || {
                    reached_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                }));
                let release = ReleaseOnDrop(Some(release_tx));
                let (done_tx, done_rx) = std::sync::mpsc::channel();
                let worker = std::thread::spawn(move || {
                    ledger.resolve_projection_delete(&resolution).unwrap();
                    done_tx.send(ledger).unwrap();
                });
                reached_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                let mut closed = next.revoker().close();
                assert!(
                    !closed.try_complete(),
                    "exact admitted resolution holds original group custody through mutation"
                );
                drop(release);
                let ledger = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                worker.join().unwrap();
                let remaining = ledger.entries[&delete.id()].prior_delete.as_ref();
                assert_eq!(remaining.is_some(), current_delete);
                if let Some(capsule) = remaining {
                    assert!(Arc::ptr_eq(&capsule.source, &current_source), "precomputed current Proven support must re-mint even if cancellation follows admission");
                }
                assert_eq!(
                    next_channel.life.revision.load(Ordering::Acquire),
                    revision + 1
                );
                assert!(closed.try_complete());
            }
        }
    }
}

#[test]
fn foreign_or_closed_group_refusal_preserves_existing_channel_and_publication_revision() {
    let f = Fixture::new();
    let mut group = ProjectionGroup::new(f.realm.group()).unwrap();
    let view = group.begin_view();
    let (mut ledger, channel, create) = ready_channel(&f, &view);
    let reply = ledger.guarded_projection(create.id(), 1).unwrap();
    let revision = channel.life.revision.load(Ordering::Acquire);
    let mut foreign = ProjectionGroup::new(b"another exact group").unwrap();
    let foreign_view = foreign.begin_view();
    assert!(matches!(
        ledger.begin_projection_view_in(&foreign_view),
        Err(ChatError::Scope)
    ));
    let mut closed = foreign_view.revoker().close();
    assert!(closed.try_complete());
    assert!(matches!(
        ledger.begin_projection_view_in(&foreign_view),
        Err(ChatError::Scope)
    ));
    assert_eq!(channel.life.revision.load(Ordering::Acquire), revision);
    assert!(reply.with_publication(|_| ()).is_ok());
    visible(
        &ledger,
        create.id(),
        "original channel body",
        ProjectionCertainty::Proven,
    );
}

#[test]
fn equal_group_labels_on_a_new_domain_cannot_renew_original_channel_tokens_or_results() {
    let f = Fixture::new();
    let mut first = ProjectionGroup::new(f.realm.group()).unwrap();
    let first_view = first.begin_view();
    let (mut ledger, channel, create) = ready_channel(&f, &first_view);
    let token = f.token(&create, &channel, None);
    let reply = ledger.guarded_projection(create.id(), 1).unwrap();
    drop(first); // mounted source lifetime retires, even if view/revoker handles survive
    let mut second = ProjectionGroup::new(f.realm.group()).unwrap();
    let second_view = second.begin_view();
    let mut replacement = ledger.begin_projection_view_in(&second_view).unwrap();
    assert!(replacement.try_ready());
    assert!(ledger.admit_projection(&token, 1).is_err());
    assert_eq!(reply.with_publication(|_| ()), Err(ChatError::Scope));
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Held { .. }
    ));
    ledger
        .admit_projection(&f.token(&create, &replacement, None), 1)
        .unwrap();
    visible(
        &ledger,
        create.id(),
        "original channel body",
        ProjectionCertainty::Proven,
    );
}

#[test]
fn group_callback_can_close_and_drop_its_own_original_owner_without_waiting_or_false_ack() {
    let f = Fixture::new();
    let mut group = ProjectionGroup::new(f.realm.group()).unwrap();
    let view = group.begin_view();
    let (ledger, _channel, create) = ready_channel(&f, &view);
    let reply = ledger.guarded_projection(create.id(), 1).unwrap();
    let mut completion = reply
        .with_publication(move |_| {
            let mut completion = view.close();
            assert!(
                !completion.try_complete(),
                "own admitted reader has not drained"
            );
            drop(group); // neither Drop can wait for its own reader
            completion
        })
        .unwrap();
    assert!(completion.try_complete());
    assert_eq!(reply.with_publication(|_| ()), Err(ChatError::Scope));
}

#[test]
fn group_owner_drop_during_callback_unwind_cancels_every_channel_without_deadlock() {
    let f = Fixture::new();
    let mut group = ProjectionGroup::new(f.realm.group()).unwrap();
    let view = group.begin_view();
    let (ledger, _channel, create) = ready_channel(&f, &view);
    let revoker = view.revoker();
    let reply = ledger.guarded_projection(create.id(), 1).unwrap();
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = reply.with_publication(move |_| {
            let _owners = (view, group);
            panic!("unwind a callback that owns its original group");
        });
    }));
    assert!(unwind.is_err());
    let mut completion = revoker.close();
    assert!(completion.try_complete());
    assert_eq!(reply.with_publication(|_| ()), Err(ChatError::Scope));
}

#[test]
fn closing_one_group_does_not_cancel_or_wait_for_an_unrelated_group() {
    let a = Fixture::new();
    let mut b = Fixture::new();
    b.realm = Realm::new(b"unrelated group", 1).unwrap();
    let mut group_a = ProjectionGroup::new(a.realm.group()).unwrap();
    let view_a = group_a.begin_view();
    let (left, _left_channel, left_create) = ready_channel(&a, &view_a);
    let mut group_b = ProjectionGroup::new(b.realm.group()).unwrap();
    let view_b = group_b.begin_view();
    let (right, _right_channel, right_create) = ready_channel(&b, &view_b);
    let (reached, release, done, worker) =
        hold_publication(right.guarded_projection(right_create.id(), 1).unwrap());
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut closed = view_a.close();
    assert!(closed.try_complete());
    assert!(matches!(
        result(&left, left_create.id()),
        GuardedProjection::Held { .. }
    ));
    drop(release);
    assert!(done.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap();
    visible(
        &right,
        right_create.id(),
        "original channel body",
        ProjectionCertainty::Proven,
    );
}
