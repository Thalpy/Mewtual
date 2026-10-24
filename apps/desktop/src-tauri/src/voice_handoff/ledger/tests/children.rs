//! Native reservation/retirement orderings. Real Root actions and loans use the existing
//! signed discovery fixture; these tests do not claim Actor intake, IPC, or WebView success.
use super::*;

#[test]
fn continuation_scan_does_not_infer_derived_success_or_incoming_authority_from_return() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, _action, parent) = returned_parent(&ledger, &coordinator, &selected, selector);
    let mut cursor = 0;
    let before = selected.resources().counts();
    assert!(
        ledger
            .confirmation_continuation(&selected, &mut cursor)
            .is_none(),
        "ordinary Return without Derived/ACK is not Start authority"
    );
    assert_eq!(selected.resources().counts(), before);
    let mut rows = Vec::new();
    for request in [
        ConfirmationPurpose::Incoming(1),
        ConfirmationPurpose::Start(None),
    ] {
        let (Begin::Factory(ticket), _) = ledger
            .begin_child(selected.clone(), &parent, request)
            .unwrap()
        else {
            panic!("original child")
        };
        ticket.mark_handoff().unwrap();
        rows.push((request, ticket.id()));
        if matches!(request, ConfirmationPurpose::Start(_)) {
            assert!(
                ledger
                    .confirmation_continuation(&selected, &mut cursor)
                    .is_none(),
                "an actual worker cannot be selected again"
            );
        }
        drop(ticket);
    }
    let before = selected.resources().counts();
    for _ in 0..4 {
        let original = ledger
            .confirmation_continuation(&selected, &mut cursor)
            .unwrap();
        assert!(original.request == ConfirmationPurpose::Start(None));
        assert!(Arc::ptr_eq(&original.parent.row, &parent.row));
        assert!(original.incoming.is_none() && original.known.is_none());
        assert_eq!(
            selected.resources().counts(),
            before,
            "scans never allocate work or renew capacity"
        );
    }
    let (_, foreign, _) = selection();
    assert!(ledger
        .confirmation_continuation(&foreign, &mut cursor)
        .is_none());
    for (request, id) in rows {
        ledger
            .lookup_child(&selected, &parent, request, Some(id))
            .unwrap()
            .close_pending()
            .unwrap();
    }
    coordinator.end(selected.id());
    parent.retire_plan().unwrap();
    assert!(ledger
        .confirmation_continuation(&selected, &mut cursor)
        .is_none());
}

#[test]
fn existing_work_projection_keeps_full_child_purpose_after_actual_parent_return_and_compaction() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, _action, parent) = returned_parent(&ledger, &coordinator, &selected, selector);
    let request = ConfirmationPurpose::Incoming(1);
    let (Begin::Factory(child), _) = ledger
        .begin_child(selected.clone(), &parent, request)
        .unwrap()
    else {
        panic!("native child Pending")
    };
    let id = child.id();
    child.mark_handoff().unwrap(); // Fixture handoff uncertainty, not Actor publication.
    drop(child); // An unseen native reservation, not an invented incoming Action.
    let first = ledger.project_work(&selected, 0).unwrap();
    assert_eq!(first.metadata.as_ref().unwrap().row, parent.id());
    let projected = ledger.project_work(&selected, first.next_slot).unwrap();
    let metadata = projected.metadata.as_ref().unwrap();
    assert_eq!(metadata.row, id);
    assert!(
        metadata.purpose
            == Purpose::Confirmation {
                parent: parent.id(),
                request
            }
    );
    assert_eq!(metadata.parent_selector, Some(selector));
    assert!(metadata.pending == PendingStatus::Open && metadata.action.is_none());
    drop(parent);
    coordinator.end(selected.id());
    let after_end = ledger.project_work(&selected, first.next_slot).unwrap();
    assert_eq!(after_end.metadata.as_ref().unwrap().row, id);
    assert_eq!(
        after_end.metadata.as_ref().unwrap().parent_selector,
        Some(selector)
    );
    assert!(ledger
        .project_work(&selected, projected.next_slot)
        .unwrap()
        .metadata
        .is_none());
}

