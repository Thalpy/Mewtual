use super::*;

#[test]
fn studio_overlay_handoff_requires_live_tenure_and_pristine_installed_source() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, expected) = prepare(&f, &mut store);
        let original = canonical(&store);
        for tenure in [None, Some(f.group.epoch() + 1)] {
            let mut b = budget(&mut store, &f);
            assert!(store
                .handoff_studio_overlay(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    basis,
                    tenure,
                    &mut rng(),
                    &mut b
                )
                .is_err());
            assert_eq!(canonical(&store), original);
            assert_eq!(
                store
                    .load_epoch_intents(SERVER, &f.logical)
                    .unwrap()
                    .local_draft()
                    .unwrap()
                    .unwrap()
                    .projection(),
                &expected
            );
        }
        let source = f.load(&store).unwrap();
        let mut op = f.title();
        op.nonce = [88; 16];
        let mut b = budget(&mut store, &f);
        store
            .edit_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                source.doc_id(),
                &f.device,
                op,
                123,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        let original = canonical(&store);
        let mut b = budget(&mut store, &f);
        assert!(store
            .handoff_studio_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                basis,
                Some(0),
                &mut rng(),
                &mut b
            )
            .is_err());
        assert_eq!(
            canonical(&store),
            original,
            "extra signed history was overwritten"
        );
        assert!(store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .overlay()
            .is_some());
    }
}

#[test]
fn studio_overlay_handoff_index_put_requires_the_actual_flipnote_source() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let object = [15; 16];
    let op = f.domain(
        IndexOp::PutObject {
            object,
            kind: StudioKind::Flipnote,
            title: "saved branch".into(),
            created_by: f.device.device_id(),
            ts: 123,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
        55,
    );
    let expected = save(&f, &mut store, &close, basis.fingerprint(), op, 123)
        .projection()
        .clone();
    install(&f, &mut store, &close);
    let original = canonical(&store);
    let mut b = budget(&mut store, &f);
    let error = store
        .handoff_studio_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis.fingerprint(),
            Some(0),
            &mut rng(),
            &mut b,
        )
        .unwrap_err();
    assert!(error.to_string().contains("unavailable Flipnote"));
    assert_eq!(canonical(&store), original);
    let target = StudioTarget::Flipnote {
        channel: f.target.channel(),
        object,
    };
    let logical = target.document(&f.group.group_id()).unwrap();
    let op = DomainOp {
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
        nonce: [55; 16],
        body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("saved branch".into()))
            .encode()
            .unwrap(),
    };
    let mut b = budget(&mut store, &f);
    store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            target,
            epoch_zero_id(logical.doc_type, &logical.logical_key),
            &f.device,
            op,
            123,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    transfer(&f, &mut store, basis.fingerprint());
    assert_eq!(f.load(&store).unwrap().projection().unwrap(), expected);
}

