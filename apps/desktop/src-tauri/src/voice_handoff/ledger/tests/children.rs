//! Native reservation/retirement orderings. Real Root actions and loans use the existing
//! signed discovery fixture; these tests do not claim Actor intake, IPC, or WebView success.
use super::*;

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
