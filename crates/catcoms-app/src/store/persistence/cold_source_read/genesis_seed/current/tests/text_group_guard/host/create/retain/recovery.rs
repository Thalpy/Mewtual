//! Actual owning-driver recovery across all original Current families. Faults run with
//! real created Files open, leaving the native prefix that the next automatic retry sees.
use super::*;
use crate::store::persistence::v2_text::V2TextRetainTestCut;

#[test]
fn actual_three_family_host_automatically_recovers_original_a_without_new_packet_or_nonce() {
    for (family, cut) in [
        (0, V2TextRetainTestCut::WalCreated),
        (1, V2TextRetainTestCut::ArtifactCreated),
        (2, V2TextRetainTestCut::HeadCreated),
    ] {
        let original = text_fixture(family, 0x7f3a0 + family as u64);
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x7f3b0 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [26; 32],
            channel: 7,
            text: "one interrupted original",
        };
        host.retry_prepare_create_request(&original.layout, request(), &mut rng)
            .unwrap();
        let old_head = std::fs::read(target.join("head.bin")).unwrap();
        host.set_next_retain_fault_for_test(cut).unwrap();
        assert!(host
            .retry_retain_create_request(&original.layout, request(), &mut rng)
            .is_err());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), old_head);
        // The first cut has a real empty pre-MAIN stage. The others have a complete
        // authenticated MAIN plus a real empty listed output or head.stage. The driver
        // must classify them and select only the corresponding fresh recovery role.
        let original_wal = if family == 0 {
            assert!(!target.join("text.wal").exists());
            assert_eq!(
                std::fs::metadata(target.join("text.wal.stage.v1"))
                    .unwrap()
                    .len(),
                0
            );
            None
        } else {
            Some(std::fs::read(target.join("text.wal")).unwrap())
        };
        host.retry_retain_create_request(&original.layout, request(), &mut NeverRng)
            .unwrap();
        let selected = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        assert_ne!(selected, old_head);
        if let Some(expected) = original_wal {
            assert_eq!(wal, expected);
        }
        assert!(!target.join("text.wal.stage.v1").exists());
        assert!(!target.join("head.stage").exists());
        host.retry_retain_create_request(&original.layout, request(), &mut NeverRng)
            .unwrap();
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    }
}