#[test]
fn studio_overlay_handoff_replays_dependency_order_and_retains_all_pixel_references() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let (insert_cid, insert_bytes) = published_pix(&store, &f, 0x41);
    let (replace_cid, replace_bytes) = published_pix(&store, &f, 0x42);
    let (second_cid, second_bytes) = published_pix(&store, &f, 0x43);
    let bodies = [
        FlipnoteOp::InsertFrame {
            frame: [2; 16],
            after: Some([1; 16]),
            cid: insert_cid,
            bytes: insert_bytes,
        },
        FlipnoteOp::ReplaceFrame {
            frame: [2; 16],
            cid: replace_cid,
            bytes: replace_bytes,
        },
        FlipnoteOp::InsertFrame {
            frame: [6; 16],
            after: Some([2; 16]),
            cid: second_cid,
            bytes: second_bytes,
        },
        FlipnoteOp::RemoveFrame { frame: [2; 16] },
    ];
    let mut nonces: Vec<_> = (100u128..116)
        .map(|n| {
            let mut op = f.title();
            op.nonce = n.to_be_bytes();
            (op.id(&f.device.device_id()), op.nonce)
        })
        .collect();
    nonces.sort_by(|a, b| b.0.cmp(&a.0));
    let mut accepted = Vec::new();
    for (i, body) in bodies.into_iter().enumerate() {
        let mut op = f.domain(body.encode().unwrap(), 0);
        op.nonce = nonces[i].1;
        save(
            &f,
            &mut store,
            &close,
            basis.fingerprint(),
            op.clone(),
            200 + i as u64,
        );
        accepted.push((op, 200 + i as u64));
    }
    let mut independent = install(&f, &mut store, &close).unit;
    for (op, ts) in &accepted {
        independent
            .edit_or_reseal(&f.device, &f.group, &mut rng(), op, *ts)
            .unwrap();
    }
    let expected = independent.projection().unwrap();
    let mut b = budget(&mut store, &f);
    store
        .handoff_studio_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis.fingerprint(),
            Some(0),
            &mut rng(),
            &mut b,
        )
        .expect("handoff lost accepted operation order");
    assert_eq!(f.load(&store).unwrap().op_count(), 4);
    assert_eq!(
        f.load(&store).unwrap().projection().unwrap(),
        expected,
        "handoff reordered the accepted branch"
    );
    drop(store);
    let mut store = open(root.path());
    let pins = store.creative_pinned_cids().unwrap();
    for cid in [[3; 32], insert_cid, replace_cid, second_cid] {
        assert!(
            pins.for_group(&f.group.group_id())
                .any(|actual| *actual == catcoms_storage::Cid::from_bytes(cid)),
            "handoff lost base/superseded/removed pixel reference"
        );
    }
}

#[test]
fn studio_overlay_handoff_old_owner_receipt_refuses_even_when_original_author_is_current() {
    let root = tempfile::tempdir().unwrap();
    let device = MlsDevice::generate().unwrap();
    // Same signing identity, fresh provider captured before the original group exists.
    let rejoining = device.duplicate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let target = target(true);
    let logical = target.document(&group.group_id()).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let mut f = Fixture {
        device,
        group,
        target,
        logical,
        id,
    };
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    let next = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.device, next.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut group = ServerGroup::join(&next, &welcome).unwrap();
    group.remove_member(&next, &f.device.device_id()).unwrap();
    // Occupy the vacated first leaf before re-admitting the author. Otherwise the original
    // identity would become committer again and the old-owner rejection would not be isolated.
    let current = MlsDevice::generate().unwrap();
    let welcome = group
        .add_member(&next, current.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut group = ServerGroup::join(&current, &welcome).unwrap();
    let tenure = group.epoch();
    let welcome = group
        .add_member(&current, rejoining.key_package().unwrap())
        .unwrap()
        .welcome;
    f.device = rejoining;
    f.group = ServerGroup::join(&f.device, &welcome).unwrap();
    assert_eq!(f.group.designated_committer(), Some(current.device_id()));
    assert_eq!(
        f.group
            .member_signature_key(&f.device.device_id())
            .as_deref(),
        Some(f.device.public_key_bytes().as_slice())
    );
    let source = f.load(&store).unwrap();
    assert_eq!(
        (source.epoch(), source.op_count(), source.phase()),
        (1, 0, EpochPhase::Open)
    );
    let receipt = source.unit.receipt_head().unwrap().unwrap();
    let expected = receipt
        .verify_current_owner(&f.group, tenure)
        .unwrap_err()
        .to_string();
    let original = canonical(&store);
    let mut b = budget(&mut store, &f);
    let result = store.handoff_studio_overlay(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        basis,
        Some(tenure),
        &mut rng(),
        &mut b,
    );
    assert!(
        matches!(result,Err(AppError::Invalid(ref s)) if s==&format!("epoch studio: {expected}")),
        "old owner receipt authorized handoff: {result:?}"
    );
    assert_eq!(canonical(&store), original);
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .overlay()
        .is_some());
}
