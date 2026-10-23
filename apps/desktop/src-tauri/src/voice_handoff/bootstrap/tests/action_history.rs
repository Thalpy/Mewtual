//! Real registry/codec and protected action factories, with the explicitly synthetic desktop
//! lifecycle grant from ledger tests. These tests model prior authenticated browser Phase
//! closure; they do not establish browser drainage or successful ServerActor/MLS handoff.
use super::*;
use crate::voice_handoff::custody::Phase;
use crate::voice_handoff::ledger::tests::{actual_action, selection, selection_with_runtime};
use crate::voice_handoff::ledger::{Begin as ActionBegin, Ledger, Purpose, RowId};
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::VoiceError;

#[test]
fn collected_child_history_pins_its_original_parent_until_actual_last_handle_collection() {
    use crate::voice_handoff::{custody::CloseMode, ledger::ConfirmationPurpose};
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let (output, route) = installed(&registry, selected.clone());
    let ActionBegin::Factory(registration) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("original parent factory")
    };
    let (join, action, pair) = actual_action(&coordinator, &selected);
    let parent_id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let parent = ledger
        .resolve(&selected, Purpose::Derive(selector), parent_id)
        .unwrap();
    action.settle_engine();
    parent
        .close_phase(route.session().identity(), CloseMode::Return)
        .unwrap();
    assert!(parent.try_compact().unwrap());
    // No provider ran: the child is deliberately uncertain local history, not a claim of
    // successful protocol confirmation. Its real strong parent edge is what this test pins.
    let request = ConfirmationPurpose::Start(None);
    let purpose = Purpose::Confirmation {
        parent: parent_id,
        request,
    };
    let (ActionBegin::Factory(registration), _) = ledger
        .begin_child(selected.clone(), &parent, request)
        .unwrap()
    else {
        panic!("original child factory")
    };
    let child_id = registration.id();
    registration.mark_handoff().unwrap();
    drop(registration);
    let child = ledger.resolve(&selected, purpose, child_id).unwrap();
    child.close_pending().unwrap();
    coordinator.end(selected.id());
    drop((join, action));
    assert!(parent.retire_plan().unwrap());
    retire(&registry, &selected);
    drop((output, route, parent));
    registry.collect_retired(&ledger); // Actual registry proof; no manufactured GC flags.
    assert!(ledger.lookup_original(&selected, purpose).is_some());
    assert!(
        ledger
            .lookup_original(&selected, Purpose::Derive(selector))
            .is_some(),
        "a held child must retain its same original parent after physical channel collection"
    );
    drop(child);
    registry.collect_retired(&ledger);
    assert!(ledger.lookup_original(&selected, purpose).is_none());
    // Child destruction occurs outside Index; its parent edge was still real during that
    // pass. A subsequent bounded pass can now collect the original parent without a cycle.
    registry.collect_retired(&ledger);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    assert!(ledger.resolve(&selected, purpose, child_id).is_err());
}

#[tokio::test]
async fn unacknowledged_final_gc_requires_matching_physical_identity_and_late_metadata_drainage() {
    use crate::voice_handoff::custody::CloseMode;
    for foreign_before_intent in [false, true] {
        let (mut coordinator, selected, selector) = selection();
        let ledger = Arc::new(Ledger::default());
        let original_registry = Registry::default();
        let foreign_registry = Registry::default();
        let (output, route) = installed(&original_registry, selected.clone());
        let (foreign_output, foreign_route) = installed(&foreign_registry, selected.clone());
        // The test has completed both channel constructors before requesting the protected
        // action. Release their original setup-work credits; retain both Control lanes.
        route.acknowledge_setup().unwrap();
        foreign_route.acknowledge_setup().unwrap();
        assert_eq!(
            route.hash(),
            foreign_route.hash(),
            "equal transcript metadata fixture"
        );
        let identity = route.session().identity().clone();
        assert!(!Arc::ptr_eq(&identity, foreign_route.session().identity()));
        let ActionBegin::Factory(registration) = ledger
            .begin(selected.clone(), Purpose::Derive(selector))
            .unwrap()
        else {
            panic!("new original")
        };
        let (join, action, pair) = actual_action(&coordinator, &selected);
        let input = action.input().unwrap();
        let id = registration
            .attach(action.clone(), pair, 16 * 1024)
            .unwrap();
        let row = ledger
            .resolve(&selected, Purpose::Derive(selector), id)
            .unwrap();
        action.settle_engine();
        if !foreign_before_intent {
            row.close_phase(&identity, CloseMode::Retire).unwrap();
            assert!(row.try_compact().unwrap());
        }
        coordinator.end(selected.id());
        // Forced shutdown/physical collection is independent of normal ACK-gated close.
        // Collect an actually different Session with identical public labels and hash.
        retire(&foreign_registry, &selected);
        drop((foreign_output, foreign_route));
        foreign_registry.collect_retired(&ledger);
        if foreign_before_intent {
            row.close_phase(&identity, CloseMode::Retire).unwrap();
            assert!(row.try_compact().unwrap());
        }
        assert!(
            row.issue_final(&identity).unwrap().proof.is_none(),
            "real late Root input remains"
        );
        drop((input, action, join));
        let issued = row.issue_final(&identity).unwrap();
        assert!(issued.proof.is_some() && !issued.acknowledged);
        drop(row);
        foreign_registry.collect_retired(&ledger);
        assert!(
            ledger
                .lookup_original(&selected, Purpose::Derive(selector))
                .is_some(),
            "foreign physical token cannot collect this row"
        );
        let late = ledger
            .resolve(&selected, Purpose::Derive(selector), id)
            .unwrap();
        retire(&original_registry, &selected);
        drop((output, route));
        original_registry.collect_retired(&ledger);
        assert_eq!(
            late.issue_final(&identity).unwrap(),
            issued,
            "physical collection must not invent final ACK"
        );
        assert!(
            ledger
                .lookup_original(&selected, Purpose::Derive(selector))
                .is_some(),
            "late metadata still owns original payment"
        );
        drop(late);
        Registry::default().collect_retired(&ledger);
        assert!(
            ledger
                .lookup_original(&selected, Purpose::Derive(selector))
                .is_none(),
            "empty sweep collects with actual original proof, without ACK"
        );
    }
}

