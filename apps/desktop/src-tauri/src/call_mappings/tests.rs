use super::*;

#[test]
fn call_transport_advice_keeps_mesh_evidence_separate_from_voice_success() {
    for public in [true, false] {
        for router in [true, false] {
            for helpers in [true, false] {
                let advice = crate::call_transport_advice(public, router, helpers);
                assert!(advice.contains("configure TURN"));
                assert_eq!(advice.contains("advertises a public mesh address"), public);
                assert_eq!(
                    advice.contains("can also try a router-mapped UDP port"),
                    router
                );
                assert_eq!(advice.contains("helpers assist with joining"), helpers);
                for unsupported in [
                    "relayed path is available",
                    "one mapped side is enough",
                    "should connect without a relay",
                    "stricter NAT",
                ] {
                    assert!(!advice.contains(unsupported));
                }
            }
        }
    }
}

fn binding() -> Binding {
    Binding {
        generation: 1,
        server: 3,
        instance: 7,
        group: vec![4; 16],
        room: "9".into(),
    }
}
fn controller(c: &mut Coordinator<&'static str>) -> String {
    let id = c
        .prepare(CandidateKind::Controller { generation: 1 }, 0)
        .unwrap();
    assert!(c.activate(&id, 0).unwrap().is_empty());
    id
}
fn owner(c: &mut Coordinator<&'static str>, controller: &str) -> String {
    let id = c
        .prepare(
            CandidateKind::Call {
                controller: controller.into(),
                binding: binding(),
            },
            0,
        )
        .unwrap();
    for cleanup in c.activate(&id, 0).unwrap() {
        c.finish_cleanup(&cleanup.id, true);
    }
    id
}

#[test]
fn delayed_prepare_activation_and_end_cannot_revoke_a_newer_call() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let old = c
        .prepare(
            CandidateKind::Call {
                controller: controller.clone(),
                binding: binding(),
            },
            0,
        )
        .unwrap();
    let current = owner(&mut c, &controller);
    assert!(c.activate(&old, 0).is_err());
    assert!(c.end(&old).is_empty());
    assert_eq!(c.binding(&current), Some(&binding()));
}

#[test]
fn call_end_keeps_pending_port_charged_and_cleans_its_late_grant() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let old = owner(&mut c, &controller);
    let attempt = c.reserve(&old, 100).unwrap();
    assert!(c.end(&old).is_empty());
    let current = owner(&mut c, &controller);
    assert!(c.reserve(&current, 100).is_err());
    let Completion::Cleanup(cleanup) = c.complete(&attempt, Ok("late-router-grant"), true) else {
        panic!("old grant was installed")
    };
    assert_eq!(cleanup.grant, "late-router-grant");
    assert!(c.reserve(&current, 100).is_err());
    c.finish_cleanup(&cleanup.id, true);
    assert!(c.reserve(&current, 100).is_ok());
}

#[test]
fn lock_and_remount_preserve_uncertain_capacity_and_same_port_exclusion() {
    let mut c = Coordinator::default();
    let old_controller = controller(&mut c);
    let old = owner(&mut c, &old_controller);
    let attempt = c.reserve(&old, 100).unwrap();
    c.revoke_all();
    assert!(matches!(
        c.complete(&attempt, Err(true), false),
        Completion::Failed
    ));
    let new_controller = controller(&mut c);
    let new = owner(&mut c, &new_controller);
    assert!(c.reserve(&new, 100).unwrap_err().contains("uncertain"));
    c.end(&old);
    c.end(&old_controller);
    c.revoke_all();
    assert_eq!(c.ports.len(), 1);
    assert!(matches!(c.ports[&100].state, PortState::Uncertain));
}

#[test]
fn old_grant_release_cannot_remove_a_replacement_at_the_same_port() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let old = owner(&mut c, &controller);
    let first = c.reserve(&old, 100).unwrap();
    assert!(matches!(
        c.complete(&first, Ok("first"), true),
        Completion::Installed
    ));
    let cleanup = c.unmap(&old, &first).unwrap();
    c.finish_cleanup(&cleanup.id, true);
    let new = owner(&mut c, &controller);
    let second = c.reserve(&new, 100).unwrap();
    assert!(matches!(
        c.complete(&second, Ok("second"), true),
        Completion::Installed
    ));
    assert!(c.unmap(&old, &first).is_none());
    c.finish_cleanup(&first, true);
    assert_eq!(c.ports[&100].id, second);
    assert!(matches!(c.ports[&100].state, PortState::Granted("second")));
}

