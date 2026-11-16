//! Known positive owner-operation formats do not imply absence of critical initialization debt.
use super::*;

#[test]
fn known_v5_v6_operation_helpers_do_not_widen_exact_v4_absence_or_first_stage_recovery() {
    for critical in [false, true] {
        for fault in [1, 3] {
            let directory = tempfile::tempdir().unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0x6ad5);
            let store = open_format(directory.path(), &mut rng, critical);
            let mut initial = store
                .prepare_owner_evidence_founder(
                    Hub::new().join(PeerId::from_u64(0x6ad5)),
                    MlsDevice::generate().unwrap(),
                    ChaCha20Rng::seed_from_u64(0x6ad5),
                    Box::new(ManualClock::new(1_000)),
                    "known-format negative recovery remains closed",
                    9,
                )
                .unwrap();
            let group = initial.group().unwrap().to_vec();
            let signer = *initial.founder().unwrap();
            store
                .owner_evidence
                .canonical_fault
                .store(fault, Ordering::SeqCst);
            assert!(initial.retry(&store, &mut rng).is_err());
            let id = key(&group, &signer);
            let canonical = directory
                .path()
                .join("owner-state")
                .join(format!("{}.bin", hex::encode(id)));
            let stage = directory
                .path()
                .join("owner-state")
                .join(format!("{}.replace.tmp", hex::encode(id)));
            assert!(!canonical.exists());
            let retained = if fault == 3 {
                Some(std::fs::read(&stage).unwrap())
            } else {
                assert!(!stage.exists());
                None
            };
            let lineage = std::fs::read(directory.path().join("chat-admissions.bin")).unwrap();
            drop(initial);
            drop(store);
            let reopened = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
            reopened.configure_owner_evidence_budget(budget()).unwrap();
            assert!(reopened
                .chat_vault_format()
                .unwrap()
                .requires_owner_operations());
            assert!(reopened.prepare_owner_evidence_restore(&group, signer).is_err(),
                "known positive format metadata cannot authorize v4-only absence or gen1 stage fallback");
            assert!(!canonical.exists());
            match retained {
                Some(bytes) => assert_eq!(std::fs::read(&stage).unwrap(), bytes),
                None => assert!(!stage.exists()),
            }
            assert_eq!(
                std::fs::read(directory.path().join("chat-admissions.bin")).unwrap(),
                lineage,
                "unsupported recovery must not silently retire or make an initializing row Current"
            );
            assert_eq!(
                reopened.owner_evidence.canonical.lock().unwrap().hot_bytes,
                0
            );
        }
    }
}