#[test]
fn known_retry_cannot_reserve_or_rearm_an_uncertain_child_without_actual_publication() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, _action, parent) = returned_parent(&ledger, &coordinator, &selected, selector);
    let request = ConfirmationPurpose::Incoming(1);
    let (Begin::Factory(ticket), _) = ledger
        .begin_child(selected.clone(), &parent, request)
        .unwrap()
    else {
        panic!("original child")
    };
    ticket.mark_handoff().unwrap();
    let id = ticket.id();
    drop(ticket);
    let child = ledger
        .lookup_child(&selected, &parent, request, Some(id))
        .unwrap();
    assert!(!child.has_pending_publication().unwrap());
    let before = selected.resources().counts();
    for _ in 0..8 {
        assert!(matches!(
            ledger.resume_known_child(selected.clone(), &parent, &child, request),
            Err(VoiceError::Capacity)
        ));
        assert_eq!(selected.resources().counts(), before);
        assert!(child.status().unwrap().action.is_none());
        assert!(!child.has_pending_publication().unwrap());
    }
    let other = Arc::new(Ledger::default());
    assert!(matches!(
        other.resume_known_child(selected.clone(), &parent, &child, request),
        Err(VoiceError::Unauthorized)
    ));
    child.close_pending().unwrap();
    assert!(
        matches!(ledger.resume_known_child(selected.clone(), &parent, &child, request).unwrap(), Begin::Existing(actual) if actual == id)
    );
    parent.retire_plan().unwrap();
}

fn returned_parent(
    ledger: &Arc<Ledger>,
    coordinator: &Coordinator,
    selected: &Arc<SelectedHandoffOwner>,
    selector: PairSelector,
) -> (
    catcoms_sync::voice_v2::presence::LocalJoin,
    Arc<CryptoAction>,
    ResolvedRow,
) {
    let (join, action, pair) = actual_action(coordinator, selected);
    let id = ticket(ledger, selected.clone(), selector)
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let parent = ledger
        .resolve(selected, Purpose::Derive(selector), id)
        .unwrap();
    action.settle_engine();
    // No provider was run in this fixture. This ordinary native Return deliberately does
    // not pretend that the pair derived keys or that a subsequent Actor Start can succeed.
    let endpoint = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    parent
        .close_phase(endpoint.session.identity(), CloseMode::Return)
        .unwrap();
    assert!(ledger.compact(id).unwrap());
    (join, action, parent)
}

#[test]
fn child_reservation_wins_before_parent_retirement_and_blocks_it_until_local_closure() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, _action, parent) = returned_parent(&ledger, &coordinator, &selected, selector);
    let handshake = parent.confirmation_handshake().unwrap();
    let request = ConfirmationPurpose::Start(None);
    let (Begin::Factory(child), trigger) = ledger
        .begin_child(selected.clone(), &parent, request)
        .unwrap()
    else {
        panic!("child factory")
    };
    assert!(trigger.is_none());
    let id = child.id();
    child.mark_handoff().unwrap();
    assert!(matches!(parent.retire_plan(), Err(VoiceError::Capacity)));
    drop(child); // a lost Actor reply remains a counted uncertain purpose
    assert!(matches!(parent.retire_plan(), Err(VoiceError::Capacity)));
    let child = ledger
        .resolve(
            &selected,
            Purpose::Confirmation {
                parent: parent.id(),
                request,
            },
            id,
        )
        .unwrap();
    assert!(child
        .row
        .key
        .parent
        .as_ref()
        .is_some_and(|row| Arc::ptr_eq(row, &parent.row)));
    let endpoint = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    assert!(matches!(
        child.close_phase(endpoint.session.identity(), CloseMode::Retire),
        Err(VoiceError::Unauthorized)
    ));
    let closed = child.close_pending().unwrap();
    assert!(matches!(
        closed.pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(!closed.drained && !closed.engine.settled);
    // Original parent control may now freeze adoption; its real witness can remain Pending.
    parent.retire_plan().unwrap();
    assert_eq!(parent.confirmation_handshake().unwrap(), handshake);
    assert!(matches!(
        ledger.begin_child(
            selected.clone(),
            &parent,
            ConfirmationPurpose::Start(Some([8; 32]))
        ),
        Err(VoiceError::Capacity)
    ));
    // Existing local closure remains historical; it is never a new Factory or Root absence.
    assert!(
        matches!(ledger.begin_child(selected, &parent, request).unwrap().0, Begin::Existing(old) if old == id)
    );
}

#[test]
fn parent_retirement_wins_before_child_reservation_without_allocating_a_purpose() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, _action, parent) = returned_parent(&ledger, &coordinator, &selected, selector);
    parent.retire_plan().unwrap();
    let counts = selected.resources().counts();
    for _ in 0..8 {
        assert!(matches!(
            ledger.begin_child(selected.clone(), &parent, ConfirmationPurpose::Start(None)),
            Err(VoiceError::Capacity)
        ));
        assert_eq!(selected.resources().counts(), counts);
    }
    assert!(ledger
        .lookup_original(
            &selected,
            Purpose::Confirmation {
                parent: parent.id(),
                request: ConfirmationPurpose::Start(None)
            }
        )
        .is_none());
}

