//! `StudioRequest::ApplyBatch`: several Score edits in one transaction. The batch exists so the
//! composer's autosave costs one server snapshot, one scan and one publication window instead of
//! one of each per placed note; these tests pin what that must not change: every item is its own
//! signed operation, an exact retry reseals rather than re-applies, a refused item names itself
//! and leaves the items before it saved, and the static bounds hold before the actor queue.

use super::*;
use catcoms_replication::studio::{ScoreCell, ScoreHeader, ScoreOp, StudioPatch};

fn score_target() -> StudioTarget {
    StudioTarget::Score {
        channel: channel(),
        object: [9; 16],
    }
}

fn recipe() -> StudioPatch {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-score-ops-v1.json"
    )))
    .unwrap();
    let body = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "set_patch")
        .unwrap()["body"]
        .as_str()
        .unwrap();
    match ScoreOp::decode(body.as_bytes()).unwrap() {
        ScoreOp::SetPatch { descriptor, .. } => descriptor,
        _ => unreachable!(),
    }
}

fn operation(nonce: u8, op: ScoreOp) -> StudioBatchOperation {
    StudioBatchOperation {
        nonce: [nonce; 16],
        body: op.encode().unwrap(),
    }
}

fn note(step: u16, row: u8, len: u16) -> ScoreOp {
    ScoreOp::SetCell {
        step,
        row,
        value: ScoreCell::Note {
            patch: recipe().id(),
            len: Some(len),
            volume: None,
        },
    }
}

struct Fixture {
    store: ServerStore,
    server: Server<catcoms_rt::MemNetwork, ChaCha20Rng>,
    epoch_id: u128,
    _dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(dir.path(), b"studio-batch-test", &mut rng()).unwrap();
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(123)),
        "alice",
    )
    .unwrap();
    let created = server
        .studio_transaction(
            &mut store,
            7,
            StudioRequest::CreateScore {
                channel: channel(),
                object: [9; 16],
                nonce: [10; 16],
                title: "soundtrack".into(),
                ts: 123,
            },
        )
        .unwrap()
        .unwrap();
    Fixture {
        store,
        server,
        epoch_id: created.epoch_id,
        _dir: dir,
    }
}

fn score(view: &StudioView) -> &catcoms_replication::studio::ScoreProjection {
    match &view.projection {
        StudioProjection::Score(p) => p,
        _ => panic!("a score view"),
    }
}

#[test]
fn batched_score_apply_saves_every_item_in_one_visit_and_an_exact_retry_reseals() {
    let mut f = fixture();
    let batch = || StudioRequest::ApplyBatch {
        target: score_target(),
        epoch_id: f.epoch_id,
        operations: vec![
            operation(
                1,
                ScoreOp::SetPatch {
                    patch: recipe().id(),
                    descriptor: recipe(),
                },
            ),
            operation(2, note(0, 0, 2)),
            operation(3, note(4, 0, 4)),
            operation(4, ScoreOp::SetHeader(ScoreHeader::Bpm(100))),
        ],
    };
    let saved = f
        .server
        .studio_transaction_with_publication(&mut f.store, 7, batch())
        .unwrap();
    assert_eq!(saved.packets.len(), 4, "one sealed operation per item");
    assert_eq!(
        saved.observed,
        vec![(score_target(), f.epoch_id)],
        "one watch for the one document the batch wrote"
    );
    let view = saved.view.as_ref().unwrap();
    let p = score(view);
    assert_eq!(p.cells[&(0, 0)].value(), Some(&note_cell(2)));
    assert_eq!(p.cells[&(4, 0)].value(), Some(&note_cell(4)));
    assert_eq!(p.bpm.as_ref().unwrap().selected.value, 100);
    assert!(p.readiness().unwrap().ready());

    // The same bytes again: every item is already held, so each reseals and nothing doubles.
    let again = f
        .server
        .studio_transaction_with_publication(&mut f.store, 7, batch())
        .unwrap();
    assert_eq!(again.packets.len(), 4);
    let p = score(again.view.as_ref().unwrap());
    assert!(p.cells[&(0, 0)].register.conflicts.is_empty());
    assert_eq!(p.cells.len(), 2);
    assert_eq!(p.patches.len(), 1);
}

fn note_cell(len: u16) -> ScoreCell {
    ScoreCell::Note {
        patch: recipe().id(),
        len: Some(len),
        volume: None,
    }
}

#[test]
fn a_refused_item_names_itself_and_the_items_before_it_stay_saved() {
    let mut f = fixture();
    // Item 2 passes the static grammar (step 20 is a legal address) but fails causal admission:
    // it ends past the score's effective 16 steps. Item 1 (the recipe) and nothing after it
    // are what the vault holds afterwards.
    let error = match f.server.studio_transaction_with_publication(
        &mut f.store,
        7,
        StudioRequest::ApplyBatch {
            target: score_target(),
            epoch_id: f.epoch_id,
            operations: vec![
                operation(
                    1,
                    ScoreOp::SetPatch {
                        patch: recipe().id(),
                        descriptor: recipe(),
                    },
                ),
                operation(2, note(20, 0, 1)),
                operation(3, note(0, 0, 1)),
            ],
        },
    ) {
        Ok(_) => panic!("a batch with an item past the score's end must be refused"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("item 2 of 3"), "{error}");
    assert!(error.contains("retry the same batch exactly"), "{error}");
    let view = f
        .server
        .studio_transaction(
            &mut f.store,
            7,
            StudioRequest::Read {
                target: score_target(),
            },
        )
        .unwrap()
        .unwrap();
    let p = score(&view);
    assert_eq!(p.patches.len(), 1, "the item before the refusal is durable");
    assert!(p.cells.is_empty(), "nothing after the refusal was applied");
}

#[test]
fn batched_apply_bounds_are_checked_before_the_actor_queue() {
    let item = |nonce: u8| operation(nonce, note(0, 0, 1));
    let request =
        |target: StudioTarget, operations: Vec<StudioBatchOperation>| StudioRequest::ApplyBatch {
            target,
            epoch_id: 0,
            operations,
        };
    assert!(request(score_target(), vec![]).validate().is_err());
    assert!(request(score_target(), (0..65).map(item).collect())
        .validate()
        .is_err());
    assert!(request(score_target(), (0..64).map(item).collect())
        .validate()
        .is_ok());
    assert!(request(score_target(), vec![item(1), item(1)])
        .validate()
        .is_err());
    assert!(
        request(target(), vec![item(1)]).validate().is_err(),
        "flipnote bodies need per-request admission"
    );
    assert!(
        request(StudioTarget::Index { channel: channel() }, vec![item(1)])
            .validate()
            .is_err()
    );
    let big_title = ScoreOp::SetHeader(ScoreHeader::Title("t".repeat(60 * 1024)));
    assert!(
        request(score_target(), vec![operation(1, big_title.clone())])
            .validate()
            .is_ok()
    );
    assert!(request(
        score_target(),
        (1..=5).map(|n| operation(n, big_title.clone())).collect()
    )
    .validate()
    .is_err());
    assert!(request(
        score_target(),
        vec![StudioBatchOperation {
            nonce: [1; 16],
            body: b"{\"op\":\"insert_frame\"}".to_vec(),
        }]
    )
    .validate()
    .is_err());
}
