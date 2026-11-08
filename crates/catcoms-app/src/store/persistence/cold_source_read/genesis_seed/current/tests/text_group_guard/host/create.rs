//! Genuine facade→query→Clear predecessor→original SDK→retained DATA. No A save,
//! Append key cut, accepted/publication result or production native activation is claimed.
use super::*;
use catcoms_sync::owner_v2::V2CreateRequest;
mod append;
mod intent;
mod retain;

#[test]
fn actual_three_family_host_prepares_one_request_and_retries_exact_data_without_storage_effects() {
    for family in 0..3 {
        let original = text_fixture(family, 0x7f300 + family as u64);
        let store = original.layout.checked_store().unwrap();
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        let rows = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
        let source = std::fs::read(target.join("source.bin")).unwrap();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x7f310 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let request = || V2CreateRequest {
            token: [17; 32],
            channel: 7,
            text: "one original request",
        };
        assert!(host.prepared_create_digest_for_test().is_none());
        host.retry_prepare_create_request(&original.layout, request(), &mut rng)
            .unwrap();
        let original_data = host.prepared_create_digest_for_test().unwrap();
        for _ in 0..2 {
            host.retry_prepare_create_request(&original.layout, request(), &mut NeverRng)
                .unwrap();
            assert_eq!(host.prepared_create_digest_for_test(), Some(original_data));
        }
        assert!(host
            .retry_prepare_create_request(
                &original.layout,
                V2CreateRequest {
                    token: [17; 32],
                    channel: 7,
                    text: "changed body"
                },
                &mut NeverRng
            )
            .is_err());
        assert_eq!(host.prepared_create_digest_for_test(), Some(original_data));
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("source.bin")).unwrap(), source);
        assert_eq!(
            std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
            rows
        );
        assert!(!target.join("text.wal").exists());
        assert_eq!(
            active_text_lane_count_for_test(store),
            1,
            "logical debt is not retired by DATA success"
        );
        host.cancel_and_drain_admitted_promotion().unwrap();
        assert!(host
            .retry_prepare_create_request(&original.layout, request(), &mut NeverRng)
            .is_err());
        assert_eq!(host.prepared_create_digest_for_test(), Some(original_data));
    }
}

#[test]
fn actual_host_never_prepares_a_new_packet_when_an_unselected_wal_is_present() {
    let original = text_fixture(0, 0x7f320);
    let owner = original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    drop(owner);
    let mut host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_original_owner(original.owner),
        host_policy(),
    );
    host.retry_empty_promotion(&original.layout, &mut ChaCha20Rng::seed_from_u64(0x7f321))
        .unwrap();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let wal = target.join("text.wal");
    let original_wal = b"unknown or interrupted WAL; preserve";
    std::fs::write(&wal, original_wal).unwrap();
    for _ in 0..2 {
        assert!(host
            .retry_prepare_create_request(
                &original.layout,
                V2CreateRequest {
                    token: [18; 32],
                    channel: 7,
                    text: "never signed as new"
                },
                &mut NeverRng
            )
            .is_err());
        assert!(host.prepared_create_digest_for_test().is_none());
        assert_eq!(std::fs::read(&wal).unwrap(), original_wal);
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    }
}