#[tokio::test]
async fn actual_channel_collection_keeps_final_proof_until_late_root_input_retires() {
    use crate::voice_handoff::custody::ConfirmationStatus;
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let (output, route) = installed(&registry, selected.clone());
    let ActionBegin::Factory(registration) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("new original action")
    };
    let (join, action, pair) = actual_action(&coordinator, &selected);
    let root_input = action.input().unwrap();
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let work = row.work().unwrap();
    let phase = work.phase().clone();
    let original_phase = Arc::downgrade(&phase);
    action.settle_engine();
    phase.close_confirmation().unwrap();
    drop(work);
    assert!(row.wait_compacted().await.unwrap().drained);
    assert_eq!(
        row.confirmation_status().unwrap(),
        ConfirmationStatus::Draining
    );
    assert!(!ledger.channel_may_close(&selected));
    coordinator.end(selected.id());
    // Force the real low-level registry's physical close, bypassing the production preflight,
    // to verify GC's independent final-proof check rather than fabricating collection evidence.
    retire(&registry, &selected);
    drop((output, route, phase, join, action, row));
    registry.collect_retired(&ledger);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_some());
    assert!(
        original_phase.upgrade().is_some(),
        "unready original proof stays indexed"
    );
    drop(root_input);
    assert_eq!(
        original_phase.upgrade().unwrap().confirmation_status(),
        ConfirmationStatus::Drained
    );
    // No Root -> Row wake exists. The ordinary bounded empty-registry sweep must observe
    // readiness and retire actual Phase/Row credits outside Index/State, without new authority.
    Registry::default().collect_retired(&ledger);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    assert!(original_phase.upgrade().is_none());
}

pub(in crate::voice_handoff) fn installed(
    registry: &Registry,
    selected: Arc<SelectedHandoffOwner>,
) -> (Arc<SetupOutput>, Route) {
    let Begin::Work(mut ticket) = registry.begin(selected, [3; 32]).unwrap() else {
        panic!("new original setup")
    };
    let (session, public, setup) = ticket.compute(&mut Rng::new()).unwrap();
    let output = Arc::new(SetupOutput {
        public,
        _credit: ticket.row._credit.clone(),
    });
    let mut prepared = Some((session, output.clone(), setup));
    ticket.install(&mut prepared).unwrap();
    drop(ticket);
    let route = registry.route(output.public().slot).unwrap();
    (output, route)
}

fn retire(registry: &Registry, selected: &SelectedHandoffOwner) {
    let row = registry
        .close_row(selected.id(), selected.executor(), None)
        .unwrap();
    row.retire_channel().unwrap();
}