#[test]
fn clean_failures_refund_but_uncertain_failures_exhaust_process_capacity() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let owner = owner(&mut c, &controller);
    let clean = c.reserve(&owner, 1).unwrap();
    c.complete(&clean, Err(false), true);
    assert!(c.ports.is_empty());
    for port in 1..=MAX_PORT_SLOTS as u16 {
        let id = c.reserve(&owner, port).unwrap();
        c.complete(&id, Err(true), true);
    }
    assert!(c.reserve(&owner, 100).unwrap_err().contains("capacity"));
    c.revoke_all();
    let controller = self::controller(&mut c);
    let owner = self::owner(&mut c, &controller);
    assert!(c.reserve(&owner, 100).unwrap_err().contains("capacity"));
    assert_eq!(c.ports.len(), MAX_PORT_SLOTS);
}

#[test]
fn retired_cleanup_is_idempotent_and_stays_charged_on_failure() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let owner = owner(&mut c, &controller);
    let id = c.reserve(&owner, 100).unwrap();
    c.complete(&id, Ok("grant"), true);
    assert_eq!(c.end(&owner).len(), 1);
    assert!(c.end(&owner).is_empty());
    c.finish_cleanup(&id, false);
    c.finish_cleanup(&id, true);
    assert!(matches!(c.ports[&100].state, PortState::Uncertain));
}

#[test]
fn controller_replacement_revokes_old_candidates_and_call_without_port_refund() {
    let mut c = Coordinator::default();
    let old_controller = controller(&mut c);
    let old = owner(&mut c, &old_controller);
    let id = c.reserve(&old, 100).unwrap();
    let candidate = c
        .prepare(
            CandidateKind::Call {
                controller: old_controller.clone(),
                binding: binding(),
            },
            0,
        )
        .unwrap();
    let current_controller = controller(&mut c);
    assert!(c.activate(&candidate, 0).is_err());
    assert!(c.binding(&old).is_none());
    assert!(c
        .prepare(
            CandidateKind::Call {
                controller: old_controller,
                binding: binding()
            },
            0
        )
        .is_err());
    assert!(matches!(
        c.complete(&id, Ok("old-grant"), true),
        Completion::Cleanup(_)
    ));
    assert!(c
        .controller
        .as_ref()
        .is_some_and(|(id, _)| id == &current_controller));
}

#[test]
fn server_replacement_and_session_revalidation_reject_late_installs() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let owner = owner(&mut c, &controller);
    let id = c.reserve(&owner, 100).unwrap();
    assert!(c.retire_server(3, 8).is_empty());
    assert!(c.binding(&owner).is_some());
    c.retire_server(3, 7);
    assert!(c.binding(&owner).is_none());
    assert!(matches!(
        c.complete(&id, Ok("old-grant"), false),
        Completion::Cleanup(_)
    ));
}

#[test]
fn candidate_capacity_expiry_and_wrong_controller_fail_closed() {
    let mut c: Coordinator<&str> = Coordinator::default();
    assert!(c
        .prepare(
            CandidateKind::Call {
                controller: "fake".into(),
                binding: binding()
            },
            0
        )
        .is_err());
    let mut ids = Vec::new();
    for _ in 0..MAX_CANDIDATES {
        ids.push(
            c.prepare(CandidateKind::Controller { generation: 1 }, 0)
                .unwrap(),
        );
    }
    assert!(c
        .prepare(CandidateKind::Controller { generation: 1 }, 0)
        .is_err());
    assert!(c.activate(&ids[0], CANDIDATE_TTL_MS).is_err());
    let fresh = c
        .prepare(
            CandidateKind::Controller { generation: 1 },
            CANDIDATE_TTL_MS,
        )
        .unwrap();
    assert!(c.activate(&fresh, CANDIDATE_TTL_MS).is_ok());
}

