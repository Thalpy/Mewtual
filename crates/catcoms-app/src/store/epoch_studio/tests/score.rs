use super::*;
use catcoms_replication::studio::{ScoreCell, ScoreHeader, ScoreOp};

fn score_fixture() -> Fixture {
    let mut f = Fixture::new(true);
    f.target = StudioTarget::Score {
        channel: [7; 16],
        object: [9; 16],
    };
    f.logical = f.target.document(&f.group.group_id()).unwrap();
    f.id = epoch_zero_id(f.logical.doc_type, &f.logical.logical_key);
    f
}

#[test]
fn score_wrapper_extension_is_explicit_ordered_and_cannot_retag_an_index() {
    let f = score_fixture();
    for (suffix, want_score, linked) in [
        (&[][..], false, false),
        (&[1][..], false, true),
        (&[2][..], true, false),
        (&[1, 2][..], true, true),
    ] {
        let scope = scope_bytes(SERVER, &f.logical).unwrap();
        let mut e = Encoder::new();
        e.put_bytes(&scope).unwrap();
        e.put_bytes(&f.target.channel()).unwrap();
        e.put_bytes(b"opaque snapshot").unwrap();
        let mut bytes = e.finish();
        bytes.extend_from_slice(suffix);
        let (target, _, link) = decode_record_link(&bytes, &scope, &f.logical).unwrap();
        assert_eq!(matches!(target, StudioTarget::Score { .. }), want_score);
        assert_eq!(link, linked);
        for tail in [&[1][..], &[2][..], &[3][..]] {
            let mut bad = bytes.clone();
            bad.extend_from_slice(tail);
            // Only the canonical legacy absent->1 and legacy optional1->score2 transitions
            // are valid; duplicate, reversed or unknown extensions cannot be ignored.
            if suffix.is_empty() && (tail == [1] || tail == [2]) || suffix == [1] && tail == [2] {
                continue;
            }
            assert!(decode_record_link(&bad, &scope, &f.logical).is_err());
        }
    }
    let logical = target(false).document(&f.group.group_id()).unwrap();
    let scope = scope_bytes(SERVER, &logical).unwrap();
    let mut e = Encoder::new();
    e.put_bytes(&scope).unwrap();
    e.put_bytes(&[7; 16]).unwrap();
    e.put_bytes(b"opaque snapshot").unwrap();
    e.put_u8(2);
    assert!(decode_record_link(&e.finish(), &scope, &logical).is_err());
}

#[test]
fn score_vault_save_reopen_and_exact_retry_preserve_family_and_clear() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let f = score_fixture();
    let title = f.domain(
        ScoreOp::SetHeader(ScoreHeader::Title("score cat".into()))
            .encode()
            .unwrap(),
        1,
    );
    let set = f.domain(
        ScoreOp::SetCell {
            step: 0,
            row: 0,
            value: ScoreCell::Drum { pad: 0 },
        }
        .encode()
        .unwrap(),
        2,
    );
    let clear = f.domain(ScoreOp::ClearCell { step: 0, row: 0 }.encode().unwrap(), 3);
    let mut first = None;
    for operation in [title, set.clone(), clear] {
        let mut b = budget(&mut store, &f);
        let (sealed, _) = store
            .edit_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                f.id,
                &f.device,
                operation.clone(),
                100,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        if operation == set {
            first = Some(sealed);
        }
    }
    let held = store
        .load_studio_epoch(SERVER, &f.group, f.target, &f.device)
        .unwrap()
        .unwrap();
    let StudioProjection::Score(p) = held.projection().unwrap() else {
        panic!()
    };
    assert!(p.cells[&(0, 0)].removed());
    let scope = scope_bytes(SERVER, &f.logical).unwrap();
    let bytes = store.read_studio_record(&scope).unwrap().unwrap();
    let (decoded, snapshot, _) = decode_record_link(&bytes.plain, &scope, &f.logical).unwrap();
    assert_eq!(decoded, f.target);
    assert_eq!(bytes.plain.last(), Some(&2));
    assert!(StudioEpoch::validate_vault_snapshot(snapshot, &f.group.group_id(), f.target).is_ok());
    // Both wrapper and inner restart family independently bind a pristine/nonempty score.
    let mut stripped = bytes.plain.to_vec();
    stripped.pop();
    let (art, inner, _) = decode_record_link(&stripped, &scope, &f.logical).unwrap();
    assert!(matches!(art, StudioTarget::Flipnote { .. }));
    assert!(StudioEpoch::validate_vault_snapshot(inner, &f.group.group_id(), art).is_err());
    assert!(store
        .load_studio_epoch(SERVER, &f.group, target(true), &f.device)
        .is_err());
    let wrong = StudioTarget::Score {
        channel: [8; 16],
        object: [9; 16],
    };
    assert!(store
        .load_studio_epoch(SERVER, &f.group, wrong, &f.device)
        .is_err());
    assert!(
        store.creative_pinned_cids().unwrap().is_empty(),
        "well-formed scores need no blob CIDs, not an Unknown scan"
    );
    drop(held);
    drop(store);
    let mut store = open(dir.path());
    let mut b = budget(&mut store, &f);
    let (again, state) = store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            f.id,
            &f.device,
            set,
            999,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    let first = first.unwrap();
    let secret = f
        .group
        .channel_secret(&f.device, first.doc_type, first.doc_id)
        .unwrap();
    assert_eq!(
        first.open(&secret).unwrap().encode(),
        again.open(&secret).unwrap().encode()
    );
    let StudioProjection::Score(p) = state.projection().unwrap() else {
        panic!()
    };
    assert!(p.cells[&(0, 0)].removed());
}