/// A real compacted native row. Phase::close models the separate authenticated browser
/// protocol, as in the existing ledger tests. Root's independent history is not GC evidence.
async fn drained(
    ledger: &Arc<Ledger>,
    coordinator: &crate::voice_authority::Coordinator,
    selected: &Arc<SelectedHandoffOwner>,
    selector: PairSelector,
) -> (
    RowId,
    Arc<Phase>,
    catcoms_sync::voice_v2::presence::LocalJoin,
) {
    let ActionBegin::Factory(registration) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("new action")
    };
    let (join, action, pair) = actual_action(coordinator, selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(selected, Purpose::Derive(selector), id)
        .unwrap();
    let work = row.work().unwrap();
    let phase = work.phase().clone();
    action.settle_engine();
    phase.close();
    drop(work);
    assert!(row.wait_compacted().await.unwrap().drained);
    // This helper promises a fully collectable plan. Ordinary Return alone no longer
    // establishes that: retire the actual original plan through its retained native owner.
    assert!(row.retire_plan().unwrap());
    (id, phase, join)
}

#[tokio::test]
async fn compacted_dead_history_waits_for_original_header_public_and_route_owners() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let (output, route) = installed(&registry, selected.clone());
    let header = route.session().header().unwrap();
    let (id, _phase, _join) = drained(&ledger, &coordinator, &selected, selector).await;
    coordinator.end(selected.id());
    retire(&registry, &selected);
    registry.collect_retired(&ledger);
    assert!(ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .is_ok());
    drop(route);
    registry.collect_retired(&ledger);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_some());
    drop(header);
    registry.collect_retired(&ledger);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_some());
    drop(output);
    registry.collect_retired(&ledger);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    assert!(ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .is_err());
}

#[tokio::test]
async fn actual_collection_proof_survives_late_metadata_handles_and_empty_registry_sweeps() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let (output, route) = installed(&registry, selected.clone());
    let (id, _phase, _join) = drained(&ledger, &coordinator, &selected, selector).await;
    let late = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    coordinator.end(selected.id());
    retire(&registry, &selected);
    drop((output, route));
    registry.collect_retired(&ledger); // the one original witness arrives here
    assert!(registry
        .original_selected(selected.id(), selected.executor())
        .is_none());
    assert!(late.reconcile().unwrap().drained);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_some());
    drop(late);
    // A different uninitialized Registry has no new witness to emit. The persistent ledger
    // proof must still let the ordinary admission hook collect a last-owner drop.
    Registry::default().collect_retired(&ledger);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
}

#[tokio::test]
async fn live_lease_and_equal_label_foreign_channel_never_supply_collection_proof() {
    let (mut coordinator, selected, selector) = selection();
    let (mut foreign_coordinator, foreign, foreign_selector) = selection();
    assert_eq!(selected.id(), foreign.id());
    assert_eq!(selected.executor(), foreign.executor());
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let foreign_registry = Registry::default();
    let (output, route) = installed(&registry, selected.clone());
    let (foreign_output, foreign_route) = installed(&foreign_registry, foreign.clone());
    let (id, _phase, _join) = drained(&ledger, &coordinator, &selected, selector).await;
    let (foreign_id, _foreign_phase, _foreign_join) =
        drained(&ledger, &foreign_coordinator, &foreign, foreign_selector).await;
    assert!(selected.lease().is_live());
    assert!(foreign.lease().is_live());
    retire(&registry, &selected);
    drop((output, route));
    registry.collect_retired(&ledger);
    assert!(
        ledger
            .resolve(&selected, Purpose::Derive(selector), id)
            .is_ok(),
        "live lease retains its closed-purpose fence"
    );
    coordinator.end(selected.id());
    foreign_coordinator.end(foreign.id());
    retire(&foreign_registry, &foreign);
    drop((foreign_output, foreign_route));
    foreign_registry.collect_retired(&ledger);
    assert!(ledger
        .resolve(&foreign, Purpose::Derive(foreign_selector), foreign_id)
        .is_err());
    assert!(
        ledger
            .resolve(&selected, Purpose::Derive(selector), id)
            .is_ok(),
        "an equal-label foreign witness cannot collect this original allocation"
    );
    registry.collect_retired(&ledger);
    assert!(ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .is_err());
}