#[test]
fn child_lookup_requires_actual_indexed_parent_allocation_and_exact_owner_purpose_and_row() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, _action, parent) = returned_parent(&ledger, &coordinator, &selected, selector);
    let request = ConfirmationPurpose::Start(None);
    let (Begin::Factory(child), _) = ledger
        .begin_child(selected.clone(), &parent, request)
        .unwrap()
    else {
        panic!("first child")
    };
    let id = child.id();
    child.mark_handoff().unwrap();
    drop(child);
    let before = selected.resources().counts();
    for child_id in [None, Some(id)] {
        let found = ledger
            .lookup_child(&selected, &parent, request, child_id)
            .unwrap();
        assert!(found.id() == id);
    }
    for wrong_id in [
        parent.id(),
        RowId {
            slot: id.slot,
            serial: id.serial + 1,
        },
    ] {
        assert!(matches!(
            ledger.lookup_child(&selected, &parent, request, Some(wrong_id)),
            Err(VoiceError::Unauthorized)
        ));
    }
    for wrong_request in [
        ConfirmationPurpose::Incoming(1),
        ConfirmationPurpose::Start(Some([7; 32])),
    ] {
        assert!(matches!(
            ledger.lookup_child(&selected, &parent, wrong_request, None),
            Err(VoiceError::Unauthorized)
        ));
    }
    let (other_coordinator, foreign_selected, foreign_selector) = selection();
    let other = Arc::new(Ledger::default());
    let (_foreign_join, _foreign_action, foreign) = returned_parent(
        &other,
        &other_coordinator,
        &foreign_selected,
        foreign_selector,
    );
    assert!(parent.id() == foreign.id());
    assert!(matches!(
        ledger.lookup_child(&selected, &foreign, request, None),
        Err(VoiceError::Unauthorized)
    ));
    assert!(matches!(
        ledger.lookup_child(&foreign_selected, &parent, request, None),
        Err(VoiceError::Unauthorized)
    ));
    // A detached foreign row with equal slot/serial cannot bypass either owner or actual
    // indexed-parent allocation checks by being wrapped in this ledger's resolver handle.
    let detached = ResolvedRow {
        ledger: ledger.clone(),
        row: foreign.row.clone(),
    };
    assert!(matches!(
        ledger.lookup_child(&selected, &detached, request, None),
        Err(VoiceError::Unauthorized)
    ));
    // SAME selected allocation and full public purpose, but another original row. This
    // passes key.matches and specifically pins the indexed Arc fence (not just labels).
    let parallel = Arc::new(Ledger::default());
    let parallel_ticket = ticket(&parallel, selected.clone(), selector);
    let parallel_parent = parallel
        .resolve(&selected, Purpose::Derive(selector), parallel_ticket.id())
        .unwrap();
    assert!(parallel_parent.id() == parent.id());
    assert!(matches!(
        ledger.lookup_child(&selected, &parallel_parent, request, None),
        Err(VoiceError::Unauthorized)
    ));
    let same_ledger_foreign_arc = ResolvedRow {
        ledger: ledger.clone(),
        row: parallel_parent.row.clone(),
    };
    assert!(same_ledger_foreign_arc
        .row
        .key
        .matches(&selected, parent.row.key.purpose));
    assert!(matches!(
        ledger.lookup_child(&selected, &same_ledger_foreign_arc, request, None),
        Err(VoiceError::Unauthorized)
    ));
    drop((
        parallel_ticket,
        parallel_parent,
        same_ledger_foreign_arc,
        parallel,
    ));
    assert_eq!(selected.resources().counts(), before);
    let found = ledger
        .lookup_child(&selected, &parent, request, Some(id))
        .unwrap();
    found.close_pending().unwrap();
    parent.retire_plan().unwrap();
    assert!(
        ledger
            .lookup_child(&selected, &parent, request, Some(id))
            .is_ok(),
        "history survives original plan retirement without live control"
    );
}

