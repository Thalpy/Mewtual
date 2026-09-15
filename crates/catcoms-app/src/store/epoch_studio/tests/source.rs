use super::*;

fn warm(store: &mut ServerStore, f: &Fixture) {
    let state = f.load(store).unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
}
fn incoming(f: &Fixture, store: &ServerStore, n: u8) -> SealedOp {
    let mut state = f.load(store).unwrap();
    let mut op = f.title();
    op.nonce = [n; 16];
    state
        .unit
        .edit_or_reseal(&f.device, &f.group, &mut rng(), &op, 100)
        .unwrap()
}
fn receive(
    store: &mut ServerStore,
    f: &Fixture,
    op: &SealedOp,
    b: &mut EpochStudioBudget,
) -> Result<(Admission, EpochStudioState), AppError> {
    store.ingest_studio_epoch_reusing(SERVER, &f.group, f.target, &f.device, op, &mut rng(), b)
}

#[test]
fn studio_source_read_reuses_exact_bytes_but_rebuilds_changed_bytes() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    warm(&mut store, &f);
    let before = crate::store::studio_full_restores_for_test();
    let read = |s: &mut ServerStore| {
        s.with_studio_source(SERVER, &f.group, f.target, &f.device, |state| {
            Ok(state.op_count())
        })
    };
    assert_eq!(read(&mut store).unwrap(), Some(1));
    assert_eq!(crate::store::studio_full_restores_for_test(), before);
    f.edit(&mut store, &mut b, f.title());
    let before = crate::store::studio_full_restores_for_test();
    assert_eq!(read(&mut store).unwrap(), Some(2));
    assert_eq!(crate::store::studio_full_restores_for_test(), before + 1);
    assert_eq!(read(&mut store).unwrap(), Some(2));
    assert_eq!(crate::store::studio_full_restores_for_test(), before + 1);
    let mut damaged = fs::read(f.path(&store)).unwrap();
    *damaged.last_mut().unwrap() ^= 1;
    fs::write(f.path(&store), damaged).unwrap();
    assert!(
        read(&mut store).is_err(),
        "a view cannot fall back to cached damaged content"
    );
    assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
}

#[test]
fn studio_source_cold_other_target_receive_does_not_evict_the_opened_source() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let art = Fixture::new(true);
    let index = Fixture::new(false);
    let mut b = budget(&mut store, &art);
    f_write(&mut store, &art, &mut b);
    let mut b = budget(&mut store, &index);
    let (_, state) = index.edit(&mut store, &mut b, index.insert());
    store.retain_received_studio_source(&index.group, &index.device, state);
    assert!(store.studio_source_is_warm(SERVER, &art.group, art.target, &art.device));
    assert!(!store.studio_source_is_warm(SERVER, &index.group, index.target, &index.device));
    warm(&mut store, &index); // Direct ownership transfer still replaces the sole slot.
    assert!(!store.studio_source_is_warm(SERVER, &art.group, art.target, &art.device));
    assert!(store.studio_source_is_warm(SERVER, &index.group, index.target, &index.device));
}
fn f_write(store: &mut ServerStore, f: &Fixture, b: &mut EpochStudioBudget) {
    f.edit(store, b, f.insert());
    warm(store, f);
}

#[test]
fn studio_source_reuse_two_edits_duplicate_and_restart_keep_exact_durable_state() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    for n in 2..=3 {
        let op = incoming(&f, &store, n);
        warm(&mut store, &f);
        let before = crate::store::studio_full_restores_for_test();
        let (admission, state) = receive(&mut store, &f, &op, &mut b).unwrap();
        assert_eq!(admission, Admission::Accepted);
        assert_eq!(state.op_count(), n as usize);
        store.retain_studio_source(&f.group, &f.device, state);
        let bytes = fs::read(f.path(&store)).unwrap();
        let (admission, state) = receive(&mut store, &f, &op, &mut b).unwrap();
        assert_eq!(admission, Admission::Duplicate);
        store.retain_studio_source(&f.group, &f.device, state);
        // A second exact duplicate proves that the unchanged flush kept the physical stamp.
        let (admission, state) = receive(&mut store, &f, &op, &mut b).unwrap();
        assert_eq!(admission, Admission::Duplicate);
        store.retain_studio_source(&f.group, &f.device, state);
        assert_eq!(fs::read(f.path(&store)).unwrap(), bytes);
        assert_eq!(crate::store::studio_full_restores_for_test(), before);
    }
    drop(store);
    let mut store = open(root.path());
    assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
    let op = incoming(&f, &store, 4);
    let mut b = budget(&mut store, &f);
    let (admission, state) = receive(&mut store, &f, &op, &mut b).unwrap();
    assert_eq!(admission, Admission::Accepted);
    assert_eq!(state.op_count(), 4);
}

