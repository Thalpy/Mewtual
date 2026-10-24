//! Independent chat bubbles meet only through ordinary native reconnect/catch-up. The schedule
//! runs each isolated pair sequentially from the same saved base: their edits are causally
//! concurrent, but this is not a claim about fifty simultaneously running network clients.
use super::*;
use automerge::Change;
use catcoms_replication::EncryptedDoc;
use catcoms_wire::{Decoder, DocType};

fn general() -> u128 {
    channel_id("general")
}
const SURVIVORS: &[usize] = &[0, 1, 2, 3, 4];
const EDITED: &str = "edited in an independent bubble";
const EMOJI: &str = "👍";

/// Observe a real native command's original signature. Never infer history from a final merged
/// recipient or turn a local queue acknowledgement into a remote delivery claim.
async fn mutation(
    scenario: &Scenario,
    index: usize,
    expected: &mut History,
    command: impl Future<Output = Result<(), String>>,
) {
    let before = scenario.online(index).history().await;
    bounded("native chat mutation", command).await.unwrap();
    let after = scenario.online(index).history().await;
    let device = scenario.clients[index].identity.as_ref().unwrap().1;
    let authored: Vec<_> = after
        .operations
        .iter()
        .filter(|(hash, op)| op.author_device == device && !before.operations.contains_key(*hash))
        .collect();
    assert_eq!(authored.len(), 1, "one command must retain one original op");
    let (hash, op) = authored[0];
    assert!(op.verify());
    assert!(expected.operations.insert(*hash, op.clone()).is_none());
}

/// Union original operations and unrelated insertions, then apply the prescribed visible outcome.
/// In particular, importing C's older base cannot reinsert A's deleted row or undo B's causal edit.
/// This oracle uses explicit expected values, not another CRDT replay of the implementation.
fn merged(base: &History, branches: &[&History], deleted: &str, edited: &str) -> History {
    let mut expected = base.clone();
    for branch in branches {
        for (hash, op) in &branch.operations {
            if let Some(old) = expected.operations.insert(*hash, op.clone()) {
                assert_eq!(old, *op, "bridges must retain the original signed bytes");
            }
        }
        for (id, row) in &branch.messages {
            if !base.messages.contains_key(id) {
                assert!(expected.messages.insert(id.clone(), row.clone()).is_none());
            }
        }
    }
    assert!(expected.messages.remove(deleted).is_some());
    expected.messages.get_mut(edited).unwrap().1 = EDITED.into();
    expected
}

/// A complete signed log must also form the expected applied graph. Equal visible text and even
/// equal retained logs must not hide unapplied dependent changes. Derive heads from signed change
/// dependencies, independently of the native document's materialized state.
fn expected_heads(history: &History) -> BTreeSet<[u8; 32]> {
    let changes: Vec<_> = history
        .operations
        .values()
        .map(|op| Change::from_bytes(op.delta.clone()).unwrap())
        .collect();
    let all: BTreeSet<_> = changes.iter().map(|change| change.hash().0).collect();
    assert_eq!(
        all.len(),
        changes.len(),
        "each original op has a distinct change"
    );
    let mut heads = all.clone();
    for change in &changes {
        for dependency in change.deps() {
            assert!(
                all.contains(&dependency.0),
                "expected log must cover every dependency"
            );
            heads.remove(&dependency.0);
        }
    }
    heads
}

/// Read-only snapshot inspection. Restored MLS/document owners stay local to this observation;
/// neither is ever passed to a live actor, transport or catch-up scheduler. Secret snapshot bytes
/// retain their Zeroizing owner and are never printed, including when an assertion fails.
async fn observed_epoch_and_heads(scenario: &Scenario, index: usize) -> (u64, BTreeSet<[u8; 32]>) {
    let snapshot = zeroize::Zeroizing::new(
        bounded(
            "native graph observation",
            scenario.online(index).actor.snapshot(),
        )
        .await
        .unwrap(),
    );
    let mut root = Decoder::new(&snapshot);
    let (device, group) = catcoms_mls::restore_server(root.get_bytes().unwrap()).unwrap();
    let identity = scenario.clients[index].identity.as_ref().unwrap();
    assert_eq!(device.device_id(), identity.1);
    assert_eq!(group.group_id(), identity.0);
    let epoch = group.epoch();
    root.get_bytes().unwrap(); // routing secrets: borrowed, never copied or logged
    root.get_bytes().unwrap(); // invite ledger
    let count = root.get_u32().unwrap();
    assert!(count <= 256, "small fixture document bound");
    for _ in 0..count {
        let bytes = root.get_bytes().unwrap();
        let mut header = Decoder::new(bytes);
        if header.get_u16().unwrap() == DocType::Channel.tag()
            && header.get_u128().unwrap() == general()
        {
            let mut doc = EncryptedDoc::restore_for_actor(bytes, &identity.1).unwrap();
            return (epoch, doc.heads().into_iter().collect());
        }
    }
    panic!("native snapshot omitted the observed channel");
}

