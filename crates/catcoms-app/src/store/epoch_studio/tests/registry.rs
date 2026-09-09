use super::*;
use catcoms_replication::registry::{registry_document, PointerKey, RegistryOp};

fn refresh(f: &Fixture, store: &mut ServerStore) -> Option<EpochRegistryState> {
    let mut b = budget(store, f);
    store
        .refresh_studio_registry_pointer(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap()
}

#[test]
fn studio_registry_refresh_uses_actual_checkpoint_and_never_duplicates_or_rewinds() {
    for art in [false, true] {
        let f = Fixture::new(art);
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        assert!(
            refresh(&f, &mut store).is_none(),
            "absence must not publish a pointer"
        );
        let key = PointerKey::new(f.logical.doc_type, f.logical.logical_key.clone()).unwrap();
        let bucket = key.bucket();
        let document = registry_document(&f.group.group_id(), bucket).unwrap();
        let mut b = budget(&mut store, &f);
        let (_, state) = f.edit(&mut store, &mut b, f.insert());
        store.retain_studio_source(&f.group, &f.device, state);
        let first = refresh(&f, &mut store).unwrap();
        assert_eq!(first.projection().unwrap().pointers.get(&key), Some(&0));
        assert_eq!(first.op_count(), 1);
        assert!(refresh(&f, &mut store).is_none());
        assert_eq!(
            store
                .load_epoch_intents(SERVER, &document)
                .unwrap()
                .pending()
                .len(),
            1
        );
        // Install an actually receipt-verified successor using the existing adoption fixture.
        // It reuses the original insert nonce, an exact accepted retry, then adopts the seed.
        super::discovery::prepared(&f, &mut store);
        let next = refresh(&f, &mut store).unwrap();
        assert_eq!(next.projection().unwrap().pointers.get(&key), Some(&1));
        assert_eq!(next.op_count(), 2);
        for op in [
            RegistryOp::Put {
                key: key.clone(),
                epoch: 7,
            },
            RegistryOp::Tombstone { key: key.clone() },
        ] {
            let mut b = budget(&mut store, &f);
            store
                .edit_registry_epoch(
                    SERVER,
                    &f.group,
                    bucket,
                    next.doc_id(),
                    &f.device,
                    op.domain_op(
                        &f.group.group_id(),
                        if matches!(op, RegistryOp::Put { .. }) {
                            [61; 16]
                        } else {
                            [62; 16]
                        },
                    )
                    .unwrap(),
                    &mut rng(),
                    &mut b.storage,
                    &mut b.intents,
                )
                .unwrap();
            assert!(
                refresh(&f, &mut store).is_none(),
                "ordinary refresh cannot overwrite a later/deleted hint"
            );
        }
        let final_state = store
            .load_registry_epoch(SERVER, &f.group, bucket, &f.device)
            .unwrap()
            .unwrap();
        assert_eq!(final_state.op_count(), 4);
        assert!(final_state.projection().unwrap().tombstones.contains(&key));
    }
}