#[tokio::test]
async fn witness_before_ack_settlement_and_phase_drain_remains_pending_until_actual_compaction() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let (output, route) = installed(&registry, selected.clone());
    let ActionBegin::Factory(registration) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("action")
    };
    let (mut join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let work = row.work().unwrap();
    let phase = work.phase().clone();
    let buffer = phase.buffer(23).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = crate::voice_authority::tests::derivation_receipt(
        &coordinator,
        selected.id(),
        &mut join,
        &action,
    );
    work.refresh().unwrap();
    coordinator.end(selected.id());
    // Internal registry collection is exercised directly to verify that even a valid physical
    // witness is NOT proof of action cleanup. The production close adapter now refuses here.
    retire(&registry, &selected);
    drop((output, route));
    registry.collect_retired(&ledger);
    assert!(!row.status().unwrap().drained);
    row.acknowledge(&receipt).unwrap();
    row.settle().unwrap();
    phase.close();
    drop(work);
    registry.collect_retired(&ledger);
    assert!(!row.status().unwrap().drained);
    drop(buffer);
    assert!(row.wait_compacted().await.unwrap().drained);
    assert!(
        !ledger.channel_may_close(&selected),
        "local Return is not the Root last-owner witness"
    );
    assert!(row.retire_plan().unwrap());
    registry.collect_retired(&ledger);
    assert!(
        row.reconcile().unwrap().drained,
        "late handle still pins exact terminal evidence"
    );
    drop(row);
    registry.collect_retired(&ledger);
    assert!(ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
}

#[tokio::test]
async fn handed_off_pending_purpose_is_not_evicted_by_actual_channel_collection() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let (output, route) = installed(&registry, selected.clone());
    let ActionBegin::Factory(registration) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("pending")
    };
    registration.mark_handoff().unwrap();
    let id = registration.id();
    coordinator.end(selected.id());
    retire(&registry, &selected);
    drop((output, route));
    registry.collect_retired(&ledger);
    drop(registration); // an actor reply may have been lost after Root pair publication
    registry.collect_retired(&ledger);
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    assert!(!row.status().unwrap().drained);
    assert!(!row.status().unwrap().engine.settled);
    assert!(!ledger.channel_may_close(&selected));
    assert!(matches!(row.work(), Err(VoiceError::Unauthorized)));
}

#[tokio::test]
async fn more_than_thirty_two_completed_generations_reuse_slots_without_aliasing_history() {
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let runtime =
        catcoms_sync::voice_v2::runtime::Runtime::new(Arc::new(catcoms_rt::ManualClock::new(1000)));
    let mut previous = None;
    for expected in 1..=36_u64 {
        let (mut coordinator, selected, selector) = selection_with_runtime(runtime.clone());
        let (output, route) = installed(&registry, selected.clone());
        let (id, _phase, _join) = drained(&ledger, &coordinator, &selected, selector).await;
        let logical = id.logical();
        assert_eq!(&logical[..8], &[0; 8]);
        assert_eq!(
            u64::from_be_bytes(logical[8..].try_into().unwrap()),
            expected
        );
        if let Some(old) = previous {
            assert!(ledger
                .resolve(&selected, Purpose::Derive(selector), old)
                .is_err());
        }
        coordinator.end(selected.id());
        retire(&registry, &selected);
        drop((output, route));
        registry.collect_retired(&ledger);
        assert!(ledger
            .lookup_original(&selected, Purpose::Derive(selector))
            .is_none());
        previous = Some(id);
    }
}

#[tokio::test]
async fn both_explicit_close_variants_preserve_control_while_original_factory_is_pending() {
    for channel in [false, true] {
        let f = Fixture::new().await;
        let (_owner, selected, selector) =
            crate::voice_handoff::factory::tests::selection(&f).await;
        let route = crate::voice_handoff::factory::tests::channel_route(&f, &selected).await;
        let ActionBegin::Factory(registration) = f
            .state
            .voice_handoff
            .begin(selected.clone(), Purpose::Derive(selector))
            .unwrap()
        else {
            panic!("pending")
        };
        let counts = selected.resources().counts();
        let result = if channel {
            collect_channel(&f.state, selected.id(), selected.executor()).await
        } else {
            settle_setup(&f.state, selected.id(), selected.executor()).await
        };
        assert!(matches!(result, Err(Refused::Capacity)));
        assert!(route.require_open().is_ok());
        assert_eq!(route.snapshot(), [1, 1, 1]);
        assert_eq!(selected.resources().counts(), counts);
        drop(registration); // demonstrably unstarted: no actor publication ever occurred
        collect_channel(&f.state, selected.id(), selected.executor())
            .await
            .unwrap();
        assert!(route.require_open().is_err());
        f.close().await;
    }
}