async fn assert_final_projection(
    scenario: &Scenario,
    indices: &[usize],
    expected: &History,
    epoch: u64,
    deleted: &str,
    edited: &str,
) {
    let heads = expected_heads(expected);
    let reactors: BTreeSet<_> = [1, 3, 4]
        .map(|index| fingerprint(&scenario.clients[index].identity.as_ref().unwrap().1))
        .into_iter()
        .collect();
    assert_eq!(reactors.len(), 3);
    for index in indices {
        assert_eq!(
            observed_epoch_and_heads(scenario, *index).await,
            (epoch, heads.clone())
        );
        let rows = bounded(
            "final chat metadata",
            scenario.online(*index).actor.messages(general()),
        )
        .await;
        assert!(!rows.iter().any(|row| row.id == deleted));
        let row = rows.iter().find(|row| row.id == edited).unwrap();
        assert_eq!(row.text, EDITED);
        assert!(row.edited > 0 && row.pinned);
        assert_eq!(row.reactions.len(), 1);
        assert_eq!(row.reactions[0].emoji, EMOJI);
        assert_eq!(
            row.reactions[0].by.iter().cloned().collect::<BTreeSet<_>>(),
            reactors
        );
    }
}

async fn three_bubbles(admission: FixtureAdmission) {
    let mut scenario = Scenario::new(true, admission);
    scenario.admit().await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    // Admission already submits these permitted dials; wait for actual native capture of the
    // extra edges this schedule needs. No restore-time dial or fabricated route is permitted.
    bounded("extra bubble bridge routes are sealed", async {
        loop {
            let mut complete = true;
            for (source, target) in [(4, 2), (3, 1)] {
                let net = network(&scenario.online(source).state).await;
                complete &= net.reconnect_routes.iter().any(|route| {
                    route.peer_id == *scenario.rules.peers[target].as_bytes()
                        && route.address == scenario.address(target).to_string()
                });
            }
            if complete {
                break;
            }
            SystemClock.sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    let mut common = History::new();
    let originals = scenario.author(ALL, "shared base", &mut common).await;
    let deleted = originals[2].clone();
    let edited = scenario
        .author(&[2], "retained edit target", &mut common)
        .await
        .remove(0);
    scenario
        .converge(ALL, &common, "shared base before three bubbles")
        .await;
    let old_epoch = observed_epoch_and_heads(&scenario, 0).await.0;
    for index in ALL {
        assert_eq!(
            observed_epoch_and_heads(&scenario, *index).await.0,
            old_epoch
        );
    }
    scenario.close(ALL).await;

    // C's later-removed author posts before the actual removal, while all future recipients are
    // offline. This proves missing historical-author ingestion after removal, not just retention
    // of an operation every recipient already held. Member5 never authors after removal.
    scenario.reopen_in(&[3, 5], false).await;
    let mut c = common.clone();
    let c_ids = scenario.author(&[3, 5], "C before removal", &mut c).await;
    mutation(
        &scenario,
        3,
        &mut c,
        scenario
            .online(3)
            .actor
            .toggle_reaction(general(), edited.clone(), EMOJI.into()),
    )
    .await;
    scenario
        .converge(&[3, 5], &c, "isolated C keeps its originals")
        .await;
    scenario.close(&[3, 5]).await;

    scenario.reopen_in(&[0, 1], false).await;
    assert_eq!(scenario.online(0).history().await, common);
    let mut a = common.clone();
    mutation(
        &scenario,
        0,
        &mut a,
        scenario
            .online(0)
            .actor
            .delete_message(general(), deleted.clone()),
    )
    .await;
    a.messages.remove(&deleted).unwrap();
    mutation(
        &scenario,
        0,
        &mut a,
        scenario
            .online(0)
            .actor
            .set_pin(general(), edited.clone(), true),
    )
    .await;
    mutation(
        &scenario,
        1,
        &mut a,
        scenario
            .online(1)
            .actor
            .toggle_reaction(general(), edited.clone(), EMOJI.into()),
    )
    .await;
    let removed = scenario.clients[5].identity.as_ref().unwrap().1;
    bounded(
        "native owner removal between bubbles",
        scenario
            .online(0)
            .actor
            .remove_member(fingerprint(&removed)),
    )
    .await
    .unwrap();
    scenario.converge_members(&[0, 1], SURVIVORS).await;
    assert_eq!(
        observed_epoch_and_heads(&scenario, 0).await.0,
        old_epoch + 1
    );
    scenario.author(&[0, 1], "A after removal", &mut a).await;
    scenario
        .converge(&[0, 1], &a, "isolated A delete and pin")
        .await;
    assert!(!a.messages.contains_key(&c_ids[1]));
    scenario.close(&[0, 1]).await;

    scenario.reopen_in(&[2, 4], false).await;
    assert_eq!(scenario.online(2).history().await, common);
    assert_eq!(observed_epoch_and_heads(&scenario, 2).await.0, old_epoch);
    let mut b = common.clone();
    mutation(
        &scenario,
        2,
        &mut b,
        scenario.online(2).actor.edit_message(
            general(),
            deleted.clone(),
            "concurrent edit hidden by deletion".into(),
        ),
    )
    .await;
    b.messages.get_mut(&deleted).unwrap().1 = "concurrent edit hidden by deletion".into();
    mutation(
        &scenario,
        2,
        &mut b,
        scenario
            .online(2)
            .actor
            .edit_message(general(), edited.clone(), EDITED.into()),
    )
    .await;
    b.messages.get_mut(&edited).unwrap().1 = EDITED.into();
    mutation(
        &scenario,
        4,
        &mut b,
        scenario
            .online(4)
            .actor
            .toggle_reaction(general(), edited.clone(), EMOJI.into()),
    )
    .await;
    scenario
        .author(&[2, 4], "B before learning removal", &mut b)
        .await;
    scenario
        .converge(&[2, 4], &b, "isolated B edits and additions")
        .await;
    scenario.close(&[2, 4]).await;

    // Causal independence is a signed-operation property, not a claim based on wall-clock order.
    for (left, right) in [(&a, &b), (&b, &c), (&c, &a)] {
        // Each isolated log must cover its own dependencies. Disjoint operation sets alone
        // would miss an accidental dependency on an operation learned from another bubble.
        expected_heads(left);
        expected_heads(right);
        assert!(left
            .operations
            .keys()
            .filter(|hash| !common.operations.contains_key(*hash))
            .all(|hash| !right.operations.contains_key(hash)));
    }
    let ab = merged(&common, &[&a, &b], &deleted, &edited);
    let expected = merged(&common, &[&a, &b, &c], &deleted, &edited);
    expected_heads(&expected); // fail before reconnect if the independently assembled oracle has gaps

    scenario.reopen_in(&[0, 2], false).await;
    scenario.converge_members(&[0, 2], SURVIVORS).await;
    scenario
        .converge(
            &[0, 2],
            &ab,
            "A and B automatically merge edit versus deletion",
        )
        .await;
    assert!(!scenario
        .online(0)
        .history()
        .await
        .messages
        .contains_key(&c_ids[1]));
    scenario.close(&[2]).await;
    scenario.reopen_in(&[3], false).await;
    scenario.converge_members(&[0, 3], SURVIVORS).await;
    scenario
        .converge(
            &[0, 3],
            &expected,
            "C re-encrypts an original removed-author operation",
        )
        .await;
    assert_final_projection(
        &scenario,
        &[0, 3],
        &expected,
        old_epoch + 1,
        &deleted,
        &edited,
    )
    .await;
    scenario.close(&[0]).await;

    // 3 is now the sole online holder of the full union. A/B's original authors0/2 remain offline;
    // 3 authored C content itself, so this is not a claim that every original author is absent.
    scenario.reopen_in(&[1, 4], false).await;
    scenario.converge_members(&[1, 3, 4], SURVIVORS).await;
    scenario
        .converge(
            &[1, 3, 4],
            &expected,
            "replacement bridge serves hidden history onward",
        )
        .await;
    assert_final_projection(
        &scenario,
        &[1, 3, 4],
        &expected,
        old_epoch + 1,
        &deleted,
        &edited,
    )
    .await;
    scenario.close(&[1, 3, 4]).await;
    for index in [1, 3, 4] {
        scenario.reopen_in(&[index], false).await;
        scenario.converge_members(&[index], SURVIVORS).await;
        assert_eq!(scenario.online(index).history().await, expected);
        assert_final_projection(
            &scenario,
            &[index],
            &expected,
            old_epoch + 1,
            &deleted,
            &edited,
        )
        .await;
        scenario.close(&[index]).await;
    }
    scenario.rules.assert_graph();
    assert!(scenario.rules.trace.lock().unwrap().dropped_gossip > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_chat_bubbles_merge_edits_deletes_and_removed_author_history_through_a_replacement_bridge(
) {
    let admission = isolate().await;
    tokio::select! {
        _ = three_bubbles(admission) => {},
        _ = SystemClock.sleep(Duration::from_secs(600)) => panic!("three-bubble chat acceptance exceeded its total bound"),
    }
}
