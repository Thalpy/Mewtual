//! Actual facade→WAL→immutable frames→fresh A selector. Declared with the real paired
//! transaction. A is retained original custody only; no accepted/publication result here.
use super::*;
mod recovery;

#[test]
fn actual_three_family_host_retains_the_same_packet_and_never_resigns_after_selection() {
    for family in 0..3 {
        let original = text_fixture(family, 0x7f350 + family as u64);
        let store = original.layout.checked_store().unwrap();
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        let source = std::fs::read(target.join("source.bin")).unwrap();
        let admission = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x7f360 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let previous = std::fs::read(target.join("head.bin")).unwrap();
        let request = || V2CreateRequest {
            token: [23; 32],
            channel: 7,
            text: "one durable original",
        };
        host.retry_retain_create_request(&original.layout, request(), &mut rng)
            .unwrap();
        let selected = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        assert_ne!(selected, previous);
        assert!(!wal.is_empty());
        for _ in 0..2 {
            host.retry_retain_create_request(&original.layout, request(), &mut NeverRng)
                .unwrap();
            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected);
            assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        }
        assert!(host
            .retry_retain_create_request(
                &original.layout,
                V2CreateRequest {
                    token: [23; 32],
                    channel: 7,
                    text: "same token, altered body",
                },
                &mut NeverRng
            )
            .is_err());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        assert_eq!(std::fs::read(target.join("source.bin")).unwrap(), source);
        assert_eq!(
            std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
            admission
        );
        assert_eq!(
            active_text_lane_count_for_test(store),
            1,
            "retained A is still pending; only the actual ordinary retirement can release it"
        );
        host.cancel_and_drain_admitted_promotion().unwrap();
        assert!(host
            .retry_retain_create_request(&original.layout, request(), &mut NeverRng)
            .is_err());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    }
}

#[test]
fn actual_host_preserves_a_foreign_wal_before_first_retain_admission() {
    let original = text_fixture(0, 0x7f370);
    let owner = original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    drop(owner);
    let mut host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_original_owner(original.owner),
        host_policy(),
    );
    let mut rng = ChaCha20Rng::seed_from_u64(0x7f371);
    host.retry_empty_promotion(&original.layout, &mut rng)
        .unwrap();
    let request = || V2CreateRequest {
        token: [24; 32],
        channel: 7,
        text: "already prepared original",
    };
    host.retry_prepare_create_request(&original.layout, request(), &mut rng)
        .unwrap();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let foreign = b"preserve unrelated or interrupted journal";
    std::fs::write(target.join("text.wal"), foreign).unwrap();
    for _ in 0..2 {
        assert!(host
            .retry_retain_create_request(&original.layout, request(), &mut rng)
            .is_err());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), foreign);
    }
}