#[tokio::test]
async fn both_explicit_close_variants_keep_control_until_compacted_final_root_proof_is_ready() {
    use crate::voice_handoff::custody::ConfirmationStatus;
    for channel in [false, true] {
        let f = Fixture::new().await;
        // The protected action uses the existing synthetic lifecycle grant. Install that
        // exact selected owner in the real registry, then exercise BOTH actual AppState
        // finish adapters. This is cleanup coverage, not normal Actor/MLS acceptance.
        let (coordinator, selected, selector) = selection();
        let (output, route) = installed(&f.state.voice_channels, selected.clone());
        let ActionBegin::Factory(registration) = f
            .state
            .voice_handoff
            .begin(selected.clone(), Purpose::Derive(selector))
            .unwrap()
        else {
            panic!("new original action")
        };
        let (join, action, pair) = actual_action(&coordinator, &selected);
        let root_input = action.input().unwrap();
        let id = registration
            .attach(action.clone(), pair, 16 * 1024)
            .unwrap();
        let row = f
            .state
            .voice_handoff
            .resolve(&selected, Purpose::Derive(selector), id)
            .unwrap();
        let work = row.work().unwrap();
        let phase = work.phase().clone();
        action.settle_engine();
        drop(work);
        row.close_phase(
            route.session().identity(),
            super::super::super::custody::CloseMode::Retire,
        )
        .unwrap();
        assert!(row.wait_compacted().await.unwrap().drained);
        assert_eq!(
            row.confirmation_status().unwrap(),
            ConfirmationStatus::Draining
        );
        // Keep the actual LocalJoin's revocation owner alive while asserting that the
        // call's Control route remains open. Its Drop legitimately revokes the call lease;
        // final Workflow retirement must instead depend on root_input, not ending the call.
        drop((phase, action));
        assert!(selected.lease().is_live());
        let counts = selected.resources().counts();
        let original_slots = route.snapshot();
        for _ in 0..2 {
            let result = if channel {
                collect_channel(&f.state, selected.id(), selected.executor()).await
            } else {
                settle_setup(&f.state, selected.id(), selected.executor()).await
            };
            assert!(matches!(result, Err(Refused::Capacity)));
            assert!(route.require_open().is_ok());
            assert_eq!(route.snapshot(), original_slots);
            assert_eq!(selected.resources().counts(), counts);
            assert_eq!(
                row.confirmation_status().unwrap(),
                ConfirmationStatus::Draining
            );
        }
        drop(root_input); // the actual late Root owner, not a supplied readiness assertion
        assert_eq!(
            row.confirmation_status().unwrap(),
            ConfirmationStatus::Drained
        );
        for stage in 0..2 {
            let result = if channel {
                collect_channel(&f.state, selected.id(), selected.executor()).await
            } else {
                settle_setup(&f.state, selected.id(), selected.executor()).await
            };
            assert!(
                matches!(result, Err(Refused::Capacity)),
                "unACKed final stage {stage}"
            );
            assert!(route.require_open().is_ok());
            let issued = row.issue_final(route.session().identity()).unwrap();
            assert!(issued.proof.is_some() && !issued.acknowledged);
        }
        let issued = row.issue_final(route.session().identity()).unwrap();
        row.acknowledge_final(route.session().identity(), issued.proof.unwrap())
            .unwrap();
        let result = if channel {
            collect_channel(&f.state, selected.id(), selected.executor()).await
        } else {
            settle_setup(&f.state, selected.id(), selected.executor()).await
        };
        assert!(matches!(result, Ok(Status::Closed { .. })));
        assert!(route.require_open().is_err());
        drop((row, route, output, join));
        f.close().await;
    }
}

#[tokio::test]
async fn action_and_channel_collection_need_no_fresh_capacity_under_full_pool_pressure() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registry = Registry::default();
    let (output, route) = installed(&registry, selected.clone());
    let (id, _phase, _join) = drained(&ledger, &coordinator, &selected, selector).await;
    coordinator.end(selected.id());
    let pool = selected.resources();
    let group = selected.binding().scope.group;
    let mut pressure = Vec::new();
    // Binary fill accounts for the actual group bytes rather than assuming that the process
    // index belongs to this group. Fixture allocations do not claim production copy bounds.
    for shift in (0..=18).rev() {
        if let Ok(credit) = pool.charge(group, 1 << shift, ResourceKind::Retained) {
            pressure.push(credit);
        }
    }
    assert!(pool.charge(group, 1, ResourceKind::Retained).is_err());
    for value in 97..=105 {
        let left = 2 * 1024 * 1024 - pool.counts().0;
        if left == 0 {
            break;
        }
        pressure.push(
            pool.charge(
                [value; 32],
                left.min(catcoms_sync::voice_v2::resources::GROUP_BYTES),
                ResourceKind::Retained,
            )
            .unwrap(),
        );
    }
    assert_eq!(pool.counts().0, 2 * 1024 * 1024);
    retire(&registry, &selected);
    drop((output, route));
    registry.collect_retired(&ledger);
    assert!(ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .is_err());
    assert!(pool.counts().0 < 2 * 1024 * 1024);
    assert!(
        !pressure.is_empty(),
        "unrelated pressure remains held throughout actual collection"
    );
}