#[tokio::test]
async fn dropped_command_waiter_does_not_drop_acquisition_or_refund_cleanup_capacity() {
    for reusable in [true, false] {
        let mut state = Coordinator::default();
        let controller = controller(&mut state);
        let owner = owner(&mut state, &controller);
        let attempt = state.reserve(&owner, 100).unwrap();
        let coordinator = Arc::new(Mutex::new(state));
        let (grant_tx, grant_rx) = tokio::sync::oneshot::channel();
        let (cleanup_started_tx, cleanup_started_rx) = tokio::sync::oneshot::channel();
        let (cleanup_tx, cleanup_rx) = tokio::sync::oneshot::channel();
        let worker = spawn_acquisition(
            coordinator.clone(),
            attempt.clone(),
            async move { grant_rx.await.unwrap() },
            || async { None::<()> },
            |grant, _| *grant,
            move |grant| async move {
                assert_eq!(grant, "late grant");
                cleanup_started_tx.send(()).unwrap();
                cleanup_rx.await.unwrap()
            },
        );
        let completion = worker.abort_handle();
        // This is the actual production ownership boundary: cancelling the command's wait
        // drops its JoinHandle while the native router worker remains independently owned.
        let waiter = tokio::spawn(worker);
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        coordinator.lock().unwrap().revoke_all();
        grant_tx.send(Ok("late grant")).unwrap();
        cleanup_started_rx.await.unwrap();
        {
            let mut state = coordinator.lock().unwrap();
            assert!(matches!(state.ports[&100].state, PortState::Cleaning));
            let controller = self::controller(&mut state);
            let owner = self::owner(&mut state, &controller);
            assert!(state.reserve(&owner, 100).is_err());
        }
        cleanup_tx.send(reusable).unwrap();
        for _ in 0..128 {
            if completion.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(completion.is_finished());
        let state = coordinator.lock().unwrap();
        if reusable {
            assert!(!state.ports.contains_key(&100));
        } else {
            assert!(matches!(state.ports[&100].state, PortState::Uncertain));
        }
    }
}

#[tokio::test]
async fn lock_between_router_completion_and_authorization_prevents_installation() {
    let mut state = Coordinator::default();
    let controller = controller(&mut state);
    let owner = owner(&mut state, &controller);
    let attempt = state.reserve(&owner, 100).unwrap();
    let coordinator = Arc::new(Mutex::new(state));
    let (authorizing_tx, authorizing_rx) = tokio::sync::oneshot::channel();
    let (authorization_tx, authorization_rx) = tokio::sync::oneshot::channel::<Option<()>>();
    let worker = spawn_acquisition(
        coordinator.clone(),
        attempt,
        async { Ok("router grant") },
        move || async move {
            authorizing_tx.send(()).unwrap();
            authorization_rx.await.unwrap()
        },
        |_, _| "must not return",
        |grant| async move {
            assert_eq!(grant, "router grant");
            false
        },
    );
    authorizing_rx.await.unwrap();
    coordinator.lock().unwrap().revoke_all();
    authorization_tx.send(None).unwrap();
    assert!(worker.await.unwrap().is_err());
    assert!(matches!(
        coordinator.lock().unwrap().ports[&100].state,
        PortState::Uncertain
    ));
}

#[tokio::test]
async fn authorization_guard_is_retained_through_current_grant_installation() {
    struct Guard(Arc<Mutex<Coordinator<&'static str>>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            assert!(matches!(
                self.0.lock().unwrap().ports[&100].state,
                PortState::Granted("current grant")
            ));
        }
    }
    let mut state = Coordinator::default();
    let controller = controller(&mut state);
    let owner = owner(&mut state, &controller);
    let attempt = state.reserve(&owner, 100).unwrap();
    let coordinator = Arc::new(Mutex::new(state));
    let authorized = coordinator.clone();
    let worker = spawn_acquisition(
        coordinator,
        attempt,
        async { Ok("current grant") },
        move || async move { Some(Guard(authorized)) },
        |grant, id| (id.to_owned(), *grant),
        |_| async { panic!("current grant must not be cleaned") },
    );
    assert_eq!(worker.await.unwrap().unwrap().1, "current grant");
}