#[test]
fn equal_parent_labels_from_another_ledger_and_unproven_response_triggers_refuse() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let other = Arc::new(Ledger::default());
    let (_join, _action, parent) = returned_parent(&ledger, &coordinator, &selected, selector);
    // Independent native owner: binding a second join to one live lease is itself refused.
    let (other_coordinator, other_selected, other_selector) = selection();
    let (_other_join, _other_action, foreign) =
        returned_parent(&other, &other_coordinator, &other_selected, other_selector);
    assert!(parent.id() == foreign.id());
    assert!(matches!(
        ledger.begin_child(selected.clone(), &foreign, ConfirmationPurpose::Start(None)),
        Err(VoiceError::Unauthorized)
    ));
    assert!(matches!(
        ledger.begin_child(
            selected.clone(),
            &parent,
            ConfirmationPurpose::Respond(parent.id())
        ),
        Err(VoiceError::Unauthorized)
    ));
    for role in [0, 10, 11, 12, 255] {
        assert!(matches!(
            ledger.begin_child(
                selected.clone(),
                &parent,
                ConfirmationPurpose::Incoming(role)
            ),
            Err(VoiceError::Unauthorized)
        ));
    }
    let purpose = Purpose::Confirmation {
        parent: parent.id(),
        request: ConfirmationPurpose::Incoming(1),
    };
    assert!(
        matches!(
            ledger.begin(selected, purpose),
            Err(VoiceError::Unauthorized)
        ),
        "generic Begin cannot bypass original parent admission"
    );
}

#[test]
fn child_purposes_share_the_existing_32_positions_without_uncertain_history_eviction() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let (_join, _action, parent) = returned_parent(&ledger, &coordinator, &selected, selector);
    let mut ids = Vec::new();
    for n in 0..ROWS - 1 {
        let request = ConfirmationPurpose::Start(Some([n as u8; 32]));
        let (Begin::Factory(child), _) = ledger
            .begin_child(selected.clone(), &parent, request)
            .unwrap()
        else {
            panic!("new paid position")
        };
        child.mark_handoff().unwrap();
        ids.push((child.id(), request));
        drop(child);
    }
    let full = selected.resources().counts();
    assert!(matches!(
        ledger.begin_child(selected.clone(), &parent, ConfirmationPurpose::Start(None)),
        Err(VoiceError::Capacity)
    ));
    assert_eq!(selected.resources().counts(), full);
    for (id, request) in ids {
        let historical = ledger
            .lookup_original(
                &selected,
                Purpose::Confirmation {
                    parent: parent.id(),
                    request,
                },
            )
            .unwrap();
        assert!(historical.id() == id);
        assert!(matches!(
            historical.status().unwrap().pending,
            PendingStatus::Open
        ));
    }
}