#[test]
fn studio_source_reuse_rejects_damage_deletion_and_gate_only_replacement() {
    for change in 0..3 {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let f = Fixture::new(true);
        let mut b = budget(&mut store, &f);
        f.edit(&mut store, &mut b, f.insert());
        let op = incoming(&f, &store, 2);
        warm(&mut store, &f);
        let path = f.path(&store);
        let original = fs::read(&path).unwrap();
        match change {
            0 => {
                let mut changed = original.clone();
                *changed.last_mut().unwrap() ^= 1;
                fs::write(&path, changed).unwrap();
            }
            1 => fs::remove_file(&path).unwrap(),
            _ => {
                let state = f.load(&store).unwrap();
                store
                    .seal_studio_epoch(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        f.receipt(&state, 7),
                        0,
                        &mut rng(),
                        &mut b,
                    )
                    .unwrap();
                warm(&mut store, &f);
                let closing = fs::read(&path).unwrap();
                // Construct a different valid same-size book/gate wrapper from the same open
                // history. Heads, file length and footprint cannot substitute for full digest.
                fs::write(&path, &original).unwrap();
                b = budget(&mut store, &f);
                store
                    .seal_studio_epoch(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        f.receipt(&state, 8),
                        0,
                        &mut rng(),
                        &mut b,
                    )
                    .unwrap();
                let replaced = fs::read(&path).unwrap();
                assert_eq!(closing.len(), replaced.len());
                assert_ne!(closing, replaced);
            }
        }
        assert!(receive(&mut store, &f, &op, &mut b).is_err());
        assert!(b.requires_reconciliation());
        assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
        if change == 2 {
            assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
        }
    }
}

#[test]
fn studio_source_reuse_is_bound_to_mount_server_group_channel_device_and_mls() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut f = Fixture::new(true);
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    warm(&mut store, &f);
    let other = MlsDevice::generate().unwrap();
    let other_group = ServerGroup::create(&other).unwrap();
    assert!(!store.studio_source_is_warm(SERVER + 1, &f.group, f.target, &f.device));
    assert!(!store.studio_source_is_warm(SERVER, &other_group, f.target, &f.device));
    assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &other));
    assert!(!store.studio_source_is_warm(
        SERVER,
        &f.group,
        StudioTarget::Flipnote {
            channel: [8; 16],
            object: [9; 16]
        },
        &f.device
    ));
    f.group
        .add_member(&f.device, other.key_package().unwrap())
        .unwrap();
    assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
    let state = f.load(&store).unwrap();
    drop(store);
    let mut store = open(root.path());
    store.retain_studio_source(&f.group, &f.device, state);
    assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
}

#[test]
fn studio_source_reuse_failed_write_or_flush_discards_owned_graph() {
    for flush in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let f = Fixture::new(true);
        let mut b = budget(&mut store, &f);
        let (first, _) = f.edit(&mut store, &mut b, f.insert());
        let op = if flush {
            first
        } else {
            incoming(&f, &store, 2)
        };
        warm(&mut store, &f);
        let original = fs::read(f.path(&store)).unwrap();
        let result = store.ingest_studio_epoch_reusing_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &op,
            &mut rng(),
            &mut b,
            |_, _| Err(invalid("injected write failure")),
            |_, _| Err(invalid("injected flush failure")),
        );
        assert!(result.unwrap_err().to_string().contains(if flush {
            "flush failure"
        } else {
            "write failure"
        }));
        assert!(b.requires_reconciliation());
        assert!(!store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
        assert_eq!(fs::read(f.path(&store)).unwrap(), original);
    }
}
