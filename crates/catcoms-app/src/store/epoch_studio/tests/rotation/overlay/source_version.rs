//! OVERLAY-TEST-001: change only the actual Closing source version, not a supplied hash.
use super::*;

pub(super) fn check(accepted: bool) {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        eligible(&f, &mut store);
        // The separate sender has not seen the receipt. Its valid signed operation is
        // deliberately withheld until the real receiver has sealed its current source.
        let mut sender = f.load(&store).unwrap();
        assert_eq!(sender.phase(), EpochPhase::Open);
        let mut late = f.title();
        late.nonce = [76; 16];
        let packet = sender
            .unit
            .edit_or_reseal(&f.device, &f.group, &mut rng(), &late, 100)
            .unwrap();
        let (close, basis) = seal_source(&f, &mut store);
        let saved =
            accepted.then(|| save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123));
        let mut before = f.load(&store).unwrap();
        assert_eq!(before.phase(), EpochPhase::Closing);
        assert_eq!(before.unit.quarantined_len(), 0);
        let original_snapshot = before.unit.snapshot().unwrap();
        let original_plan = before.unit.prepare_settlement(&close, &f.group, 0).unwrap();
        let receipt = original_plan.receipt().clone();
        let seed = original_plan.checkpoint().bytes().to_vec();
        assert_eq!(receipt.close_record_hash, close.hash());
        receipt.verify_current_owner(&f.group, 0).unwrap();
        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let intent_path = store
            .dir
            .join("servers")
            .join(format!("{}.intents", blake3::hash(&scope).to_hex()));
        let original_intents = fs::read(&intent_path).unwrap();
        let original_source = fs::read(f.path(&store)).unwrap();

        // Explicitly warm the real source before the bounded ingest/store path. No gate
        // fields, snapshots or source stamps are edited by the fixture.
        warm(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let (admission, received) = store
            .ingest_studio_epoch_reusing(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &packet,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        assert_eq!(admission, Admission::Quarantined);
        assert_eq!(received.unit.quarantined_len(), 1);
        assert_ne!(fs::read(f.path(&store)).unwrap(), original_source);
        assert_eq!(fs::read(&intent_path).unwrap(), original_intents);
        drop(store);

        // Reopen so a transient cache cannot substitute for the changed persisted source.
        let mut store = open(root.path());
        let mut changed = f.load(&store).unwrap();
        assert_eq!(changed.phase(), EpochPhase::Closing);
        assert_eq!(changed.doc_id(), before.doc_id());
        assert_eq!(changed.epoch(), before.epoch());
        assert_eq!(changed.unit.target(), before.unit.target());
        assert_eq!(changed.unit.quarantined_len(), 1);
        assert_ne!(changed.unit.snapshot().unwrap(), original_snapshot);
        assert!(!original_plan.matches_source(&mut changed.unit).unwrap());
        assert_eq!(changed.unit.receipt_head().unwrap(), Some(&receipt));
        let changed_plan = changed
            .unit
            .prepare_settlement(&close, &f.group, 0)
            .unwrap();
        assert_eq!(changed_plan.receipt(), &receipt);
        assert_eq!(changed_plan.checkpoint().bytes(), seed);
        assert_eq!(changed.projection().unwrap(), before.projection().unwrap());
        // The exact same close and owner-tenure input still independently verify. Fresh
        // preparation succeeds, ruling out Open/Fault/Unknown or a changed closure/seed.
        receipt.verify_current_owner(&f.group, 0).unwrap();
        let mut b = budget(&mut store, &f);
        let fresh = store
            .prepare_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                &mut b,
            )
            .unwrap();
        assert_ne!(
            fresh.fingerprint(),
            basis.fingerprint(),
            "overlay basis ignored changed persisted Closing source version"
        );

        let mut new_request = f.title();
        new_request.nonce = [77; 16];
        let result = store.save_studio_closing_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            Some(0),
            basis.fingerprint(),
            new_request.clone(),
            456,
            &mut rng(),
            &mut b,
        );
        assert!(
            result.is_err(),
            "stale Closing basis admitted a new overlay request"
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            invalid("Closing overlay basis changed").to_string()
        );
        assert_eq!(fs::read(&intent_path).unwrap(), original_intents);
        let source_after_ingest = canonical(&store);
        if let Some(saved) = saved {
            let retry = save(&f, &mut store, &close, basis.fingerprint(), f.title(), 999);
            assert_eq!(retry.accepted(), 1);
            assert_eq!(retry.basis(), saved.basis());
            assert_eq!(retry.projection(), saved.projection());
            assert_eq!(fs::read(&intent_path).unwrap(), original_intents);
        } else {
            assert!(store
                .load_epoch_intents(SERVER, &f.logical)
                .unwrap()
                .overlay()
                .is_none());
            // Positive control: this identical request is valid with the freshly obtained
            // basis. The stale request was not rejected for unrelated typed semantics.
            let saved = save(
                &f,
                &mut store,
                &close,
                fresh.fingerprint(),
                new_request,
                456,
            );
            assert_eq!(saved.accepted(), 1);
        }
        assert_eq!(canonical(&store), source_after_ingest);
    }
}
