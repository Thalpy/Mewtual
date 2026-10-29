use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_handoff::factory::tests::{channel_route, selection};
use crate::voice_handoff::ledger::Begin;

#[tokio::test]
async fn work_reply_is_serial_paid_bounded_and_available_after_end_and_lock() {
    let f = Fixture::new().await;
    let (owner, selected, selector) = selection(&f).await;
    let _route = channel_route(&f, &selected).await;
    let executor = hex::encode(selected.executor());
    let Begin::Factory(registration) = f
        .state
        .voice_handoff
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("new Pending")
    };
    let id = registration.id();
    // Preserve an uncertain native handoff rather than the proven-unstarted removal path.
    // This marks fixture ownership only; it does not claim an Actor published any action.
    registration.mark_handoff().unwrap();
    drop(registration);
    let pool = selected.resources();
    let group = selected.binding().scope.group;
    let pressure = pool
        .charge(
            group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.group_counts(group).unwrap().0,
            catcoms_sync::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    assert_eq!(
        pool.group_counts(group).unwrap().0,
        catcoms_sync::voice_v2::resources::GROUP_BYTES
    );
    let before = pool.counts();
    let reply = read_inner(&f.state, owner.clone(), executor.clone(), 0).unwrap();
    assert_eq!(pool.counts(), before);
    assert!(
        read_inner(&f.state, owner.clone(), executor.clone(), 0).is_err(),
        "one original serial projection"
    );
    assert!(
        f.state
            .voice_channels
            .project(&owner, selected.executor())
            .is_err(),
        "setup cannot overlap work serialization"
    );
    crate::register_ui_lock_request(&f.state);
    f.state.voice_authority.lock().unwrap().revoke_all();
    let after_lock = pool.counts();
    let json = reply.json().unwrap();
    assert_eq!(json.capacity(), PUBLIC_REPLY_BYTES);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["work"]["state"], "pending");
    assert_eq!(value["work"]["row"], hex::encode(id.logical()));
    assert_eq!(value["work"]["identity"], serde_json::Value::Null);
    assert_eq!(
        pool.counts(),
        after_lock,
        "serialization retains its original payment after lock"
    );
    drop(reply);
    let original = f
        .state
        .voice_handoff
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    original.close_pending().unwrap();
    let closed = read_inner(&f.state, owner.clone(), executor.clone(), 0).unwrap();
    let value: serde_json::Value = serde_json::from_str(&closed.json().unwrap()).unwrap();
    assert_eq!(value["work"]["state"], "closedPending");
    assert_eq!(
        value["work"]["actorHandoff"], true,
        "local closure preserves uncertainty about the original native handoff"
    );
    assert!(read_inner(&f.state, owner.clone(), executor.clone(), 33).is_err());
    assert!(read_inner(&f.state, owner, hex::encode([8; 32]), 0).is_err());
    drop(closed);
    drop(original);
    drop(pressure);
    f.close().await;
}

#[tokio::test]
async fn delayed_work_reply_keeps_only_its_original_rows_paid_after_host_and_app_teardown() {
    let f = Fixture::new().await;
    let (owner, selected, selector) = selection(&f).await;
    let route = channel_route(&f, &selected).await;
    let mut config = serde_json::from_str(include_str!("../../../../tauri.conf.json")).unwrap();
    let host = Host::prepare(&f.state, &mut config, false).unwrap();
    host.start_in_tauri_runtime().unwrap();
    let Begin::Factory(registration) = f
        .state
        .voice_handoff
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("native Pending")
    };
    // Exercise an unseen uncertain Pending that survives its original producer reply.
    registration.mark_handoff().unwrap();
    drop(registration);
    let reply = read_inner(&f.state, owner, hex::encode(selected.executor()), 0).unwrap();
    let pool = selected.resources();
    let ledger = Arc::downgrade(&f.state.voice_handoff);
    let channels = Arc::downgrade(&f.state.voice_channels);
    drop(route);
    drop(selected);
    host.finish().await.unwrap();
    drop(host);
    f.close().await;
    assert!(
        ledger.upgrade().is_none(),
        "a public reply cannot retain the process ledger/index"
    );
    assert!(
        channels.upgrade().is_none(),
        "a public reply cannot retain the channel registry"
    );
    let retained = pool.counts();
    assert!(
        retained.0 >= 8192 + 2304,
        "both exact projected native rows remain paid"
    );
    let json = reply.json().unwrap();
    assert_eq!(pool.counts(), retained);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["work"]["state"],
        "pending"
    );
    drop(reply);
    assert_eq!(
        pool.counts(),
        (0, 0),
        "actual last native reply owner returns all remaining payments"
    );
}

#[test]
fn maximum_work_purposes_and_u64s_fit_the_same_capped_json_without_intermediate_strings() {
    use crate::voice_handoff::ledger::RowId;
    use catcoms_sync::voice_v2::presence::PairSelector;
    let mut logical = [255; 16];
    logical[..8].copy_from_slice(&31u64.to_be_bytes());
    let row = RowId::from_logical(logical).unwrap();
    let mut earlier = logical;
    earlier[..8].copy_from_slice(&30u64.to_be_bytes());
    earlier[8..].copy_from_slice(&(u64::MAX - 1).to_be_bytes());
    let parent = RowId::from_logical(earlier).unwrap();
    earlier[..8].copy_from_slice(&29u64.to_be_bytes());
    earlier[8..].copy_from_slice(&(u64::MAX - 2).to_be_bytes());
    let trigger = RowId::from_logical(earlier).unwrap();
    let selector = PairSelector {
        peer: catcoms_crypto::DeviceId::from_bytes([255; 32]),
        join: [255; 32],
        key_ref: [255; 32],
        epoch: u64::MAX,
    };
    for purpose in [
        Purpose::Derive(selector),
        Purpose::Confirmation {
            parent,
            request: ConfirmationPurpose::Start(Some([255; 32])),
        },
        Purpose::Confirmation {
            parent,
            request: ConfirmationPurpose::Respond(trigger),
        },
        Purpose::Confirmation {
            parent,
            request: ConfirmationPurpose::Incoming(9),
        },
    ] {
        let metadata = WorkMetadata {
            row,
            purpose,
            parent_selector: matches!(purpose, Purpose::Confirmation { .. }).then_some(selector),
            pending: PendingStatus::NotPending,
            action: Some((u64::MAX, u64::MAX, [255; 32])),
        };
        let mut output = JsonBuffer {
            bytes: Vec::with_capacity(PUBLIC_REPLY_BYTES),
        };
        serde_json::to_writer(&mut output, &Descriptor(&metadata)).unwrap();
        assert_eq!(output.bytes.capacity(), PUBLIC_REPLY_BYTES);
        // Leave room for original owner/executor and outer snapshot fields, not merely the
        // standalone descriptor. This is serialization-only, never an admitted Action fixture.
        assert!(output.bytes.len() + 256 < PUBLIC_REPLY_BYTES);
        let value: serde_json::Value = serde_json::from_slice(&output.bytes).unwrap();
        assert_eq!(value["identity"]["action"], u64::MAX.to_string());
        assert_eq!(value["identity"]["version"], u64::MAX.to_string());
        assert_eq!(value["row"], hex::encode(logical));
        if matches!(purpose, Purpose::Confirmation { .. }) {
            assert_eq!(
                value["purpose"]["parentPurpose"]["epoch"],
                u64::MAX.to_string()
            );
        }
    }
}