#[test]
fn score_wrong_family_and_malformed_operations_refuse_before_intent_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = open(dir.path());
    let f = score_fixture();
    let title = f.domain(
        ScoreOp::SetHeader(ScoreHeader::Title("score".into()))
            .encode()
            .unwrap(),
        1,
    );
    let mut b = budget(&mut store, &f);
    store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            f.id,
            &f.device,
            title,
            100,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    let before = store
        .load_epoch_intents_structural(SERVER, &f.logical)
        .unwrap()
        .encode(b"score-unchanged-intents")
        .unwrap();
    let art = f.domain(
        FlipnoteOp::SetHeader(FlipnoteHeader::Title("art".into()))
            .encode()
            .unwrap(),
        2,
    );
    let mut b = budget(&mut store, &f);
    assert!(store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            target(true),
            f.id,
            &f.device,
            art,
            100,
            &mut rng(),
            &mut b
        )
        .is_err());
    let invalid = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [1; 16],
            after: None,
            cid: [2; 32],
            bytes: 10,
        }
        .encode()
        .unwrap(),
        3,
    );
    let mut b = budget(&mut store, &f);
    assert!(store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            f.id,
            &f.device,
            invalid,
            100,
            &mut rng(),
            &mut b
        )
        .is_err());
    assert_eq!(
        store
            .load_epoch_intents_structural(SERVER, &f.logical)
            .unwrap()
            .encode(b"score-unchanged-intents")
            .unwrap(),
        before
    );
}

#[test]
fn score_open_source_survives_index_refresh_without_retagging_or_cold_restore() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = score_fixture();
    let mut b = budget(&mut store, &f);
    let (_, state) = f.edit(
        &mut store,
        &mut b,
        f.domain(
            ScoreOp::SetHeader(ScoreHeader::Title("opened score".into()))
                .encode()
                .unwrap(),
            1,
        ),
    );
    store.retain_studio_source(&f.group, &f.device, state);
    assert!(store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));

    // The numeric/physical object namespace is shared. Opposite-family labels cannot borrow
    // the warm Score graph or satisfy a pointer, even though their logical keys are identical.
    let art = target(true);
    assert_eq!(f.logical, art.document(&f.group.group_id()).unwrap());
    assert!(!store.studio_source_is_warm(SERVER, &f.group, art, &f.device));
    assert!(!store
        .studio_object_holds_work(SERVER, &f.group, art)
        .unwrap());
    assert!(store
        .studio_object_holds_work(SERVER, &f.group, f.target)
        .unwrap());
    let obligations = [
        crate::studio::object_target([7; 16], [9; 16], StudioKind::Flipnote),
        crate::studio::object_target([7; 16], [9; 16], StudioKind::Score),
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        obligations.len(),
        2,
        "conflicting same-id claims must retain both family obligations"
    );

    let index = target(false);
    let document = index.document(&f.group.group_id()).unwrap();
    let body = IndexOp::PutObject {
        object: [9; 16],
        kind: StudioKind::Score,
        title: "opened score".into(),
        created_by: f.device.device_id(),
        ts: 100,
        expiry: StudioExpiry::Never,
    }
    .encode()
    .unwrap();
    let operation = DomainOp {
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        nonce: [2; 16],
        body,
    };
    let mut b = budget(&mut store, &f);
    store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            index,
            epoch_zero_id(document.doc_type, &document.logical_key),
            &f.device,
            operation,
            100,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    let before = crate::store::studio_full_restores_for_test();
    assert_eq!(
        store
            .with_studio_source(SERVER, &f.group, index, &f.device, |s| Ok(s.op_count()))
            .unwrap(),
        Some(1)
    );
    assert!(store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
    let after_index = crate::store::studio_full_restores_for_test();
    assert!(after_index >= before);
    assert_eq!(
        store
            .with_studio_source(SERVER, &f.group, f.target, &f.device, |s| Ok(s.op_count()))
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        crate::store::studio_full_restores_for_test(),
        after_index,
        "Index refresh evicted the Score graph"
    );
    assert!(store
        .load_studio_epoch(SERVER, &f.group, art, &f.device)
        .is_err());
}
