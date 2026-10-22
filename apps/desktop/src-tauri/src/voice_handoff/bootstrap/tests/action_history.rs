//! Real registry/codec and protected action factories, with the explicitly synthetic desktop
//! lifecycle grant from ledger tests. These tests model prior authenticated browser Phase
//! closure; they do not establish browser drainage or successful ServerActor/MLS handoff.
use super::*;
use crate::voice_handoff::custody::Phase;
use crate::voice_handoff::ledger::tests::{actual_action, selection, selection_with_runtime};
use crate::voice_handoff::ledger::{Begin as ActionBegin, Ledger, Purpose, RowId};
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::VoiceError;

fn installed(
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
            pool.charge([value; 32], left.min(256 * 1024), ResourceKind::Retained)
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
